//! Block storage device drivers with backend dispatch.
//!
//! Routes read/write requests to the active storage backend (ATA PIO or AHCI DMA).
//! The backend is selected at boot based on hardware detection.
//!
//! I/O backends that cannot safely overlap requests are serialized via
//! `IO_LOCK` — a yielding lock that gives up the CPU time slice instead of
//! busy-spinning when contended. Reads are accelerated by the global block
//! cache (`fs::blockcache`).

pub mod ahci;
pub mod ata;
pub mod atapi;
pub mod blockdev;
pub mod lsi_scsi;
pub mod nvme;
pub mod sdhci;

use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// ── Per-Device I/O Override ──────────────────────────────────────────────────
// Secondary storage drivers (USB mass storage, etc.) register their own
// read/write handlers for specific disk IDs, bypassing the global backend.

/// I/O handler override for a specific disk.
pub(crate) struct DeviceIoHandler {
    pub disk_id: u8,
    pub read_fn: fn(u8, u32, u32, &mut [u8]) -> bool, // (disk_id, lba, count, buf)
    pub write_fn: fn(u8, u32, u32, &[u8]) -> bool,    // (disk_id, lba, count, buf)
}

pub(crate) static IO_OVERRIDES: Spinlock<Vec<DeviceIoHandler>> = Spinlock::new(Vec::new());

/// Register a per-device I/O handler (called by USB storage driver, etc.).
pub fn register_device_io(
    disk_id: u8,
    read_fn: fn(u8, u32, u32, &mut [u8]) -> bool,
    write_fn: fn(u8, u32, u32, &[u8]) -> bool,
) {
    IO_OVERRIDES.lock().push(DeviceIoHandler {
        disk_id,
        read_fn,
        write_fn,
    });
}

/// Remove a per-device I/O handler (for hot-unplug).
pub fn unregister_device_io(disk_id: u8) {
    IO_OVERRIDES.lock().retain(|h| h.disk_id != disk_id);
}

/// Read sectors via a per-device I/O override (for ISO 9660 USB CDROM integration).
pub fn read_via_override(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let overrides = IO_OVERRIDES.lock();
    if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
        let f = handler.read_fn;
        drop(overrides);
        return f(disk_id, lba, count, buf);
    }
    false
}

#[derive(Copy, Clone, PartialEq)]
enum StorageBackend {
    Ata,
    Ahci,
    Nvme,
    LsiScsi,
}

static mut BACKEND: StorageBackend = StorageBackend::Ata;

/// Yielding lock for serializing disk I/O.  Does NOT disable interrupts.
/// When contended, yields the CPU time slice instead of busy-spinning,
/// allowing other threads to make progress while waiting for I/O.
static IO_LOCK: AtomicBool = AtomicBool::new(false);
static IO_LOCK_OWNER_TID: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_ACQUIRED_TICK: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_OP_KIND: AtomicU32 = AtomicU32::new(IO_OP_UNKNOWN);
static IO_LOCK_OP_DISK: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_OP_LBA: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_OP_COUNT: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_WAIT_LOGS: AtomicU32 = AtomicU32::new(0);
static IO_LOCK_HOLD_LOGS: AtomicU32 = AtomicU32::new(0);
static AHCI_FLUSH_FAILED_MASK: AtomicU32 = AtomicU32::new(0);
const IO_LOCK_WAIT_WARN_MS: u32 = 50;
const IO_LOCK_HOLD_WARN_MS: u32 = 250;
const IO_LOCK_LOG_LIMIT: u32 = 64;
// Keep legacy serialized I/O slices short.  Large locked transfers make every
// waiter resume as a pinned kernel continuation on its previous CPU, which can
// starve latency-sensitive threads such as the compositor under LXE I/O storms.
const MAX_LOCKED_IO_SECTORS: u32 = 64;
const MAX_LOCKED_READAHEAD_SECTORS: u32 = 64;
const MAX_LOCKED_WRITE_SECTORS: u32 = 64;
const MAX_LOCKED_WRITEBACK_SECTORS: u32 = 64;
const MAX_UNLOCKED_AHCI_IO_SECTORS: u32 = 512;
const IO_OP_UNKNOWN: u32 = 0;
const IO_OP_READ: u32 = 1;
const IO_OP_READAHEAD: u32 = 2;
const IO_OP_WRITE: u32 = 3;
const IO_OP_WRITEBACK: u32 = 4;
const IO_OP_FLUSH: u32 = 5;

