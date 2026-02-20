//! Icon cache — loads, decodes, and caches 16x16 ARGB icons.

use anyos_std::fs;
use anyos_std::String;
use anyos_std::Vec;

use crate::types::ICON_DISPLAY_SIZE;

struct CachedIcon {
    path: String,
    pixels: [u32; 256],
}

pub struct IconCache {
    entries: Vec<CachedIcon>,
}

impl IconCache {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn get(&self, path: &str) -> Option<&[u32; 256]> {
        for e in &self.entries {
            if e.path == path {
                return Some(&e.pixels);
            }
        }
        None
    }

    pub fn get_or_load(&mut self, path: &str) -> Option<&[u32; 256]> {
        for i in 0..self.entries.len() {
            if self.entries[i].path == path {
                return Some(&self.entries[i].pixels);
            }
        }

        let fd = fs::open(path, 0);
        if fd == u32::MAX {
            return None;
        }

        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            data.extend_from_slice(&buf[..n as usize]);
        }
        fs::close(fd);

        if data.is_empty() {
            return None;
        }

        let info = match libimage_client::probe(&data) {
            Some(i) => i,
            None => return None,
        };

        let src_w = info.width;
        let src_h = info.height;
        let src_pixels = (src_w as usize) * (src_h as usize);

        let mut pixels: Vec<u32> = Vec::new();
        pixels.resize(src_pixels, 0);
        let mut scratch: Vec<u8> = Vec::new();
        scratch.resize(info.scratch_needed as usize, 0);

        if libimage_client::decode(&data, &mut pixels, &mut scratch).is_err() {
            return None;
        }

        let mut icon_pixels = [0u32; 256];
        if src_w == ICON_DISPLAY_SIZE && src_h == ICON_DISPLAY_SIZE {
            icon_pixels.copy_from_slice(&pixels[..256]);
        } else {
            libimage_client::scale_image(
                &pixels, src_w, src_h,
                &mut icon_pixels, ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE,
                libimage_client::MODE_SCALE,
            );
        }

        self.entries.push(CachedIcon {
            path: String::from(path),
            pixels: icon_pixels,
        });

        let last = self.entries.len() - 1;
        Some(&self.entries[last].pixels)
    }
}

/// Alpha-blend icon pixels against a solid background color, then blit.
pub fn blit_icon_alpha(win: u32, x: i16, y: i16, pixels: &[u32; 256], bg: u32) {
    let mut buf = [0u32; 256];
    let bg_r = (bg >> 16) & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bg_b = bg & 0xFF;
    for i in 0..256 {
        let px = pixels[i];
        let a = (px >> 24) & 0xFF;
        if a >= 255 {
            buf[i] = px | 0xFF000000;
        } else if a == 0 {
            buf[i] = bg | 0xFF000000;
        } else {
            let inv = 255 - a;
            let r = (((px >> 16) & 0xFF) * a + bg_r * inv) / 255;
            let g = (((px >> 8) & 0xFF) * a + bg_g * inv) / 255;
            let b = ((px & 0xFF) * a + bg_b * inv) / 255;
            buf[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }
    anyos_std::ui::window::blit(win, x, y, ICON_DISPLAY_SIZE as u16, ICON_DISPLAY_SIZE as u16, &buf);
}

/// Tint icon pixels: replace RGB with tint color, preserve alpha. If `dimmed`, reduce alpha.
pub fn tint_icon(pixels: &[u32], count: usize, tint: u32, dimmed: bool) -> [u32; 256] {
    let tr = (tint >> 16) & 0xFF;
    let tg = (tint >> 8) & 0xFF;
    let tb = tint & 0xFF;
    let mut out = [0u32; 256];
    for i in 0..count.min(256) {
        let a = (pixels[i] >> 24) & 0xFF;
        if a == 0 { continue; }
        let a = if dimmed { a / 3 } else { a };
        out[i] = (a << 24) | (tr << 16) | (tg << 8) | tb;
    }
    out
}
