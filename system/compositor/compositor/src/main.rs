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

mod compositor;
mod config;
mod desktop;
mod ipc_protocol;
mod keys;
mod menu;
mod render;

use render::{acquire_lock, release_lock, desktop_ref};

anyos_std::entry!(main);

// ── Main (Management Thread) ────────────────────────────────────────────────

fn main() {
    println!("compositor: starting userspace compositor...");

    // Mark this process as critical — kernel RSP recovery will NOT kill it
    anyos_std::sys::set_critical();

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

    // Step 3: Initialize fonts (must happen before any text rendering)
    libfont_client::init();

    // Step 4: Initialize desktop (single-threaded, no lock needed yet)
    let mut desktop = alloc::boxed::Box::new(desktop::Desktop::new(
        fb_ptr, width, height, fb_info.pitch,
    ));
    desktop.init();

    // Step 3b: Take over cursor from kernel splash mode
    let (splash_x, splash_y) = ipc::cursor_takeover();
    desktop.set_cursor_pos(splash_x, splash_y);
    if desktop.has_hw_cursor() {
        desktop.init_hw_cursor();
        println!("compositor: HW cursor enabled (pos={},{})", splash_x, splash_y);
    } else {
        println!("compositor: SW cursor (pos={},{})", splash_x, splash_y);
    }

    // Initial full-screen compose
    desktop.compositor.damage_all();
    desktop.compose();

    println!("compositor: desktop drawn ({}x{})", width, height);

    // Step 5: Create event channel for app IPC
    let compositor_channel = ipc::evt_chan_create("compositor");
    let compositor_sub = ipc::evt_chan_subscribe(compositor_channel, 0);
    println!("compositor: event channel created (id={})", compositor_channel);

    // Step 5b: Subscribe to system events (process exit notifications)
    let sys_sub = ipc::evt_sys_subscribe(0);

    desktop.compose();

    // Step 6: Signal boot ready so the kernel knows desktop is up
    anyos_std::sys::boot_ready();

    // Step 7: Move Desktop to global and spawn render thread
    unsafe {
        render::set_desktop_ptr(alloc::boxed::Box::into_raw(desktop));
    }
    spawn_render_thread();

    // Step 8: Spawn login window — authentication happens in main loop
    let login_tid = process::spawn("/System/login", "");
    let mut login_pending = login_tid != u32::MAX;
    let mut dock_spawned = false;
    if login_pending {
        println!("compositor: login window spawned, waiting for authentication...");
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.set_menubar_visible(false);
        release_lock();
    } else {
        println!("compositor: login not found, continuing as root");
    }

    // Step 8b: If no login needed, spawn dock + conf immediately
    if !login_pending {
        let _dock_tid = process::spawn("/System/compositor/dock", "");
        println!("compositor: dock spawned");
        config::launch_compositor_conf();
        dock_spawned = true;
    }

    println!("compositor: entering main loop (multi-threaded)");

    // ── Management Thread Loop ──────────────────────────────────────────────

    management_loop(
        compositor_channel, compositor_sub, sys_sub,
        login_tid, &mut login_pending, &mut dock_spawned,
    );
}

/// Allocate a stack and spawn the render thread at priority 127.
fn spawn_render_thread() {
    // Allocate render thread stack (128 KiB) via the heap allocator.
    // CRITICAL: Do NOT use raw process::sbrk() here — that bypasses the bump
    // allocator, leaving HEAP_POS/HEAP_END stale. Subsequent heap allocations
    // would return pointers inside the render stack, corrupting it.
    let render_stack_size: usize = 128 * 1024;
    let render_stack_vec = alloc::vec![0u8; render_stack_size];
    let render_stack_base = render_stack_vec.as_ptr() as usize;
    core::mem::forget(render_stack_vec); // Leak — render thread runs forever
    // x86_64 ABI: RSP%16 == 8 at function entry (as if `call` pushed RA).
    let render_stack_top = ((render_stack_base + render_stack_size) & !0xF) - 8;
    // Render thread gets highest priority (127) for smooth 60 Hz compositing.
    let render_tid = process::thread_create_with_priority(
        render::render_thread_entry, render_stack_top, "compositor/gpu", 127,
    );
    println!(
        "compositor: render thread spawned (TID={}, stack=0x{:X}, priority=127)",
        render_tid, render_stack_base
    );

    // Lower management thread priority so render thread gets preferential scheduling
    process::set_priority(0, 120);
    println!("compositor: management thread priority set to 120");
}

