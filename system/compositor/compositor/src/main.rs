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
use anyos_std::sys;
use anyos_std::println;
use anyos_std::Vec;

mod compositor;
mod config;
mod desktop;
mod ipc_protocol;
mod keys;
mod menu;
mod render;

use render::{acquire_lock, release_lock, desktop_ref, signal_render};

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

    // Step 4: Initialize desktop at current (boot) resolution
    let mut desktop = alloc::boxed::Box::new(desktop::Desktop::new(
        fb_ptr, width, height, fb_info.pitch,
    ));
    desktop.init();

    // Step 4b: Restore saved resolution from compositor.conf (if different from current).
    // Done AFTER Desktop::new so the well-tested handle_resolution_change() path
    // handles all surface resizing, re-mapping, and recomposition.
    if let Some(saved) = config::read_resolution() {
        if saved.width != width || saved.height != height {
            println!(
                "compositor: restoring saved resolution {}x{} (current: {}x{})",
                saved.width, saved.height, width, height
            );
            if anyos_std::ui::window::set_resolution(saved.width, saved.height) {
                desktop.handle_resolution_change(saved.width, saved.height);
                println!(
                    "compositor: resolution restored to {}x{}",
                    saved.width, saved.height
                );
            } else {
                println!("compositor: failed to restore saved resolution, keeping {}x{}", width, height);
            }
        }
    }

    // Step 4c: Restore saved theme from compositor.conf
    if let Some(saved_theme) = config::read_theme() {
        let is_light = saved_theme.mode == "light";
        if is_light {
            desktop::set_theme(1);
            println!("compositor: restored theme: light");
        } else {
            println!("compositor: restored theme: dark");
        }
    }

    // Step 4d: Restore saved font smoothing from compositor.conf
    if let Some(mode) = config::read_font_smoothing() {
        desktop::set_font_smoothing(mode);
        let mode_name = match mode {
            0 => "none",
            1 => "greyscale",
            _ => "subpixel",
        };
        println!("compositor: restored font smoothing: {}", mode_name);
    }

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

    println!("compositor: desktop drawn ({}x{})", desktop.screen_width, desktop.screen_height);

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
        render::set_compositor_channel(compositor_channel);
    }
    spawn_render_thread();

    // Step 8: Launch login-time services (e.g. inputmon for keyboard layout)
    config::launch_login_services();

    // Step 9: Spawn login window — authentication happens in main loop
    let mut login_tid = process::spawn("/System/login", "");
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

    // Service TIDs to kill during logout (dock + autostart programs, NOT login services)
    let mut service_tids: Vec<u32> = Vec::new();

    // Step 9b: If no login needed, spawn dock + conf immediately
    if !login_pending {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.init_desktop_icons();
        release_lock();

        let dock_tid = process::spawn("/System/compositor/dock", "");
        if dock_tid != u32::MAX {
            service_tids.push(dock_tid);
        }
        println!("compositor: dock spawned");
        service_tids.extend(config::launch_autostart());
        dock_spawned = true;
    }

    println!("compositor: entering main loop (multi-threaded)");

    // ── Management Thread Loop ──────────────────────────────────────────────

    management_loop(
        compositor_channel, compositor_sub, sys_sub,
        &mut login_tid, &mut login_pending, &mut dock_spawned,
        &mut service_tids,
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
    login_tid: &mut u32,
    login_pending: &mut bool,
    dock_spawned: &mut bool,
    service_tids: &mut Vec<u32>,
) {
    let mut events_buf = [[0u32; 5]; 256];
    let mut ipc_buf = [0u32; 5];
    let mut login_retries: u32 = 0;
    let mut login_failed = false;
    const MAX_LOGIN_RETRIES: u32 = 10;

    // ── Debug stats (reset every 5 seconds) ──
    let mut mgmt_loops: u32 = 0;
    let mut mgmt_input: u32 = 0;
    let mut mgmt_ipc: u32 = 0;
    let mut mgmt_sys: u32 = 0;
    let mut mgmt_idle: u32 = 0;
    let mut mgmt_last_report: u32 = sys::uptime_ms();

    loop {
        // ── Periodic stats dump ──
        let now_ms = sys::uptime_ms();
        if now_ms.wrapping_sub(mgmt_last_report) >= 30000 {
            println!(
                "MGMT-STATS: loops={} input={} ipc={} sys={} idle={}",
                mgmt_loops, mgmt_input, mgmt_ipc, mgmt_sys, mgmt_idle
            );
            mgmt_loops = 0;
            mgmt_input = 0;
            mgmt_ipc = 0;
            mgmt_sys = 0;
            mgmt_idle = 0;
            mgmt_last_report = now_ms;
        }
        mgmt_loops += 1;

        // Block until: IPC event arrives, input IRQ fires, or timeout.
        // Input IRQs (keyboard/mouse) call wake_compositor_if_blocked() in the
        // kernel, so we don't need a short polling timeout. Use a large timeout
        // and let the kernel wake us on demand — reduces idle wakeups from
        // 62.5/sec (16ms) to near zero.
        let timeout = if *login_pending { 100 } else { 5000 };
        ipc::evt_chan_wait(compositor_channel, compositor_sub, timeout);

        // ── Check if login window has exited ──
        if *login_pending {
            let status = process::try_waitpid(*login_tid);
            if status != process::STILL_RUNNING {
                // Determine if login failed:
                // - Crash: signal exit codes 129-255
                // - Cancelled/no auth: u32::MAX
                let is_crash = status > 128 && status < 256;
                let is_cancelled = status == u32::MAX;

                if is_crash || is_cancelled {
                    login_retries += 1;
                    if is_crash {
                        println!("compositor: login crashed (exit={}), attempt {}/{}", status, login_retries, MAX_LOGIN_RETRIES);
                    } else {
                        println!("compositor: login exited without authentication, attempt {}/{}", login_retries, MAX_LOGIN_RETRIES);
                    }

                    if login_retries >= MAX_LOGIN_RETRIES {
                        println!("compositor: FATAL — login failed {} times, giving up", MAX_LOGIN_RETRIES);
                        *login_pending = false;
                        login_failed = true;
                    } else {
                        let new_tid = process::spawn("/System/login", "");
                        if new_tid != u32::MAX {
                            *login_tid = new_tid;
                            println!("compositor: login re-spawned (TID={})", new_tid);
                        } else {
                            println!("compositor: FATAL — cannot re-spawn login");
                            *login_pending = false;
                        }
                    }
                } else {
                    // Valid uid — authentication succeeded (uid=0 for root, 1000+ for users)
                    *login_pending = false;
                    let exit_uid = status;
                    println!("compositor: authentication complete, uid={}", exit_uid);

                    if exit_uid != 0 {
                        process::set_identity(exit_uid as u16);
                        println!("compositor: identity switched to uid={}", exit_uid);
                    }

                    let uid = process::getuid();
                    let mut name_buf = [0u8; 32];
                    let nlen = process::getusername(uid, &mut name_buf);
                    if nlen != u32::MAX && nlen > 0 {
                        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
                            anyos_std::env::set("USER", username);
                            let home = alloc::format!("/Users/{}", username);
                            anyos_std::env::set("HOME", &home);
                        }
                    }
                }
            }
        }

        // Spawn dock + conf programs once login succeeded (not if login failed)
        if !*login_pending && !*dock_spawned && !login_failed {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.set_menubar_visible(true);
            desktop.init_desktop_icons();
            release_lock();
            signal_render();

            let dock_tid = process::spawn("/System/compositor/dock", "");
            if dock_tid != u32::MAX {
                service_tids.push(dock_tid);
            }
            println!("compositor: dock spawned");
            service_tids.extend(config::launch_autostart());
            *dock_spawned = true;
        }

        // Poll raw input events (no lock needed — just reading from kernel)
        let event_count = ipc::input_poll(&mut events_buf) as usize;

        // Process input under lock (skip entirely if no events — avoids lock contention)
        if event_count > 0 {
            mgmt_input += 1;
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            desktop.process_input(&events_buf, event_count);
            desktop.damage_cursor();
            desktop.compositor.flush_gpu();
            release_lock();
            signal_render();
        }

        // Poll IPC commands from apps (up to 16 per frame)
        let had_ipc = handle_ipc_commands(compositor_channel, compositor_sub, &mut ipc_buf);

        // Poll system events (process exit, resolution change)
        let had_sys = handle_system_events(compositor_channel, sys_sub);

        if had_ipc { mgmt_ipc += 1; }
        if had_sys { mgmt_sys += 1; }
        if event_count == 0 && !had_ipc && !had_sys { mgmt_idle += 1; }

        // Poll desktop icons for mount changes (only after login, every ~3s)
        let had_mounts = if !*login_pending && *dock_spawned {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let changed = desktop.poll_desktop_icons();
            release_lock();
            changed
        } else {
            false
        };

        // Signal render thread ONLY when actual work was processed.
        // Previously this was unconditional, causing the render thread to wake
        // ~62.5 times/sec even when idle (both threads at 1-2% for zero work).
        if had_ipc || had_sys || had_mounts {
            signal_render();
        }

        // Drain window events under lock, then emit outside lock.
        // Only acquire the lock if there's reason to expect events (input, IPC, or
        // system events could have generated outgoing events for apps).
        let had_work = event_count > 0 || had_ipc || had_sys;
        if had_work {
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
        }

        // ── Check for logout request ──
        {
            acquire_lock();
            let desktop = unsafe { desktop_ref() };
            let logout = desktop.logout_requested;
            if logout {
                desktop.logout_requested = false;
            }
            release_lock();

            if logout {
                perform_logout(
                    compositor_channel, login_tid, login_pending, dock_spawned,
                    service_tids,
                );
            }
        }

        // NOTE: tick_animations(), update_clock(), and compose() are all handled
        // by the render thread — not called here.
        // No adaptive sleep needed — evt_chan_wait at the top of the loop handles it.
    }
}

