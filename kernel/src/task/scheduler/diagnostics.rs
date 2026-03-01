//! Thread info / diagnostics, lock management, and I/O accounting.

use super::{get_cpu_id, SCHEDULER};
use crate::task::thread::ThreadState;
use alloc::vec::Vec;

/// Snapshot of a thread's state for the `ps` / sysinfo syscall.
pub struct ThreadInfo {
    pub tid: u32,
    pub priority: u8,
    pub state: &'static str,
    pub name: alloc::string::String,
    pub cpu_ticks: u32,
    pub arch_mode: u8,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    pub user_pages: u32,
    pub uid: u16,
}

/// List all live threads (lock-free heap allocation pattern).
pub fn list_threads() -> Vec<ThreadInfo> {
    const MAX_SNAP: usize = 64;
    struct ThreadSnap {
        tid: u32, priority: u8, state: u8, arch_mode: u8,
        cpu_ticks: u32, io_read_bytes: u64, io_write_bytes: u64,
        user_pages: u32, name: [u8; 32], name_len: u8, uid: u16,
    }
    let mut buf = [const {
        ThreadSnap { tid: 0, priority: 0, state: 0, arch_mode: 0, cpu_ticks: 0,
            io_read_bytes: 0, io_write_bytes: 0, user_pages: 0, name: [0; 32], name_len: 0, uid: 0 }
    }; MAX_SNAP];
    let mut count = 0;

    {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            let online_cpus = crate::arch::hal::cpu_count();
            for thread in &sched.threads {
                if thread.state == ThreadState::Terminated { continue; }
                if thread.is_idle && !sched.idle_tid[..online_cpus].contains(&thread.tid) { continue; }
                if count >= MAX_SNAP { break; }
                let state_num = match thread.state {
                    ThreadState::Ready => 0u8,
                    ThreadState::Running => 1,
                    ThreadState::Blocked => 2,
                    ThreadState::Terminated => unreachable!(),
                };
                let name_str = thread.name_str();
                let len = name_str.len().min(32);
                let mut name_buf = [0u8; 32];
                name_buf[..len].copy_from_slice(&name_str.as_bytes()[..len]);
                buf[count] = ThreadSnap {
                    tid: thread.tid, priority: thread.priority, state: state_num,
                    arch_mode: thread.arch_mode as u8, cpu_ticks: thread.cpu_ticks,
                    io_read_bytes: thread.io_read_bytes, io_write_bytes: thread.io_write_bytes,
                    user_pages: thread.user_pages, name: name_buf, name_len: len as u8,
                    uid: thread.uid,
                };
                count += 1;
            }
        }
    }

    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let snap = &buf[i];
        result.push(ThreadInfo {
            tid: snap.tid,
            priority: snap.priority,
            state: match snap.state { 0 => "ready", 1 => "running", _ => "blocked" },
            name: alloc::string::String::from(
                core::str::from_utf8(&snap.name[..snap.name_len as usize]).unwrap_or("?")
            ),
            cpu_ticks: snap.cpu_ticks,
            arch_mode: snap.arch_mode,
            io_read_bytes: snap.io_read_bytes,
            io_write_bytes: snap.io_write_bytes,
            user_pages: snap.user_pages,
            uid: snap.uid,
        });
    }
    result
}

// =============================================================================
// Lock management
// =============================================================================

pub fn is_scheduler_locked_by_cpu(cpu: u32) -> bool { SCHEDULER.is_held_by_cpu(cpu) }
pub fn is_scheduler_locked() -> bool { SCHEDULER.is_locked() }

/// # Safety
/// Must only be called when `is_scheduler_locked_by_cpu(cpu)` returns true.
pub unsafe fn force_unlock_scheduler() {
    SCHEDULER.force_unlock();
    crate::serial_println!("  RECOVERED: force-released scheduler lock");
}

// =============================================================================
// I/O accounting
// =============================================================================

pub fn record_io_read(bytes: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].io_read_bytes += bytes;
        }
    }
}

pub fn record_io_write(bytes: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].io_write_bytes += bytes;
        }
    }
}

pub fn adjust_thread_user_pages(tid: u32, delta: i32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if delta >= 0 {
                thread.user_pages = thread.user_pages.saturating_add(delta as u32);
            } else {
                thread.user_pages = thread.user_pages.saturating_sub((-delta) as u32);
            }
        }
    }
}

pub fn adjust_current_user_pages(delta: i32) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            if delta >= 0 {
                sched.threads[idx].user_pages = sched.threads[idx].user_pages.saturating_add(delta as u32);
            } else {
                sched.threads[idx].user_pages = sched.threads[idx].user_pages.saturating_sub((-delta) as u32);
            }
        }
    }
}
