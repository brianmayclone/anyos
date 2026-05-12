use anyos_std::process;
use core::sync::atomic::Ordering;
use liblxecore::config::LxeConfig;
use liblxecore::manager::{self, ManagerProgress, ManagerProgressSink};

use crate::state::*;

pub fn spawn(op: Operation) {
    reset_worker(op);
    if let Ok(thread) = process::Thread::spawn_with_stack(worker_entry, 384 * 1024, "lxemanager") {
        core::mem::forget(thread);
    } else {
        fail("Could not start LXE Manager worker thread.");
    }
}

fn worker_entry() {
    let mut config = LxeConfig::load();
    let op = Operation::from_u32(WORKER_OP.load(Ordering::Acquire));
    let mut sink = SharedSink;
    let result = match op {
        Operation::Install => manager::install_or_repair(&mut config, &mut sink),
        Operation::Repair => manager::repair_only(&mut config, &mut sink),
        Operation::Delete => manager::delete_rootfs(&mut config, &mut sink),
    };
    if let Err(err) = result {
        fail(&err);
        return;
    }
    WORKER_ACTIVE.store(false, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

fn fail(message: &str) {
    write_error(message);
    WORKER_ERROR.store(true, Ordering::Release);
    WORKER_ACTIVE.store(false, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

struct SharedSink;

impl ManagerProgressSink for SharedSink {
    fn update(&mut self, progress: &ManagerProgress) {
        PROGRESS_PERCENT.store(progress.percent, Ordering::Release);
        PROGRESS_STEP.store(progress.step, Ordering::Release);
        PROGRESS_STEPS.store(progress.steps, Ordering::Release);
        PROGRESS_BYTES_DONE.store(progress.bytes_done, Ordering::Release);
        PROGRESS_BYTES_TOTAL.store(progress.bytes_total, Ordering::Release);
        PROGRESS_ETA.store(progress.eta_secs, Ordering::Release);
        write_progress_text(&progress.phase, &progress.detail);
        PROGRESS_SEQ.fetch_add(1, Ordering::Release);
    }
}
