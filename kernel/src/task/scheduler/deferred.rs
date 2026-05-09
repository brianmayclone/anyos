//! Deferred fault/teardown queues.
//!
//! `kill_thread` must not call `destroy_user_page_directory` while holding
//! the scheduler lock — page-table walks and hundreds of `free_frame` calls
//! take milliseconds, causing SPIN TIMEOUT on other CPUs.

use crate::fs::fd_table::FdKind;
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
    pub(super) const fn new() -> Self {
        Self {
            entries: [None; 64],
        }
    }

    pub(super) fn push(&mut self, pd: PhysAddr, tid: u32) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some((pd, tid));
                return;
            }
        }
        // Queue full (64 pending PDs) — drain one slot synchronously.
        // This is a last-resort fallback for pathological fork storms.
        crate::serial_verbose_println!(
            "WARNING: deferred PD queue full, destroying one synchronously"
        );
        if let Some(Some((old_pd, old_tid))) = self
            .entries
            .iter_mut()
            .find(|s| s.is_some())
            .map(|s| s.take())
        {
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
            if slot.is_none() {
                *slot = Some((pd, tid));
                return;
            }
        }
    }

    pub(super) fn drain(&mut self) -> [Option<(PhysAddr, u32)>; 64] {
        let result = self.entries;
        self.entries = [None; 64];
        result
    }
}

pub(super) static DEFERRED_PD_DESTROY: Spinlock<DeferredPdQueue> =
    Spinlock::new(DeferredPdQueue::new());

/// Deferred thread resource cleanup requested by fault-exit paths.
///
/// Fault recovery must not synchronously close files, tear down TCP state, or
/// emit system-bus events while still unwinding out of a broken exception
/// context. Those operations take additional global locks and were a recurring
/// source of secondary deadlocks/crashes after the original userspace fault.
#[derive(Clone, Copy)]
pub(super) struct DeferredThreadCleanup {
    pub tid: u32,
    pub exit_code: u32,
    pub emit_exit_event: bool,
}

pub(super) struct DeferredThreadCleanupQueue {
    entries: [Option<DeferredThreadCleanup>; 128],
    next_overwrite: usize,
}

impl DeferredThreadCleanupQueue {
    pub(super) const fn new() -> Self {
        Self {
            entries: [None; 128],
            next_overwrite: 0,
        }
    }

    pub(super) fn push(&mut self, entry: DeferredThreadCleanup) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some(entry);
                return;
            }
        }
        crate::serial_verbose_println!(
            "WARNING: deferred thread cleanup queue full, overwriting oldest entry"
        );
        self.entries[self.next_overwrite] = Some(entry);
        self.next_overwrite = (self.next_overwrite + 1) % self.entries.len();
    }

    pub(super) fn drain(&mut self) -> [Option<DeferredThreadCleanup>; 128] {
        let result = self.entries;
        self.entries = [None; 128];
        self.next_overwrite = 0;
        result
    }
}

pub(super) static DEFERRED_THREAD_CLEANUP: Spinlock<DeferredThreadCleanupQueue> =
    Spinlock::new(DeferredThreadCleanupQueue::new());

pub(super) fn process_deferred_thread_cleanup(entry: DeferredThreadCleanup) {
    let closed = super::close_all_fds_for_thread(entry.tid);
    for kind in closed.iter() {
        match kind {
            FdKind::File { global_id } => crate::fs::vfs::decref(*global_id),
            FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(*pipe_id),
            FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(*pipe_id),
            FdKind::Tty | FdKind::LinuxProc { .. } | FdKind::None => {}
        }
    }

    crate::net::tcp::cleanup_for_thread(entry.tid);
    crate::net::udp::cleanup_for_thread(entry.tid);
    crate::drivers::audio::mixer::close_channels_for_pid(entry.tid);

    if entry.emit_exit_event {
        crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
            crate::ipc::event_bus::EVT_PROCESS_EXITED,
            entry.tid,
            entry.exit_code,
            0,
            0,
        ));
    }
}