/// Process IPC commands from apps (CMD_CREATE_WINDOW, CMD_SET_THEME, etc.)
///
/// Two-pass design to reduce flicker:
/// 1. Poll all pending events into a buffer (no lock)
/// 2. Process fast commands (CMD_PRESENT, etc.) under a SINGLE lock hold
///    so the render thread can't fire between consecutive presents.
/// Commands that need work outside the lock (CREATE_WINDOW, RESIZE_SHM,
/// SET_THEME) are handled with their own lock cycles.
///
/// Returns `true` if any commands were processed.
fn handle_ipc_commands(
    compositor_channel: u32,
    compositor_sub: u32,
    ipc_buf: &mut [u32; 5],
) -> bool {
    // Pass 1: poll all pending IPC events into a local buffer
    let mut cmds = [[0u32; 5]; 16];
    let mut cmd_count = 0usize;
    for i in 0..16 {
        if !ipc::evt_chan_poll(compositor_channel, compositor_sub, ipc_buf) {
            break;
        }
        cmds[i] = *ipc_buf;
        cmd_count += 1;
    }
    if cmd_count == 0 {
        return false;
    }

    // Collect responses to send outside lock
    let mut responses: Vec<(Option<u32>, [u32; 5])> = Vec::new();

    // Pass 2: process commands — batch fast ones under a single lock hold
    let mut i = 0;
    while i < cmd_count {
        let cmd = cmds[i];
        if cmd[0] < 0x1000 || cmd[0] >= 0x2000 {
            i += 1;
            continue;
        }

        match cmd[0] {
            // CMD_CREATE_WINDOW: heavy work OUTSIDE lock, fast attach UNDER lock
            ipc_protocol::CMD_CREATE_WINDOW => {
                let app_tid = cmd[1];
                let wh = cmd[2];
                let width = wh >> 16;
                let height = wh & 0xFFFF;
                let xy = cmd[3];
                let raw_x = (xy >> 16) as u16;
                let raw_y = (xy & 0xFFFF) as u16;
                let shm_id_and_flags = cmd[4];
                let shm_id = shm_id_and_flags >> 16;
                let flags = shm_id_and_flags & 0xFFFF;

                if shm_id != 0 && width != 0 && height != 0 {
                    // ── OUTSIDE LOCK: expensive operations ──
                    let shm_addr = ipc::shm_map(shm_id);
                    if shm_addr != 0 {
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
                            raw_x, raw_y,
                        );
                        let target = desktop.get_sub_id_for_tid(app_tid);
                        release_lock();

                        responses.push((
                            target,
                            [
                                ipc_protocol::RESP_WINDOW_CREATED,
                                win_id, shm_id, app_tid, 0,
                            ],
                        ));
                    }
                }
                i += 1;
            }
            // CMD_RESIZE_SHM: shm_map OUTSIDE lock (potentially slow)
            ipc_protocol::CMD_RESIZE_SHM => {
                let new_shm_id = cmd[2];
                let shm_addr = if new_shm_id > 0 {
                    ipc::shm_map(new_shm_id)
                } else {
                    0
                };
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                if let Some(resp) =
                    desktop.handle_resize_shm_pre_mapped(&cmd, shm_addr as usize)
                {
                    responses.push(resp);
                }
                release_lock();
                i += 1;
            }
            // CMD_SET_FONT_SMOOTHING: write to shared DLL page + repaint
            ipc_protocol::CMD_SET_FONT_SMOOTHING => {
                let new_mode = cmd[1].min(2);
                let old_mode = desktop::theme::read_font_smoothing();
                if new_mode != old_mode {
                    desktop::set_font_smoothing(new_mode);
                    config::save_font_smoothing(new_mode);
                    ipc::evt_chan_emit(compositor_channel, &[
                        ipc_protocol::EVT_FONT_SMOOTHING_CHANGED,
                        new_mode, 0, 0, 0,
                    ]);
                    // Force all windows to repaint with new font rendering
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    desktop.compositor.damage_all();
                    release_lock();
                    signal_render();
                }
                i += 1;
            }
            // CMD_SET_THEME: write to shared DLL page + repaint
            ipc_protocol::CMD_SET_THEME => {
                let new_theme = cmd[1].min(1);
                let old_theme = unsafe {
                    core::ptr::read_volatile(0x0400_000C as *const u32)
                };
                if new_theme != old_theme {
                    desktop::set_theme(new_theme);
                    // Persist theme choice to compositor.conf
                    config::save_theme(if new_theme == 0 { "dark" } else { "light" }, "");
                    acquire_lock();
                    let desktop = unsafe { desktop_ref() };
                    desktop.on_theme_change();
                    release_lock();
                    ipc::evt_chan_emit(compositor_channel, &[
                        ipc_protocol::EVT_THEME_CHANGED,
                        new_theme, old_theme, 0, 0,
                    ]);
                    // Wake render thread immediately so all apps repaint
                    signal_render();
                }
                i += 1;
            }
            // All other fast commands: batch under a single lock hold.
            // This prevents the render thread from firing between consecutive
            // CMD_PRESENTs during rapid scrolling (eliminates partial-update flicker).
            _ => {
                acquire_lock();
                let desktop = unsafe { desktop_ref() };
                // Process this and all consecutive fast commands under one lock
                while i < cmd_count {
                    let c = cmds[i];
                    if c[0] < 0x1000 || c[0] >= 0x2000 {
                        i += 1;
                        continue;
                    }
                    // Break out for commands that need outside-lock work
                    match c[0] {
                        ipc_protocol::CMD_CREATE_WINDOW
                        | ipc_protocol::CMD_RESIZE_SHM
                        | ipc_protocol::CMD_SET_THEME
                        | ipc_protocol::CMD_SET_FONT_SMOOTHING => break,
                        _ => {}
                    }
                    if let Some(resp) = desktop.handle_ipc_command(&c) {
                        responses.push(resp);
                    }
                    i += 1;
                }
                release_lock();
            }
        }
    }

    // Send all responses outside lock
    for (target_sub, response) in &responses {
        if let Some(sub_id) = target_sub {
            ipc::evt_chan_emit_to(compositor_channel, *sub_id, response);
        } else {
            ipc::evt_chan_emit(compositor_channel, response);
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
    true
}

/// Process system events (process exit, resolution change).
///
/// Returns `true` if any events were processed.
fn handle_system_events(compositor_channel: u32, sys_sub: u32) -> bool {
    let mut sys_buf = [0u32; 5];
    let mut had_work = false;
    while ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
        had_work = true;
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
            // EVT_RESOLUTION_CHANGED — persist the new resolution to compositor.conf
            let new_w = sys_buf[1];
            let new_h = sys_buf[2];
            desktop.handle_resolution_change(new_w, new_h);
            release_lock();
            config::save_resolution(new_w, new_h);
        } else {
            release_lock();
        }
    }
    had_work
}

