//! RGBA icon loading and rendering. Icons are stored as raw binary files
//! with a simple header (width, height) followed by RGBA pixel data.

use alloc::vec::Vec;
use crate::graphics::surface::Surface;

/// A loaded RGBA icon image
pub struct Icon {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (4 bytes per pixel)
    pub pixels: Vec<u8>,
}

impl Icon {
    /// Parse an icon from raw binary data.
    /// Format: [width:u32 LE][height:u32 LE][RGBA pixel data]
    pub fn from_raw(data: &[u8]) -> Option<Icon> {
        if data.len() < 8 {
            return None;
        }
        let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let expected = (width * height * 4) as usize;
        if data.len() < 8 + expected || width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        let pixels = data[8..8 + expected].to_vec();
        Some(Icon { width, height, pixels })
    }

    /// Draw this icon onto a surface with alpha blending.
    pub fn draw_on(&self, surface: &mut Surface, x: i32, y: i32) {
        let sw = surface.width as i32;
        let sh = surface.height as i32;

        for row in 0..self.height as i32 {
            let py = y + row;
            if py < 0 || py >= sh { continue; }

            for col in 0..self.width as i32 {
                let px = x + col;
                if px < 0 || px >= sw { continue; }

                let src_off = ((row as u32 * self.width + col as u32) * 4) as usize;
                let alpha = self.pixels[src_off + 3];
                if alpha == 0 { continue; }

                let sr = self.pixels[src_off];
                let sg = self.pixels[src_off + 1];
                let sb = self.pixels[src_off + 2];

                if alpha == 255 {
                    // Fully opaque — direct write
                    let dst_off = (py as u32 * surface.width + px as u32) as usize;
                    surface.pixels[dst_off] =
                        (0xFF << 24) | ((sr as u32) << 16) | ((sg as u32) << 8) | (sb as u32);
                } else {
                    // Alpha blend
                    let dst_off = (py as u32 * surface.width + px as u32) as usize;
                    let dst = surface.pixels[dst_off];
                    let dr = ((dst >> 16) & 0xFF) as u32;
                    let dg = ((dst >> 8) & 0xFF) as u32;
                    let db = (dst & 0xFF) as u32;
                    let a = alpha as u32;
                    let inv_a = 255 - a;
                    let r = (sr as u32 * a + dr * inv_a) / 255;
                    let g = (sg as u32 * a + dg * inv_a) / 255;
                    let b = (sb as u32 * a + db * inv_a) / 255;
                    surface.pixels[dst_off] = (0xFF << 24) | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Load an icon from the filesystem
pub fn load_icon(path: &str) -> Option<Icon> {
    match crate::fs::vfs::read_file_to_vec(path) {
        Ok(data) => Icon::from_raw(&data),
        Err(_) => None,
    }
}
