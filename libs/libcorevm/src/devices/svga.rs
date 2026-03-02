//! Simple VGA/SVGA framebuffer emulation.
//!
//! Emulates a VGA-compatible display adapter with support for text mode
//! (80x25), standard VGA graphics modes, and a linear framebuffer mode
//! for SVGA resolutions.
//!
//! # I/O Ports
//!
//! | Port Range | Description |
//! |------------|-------------|
//! | 0x3C0-0x3C1 | Attribute controller (index/data flip-flop) |
//! | 0x3C2 | Miscellaneous output register (write) / Input status 0 (read) |
//! | 0x3C4-0x3C5 | Sequencer (index/data) |
//! | 0x3C6 | DAC pixel mask |
//! | 0x3C7 | DAC read address |
//! | 0x3C8 | DAC write address |
//! | 0x3C9 | DAC data (R/G/B components, 6-bit) |
//! | 0x3CE-0x3CF | Graphics controller (index/data) |
//! | 0x3D4-0x3D5 | CRTC (index/data) |
//! | 0x3DA | Input Status Register 1 (read) / Attribute reset (read) |
//!
//! # MMIO
//!
//! The legacy VGA framebuffer is mapped at physical address 0xA0000
//! (128 KB window). In linear framebuffer mode, a larger MMIO region
//! is used.

use alloc::vec;
use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;
use crate::memory::mmio::MmioHandler;

/// VGA display mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VgaMode {
    /// 80x25 text mode with character + attribute pairs.
    Text80x25,
    /// 320x200 with 256-color palette (Mode 13h).
    Graphics320x200x256,
    /// 640x480 with 16-color palette.
    Graphics640x480x16,
    /// Linear framebuffer at an arbitrary resolution and bit depth.
    LinearFramebuffer {
        /// Horizontal resolution in pixels.
        width: u32,
        /// Vertical resolution in pixels.
        height: u32,
        /// Bits per pixel (8, 16, 24, or 32).
        bpp: u8,
    },
}

/// VGA/SVGA display adapter emulation.
#[derive(Debug)]
pub struct Svga {
    /// Current display mode.
    pub mode: VgaMode,
    /// Pixel data for graphics modes. Size depends on the current mode.
    pub framebuffer: Vec<u8>,
    /// Text mode buffer: 80 x 25 cells, each a `u16` (low byte = char,
    /// high byte = attribute).
    pub text_buffer: Vec<u16>,
    /// 256-entry DAC color palette. Each entry is `[R, G, B]` with 6-bit
    /// values (0-63).
    pub dac_palette: [[u8; 3]; 256],
    /// DAC write index: the palette entry that will be written next.
    pub dac_write_index: u8,
    /// DAC read index: the palette entry that will be read next.
    pub dac_read_index: u8,
    /// Component counter within the current DAC palette entry (0=R, 1=G, 2=B).
    pub dac_component: u8,
    /// Currently selected CRTC register index.
    pub crtc_index: u8,
    /// CRTC register file (25 registers).
    pub crtc_regs: [u8; 25],
    /// Currently selected sequencer register index.
    pub seq_index: u8,
    /// Sequencer register file (5 registers).
    pub seq_regs: [u8; 5],
    /// Currently selected graphics controller register index.
    pub gc_index: u8,
    /// Graphics controller register file (9 registers).
    pub gc_regs: [u8; 9],
    /// Currently selected attribute controller register index.
    pub attr_index: u8,
    /// Attribute controller register file (21 registers).
    pub attr_regs: [u8; 21],
    /// Attribute controller address/data flip-flop.
    /// `false` = next write to 0x3C0 is an index, `true` = data.
    pub attr_flip_flop: bool,
    /// Miscellaneous output register.
    pub misc_output: u8,
    /// Number of MMIO writes received (debug counter).
    pub mmio_write_count: u64,
    /// Number of MMIO writes to the text buffer region (offset >= 0x18000).
    pub mmio_text_write_count: u64,
    /// Current horizontal resolution in pixels.
    pub width: u32,
    /// Current vertical resolution in pixels.
    pub height: u32,
    /// Current bits per pixel.
    pub bpp: u8,
    /// Bochs VBE index register (port 0x1CE).
    pub vbe_index: u16,
    /// Bochs VBE data registers (20 entries, indexed by `vbe_index`).
    pub vbe_regs: [u16; 20],
}

