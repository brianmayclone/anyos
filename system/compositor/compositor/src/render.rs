//! Render thread and shared Desktop state management.
//!
//! The Desktop is allocated once on the heap and accessed via a raw pointer
//! behind a simple spinlock. The management thread (main) and render thread
//! share access through acquire_lock/release_lock.
//!
//! Rendering is event-driven: the render thread sleeps until signaled that
//! new work is available (damage, input, animations). This mirrors how
//! macOS (CVDisplayLink) and Windows (DWM) compositors work — render only
//! when there's something to display, not on a blind timer.

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

/// Signal flag: set by management thread when new damage is available.
/// The render thread checks this to decide whether to compose or sleep.
static RENDER_NEEDED: AtomicBool = AtomicBool::new(true);

/// Event channel ID for sending frame ACKs directly from the render thread.
/// Set once during init by the management thread, never changed.
static mut COMPOSITOR_CHANNEL: u32 = 0;

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

/// Store the compositor event channel ID so the render thread can emit frame ACKs.
pub unsafe fn set_compositor_channel(ch: u32) {
    COMPOSITOR_CHANNEL = ch;
}

/// Signal the render thread that new work is available (damage, input, etc.).
/// Called by the management thread after processing events or IPC commands.
pub fn signal_render() {
    RENDER_NEEDED.store(true, Ordering::Release);
}

// ── Render Thread ────────────────────────────────────────────────────────────

/// Render thread entry point — event-driven compositing at up to ~60 Hz.
///
/// Instead of polling at a fixed 16ms interval, the render thread:
/// 1. Checks if new work was signaled (RENDER_NEEDED flag)
/// 2. If yes: compose and pace to 60fps
/// 3. If no: sleep briefly and re-check (low CPU when idle)
///
/// Animations and clock are ticked here (not in the management thread) so they
/// stay smooth even when the management thread is busy with IPC / shm_map.
///
/// CRITICAL: Uses try_lock() — NEVER blocks on the management thread.
pub fn render_thread_entry() {
    println!("compositor: render thread running");
    let mut frame: u32 = 0;
    let mut idle_count: u32 = 0;

    loop {
        // Check if the management thread signaled new work
        let work_available = RENDER_NEEDED.swap(false, Ordering::Acquire);

        // Also compose periodically for clock updates (every ~1s)
        let periodic = frame % 60 == 0;

        if work_available || periodic {
            idle_count = 0;
            let t0 = sys::uptime_ms();

            if try_lock() {
                crate::desktop::theme::refresh_theme_cache();
                let desktop = unsafe { desktop_ref() };
                // Tick animations + clock before compositing so updated state is
                // reflected in the same frame — no extra lock round-trip needed.
                let has_animations = desktop.tick_animations();
                if periodic {
                    desktop.update_clock();
                }
                desktop.process_deferred_wallpaper();
                desktop.compose();

                // Emit frame ACKs immediately after VSync (compose + flush_gpu).
                // This is the VSync callback — apps learn their frame is on screen.
                let channel = unsafe { COMPOSITOR_CHANNEL };
                if channel != 0 && !desktop.frame_ack_queue.is_empty() {
                    for &(sub_id, window_id) in &desktop.frame_ack_queue {
                        anyos_std::ipc::evt_chan_emit_to(channel, sub_id, &[
                            crate::ipc_protocol::EVT_FRAME_ACK, window_id, 0, 0, 0,
                        ]);
                    }
                    desktop.frame_ack_queue.clear();
                }
                release_lock();

                // If animations are still active, keep rendering next frame
                if has_animations {
                    RENDER_NEEDED.store(true, Ordering::Release);
                }

                frame = frame.wrapping_add(1);
            } else {
                // Lock contended — management thread is doing work.
                // Sleep briefly and retry. Previous frame stays on screen.
                process::sleep(1);
                RENDER_NEEDED.store(true, Ordering::Release);
                continue;
            }

            // Frame pacing: sleep remainder to hit ~60fps max
            let elapsed = sys::uptime_ms().wrapping_sub(t0);
            let target = 16u32;
            if elapsed < target {
                process::sleep(target - elapsed);
            }
        } else {
            // No work signaled — idle sleep with adaptive interval.
            // Start at 2ms, ramp up to 16ms after sustained idle.
            idle_count = idle_count.saturating_add(1);
            let sleep_ms = if idle_count < 8 { 2 } else { 16 };
            process::sleep(sleep_ms);
            frame = frame.wrapping_add(1);
        }
    }
}
