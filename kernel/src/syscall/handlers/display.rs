//! Display, GPU, audio, and compositor syscall handlers.
//!
//! Covers screen info, resolution management, GPU acceleration (2D/3D),
//! VRAM mapping, audio output, compositor registration, framebuffer
//! mapping, input polling, screen capture, and boot readiness.

#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;
#[allow(unused_imports)]
use super::{is_compositor, is_display_owner, COMPOSITOR_PD, COMPOSITOR_TID, DISPLAY_OWNER_PD};

use core::sync::atomic::Ordering;

// =========================================================================
// Screen info / Resolution
// =========================================================================

#[cfg(target_arch = "x86_64")]
pub fn sys_screen_size(buf_ptr: u64) -> u32 {
    if buf_ptr == 0 {
        return u32::MAX;
    }
    match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some((w, h, _pitch, _addr)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = w;
                *buf.add(1) = h;
            }
            0
        }
        None => {
            // Fallback to boot framebuffer info
            match crate::drivers::framebuffer::info() {
                Some(fb) => {
                    unsafe {
                        let buf = buf_ptr as *mut u32;
                        *buf = fb.width;
                        *buf.add(1) = fb.height;
                    }
                    0
                }
                None => u32::MAX,
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_screen_size(buf_ptr: u64) -> u32 {
    if buf_ptr == 0 {
        return u32::MAX;
    }
    match crate::drivers::framebuffer::info() {
        Some(fb) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = fb.width;
                *buf.add(1) = fb.height;
            }
            0
        }
        None => u32::MAX,
    }
}

/// sys_set_resolution - Change display resolution via GPU driver.
///
/// IMPORTANT: set_mode() allocates low-identity framebuffer pages via
/// alloc_contiguous() and zeroes them using identity-mapped access
/// (fb_phys as *mut u8). During a syscall, the CPU uses the user process's CR3,
/// which only identity-maps the first 64 MiB (PD[0..31]). We switch to the
/// kernel CR3 for the duration of set_mode() so legacy GPU code uses the
/// kernel's identity window consistently.
#[cfg(target_arch = "x86_64")]
pub fn sys_set_resolution(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return u32::MAX;
    }

    // Switch to kernel CR3 with interrupts disabled to prevent context-switch
    // races (another CPU could restore a different CR3 via the scheduler).
    let result: Option<Option<(u32, u32, u32, u32)>>;
    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));
        let old_cr3 = crate::memory::virtual_mem::current_cr3();
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3, options(nostack));

        result = crate::drivers::gpu::with_gpu(|g| g.set_mode(width, height, 32));

        core::arch::asm!("mov cr3, {}", in(reg) old_cr3, options(nostack));
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    match result {
        Some(Some(_)) => {
            // Update kernel-side cursor bounds for the new resolution
            crate::drivers::gpu::update_cursor_bounds(width, height);
            // Update VMMDev screen size for absolute mouse coordinate scaling
            crate::drivers::vmmdev::set_screen_size(width as u16, height as u16);
            // Update vmmouse screen size for backdoor coordinate scaling
            crate::drivers::input::vmmouse::update_screen_size(width, height);
            // Notify all subscribers about the resolution change
            crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
                crate::ipc::event_bus::EVT_RESOLUTION_CHANGED,
                width,
                height,
                0,
                0,
            ));
            0
        }
        _ => u32::MAX,
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_set_resolution(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 {
        return u32::MAX;
    }
    match crate::drivers::framebuffer::info() {
        Some(fb) if fb.width == width && fb.height == height => 0,
        _ => u32::MAX,
    }
}

/// sys_list_resolutions - List supported display resolutions.
/// Writes (width, height) pairs as u32 pairs to buf. Returns number of modes.
#[cfg(target_arch = "x86_64")]
pub fn sys_list_resolutions(buf_ptr: u64, buf_len: u32) -> u32 {
    let modes = crate::drivers::gpu::with_gpu(|g| {
        let m = g.supported_modes();
        // Copy to a fixed-size buffer to return outside the lock
        let mut result = [(0u32, 0u32); 16];
        let count = m.len().min(16);
        for i in 0..count {
            result[i] = m[i];
        }
        (result, count)
    });

    match modes {
        Some((mode_list, count)) => {
            if buf_ptr != 0 && buf_len > 0 {
                let max_entries = (buf_len as usize / 8).min(count); // 8 bytes per (u32, u32)
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    for i in 0..max_entries {
                        *buf.add(i * 2) = mode_list[i].0;
                        *buf.add(i * 2 + 1) = mode_list[i].1;
                    }
                }
            }
            count as u32
        }
        None => 0, // No GPU driver registered
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_list_resolutions(buf_ptr: u64, buf_len: u32) -> u32 {
    let Some(fb) = crate::drivers::framebuffer::info() else {
        return 0;
    };
    if buf_ptr != 0 && buf_len >= 8 {
        unsafe {
            let buf = buf_ptr as *mut u32;
            *buf = fb.width;
            *buf.add(1) = fb.height;
        }
    }
    1
}

/// sys_gpu_info - Get GPU driver info. Writes driver name to buf. Returns name length.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_info(buf_ptr: u64, buf_len: u32) -> u32 {
    let name = crate::drivers::gpu::with_gpu(|g| {
        let mut s = alloc::string::String::new();
        s.push_str(g.name());
        s
    });

    match name {
        Some(n) => {
            if buf_ptr != 0 && buf_len > 0 {
                let bytes = n.as_bytes();
                let copy_len = bytes.len().min(buf_len as usize - 1);
                unsafe {
                    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len + 1);
                    buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
                    buf[copy_len] = 0; // null-terminate
                }
            }
            n.len() as u32
        }
        None => 0,
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_info(buf_ptr: u64, buf_len: u32) -> u32 {
    let name = if crate::drivers::arm::gpu::is_available() {
        "virtio-gpu-mmio"
    } else if crate::drivers::framebuffer::is_available() {
        "framebuffer"
    } else {
        return 0;
    };

    if buf_ptr != 0 && buf_len > 0 {
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(buf_len as usize - 1);
        unsafe {
            let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len + 1);
            buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            buf[copy_len] = 0;
        }
    }
    name.len() as u32
}

// =========================================================================
// GPU acceleration queries
// =========================================================================

/// SYS_GPU_HAS_ACCEL: Query if GPU acceleration is available.
pub fn sys_gpu_has_accel() -> u32 {
    use core::sync::atomic::Ordering;
    if crate::GPU_ACCEL.load(Ordering::Relaxed) {
        1
    } else {
        0
    }
}

/// SYS_GPU_HAS_HW_CURSOR: Query if GPU hardware cursor is available.
pub fn sys_gpu_has_hw_cursor() -> u32 {
    use core::sync::atomic::Ordering;
    if crate::GPU_HW_CURSOR.load(Ordering::Relaxed) {
        1
    } else {
        0
    }
}

// =========================================================================
// Audio
// =========================================================================

/// SYS_AUDIO_WRITE: Write PCM data to audio output.
/// arg1 = pointer to PCM data buffer, arg2 = length in bytes.
/// Returns number of bytes written.
#[cfg(target_arch = "x86_64")]
pub fn sys_audio_write(buf_ptr: u64, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return 0;
    }
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, buf_len as usize) };
    crate::drivers::audio::write_pcm(data) as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_audio_write(_buf_ptr: u64, _buf_len: u32) -> u32 {
    0
}