impl Svga {
    /// Create a new VGA adapter starting in 80x25 text mode.
    ///
    /// The framebuffer is allocated large enough for the specified
    /// resolution at 32 bpp (used when switching to linear framebuffer
    /// mode).
    pub fn new(width: u32, height: u32) -> Self {
        let fb_size = (width * height * 4) as usize;
        let mut dac_palette = [[0u8; 3]; 256];

        // Initialize the first 16 palette entries with standard VGA colors.
        let standard_colors: [[u8; 3]; 16] = [
            [0, 0, 0],       // 0: black
            [0, 0, 42],      // 1: blue
            [0, 42, 0],      // 2: green
            [0, 42, 42],     // 3: cyan
            [42, 0, 0],      // 4: red
            [42, 0, 42],     // 5: magenta
            [42, 21, 0],     // 6: brown
            [42, 42, 42],    // 7: light gray
            [21, 21, 21],    // 8: dark gray
            [21, 21, 63],    // 9: light blue
            [21, 63, 21],    // 10: light green
            [21, 63, 63],    // 11: light cyan
            [63, 21, 21],    // 12: light red
            [63, 21, 63],    // 13: light magenta
            [63, 63, 21],    // 14: yellow
            [63, 63, 63],    // 15: white
        ];
        for (i, color) in standard_colors.iter().enumerate() {
            dac_palette[i] = *color;
        }

        Svga {
            mode: VgaMode::Text80x25,
            framebuffer: vec![0u8; fb_size],
            text_buffer: vec![0x0720u16; 80 * 25], // space with light gray on black
            dac_palette,
            dac_write_index: 0,
            dac_read_index: 0,
            dac_component: 0,
            crtc_index: 0,
            crtc_regs: [0; 25],
            seq_index: 0,
            seq_regs: [0; 5],
            gc_index: 0,
            gc_regs: [0; 9],
            attr_index: 0,
            attr_regs: [0; 21],
            attr_flip_flop: false,
            misc_output: 0,
            mmio_write_count: 0,
            mmio_text_write_count: 0,
            width,
            height,
            bpp: 32,
            vbe_index: 0,
            vbe_regs: {
                let mut r = [0u16; 20];
                // VBE_DISPI_INDEX_ID: report Bochs VBE version 0xB0C5.
                r[0] = 0xB0C5;
                // VBE_DISPI_INDEX_XRES
                r[1] = width as u16;
                // VBE_DISPI_INDEX_YRES
                r[2] = height as u16;
                // VBE_DISPI_INDEX_BPP
                r[3] = 32;
                // VBE_DISPI_INDEX_ENABLE: disabled initially.
                r[4] = 0;
                // VBE_DISPI_INDEX_VIRT_WIDTH
                r[6] = width as u16;
                // VBE_DISPI_INDEX_VIRT_HEIGHT
                r[7] = height as u16;
                // VBE_DISPI_INDEX_VIDEO_MEMORY_64K: report 8 MiB VRAM (128 * 64KB).
                r[10] = 128;
                r
            },
        }
    }

    /// Get a reference to the raw framebuffer pixel data.
    ///
    /// The format depends on the current mode and bpp setting.
    pub fn get_framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Get a reference to the text mode buffer.
    ///
    /// Each entry is a `u16`: low byte = ASCII character, high byte =
    /// color attribute. The buffer is organized as 80 columns x 25 rows
    /// in row-major order.
    pub fn get_text_buffer(&self) -> &[u16] {
        &self.text_buffer
    }

    /// Switch to a new display mode.
    ///
    /// Reallocates the framebuffer if the new mode requires a different
    /// size. The text buffer is always preserved.
    pub fn set_mode(&mut self, mode: VgaMode) {
        let (new_width, new_height, new_bpp) = match &mode {
            VgaMode::Text80x25 => (720, 400, 8u8), // typical text mode pixel dimensions
            VgaMode::Graphics320x200x256 => (320, 200, 8),
            VgaMode::Graphics640x480x16 => (640, 480, 4),
            VgaMode::LinearFramebuffer { width, height, bpp } => (*width, *height, *bpp),
        };

        let fb_size = (new_width as usize) * (new_height as usize) * ((new_bpp as usize + 7) / 8);
        if fb_size > self.framebuffer.len() {
            self.framebuffer.resize(fb_size, 0);
        }
        // Clear the framebuffer on mode switch.
        for byte in self.framebuffer.iter_mut() {
            *byte = 0;
        }

        self.width = new_width;
        self.height = new_height;
        self.bpp = new_bpp;
        self.mode = mode;
    }
}

