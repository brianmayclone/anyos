//! System control icons — pre-built icons for buttons and UI elements.
//!
//! Icons are loaded from `/System/media/icons/controls/{name}.png`.
//! Available icons: left, right, refresh, secure, insecure, check, clear, help.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

const CONTROLS_DIR: &str = "/System/media/icons/controls/";

/// A decoded control icon ready for rendering.
pub struct ControlIcon {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

/// Load a control icon by name, scaled to the desired size.
///
/// Looks for `/System/media/icons/controls/{name}.png`.
/// If `size` is non-zero, the icon is scaled to `size x size` pixels.
/// If `size` is 0, the icon is returned at its native resolution.
/// Returns `None` if the file doesn't exist or can't be decoded.
///
/// # Example
/// ```
/// let icon = load_control_icon("refresh", 16).unwrap();
/// imageview(win, x, y, icon.width, icon.height, &icon.pixels, icon.width, icon.height);
/// ```
pub fn load_control_icon(name: &str, size: u32) -> Option<ControlIcon> {
    // Build path
    let mut path = [0u8; 128];
    let prefix = CONTROLS_DIR.as_bytes();
    let name_b = name.as_bytes();
    let suffix = b".png";
    let total = prefix.len() + name_b.len() + suffix.len();
    if total >= path.len() { return None; }
    path[..prefix.len()].copy_from_slice(prefix);
    path[prefix.len()..prefix.len() + name_b.len()].copy_from_slice(name_b);
    path[prefix.len() + name_b.len()..total].copy_from_slice(suffix);

    let path_str = unsafe { core::str::from_utf8_unchecked(&path[..total]) };
    let icon = load_icon_file(path_str)?;

    if size > 0 && (icon.width != size || icon.height != size) {
        let mut scaled = vec![0u32; (size * size) as usize];
        if libimage_client::scale_image(
            &icon.pixels, icon.width, icon.height,
            &mut scaled, size, size,
            libimage_client::MODE_CONTAIN,
        ) {
            Some(ControlIcon { pixels: scaled, width: size, height: size })
        } else {
            Some(icon) // scaling failed, return original
        }
    } else {
        Some(icon)
    }
}

fn load_icon_file(path: &str) -> Option<ControlIcon> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX { return None; }

    // Get file size
    let mut stat = [0u32; 4];
    if anyos_std::fs::fstat(fd, &mut stat) != 0 {
        anyos_std::fs::close(fd);
        return None;
    }
    let size = stat[1] as usize;
    if size == 0 || size > 256 * 1024 {
        anyos_std::fs::close(fd);
        return None;
    }

    // Read file
    let mut data = vec![0u8; size];
    let mut read = 0usize;
    while read < size {
        let n = anyos_std::fs::read(fd, &mut data[read..]);
        if n == 0 || n == u32::MAX { break; }
        read += n as usize;
    }
    anyos_std::fs::close(fd);
    if read == 0 { return None; }

    // Probe
    let info = libimage_client::probe(&data[..read])?;

    // Decode
    let pixel_count = (info.width * info.height) as usize;
    let mut pixels = vec![0u32; pixel_count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    libimage_client::decode(&data[..read], &mut pixels, &mut scratch).ok()?;

    Some(ControlIcon {
        pixels,
        width: info.width,
        height: info.height,
    })
}
