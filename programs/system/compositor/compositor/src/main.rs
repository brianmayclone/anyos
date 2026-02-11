//! Userspace Compositor for anyOS (WP19)
//!
//! Full compositor with:
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
mod desktop;
mod ipc_protocol;
mod keys;
mod menu;

anyos_std::entry!(main);

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

    // Step 3: Initialize desktop
    let mut desktop = desktop::Desktop::new(fb_ptr, width, height, fb_info.pitch);
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
    println!("compositor: entering main loop");

    // ── Main Compositor Loop ────────────────────────────────────────────────

    let mut events_buf = [[0u32; 5]; 256];
    let mut ipc_buf = [0u32; 5];
    let mut frame_count: u32 = 0;

    loop {
        // Poll raw input events
        let event_count = ipc::input_poll(&mut events_buf) as usize;

        let mut needs_compose = false;

        if event_count > 0 {
            desktop.damage_cursor();
            needs_compose = desktop.process_input(&events_buf, event_count);
            desktop.damage_cursor();
        }

        // Flush HW cursor move commands (always, regardless of compose)
        desktop.compositor.flush_gpu();

        // Poll IPC commands from apps (up to 16 per frame)
        for _ in 0..16 {
            if !ipc::evt_chan_poll(compositor_channel, compositor_sub, &mut ipc_buf) {
                break;
            }
            if ipc_buf[0] >= 0x1000 && ipc_buf[0] < 0x2000 {
                if let Some((target_sub, response)) = desktop.handle_ipc_command(&ipc_buf) {
                    // Send response to the requesting app (unicast if sub_id known)
                    if let Some(sub_id) = target_sub {
                        ipc::evt_chan_emit_to(compositor_channel, sub_id, &response);
                    } else {
                        ipc::evt_chan_emit(compositor_channel, &response);
                    }
                }
                needs_compose = true;
            }
        }

        // Poll system events (process exit, resolution change)
        let mut sys_buf = [0u32; 5];
        while ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
            if sys_buf[0] == 0x0021 { // EVT_PROCESS_EXITED
                let exited_tid = sys_buf[1];
                desktop.on_process_exit(exited_tid);
                needs_compose = true;
            } else if sys_buf[0] == 0x0040 { // EVT_RESOLUTION_CHANGED
                let new_w = sys_buf[1];
                let new_h = sys_buf[2];
                desktop.handle_resolution_change(new_w, new_h);
                needs_compose = true;
            }
        }

        // Forward queued window events to apps via targeted delivery
        let ipc_events = desktop.drain_ipc_events();
        for (target_sub, evt) in &ipc_events {
            if let Some(sub_id) = target_sub {
                // Unicast: only the owning app receives this event
                ipc::evt_chan_emit_to(compositor_channel, *sub_id, evt);
            } else {
                // Fallback: broadcast (app didn't register its sub_id)
                ipc::evt_chan_emit(compositor_channel, evt);
            }
        }

        // Tick button animations — if any are active, force recompose
        if desktop.tick_animations() {
            needs_compose = true;
        }

        // Update clock every ~60 frames (~1 second at 60Hz)
        frame_count = frame_count.wrapping_add(1);
        if frame_count % 60 == 0 {
            desktop.update_clock();
        }

        // Compose if anything changed
        if needs_compose {
            desktop.compose();
        }

        // Sleep to maintain ~60 Hz frame rate
        process::sleep(16);
    }
}