/// SYS_AUDIO_CTL: Audio control operations.
/// arg1 = command, arg2 = argument.
///   0 = stop playback
///   1 = set volume (arg2 = 0-100)
///   2 = get volume (returns 0-100)
///   3 = get status (returns 1 if playing, 0 if not)
///   4 = is available (returns 1 if audio hw present)
#[cfg(target_arch = "x86_64")]
pub fn sys_audio_ctl(cmd: u32, arg: u32) -> u32 {
    match cmd {
        0 => {
            crate::drivers::audio::stop();
            0
        }
        1 => {
            crate::drivers::audio::set_volume(arg as u8);
            0
        }
        2 => crate::drivers::audio::get_volume() as u32,
        3 => {
            if crate::drivers::audio::is_playing() {
                1
            } else {
                0
            }
        }
        4 => {
            if crate::drivers::audio::is_available() {
                1
            } else {
                0
            }
        }
        _ => u32::MAX,
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_audio_ctl(cmd: u32, _arg: u32) -> u32 {
    match cmd {
        4 => 0, // audio not available on ARM64 yet
        _ => u32::MAX,
    }
}

// =========================================================================
// Compositor-privileged syscalls
// =========================================================================

/// Register calling process as the compositor. First caller wins.
/// Returns 0 on success, u32::MAX if already registered.
pub fn sys_register_compositor() -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if COMPOSITOR_TID
        .compare_exchange(0, tid, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // Store the compositor's page directory so child threads (render thread)
        // are also recognized as compositor by is_compositor().
        if let Some(pd) = crate::task::scheduler::current_thread_page_directory() {
            COMPOSITOR_PD.store(pd.as_u64(), Ordering::SeqCst);
        }

        // Boost compositor to realtime priority so UI never stutters
        crate::task::scheduler::set_thread_priority(tid, 127);
        crate::serial_verbose_println!("[OK] Compositor registered (TID={}, priority=127)", tid);

        // Multi-monitor: vmmouse stays enabled. Absolute coords are
        // scoped to scanout 0's framebuffer dimensions, but the
        // compositor's apply_mouse_move_absolute path derives relative
        // deltas from successive absolute samples on multi-monitor
        // setups, so the cursor traverses virtual_desktop_bounds
        // correctly. Disabling vmmouse leaves QEMU with no way to
        // translate the host's absolute pointer position into PS/2
        // deltas without saturating at ±127, producing the runaway
        // drift to the bottom-right corner that we observed in the
        // [mouse-diag] traces under both `-display sdl` multi-window
        // and `-display gtk`.
        #[cfg(target_arch = "x86_64")]
        let advertised = crate::drivers::gpu::with_gpu(|g| g.display_count()).unwrap_or(1);
        #[cfg(target_arch = "aarch64")]
        let advertised = 1;
        if advertised > 1 {
            crate::serial_println!(
                "[OK] multi-monitor ({}): vmmouse stays enabled (delta-derivation in compositor)",
                advertised
            );
        }
        0
    } else {
        u32::MAX // Already registered
    }
}

/// Take over cursor from kernel splash mode. Compositor-only.
/// Disables the kernel's IRQ-driven cursor tracking, drains stale mouse events,
/// and returns the splash cursor position packed as (x << 16) | (y & 0xFFFF).
/// The compositor uses this to initialize its logical cursor to match the HW cursor.
#[cfg(target_arch = "x86_64")]
pub fn sys_cursor_takeover() -> u32 {
    if !is_compositor() {
        return 0;
    }
    let (x, y) = crate::drivers::gpu::splash_cursor_position();
    crate::drivers::gpu::disable_splash_cursor();
    crate::drivers::input::mouse::clear_buffer();
    crate::serial_verbose_println!("Compositor cursor takeover: splash pos ({}, {})", x, y);
    ((x as u16 as u32) << 16) | (y as u16 as u32)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_cursor_takeover() -> u32 {
    if !is_compositor() {
        return 0;
    }
    if let Some((_fb_virt, w, h)) = crate::drivers::arm::gpu::framebuffer_info() {
        let x = (w / 2) as u16 as u32;
        let y = (h / 2) as u16 as u32;
        return (x << 16) | y;
    }
    0
}

/// Map the GPU framebuffer into the compositor's address space.
/// ebx=out_info_ptr (pointer to FbMapInfo struct, 16 bytes).
/// Returns 0 on success, u32::MAX on failure.
///
/// FbMapInfo layout: { fb_vaddr: u32, width: u32, height: u32, pitch: u32 }
#[cfg(target_arch = "x86_64")]
pub fn sys_map_framebuffer(out_info_ptr: u64) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    // Get framebuffer info from GPU driver. The first-page address from
    // get_mode() is informational only; the actual per-page mapping
    // walks framebuffer_pages() to support scatter-gather framebuffers
    // that aren't physically contiguous (the runtime mode-change path
    // on virtio-gpu allocates pages individually so it doesn't fail on
    // long-running systems where contiguous physmem is fragmented).
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return u32::MAX,
    };
    let pages_vec: alloc::vec::Vec<u64> =
        match crate::drivers::gpu::with_gpu(|g| g.framebuffer_pages()) {
            Some(v) => v,
            None => return u32::MAX,
        };
    if pages_vec.is_empty() {
        return u32::MAX;
    }

    // Map the visible framebuffer into the compositor's address space at 0x20000000.
    // A fixed 16 MiB mapping is not enough for 4K-ish VirtIO modes
    // (for example 3828x2030x4 is about 29.6 MiB).
    let fb_user_base: u64 = 0x2000_0000;
    let fb_total_bytes = (height as usize).saturating_mul(pitch as usize);
    if fb_total_bytes == 0 {
        return u32::MAX;
    }
    let pages = (fb_total_bytes + crate::memory::FRAME_SIZE - 1) / crate::memory::FRAME_SIZE;
    if pages > pages_vec.len() {
        crate::serial_verbose_println!(
            "sys_map_framebuffer: page-list short ({} < {})",
            pages_vec.len(),
            pages
        );
        return u32::MAX;
    }

    for i in 0..pages {
        let phys_addr = crate::memory::address::PhysAddr::new(pages_vec[i]);
        let virt_addr = crate::memory::address::VirtAddr::new(
            fb_user_base + (i * crate::memory::FRAME_SIZE) as u64,
        );
        // Present + Writable + User + Write-Through (0x0F)
        crate::memory::virtual_mem::map_page(virt_addr, phys_addr, 0x0F);
    }

    // Write FbMapInfo struct to user memory
    if out_info_ptr != 0 {
        if !super::helpers::is_valid_user_ptr(out_info_ptr, 16) {
            return u32::MAX;
        }
        let info = unsafe { &mut *(out_info_ptr as *mut [u32; 4]) };
        info[0] = fb_user_base as u32;
        info[1] = width;
        info[2] = height;
        info[3] = pitch;
    }

    crate::serial_verbose_println!(
        "[OK] Framebuffer mapped to compositor at {:#010x} ({}x{}, pitch={}, phys={:#x})",
        fb_user_base,
        width,
        height,
        pitch,
        fb_phys
    );
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_map_framebuffer(out_info_ptr: u64) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    let (fb_phys, _fb_virt, width, height, pitch) =
        match crate::drivers::arm::gpu::framebuffer_mapping_info() {
            Some(info) => info,
            None => return u32::MAX,
        };

    let fb_user_base: u64 = 0x2000_0000;
    let fb_map_size = ((pitch as usize * height as usize) + 0xFFF) & !0xFFF;
    let pages = fb_map_size / crate::memory::FRAME_SIZE;

    for i in 0..pages {
        let phys_addr =
            crate::memory::address::PhysAddr::new(fb_phys + (i * crate::memory::FRAME_SIZE) as u64);
        let virt_addr = crate::memory::address::VirtAddr::new(
            fb_user_base + (i * crate::memory::FRAME_SIZE) as u64,
        );
        let _ = crate::memory::virtual_mem::map_page(virt_addr, phys_addr, 0x0F);
    }

    if out_info_ptr != 0 {
        if !super::helpers::is_valid_user_ptr(out_info_ptr, 16) {
            return u32::MAX;
        }
        let info = unsafe { &mut *(out_info_ptr as *mut [u32; 4]) };
        info[0] = fb_user_base as u32;
        info[1] = width;
        info[2] = height;
        info[3] = pitch;
    }

    0
}

