//! Display, GPU, audio, and compositor syscall handlers.
//!
//! Covers screen info, resolution management, GPU acceleration (2D/3D),
//! VRAM mapping, audio output, compositor registration, framebuffer
//! mapping, input polling, screen capture, and boot readiness.

#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;
#[allow(unused_imports)]
use super::{COMPOSITOR_TID, COMPOSITOR_PD, is_compositor};

use core::sync::atomic::Ordering;

// =========================================================================
// Screen info / Resolution
// =========================================================================

#[cfg(target_arch = "x86_64")]
pub fn sys_screen_size(buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }
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
pub fn sys_screen_size(_buf_ptr: u32) -> u32 {
    u32::MAX
}

/// sys_set_resolution - Change display resolution via GPU driver.
///
/// IMPORTANT: set_mode() allocates new framebuffer pages via alloc_contiguous()
/// and zeroes them using identity-mapped access (fb_phys as *mut u8). During a
/// syscall, the CPU uses the user process's CR3 which only identity-maps 64 MiB
/// (PD[0..31]). If the physical allocator returns pages above 64 MiB, the zero
/// write would page-fault. We switch to the kernel CR3 (128 MiB identity-mapped)
/// for the duration of set_mode() to prevent this.
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
            crate::ipc::event_bus::system_emit(
                crate::ipc::event_bus::EventData::new(
                    crate::ipc::event_bus::EVT_RESOLUTION_CHANGED,
                    width, height, 0, 0,
                ),
            );
            0
        }
        _ => u32::MAX,
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_set_resolution(_width: u32, _height: u32) -> u32 {
    u32::MAX
}