/// The management thread event loop — processes input, IPC commands, and system events.
fn management_loop(
    compositor_channel: u32,
    compositor_sub: u32,
    sys_sub: u32,
    login_tid: u32,
    login_pending: &mut bool,
    dock_spawned: &mut bool,
) {
    let mut events_buf = [[0u32; 5]; 256];
    let mut ipc_buf = [0u32; 5];
    loop {
        // ── Check if login window has exited ──
        if *login_pending {
            let status = process::try_waitpid(login_tid);
            if status != process::STILL_RUNNING {
                *login_pending = false;
                let exit_uid = status;
                println!("compositor: authentication complete, uid={}", exit_uid);

                if exit_uid != u32::MAX && exit_uid != 0 {
                    process::set_identity(exit_uid as u16);
                    println!("compositor: identity switched to uid={}", exit_uid);
                }

                let uid = process::getuid();
                let mut name_buf = [0u8; 32];
                let nlen = process::getusername(uid, &mut name_buf);
                if nlen != u32::MAX && nlen > 0 {
                    if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
                        anyos_std::env::set("USER", username);
                        if uid != 0 {
                            let home = alloc::format!("/Users/{}", username);
                            anyos_std::env::set("HOME", &home);
                        }
                    }
                }
            }
        }

        // Spawn dock + conf programs once login is done
        if !*login_pending && !*dock_spawned {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.set_menubar_visible(true);
            release_lock();

            let _dock_tid = process::spawn("/System/compositor/dock", "");
            println!("compositor: dock spawned");
            config::launch_compositor_conf();
            *dock_spawned = true;
        }

        // Poll raw input events (no lock needed — just reading from kernel)
        let event_count = ipc::input_poll(&mut events_buf) as usize;

        // Process input under lock (skip entirely if no events — avoids lock contention)
        if event_count > 0 {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.damage_cursor();
            desktop.process_input(&events_buf, event_count);
            desktop.damage_cursor();
            desktop.compositor.flush_gpu();
            release_lock();
        }

        // Poll IPC commands from apps (up to 16 per frame)
        handle_ipc_commands(compositor_channel, compositor_sub, &mut ipc_buf);

        // Poll system events (process exit, resolution change)
        handle_system_events(compositor_channel, sys_sub);

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

        // NOTE: tick_animations(), update_clock(), and compose() are all handled
        // by the render thread — not called here.

        process::sleep(16);
    }
}

