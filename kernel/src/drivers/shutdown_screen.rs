//! Shutdown/Reboot screen — displays a dark screen with the anyOS logo text
//! and a status message while the system is shutting down or rebooting.
//!
//! Writes directly to the VESA framebuffer (like RSOD), bypassing the
//! compositor which is already terminated at this point.

/// Embedded 8x16 bitmap font (same as RSOD / boot_console)
static FONT_DATA: &[u8] = include_bytes!("../graphics/font_8x16.bin");
const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

const BG_COLOR: u32 = 0xFF1A1A1A; // Dark background
const TEXT_COLOR: u32 = 0xFFCCCCCC; // Light gray text
const LOGO_COLOR: u32 = 0xFFFFFFFF; // White for "anyOS"

/// Ensure we're using the kernel page tables so the framebuffer is accessible.
fn ensure_kernel_cr3() {
    #[cfg(target_arch = "x86_64")]
    {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        let current_cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
        }
        if current_cr3 != kernel_cr3 {
            unsafe {
                core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3);
            }
        }
    }
}

struct ScreenWriter {
    fb_addr: u64,
    fb_pitch: u32,
    fb_width: u32,
    fb_height: u32,
}

impl ScreenWriter {
    fn new() -> Option<Self> {
        let fb = crate::drivers::framebuffer::info()?;
        let fb_addr = fb.addr as u64;

        #[cfg(target_arch = "x86_64")]
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
        #[cfg(not(target_arch = "x86_64"))]
        let (fb_pitch, fb_width, fb_height) = (fb.pitch, fb.width, fb.height);

        if fb_width == 0 || fb_height == 0 {
            return None;
        }

        Some(ScreenWriter {
            fb_addr,
            fb_pitch,
            fb_width,
            fb_height,
        })
    }

    fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        for dy in 0..h {
            let py = y + dy;
            if py >= self.fb_height {
                break;
            }
            let row_base = self.fb_addr + py as u64 * self.fb_pitch as u64;
            for dx in 0..w {
                let px = x + dx;
                if px >= self.fb_width {
                    break;
                }
                let ptr = (row_base + px as u64 * 4) as *mut u32;
                unsafe {
                    ptr.write_volatile(color);
                }
            }
        }
    }

    fn clear(&self) {
        self.fill_rect(0, 0, self.fb_width, self.fb_height, BG_COLOR);
    }

    fn draw_char_2x(&self, x: u32, y: u32, ch: u8, color: u32) {
        let c = ch as u32;
        if c < 32 || c > 126 {
            return;
        }
        let idx = (c - 32) as usize;
        let glyph_off = idx * FONT_H as usize;
        if glyph_off + FONT_H as usize > FONT_DATA.len() {
            return;
        }
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

    fn draw_char_1x(&self, x: u32, y: u32, ch: u8, color: u32) {
        let c = ch as u32;
        if c < 32 || c > 126 {
            return;
        }
        let idx = (c - 32) as usize;
        let glyph_off = idx * FONT_H as usize;
        if glyph_off + FONT_H as usize > FONT_DATA.len() {
            return;
        }
        for row in 0..FONT_H {
            let bits = FONT_DATA[glyph_off + row as usize];
            for col in 0..FONT_W {
                if bits & (0x80 >> col) != 0 {
                    self.put_pixel(x + col, y + row, color);
                }
            }
        }
    }

    fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.fb_width || y >= self.fb_height {
            return;
        }
        let offset = y as u64 * self.fb_pitch as u64 + x as u64 * 4;
        let ptr = (self.fb_addr + offset) as *mut u32;
        unsafe {
            ptr.write_volatile(color);
        }
    }

    fn draw_string_2x(&self, x: u32, y: u32, text: &str, color: u32) {
        let mut cx = x;
        for byte in text.bytes() {
            if cx + FONT_W * 2 > self.fb_width {
                break;
            }
            self.draw_char_2x(cx, y, byte, color);
            cx += FONT_W * 2;
        }
    }

    fn draw_string_1x(&self, x: u32, y: u32, text: &str, color: u32) {
        let mut cx = x;
        for byte in text.bytes() {
            if cx + FONT_W > self.fb_width {
                break;
            }
            self.draw_char_1x(cx, y, byte, color);
            cx += FONT_W;
        }
    }

    /// Center a string horizontally at the given y coordinate (2x scale).
    fn draw_centered_2x(&self, y: u32, text: &str, color: u32) {
        let text_w = text.len() as u32 * FONT_W * 2;
        let x = if text_w < self.fb_width {
            (self.fb_width - text_w) / 2
        } else {
            0
        };
        self.draw_string_2x(x, y, text, color);
    }

    /// Center a string horizontally at the given y coordinate (1x scale).
    fn draw_centered_1x(&self, y: u32, text: &str, color: u32) {
        let text_w = text.len() as u32 * FONT_W;
        let x = if text_w < self.fb_width {
            (self.fb_width - text_w) / 2
        } else {
            0
        };
        self.draw_string_1x(x, y, text, color);
    }
}

/// Flush the framebuffer to the display.
///
/// VMware SVGA and VirtIO GPU require an explicit update command to make
/// direct framebuffer writes visible on screen. For Bochs VGA this is a no-op.
fn flush_display(w: &ScreenWriter) {
    #[cfg(target_arch = "x86_64")]
    if let Some(mut guard) = crate::drivers::gpu::try_lock_gpu() {
        if let Some(gpu) = guard.as_mut() {
            gpu.update_rect(0, 0, w.fb_width, w.fb_height);
        }
    }
}

/// Show the shutdown/reboot screen. Call this during the shutdown sequence.
/// Writes directly to the VESA framebuffer and flushes the GPU.
///
/// `is_reboot`: if true, shows "Restarting...", otherwise "Shutting down...".
pub fn show(is_reboot: bool) {
    ensure_kernel_cr3();

    let scr = match ScreenWriter::new() {
        Some(s) => s,
        None => return, // No framebuffer available (headless / nogui)
    };

    scr.clear();

    // Center the logo and message vertically
    let center_y = scr.fb_height / 2;

    // "anyOS" in 2x scale — centered
    scr.draw_centered_2x(center_y - 40, "anyOS", LOGO_COLOR);

    // Status message in 1x scale — centered below logo
    let msg = if is_reboot {
        "Restarting..."
    } else {
        "Shutting down..."
    };
    scr.draw_centered_1x(center_y + 10, msg, TEXT_COLOR);

    // Flush to GPU so the screen is actually visible (required for VMware SVGA, VirtIO)
    flush_display(&scr);
}
