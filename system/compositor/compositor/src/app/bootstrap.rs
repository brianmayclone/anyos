//! Compositor startup and bootstrapping.

use alloc::string::String;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyos_std::env;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::println;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::Vec;
use libami::{AmiClient, AmiValue};

use crate::config;
use crate::desktop;
use crate::render::{self, acquire_lock, desktop_ref, release_lock, signal_render};

use super::management::management_loop;

// ── Init waiter thread ──────────────────────────────────────────────────────
// A small thread that blocks on waitpid(init) and sets an atomic flag when
// init has exited.  The management loop checks the flag — no polling needed.

static INIT_WAIT_TID: AtomicU32 = AtomicU32::new(u32::MAX);
static INIT_DONE: AtomicBool = AtomicBool::new(false);
static INIT_EXIT_CODE: AtomicU32 = AtomicU32::new(0);

fn ensure_bootstrap_env() {
    if let Ok(content) = fs::read_to_string("/System/settings/language.conf") {
        let lang = content.trim();
        if !lang.is_empty() {
            env::set("LANG", lang);
            return;
        }
    }

    env::set("LANG", "en");
}

fn init_waiter_entry() {
    let tid = INIT_WAIT_TID.load(Ordering::Acquire);
    let code = process::waitpid(tid);
    INIT_EXIT_CODE.store(code, Ordering::Release);
    INIT_DONE.store(true, Ordering::Release);
}

fn spawn_init_waiter(init_tid: u32) {
    INIT_WAIT_TID.store(init_tid, Ordering::Release);
    INIT_DONE.store(false, Ordering::Release);

    let stack_size: usize = 16 * 1024;
    let stack_vec = alloc::vec![0u8; stack_size];
    let stack_base = stack_vec.as_ptr() as usize;
    core::mem::forget(stack_vec);
    #[cfg(target_arch = "x86_64")]
    let stack_top = ((stack_base + stack_size) & !0xF) - 8;
    #[cfg(target_arch = "aarch64")]
    let stack_top = (stack_base + stack_size) & !0xF;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // x86 threads return directly to the synthetic return slot at stack_top.
        *(stack_top as *mut usize) = process::thread_exit_stub_addr();
    }
    process::thread_create_with_priority(init_waiter_entry, stack_top, "compositor/init-wait", 50);
}

fn register_with_ami(width: u32, height: u32, setup_mode: bool) {
    for _ in 0..100 {
        match AmiClient::connect("compositor") {
            Ok(mut ami) => {
                let _ = ami.set(
                    "compositor.status",
                    AmiValue::String(String::from("starting")),
                );
                let _ = ami.set("compositor.framebuffer.width", AmiValue::Int(width as i64));
                let _ = ami.set(
                    "compositor.framebuffer.height",
                    AmiValue::Int(height as i64),
                );
                let _ = ami.set("compositor.setup_mode", AmiValue::Bool(setup_mode));
                let _ = ami.set("compositor.status", AmiValue::String(String::from("ready")));
                println!("compositor: registered with AMID");
                return;
            }
            Err(_) => {
                process::sleep(20);
            }
        }
    }
    println!("compositor: WARNING — AMID not reachable during bootstrap");
}

pub fn is_init_done() -> bool {
    INIT_DONE.load(Ordering::Acquire)
}

pub fn init_exit_code() -> u32 {
    INIT_EXIT_CODE.load(Ordering::Acquire)
}

