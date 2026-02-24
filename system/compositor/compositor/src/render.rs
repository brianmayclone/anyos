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
/// 3. If no: sleep with adaptive interval (up to 250ms) and re-check
///
/// The clock is checked during idle at ~1Hz. If the minute changed,
/// `signal_render()` triggers a compose for just the clock region — no
/// forced periodic compositing needed.
///
/// Animations and clock are ticked here (not in the management thread) so they
/// stay smooth even when the management thread is busy with IPC / shm_map.
///
/// CRITICAL: Uses try_lock() — NEVER blocks on the management thread.
pub fn render_thread_entry() {
    println!("compositor: render thread running");
    let mut idle_count: u32 = 0;
    let mut last_clock_check: u32 = 0;

    // ── Debug stats (reset every 5 seconds) ──
    let mut stat_wakeups: u32 = 0;       // times work_available was true
    let mut stat_damage: u32 = 0;        // times compose() had actual damage
    let mut stat_animations: u32 = 0;    // times has_animations was true
    let mut stat_no_damage: u32 = 0;     // times compose() had NO damage
    let mut stat_lock_fail: u32 = 0;     // times try_lock() failed
    let mut stat_idle_loops: u32 = 0;    // times we entered idle branch
    let mut stat_last_report: u32 = sys::uptime_ms();

    loop {
        // ── Periodic stats dump (every 5 seconds) ──
        let now_ms = sys::uptime_ms();
        if now_ms.wrapping_sub(stat_last_report) >= 5000 {
            println!(
                "GPU-STATS: wake={} dmg={} anim={} no_dmg={} lock_fail={} idle={}",
                stat_wakeups, stat_damage, stat_animations,
                stat_no_damage, stat_lock_fail, stat_idle_loops
            );
            stat_wakeups = 0;
            stat_damage = 0;
            stat_animations = 0;
            stat_no_damage = 0;
            stat_lock_fail = 0;
            stat_idle_loops = 0;
            stat_last_report = now_ms;
        }

        // Check if the management thread signaled new work
        let work_available = RENDER_NEEDED.swap(false, Ordering::Acquire);

        if work_available {
            stat_wakeups += 1;
            let t0 = sys::uptime_ms();

            if try_lock() {
                crate::desktop::theme::refresh_theme_cache();
                let desktop = unsafe { desktop_ref() };
                // Tick animations + clock before compositing so updated state is
                // reflected in the same frame — no extra lock round-trip needed.
                let has_animations = desktop.tick_animations();
                desktop.update_clock();
                desktop.process_deferred_wallpaper();
                let had_damage = desktop.compose();

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

                if has_animations { stat_animations += 1; }
                if had_damage { stat_damage += 1; } else { stat_no_damage += 1; }

                // If animations are still active, keep rendering next frame
                if has_animations {
                    RENDER_NEEDED.store(true, Ordering::Release);
                }

                // Frame pacing: only sleep when actual pixels were composited.
                // If compose() had no damage, the cycle was nearly free — skip
                // the 16ms sleep so the render thread returns to idle quickly.
                if had_damage || has_animations {
                    idle_count = 0;
                    let elapsed = sys::uptime_ms().wrapping_sub(t0);
                    let target = 16u32;
                    if elapsed < target {
                        process::sleep(target - elapsed);
                    }
                }
            } else {
                stat_lock_fail += 1;
                // Lock contended — management thread is doing work.
                // Sleep briefly and retry. Previous frame stays on screen.
                process::sleep(1);
                RENDER_NEEDED.store(true, Ordering::Release);
                continue;
            }
        } else {
            stat_idle_loops += 1;
            // No work signaled — idle sleep with adaptive interval.
            // Ramp: 2ms → 4ms → 8ms → 16ms → 50ms → 100ms → 250ms
            // This reduces idle CPU from ~5% to <0.1% while still responding
            // quickly when new work arrives (worst-case 250ms latency at deep idle).
            idle_count = idle_count.saturating_add(1);
            let sleep_ms = match idle_count {
                0..=3 => 2,
                4..=7 => 4,
                8..=15 => 8,
                16..=31 => 16,
                32..=63 => 50,
                64..=127 => 100,
                _ => 250,
            };
            process::sleep(sleep_ms);

            // Check clock at ~1Hz during idle (cheap: just read time + compare minute).
            // If the minute changed, signal_render() triggers a compose for the clock.
            let now = sys::uptime_ms();
            if now.wrapping_sub(last_clock_check) >= 1000 {
                last_clock_check = now;
                if try_lock() {
                    let desktop = unsafe { desktop_ref() };
                    let clock_changed = desktop.update_clock();
                    release_lock();
                    if clock_changed {
                        signal_render();
                    }
                }
            }
        }
    }
}