/// Submit GPU acceleration commands from the compositor.
/// ebx=cmd_buf_ptr, ecx=cmd_count.
/// Returns number of commands executed, or u32::MAX on error.
///
/// Each command is 36 bytes: { cmd_type: u32, args: [u32; 8] }
/// Command types: 1=UPDATE, 2=FILL_RECT, 3=COPY_RECT, 4=CURSOR_MOVE,
///                5=CURSOR_SHOW, 6=DEFINE_CURSOR, 7=FLIP
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_command(cmd_buf_ptr: u64, cmd_count: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if cmd_count == 0 || cmd_buf_ptr == 0 {
        return 0;
    }

    let count = cmd_count.min(256) as usize; // Cap at 256 commands per call
    let byte_size = count * 36; // 9 u32s * 4 bytes each
    if !is_valid_user_ptr(cmd_buf_ptr as u64, byte_size as u64) {
        return 0;
    }
    let cmds = unsafe { core::slice::from_raw_parts(cmd_buf_ptr as *const [u32; 9], count) };
    let mut last_cmd_type = 0u32;

    // Process all commands in a single GPU lock acquisition.
    // UPDATE commands use transfer_rect (no flush) and accumulate a
    // bounding box; a single flush_display at the end covers them all.
    let result = crate::drivers::gpu::with_gpu(|g| {
        let mut executed = 0u32;
        // Bounding box for batched UPDATE transfers
        let mut flush_x0 = u32::MAX;
        let mut flush_y0 = u32::MAX;
        let mut flush_x1 = 0u32;
        let mut flush_y1 = 0u32;

        for cmd in cmds {
            let cmd_type = cmd[0];
            last_cmd_type = cmd_type;
            let ok = match cmd_type {
                1 => {
                    // UPDATE(x, y, w, h) — accumulate bbox, defer transfer+flush
                    let (x, y, w, h) = (cmd[1], cmd[2], cmd[3], cmd[4]);
                    // Only expand bounding box; transfer is batched at the end
                    if w > 0 && h > 0 {
                        flush_x0 = flush_x0.min(x);
                        flush_y0 = flush_y0.min(y);
                        flush_x1 = flush_x1.max(x + w);
                        flush_y1 = flush_y1.max(y + h);
                    }
                    true
                }
                2 => {
                    // FILL_RECT(x, y, w, h, color)
                    g.accel_fill_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5])
                }
                3 => {
                    // COPY_RECT(sx, sy, dx, dy, w, h)
                    g.accel_copy_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5], cmd[6])
                }
                4 => {
                    // CURSOR_MOVE(x, y)
                    if !crate::drivers::gpu::is_splash_cursor_active() {
                        g.move_cursor(cmd[1], cmd[2]);
                    }
                    true
                }
                5 => {
                    // CURSOR_SHOW(visible)
                    g.show_cursor(cmd[1] != 0);
                    true
                }
                6 => {
                    // DEFINE_CURSOR(w, h, hotx, hoty, pixels_ptr_lo, pixels_ptr_hi, pixel_count)
                    let w = cmd[1];
                    let h = cmd[2];
                    let hotx = cmd[3];
                    let hoty = cmd[4];
                    let ptr = (cmd[5] as u64) | ((cmd[6] as u64) << 32);
                    let count = cmd[7] as usize;
                    if w == 0 || h == 0 || count == 0 || ptr == 0 {
                        false
                    } else if count != (w * h) as usize {
                        false
                    } else if !is_valid_user_ptr(ptr, (count * 4) as u64) {
                        crate::serial_verbose_println!(
                            "GPU DEFINE_CURSOR: invalid pixel ptr {:#x} count={}",
                            ptr,
                            count
                        );
                        false
                    } else {
                        let pixels =
                            unsafe { core::slice::from_raw_parts(ptr as *const u32, count) };
                        g.define_cursor(w, h, hotx, hoty, pixels);
                        true
                    }
                }
                7 => {
                    // FLIP
                    g.flip();
                    true
                }
                8 => {
                    // SYNC
                    g.sync();
                    true
                }
                9 => {
                    // VRAM_INFO
                    true
                }
                11 => {
                    // CURSOR_MOVE_OUTPUT(output_id, x, y) — multi-monitor
                    // HW-cursor routing. Compositor sends this when
                    // the cursor crosses output boundaries; the kernel
                    // forwards to the driver's per-scanout routine.
                    if !crate::drivers::gpu::is_splash_cursor_active() {
                        g.move_cursor_for_output(cmd[1], cmd[2], cmd[3]);
                    }
                    true
                }
                12 => {
                    // CURSOR_SHOW_OUTPUT(output_id, visible) — show
                    // or hide the HW cursor on a specific scanout.
                    g.show_cursor_for_output(cmd[1], cmd[2] != 0);
                    true
                }
                _ => false,
            };
            if ok {
                executed += 1;
            }
        }

        // Single batched transfer + flush for all UPDATE rects
        if flush_x0 < flush_x1 && flush_y0 < flush_y1 {
            let bw = flush_x1 - flush_x0;
            let bh = flush_y1 - flush_y0;
            g.transfer_rect(flush_x0, flush_y0, bw, bh);
            g.flush_display(flush_x0, flush_y0, bw, bh);
        }

        executed
    });

    let executed = result.unwrap_or(0);
    if executed == 0 && last_cmd_type != 0 {
        crate::serial_println!(
            "[gpu] SYS_GPU_COMMAND returned 0 after last_cmd={} count={}",
            last_cmd_type,
            count
        );
    }
    executed
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_command(cmd_buf_ptr: u64, cmd_count: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if cmd_count == 0 || cmd_buf_ptr == 0 {
        return 0;
    }

    let count = cmd_count.min(256) as usize;
    let byte_size = count * 36;
    if !is_valid_user_ptr(cmd_buf_ptr as u64, byte_size as u64) {
        return 0;
    }

    let cmds = unsafe { core::slice::from_raw_parts(cmd_buf_ptr as *const [u32; 9], count) };
    let mut flush_x0 = u32::MAX;
    let mut flush_y0 = u32::MAX;
    let mut flush_x1 = 0u32;
    let mut flush_y1 = 0u32;
    let mut executed = 0u32;
    let mut update_count = 0u32;
    let mut sync_count = 0u32;
    let mut cursor_count = 0u32;

    for cmd in cmds {
        match cmd[0] {
            1 => {
                let (x, y, w, h) = (cmd[1], cmd[2], cmd[3], cmd[4]);
                if w > 0 && h > 0 {
                    flush_x0 = flush_x0.min(x);
                    flush_y0 = flush_y0.min(y);
                    flush_x1 = flush_x1.max(x + w);
                    flush_y1 = flush_y1.max(y + h);
                }
                update_count += 1;
                executed += 1;
            }
            4 => {
                crate::drivers::arm::gpu::move_cursor(cmd[1], cmd[2]);
                cursor_count += 1;
                executed += 1;
            }
            5 => {
                crate::drivers::arm::gpu::show_cursor(cmd[1] != 0);
                cursor_count += 1;
                executed += 1;
            }
            6 => {
                let w = cmd[1];
                let h = cmd[2];
                let hotx = cmd[3];
                let hoty = cmd[4];
                let ptr = (cmd[5] as u64) | ((cmd[6] as u64) << 32);
                let count = cmd[7] as usize;
                if w > 0
                    && h > 0
                    && ptr != 0
                    && count == (w * h) as usize
                    && is_valid_user_ptr(ptr, (count * 4) as u64)
                {
                    let pixels = unsafe { core::slice::from_raw_parts(ptr as *const u32, count) };
                    crate::drivers::arm::gpu::define_cursor(w, h, hotx, hoty, pixels);
                    executed += 1;
                }
            }
            7 | 8 => {
                if let Some((_fb_virt, w, h)) = crate::drivers::arm::gpu::framebuffer_info() {
                    crate::drivers::arm::gpu::flush(0, 0, w, h);
                    sync_count += 1;
                    executed += 1;
                }
            }
            _ => {}
        }
    }

    if flush_x0 < flush_x1 && flush_y0 < flush_y1 {
        crate::drivers::arm::gpu::flush(
            flush_x0,
            flush_y0,
            flush_x1 - flush_x0,
            flush_y1 - flush_y0,
        );
    }

    executed
}

