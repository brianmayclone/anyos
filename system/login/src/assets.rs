//! File I/O and image loading helpers.

use alloc::vec;
use alloc::vec::Vec;

use anyos_std::fs;

/// Load a file from disk into a Vec<u8>.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut content = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    Some(content)
}

/// Load and decode a PNG image, returning (pixels, width, height).
pub fn load_png(path: &str) -> Option<(Vec<u32>, u32, u32)> {
    let data = read_file(path)?;
    let info = libimage_client::probe(&data)?;
    let w = info.width;
    let h = info.height;
    let mut pixels = vec![0u32; (w as usize).saturating_mul(h as usize)];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    libimage_client::decode(&data, &mut pixels, &mut scratch).ok()?;
    Some((pixels, w, h))
}

/// Pre-scaled logo: (pixels, display_w, display_h).
/// Uses the white logo for dark mode and the black logo for light mode.
pub fn load_and_scale_logo(target_h: u32) -> Option<(Vec<u32>, u32, u32)> {
    let path = if libanyui_client::theme::is_light() {
        "/System/media/anyos_b.png"
    } else {
        "/System/media/anyos_w.png"
    };
    let (pixels, img_w, img_h) = load_png(path)?;
    if img_h == 0 {
        return None;
    }
    let display_w = (img_w as u64 * target_h as u64 / img_h as u64) as u32;
    if display_w == 0 {
        return None;
    }
    let mut scaled = vec![0u32; (display_w as usize).saturating_mul(target_h as usize)];
    if libimage_client::scale_image(&pixels, img_w, img_h, &mut scaled, display_w, target_h, libimage_client::MODE_SCALE) {
        Some((scaled, display_w, target_h))
    } else {
        Some((pixels, img_w, img_h))
    }
}
