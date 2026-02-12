//! Red Screen of Death (RSOD) — catastrophic crash display.
//!
//! Renders a full-screen red error display with white text showing
//! comprehensive crash information, similar to Windows' Blue Screen
//! of Death but in red. Works without heap allocation, writing directly
//! to the VESA framebuffer via volatile writes.

use crate::arch::x86::idt::InterruptFrame;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

/// Re-entrancy guard: prevents infinite RSOD recursion when the RSOD
/// itself triggers a fault (e.g. framebuffer not mapped in current CR3).
static RSOD_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Switch to kernel CR3 so the framebuffer identity mapping is available.
/// The framebuffer at physical ~0xFD000000 is identity-mapped in the kernel
/// PML4 but NOT in user process page tables.
fn ensure_kernel_cr3() {
    let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
    let current_cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) current_cr3); }
    if current_cr3 != kernel_cr3 {
        unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
    }
}

/// Embedded 8x16 bitmap font (ASCII 32-126, same data as boot_console)
static FONT_DATA: &[u8] = include_bytes!("../graphics/font_8x16.bin");
const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

// Colors
const BG_COLOR: u32 = 0xFFCC0000;     // Bold red background
const TEXT_COLOR: u32 = 0xFFFFFFFF;    // White text
const DIM_COLOR: u32 = 0xFFFFAAAA;     // Dimmed pinkish-white for secondary info
const HEADER_BG: u32 = 0xFF990000;     // Darker red for header strip

// Layout
const MARGIN_X: u32 = 40;
const MARGIN_Y: u32 = 30;
const HEADER_HEIGHT: u32 = 60;

/// RSOD writer state — tracks cursor position for formatted output.
/// No heap allocation, writes directly to framebuffer.
struct RsodWriter {
    fb_addr: u64,
    fb_pitch: u32,
    fb_width: u32,
    fb_height: u32,
    cursor_x: u32,
    cursor_y: u32,
    color: u32,
}

impl RsodWriter {
    fn new() -> Option<Self> {
        // Use boot framebuffer info (identity-mapped at physical address).
        // This is safe during panic — no locks required.
        // Try GPU try_lock for current resolution (may have changed at runtime),
        // fall back to boot info if GPU lock is held.
        let fb = crate::drivers::framebuffer::info()?;
        let fb_addr = fb.addr as u64;

        // Try to get current mode from GPU (non-blocking)
        let (fb_pitch, fb_width, fb_height) =
            if let Some(mut guard) = crate::drivers::gpu::try_lock_gpu() {
                if let Some(g) = guard.as_mut() {
                    let (w, h, p, _) = g.get_mode();
                    (p, w, h)
                } else {
                    (fb.pitch, fb.width, fb.height)
                }
            } else {
                (fb.pitch, fb.width, fb.height)
            };

        if fb_width == 0 || fb_height == 0 {
            return None;
        }

        Some(RsodWriter {
            fb_addr,
            fb_pitch,
            fb_width,
            fb_height,
            cursor_x: MARGIN_X,
            cursor_y: MARGIN_Y + HEADER_HEIGHT + 20,
            color: TEXT_COLOR,
        })
    }

    fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.fb_width || y >= self.fb_height {
            return;
        }
        let offset = y as u64 * self.fb_pitch as u64 + x as u64 * 4;
        let ptr = (self.fb_addr + offset) as *mut u32;
        unsafe { ptr.write_volatile(color); }
    }

    fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        for dy in 0..h {
            let py = y + dy;
            if py >= self.fb_height { break; }
            let row_base = self.fb_addr + py as u64 * self.fb_pitch as u64;
            for dx in 0..w {
                let px = x + dx;
                if px >= self.fb_width { break; }
                let ptr = (row_base + px as u64 * 4) as *mut u32;
                unsafe { ptr.write_volatile(color); }
            }
        }
    }

    fn clear(&self) {
        self.fill_rect(0, 0, self.fb_width, self.fb_height, BG_COLOR);
    }

    fn draw_header_strip(&self) {
        self.fill_rect(0, 0, self.fb_width, MARGIN_Y + HEADER_HEIGHT, HEADER_BG);
    }

    /// Draw a character at 1x scale
    fn draw_char_1x(&self, x: u32, y: u32, ch: u8, color: u32) {
        let c = ch as u32;
        if c < 32 || c > 126 { return; }
        let idx = (c - 32) as usize;
        let glyph_off = idx * FONT_H as usize;
        if glyph_off + FONT_H as usize > FONT_DATA.len() { return; }

        for row in 0..FONT_H {
            let bits = FONT_DATA[glyph_off + row as usize];
            for col in 0..FONT_W {
                if bits & (0x80 >> col) != 0 {
                    self.put_pixel(x + col, y + row, color);
                }
            }
        }
    }

    /// Draw a character at 2x scale (for headers)
    fn draw_char_2x(&self, x: u32, y: u32, ch: u8, color: u32) {
        let c = ch as u32;
        if c < 32 || c > 126 { return; }
        let idx = (c - 32) as usize;
        let glyph_off = idx * FONT_H as usize;
        if glyph_off + FONT_H as usize > FONT_DATA.len() { return; }

        for row in 0..FONT_H {
            let bits = FONT_DATA[glyph_off + row as usize];
            for col in 0..FONT_W {
                if bits & (0x80 >> col) != 0 {
                    let px = x + col * 2;
                    let py = y + row * 2;
                    self.put_pixel(px, py, color);
                    self.put_pixel(px + 1, py, color);
                    self.put_pixel(px, py + 1, color);
                    self.put_pixel(px + 1, py + 1, color);
                }
            }
        }
    }

    /// Draw a string at 2x scale
    fn draw_string_2x(&self, x: u32, y: u32, text: &str, color: u32) {
        let mut cx = x;
        for byte in text.bytes() {
            if cx + FONT_W * 2 > self.fb_width { break; }
            self.draw_char_2x(cx, y, byte, color);
            cx += FONT_W * 2;
        }
    }

    fn set_color(&mut self, color: u32) {
        self.color = color;
    }

    fn newline(&mut self) {
        self.cursor_x = MARGIN_X;
        self.cursor_y += FONT_H;
        // If we run off the bottom, stop (don't scroll — this is a crash screen)
    }

    fn blank_line(&mut self) {
        self.newline();
    }
}