#[derive(Copy, Clone)]
struct IoLockOp {
    kind: u32,
    disk_id: u8,
    lba: u32,
    count: u32,
}

impl IoLockOp {
    const fn new(kind: u32, disk_id: u8, lba: u32, count: u32) -> Self {
        Self {
            kind,
            disk_id,
            lba,
            count,
        }
    }
}

fn io_op_name(kind: u32) -> &'static str {
    match kind {
        IO_OP_READ => "read",
        IO_OP_READAHEAD => "readahead",
        IO_OP_WRITE => "write",
        IO_OP_WRITEBACK => "writeback",
        IO_OP_FLUSH => "flush",
        _ => "unknown",
    }
}

/// Counter for I/O operations in progress (for statistics).
static IO_OPS_TOTAL: AtomicU32 = AtomicU32::new(0);

/// Adaptive readahead state — tracks the last read position to detect
/// sequential access patterns and scale readahead accordingly.
static LAST_READ_END_LBA: AtomicU32 = AtomicU32::new(0);
/// Current readahead level (in sectors). Doubles on sequential hits,
/// resets to minimum on random access. Range: 16..512 sectors (8-256 KiB).
static READAHEAD_LEVEL: AtomicU32 = AtomicU32::new(64);
const READAHEAD_MIN: u32 = 16; //   8 KiB
const READAHEAD_MAX: u32 = 512; // 256 KiB
const READ_CACHE_POPULATE_MAX: u32 = 64; // avoid polluting cache with 64K+ streams
const READAHEAD_TRIGGER_MAX: u32 = 128;

/// Reusable readahead buffer pool.
///
/// Large reads with readahead previously did `alloc::vec![0u8; fetch_bytes]`
/// on every call (up to ~300 KiB) and dropped it again at the end of the
/// function. Under sustained I/O that kept the kernel heap allocator hot on
/// every CPU simultaneously and amplified any latent allocator bug. The pool
/// keeps four static buffers around so the steady-state miss path needs no
/// heap allocation at all; it falls back to a `Vec` when all slots are busy or
/// when an unusually large fetch does not fit the fixed slot.
const READAHEAD_POOL_SIZE: usize = 4;
const READAHEAD_BUF_BYTES: usize = (READAHEAD_MAX as usize + READAHEAD_TRIGGER_MAX as usize) * 512;

struct ReadaheadSlot {
    in_use: AtomicBool,
    buf: core::cell::UnsafeCell<[u8; READAHEAD_BUF_BYTES]>,
}

// Safety: the `in_use` flag provides exclusive access to the UnsafeCell.
unsafe impl Sync for ReadaheadSlot {}

static READAHEAD_POOL: [ReadaheadSlot; READAHEAD_POOL_SIZE] = [
    ReadaheadSlot {
        in_use: AtomicBool::new(false),
        buf: core::cell::UnsafeCell::new([0u8; READAHEAD_BUF_BYTES]),
    },
    ReadaheadSlot {
        in_use: AtomicBool::new(false),
        buf: core::cell::UnsafeCell::new([0u8; READAHEAD_BUF_BYTES]),
    },
    ReadaheadSlot {
        in_use: AtomicBool::new(false),
        buf: core::cell::UnsafeCell::new([0u8; READAHEAD_BUF_BYTES]),
    },
    ReadaheadSlot {
        in_use: AtomicBool::new(false),
        buf: core::cell::UnsafeCell::new([0u8; READAHEAD_BUF_BYTES]),
    },
];

/// RAII handle — drops release the `in_use` flag so the slot is reusable.
struct ReadaheadLease {
    index: usize,
}

impl Drop for ReadaheadLease {
    fn drop(&mut self) {
        READAHEAD_POOL[self.index]
            .in_use
            .store(false, Ordering::Release);
    }
}

fn acquire_readahead_slot() -> Option<ReadaheadLease> {
    for i in 0..READAHEAD_POOL_SIZE {
        if READAHEAD_POOL[i]
            .in_use
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return Some(ReadaheadLease { index: i });
        }
    }
    None
}

/// Borrow a mutable slice into the pool buffer. The lease guarantees
/// exclusive ownership until it drops, so the raw-pointer → &mut slice is safe.
fn readahead_slot_bytes(lease: &ReadaheadLease, len: usize) -> &'static mut [u8] {
    debug_assert!(len <= READAHEAD_BUF_BYTES);
    unsafe {
        let ptr = (*READAHEAD_POOL[lease.index].buf.get()).as_mut_ptr();
        core::slice::from_raw_parts_mut(ptr, len)
    }
}

