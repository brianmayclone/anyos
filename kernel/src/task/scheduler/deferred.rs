//! Deferred page-directory destruction queue.
//!
//! `kill_thread` must not call `destroy_user_page_directory` while holding
//! the scheduler lock — page-table walks and hundreds of `free_frame` calls
//! take milliseconds, causing SPIN TIMEOUT on other CPUs.

use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;

/// Deferred page-directory destruction queue.
///
/// ## tid semantics
/// * `tid != 0`: thread was still running on another CPU at kill time;
///   `cleanup_process(tid)` must run with the dying CR3 before destroy.
/// * `tid == 0`: `cleanup_process` already ran in `kill_thread`; just destroy.
pub(super) struct DeferredPdQueue {
    entries: [Option<(PhysAddr, u32)>; 64],
}

impl DeferredPdQueue {
    pub(super) const fn new() -> Self { Self { entries: [None; 64] } }

    pub(super) fn push(&mut self, pd: PhysAddr, tid: u32) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() { *slot = Some((pd, tid)); return; }
        }
        // Queue full (64 pending PDs) — drain one slot synchronously.
        // This is a last-resort fallback for pathological fork storms.
        crate::serial_println!("WARNING: deferred PD queue full, destroying one synchronously");
        if let Some(Some((old_pd, old_tid))) = self.entries.iter_mut().find(|s| s.is_some()).map(|s| s.take()) {
            if old_tid != 0 {
                let rflags = crate::arch::hal::save_and_disable_interrupts();
                let saved_cr3 = crate::arch::hal::current_page_table();
                crate::arch::hal::switch_page_table(old_pd.as_u64());
                crate::ipc::shared_memory::cleanup_process(old_tid);
                crate::arch::hal::switch_page_table(saved_cr3);
                crate::arch::hal::restore_interrupt_state(rflags);
            }
            crate::memory::virtual_mem::destroy_user_page_directory(old_pd);
            crate::memory::vma::destroy_process(old_pd);
        }
        // Now there is a free slot — insert the new entry.
        for slot in self.entries.iter_mut() {
            if slot.is_none() { *slot = Some((pd, tid)); return; }
        }
    }

    pub(super) fn drain(&mut self) -> [Option<(PhysAddr, u32)>; 64] {
        let result = self.entries;
        self.entries = [None; 64];
        result
    }
}

pub(super) static DEFERRED_PD_DESTROY: Spinlock<DeferredPdQueue> = Spinlock::new(DeferredPdQueue::new());
