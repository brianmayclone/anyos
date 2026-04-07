//! Display setup and userspace launch for x86_64 boot.

use crate::arch;
use crate::boot::{GPU_ACCEL, GPU_HW_CURSOR, SETUP_MODE};
use crate::boot_info::BootInfo;
use crate::drivers;
use crate::serial_println;
use crate::task;
use alloc::string::String;
use core::sync::atomic::Ordering;

pub(super) fn start_userspace(_boot_info: &BootInfo, nogui: bool) -> ! {
    let Some(framebuffer) = drivers::framebuffer::info() else {
        serial_println!("FATAL: No framebuffer available, cannot start userspace.");
        loop {
            arch::hal::halt();
        }
    };

    if !drivers::gpu::is_available() {
        drivers::gpu::bochs_vga::init(
            framebuffer.addr as u32,
            framebuffer.width,
            framebuffer.height,
            framebuffer.pitch,
        );
    }

    log_gpu_driver();
    apply_requested_resolution();
    auto_apply_native_resolution();

    if drivers::gpu::with_gpu(|gpu| gpu.has_accel()).unwrap_or(false) {
        GPU_ACCEL.store(true, Ordering::Relaxed);
    }

    arch::hal::disable_interrupts();
    task::scheduler::spawn(task::cpu_monitor::start, 10, "cpu_monitor");
    task::scheduler::spawn(drivers::usb::poll_thread, 50, "usb_poll");

    drivers::boot_console::stop_spinner();

    if nogui {
        start_textmode_console();
    } else {
        start_graphical_userspace(framebuffer.width, framebuffer.height);
    }

    task::scheduler::run();
}

fn log_gpu_driver() {
    if let Some(name) = drivers::gpu::with_gpu(|gpu| {
        let mut name = String::new();
        name.push_str(gpu.name());
        name
    }) {
        serial_println!("[OK] GPU driver: {}", name);
    }
}

fn apply_requested_resolution() {
    if let Some((width, height)) = drivers::gpu::preferred_resolution() {
        if let Some(Some((new_w, new_h, pitch, addr))) =
            drivers::gpu::with_gpu(|gpu| gpu.set_mode(width, height, 32))
        {
            update_display_geometry(new_w, new_h, pitch, addr);
            serial_println!("[OK] Applied boot resolution: {}x{}", new_w, new_h);
        }
    }
}

fn auto_apply_native_resolution() {
    drivers::monitor::detect_and_register();

    let is_3d = drivers::gpu::with_gpu(|gpu| gpu.has_3d()).unwrap_or(false);
    if is_3d || drivers::gpu::preferred_resolution().is_some() {
        return;
    }

    let mut target = None;
    if let Some(monitor) = drivers::monitor::get_monitor(0) {
        let width = monitor.edid.preferred_width;
        let height = monitor.edid.preferred_height;
        if width >= 1024 && height >= 768 {
            target = Some((width, height));
        }
    }

    if target.is_none() {
        if let Some((width, height, true)) =
            drivers::gpu::with_gpu(|gpu| gpu.display_info(0)).flatten()
        {
            if width >= 1024 && height >= 768 {
                target = Some((width, height));
            }
        }
    }

    let Some((target_w, target_h)) = target else {
        return;
    };
    let Some((active_w, active_h, _, _)) = drivers::gpu::with_gpu(|gpu| gpu.get_mode()) else {
        return;
    };
    if (active_w, active_h) == (target_w, target_h) {
        return;
    }

    if let Some(Some((new_w, new_h, pitch, addr))) =
        drivers::gpu::with_gpu(|gpu| gpu.set_mode(target_w, target_h, 32))
    {
        serial_println!("[OK] Applied native monitor resolution: {}x{}", new_w, new_h);
        update_display_geometry(new_w, new_h, pitch, addr);
    }
}

fn update_display_geometry(width: u32, height: u32, pitch: u32, addr: u32) {
    drivers::framebuffer::update(addr as u64, pitch, width, height, 32);
    drivers::gpu::update_cursor_bounds(width, height);
    drivers::input::vmmouse::update_screen_size(width, height);
    drivers::vmmdev::set_screen_size(width as u16, height as u16);
}

fn start_textmode_console() {
    drivers::textcon::init();

    if let Some((width, height, _, _)) = drivers::gpu::with_gpu(|gpu| gpu.get_mode()) {
        drivers::gpu::with_gpu(|gpu| {
            gpu.transfer_rect(0, 0, width, height);
            gpu.flush_display(0, 0, width, height);
        });
    }

    serial_println!("[OK] NOGUI mode: skipping compositor/init");
    match task::loader::load_and_run("/System/bin/textmode_console", "textmode_console") {
        Ok(tid) => serial_println!("[OK] textmode_console spawned (TID={})", tid),
        Err(err) => serial_println!("  WARN: Failed to load textmode_console: {}", err),
    }
}

fn start_graphical_userspace(fallback_width: u32, fallback_height: u32) {
    let (display_width, display_height) = drivers::gpu::with_gpu(|gpu| {
        let (width, height, _, _) = gpu.get_mode();
        (width, height)
    })
    .unwrap_or((fallback_width, fallback_height));

    if drivers::gpu::with_gpu(|gpu| gpu.has_hw_cursor()).unwrap_or(false) {
        GPU_HW_CURSOR.store(true, Ordering::Relaxed);
        drivers::gpu::enable_splash_cursor(display_width, display_height);
    }

    flush_gpu_before_compositor();

    let setup_mode = SETUP_MODE.load(Ordering::Relaxed);
    spawn_compositor(setup_mode);

    if setup_mode {
        serial_println!("[SETUP] Setup mode - compositor will launch installer");
    } else {
        match task::loader::load_and_run("/System/init", "init") {
            Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
            Err(err) => serial_println!("  WARN: Failed to load /System/init: {}", err),
        }
    }

    serial_println!("Userspace started, entering scheduler...");
}

fn flush_gpu_before_compositor() {
    if let Some((width, height, _, _)) = drivers::gpu::with_gpu(|gpu| gpu.get_mode()) {
        drivers::gpu::with_gpu(|gpu| {
            gpu.transfer_rect(0, 0, width, height);
            gpu.flush_display(0, 0, width, height);
        });
        serial_println!("[OK] Pre-compositor GPU flush ({}x{})", width, height);
    }
}

fn spawn_compositor(setup_mode: bool) {
    let result = if setup_mode {
        task::loader::load_and_run_with_args(
            "/System/compositor/compositor",
            "compositor",
            "compositor setupmode",
        )
    } else {
        task::loader::load_and_run("/System/compositor/compositor", "compositor")
    };

    match result {
        Ok(tid) if setup_mode => {
            serial_println!("[OK] Compositor spawned in setup mode (TID={})", tid)
        }
        Ok(tid) => serial_println!("[OK] Userspace compositor spawned (TID={})", tid),
        Err(err) => serial_println!("  WARN: Failed to load compositor: {}", err),
    }
}