/// Poll raw input events for the compositor.
/// ebx=buf_ptr (array of RawInputEvent), ecx=max_events.
/// Returns number of events written.
///
/// RawInputEvent layout (20 bytes): { event_type: u32, arg0-arg3: u32 }
/// Event types:
///   1 = KEY_DOWN:     arg0=scancode, arg1=char_value, arg2=modifiers
///   2 = KEY_UP:       arg0=scancode, arg1=char_value, arg2=modifiers
///   3 = MOUSE_MOVE:   arg0=dx(i32), arg1=dy(i32)
///   4 = MOUSE_BUTTON: arg0=buttons, arg1=1(down)/0(up)
///   5 = MOUSE_SCROLL: arg0=dz(i32)
#[cfg(target_arch = "x86_64")]
pub fn sys_input_poll(buf_ptr: u64, max_events: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if buf_ptr == 0 || max_events == 0 {
        return 0;
    }

    let max = max_events.min(256) as usize;
    let byte_size = max * 20; // 5 u32s * 4 bytes each
    if !is_valid_user_ptr(buf_ptr as u64, byte_size as u64) {
        return 0;
    }
    let events = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut [u32; 5], max) };
    let mut count = 0usize;

    // Drain keyboard events
    while count < max {
        match crate::drivers::input::keyboard::read_event() {
            Some(key_evt) => {
                let event_type: u32 = if key_evt.pressed { 1 } else { 2 };
                let char_val = match key_evt.key {
                    crate::drivers::input::keyboard::Key::Char(c) => c as u32,
                    crate::drivers::input::keyboard::Key::Enter => 0x0D,
                    crate::drivers::input::keyboard::Key::Backspace => 0x08,
                    crate::drivers::input::keyboard::Key::Tab => 0x09,
                    crate::drivers::input::keyboard::Key::Escape => 0x1B,
                    crate::drivers::input::keyboard::Key::Space => 0x20,
                    crate::drivers::input::keyboard::Key::Delete => 0x7F,
                    _ => 0,
                };
                let mods = (key_evt.modifiers.shift as u32)
                    | ((key_evt.modifiers.ctrl as u32) << 1)
                    | ((key_evt.modifiers.alt as u32) << 2)
                    | ((key_evt.modifiers.caps_lock as u32) << 3)
                    | ((key_evt.modifiers.altgr as u32) << 4);

                // Virtual scancodes for E0-prefixed multimedia keys
                // (raw scancodes 0x30/0x2E/0x20 collide with letter keys)
                let scancode_out = match key_evt.key {
                    crate::drivers::input::keyboard::Key::VolumeUp => 0x130u32,
                    crate::drivers::input::keyboard::Key::VolumeDown => 0x12E,
                    crate::drivers::input::keyboard::Key::VolumeMute => 0x120,
                    crate::drivers::input::keyboard::Key::LeftSuper => 0x15B,
                    crate::drivers::input::keyboard::Key::RightSuper => 0x15C,
                    _ => key_evt.scancode as u32,
                };
                events[count] = [event_type, scancode_out, char_val, mods, 0];
                count += 1;
            }
            None => break,
        }
    }

    // Poll absolute mouse position from hypervisor (VMMDev or SVGA cursor bypass).
    // Do this before draining PS/2 events so absolute position is always fresh.
    //
    // Skip on multi-monitor: both sources report coordinates scoped to the
    // primary scanout's framebuffer dimensions — there's no per-output
    // tagging, so on a multi-head setup the cursor would get warped onto
    // primary-relative positions every poll, fighting against the PS/2
    // relative path that correctly traverses the virtual desktop. Same
    // rationale as vmmouse::force_disable() in sys_register_compositor.
    let multi_monitor = crate::drivers::gpu::with_gpu(|g| g.display_count()).unwrap_or(1) > 1;
    if !multi_monitor {
        if crate::drivers::vmmdev::is_available() {
            if let Some((x, y, _btns)) = crate::drivers::vmmdev::poll_mouse() {
                // Position-only: button state comes from IRQ-driven inject_absolute
                crate::drivers::input::mouse::inject_position(x, y);
            }
        }
        // VMware SVGA FIFO cursor bypass: host writes cursor pos to FIFO memory
        if let Some((x, y)) = crate::drivers::gpu::vmware_svga::poll_cursor() {
            // Position-only: button state comes from IRQ-driven inject_absolute
            crate::drivers::input::mouse::inject_position(x, y);
        }
    }

    // Drain mouse events
    while count < max {
        match crate::drivers::input::mouse::read_event() {
            Some(mouse_evt) => {
                use crate::drivers::input::mouse::MouseEventType;
                // Multi-monitor: stash the producing output_id in arg3
                // for every mouse event. Legacy paths pass 0xFF
                // (OUTPUT_AGNOSTIC); per-output virtio-input devices
                // pass their bound scanout id. The compositor uses
                // this to translate absolute coords (event_type 6)
                // into the virtual desktop via output.virtual_x/y.
                let oid = mouse_evt.output_id as u32;
                let (event_type, arg0, arg1, arg2, arg3) = match mouse_evt.event_type {
                    MouseEventType::Move => {
                        (3u32, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, oid)
                    }
                    MouseEventType::MoveAbsolute => {
                        // event_type 6 = absolute position (pixel coords)
                        (6u32, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, oid)
                    }
                    MouseEventType::ButtonDown => {
                        let btns = (mouse_evt.buttons.left as u32)
                            | ((mouse_evt.buttons.right as u32) << 1)
                            | ((mouse_evt.buttons.middle as u32) << 2);
                        (4, btns, 1, mouse_evt.dx as u32, mouse_evt.dy as u32)
                    }
                    MouseEventType::ButtonUp => {
                        let btns = (mouse_evt.buttons.left as u32)
                            | ((mouse_evt.buttons.right as u32) << 1)
                            | ((mouse_evt.buttons.middle as u32) << 2);
                        (4, btns, 0, mouse_evt.dx as u32, mouse_evt.dy as u32)
                    }
                    MouseEventType::Scroll => (5, mouse_evt.dz as u32, 0, 0, 0),
                };
                events[count] = [event_type, arg0, arg1, arg2, arg3];
                count += 1;
            }
            None => break,
        }
    }

    count as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_input_poll(buf_ptr: u64, max_events: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if buf_ptr == 0 || max_events == 0 {
        return 0;
    }

    let max = max_events.min(256) as usize;
    let byte_size = max * 20;
    if !is_valid_user_ptr(buf_ptr as u64, byte_size as u64) {
        return 0;
    }

    let events = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut [u32; 5], max) };
    let mut count = 0usize;

    while count < max {
        match crate::drivers::input::keyboard::read_event() {
            Some(key_evt) => {
                let event_type: u32 = if key_evt.pressed { 1 } else { 2 };
                let char_val = match key_evt.key {
                    crate::drivers::input::keyboard::Key::Char(c) => c as u32,
                    crate::drivers::input::keyboard::Key::Enter => 0x0D,
                    crate::drivers::input::keyboard::Key::Backspace => 0x08,
                    crate::drivers::input::keyboard::Key::Tab => 0x09,
                    crate::drivers::input::keyboard::Key::Escape => 0x1B,
                    crate::drivers::input::keyboard::Key::Space => 0x20,
                    crate::drivers::input::keyboard::Key::Delete => 0x7F,
                    _ => 0,
                };
                let mods = (key_evt.modifiers.shift as u32)
                    | ((key_evt.modifiers.ctrl as u32) << 1)
                    | ((key_evt.modifiers.alt as u32) << 2)
                    | ((key_evt.modifiers.caps_lock as u32) << 3)
                    | ((key_evt.modifiers.altgr as u32) << 4);
                let scancode_out = match key_evt.key {
                    crate::drivers::input::keyboard::Key::VolumeUp => 0x130u32,
                    crate::drivers::input::keyboard::Key::VolumeDown => 0x12E,
                    crate::drivers::input::keyboard::Key::VolumeMute => 0x120,
                    crate::drivers::input::keyboard::Key::LeftSuper => 0x15B,
                    crate::drivers::input::keyboard::Key::RightSuper => 0x15C,
                    _ => key_evt.scancode as u32,
                };
                events[count] = [event_type, scancode_out, char_val, mods, 0];
                count += 1;
            }
            None => break,
        }
    }

    while count < max {
        match crate::drivers::arm::input::pop_mouse_event() {
            Some(mouse_evt) => {
                use crate::drivers::arm::input::MouseEventType;
                events[count] = match mouse_evt.event_type {
                    MouseEventType::Move => [3, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, 0],
                    MouseEventType::MoveAbsolute => {
                        [6, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, 0]
                    }
                    MouseEventType::ButtonDown => [
                        4,
                        mouse_evt.buttons as u32,
                        1,
                        mouse_evt.dx as u32,
                        mouse_evt.dy as u32,
                    ],
                    MouseEventType::ButtonUp => [
                        4,
                        mouse_evt.buttons as u32,
                        0,
                        mouse_evt.dx as u32,
                        mouse_evt.dy as u32,
                    ],
                    MouseEventType::Scroll => [5, mouse_evt.dz as u32, 0, 0, 0],
                };
                count += 1;
            }
            None => break,
        }
    }

    count as u32
}

// =========================================================================
// Boot readiness / Screen capture
// =========================================================================

/// SYS_BOOT_READY: Signal from the compositor that the desktop is fully up.
pub fn sys_boot_ready() -> u32 {
    crate::serial_verbose_println!("[OK] Boot ready signal received from compositor");
    0
}

/// SYS_CAPTURE_SCREEN: Capture the current framebuffer contents to a user buffer.
/// arg1 = buf_ptr (pointer to u32 ARGB pixels)
/// arg2 = buf_size (buffer size in bytes)
/// arg3 = info_ptr (pointer to write [width: u32, height: u32])
/// Returns: 0 on success, 1 = no GPU, 2 = buffer too small.
#[cfg(target_arch = "x86_64")]
pub fn sys_capture_screen(buf_ptr: u64, buf_size: u32, info_ptr: u64) -> u32 {
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return 1,
    };

    // Always write dimensions + pitch to info struct (even if buffer too small),
    // so callers can probe the resolution without a full-size buffer.
    // info layout: [width: u32, height: u32, pitch: u32]
    if info_ptr != 0 {
        unsafe {
            let info = info_ptr as *mut u32;
            *info = width;
            *info.add(1) = height;
            *info.add(2) = pitch;
        }
    }

    let needed = width * height * 4;
    if buf_size < needed {
        return 2;
    }

    // Map framebuffer physical pages into the current process at 0x30000000
    // (read-only user access: PAGE_PRESENT | PAGE_USER).
    // Skip re-mapping if already mapped (check first page's PTE).
    let fb_map_base: u64 = 0x3000_0000;
    let fb_total_bytes = height as usize * pitch as usize;
    let fb_pages = (fb_total_bytes + 0xFFF) / 0x1000;

    let first_virt = crate::memory::address::VirtAddr::new(fb_map_base);
    let already_mapped = crate::memory::virtual_mem::read_pte(first_virt) & 0x01 != 0;
    if !already_mapped {
        for i in 0..fb_pages {
            let phys = crate::memory::address::PhysAddr::new(fb_phys as u64 + (i * 0x1000) as u64);
            let virt = crate::memory::address::VirtAddr::new(fb_map_base + (i * 0x1000) as u64);
            crate::memory::virtual_mem::map_page(virt, phys, 0x05);
        }
    }

    // Copy pixels row by row (pitch may differ from width*4)
    unsafe {
        let src = fb_map_base as *const u8;
        let dst = buf_ptr as *mut u8;
        for y in 0..height as usize {
            let src_row = src.add(y * pitch as usize);
            let dst_row = dst.add(y * width as usize * 4);
            core::ptr::copy_nonoverlapping(src_row, dst_row, width as usize * 4);
        }
    }

    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_capture_screen(buf_ptr: u64, buf_size: u32, info_ptr: u64) -> u32 {
    let Some(fb) = crate::drivers::framebuffer::info() else {
        return 1;
    };

    let bytes = (fb.pitch as usize) * (fb.height as usize);
    if (buf_size as usize) < bytes {
        return 2;
    }

    if info_ptr != 0 {
        unsafe {
            let info = info_ptr as *mut u32;
            *info = fb.width;
            *info.add(1) = fb.height;
        }
    }

    if buf_ptr != 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(fb.addr as *const u8, buf_ptr as *mut u8, bytes);
        }
    }

    0
}

