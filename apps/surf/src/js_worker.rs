// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Background JavaScript worker for Surf.
//!
//! Large scripts must not run on the UI thread. The UI detaches the DOM/JS
//! runtime bundle from a tab, submits it here, and later reattaches the result.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as ui_lib;

pub(crate) enum JsWorkerJob {
    Script {
        slot: usize,
        label: String,
        script_label: String,
        script: String,
    },
    Timer {
        delta_ms: u64,
        callback_budget: usize,
    },
}

pub(crate) struct JsWorkerRequest {
    pub(crate) tab_index: usize,
    pub(crate) job: JsWorkerJob,
    pub(crate) state: libwebview::JsExecutionState,
    pub(crate) generation: u32,
}

pub(crate) enum JsWorkerResultKind {
    Script {
        slot: usize,
        label: String,
        script_label: String,
    },
    Timer {
        fired: usize,
    },
}

pub(crate) struct JsWorkerResult {
    pub(crate) tab_index: usize,
    pub(crate) kind: JsWorkerResultKind,
    pub(crate) state: libwebview::JsExecutionState,
    pub(crate) exec_ms: u32,
    pub(crate) generation: u32,
}

static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
static RESULT_LOCK: AtomicBool = AtomicBool::new(false);
static RESULT_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE_WORKERS: AtomicU32 = AtomicU32::new(0);
static BUSY_WORKERS: AtomicU32 = AtomicU32::new(0);

static mut REQUEST_QUEUE: Option<VecDeque<JsWorkerRequest>> = None;
static mut RESULT_QUEUE: Option<Vec<JsWorkerResult>> = None;

fn acquire(lock: &AtomicBool) {
    while lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn release(lock: &AtomicBool) {
    lock.store(false, Ordering::Release);
}

pub(crate) fn init() {
    acquire(&REQUEST_LOCK);
    acquire(&RESULT_LOCK);
    unsafe {
        REQUEST_QUEUE = Some(VecDeque::new());
        RESULT_QUEUE = Some(Vec::new());
    }
    release(&RESULT_LOCK);
    release(&REQUEST_LOCK);
}

extern "C" fn result_ready_cb(_userdata: u64) {
    RESULT_NOTIFY_PENDING.store(false, Ordering::Release);
    crate::handle_js_worker_results_ready();
}

fn notify_result_ready() {
    if RESULT_NOTIFY_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        ui_lib::marshal_dispatch(result_ready_cb, 0);
    }
}

pub(crate) fn submit(req: JsWorkerRequest) {
    acquire(&REQUEST_LOCK);
    unsafe {
        if let Some(queue) = REQUEST_QUEUE.as_mut() {
            queue.push_back(req);
        }
    }
    release(&REQUEST_LOCK);
    ensure_worker();
}

pub(crate) fn drain_results() -> Vec<JsWorkerResult> {
    acquire(&RESULT_LOCK);
    let results = unsafe {
        RESULT_QUEUE
            .as_mut()
            .map(|queue| core::mem::replace(queue, Vec::new()))
            .unwrap_or_default()
    };
    release(&RESULT_LOCK);
    results
}

pub(crate) fn has_pending_activity() -> bool {
    if BUSY_WORKERS.load(Ordering::Relaxed) > 0 {
        return true;
    }
    acquire(&REQUEST_LOCK);
    let request_pending = unsafe { REQUEST_QUEUE.as_ref().is_some_and(|q| !q.is_empty()) };
    release(&REQUEST_LOCK);
    if request_pending {
        return true;
    }
    acquire(&RESULT_LOCK);
    let result_pending = unsafe { RESULT_QUEUE.as_ref().is_some_and(|q| !q.is_empty()) };
    release(&RESULT_LOCK);
    result_pending
}

fn ensure_worker() {
    if ACTIVE_WORKERS
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    match anyos_std::process::Thread::spawn_with_stack(worker_entry, 1024 * 1024, "surf-js") {
        Ok(handle) => {
            core::mem::forget(handle);
            anyos_std::println!(
                "[surf-js-worker {:>8}ms] worker thread started",
                anyos_std::sys::uptime_ms()
            );
        }
        Err(_) => {
            ACTIVE_WORKERS.store(0, Ordering::SeqCst);
            anyos_std::println!(
                "[surf-js-worker {:>8}ms] ERROR: failed to spawn worker thread",
                anyos_std::sys::uptime_ms()
            );
        }
    }
}

fn take_request() -> Option<JsWorkerRequest> {
    acquire(&REQUEST_LOCK);
    let req = unsafe {
        REQUEST_QUEUE.as_mut().and_then(|queue| {
            if queue.is_empty() {
                None
            } else {
                queue.pop_front()
            }
        })
    };
    release(&REQUEST_LOCK);
    req
}

fn request_pending() -> bool {
    acquire(&REQUEST_LOCK);
    let pending = unsafe { REQUEST_QUEUE.as_ref().is_some_and(|q| !q.is_empty()) };
    release(&REQUEST_LOCK);
    pending
}

fn push_result(result: JsWorkerResult) {
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(queue) = RESULT_QUEUE.as_mut() {
            queue.push(result);
        }
    }
    release(&RESULT_LOCK);
    notify_result_ready();
}

fn worker_entry() {
    let mut idle_loops = 0u32;
    loop {
        let Some(mut req) = take_request() else {
            idle_loops += 1;
            if idle_loops >= 100 {
                break;
            }
            anyos_std::process::sleep(5);
            continue;
        };
        idle_loops = 0;
        BUSY_WORKERS.fetch_add(1, Ordering::SeqCst);
        let start_ms = anyos_std::sys::uptime_ms();
        let kind = match req.job {
            JsWorkerJob::Script {
                slot,
                label,
                script_label,
                script,
            } => {
                req.state.execute_script_source(script);
                JsWorkerResultKind::Script {
                    slot,
                    label,
                    script_label,
                }
            }
            JsWorkerJob::Timer {
                delta_ms,
                callback_budget,
            } => {
                let fired = req.state.run_timers_with_budget(delta_ms, callback_budget);
                JsWorkerResultKind::Timer { fired }
            }
        };
        let exec_ms = anyos_std::sys::uptime_ms().wrapping_sub(start_ms);
        BUSY_WORKERS.fetch_sub(1, Ordering::SeqCst);
        push_result(JsWorkerResult {
            tab_index: req.tab_index,
            kind,
            state: req.state,
            exec_ms,
            generation: req.generation,
        });
    }
    ACTIVE_WORKERS.store(0, Ordering::SeqCst);
    if request_pending() {
        ensure_worker();
    }
    anyos_std::process::exit(0);
}