pub fn run() {
    println!("compositor: starting userspace compositor...");
    ensure_bootstrap_env();

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

    register_with_ami(width, height, setup_mode);

    // Start fontd (font server) — must be running before libfont_client::init()
    // so that font data is served from shared memory instead of each process
    // loading fonts from disk independently.
    //
    // Subscribe to the channel BEFORE spawning fontd to avoid a race condition
    // where fontd emits EVT_FONTD_READY before we're listening.
    let fontd_chan = ipc::evt_chan_create("fontd");
    let fontd_sub = ipc::evt_chan_subscribe(fontd_chan, 0);

    let fontd_tid = process::spawn("/System/fontd", "");
    if fontd_tid != u32::MAX {
        println!(
            "compositor: fontd spawned (TID={}), waiting for ready...",
            fontd_tid
        );
        let mut ready = false;
        for _ in 0..100 {
            ipc::evt_chan_wait(fontd_chan, fontd_sub, 50);
            let mut evt = [0u32; 5];
            if ipc::evt_chan_poll(fontd_chan, fontd_sub, &mut evt) && evt[0] == 0x6000 {
                ready = true;
                break;
            }
        }
        if ready {
            println!("compositor: fontd ready");
        } else {
            println!("compositor: WARNING — fontd did not signal ready, proceeding anyway");
        }
    } else {
        println!("compositor: WARNING — fontd not found, fonts will load from disk");
    }
    ipc::evt_chan_unsubscribe(fontd_chan, fontd_sub);

    println!("compositor: loading libfont...");
    libfont_client::init();
    println!("compositor: libfont loaded");

    // Start displayd — multi-monitor layout daemon. It owns the layout
    // policy: reads display.conf at boot and atomically applies the
    // resulting OutputLayout via SYS_DISPLAY_SET_LAYOUT, then listens
    // for hot-plug events and re-applies. Same fire-and-forget pattern
    // as fontd; we don't block on it because the kernel already has
    // a sane default layout active by the time the compositor starts
    // (init_secondary_outputs further down still works without
    // displayd, just with the kernel's bootstrap layout).
    let displayd_tid = process::spawn("/System/displayd", "");
    if displayd_tid != u32::MAX {
        println!(
            "compositor: displayd spawned (TID={})",
            displayd_tid
        );
    } else {
        println!("compositor: WARNING — displayd not found; using boot-time layout only");
    }

    println!("compositor: creating desktop...");
    let mut desktop =
        alloc::boxed::Box::new(desktop::Desktop::new(fb_ptr, width, height, fb_info.pitch));
    println!("compositor: desktop created, initializing...");
    desktop.init_no_wallpaper();
    println!("compositor: desktop initialized");

    // Skip config restore in setup mode — no saved config on install media
    if !setup_mode {
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
                    println!(
                        "compositor: failed to restore saved resolution, keeping {}x{}",
                        width, height
                    );
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
    } else {
        desktop::theme::set_scale_factor(100);
        println!("compositor: setup mode — skipping config restore");
    }

    let (splash_x, splash_y) = ipc::cursor_takeover();
    desktop.set_cursor_pos(splash_x, splash_y);
    if desktop.has_hw_cursor() {
        desktop.init_hw_cursor();
        println!(
            "compositor: HW cursor enabled (pos={},{})",
            splash_x, splash_y
        );
    } else {
        println!("compositor: SW cursor (pos={},{})", splash_x, splash_y);
    }

    for attempt in 0..3u32 {
        desktop.compositor.damage_all();
        desktop.compose();
        desktop
            .compositor
            .gpu_cmds
            .push([8, 0, 0, 0, 0, 0, 0, 0, 0]);
        desktop.compositor.flush_gpu();
        if attempt == 0 {
            println!(
                "compositor: desktop drawn ({}x{})",
                desktop.screen_width, desktop.screen_height
            );
        } else {
            println!("compositor: initial compose retry #{}", attempt);
        }
        if attempt < 2 {
            anyos_std::process::sleep(50);
        }
    }

    let compositor_channel = ipc::evt_chan_create("compositor");
    let compositor_sub = ipc::evt_chan_subscribe(compositor_channel, 0);
    println!(
        "compositor: event channel created (id={})",
        compositor_channel
    );

    let sys_sub = ipc::evt_sys_subscribe(0);
    desktop.compositor.damage_all();
    sys::boot_ready();

    unsafe {
        render::set_desktop_ptr(alloc::boxed::Box::into_raw(desktop));
        render::set_compositor_channel(compositor_channel);
    }
    spawn_render_thread();

    if !setup_mode {
        // Step 3: Load wallpaper (synchronous — must complete before init)
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.load_default_wallpaper_pub();
        desktop.compositor.damage_all();
        release_lock();
        signal_render();
        println!("compositor: wallpaper loaded");

        // Process deferred wallpaper immediately so it's visible before init
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        if desktop.wallpaper_pending {
            desktop.process_deferred_wallpaper();
        }
        desktop.compositor.damage_all();
        release_lock();
        signal_render();

        // Step 4: Start init process (login will be deferred until init completes)
        config::launch_login_services();
    }

    let mut init_pending = false;
    if !setup_mode {
        let init_tid = process::spawn("/System/init", "");
        if init_tid != u32::MAX {
            spawn_init_waiter(init_tid);
            init_pending = true;
            println!(
                "compositor: init spawned (TID={}), login deferred until init completes",
                init_tid
            );
        } else {
            println!("compositor: WARNING — /System/init could not be spawned");
        }
    }

    // Login is NOT spawned yet — will be spawned after init completes (in management_loop)
    let mut login_tid = if setup_mode || init_pending {
        u32::MAX
    } else {
        process::spawn("/System/login", "")
    };
    let mut login_pending = login_tid != u32::MAX;
    let mut dock_spawned = setup_mode;
    if setup_mode {
        println!("compositor: setup mode — loading wallpaper, launching installer");
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.set_menubar_visible(true);
        desktop.load_wallpaper("/media/wallpapers/setup.png");
        desktop.compositor.damage_all();
        release_lock();
        signal_render();
        process::spawn("/Applications/Installer.app/Installer", "");
    } else {
        // Hide menubar until login completes (init must finish first, then login)
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        desktop.set_menubar_visible(false);
        release_lock();
        if login_pending {
            println!("compositor: login window spawned, waiting for authentication...");
        } else if init_pending {
            println!("compositor: waiting for init to complete before showing login...");
        } else {
            println!("compositor: WARNING — neither init nor login could be spawned");
        }
    }

    let mut service_tids: Vec<u32> = Vec::new();
    println!("compositor: entering main loop (multi-threaded)");

    management_loop(
        compositor_channel,
        compositor_sub,
        sys_sub,
        &mut init_pending,
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
    #[cfg(target_arch = "x86_64")]
    let render_stack_top = ((render_stack_base + render_stack_size) & !0xF) - 8;
    #[cfg(target_arch = "aarch64")]
    let render_stack_top = (render_stack_base + render_stack_size) & !0xF;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // On AArch64 the stdlib thread trampoline installs its own return slots.
        *(render_stack_top as *mut usize) = process::thread_exit_stub_addr();
    }
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
