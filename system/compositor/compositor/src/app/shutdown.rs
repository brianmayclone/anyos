//! Shutdown and restart handling.

use anyos_std::println;
use anyos_std::process;
use anyos_std::Vec;

use crate::compositor;
use crate::render::{acquire_lock, desktop_ref, release_lock};

pub(crate) fn perform_shutdown(mode: u8, service_tids: &mut Vec<u32>) {
    let action = if mode == 2 { "restart" } else { "shutdown" };
    println!(
        "compositor: {} requested — terminating all processes...",
        action
    );

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

    for &tid in service_tids.iter() {
        if !tids_to_kill.contains(&tid) {
            tids_to_kill.push(tid);
        }
    }
    service_tids.clear();

    for &tid in &tids_to_kill {
        process::kill(tid);
    }
    process::sleep(300);

    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        let remaining: Vec<u32> = desktop.windows.iter().map(|w| w.id).collect();
        for id in remaining {
            desktop.destroy_window(id);
        }
        release_lock();
    }

    println!("compositor: all processes terminated, drawing shutdown screen...");

    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        let c = &mut desktop.compositor;
        let fb_ptr = c.fb_ptr;
        let w = c.fb_width;
        let h = c.fb_height;
        let pitch = c.fb_pitch;
        let fb_stride = (pitch / 4) as usize;

        const TOP: u32 = 0xFF0D0D14;
        const BOT: u32 = 0xFF020204;
        let top_r = ((TOP >> 16) & 0xFF) as i32;
        let top_g = ((TOP >> 8) & 0xFF) as i32;
        let top_b = (TOP & 0xFF) as i32;
        let bot_r = ((BOT >> 16) & 0xFF) as i32;
        let bot_g = ((BOT >> 8) & 0xFF) as i32;
        let bot_b = (BOT & 0xFF) as i32;

        for y in 0..h {
            let t = y as i32;
            let hh = h.max(1) as i32;
            let r = (top_r + (bot_r - top_r) * t / hh) as u32;
            let g = (top_g + (bot_g - top_g) * t / hh) as u32;
            let b = (top_b + (bot_b - top_b) * t / hh) as u32;
            let color = 0xFF000000 | (r << 16) | (g << 8) | b;
            for x in 0..w {
                unsafe {
                    core::ptr::write_volatile(
                        fb_ptr.add(y as usize * fb_stride + x as usize),
                        color,
                    );
                }
            }
        }

        draw_shutdown_logo(
            fb_ptr, w, h, fb_stride, top_r, top_g, top_b, bot_r, bot_g, bot_b,
        );

        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("sfence", options(nostack, preserves_flags));
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("dsb st", options(nostack, preserves_flags));
        }

        c.gpu_cmds
            .push([compositor::gpu::GPU_CURSOR_SHOW, 0, 0, 0, 0, 0, 0, 0, 0]);
        c.gpu_cmds
            .push([compositor::gpu::GPU_UPDATE, 0, 0, w, h, 0, 0, 0, 0]);
        c.flush_gpu();

        release_lock();
    }

    process::sleep(100);
    println!("compositor: invoking kernel {}...", action);

    if mode == 2 {
        process::reboot();
    } else {
        process::shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_shutdown_logo(
    fb_ptr: *mut u32,
    fb_w: u32,
    fb_h: u32,
    fb_stride: usize,
    top_r: i32,
    top_g: i32,
    top_b: i32,
    bot_r: i32,
    bot_g: i32,
    bot_b: i32,
) {
    let Some(data) = crate::desktop::read_file_bounded("/System/media/anyos_w.png", 64 * 1024)
    else {
        return;
    };

    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => return,
    };
    let src_w = info.width;
    let src_h = info.height;
    let pixel_count = (src_w * src_h) as usize;
    if pixel_count == 0 || pixel_count > 64 * 1024 {
        return;
    }

    let mut pixels = alloc::vec![0u32; pixel_count];
    let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
    if libimage_client::decode(&data, &mut pixels, &mut scratch).is_err() {
        return;
    }

    let start_x = if fb_w > src_w { (fb_w - src_w) / 2 } else { 0 };
    let start_y = if fb_h > src_h { (fb_h - src_h) / 2 } else { 0 };
    let hh = fb_h.max(1) as i32;
    for ly in 0..src_h {
        let sy = start_y + ly;
        if sy >= fb_h {
            break;
        }

        let t = sy as i32;
        let bg_r = (top_r + (bot_r - top_r) * t / hh) as u32;
        let bg_g = (top_g + (bot_g - top_g) * t / hh) as u32;
        let bg_b = (top_b + (bot_b - top_b) * t / hh) as u32;

        for lx in 0..src_w {
            let sx = start_x + lx;
            if sx >= fb_w {
                break;
            }

            let argb = pixels[(ly * src_w + lx) as usize];
            let a = (argb >> 24) & 0xFF;
            if a == 0 {
                continue;
            }

            let sr = (argb >> 16) & 0xFF;
            let sg = (argb >> 8) & 0xFF;
            let sb = argb & 0xFF;

            let out = if a >= 255 {
                0xFF000000 | (sr << 16) | (sg << 8) | sb
            } else {
                let inv = 255 - a;
                let or = (sr * a + bg_r * inv) / 255;
                let og = (sg * a + bg_g * inv) / 255;
                let ob = (sb * a + bg_b * inv) / 255;
                0xFF000000 | (or << 16) | (og << 8) | ob
            };

            unsafe {
                core::ptr::write_volatile(fb_ptr.add(sy as usize * fb_stride + sx as usize), out);
            }
        }
    }
}