impl IoHandler for Svga {
    /// Read from VGA I/O ports (0x3C0-0x3DA) and Bochs VBE ports (0x1CE-0x1CF).
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            // Bochs VBE index register.
            0x1CE => return Ok(self.vbe_index as u32),
            // Bochs VBE data register.
            0x1CF => {
                let idx = self.vbe_index as usize;
                return Ok(if idx < self.vbe_regs.len() {
                    self.vbe_regs[idx] as u32
                } else {
                    0
                });
            }
            0x3C0 => {
                // Attribute controller: return current index.
                self.attr_index
            }
            0x3C1 => {
                // Attribute controller data read.
                let idx = (self.attr_index & 0x1F) as usize;
                if idx < self.attr_regs.len() {
                    self.attr_regs[idx]
                } else {
                    0
                }
            }
            0x3C2 => {
                // Input Status Register 0 / Misc Output read.
                self.misc_output
            }
            0x3C4 => self.seq_index,
            0x3C5 => {
                let idx = (self.seq_index & 0x07) as usize;
                if idx < self.seq_regs.len() {
                    self.seq_regs[idx]
                } else {
                    0
                }
            }
            0x3C6 => {
                // DAC pixel mask — always 0xFF (all planes enabled).
                0xFF
            }
            0x3C7 => {
                // DAC state: 0x03 = read mode.
                0x03
            }
            0x3C8 => self.dac_write_index,
            0x3C9 => {
                // DAC data read: cycle through R, G, B for current read index.
                let idx = self.dac_read_index as usize;
                let component = self.dac_component as usize;
                let val = self.dac_palette[idx][component];
                self.dac_component += 1;
                if self.dac_component >= 3 {
                    self.dac_component = 0;
                    self.dac_read_index = self.dac_read_index.wrapping_add(1);
                }
                val
            }
            0x3CE => self.gc_index,
            0x3CF => {
                let idx = (self.gc_index & 0x0F) as usize;
                if idx < self.gc_regs.len() {
                    self.gc_regs[idx]
                } else {
                    0
                }
            }
            0x3D4 => self.crtc_index,
            0x3D5 => {
                let idx = (self.crtc_index & 0x3F) as usize;
                if idx < self.crtc_regs.len() {
                    self.crtc_regs[idx]
                } else {
                    0
                }
            }
            0x3DA => {
                // Input Status Register 1.
                // Reading this register resets the attribute controller flip-flop.
                self.attr_flip_flop = false;
                // Return vertical retrace bit (bit 3) toggling — many guests
                // poll this to synchronize with VBLANK.
                // Always report "not in retrace" for simplicity.
                0x00
            }
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to VGA I/O ports (0x3C0-0x3DA) and Bochs VBE ports (0x1CE-0x1CF).
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            // Bochs VBE index register.
            0x1CE => {
                self.vbe_index = val as u16;
                return Ok(());
            }
            // Bochs VBE data register.
            0x1CF => {
                let idx = self.vbe_index as usize;
                let v = val as u16;
                if idx < self.vbe_regs.len() {
                    self.vbe_regs[idx] = v;
                }
                // VBE_DISPI_INDEX_ENABLE (4): mode switch.
                if idx == 4 && (v & 0x01) != 0 {
                    let w = self.vbe_regs[1] as u32;
                    let h = self.vbe_regs[2] as u32;
                    let bpp = self.vbe_regs[3] as u8;
                    if w > 0 && h > 0 && bpp > 0 {
                        self.set_mode(VgaMode::LinearFramebuffer {
                            width: w,
                            height: h,
                            bpp,
                        });
                    }
                } else if idx == 4 && (v & 0x01) == 0 {
                    // VBE disabled — return to text mode.
                    self.set_mode(VgaMode::Text80x25);
                }
                return Ok(());
            }
            0x3C0 => {
                // Attribute controller: alternates between index and data writes.
                if !self.attr_flip_flop {
                    self.attr_index = byte & 0x3F;
                } else {
                    let idx = (self.attr_index & 0x1F) as usize;
                    if idx < self.attr_regs.len() {
                        self.attr_regs[idx] = byte;
                    }
                }
                self.attr_flip_flop = !self.attr_flip_flop;
            }
            0x3C2 => {
                // Miscellaneous output register.
                self.misc_output = byte;
            }
            0x3C4 => self.seq_index = byte,
            0x3C5 => {
                let idx = (self.seq_index & 0x07) as usize;
                if idx < self.seq_regs.len() {
                    self.seq_regs[idx] = byte;
                }
            }
            0x3C6 => { /* DAC pixel mask — ignore writes */ }
            0x3C7 => {
                // DAC read address.
                self.dac_read_index = byte;
                self.dac_component = 0;
            }
            0x3C8 => {
                // DAC write address.
                self.dac_write_index = byte;
                self.dac_component = 0;
            }
            0x3C9 => {
                // DAC data write: cycle through R, G, B for current write index.
                let idx = self.dac_write_index as usize;
                let component = self.dac_component as usize;
                self.dac_palette[idx][component] = byte & 0x3F; // 6-bit values
                self.dac_component += 1;
                if self.dac_component >= 3 {
                    self.dac_component = 0;
                    self.dac_write_index = self.dac_write_index.wrapping_add(1);
                }
            }
            0x3CE => self.gc_index = byte,
            0x3CF => {
                let idx = (self.gc_index & 0x0F) as usize;
                if idx < self.gc_regs.len() {
                    self.gc_regs[idx] = byte;
                }
            }
            0x3D4 => self.crtc_index = byte,
            0x3D5 => {
                let idx = (self.crtc_index & 0x3F) as usize;
                if idx < self.crtc_regs.len() {
                    self.crtc_regs[idx] = byte;
                }
            }
            0x3DA => { /* Input Status Register 1 is read-only */ }
            _ => {}
        }
        Ok(())
    }
}

