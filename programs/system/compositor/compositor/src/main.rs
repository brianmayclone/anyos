//! Userspace Compositor for anyOS (WP19)
//!
//! Multi-threaded compositor with:
//!   - Management thread: IPC, window lifecycle, input, menus
//!   - Render thread: layer compositing, framebuffer flush, GPU commands
//!   - Layer-based compositing with damage tracking
//!   - macOS dark theme desktop (menubar, wallpaper, window chrome)
//!   - Window management (create, destroy, focus, drag, resize)
//!   - HW cursor support with SW fallback
//!   - GPU acceleration commands (UPDATE, FLIP, CURSOR)
//!   - Event channel IPC for app communication

#![no_std]
#![no_main]

use anyos_std::ipc;
use anyos_std::process;
use anyos_std::println;
use core::sync::atomic::{AtomicBool, Ordering};

mod compositor;
mod desktop;
mod ipc_protocol;
mod keys;
mod menu;

anyos_std::entry!(main);

// ── Shared state between management and render threads ──────────────────────

/// Spinlock protecting access to the Desktop.
static DESKTOP_LOCK: AtomicBool = AtomicBool::new(false);

/// Raw pointer to the heap-allocated Desktop (set once during init, never freed).
static mut DESKTOP_PTR: *mut desktop::Desktop = core::ptr::null_mut();

fn acquire_lock() {
    while DESKTOP_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn release_lock() {
    DESKTOP_LOCK.store(false, Ordering::Release);
}

/// Get a mutable reference to the Desktop. Caller MUST hold DESKTOP_LOCK.
unsafe fn desktop_ref() -> &'static mut desktop::Desktop {
    &mut *DESKTOP_PTR
}

// ── Render Thread ───────────────────────────────────────────────────────────

/// Render thread entry point — composites and flushes at ~60 Hz.
fn render_thread_entry() {
    println!("compositor: render thread running");
    let mut frame: u32 = 0;
    loop {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.compose();
        release_lock();

        frame = frame.wrapping_add(1);
        if frame % 120 == 0 {
            println!("compositor: render frame {}", frame);
        }

        process::sleep(16);
    }
}

// ── Main (Management Thread) ────────────────────────────────────────────────

