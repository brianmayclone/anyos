//! Async block I/O scheduler.
//!
//! This layer decouples VFS/filesystem planning from backend execution.  It is
//! intentionally conservative: existing synchronous storage drivers remain the
//! execution backend, while foreground reads/writes get completions and
//! readahead becomes best-effort background work.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::sync::spinlock::Spinlock;

const WORKER_COUNT: usize = 2;
const WORKER_PRIORITY: u8 = 96;
const MAX_PENDING_REQUESTS: usize = 128;
const MAX_ASYNC_PREFETCH_SECTORS: u32 = 128;
const MAX_PENDING_PREFETCH_REQUESTS: usize = 24;
const MAX_PENDING_PREFETCH_WITH_FOREGROUND: usize = 6;
const PREFETCH_CACHE_POPULATE_SECTORS: u32 = 32;
const MIN_FOREGROUND_ASYNC_SECTORS: u32 = 1;

const PRIORITY_FOREGROUND: u8 = 0;
const PRIORITY_WRITE: u8 = 1;
const PRIORITY_READAHEAD: u8 = 8;

#[derive(Copy, Clone, PartialEq, Eq)]
enum AsyncOp {
    Read,
    Write,
    Prefetch,
}

enum AsyncBuffer {
    Borrowed { addr: usize, len: usize },
    Owned(Vec<u8>),
}

struct AsyncRequest {
    id: u64,
    op: AsyncOp,
    priority: u8,
    disk_id: u8,
    lba: u32,
    count: u32,
    op_kind: u32,
    buffer: AsyncBuffer,
    completion: Option<Arc<IoCompletion>>,
}

struct AsyncQueue {
    pending: VecDeque<AsyncRequest>,
    sleeping_workers: [u32; WORKER_COUNT],
}