/// Process IPC commands from apps (CMD_CREATE_WINDOW, CMD_SET_THEME, etc.)
fn handle_ipc_commands(
    compositor_channel: u32,
    compositor_sub: u32,
    ipc_buf: &mut [u32; 5],
) {
    for _ in 0..16 {
        if !ipc::evt_chan_poll(compositor_channel, compositor_sub, ipc_buf) {
            break;
        }
        if ipc_buf[0] >= 0x1000 && ipc_buf[0] < 0x2000 {
            let response = match ipc_buf[0] {
                // CMD_CREATE_WINDOW: heavy work OUTSIDE lock, fast attach UNDER lock
                ipc_protocol::CMD_CREATE_WINDOW => {
                    let app_tid = ipc_buf[1];
                    let width = ipc_buf[2];
                    let height = ipc_buf[3];
                    let shm_id_and_flags = ipc_buf[4];
                    let shm_id = shm_id_and_flags >> 16;
                    let flags = shm_id_and_flags & 0xFFFF;

                    if shm_id == 0 || width == 0 || height == 0 {
                        None
                    } else {
                        // ── OUTSIDE LOCK: expensive operations ──
                        let shm_addr = ipc::shm_map(shm_id);
                        if shm_addr == 0 {
                            None
                        } else {
                            let borderless = flags & desktop::WIN_FLAG_BORDERLESS != 0;
                            let full_h = if borderless {
                                height
                            } else {
                                height + desktop::TITLE_BAR_HEIGHT
                            };

                            let mut pre_pixels =
                                alloc::vec![0u32; (width * full_h) as usize];

                            if !borderless {
                                desktop::pre_render_chrome_ex(
                                    &mut pre_pixels, width, full_h, "Window", true, flags,
                                );
                                desktop::copy_shm_to_pixels(
                                    &mut pre_pixels,
                                    width,
                                    desktop::TITLE_BAR_HEIGHT,
                                    shm_addr as *const u32,
                                    width,
                                    height,
                                );
                            }

                            // ── UNDER LOCK: fast metadata-only operations ──
                            acquire_lock();
                            let desktop = unsafe { desktop_ref() };
                            let win_id = desktop.create_ipc_window_fast(
                                app_tid, width, height, flags,
                                shm_id, shm_addr as *mut u32, pre_pixels,
                            );
                            let target = desktop.get_sub_id_for_tid(app_tid);
                            release_lock();

                            Some((
                                target,
                                [
                                    ipc_protocol::RESP_WINDOW_CREATED,
                                    win_id, shm_id, app_tid, 0,
                                ],
                            ))
                        }
                    }
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
                        desktop.handle_resize_shm_pre_mapped(ipc_buf, shm_addr as usize);
                    release_lock();
                    resp
                }
                // CMD_SET_THEME: write to shared DLL page + repaint
                ipc_protocol::CMD_SET_THEME => {
                    let new_theme = ipc_buf[1].min(1);
                    let old_theme = unsafe {
                        core::ptr::read_volatile(0x0400_000C as *const u32)
                    };
                    if new_theme != old_theme {
                        desktop::set_theme(new_theme);
                        acquire_lock();
                        let desktop = unsafe { desktop_ref() };
                        desktop.on_theme_change();
                        release_lock();
                        ipc::evt_chan_emit(compositor_channel, &[
                            ipc_protocol::EVT_THEME_CHANGED,
                            new_theme, old_theme, 0, 0,
                        ]);
                    }
                    None
                }
                // All other commands: handle under lock (fast)
                _ => {
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    let resp = desktop.handle_ipc_command(ipc_buf);
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

                // Broadcast window lifecycle events for dock filtering
                if response[0] == ipc_protocol::RESP_WINDOW_CREATED {
                    ipc::evt_chan_emit(compositor_channel, &[
                        ipc_protocol::EVT_WINDOW_OPENED,
                        response[3], // app_tid
                        response[1], // win_id
                        0, 0,
                    ]);
                } else if response[0] == ipc_protocol::RESP_WINDOW_DESTROYED {
                    let app_tid = response[2];
                    let remaining_windows = response[3];
                    if remaining_windows == 0 {
                        ipc::evt_chan_emit(compositor_channel, &[
                            ipc_protocol::EVT_WINDOW_CLOSED,
                            app_tid, 0, 0, 0,
                        ]);
                    }
                }
            }
        }
    }
}

/// Process system events (process exit, resolution change).
fn handle_system_events(compositor_channel: u32, sys_sub: u32) {
    let mut sys_buf = [0u32; 5];
    while ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        if sys_buf[0] == 0x0021 {
            // EVT_PROCESS_EXITED
            let exited_tid = sys_buf[1];
            let exit_code = sys_buf[2];
            desktop.on_process_exit(exited_tid);

            // Check if this was a crash (signal > 128 indicates a fatal signal)
            if exit_code > 128 && exit_code < 256 {
                // Query crash info from kernel
                let mut crash_buf = [0u8; core::mem::size_of::<desktop::crash_dialog::CrashReport>()];
                let bytes = anyos_std::sys::get_crash_info(exited_tid, &mut crash_buf);
                if bytes > 0 {
                    desktop.show_crash_dialog(exited_tid, exit_code, &crash_buf);
                }
            }

            release_lock();
            ipc::evt_chan_emit(compositor_channel, &[
                ipc_protocol::EVT_WINDOW_CLOSED,
                exited_tid, 0, 0, 0,
            ]);
        } else if sys_buf[0] == 0x0040 {
            // EVT_RESOLUTION_CHANGED
            let new_w = sys_buf[1];
            let new_h = sys_buf[2];
            desktop.handle_resolution_change(new_w, new_h);
            release_lock();
        } else {
            release_lock();
        }
    }
}