/// sys_list_resolutions - List supported display resolutions.
/// Writes (width, height) pairs as u32 pairs to buf. Returns number of modes.
#[cfg(target_arch = "x86_64")]
pub fn sys_list_resolutions(buf_ptr: u32, buf_len: u32) -> u32 {
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
pub fn sys_list_resolutions(_buf_ptr: u32, _buf_len: u32) -> u32 {
    0
}

/// sys_gpu_info - Get GPU driver info. Writes driver name to buf. Returns name length.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_info(buf_ptr: u32, buf_len: u32) -> u32 {
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
pub fn sys_gpu_info(_buf_ptr: u32, _buf_len: u32) -> u32 {
    0
}

// =========================================================================
// GPU acceleration queries
// =========================================================================

/// SYS_GPU_HAS_ACCEL: Query if GPU acceleration is available.
pub fn sys_gpu_has_accel() -> u32 {
    use core::sync::atomic::Ordering;
    if crate::GPU_ACCEL.load(Ordering::Relaxed) { 1 } else { 0 }
}

/// SYS_GPU_HAS_HW_CURSOR: Query if GPU hardware cursor is available.
pub fn sys_gpu_has_hw_cursor() -> u32 {
    use core::sync::atomic::Ordering;
    if crate::GPU_HW_CURSOR.load(Ordering::Relaxed) { 1 } else { 0 }
}

// =========================================================================
// Audio
// =========================================================================

/// SYS_AUDIO_WRITE: Write PCM data to audio output.
/// arg1 = pointer to PCM data buffer, arg2 = length in bytes.
/// Returns number of bytes written.
#[cfg(target_arch = "x86_64")]
pub fn sys_audio_write(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return 0;
    }
    let data = unsafe {
        core::slice::from_raw_parts(buf_ptr as *const u8, buf_len as usize)
    };
    crate::drivers::audio::write_pcm(data) as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_audio_write(_buf_ptr: u32, _buf_len: u32) -> u32 {
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
        0 => { crate::drivers::audio::stop(); 0 }
        1 => { crate::drivers::audio::set_volume(arg as u8); 0 }
        2 => crate::drivers::audio::get_volume() as u32,
        3 => if crate::drivers::audio::is_playing() { 1 } else { 0 },
        4 => if crate::drivers::audio::is_available() { 1 } else { 0 },
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
    if COMPOSITOR_TID.compare_exchange(0, tid, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        // Store the compositor's page directory so child threads (render thread)
        // are also recognized as compositor by is_compositor().
        if let Some(pd) = crate::task::scheduler::current_thread_page_directory() {
            COMPOSITOR_PD.store(pd.as_u64(), Ordering::SeqCst);
        }

        // Boost compositor to realtime priority so UI never stutters
        crate::task::scheduler::set_thread_priority(tid, 127);
        crate::serial_println!("[OK] Compositor registered (TID={}, priority=127)", tid);
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
    crate::serial_println!("Compositor cursor takeover: splash pos ({}, {})", x, y);
    ((x as u16 as u32) << 16) | (y as u16 as u32)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_cursor_takeover() -> u32 {
    0
}

/// Map the GPU framebuffer into the compositor's address space.
/// ebx=out_info_ptr (pointer to FbMapInfo struct, 16 bytes).
/// Returns 0 on success, u32::MAX on failure.
///
/// FbMapInfo layout: { fb_vaddr: u32, width: u32, height: u32, pitch: u32 }
#[cfg(target_arch = "x86_64")]
pub fn sys_map_framebuffer(out_info_ptr: u32) -> u32 {
    if !is_compositor() {
        return u32::MAX;
    }

    // Get framebuffer info from GPU driver
    let (width, height, pitch, fb_phys) = match crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
        Some(m) => m,
        None => return u32::MAX,
    };

    // Map 16 MiB of VRAM into the compositor's address space at 0x20000000
    // (covers all resolutions up to 1920x1080 double-buffered)
    let fb_user_base: u64 = 0x2000_0000;
    let fb_map_size: usize = 16 * 1024 * 1024;
    let pages = fb_map_size / crate::memory::FRAME_SIZE;

    for i in 0..pages {
        let phys_addr = crate::memory::address::PhysAddr::new(
            fb_phys as u64 + (i * crate::memory::FRAME_SIZE) as u64,
        );
        let virt_addr = crate::memory::address::VirtAddr::new(
            fb_user_base + (i * crate::memory::FRAME_SIZE) as u64,
        );
        // Present + Writable + User + Write-Through (0x0F)
        crate::memory::virtual_mem::map_page(virt_addr, phys_addr, 0x0F);
    }

    // Write FbMapInfo struct to user memory
    if out_info_ptr != 0 {
        let info = unsafe { &mut *(out_info_ptr as *mut [u32; 4]) };
        info[0] = fb_user_base as u32;
        info[1] = width;
        info[2] = height;
        info[3] = pitch;
    }

    crate::serial_println!(
        "[OK] Framebuffer mapped to compositor at {:#010x} ({}x{}, pitch={}, phys={:#x})",
        fb_user_base, width, height, pitch, fb_phys
    );
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_map_framebuffer(_out_info_ptr: u32) -> u32 {
    u32::MAX
}

/// Submit GPU acceleration commands from the compositor.
/// ebx=cmd_buf_ptr, ecx=cmd_count.
/// Returns number of commands executed, or u32::MAX on error.
///
/// Each command is 36 bytes: { cmd_type: u32, args: [u32; 8] }
/// Command types: 1=UPDATE, 2=FILL_RECT, 3=COPY_RECT, 4=CURSOR_MOVE,
///                5=CURSOR_SHOW, 6=DEFINE_CURSOR, 7=FLIP
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_command(cmd_buf_ptr: u32, cmd_count: u32) -> u32 {
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
    let cmds = unsafe {
        core::slice::from_raw_parts(cmd_buf_ptr as *const [u32; 9], count)
    };

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
            let ok = match cmd_type {
                1 => { // UPDATE(x, y, w, h) — accumulate bbox, defer transfer+flush
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
                2 => { // FILL_RECT(x, y, w, h, color)
                    g.accel_fill_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5])
                }
                3 => { // COPY_RECT(sx, sy, dx, dy, w, h)
                    g.accel_copy_rect(cmd[1], cmd[2], cmd[3], cmd[4], cmd[5], cmd[6])
                }
                4 => { // CURSOR_MOVE(x, y)
                    if !crate::drivers::gpu::is_splash_cursor_active() {
                        g.move_cursor(cmd[1], cmd[2]);
                    }
                    true
                }
                5 => { // CURSOR_SHOW(visible)
                    g.show_cursor(cmd[1] != 0);
                    true
                }
                6 => { // DEFINE_CURSOR(w, h, hotx, hoty, pixels_ptr_lo, pixels_ptr_hi, pixel_count)
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
                        crate::serial_println!("GPU DEFINE_CURSOR: invalid pixel ptr {:#x} count={}", ptr, count);
                        false
                    } else {
                        let pixels = unsafe {
                            core::slice::from_raw_parts(ptr as *const u32, count)
                        };
                        g.define_cursor(w, h, hotx, hoty, pixels);
                        true
                    }
                }
                7 => { // FLIP
                    g.flip();
                    true
                }
                8 => { // SYNC
                    g.sync();
                    true
                }
                9 => { // VRAM_INFO
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

    result.unwrap_or(0)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_command(_cmd_buf_ptr: u32, _cmd_count: u32) -> u32 {
    u32::MAX
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
pub fn sys_input_poll(buf_ptr: u32, max_events: u32) -> u32 {
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
    let events = unsafe {
        core::slice::from_raw_parts_mut(buf_ptr as *mut [u32; 5], max)
    };
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

    // Drain mouse events
    while count < max {
        match crate::drivers::input::mouse::read_event() {
            Some(mouse_evt) => {
                use crate::drivers::input::mouse::MouseEventType;
                let (event_type, arg0, arg1, arg2, arg3) = match mouse_evt.event_type {
                    MouseEventType::Move => {
                        (3u32, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, 0)
                    }
                    MouseEventType::MoveAbsolute => {
                        // event_type 6 = absolute position (pixel coords)
                        (6u32, mouse_evt.dx as u32, mouse_evt.dy as u32, 0, 0)
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
                    MouseEventType::Scroll => {
                        (5, mouse_evt.dz as u32, 0, 0, 0)
                    }
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
pub fn sys_input_poll(_buf_ptr: u32, _max_events: u32) -> u32 {
    0
}

// =========================================================================
// Boot readiness / Screen capture
// =========================================================================

/// SYS_BOOT_READY: Signal from the compositor that the desktop is fully up.
pub fn sys_boot_ready() -> u32 {
    crate::serial_println!("[OK] Boot ready signal received from compositor");
    0
}

/// SYS_CAPTURE_SCREEN: Capture the current framebuffer contents to a user buffer.
/// arg1 = buf_ptr (pointer to u32 ARGB pixels)
/// arg2 = buf_size (buffer size in bytes)
/// arg3 = info_ptr (pointer to write [width: u32, height: u32])
/// Returns: 0 on success, 1 = no GPU, 2 = buffer too small.
#[cfg(target_arch = "x86_64")]
pub fn sys_capture_screen(buf_ptr: u32, buf_size: u32, info_ptr: u32) -> u32 {
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
            let phys = crate::memory::address::PhysAddr::new(
                fb_phys as u64 + (i * 0x1000) as u64,
            );
            let virt = crate::memory::address::VirtAddr::new(
                fb_map_base + (i * 0x1000) as u64,
            );
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
pub fn sys_capture_screen(_buf_ptr: u32, _buf_size: u32, _info_ptr: u32) -> u32 {
    1 // no GPU
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
    0
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
            crate::serial_println!("VRAM_MAP: thread {} has no page directory", target_tid);
            return 0;
        }
    };

    let user_va_base: u64 = 0x1800_0000;
    let pages = ((num_bytes as usize + 4095) / 4096) as usize;

    // Map VRAM pages into the target's address space
    // Flags: Present + Writable + User + Write-Through + PTE_VRAM
    let flags: u64 = 0x0F | crate::memory::virtual_mem::PTE_VRAM; // 0x20F

    for i in 0..pages {
        let phys = crate::memory::address::PhysAddr::new(
            fb_phys + vram_offset as u64 + (i * 4096) as u64,
        );
        let virt = crate::memory::address::VirtAddr::new(
            user_va_base + (i * 4096) as u64,
        );
        crate::memory::virtual_mem::map_page_in_pd(pd_phys, virt, phys, flags);
    }

    crate::serial_println!(
        "VRAM_MAP: mapped {} pages at VA {:#x} for T{} (fb_phys={:#x}, offset={:#x})",
        pages, user_va_base, target_tid, fb_phys, vram_offset
    );

    user_va_base as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_vram_map(_target_tid: u32, _vram_offset: u32, _num_bytes: u32) -> u32 {
    0
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
pub fn sys_gpu_register_backbuffer(buf_ptr: u32, buf_size: u32) -> u32 {
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
        let pte = crate::memory::virtual_mem::read_pte(
            crate::memory::address::VirtAddr::new(va),
        );
        if pte & 1 == 0 {
            // Page not present — cannot register
            crate::serial_println!(
                "GPU_REGISTER_BACKBUFFER: page {} not present (va={:#x})",
                i, va
            );
            return u32::MAX;
        }
        let phys = pte & 0x000F_FFFF_FFFF_F000;
        phys_pages.push(phys);
    }

    // Register with GPU driver (pass sub-page offset so GMR blit aligns correctly)
    let sub_page_offset = (buf_ptr as u32) & 0xFFF;
    let ok = crate::drivers::gpu::with_gpu(|g| {
        g.register_back_buffer(&phys_pages, sub_page_offset)
    });

    match ok {
        Some(true) => {
            crate::serial_println!(
                "GPU_REGISTER_BACKBUFFER: registered {} pages (buf={:#x}, size={})",
                pages, buf_ptr, buf_size
            );
            0
        }
        _ => {
            crate::serial_println!("GPU_REGISTER_BACKBUFFER: GPU driver rejected registration");
            u32::MAX
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_register_backbuffer(_buf_ptr: u32, _buf_size: u32) -> u32 {
    u32::MAX
}

// =========================================================================
// GPU 3D Acceleration (SVGA3D)
// =========================================================================

/// SYS_GPU_3D_QUERY (513): Query 3D GPU capabilities.
/// query_type: 0 = has_3d, 1 = hw_version
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_query(query_type: u32) -> u32 {
    crate::drivers::gpu::with_gpu(|g| {
        match query_type {
            0 => g.has_3d() as u32,
            1 => g.hw_version_3d(),
            _ => 0,
        }
    }).unwrap_or(0)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_query(_query_type: u32) -> u32 {
    0
}

/// SYS_GPU_3D_SUBMIT (512): Submit raw SVGA3D command words to the GPU FIFO.
/// buf_ptr: pointer to u32 word array in user memory
/// word_count: number of u32 words
///
/// Validates that all command IDs are in the SVGA3D range (1040..1099)
/// and that command sizes don't exceed the buffer.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_submit(buf_ptr: u32, word_count: u32) -> u32 {
    use crate::drivers::gpu::vmware_svga::{SVGA_3D_CMD_MIN, SVGA_3D_CMD_MAX};

    if buf_ptr == 0 || word_count == 0 {
        return u32::MAX;
    }

    // Cap at 4096 words (16 KiB) per submission
    let count = word_count.min(4096) as usize;
    let byte_size = (count * 4) as u64;

    // Validate pointer is in user space and properly mapped
    if !is_valid_user_ptr(buf_ptr as u64, byte_size) {
        return u32::MAX;
    }

    let words = unsafe {
        core::slice::from_raw_parts(buf_ptr as *const u32, count)
    };

    // Validate command buffer structure:
    // Each SVGA3D command is [cmd_id, size_bytes, payload...]
    // where size_bytes is the byte count of the payload only.
    let mut offset = 0;
    while offset < words.len() {
        if offset + 2 > words.len() {
            return u32::MAX; // Truncated header
        }
        let cmd_id = words[offset];
        let size_bytes = words[offset + 1];

        // Validate command ID is in SVGA3D range
        if cmd_id < SVGA_3D_CMD_MIN || cmd_id > SVGA_3D_CMD_MAX {
            return u32::MAX;
        }

        // Validate payload size doesn't exceed remaining buffer
        let payload_words = ((size_bytes + 3) / 4) as usize;
        if offset + 2 + payload_words > words.len() {
            return u32::MAX;
        }

        offset += 2 + payload_words;
    }

    // Submit validated buffer to GPU
    crate::drivers::gpu::with_gpu(|g| {
        if g.submit_3d_commands(words) { 0u32 } else { u32::MAX }
    }).unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_submit(_buf_ptr: u32, _word_count: u32) -> u32 {
    u32::MAX
}

/// SYS_GPU_3D_SYNC (514): Wait for all pending 3D commands to complete.
#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_sync() -> u32 {
    crate::drivers::gpu::with_gpu(|g| {
        g.sync();
        0u32
    }).unwrap_or(u32::MAX)
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
pub fn sys_gpu_3d_surface_dma(sid: u32, buf_ptr: u32, buf_len: u32, width: u32, height: u32) -> u32 {
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
        if g.dma_surface_upload(sid, data, width, height) { 0u32 } else { u32::MAX }
    }).unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_surface_dma(_sid: u32, _buf_ptr: u32, _buf_len: u32, _width: u32, _height: u32) -> u32 {
    u32::MAX
}

#[cfg(target_arch = "x86_64")]
pub fn sys_gpu_3d_surface_dma_read(sid: u32, buf_ptr: u32, buf_len: u32, width: u32, height: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || width == 0 || height == 0 {
        return u32::MAX;
    }
    let len = buf_len.min(65536 * 16) as usize; // Up to 1 MiB for readback
    if !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    crate::drivers::gpu::with_gpu(|g| {
        if g.dma_surface_download(sid, buf, width, height) { 0u32 } else { u32::MAX }
    }).unwrap_or(u32::MAX)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_gpu_3d_surface_dma_read(_sid: u32, _buf_ptr: u32, _buf_len: u32, _width: u32, _height: u32) -> u32 {
    u32::MAX
}