fn main() {
    println!("compositor: starting userspace compositor...");

    // Step 1: Register as the system compositor
    if ipc::register_compositor() != 0 {
        println!("compositor: FAILED — another compositor is already registered");
        return;
    }
    println!("compositor: registered as system compositor");

    // Step 2: Map the framebuffer into our address space
    let fb_info = match ipc::map_framebuffer() {
        Some(info) => info,
        None => {
            println!("compositor: FAILED to map framebuffer");
            return;
        }
    };
    println!(
        "compositor: framebuffer mapped at 0x{:08X} ({}x{}, pitch={})",
        fb_info.fb_addr, fb_info.width, fb_info.height, fb_info.pitch
    );

    let width = fb_info.width;
    let height = fb_info.height;
    let fb_ptr = fb_info.fb_addr as *mut u32;

    // Step 3: Initialize desktop (single-threaded, no lock needed yet)
    let mut desktop = alloc::boxed::Box::new(desktop::Desktop::new(
        fb_ptr, width, height, fb_info.pitch,
    ));
    desktop.init();

    // Step 3b: Enable hardware cursor
    desktop.init_hw_cursor();
    println!("compositor: HW cursor enabled");

    // Initial full-screen compose
    desktop.compositor.damage_all();
    desktop.compose();

    println!("compositor: desktop drawn ({}x{})", width, height);

    // Step 4: Create event channel for app IPC
    let compositor_channel = ipc::evt_chan_create("compositor");
    let compositor_sub = ipc::evt_chan_subscribe(compositor_channel, 0);
    println!("compositor: event channel created (id={})", compositor_channel);

    // Step 4b: Subscribe to system events (process exit notifications)
    let sys_sub = ipc::evt_sys_subscribe(0);

    // Step 5: Spawn the dock
    let _dock_tid = process::spawn("/system/compositor/dock", "");
    println!("compositor: dock spawned");

    desktop.compose();

    // Step 6: Signal boot ready so the kernel knows desktop is up
    anyos_std::sys::boot_ready();

    // Step 7: Move Desktop to global and spawn render thread
    unsafe {
        DESKTOP_PTR = alloc::boxed::Box::into_raw(desktop);
    }

    // Allocate render thread stack (128 KiB) via the heap allocator.
    // CRITICAL: Do NOT use raw process::sbrk() here — that bypasses the bump
    // allocator, leaving HEAP_POS/HEAP_END stale. Subsequent heap allocations
    // would return pointers inside the render stack, corrupting it.
    let render_stack_size: usize = 128 * 1024;
    let render_stack_vec = alloc::vec![0u8; render_stack_size];
    let render_stack_base = render_stack_vec.as_ptr() as usize;
    core::mem::forget(render_stack_vec); // Leak — render thread runs forever
    // RSP must be STACK_TOP - 8 for x86_64 ABI alignment
    let render_stack_top = render_stack_base + render_stack_size - 8;
    let render_tid = process::thread_create(render_thread_entry, render_stack_top, "compositor/gpu");
    println!(
        "compositor: render thread spawned (TID={}, stack=0x{:X})",
        render_tid, render_stack_base
    );

    println!("compositor: entering main loop (multi-threaded)");

    // ── Management Thread Loop ──────────────────────────────────────────────

    let mut events_buf = [[0u32; 5]; 256];
    let mut ipc_buf = [0u32; 5];
    let mut frame_count: u32 = 0;

    loop {
        // Poll raw input events (no lock needed — just reading from kernel)
        let event_count = ipc::input_poll(&mut events_buf) as usize;

        // Process input under lock
        if event_count > 0 {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.damage_cursor();
            desktop.process_input(&events_buf, event_count);
            desktop.damage_cursor();
            // Flush HW cursor move commands while holding lock
            desktop.compositor.flush_gpu();
            release_lock();
        } else {
            // Still flush any queued HW cursor commands
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.compositor.flush_gpu();
            release_lock();
        }

        // Poll IPC commands from apps (up to 16 per frame)
        for _ in 0..16 {
            if !ipc::evt_chan_poll(compositor_channel, compositor_sub, &mut ipc_buf) {
                break;
            }
            if ipc_buf[0] >= 0x1000 && ipc_buf[0] < 0x2000 {
                let response = match ipc_buf[0] {
                    // CMD_CREATE_WINDOW: shm_map OUTSIDE lock (potentially slow)
                    ipc_protocol::CMD_CREATE_WINDOW => {
                        let shm_id = ipc_buf[4] >> 16;
                        let shm_addr = if shm_id > 0 {
                            ipc::shm_map(shm_id)
                        } else {
                            0
                        };
                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        let resp =
                            desktop.handle_create_window_pre_mapped(&ipc_buf, shm_addr as usize);
                        release_lock();
                        resp
                    }
                    // CMD_RESIZE_SHM: shm_map OUTSIDE lock (potentially slow)
                    ipc_protocol::CMD_RESIZE_SHM => {
                        let new_shm_id = ipc_buf[2];
                        let shm_addr = if new_shm_id > 0 {
                            ipc::shm_map(new_shm_id)
                        } else {
                            0
                        };
                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        let resp =
                            desktop.handle_resize_shm_pre_mapped(&ipc_buf, shm_addr as usize);
                        release_lock();
                        resp
                    }
                    // All other commands: handle under lock (fast)
                    _ => {
                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        let resp = desktop.handle_ipc_command(&ipc_buf);
                        release_lock();
                        resp
                    }
                };

                // Send response outside lock (just a syscall)
                if let Some((target_sub, response)) = response {
                    if let Some(sub_id) = target_sub {
                        ipc::evt_chan_emit_to(compositor_channel, sub_id, &response);
                    } else {
                        ipc::evt_chan_emit(compositor_channel, &response);
                    }
                }
            }
        }

        // Poll system events (process exit, resolution change)
        {
            let mut sys_buf = [0u32; 5];
            while ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                if sys_buf[0] == 0x0021 {
                    // EVT_PROCESS_EXITED
                    let exited_tid = sys_buf[1];
                    desktop.on_process_exit(exited_tid);
                } else if sys_buf[0] == 0x0040 {
                    // EVT_RESOLUTION_CHANGED
                    let new_w = sys_buf[1];
                    let new_h = sys_buf[2];
                    desktop.handle_resolution_change(new_w, new_h);
                }
                release_lock();
            }
        }

        // Drain window events under lock, then emit outside lock
        let ipc_events = {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let events = desktop.drain_ipc_events();
            release_lock();
            events
        };
        for (target_sub, evt) in &ipc_events {
            if let Some(sub_id) = target_sub {
                ipc::evt_chan_emit_to(compositor_channel, *sub_id, evt);
            } else {
                ipc::evt_chan_emit(compositor_channel, evt);
            }
        }

        // Tick animations + clock under lock
        {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.tick_animations();
            frame_count = frame_count.wrapping_add(1);
            if frame_count % 60 == 0 {
                desktop.update_clock();
            }
            release_lock();
        }

        // NOTE: compose() is handled by the render thread — not called here.

        // Sleep to maintain ~60 Hz management frame rate
        process::sleep(16);
    }
}