impl MmioHandler for Svga {
    /// Read from the VGA framebuffer MMIO region (base 0xA0000, 128 KB).
    ///
    /// In text mode, reads from the text buffer (0xB8000 offset mapped to
    /// 0x18000 within the MMIO window). In graphics modes, reads directly
    /// from the framebuffer.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        match self.mode {
            VgaMode::Text80x25 => {
                // Text buffer at offset 0x18000 (0xB8000 - 0xA0000).
                // Handle multi-byte reads (e.g. 16-bit word = char + attr).
                let base = offset.wrapping_sub(0x18000) as usize;
                let buf_bytes = self.text_buffer.len() * 2;
                let mut result: u64 = 0;
                for i in 0..(size as usize) {
                    let text_offset = base + i;
                    if text_offset >= buf_bytes {
                        break;
                    }
                    let cell_idx = text_offset / 2;
                    let cell_val = self.text_buffer[cell_idx];
                    let byte = if text_offset & 1 == 0 {
                        (cell_val & 0xFF) as u8
                    } else {
                        (cell_val >> 8) as u8
                    };
                    result |= (byte as u64) << (i * 8);
                }
                Ok(result)
            }
            _ => {
                // Graphics mode: read from framebuffer.
                let off = offset as usize;
                if off >= self.framebuffer.len() {
                    return Ok(0);
                }
                let val = match size {
                    1 => self.framebuffer[off] as u64,
                    2 => {
                        let end = (off + 2).min(self.framebuffer.len());
                        let mut v = 0u64;
                        for i in off..end {
                            v |= (self.framebuffer[i] as u64) << ((i - off) * 8);
                        }
                        v
                    }
                    4 => {
                        let end = (off + 4).min(self.framebuffer.len());
                        let mut v = 0u64;
                        for i in off..end {
                            v |= (self.framebuffer[i] as u64) << ((i - off) * 8);
                        }
                        v
                    }
                    _ => {
                        let end = (off + size as usize).min(self.framebuffer.len());
                        let mut v = 0u64;
                        for i in off..end {
                            v |= (self.framebuffer[i] as u64) << ((i - off) * 8);
                        }
                        v
                    }
                };
                Ok(val)
            }
        }
    }

    /// Write to the VGA framebuffer MMIO region (base 0xA0000, 128 KB).
    ///
    /// In text mode, writes go to the text buffer. In graphics modes,
    /// writes go directly to the framebuffer.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        self.mmio_write_count += 1;
        if offset >= 0x18000 {
            self.mmio_text_write_count += 1;
        }
        match self.mode {
            VgaMode::Text80x25 => {
                // Text buffer at offset 0x18000 (0xB8000 - 0xA0000).
                // Handle multi-byte writes (e.g. 16-bit word = char + attr).
                let base = offset.wrapping_sub(0x18000) as usize;
                let buf_bytes = self.text_buffer.len() * 2;
                for i in 0..(size as usize) {
                    let text_offset = base + i;
                    if text_offset >= buf_bytes {
                        break;
                    }
                    let byte = ((val >> (i * 8)) & 0xFF) as u16;
                    let cell_idx = text_offset / 2;
                    if text_offset & 1 == 0 {
                        // Low byte (character).
                        self.text_buffer[cell_idx] =
                            (self.text_buffer[cell_idx] & 0xFF00) | byte;
                    } else {
                        // High byte (attribute).
                        self.text_buffer[cell_idx] =
                            (self.text_buffer[cell_idx] & 0x00FF) | (byte << 8);
                    }
                }
            }
            _ => {
                // Graphics mode: write to framebuffer.
                let off = offset as usize;
                let count = size as usize;
                for i in 0..count {
                    let idx = off + i;
                    if idx < self.framebuffer.len() {
                        self.framebuffer[idx] = ((val >> (i * 8)) & 0xFF) as u8;
                    }
                }
            }
        }
        Ok(())
    }
}