/// Handle user logout: kill all user processes, clean up desktop state, re-spawn login.
fn perform_logout(
    compositor_channel: u32,
    login_tid: &mut u32,
    login_pending: &mut bool,
    dock_spawned: &mut bool,
    service_tids: &mut Vec<u32>,
) {
    println!("compositor: logout requested — terminating user processes...");

    // Collect all known user TIDs from windows and app subscriptions
    let mut tids_to_kill: Vec<u32>;
    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        tids_to_kill = Vec::with_capacity(desktop.windows.len() + desktop.app_subs.len());
        for win in &desktop.windows {
            if win.owner_tid != 0 && !tids_to_kill.contains(&win.owner_tid) {
                tids_to_kill.push(win.owner_tid);
            }
        }
        for &(tid, _) in &desktop.app_subs {
            if tid != 0 && !tids_to_kill.contains(&tid) {
                tids_to_kill.push(tid);
            }
        }
        release_lock();
    }

    // Also kill tracked service TIDs (dock, autostart programs)
    for &tid in service_tids.iter() {
        if !tids_to_kill.contains(&tid) {
            tids_to_kill.push(tid);
        }
    }
    service_tids.clear();

    // Send kill signal to each process
    for &tid in &tids_to_kill {
        process::kill(tid);
    }

    // Give processes time to exit
    process::sleep(200);

    // Force-clean remaining state (system events from killed processes will be
    // drained by the regular management loop after we return)
    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };

        // Force-destroy any remaining windows (in case system events were missed)
        let remaining: Vec<u32> = desktop.windows.iter().map(|w| w.id).collect();
        for id in remaining {
            desktop.destroy_window(id);
        }

        // Reset desktop state
        desktop.app_subs.clear();
        desktop.menu_bar = crate::menu::MenuBar::new();
        desktop.focused_window = None;
        desktop.crash_dialogs.clear();
        desktop.tray_ipc_events.clear();
        desktop.clipboard_data.clear();
        desktop.desktop_icons.icons.clear();
        desktop.desktop_icons.selected_icon = None;

        // Hide menubar and clear desktop icons for login screen
        desktop.set_menubar_visible(false);
        desktop.reload_wallpaper_and_icons();
        desktop.compositor.damage_all();
        release_lock();
    }
    signal_render();

    // Re-spawn login
    let new_tid = process::spawn("/System/login", "");
    if new_tid != u32::MAX {
        *login_tid = new_tid;
        *login_pending = true;
        *dock_spawned = false;
        println!("compositor: logged out, login re-spawned (TID={})", new_tid);
    } else {
        println!("compositor: FATAL — cannot spawn login after logout");
    }
}