// =========================================================================
// VRAM direct surface syscalls
// =========================================================================

/// SYS_GPU_VRAM_SIZE (256): Return total GPU VRAM size in bytes.
/// Compositor-only. Returns 0 if no GPU driver.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_vram_size() -> u32 {
    if !is_compositor() {
        return 0;
    }
    crate::drivers::gpu::with_gpu(|g| g.vram_size()).unwrap_or(0)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_vram_size() -> u32 {
    crate::drivers::framebuffer::info()
        .map(|fb| fb.pitch.saturating_mul(fb.height))
        .unwrap_or(0)
}

/// SYS_VRAM_MAP (257): Map VRAM pages into a target app's address space.
/// Compositor-only. Used for VRAM-direct surfaces (zero-copy GPU rendering).
///
/// arg1 = target_tid
/// arg2 = vram_byte_offset (must be page-aligned)
/// arg3 = num_bytes (rounded up to pages)
///
/// Maps VRAM at user VA 0x18000000 in the target process with Write-Through + PTE_VRAM.
/// Returns 0x18000000 on success, 0 on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_vram_map(target_tid: u32, vram_offset: u32, num_bytes: u32) -> u32 {
    if !is_compositor() {
        return 0;
    }
    if num_bytes == 0 || (vram_offset & 0xFFF) != 0 {
        return 0;
    }

    // Get framebuffer physical base from GPU
    let fb_phys = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some((_, _, _, phys)) => phys as u64,
        None => return 0,
    };

    // Get target thread's page directory
    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => {
            crate::serial_verbose_println!("VRAM_MAP: thread {} has no page directory", target_tid);
            return 0;
        }
    };

    let user_va_base: u64 = 0x1800_0000;
    let pages = ((num_bytes as usize + 4095) / 4096) as usize;

    // Map VRAM pages into the target's address space
    // Flags: Present + Writable + User + Write-Through + PTE_VRAM
    let flags: u64 = 0x0F | crate::memory::virtual_mem::PTE_VRAM; // 0x20F

    for i in 0..pages {
        let phys =
            crate::memory::address::PhysAddr::new(fb_phys + vram_offset as u64 + (i * 4096) as u64);
        let virt = crate::memory::address::VirtAddr::new(user_va_base + (i * 4096) as u64);
        crate::memory::virtual_mem::map_page_in_pd(pd_phys, virt, phys, flags);
    }

    crate::serial_verbose_println!(
        "VRAM_MAP: mapped {} pages at VA {:#x} for T{} (fb_phys={:#x}, offset={:#x})",
        pages,
        user_va_base,
        target_tid,
        fb_phys,
        vram_offset
    );

    user_va_base as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_vram_map(target_tid: u32, vram_offset: u32, num_bytes: u32) -> u32 {
    if !is_compositor() {
        return 0;
    }
    if num_bytes == 0 || (vram_offset & 0xFFF) != 0 {
        return 0;
    }

    let (fb_phys, _fb_virt, _width, height, pitch) =
        match crate::drivers::arm::gpu::framebuffer_mapping_info() {
            Some(info) => info,
            None => return 0,
        };
    let fb_total_bytes = (height as usize).saturating_mul(pitch as usize);
    let map_offset = vram_offset as usize;
    let map_bytes = ((num_bytes as usize) + 4095) & !4095usize;
    if map_offset >= fb_total_bytes || map_offset.saturating_add(map_bytes) > fb_total_bytes {
        return 0;
    }

    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => return 0,
    };

    let user_va_base: u64 = 0x1800_0000;
    let pages = map_bytes / 4096;
    for i in 0..pages {
        let phys =
            crate::memory::address::PhysAddr::new(fb_phys + map_offset as u64 + (i * 4096) as u64);
        let virt = crate::memory::address::VirtAddr::new(user_va_base + (i * 4096) as u64);
        if !crate::memory::virtual_mem::map_page_in_pd(pd_phys, virt, phys, 0x0F) {
            return 0;
        }
    }

    user_va_base as u32
}

/// SYS_GPU_REGISTER_BACKBUFFER (258): Register a userspace back buffer for GPU DMA.
///
/// The compositor calls this with a pointer to its `back_buffer` and size.
/// The kernel walks the page tables to collect physical pages, then registers
/// them as a GMR with the GPU driver. After this, `transfer_rect` uses GPU
/// DMA from the back buffer instead of requiring a CPU memcpy to VRAM.
///
/// arg1 = buffer virtual address (user-space pointer)
/// arg2 = buffer size in bytes
///
/// Returns 0 on success, u32::MAX on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_register_backbuffer(buf_ptr: u64, buf_size: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if buf_size == 0 || buf_ptr == 0 {
        return u32::MAX;
    }

    // Account for sub-page offset: data starts at buf_ptr & 0xFFF within the
    // first page, so we need extra page(s) to cover the tail of the buffer.
    let sub_page_offset = (buf_ptr as usize) & 0xFFF;
    let pages = (buf_size as usize + sub_page_offset + 4095) / 4096;
    let page_base = (buf_ptr as u64) & !0xFFF; // align down to page boundary
    let mut phys_pages: alloc::vec::Vec<u64> = alloc::vec::Vec::with_capacity(pages);

    // Walk page tables to collect physical addresses for each page
    for i in 0..pages {
        let va = page_base + (i as u64) * 4096;
        let pte = crate::memory::virtual_mem::read_pte(crate::memory::address::VirtAddr::new(va));
        if pte & 1 == 0 {
            // Page not present — cannot register
            crate::serial_verbose_println!(
                "GPU_REGISTER_BACKBUFFER: page {} not present (va={:#x})",
                i,
                va
            );
            return u32::MAX;
        }
        let phys = pte & 0x000F_FFFF_FFFF_F000;
        phys_pages.push(phys);
    }

    // Register with GPU driver (pass sub-page offset so GMR blit aligns correctly)
    let sub_page_offset = (buf_ptr as u32) & 0xFFF;
    let ok =
        crate::drivers::gpu::with_gpu(|g| g.register_back_buffer(&phys_pages, sub_page_offset));

    match ok {
        Some(true) => {
            crate::serial_verbose_println!(
                "GPU_REGISTER_BACKBUFFER: registered {} pages (buf={:#x}, size={})",
                pages,
                buf_ptr,
                buf_size
            );
            0
        }
        _ => {
            crate::serial_verbose_println!(
                "GPU_REGISTER_BACKBUFFER: GPU driver rejected registration"
            );
            u32::MAX
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_register_backbuffer(_buf_ptr: u64, _buf_size: u32) -> u32 {
    u32::MAX
}

// =========================================================================
// Fullscreen direct framebuffer access
// =========================================================================

/// SYS_GRANT_FRAMEBUFFER (259): Map the GPU framebuffer into a target app's address space.
/// Compositor-only. Used for fullscreen direct framebuffer access.
///
/// arg1 = target_tid
/// arg2 = out_info_ptr (in compositor's address space): writes [fb_va: u32, width: u32, height: u32, pitch: u32]
///
/// Maps the framebuffer at VA 0x19000000 in the target process (different from VRAM surfaces at 0x18000000).
/// Returns 0 on success, u32::MAX on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_grant_framebuffer(target_tid: u32, out_info_ptr: u64) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    // Get framebuffer info from GPU driver
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return u32::MAX,
    };
    // Per-page list — driver may have allocated the framebuffer as
    // scatter-gather rather than one contiguous block.
    let pages_vec: alloc::vec::Vec<u64> =
        match crate::drivers::gpu::with_gpu(|g| g.framebuffer_pages()) {
            Some(v) => v,
            None => return u32::MAX,
        };

    // Get target thread's page directory
    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => return u32::MAX,
    };

    let fb_user_base: u64 = 0x1900_0000;
    let fb_total_bytes = height as usize * pitch as usize;
    let pages = (fb_total_bytes + 4095) / 4096;
    if pages > pages_vec.len() {
        return u32::MAX;
    }

    // Map framebuffer pages into the target's address space
    // Flags: Present + Writable + User + Write-Through (0x0F)
    for i in 0..pages {
        let phys = crate::memory::address::PhysAddr::new(pages_vec[i]);
        let virt = crate::memory::address::VirtAddr::new(fb_user_base + (i * 4096) as u64);
        crate::memory::virtual_mem::map_page_in_pd(pd_phys, virt, phys, 0x0F);
    }

    // Write info struct to compositor's user memory
    if out_info_ptr != 0 {
        unsafe {
            let info = out_info_ptr as *mut u32;
            *info = fb_user_base as u32;
            *info.add(1) = width;
            *info.add(2) = height;
            *info.add(3) = pitch;
        }
    }

    crate::serial_verbose_println!(
        "GRANT_FRAMEBUFFER: mapped {} pages at VA {:#x} for T{} (fb_phys={:#x})",
        pages,
        fb_user_base,
        target_tid,
        fb_phys
    );
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_grant_framebuffer(target_tid: u32, out_info_ptr: u64) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    let (fb_phys, _fb_virt, width, height, pitch) =
        match crate::drivers::arm::gpu::framebuffer_mapping_info() {
            Some(info) => info,
            None => return u32::MAX,
        };

    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => return u32::MAX,
    };

    let fb_user_base: u64 = 0x1900_0000;
    let fb_total_bytes = (height as usize).saturating_mul(pitch as usize);
    let pages = (fb_total_bytes + 4095) / 4096;

    for i in 0..pages {
        let phys = crate::memory::address::PhysAddr::new(fb_phys + (i * 4096) as u64);
        let virt = crate::memory::address::VirtAddr::new(fb_user_base + (i * 4096) as u64);
        if !crate::memory::virtual_mem::map_page_in_pd(pd_phys, virt, phys, 0x0F) {
            return u32::MAX;
        }
    }

    if out_info_ptr != 0 {
        unsafe {
            let info = out_info_ptr as *mut u32;
            *info = fb_user_base as u32;
            *info.add(1) = width;
            *info.add(2) = height;
            *info.add(3) = pitch;
        }
    }

    0
}