/// Per-Disk Sektor-Anzahl, einmalig beim Disk-Init gesetzt (AHCI/ATA/NVMe).
///
/// Lookup via [`disk_sector_count`] ist ein einziger atomic Load — kein Lock,
/// keine Allokation. Wird vom Readahead-Pfad in [`read_sectors_on_disk`]
/// genutzt, um Fetches nicht über das Disk-Ende hinaus auszudehnen (sonst
/// verweigert QEMU/AHCI den Transfer am letzten Sektor und hängt 5 s im
/// Kommando-Timeout).
///
/// Wert `0` bedeutet "unbekannt" — der Readahead-Pfad verzichtet dann auf
/// Readahead statt ein potenziell ungültiges Read auszulösen.
const MAX_DISKS: usize = 16;
static DISK_SECTORS: [AtomicU64; MAX_DISKS] = {
    // Keine `AtomicU64::new` in const context für Array-Initialisierung —
    // deshalb der `[const { ... }; N]` Trick.
    [const { AtomicU64::new(0) }; MAX_DISKS]
};

/// Hinterlegt die Sektor-Anzahl einer physischen Disk.
///
/// Wird von AHCI/ATA/NVMe beim Init aufgerufen, sobald `IDENTIFY DEVICE`
/// (oder das Äquivalent) die Kapazität geliefert hat. Nach dem Aufruf
/// kennt der Readahead-Pfad die Disk-Grenze und clampt Fetches korrekt.
pub fn set_disk_sector_count(disk_id: u8, sector_count: u64) {
    if (disk_id as usize) < MAX_DISKS {
        DISK_SECTORS[disk_id as usize].store(sector_count, Ordering::Relaxed);
    }
}

/// Liefert die registrierte Sektor-Anzahl einer physischen Disk, oder `0`
/// wenn kein Wert bekannt ist.
pub fn disk_sector_count(disk_id: u8) -> u64 {
    if (disk_id as usize) >= MAX_DISKS {
        return 0;
    }
    DISK_SECTORS[disk_id as usize].load(Ordering::Relaxed)
}

fn has_io_override(disk_id: u8) -> bool {
    IO_OVERRIDES.lock().iter().any(|h| h.disk_id == disk_id)
}

fn disk_limit_for_backend_io(disk_id: u8) -> u64 {
    let direct = disk_sector_count(disk_id);
    if direct > 0 {
        return direct;
    }

    // Several legacy paths pass a VFS/blockdev identifier as the cache key
    // while still targeting the primary physical disk. If there is no explicit
    // per-device override, the request falls through to the global backend.
    if disk_id != 0 && !has_io_override(disk_id) {
        return disk_sector_count(0);
    }

    0
}

fn check_backend_io_bounds(disk_id: u8, lba: u32, count: u32, op: &str) -> bool {
    let limit = disk_limit_for_backend_io(disk_id);
    if limit == 0 {
        return true;
    }

    let end = (lba as u64).saturating_add(count as u64);
    if end <= limit {
        return true;
    }

    crate::serial_println!(
        "[storage] refusing {} beyond disk: disk={} lba={} count={} sectors={}",
        op,
        disk_id,
        lba,
        count,
        limit
    );
    false
}

#[inline]
fn io_lock_acquire(op: IoLockOp) {
    let start = crate::arch::hal::timer_current_ticks();
    let owner_at_start = IO_LOCK_OWNER_TID.load(Ordering::Relaxed);
    let owner_op_at_start = IO_LOCK_OP_KIND.load(Ordering::Relaxed);
    let owner_disk_at_start = IO_LOCK_OP_DISK.load(Ordering::Relaxed);
    let owner_lba_at_start = IO_LOCK_OP_LBA.load(Ordering::Relaxed);
    let owner_count_at_start = IO_LOCK_OP_COUNT.load(Ordering::Relaxed);
    // Fast path: try once without yielding
    if IO_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        io_lock_record_acquired(
            start,
            owner_at_start,
            owner_op_at_start,
            owner_disk_at_start,
            owner_lba_at_start,
            owner_count_at_start,
            op,
        );
        return;
    }
    // Slow path: yield between attempts (avoids burning CPU cycles)
    io_lock_acquire_slow(
        start,
        owner_at_start,
        owner_op_at_start,
        owner_disk_at_start,
        owner_lba_at_start,
        owner_count_at_start,
        op,
    );
}