impl AsyncQueue {
    const fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            sleeping_workers: [0; WORKER_COUNT],
        }
    }

    fn pop_best(&mut self) -> Option<AsyncRequest> {
        if self.pending.is_empty() {
            return None;
        }

        let mut best_idx = 0usize;
        let mut best_priority = self.pending[0].priority;
        for (idx, req) in self.pending.iter().enumerate().skip(1) {
            if req.priority < best_priority {
                best_priority = req.priority;
                best_idx = idx;
            }
        }
        self.pending.remove(best_idx)
    }

    fn remember_sleeping_worker(&mut self, tid: u32) {
        if tid == 0 || self.sleeping_workers.iter().any(|&w| w == tid) {
            return;
        }
        if let Some(slot) = self.sleeping_workers.iter_mut().find(|w| **w == 0) {
            *slot = tid;
        }
    }

    fn take_sleeping_worker(&mut self) -> u32 {
        for slot in &mut self.sleeping_workers {
            if *slot != 0 {
                let tid = *slot;
                *slot = 0;
                return tid;
            }
        }
        0
    }

    fn make_room_for(&mut self, priority: u8) -> bool {
        if priority == PRIORITY_READAHEAD {
            if self
                .pending
                .iter()
                .any(|req| req.priority == PRIORITY_WRITE)
            {
                return false;
            }
            let pending_prefetch = self
                .pending
                .iter()
                .filter(|req| req.op == AsyncOp::Prefetch)
                .count();
            let foreground_pending = self
                .pending
                .iter()
                .any(|req| req.priority == PRIORITY_FOREGROUND);
            let prefetch_limit = if foreground_pending {
                MAX_PENDING_PREFETCH_WITH_FOREGROUND
            } else {
                MAX_PENDING_PREFETCH_REQUESTS
            };
            if pending_prefetch >= prefetch_limit {
                return false;
            }
        }

        if self.pending.len() < MAX_PENDING_REQUESTS {
            return true;
        }
        if priority > PRIORITY_WRITE {
            return false;
        }

        for idx in (0..self.pending.len()).rev() {
            if self.pending[idx].op == AsyncOp::Prefetch {
                let _ = self.pending.remove(idx);
                DROPPED.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }
}

struct IoCompletionState {
    done: bool,
    ok: bool,
    bytes: usize,
    waiter_tid: u32,
}

pub struct IoCompletion {
    state: Spinlock<IoCompletionState>,
}

impl IoCompletion {
    fn new() -> Self {
        Self {
            state: Spinlock::new(IoCompletionState {
                done: false,
                ok: false,
                bytes: 0,
                waiter_tid: 0,
            }),
        }
    }
}

static STARTED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static SUBMITTED: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static FALLBACK_SYNC: AtomicU64 = AtomicU64::new(0);
static WORKER_TIDS: [AtomicU32; WORKER_COUNT] = [const { AtomicU32::new(0) }; WORKER_COUNT];
static QUEUE: Spinlock<AsyncQueue> = Spinlock::new(AsyncQueue::new());

pub fn init() {
    if STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let mut spawned = 0usize;
    for slot in &WORKER_TIDS {
        let tid = crate::task::scheduler::spawn(async_io_worker, WORKER_PRIORITY, "aio_blk");
        if tid != 0 {
            slot.store(tid, Ordering::Release);
            spawned += 1;
        }
    }

    if spawned == 0 {
        STARTED.store(false, Ordering::Release);
        crate::serial_println!("[storage/aio] failed to start workers; using sync I/O");
    } else {
        crate::serial_println!(
            "[storage/aio] async block I/O started ({} workers)",
            spawned
        );
    }
}

pub fn is_started() -> bool {
    STARTED.load(Ordering::Acquire)
}

fn current_is_worker() -> bool {
    let tid = crate::task::scheduler::current_tid();
    tid != 0
        && WORKER_TIDS
            .iter()
            .any(|worker| worker.load(Ordering::Acquire) == tid)
}

fn can_queue() -> bool {
    is_started() && crate::task::scheduler::current_tid() != 0 && !current_is_worker()
}

fn enqueue(req: AsyncRequest) -> bool {
    let wake_tid = {
        let mut queue = QUEUE.lock();
        if !queue.make_room_for(req.priority) {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        queue.pending.push_back(req);
        queue.take_sleeping_worker()
    };

    SUBMITTED.fetch_add(1, Ordering::Relaxed);
    if wake_tid != 0 {
        crate::task::scheduler::wake_thread(wake_tid);
    }
    true
}

fn wait_for_completion(completion: &Arc<IoCompletion>) -> (bool, usize) {
    loop {
        {
            let mut state = completion.state.lock();
            if state.done {
                return (state.ok, state.bytes);
            }
            let tid = crate::task::scheduler::current_tid();
            if tid == 0 {
                return (false, 0);
            }
            state.waiter_tid = tid;
            crate::task::scheduler::prepare_to_block_current();
        }
        crate::task::scheduler::schedule();
    }
}

fn complete(completion: Arc<IoCompletion>, ok: bool, bytes: usize) {
    let waiter = {
        let mut state = completion.state.lock();
        state.ok = ok;
        state.bytes = bytes;
        state.done = true;
        let waiter = state.waiter_tid;
        state.waiter_tid = 0;
        waiter
    };

    COMPLETED.fetch_add(1, Ordering::Relaxed);
    if waiter != 0 {
        crate::task::scheduler::wake_thread(waiter);
    }
}

pub fn read_sectors_wait(disk_id: u8, lba: u32, count: u32, buf: &mut [u8], op_kind: u32) -> bool {
    let bytes = count as usize * 512;
    if count < MIN_FOREGROUND_ASYNC_SECTORS || bytes > buf.len() || !can_queue() {
        FALLBACK_SYNC.fetch_add(1, Ordering::Relaxed);
        return super::read_sectors_raw_chunked(disk_id, lba, count, buf, op_kind);
    }

    let completion = Arc::new(IoCompletion::new());
    let req = AsyncRequest {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        op: AsyncOp::Read,
        priority: PRIORITY_FOREGROUND,
        disk_id,
        lba,
        count,
        op_kind,
        buffer: AsyncBuffer::Borrowed {
            addr: buf.as_mut_ptr() as usize,
            len: buf.len(),
        },
        completion: Some(Arc::clone(&completion)),
    };

    if !enqueue(req) {
        FALLBACK_SYNC.fetch_add(1, Ordering::Relaxed);
        return super::read_sectors_raw_chunked(disk_id, lba, count, buf, op_kind);
    }

    let (ok, copied) = wait_for_completion(&completion);
    ok && copied >= bytes
}

pub fn write_sectors_wait(disk_id: u8, lba: u32, count: u32, buf: &[u8], op_kind: u32) -> bool {
    let bytes = count as usize * 512;
    if count < MIN_FOREGROUND_ASYNC_SECTORS || bytes > buf.len() || !can_queue() {
        FALLBACK_SYNC.fetch_add(1, Ordering::Relaxed);
        return super::write_sectors_raw_chunked(disk_id, lba, count, buf, op_kind);
    }

    let completion = Arc::new(IoCompletion::new());
    let req = AsyncRequest {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        op: AsyncOp::Write,
        priority: PRIORITY_WRITE,
        disk_id,
        lba,
        count,
        op_kind,
        buffer: AsyncBuffer::Borrowed {
            addr: buf.as_ptr() as usize,
            len: buf.len(),
        },
        completion: Some(Arc::clone(&completion)),
    };

    if !enqueue(req) {
        FALLBACK_SYNC.fetch_add(1, Ordering::Relaxed);
        return super::write_sectors_raw_chunked(disk_id, lba, count, buf, op_kind);
    }

    let (ok, written) = wait_for_completion(&completion);
    ok && written >= bytes
}

pub fn prefetch_sectors(disk_id: u8, lba: u32, count: u32) {
    if count == 0 || !is_started() {
        return;
    }

    let mut done = 0u32;
    while done < count {
        let batch = (count - done).min(MAX_ASYNC_PREFETCH_SECTORS);
        let bytes = batch as usize * 512;
        let req = AsyncRequest {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            op: AsyncOp::Prefetch,
            priority: PRIORITY_READAHEAD,
            disk_id,
            lba: lba + done,
            count: batch,
            op_kind: super::IO_OP_READAHEAD,
            buffer: AsyncBuffer::Owned(vec![0u8; bytes]),
            completion: None,
        };
        if !enqueue(req) {
            return;
        }
        done += batch;
    }
}

extern "C" fn async_io_worker() {
    loop {
        let req = loop {
            let tid = crate::task::scheduler::current_tid();
            {
                let mut queue = QUEUE.lock();
                if let Some(req) = queue.pop_best() {
                    break req;
                }
                queue.remember_sleeping_worker(tid);
                crate::task::scheduler::prepare_to_block_current();
            }
            crate::task::scheduler::schedule();
        };
        process_request(req);
    }
}

fn process_request(mut req: AsyncRequest) {
    let was_prefetch = req.op == AsyncOp::Prefetch;
    let _id = req.id;
    let bytes = req.count as usize * 512;
    let (ok, completed_bytes) = match req.op {
        AsyncOp::Read => match req.buffer {
            AsyncBuffer::Borrowed { addr, len } if len >= bytes => {
                let buf = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, bytes) };
                let ok = super::read_sectors_raw_chunked(
                    req.disk_id,
                    req.lba,
                    req.count,
                    buf,
                    req.op_kind,
                );
                (ok, if ok { bytes } else { 0 })
            }
            _ => (false, 0),
        },
        AsyncOp::Write => match req.buffer {
            AsyncBuffer::Borrowed { addr, len } if len >= bytes => {
                let buf = unsafe { core::slice::from_raw_parts(addr as *const u8, bytes) };
                let ok = super::write_sectors_raw_chunked(
                    req.disk_id,
                    req.lba,
                    req.count,
                    buf,
                    req.op_kind,
                );
                (ok, if ok { bytes } else { 0 })
            }
            _ => (false, 0),
        },
        AsyncOp::Prefetch => match req.buffer {
            AsyncBuffer::Owned(ref mut buf) if buf.len() >= bytes => {
                let ok = super::read_sectors_raw_chunked(
                    req.disk_id,
                    req.lba,
                    req.count,
                    &mut buf[..bytes],
                    req.op_kind,
                );
                if ok {
                    populate_prefetch_cache(req.disk_id, req.lba, req.count, &mut buf[..bytes]);
                }
                (ok, if ok { bytes } else { 0 })
            }
            _ => (false, 0),
        },
    };

    if let Some(completion) = req.completion.take() {
        complete(completion, ok, completed_bytes);
    }

    if was_prefetch {
        throttle_prefetch_worker();
    }
}

fn populate_prefetch_cache(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) {
    let mut done = 0u32;
    while done < count {
        let batch = (count - done).min(PREFETCH_CACHE_POPULATE_SECTORS);
        let offset = done as usize * 512;
        let bytes = batch as usize * 512;
        crate::fs::blockcache::overlay_cached(
            disk_id,
            lba + done,
            batch,
            &mut buf[offset..offset + bytes],
        );
        crate::fs::blockcache::populate(disk_id, lba + done, batch, &buf[offset..offset + bytes]);
        done += batch;
        if done < count {
            crate::task::scheduler::schedule();
        }
    }
}

fn throttle_prefetch_worker() {
    crate::task::scheduler::schedule();
}

pub fn stats() -> (u64, u64, u64, u64) {
    (
        SUBMITTED.load(Ordering::Relaxed),
        COMPLETED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        FALLBACK_SYNC.load(Ordering::Relaxed),
    )
}