/// SYS_REVOKE_FRAMEBUFFER (261): Unmap the framebuffer from a target app's address space.
/// Compositor-only. Removes the mapping created by sys_grant_framebuffer.
///
/// arg1 = target_tid
/// Returns 0 on success, u32::MAX on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_revoke_framebuffer(target_tid: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    // Get framebuffer info to know how many pages to unmap
    let (_, height, pitch, _) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return u32::MAX,
    };

    // Get target thread's page directory
    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => return u32::MAX,
    };

    let fb_user_base: u64 = 0x1900_0000;
    let fb_total_bytes = height as usize * pitch as usize;
    let pages = (fb_total_bytes + 4095) / 4096;

    // Unmap pages from the target's address space
    for i in 0..pages {
        let virt = crate::memory::address::VirtAddr::new(fb_user_base + (i * 4096) as u64);
        crate::memory::virtual_mem::unmap_page_in_pd(pd_phys, virt);
    }

    crate::serial_verbose_println!(
        "REVOKE_FRAMEBUFFER: unmapped {} pages at VA {:#x} for T{}",
        pages,
        fb_user_base,
        target_tid
    );
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_revoke_framebuffer(target_tid: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    let (_fb_phys, _fb_virt, _width, height, pitch) =
        match crate::drivers::arm::gpu::framebuffer_mapping_info() {
            Some(info) => info,
            None => return u32::MAX,
        };

    let pd_phys = match crate::task::scheduler::thread_page_directory(target_tid) {
        Some(pd) => pd,
        None => return u32::MAX,
    };

    let fb_user_base: u64 = 0x1900_0000;
    let fb_total_bytes = (height as usize).saturating_mul(pitch as usize);
    let pages = (fb_total_bytes + 4095) / 4096;

    for i in 0..pages {
        let virt = crate::memory::address::VirtAddr::new(fb_user_base + (i * 4096) as u64);
        crate::memory::virtual_mem::unmap_page_in_pd(pd_phys, virt);
    }

    0
}

// =========================================================================
// GPU 3D Acceleration (SVGA3D)
// =========================================================================