#[cold]
fn io_lock_acquire_slow(
    start: u32,
    owner_at_start: u32,
    owner_op_at_start: u32,
    owner_disk_at_start: u32,
    owner_lba_at_start: u32,
    owner_count_at_start: u32,
    op: IoLockOp,
) {
    let can_yield = crate::task::scheduler::current_tid() > 0;
    loop {
        // Brief spin (8 iterations) before yielding — handles very short holds
        for _ in 0..8 {
            if IO_LOCK
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                io_lock_record_acquired(
                    start,
                    owner_at_start,
                    owner_op_at_start,
                    owner_disk_at_start,
                    owner_lba_at_start,
                    owner_count_at_start,
                    op,
                );
                return;
            }
            core::hint::spin_loop();
        }
        // Back off so IO_LOCK storms don't turn into hot scheduler loops.
        // Use an immediate one-tick sleep rather than the generic contention
        // helper's initial pure yields: these waiters cannot make progress
        // until the storage owner releases IO_LOCK, and keeping them runnable
        // just makes the scheduler re-enter pinned kernel continuations.
        if can_yield {
            let wake_at = crate::arch::hal::timer_current_ticks().wrapping_add(1);
            crate::task::scheduler::sleep_until(wake_at);
        }
    }
}

fn ticks_to_ms(ticks: u32) -> u32 {
    let hz = crate::arch::hal::timer_frequency_hz() as u32;
    if hz == 0 {
        ticks
    } else {
        ((ticks as u64 * 1000) / hz as u64) as u32
    }
}

fn io_lock_record_acquired(
    start: u32,
    owner_at_start: u32,
    owner_op_at_start: u32,
    owner_disk_at_start: u32,
    owner_lba_at_start: u32,
    owner_count_at_start: u32,
    op: IoLockOp,
) {
    let now = crate::arch::hal::timer_current_ticks();
    let waited_ms = ticks_to_ms(now.wrapping_sub(start));
    let tid = crate::task::scheduler::current_tid();
    IO_LOCK_OP_KIND.store(op.kind, Ordering::Relaxed);
    IO_LOCK_OP_DISK.store(op.disk_id as u32, Ordering::Relaxed);
    IO_LOCK_OP_LBA.store(op.lba, Ordering::Relaxed);
    IO_LOCK_OP_COUNT.store(op.count, Ordering::Relaxed);
    IO_LOCK_OWNER_TID.store(tid, Ordering::Relaxed);
    IO_LOCK_ACQUIRED_TICK.store(now, Ordering::Relaxed);
    if waited_ms >= IO_LOCK_WAIT_WARN_MS
        && IO_LOCK_WAIT_LOGS.fetch_add(1, Ordering::Relaxed) < IO_LOCK_LOG_LIMIT
    {
        crate::serial_println!(
            "[storage] IO_LOCK wait {} ms tid={} want={} disk={} lba={} count={} owner_at_start={} owner_op={} owner_disk={} owner_lba={} owner_count={}",
            waited_ms,
            tid,
            io_op_name(op.kind),
            op.disk_id,
            op.lba,
            op.count,
            owner_at_start,
            io_op_name(owner_op_at_start),
            owner_disk_at_start,
            owner_lba_at_start,
            owner_count_at_start
        );
    }
}

#[inline]
fn io_lock_release() {
    let now = crate::arch::hal::timer_current_ticks();
    let acquired = IO_LOCK_ACQUIRED_TICK.load(Ordering::Relaxed);
    let held_ms = ticks_to_ms(now.wrapping_sub(acquired));
    let owner = IO_LOCK_OWNER_TID.swap(0, Ordering::Relaxed);
    let op_kind = IO_LOCK_OP_KIND.swap(IO_OP_UNKNOWN, Ordering::Relaxed);
    let op_disk = IO_LOCK_OP_DISK.swap(0, Ordering::Relaxed);
    let op_lba = IO_LOCK_OP_LBA.swap(0, Ordering::Relaxed);
    let op_count = IO_LOCK_OP_COUNT.swap(0, Ordering::Relaxed);
    IO_LOCK.store(false, Ordering::Release);
    if held_ms >= IO_LOCK_HOLD_WARN_MS
        && IO_LOCK_HOLD_LOGS.fetch_add(1, Ordering::Relaxed) < IO_LOCK_LOG_LIMIT
    {
        crate::serial_println!(
            "[storage] IO_LOCK held {} ms owner_tid={} op={} disk={} lba={} count={}",
            held_ms,
            owner,
            io_op_name(op_kind),
            op_disk,
            op_lba,
            op_count
        );
    }
}