impl fmt::Write for RsodWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            match byte {
                b'\n' => {
                    self.newline();
                }
                _ => {
                    if self.cursor_y + FONT_H <= self.fb_height {
                        if self.cursor_x + FONT_W <= self.fb_width - MARGIN_X {
                            self.draw_char_1x(self.cursor_x, self.cursor_y, byte, self.color);
                        }
                        self.cursor_x += FONT_W;
                        if self.cursor_x + FONT_W > self.fb_width - MARGIN_X {
                            self.newline();
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Display an RSOD for a kernel panic (called from panic handler).
pub fn show_panic(message: &fmt::Arguments) {
    // Prevent recursive RSOD (if rendering itself faults)
    if RSOD_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }

    // Switch to kernel CR3 — framebuffer is only mapped in kernel page tables
    ensure_kernel_cr3();

    let mut w = match RsodWriter::new() {
        Some(w) => w,
        None => return, // No framebuffer available
    };

    // Hide hardware cursor so it doesn't overlay the crash screen
    // Hide HW cursor (non-blocking to avoid deadlock during panic)
    if let Some(mut g) = crate::drivers::gpu::try_lock_gpu() {
        if let Some(gpu) = g.as_mut() { gpu.show_cursor(false); }
    }

    w.clear();
    w.draw_header_strip();

    // Title
    w.draw_string_2x(MARGIN_X, MARGIN_Y + 10, ":( KERNEL PANIC", TEXT_COLOR);

    // Panic message
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "Your system encountered a fatal error and has been halted.\n");
    w.blank_line();

    w.set_color(TEXT_COLOR);
    let _ = write!(w, "Error: {}\n", message);
    w.blank_line();

    // System info
    w.set_color(DIM_COLOR);
    write_system_info(&mut w);
    w.blank_line();

    // Thread info
    w.set_color(TEXT_COLOR);
    write_thread_info(&mut w);
    w.blank_line();

    // Footer
    w.set_color(DIM_COLOR);
    let _ = write!(w, "The system has been halted to prevent data corruption.\n");
    let _ = write!(w, "Please restart your computer.\n");

    // Flush the framebuffer to the display (required for VMware SVGA which
    // needs an explicit UPDATE command to refresh the screen).
    flush_display(&w);
}

/// Display an RSOD for a fatal CPU exception (called from ISR handler).
pub fn show_exception(frame: &InterruptFrame, exception_name: &str) {
    // Prevent recursive RSOD (if rendering itself faults)
    if RSOD_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }

    // Switch to kernel CR3 — framebuffer is only mapped in kernel page tables
    ensure_kernel_cr3();

    let mut w = match RsodWriter::new() {
        Some(w) => w,
        None => return,
    };

    // Hide HW cursor (non-blocking to avoid deadlock during panic)
    if let Some(mut g) = crate::drivers::gpu::try_lock_gpu() {
        if let Some(gpu) = g.as_mut() { gpu.show_cursor(false); }
    }

    w.clear();
    w.draw_header_strip();

    // Title
    w.draw_string_2x(MARGIN_X, MARGIN_Y + 10, ":( FATAL EXCEPTION", TEXT_COLOR);

    // Exception type
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "A fatal {} occurred and the system has been halted.\n", exception_name);
    w.blank_line();