/// SYS_GPU_3D_QUERY (513): Query 3D GPU capabilities.
/// query_type: 0 = has_3d, 1 = hw_version
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_query(query_type: u32) -> u32 {
    crate::drivers::gpu::with_gpu(|g| match query_type {
        0 => g.has_3d() as u32,
        1 => g.hw_version_3d(),
        _ => 0,
    })
    .unwrap_or(0)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_query(_query_type: u32) -> u32 {
    0
}

/// SYS_GPU_3D_SUBMIT (512): Submit raw 3D command words to the GPU.
/// buf_ptr: pointer to u32 word array in user memory
/// word_count: number of u32 words
///
/// For SVGA3D: validates command IDs are in range 1040..1099.
/// For virgl: passes raw Gallium command words through without validation.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_submit(buf_ptr: u64, word_count: u32) -> u32 {
    if buf_ptr == 0 || word_count == 0 {
        return u32::MAX;
    }

    // Cap at 16384 words (64 KiB) per submission
    let count = word_count.min(16384) as usize;
    let byte_size = (count * 4) as u64;

    // Validate pointer is in user space and properly mapped
    if !is_valid_user_ptr(buf_ptr as u64, byte_size) {
        return u32::MAX;
    }

    let words = unsafe { core::slice::from_raw_parts(buf_ptr as *const u32, count) };

    // Driver-specific validation: SVGA3D needs command ID checks,
    // virgl passes raw Gallium commands without structure validation.
    let driver_type = crate::drivers::gpu::with_gpu(|g| {
        let mut s = alloc::string::String::new();
        s.push_str(g.driver_type_name());
        s
    });

    if let Some(ref dt) = driver_type {
        if dt == "svga3d" {
            use crate::drivers::gpu::vmware_svga::{SVGA_3D_CMD_MAX, SVGA_3D_CMD_MIN};
            // Validate SVGA3D command buffer structure
            let mut offset = 0;
            while offset < words.len() {
                if offset + 2 > words.len() {
                    return u32::MAX;
                }
                let cmd_id = words[offset];
                let size_bytes = words[offset + 1];

                if cmd_id < SVGA_3D_CMD_MIN || cmd_id > SVGA_3D_CMD_MAX {
                    return u32::MAX;
                }

                let payload_words = ((size_bytes + 3) / 4) as usize;
                if offset + 2 + payload_words > words.len() {
                    return u32::MAX;
                }

                offset += 2 + payload_words;
            }
        }
        // virgl: no structural validation needed — host renderer validates
    }

    // Submit to GPU
    crate::drivers::gpu::with_gpu(|g| {
        if g.submit_3d_commands(words) {
            0u32
        } else {
            u32::MAX
        }
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_submit(_buf_ptr: u64, _word_count: u32) -> u32 {
    u32::MAX
}

/// SYS_GPU_3D_SYNC (514): Wait for all pending 3D commands to complete.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_sync() -> u32 {
    crate::drivers::gpu::with_gpu(|g| {
        g.sync();
        0u32
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_sync() -> u32 {
    u32::MAX
}

/// SYS_GPU_3D_SURFACE_DMA (515): Upload data to a GPU surface via DMA.
/// arg1: surface ID
/// arg2: user pointer to data buffer
/// arg3: data length in bytes
/// arg4: surface width (pixels)
/// arg5: surface height (pixels)
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_surface_dma(
    sid: u32,
    buf_ptr: u64,
    buf_len: u32,
    width: u32,
    height: u32,
) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || width == 0 || height == 0 {
        return u32::MAX;
    }
    // Cap at 1 MiB per upload (matches DMA staging buffer size)
    let len = buf_len.min(1024 * 1024) as usize;
    if !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
    crate::drivers::gpu::with_gpu(|g| {
        if g.dma_surface_upload(sid, data, width, height) {
            0u32
        } else {
            u32::MAX
        }
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_surface_dma(
    _sid: u32,
    _buf_ptr: u64,
    _buf_len: u32,
    _width: u32,
    _height: u32,
) -> u32 {
    u32::MAX
}

#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_surface_dma_read(
    sid: u32,
    buf_ptr: u64,
    buf_len: u32,
    width: u32,
    height: u32,
) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || width == 0 || height == 0 {
        return u32::MAX;
    }
    let len = buf_len.min(65536 * 16) as usize; // Up to 1 MiB for readback
    if !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    crate::drivers::gpu::with_gpu(|g| {
        if g.dma_surface_download(sid, buf, width, height) {
            0u32
        } else {
            u32::MAX
        }
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_surface_dma_read(
    _sid: u32,
    _buf_ptr: u64,
    _buf_len: u32,
    _width: u32,
    _height: u32,
) -> u32 {
    u32::MAX
}

/// SYS_GPU_QUERY_TYPE (517): Query the GPU driver type name for userspace .drv loading.
/// Writes driver type name (e.g. "svga3d", "virgl", "none") as null-terminated string to buf.
/// Returns the string length (excluding null terminator).
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_query_type(buf_ptr: u64, buf_len: u32) -> u32 {
    let type_name = crate::drivers::gpu::with_gpu(|g| {
        let mut s = alloc::string::String::new();
        s.push_str(g.driver_type_name());
        s
    });

    let name = match type_name {
        Some(n) => n,
        None => alloc::string::String::from("none"),
    };

    if buf_ptr != 0 && buf_len > 0 {
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(buf_len as usize - 1);
        unsafe {
            let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len + 1);
            buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            buf[copy_len] = 0;
        }
    }
    name.len() as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_query_type(buf_ptr: u64, buf_len: u32) -> u32 {
    let name = "none";

    if buf_ptr != 0 && buf_len > 0 {
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(buf_len as usize - 1);
        unsafe {
            let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len + 1);
            buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            buf[copy_len] = 0;
        }
    }
    name.len() as u32
}

// =========================================================================
// GPU 3D Resource Management (virgl control plane)
// =========================================================================

/// SYS_GPU_3D_RESOURCE_CREATE (518): Create a virgl 3D resource via the control plane.
/// arg1=target, arg2=format, arg3=bind, arg4=width, arg5=height
/// Uses defaults: depth=1, array_size=1, last_level=0, nr_samples=0, flags=0.
/// Returns the allocated resource ID, or u32::MAX on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_resource_create(
    target: u32,
    format: u32,
    bind: u32,
    width: u32,
    height: u32,
) -> u32 {
    if width == 0 || height == 0 {
        return u32::MAX;
    }
    crate::drivers::gpu::with_gpu(|g| {
        g.create_3d_resource(target, format, bind, width, height, 1, 1, 0, 0, 0)
            .unwrap_or(u32::MAX)
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_resource_create(
    _target: u32,
    _format: u32,
    _bind: u32,
    _width: u32,
    _height: u32,
) -> u32 {
    u32::MAX
}

/// SYS_GPU_3D_RESOURCE_DESTROY (519): Destroy a virgl 3D resource.
/// arg1=resource_id. Returns 0 on success, u32::MAX on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_resource_destroy(resource_id: u32) -> u32 {
    if resource_id == 0 {
        return u32::MAX;
    }
    crate::drivers::gpu::with_gpu(|g| {
        if g.destroy_3d_resource(resource_id) {
            0u32
        } else {
            u32::MAX
        }
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_resource_destroy(_resource_id: u32) -> u32 {
    u32::MAX
}

// ───────────────────────────────────────────────────────────────────────
// Multi-monitor display syscalls (700–704). See
// docs/multimonitor-architecture.md for the design.
// ───────────────────────────────────────────────────────────────────────

/// Wire format for SYS_DISPLAY_LIST entries.
///
/// 64 bytes per output, fixed layout — keeps userspace marshalling
/// trivial and the syscall ABI versionable. Mirrors the kernel
/// `OutputInfo` struct minus the variable-length mode list (which is
/// queried separately in a future SYS_DISPLAY_MODES syscall to keep the
/// per-call buffer bounded).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct DisplayInfoFfi {
    pub id: u32,             //  0
    pub connected: u32,      //  4  (boolean as u32 for alignment)
    pub current_w: u32,      //  8
    pub current_h: u32,      // 12
    pub preferred_w: u32,    // 16
    pub preferred_h: u32,    // 20
    pub refresh_mhz: u32,    // 24
    pub bpp: u32,            // 28
    pub physical_mm: u32,    // 32  (low u16 = width mm, high u16 = height mm)
    pub edid_hash: u64,      // 36
    pub manufacturer: u32,   // 44  (4 ASCII bytes, null-terminated)
    pub flags: u32,          // 48  (bit 0 = primary, bit 1 = mirror)
    pub mirror_of: u32,      // 52  (0xFFFFFFFF = none)
    pub _reserved: [u32; 2], // 56..64
}

/// Wire format for SYS_DISPLAY_SET_LAYOUT entries (36 bytes each).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct LayoutEntryFfi {
    pub id: u32,               //  0
    pub virtual_x: i32,        //  4
    pub virtual_y: i32,        //  8
    pub mode_w: u32,           // 12
    pub mode_h: u32,           // 16
    pub mode_refresh_mhz: u32, // 20
    pub scale: u32,            // 24  (percent, 100 = 1.0x)
    pub flags: u32,            // 28  (bit 0 = primary)
    pub mirror_of: u32,        // 32  (0xFFFFFFFF = none)
}

/// SYS_DISPLAY_LIST (700): enumerate all advertised display outputs.
/// `buf_ptr` points to a `[DisplayInfoFfi; N]`; `buf_count` is N.
/// Returns the number of entries written (≤ N), or `u32::MAX` on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_display_list(buf_ptr: u64, buf_count: u32) -> u32 {
    if buf_ptr == 0 || buf_count == 0 {
        return u32::MAX;
    }
    let bytes = (buf_count as u64) * (core::mem::size_of::<DisplayInfoFfi>() as u64);
    if !super::helpers::is_valid_user_ptr(buf_ptr, bytes) {
        return u32::MAX;
    }
    let written = crate::drivers::gpu::with_gpu(|g| {
        let total = g.display_count().min(buf_count) as u32;
        let dst = unsafe {
            core::slice::from_raw_parts_mut(buf_ptr as *mut DisplayInfoFfi, total as usize)
        };
        for i in 0..total {
            let info = match g.output_info(i) {
                Some(v) => v,
                None => continue,
            };
            let mut e = DisplayInfoFfi::default();
            e.id = info.id;
            e.connected = if info.connected { 1 } else { 0 };
            if let Some(m) = info.current_mode {
                e.current_w = m.width;
                e.current_h = m.height;
                e.refresh_mhz = m.refresh_mhz;
                e.bpp = m.bpp as u32;
            }
            if let Some(m) = info.preferred_mode {
                e.preferred_w = m.width;
                e.preferred_h = m.height;
                if e.refresh_mhz == 0 {
                    e.refresh_mhz = m.refresh_mhz;
                }
            }
            e.physical_mm = (info.physical_mm.0 as u32) | ((info.physical_mm.1 as u32) << 16);
            e.edid_hash = info.edid_hash;
            e.manufacturer = u32::from_le_bytes(info.manufacturer);
            if let Some(target) = info.mirror_of {
                e.flags |= 2;
                e.mirror_of = target;
            } else {
                e.mirror_of = u32::MAX;
            }
            dst[i as usize] = e;
        }
        total
    });
    written.unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_list(_buf_ptr: u64, _buf_count: u32) -> u32 {
    0
}

/// SYS_DISPLAY_SET_LAYOUT (701): atomically apply a complete layout.
/// `entries_ptr` points to a `[LayoutEntryFfi; entry_count]`. Returns 0
/// on success, or a non-zero `LayoutError::code()` on validation failure
/// (current layout is preserved). `u32::MAX` indicates a hard error
/// (invalid arguments, no GPU).
#[cfg(target_arch = "x86_64")]
pub fn sys_display_set_layout(entries_ptr: u64, entry_count: u32) -> u32 {
    // Allowed for the registered compositor *or* the registered
    // display-layout owner (typically `displayd`). Two callers because
    // both need to write — compositor for boot-time bring-up,
    // displayd for hot-plug + user-driven layout changes.
    if !is_compositor() && !is_display_owner() {
        return u32::MAX;
    }
    if entries_ptr == 0 || entry_count == 0 || entry_count > 32 {
        return u32::MAX;
    }
    let bytes = (entry_count as u64) * (core::mem::size_of::<LayoutEntryFfi>() as u64);
    if !super::helpers::is_valid_user_ptr(entries_ptr, bytes) {
        return u32::MAX;
    }
    let raw = unsafe {
        core::slice::from_raw_parts(entries_ptr as *const LayoutEntryFfi, entry_count as usize)
    };
    use crate::drivers::gpu::output::{OutputLayout, OutputLayoutEntry, OutputMode};
    let mut layout = OutputLayout::empty();
    for e in raw {
        let refresh = if e.mode_refresh_mhz == 0 {
            60_000
        } else {
            e.mode_refresh_mhz
        };
        let scale = if e.scale == 0 {
            100
        } else {
            e.scale.min(400) as u16
        };
        // Rotation lives in flags bits 4..6 (00=0°, 01=90°, 10=180°,
        // 11=270°). When rotation swaps the logical orientation we
        // expose virtual_w/virtual_h with the post-rotation dimensions
        // so the compositor lays out windows in portrait coordinates,
        // even though the underlying mode (panel native scanout) is
        // always landscape.
        let rotation = ((e.flags >> 4) & 0b11) as u8;
        let scaled_w = ((e.mode_w as u32) * 100u32) / (scale as u32);
        let scaled_h = ((e.mode_h as u32) * 100u32) / (scale as u32);
        let (vw, vh) = if rotation == 1 || rotation == 3 {
            (scaled_h, scaled_w)
        } else {
            (scaled_w, scaled_h)
        };
        layout.entries.push(OutputLayoutEntry {
            id: e.id,
            virtual_x: e.virtual_x,
            virtual_y: e.virtual_y,
            virtual_w: vw,
            virtual_h: vh,
            mode: OutputMode {
                width: e.mode_w,
                height: e.mode_h,
                refresh_mhz: refresh,
                bpp: 32,
            },
            scale,
            mirror_of: if e.mirror_of == u32::MAX {
                None
            } else {
                Some(e.mirror_of)
            },
            primary: (e.flags & 1) != 0,
            rotation,
        });
    }
    crate::drivers::gpu::with_gpu(|g| match g.apply_layout(&layout) {
        Ok(()) => 0u32,
        Err(err) => err.code(),
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_set_layout(_entries_ptr: u64, _entry_count: u32) -> u32 {
    u32::MAX
}

/// SYS_DISPLAY_GET_ROTATION (706): read the current rotation (0..=3,
/// 90° steps) for `output_id`. Returns `u32::MAX` for unknown ids or
/// when no GPU is registered. The compositor uses this to decide
/// whether to apply software rotation when blitting.
#[cfg(target_arch = "x86_64")]
pub fn sys_display_get_rotation(output_id: u32) -> u32 {
    crate::drivers::gpu::output_rotation(output_id)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_get_rotation(_output_id: u32) -> u32 {
    0
}

/// SYS_DISPLAY_MAP_FB (702): map output `output_id`'s framebuffer into
/// the compositor's address space. `out_info_ptr` points to a
/// `[u32; 4]` filled with `[user_base, width, height, pitch]`.
/// Returns 0 on success, `u32::MAX` on failure.
///
/// The mapping is placed at `0x2000_0000 + output_id * 0x0400_0000`
/// (64 MiB stride) so the compositor can hold mappings for up to 16
/// outputs simultaneously without manual address management.
#[cfg(target_arch = "x86_64")]
pub fn sys_display_map_fb(output_id: u32, out_info_ptr: u64) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if output_id >= crate::drivers::gpu::output::MAX_OUTPUTS as u32 {
        return u32::MAX;
    }
    // Secondary outputs are activated at GPU driver init (boot time, kernel CR3
    // with full identity map). If there is no mode here, the driver either
    // hasn't been initialized yet or this scanout was disabled in the cold-boot
    // layout — either way map_fb must fail rather than try to allocate phys
    // pages now under user CR3 (which only identity-maps the first 64 MiB).
    let mode = crate::drivers::gpu::with_gpu(|g| g.mode_for_output(output_id)).flatten();
    let (width, height, pitch, fb_phys) = match mode {
        Some(m) => m,
        None => {
            crate::serial_println!(
                "[!] sys_display_map_fb: output {} has no active mode (set at boot only)",
                output_id
            );
            return u32::MAX;
        }
    };
    let fb_user_base: u64 = 0x2000_0000u64 + (output_id as u64) * 0x0400_0000u64;
    let fb_total_bytes = (height as usize).saturating_mul(pitch as usize);
    if fb_total_bytes == 0 {
        return u32::MAX;
    }
    let pages = (fb_total_bytes + crate::memory::FRAME_SIZE - 1) / crate::memory::FRAME_SIZE;
    for i in 0..pages {
        let phys_addr = crate::memory::address::PhysAddr::new(
            fb_phys as u64 + (i * crate::memory::FRAME_SIZE) as u64,
        );
        let virt_addr = crate::memory::address::VirtAddr::new(
            fb_user_base + (i * crate::memory::FRAME_SIZE) as u64,
        );
        crate::memory::virtual_mem::map_page(virt_addr, phys_addr, 0x0F);
    }
    if out_info_ptr != 0 {
        if !super::helpers::is_valid_user_ptr(out_info_ptr, 16) {
            return u32::MAX;
        }
        let info = unsafe { &mut *(out_info_ptr as *mut [u32; 4]) };
        info[0] = fb_user_base as u32;
        info[1] = width;
        info[2] = height;
        info[3] = pitch;
    }
    crate::serial_verbose_println!(
        "[OK] Output {} fb mapped at {:#010x} ({}x{}, pitch={}, phys={:#x})",
        output_id,
        fb_user_base,
        width,
        height,
        pitch,
        fb_phys
    );
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_map_fb(_output_id: u32, _out_info_ptr: u64) -> u32 {
    u32::MAX
}

/// SYS_DISPLAY_FLUSH (703): transfer + flush a rect on `output_id`.
/// `xy` packs `(x << 16) | y`, `wh` packs `(w << 16) | h`.
/// Returns 0 on success, `u32::MAX` on failure.
#[cfg(target_arch = "x86_64")]
pub fn sys_display_flush(output_id: u32, xy: u32, wh: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    if output_id >= crate::drivers::gpu::output::MAX_OUTPUTS as u32 {
        return u32::MAX;
    }
    let x = (xy >> 16) & 0xFFFF;
    let y = xy & 0xFFFF;
    let w = (wh >> 16) & 0xFFFF;
    let h = wh & 0xFFFF;
    // First-call diagnostic only — log once per output so the boot test
    // can confirm the secondary render pass reaches the kernel without
    // spamming every frame.
    static FLUSH_SEEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let mask = 1u32 << (output_id & 31);
    let prev = FLUSH_SEEN.fetch_or(mask, core::sync::atomic::Ordering::Relaxed);
    if prev & mask == 0 {
        crate::serial_println!(
            "[OK] sys_display_flush: first call for output {} ({}x{} at {},{})",
            output_id,
            w,
            h,
            x,
            y
        );
    }
    crate::drivers::gpu::with_gpu(|g| g.update_rect_for_output(output_id, x, y, w, h));
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_flush(_output_id: u32, _xy: u32, _wh: u32) -> u32 {
    0
}

/// SYS_REGISTER_DISPLAY_OWNER (705): mark the calling process as the
/// authoritative display-layout owner (typically `displayd`). After
/// this call the process can issue SYS_DISPLAY_SET_LAYOUT even though
/// it isn't the registered compositor. First caller wins; subsequent
/// callers receive `u32::MAX`.
///
/// No additional capability check beyond first-caller-wins — the same
/// design SYS_REGISTER_COMPOSITOR uses. In practice the system-services
/// layer (sessionhost / compositor bootstrap) is responsible for
/// spawning a single trusted displayd before any other process gets to
/// register.
#[cfg(target_arch = "x86_64")]
pub fn sys_register_display_owner() -> u32 {
    if let Some(pd) = crate::task::scheduler::current_thread_page_directory() {
        if DISPLAY_OWNER_PD
            .compare_exchange(0, pd.as_u64(), Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            crate::serial_verbose_println!(
                "[OK] Display layout owner registered (PD={:#x})",
                pd.as_u64()
            );
            return 0;
        }
    }
    u32::MAX
}

#[cfg(target_arch = "aarch64")]
pub fn sys_register_display_owner() -> u32 {
    u32::MAX
}

/// SYS_DISPLAY_POLL_EVENT (704): drain one DisplayEvent.
/// Returns:
///  - `0` if no event pending
///  - `1 | (output_id << 8)` for HotplugChanged (output_id ignored, =0)
///  - `2 | (output_id << 8)` for PreferredModeChanged
///  - `3` for LayoutApplied
///  - `u32::MAX` on error
#[cfg(target_arch = "x86_64")]
pub fn sys_display_poll_event() -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }
    use crate::drivers::gpu::output::DisplayEvent;
    crate::drivers::gpu::with_gpu(|g| match g.poll_display_event() {
        None => 0u32,
        Some(DisplayEvent::HotplugChanged) => 1,
        Some(DisplayEvent::PreferredModeChanged { output }) => 2 | (output << 8),
        Some(DisplayEvent::LayoutApplied) => 3,
    })
    .unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_display_poll_event() -> u32 {
    0
}