fn io_lock_required(disk_id: u8, op_kind: u32) -> bool {
    if disk_id != 0 {
        return true;
    }
    if !matches!(
        op_kind,
        IO_OP_READ | IO_OP_READAHEAD | IO_OP_WRITE | IO_OP_WRITEBACK
    ) {
        return true;
    }
    match unsafe { BACKEND } {
        StorageBackend::Ahci => !ahci::supports_parallel_io(),
        _ => true,
    }
}

#[inline]
fn locked_io_batch_sectors(op_kind: u32) -> u32 {
    if op_kind == IO_OP_WRITEBACK {
        MAX_LOCKED_WRITEBACK_SECTORS
    } else if op_kind == IO_OP_READAHEAD {
        MAX_LOCKED_READAHEAD_SECTORS
    } else if op_kind == IO_OP_WRITE {
        MAX_LOCKED_WRITE_SECTORS
    } else {
        MAX_LOCKED_IO_SECTORS
    }
}

#[inline]
fn schedule_between_io_chunks() {
    if crate::task::scheduler::current_tid() > 0 {
        crate::task::scheduler::schedule();
    }
}

fn read_sectors_raw_chunked(
    disk_id: u8,
    lba: u32,
    count: u32,
    buf: &mut [u8],
    op_kind: u32,
) -> bool {
    let mut done = 0u32;
    let use_lock = io_lock_required(disk_id, op_kind);
    let max_batch = if use_lock {
        locked_io_batch_sectors(op_kind)
    } else {
        MAX_UNLOCKED_AHCI_IO_SECTORS
    };
    while done < count {
        let batch = (count - done).min(max_batch);
        let offset = done as usize * 512;
        let len = batch as usize * 512;
        let ok = if use_lock {
            io_lock_acquire(IoLockOp::new(op_kind, disk_id, lba + done, batch));
            let ok = read_sectors_raw_for_disk(
                disk_id,
                lba + done,
                batch,
                &mut buf[offset..offset + len],
            );
            io_lock_release();
            ok
        } else {
            read_sectors_raw_for_disk(disk_id, lba + done, batch, &mut buf[offset..offset + len])
        };
        if !ok {
            return false;
        }
        done += batch;
        if done < count {
            schedule_between_io_chunks();
        }
    }
    true
}

fn write_sectors_raw_chunked(disk_id: u8, lba: u32, count: u32, buf: &[u8], op_kind: u32) -> bool {
    let mut done = 0u32;
    let use_lock = io_lock_required(disk_id, op_kind);
    let max_batch = if use_lock {
        locked_io_batch_sectors(op_kind)
    } else {
        MAX_UNLOCKED_AHCI_IO_SECTORS
    };
    while done < count {
        let batch = (count - done).min(max_batch);
        let offset = done as usize * 512;
        let len = batch as usize * 512;
        let ok = if use_lock {
            io_lock_acquire(IoLockOp::new(op_kind, disk_id, lba + done, batch));
            let ok =
                write_sectors_raw_for_disk(disk_id, lba + done, batch, &buf[offset..offset + len]);
            io_lock_release();
            ok
        } else {
            write_sectors_raw_for_disk(disk_id, lba + done, batch, &buf[offset..offset + len])
        };
        if !ok {
            return false;
        }
        done += batch;
        if done < count {
            schedule_between_io_chunks();
        }
    }
    true
}

/// Switch the active storage backend to AHCI (called after AHCI init succeeds).
pub fn set_backend_ahci() {
    unsafe {
        BACKEND = StorageBackend::Ahci;
    }
}

/// Switch the active storage backend to NVMe (called after NVMe init succeeds).
pub fn set_backend_nvme() {
    unsafe {
        BACKEND = StorageBackend::Nvme;
    }
}

/// Switch the active storage backend to LSI Logic SCSI.
pub fn set_backend_lsi() {
    unsafe {
        BACKEND = StorageBackend::LsiScsi;
    }
}

/// Read `count` sectors starting at `lba` into `buf`.
///
/// First checks the global block cache — sectors already in RAM are served
/// directly without touching the disk.  Cache misses are read from the backend
/// and then populated into the cache for future reads.
///
/// For sequential access patterns, extra sectors are read ahead (up to 128
/// sectors / 64 KiB) to amortize disk latency.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    read_sectors_on_disk(0, lba, count, buf)
}

