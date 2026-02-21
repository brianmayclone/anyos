//! Render thread and shared Desktop state management.
//!
//! The Desktop is allocated once on the heap and accessed via a raw pointer
//! behind a simple spinlock. The management thread (main) and render thread
//! share access through acquire_lock/release_lock.

use anyos_std::process;
use anyos_std::sys;
use anyos_std::println;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::desktop::Desktop;

// ── Shared State ─────────────────────────────────────────────────────────────

/// Spinlock protecting access to the Desktop.
static DESKTOP_LOCK: AtomicBool = AtomicBool::new(false);

/// Raw pointer to the heap-allocated Desktop (set once during init, never freed).
static mut DESKTOP_PTR: *mut Desktop = core::ptr::null_mut();

pub fn acquire_lock() {
    while DESKTOP_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Try to acquire the lock without blocking. Returns true if acquired.
pub fn try_lock() -> bool {
    DESKTOP_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
}

pub fn release_lock() {
    DESKTOP_LOCK.store(false, Ordering::Release);
}

/// Get a mutable reference to the Desktop. Caller MUST hold DESKTOP_LOCK.
pub unsafe fn desktop_ref() -> &'static mut Desktop {
    &mut *DESKTOP_PTR
}

/// Store the Desktop pointer for shared access. Called once during init.
pub unsafe fn set_desktop_ptr(ptr: *mut Desktop) {
    DESKTOP_PTR = ptr;
}

// ── Render Thread ────────────────────────────────────────────────────────────

/// Render thread entry point — composites and flushes at ~60 Hz.
///
/// Animations and clock are ticked here (not in the management thread) so they
/// stay smooth even when the management thread is busy with IPC / shm_map.
///
/// CRITICAL: Uses try_lock() — NEVER blocks on the management thread.
/// If the lock is held (e.g. during window creation), the render thread
/// simply retries after a short sleep. The previous frame stays on-screen
/// so the user sees no glitch — just a held frame for 2ms instead of
/// a 200ms stall.
pub fn render_thread_entry() {
    println!("compositor: render thread running");
    let mut frame: u32 = 0;
    loop {
        let t0 = sys::uptime_ms();

        if try_lock() {
            let desktop = unsafe { desktop_ref() };
            // Tick animations + clock before compositing so updated state is
            // reflected in the same frame — no extra lock round-trip needed.
            desktop.tick_animations();
            if frame % 60 == 0 {
                desktop.update_clock();
            }
            // Process deferred wallpaper reload (after resolution change)
            desktop.process_deferred_wallpaper();
            desktop.compose();
            release_lock();

            frame = frame.wrapping_add(1);
            if frame % 120 == 0 {
                println!("compositor: render frame {}", frame);
            }
        } else {
            // Lock contended — management thread is doing work (e.g. window creation).
            // Don't block: sleep briefly and retry. Previous frame stays on screen.
            process::yield_cpu();
        }

        // Dynamic frame pacing: sleep only the remainder to hit ~60fps
        let elapsed = sys::uptime_ms().wrapping_sub(t0);
        let target = 16u32;
        if elapsed < target {
            process::sleep(target - elapsed);
        }
    }
}