    // Registers
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "--- Registers ---\n");
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "RIP = {:#018x}    CS  = {:#06x}    SS  = {:#06x}\n", frame.rip, frame.cs, frame.ss);
    let _ = write!(w, "RSP = {:#018x}    RBP = {:#018x}    RFLAGS = {:#018x}\n", frame.rsp, frame.rbp, frame.rflags);
    let _ = write!(w, "RAX = {:#018x}    RBX = {:#018x}    RCX = {:#018x}\n", frame.rax, frame.rbx, frame.rcx);
    let _ = write!(w, "RDX = {:#018x}    RSI = {:#018x}    RDI = {:#018x}\n", frame.rdx, frame.rsi, frame.rdi);
    let _ = write!(w, "R8  = {:#018x}    R9  = {:#018x}    R10 = {:#018x}\n", frame.r8, frame.r9, frame.r10);
    let _ = write!(w, "R11 = {:#018x}    R12 = {:#018x}    R13 = {:#018x}\n", frame.r11, frame.r12, frame.r13);
    let _ = write!(w, "R14 = {:#018x}    R15 = {:#018x}\n", frame.r14, frame.r15);

    // Error code + INT number
    let _ = write!(w, "INT = {}    ERR = {:#018x}\n", frame.int_no, frame.err_code);

    // CR2 (for page faults), CR3
    let cr2: u64;
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    let _ = write!(w, "CR2 = {:#018x}    CR3 = {:#018x}\n", cr2, cr3);
    w.blank_line();

    // Stack dump
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "--- Stack Dump (from RSP) ---\n");
    w.set_color(DIM_COLOR);
    let stack_ptr = frame.rsp as *const u64;
    // Only dump if the stack pointer looks valid
    if frame.rsp >= 0xFFFF_FFFF_8000_0000 && frame.rsp < 0xFFFF_FFFF_F000_0000 && frame.rsp & 7 == 0 {
        for i in 0..8u64 {
            let val = unsafe { stack_ptr.add(i as usize).read_volatile() };
            let _ = write!(w, "  [RSP+{:#04x}] = {:#018x}\n", i * 8, val);
        }
    } else {
        let _ = write!(w, "  (RSP outside kernel range, cannot dump)\n");
    }
    w.blank_line();

    // Call trace (RBP chain)
    w.set_color(TEXT_COLOR);
    let _ = write!(w, "--- Call Trace ---\n");
    w.set_color(DIM_COLOR);
    let mut bp = frame.rbp;
    let mut depth = 0;
    while bp >= 0xFFFF_FFFF_8000_0000 && bp < 0xFFFF_FFFF_F000_0000 && bp & 7 == 0 && depth < 10 {
        let ret_addr = unsafe { *((bp + 8) as *const u64) };
        let prev_bp = unsafe { *(bp as *const u64) };
        let _ = write!(w, "  #{}: {:#018x}\n", depth, ret_addr);
        if prev_bp <= bp { break; } // prevent infinite loops
        bp = prev_bp;
        depth += 1;
    }
    if depth == 0 {
        let _ = write!(w, "  (no valid frame chain)\n");
    }
    w.blank_line();

    // System + thread info
    w.set_color(DIM_COLOR);
    write_system_info(&mut w);
    write_thread_info(&mut w);
    w.blank_line();

    w.set_color(DIM_COLOR);
    let _ = write!(w, "The system has been halted. Please restart your computer.\n");

    // Flush the framebuffer to the display (required for VMware SVGA).
    flush_display(&w);
}

/// Write system info (uptime, CPU count, memory)
fn write_system_info(w: &mut RsodWriter) {
    let ticks = crate::arch::x86::pit::TICK_COUNT.load(Ordering::Relaxed);
    let tick_hz = crate::arch::x86::pit::TICK_HZ;
    let uptime_secs = ticks / tick_hz;
    let uptime_mins = uptime_secs / 60;
    let uptime_hours = uptime_mins / 60;
    let cpu_count = crate::arch::x86::smp::cpu_count();
    let cpu_id = crate::arch::x86::smp::current_cpu_id();
    let free_frames = crate::memory::physical::free_frames();
    let total_frames = crate::memory::physical::total_frames();
    let free_mb = (free_frames * 4096) / (1024 * 1024);
    let total_mb = (total_frames * 4096) / (1024 * 1024);

    let _ = write!(w, "Uptime: {}h {}m {}s    CPU: {}/{} (current/total)    ",
        uptime_hours, uptime_mins % 60, uptime_secs % 60,
        cpu_id, cpu_count);
    let _ = write!(w, "Memory: {} MiB free / {} MiB total\n", free_mb, total_mb);
}

/// Write current thread info
fn write_thread_info(w: &mut RsodWriter) {
    let tid = crate::task::scheduler::debug_current_tid();
    let name_buf = crate::task::scheduler::current_thread_name();
    let name_len = name_buf.iter().position(|&b| b == 0).unwrap_or(32);
    let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("???");
    let is_user = crate::task::scheduler::is_current_thread_user();
    let mode = if is_user { "user" } else { "kernel" };

    let _ = write!(w, "Thread: TID={} name=\"{}\" mode={}\n", tid, name, mode);
}

/// Flush the RSOD framebuffer writes to the display.
///
/// VMware SVGA requires an explicit UPDATE command via the FIFO to refresh
/// screen contents. Without this, direct framebuffer writes are invisible.
/// For Bochs VGA this is a no-op (framebuffer writes are auto-detected).
fn flush_display(w: &RsodWriter) {
    // Try to get GPU lock (non-blocking to avoid deadlock)
    if let Some(mut guard) = crate::drivers::gpu::try_lock_gpu() {
        if let Some(gpu) = guard.as_mut() {
            gpu.update_rect(0, 0, w.fb_width, w.fb_height);
        }
    }
}