/// Read `count` sectors from a specific physical disk into `buf`.
///
/// Disk 0 uses the active global backend. Other disks use registered per-device
/// overrides when available. The block cache and adaptive readahead are keyed
/// by `disk_id` so mounted secondary volumes benefit from the same fast path.
pub fn read_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if count == 0 {
        return true;
    }
    if !check_backend_io_bounds(disk_id, lba, count, "read") {
        return false;
    }
    IO_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);

    // ── Check if block cache is available ──────────────────────────────
    let cache_active = crate::fs::blockcache::is_ready();

    // ── Fast path: try to serve entirely from block cache ──────────────
    let cached = if cache_active {
        crate::fs::blockcache::cached_read(disk_id, lba, count, buf)
    } else {
        0
    };
    if cached == count {
        return true;
    }

    // ── Partial or full miss: read remaining sectors from disk ─────────
    let miss_lba = lba + cached;
    let miss_count = count - cached;
    let miss_offset = cached as usize * 512;

    // Adaptive readahead (only when cache is active)
    let readahead = if cache_active && miss_count <= READAHEAD_TRIGGER_MAX {
        let last_end = LAST_READ_END_LBA.load(Ordering::Relaxed);
        let is_sequential = miss_lba == last_end || (miss_lba > 0 && miss_lba <= last_end + 8);
        if is_sequential {
            let level = READAHEAD_LEVEL.load(Ordering::Relaxed);
            let new_level = (level * 2).min(READAHEAD_MAX);
            READAHEAD_LEVEL.store(new_level, Ordering::Relaxed);
            new_level
        } else {
            READAHEAD_LEVEL.store(READAHEAD_MIN, Ordering::Relaxed);
            READAHEAD_MIN
        }
    } else {
        0
    };

    // Clamp readahead so the fetch never extends past the disk's last sector.
    // Ohne diesen Clamp versucht der Treiber am Disk-Ende einen Read über die
    // Kapazität hinaus — QEMU/AHCI hängt dann 5 s im Kommando-Timeout, was
    // z.B. `df` auf CoreFS-Mounts massiv verlangsamt (Secondary-Superblock
    // liegt genau im letzten Block des Volumes). Bei unbekannter Kapazität
    // (disk_sector_count == 0) lassen wir den Readahead unverändert.
    let readahead = {
        let disk_sectors = disk_sector_count(disk_id);
        if disk_sectors > 0 {
            let end = (miss_lba as u64).saturating_add(miss_count as u64);
            if end >= disk_sectors {
                0
            } else {
                let room = (disk_sectors - end).min(u32::MAX as u64) as u32;
                readahead.min(room)
            }
        } else {
            readahead
        }
    };

    let total_fetch = miss_count + readahead;
    let fetch_bytes = total_fetch as usize * 512;
    let populate_after_read = miss_count <= READ_CACHE_POPULATE_MAX;
    if cache_active {
        LAST_READ_END_LBA.store(miss_lba + total_fetch, Ordering::Relaxed);
    }

    // Prefer a pool buffer to avoid a heap allocation per read (see
    // READAHEAD_POOL comment). Fall back to Vec when all slots are in use or
    // the adaptive fetch is larger than one fixed pool slot.
    let result = if readahead > 0 {
        if fetch_bytes <= READAHEAD_BUF_BYTES {
            if let Some(lease) = acquire_readahead_slot() {
                let big_buf = readahead_slot_bytes(&lease, fetch_bytes);
                let ok = read_sectors_raw_chunked(
                    disk_id,
                    miss_lba,
                    total_fetch,
                    big_buf,
                    IO_OP_READAHEAD,
                );
                if ok {
                    crate::fs::blockcache::overlay_cached(disk_id, miss_lba, total_fetch, big_buf);
                    let needed = miss_count as usize * 512;
                    let copy_end = needed.min(buf.len() - miss_offset);
                    buf[miss_offset..miss_offset + copy_end].copy_from_slice(&big_buf[..copy_end]);
                    if populate_after_read {
                        crate::fs::blockcache::populate(
                            disk_id,
                            miss_lba,
                            total_fetch,
                            &big_buf[..fetch_bytes],
                        );
                    }
                    true
                } else {
                    false
                }
            } else {
                let mut big_buf = alloc::vec![0u8; fetch_bytes];
                let ok = read_sectors_raw_chunked(
                    disk_id,
                    miss_lba,
                    total_fetch,
                    &mut big_buf,
                    IO_OP_READAHEAD,
                );
                if ok {
                    crate::fs::blockcache::overlay_cached(
                        disk_id,
                        miss_lba,
                        total_fetch,
                        &mut big_buf,
                    );
                    let needed = miss_count as usize * 512;
                    let copy_end = needed.min(buf.len() - miss_offset);
                    buf[miss_offset..miss_offset + copy_end].copy_from_slice(&big_buf[..copy_end]);
                    if populate_after_read {
                        crate::fs::blockcache::populate(disk_id, miss_lba, total_fetch, &big_buf);
                    }
                    true
                } else {
                    false
                }
            }
        } else {
            let mut big_buf = alloc::vec![0u8; fetch_bytes];
            let ok = read_sectors_raw_chunked(
                disk_id,
                miss_lba,
                total_fetch,
                &mut big_buf,
                IO_OP_READAHEAD,
            );
            if ok {
                crate::fs::blockcache::overlay_cached(disk_id, miss_lba, total_fetch, &mut big_buf);
                let needed = miss_count as usize * 512;
                let copy_end = needed.min(buf.len() - miss_offset);
                buf[miss_offset..miss_offset + copy_end].copy_from_slice(&big_buf[..copy_end]);
                if populate_after_read {
                    crate::fs::blockcache::populate(disk_id, miss_lba, total_fetch, &big_buf);
                }
                true
            } else {
                false
            }
        }
    } else {
        // No readahead: read directly into caller buffer
        let ok = read_sectors_raw_chunked(
            disk_id,
            miss_lba,
            miss_count,
            &mut buf[miss_offset..],
            IO_OP_READ,
        );
        if ok && cache_active {
            let fetched_bytes = miss_count as usize * 512;
            if buf.len() >= miss_offset + fetched_bytes {
                crate::fs::blockcache::overlay_cached(
                    disk_id,
                    miss_lba,
                    miss_count,
                    &mut buf[miss_offset..miss_offset + fetched_bytes],
                );
            }
            // Populate cache with small/random reads only. Large streaming reads
            // otherwise evict useful metadata and pay one cache insert per sector.
            if populate_after_read && buf.len() >= miss_offset + fetched_bytes {
                crate::fs::blockcache::populate(
                    disk_id,
                    miss_lba,
                    miss_count,
                    &buf[miss_offset..miss_offset + fetched_bytes],
                );
            }
        }
        ok
    };

    result
}

