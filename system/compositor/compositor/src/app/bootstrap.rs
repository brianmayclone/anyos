//! Compositor startup and bootstrapping.

use anyos_std::ipc;
use anyos_std::println;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::Vec;

use crate::config;
use crate::desktop;
use crate::render::{self, acquire_lock, desktop_ref, release_lock, signal_render};

use super::management::management_loop;

pub fn run() {
    println!("compositor: starting userspace compositor...");

    let mut setup_mode = false;
    {
        let mut args_buf = [0u8; 256];
        let args_str = anyos_std::process::args(&mut args_buf);
        for token in args_str.split_ascii_whitespace() {
            if token == "setupmode" {
                setup_mode = true;
                println!("compositor: SETUP MODE — no login, no dock");
            }
        }
    }

    sys::set_critical();

    if ipc::register_compositor() != 0 {
        println!("compositor: FAILED — another compositor is already registered");
        return;
    }
    println!("compositor: registered as system compositor");

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

    libfont_client::init();

    let mut desktop = alloc::boxed::Box::new(desktop::Desktop::new(
        fb_ptr, width, height, fb_info.pitch,
    ));
    desktop.init_no_wallpaper();

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

    if let Some(saved_theme) = config::read_theme() {
        let is_light = saved_theme.mode == "light";
        if is_light {
            desktop::set_theme(1);
            println!("compositor: restored theme: light");
        } else {
            println!("compositor: restored theme: dark");
        }
    }

    if let Some(mode) = config::read_font_smoothing() {
        desktop::set_font_smoothing(mode);
        let mode_name = match mode {
            0 => "none",
            1 => "greyscale",
            _ => "subpixel",
        };
        println!("compositor: restored font smoothing: {}", mode_name);
    }

    if let Some(scale) = config::read_scale_factor() {
        desktop::theme::set_scale_factor(scale);
        if scale != 100 {
            desktop.handle_scale_change();
        }
        println!("compositor: restored DPI scale: {}%", scale);
    } else {
        desktop::theme::set_scale_factor(100);
    }

    let (splash_x, splash_y) = ipc::cursor_takeover();
    desktop.set_cursor_pos(splash_x, splash_y);
    if desktop.has_hw_cursor() {
        desktop.init_hw_cursor();
        println!("compositor: HW cursor enabled (pos={},{})", splash_x, splash_y);
    } else {
        println!("compositor: SW cursor (pos={},{})", splash_x, splash_y);
    }

    for attempt in 0..3u32 {
        desktop.compositor.damage_all();
        desktop.compose();
        desktop.compositor.gpu_cmds.push([8, 0, 0, 0, 0, 0, 0, 0, 0]);
        desktop.compositor.flush_gpu();
        if attempt == 0 {
            println!("compositor: desktop drawn ({}x{})", desktop.screen_width, desktop.screen_height);
        } else {
            println!("compositor: initial compose retry #{}", attempt);
        }
        if attempt < 2 {
            anyos_std::process::sleep(50);
        }
    }

    let compositor_channel = ipc::evt_chan_create("compositor");
    let compositor_sub = ipc::evt_chan_subscribe(compositor_channel, 0);
    println!("compositor: event channel created (id={})", compositor_channel);

    let sys_sub = ipc::evt_sys_subscribe(0);
    desktop.compositor.damage_all();
    sys::boot_ready();

    unsafe {
        render::set_desktop_ptr(alloc::boxed::Box::into_raw(desktop));
        render::set_compositor_channel(compositor_channel);
    }
    spawn_render_thread();

    if !setup_mode {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.load_default_wallpaper_pub();
        desktop.compositor.damage_all();
        release_lock();
        signal_render();
        println!("compositor: wallpaper loaded (deferred)");
    }

    if !setup_mode {
        config::launch_login_services();
    }

    let mut login_tid = if setup_mode { u32::MAX } else { process::spawn("/System/login", "") };
    let mut login_pending = login_tid != u32::MAX;
    let mut dock_spawned = setup_mode;
    if setup_mode {
        println!("compositor: setup mode — loading setup wallpaper, launching installer");
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.set_menubar_visible(true);
        desktop.load_wallpaper("/media/wallpapers/setup.png");
        release_lock();
        process::spawn("/Applications/Installer.app/Installer", "");
    } else if login_pending {
        println!("compositor: login window spawned, waiting for authentication...");
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.set_menubar_visible(false);
        release_lock();
    } else {
        println!("compositor: WARNING — /System/login could not be spawned; waiting for login before activating desktop");
    }

    let mut service_tids: Vec<u32> = Vec::new();
    println!("compositor: entering main loop (multi-threaded)");

    management_loop(
        compositor_channel,
        compositor_sub,
        sys_sub,
        &mut login_tid,
        &mut login_pending,
        &mut dock_spawned,
        &mut service_tids,
    );
}

fn spawn_render_thread() {
    let render_stack_size: usize = 512 * 1024;
    let render_stack_vec = alloc::vec![0u8; render_stack_size];
    let render_stack_base = render_stack_vec.as_ptr() as usize;
    core::mem::forget(render_stack_vec);
    let render_stack_top = ((render_stack_base + render_stack_size) & !0xF) - 8;
    let render_tid = process::thread_create_with_priority(
        render::render_thread_entry,
        render_stack_top,
        "compositor/gpu",
        127,
    );
    println!(
        "compositor: render thread spawned (TID={}, stack=0x{:X}, priority=127)",
        render_tid, render_stack_base
    );

    process::set_priority(0, 120);
    println!("compositor: management thread priority set to 120");
}