/// Raw read without cache — dispatches to the active backend.
fn read_sectors_raw(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    read_sectors_raw_for_disk(0, lba, count, buf)
}

fn read_sectors_raw_for_disk(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if !check_backend_io_bounds(disk_id, lba, count, "direct read") {
        return false;
    }
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.read_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::read_sectors(cur_lba, batch, &mut buf[offset..]) {
                    return false;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            true
        }
        StorageBackend::Ahci => ahci::read_sectors(lba, count, buf),
        StorageBackend::Nvme => nvme::read_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::read_sectors(lba, count, buf),
    }
}

/// Write `count` sectors starting at `lba` from `buf`.
///
/// Uses write-back caching: data is stored in the block cache as dirty sectors
/// and written to disk lazily during writeback. This makes writes extremely fast
/// (RAM speed) while maintaining read coherency.
///
/// Bulk writes go directly to disk.  The write-back cache is intentionally
/// reserved for single-cluster/metadata-sized writes; 16 KiB+ streaming writes
/// otherwise spend too much time inserting sector-sized dirty cache entries
/// only to flush them as data shortly afterwards.
const WRITE_BACK_MAX_SECTORS: u32 = 8;

pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_on_disk(0, lba, count, buf)
}

/// Write `count` sectors to a specific physical disk from `buf`.
pub fn write_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if count == 0 {
        return true;
    }
    if !check_backend_io_bounds(disk_id, lba, count, "write") {
        return false;
    }
    if crate::fs::blockcache::is_ready()
        && !crate::fs::blockcache::write_range_allows(disk_id, lba, count)
    {
        crate::serial_println!(
            "[storage] refusing write outside system partition: disk={} lba={} count={}",
            disk_id,
            lba,
            count
        );
        return false;
    }
    IO_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);

    let cache_active = crate::fs::blockcache::is_ready();
    let data_len = count as usize * 512;

    // Write-back path: small writes go to cache, bulk writes bypass.
    if cache_active && count <= WRITE_BACK_MAX_SECTORS && buf.len() >= data_len {
        if crate::fs::blockcache::should_flush_before_write_back(disk_id, count) {
            crate::fs::blockcache::writeback_flush(disk_id);
        }
        crate::fs::blockcache::write_back(disk_id, lba, count, &buf[..data_len]);
        return true;
    }

    // Large write or no cache: go directly to disk
    let result = write_sectors_raw_chunked(disk_id, lba, count, buf, IO_OP_WRITE);
    if result && cache_active {
        // Direct/bulk writes already reached the backend. Keeping a clean copy
        // of every streamed sector in the read cache turns large writes into
        // thousands of cache insertions and LRU scans. Drop any stale entries
        // instead; the next read can fetch the data from disk or from the
        // filesystem's own higher-level cache.
        crate::fs::blockcache::invalidate(disk_id, lba, count);
    }
    result
}

/// Raw write without cache — dispatches to the active backend.
fn write_sectors_raw(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_raw_for_disk(0, lba, count, buf)
}

fn write_sectors_raw_for_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if !check_backend_io_bounds(disk_id, lba, count, "direct write") {
        return false;
    }
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.write_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::write_sectors(cur_lba, batch, &buf[offset..]) {
                    return false;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            true
        }
        StorageBackend::Ahci => ahci::write_sectors(lba, count, buf),
        StorageBackend::Nvme => nvme::write_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::write_sectors(lba, count, buf),
    }
}

/// Write sectors directly to disk, bypassing the block cache.
/// Used by the cache writeback mechanism itself to avoid recursion.
pub fn write_sectors_direct(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_direct_on_disk(0, lba, count, buf)
}

/// Write sectors directly to a specific disk, bypassing the block cache.
pub fn write_sectors_direct_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if count == 0 {
        return true;
    }
    if !check_backend_io_bounds(disk_id, lba, count, "writeback") {
        return false;
    }
    if crate::fs::blockcache::is_ready()
        && !crate::fs::blockcache::write_range_allows(disk_id, lba, count)
    {
        crate::serial_println!(
            "[storage] refusing direct write outside system partition: disk={} lba={} count={}",
            disk_id,
            lba,
            count
        );
        return false;
    }
    let result = write_sectors_raw_chunked(disk_id, lba, count, buf, IO_OP_WRITEBACK);
    result
}

/// Flush storage write cache to persistent media.
pub fn flush() {
    flush_disk(0);
}

/// Flush a specific disk's hardware write cache to persistent media when the
/// active backend supports it.
pub fn flush_disk(disk_id: u8) {
    if !matches!(unsafe { BACKEND }, StorageBackend::Ahci) {
        return;
    }
    let bit = if disk_id < 32 { 1u32 << disk_id } else { 0 };
    if bit != 0 && (AHCI_FLUSH_FAILED_MASK.load(Ordering::Relaxed) & bit) != 0 {
        return;
    }
    io_lock_acquire(IoLockOp::new(IO_OP_FLUSH, disk_id, 0, 0));
    let ok = ahci::flush_disk(disk_id);
    io_lock_release();
    if !ok && bit != 0 {
        AHCI_FLUSH_FAILED_MASK.fetch_or(bit, Ordering::Relaxed);
        crate::serial_println!(
            "[storage] AHCI flush disabled after timeout/failure for disk={}",
            disk_id
        );
    }
}

// ── HAL integration ─────────────────────────────────────────────────────────

use crate::drivers::hal::{Driver, DriverError, DriverType};
use alloc::boxed::Box;

struct StorageHalDriver {
    name: &'static str,
}

impl Driver for StorageHalDriver {
    fn name(&self) -> &str {
        self.name
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Block
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let lba = (offset / 512) as u32;
        let count = ((buf.len() + 511) / 512) as u32;
        if read_sectors(lba, count, buf) {
            Ok(count as usize * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        let lba = (offset / 512) as u32;
        let count = ((buf.len() + 511) / 512) as u32;
        if write_sectors(lba, count, buf) {
            Ok(count as usize * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Create a HAL Driver wrapper for the storage subsystem (called from driver probe).
pub(crate) fn create_hal_driver(name: &'static str) -> Option<Box<dyn Driver>> {
    Some(Box::new(StorageHalDriver { name }))
}

/// Probe for IDE controller (uses default ATA PIO backend).
pub fn ide_probe(_pci: &crate::drivers::pci::PciDevice) -> Option<Box<dyn Driver>> {
    create_hal_driver("IDE Controller")
}
