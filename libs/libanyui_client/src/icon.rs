//! Icon — class-based wrapper around libimage's icon decoding functions.
//!
//! # Usage
//! ```rust
//! let icon = Icon::for_filetype("txt").unwrap();
//! let image_view = icon.into_image_view(32, 32);
//!
//! let app_icon = Icon::for_application("terminal").unwrap();
//! ```

use alloc::vec;
use alloc::vec::Vec;
use crate::controls::ImageView;

/// A decoded icon with its pixel data.
pub struct Icon {
    /// Decoded ARGB pixel buffer.
    pub pixels: Vec<u32>,
    /// Icon width in pixels.
    pub width: u32,
    /// Icon height in pixels.
    pub height: u32,
}

impl Icon {
    /// Load an icon for a file extension (e.g., "txt", "png", "rs").
    ///
    /// Reads `/System/media/icons/mimetypes.conf` to map extension to icon name,
    /// then loads the ICO file from `/System/media/icons/`.
    pub fn for_filetype(ext: &str) -> Option<Self> {
        Self::for_filetype_sized(ext, 32)
    }

    /// Load an icon for a file extension at a specific size.
    pub fn for_filetype_sized(ext: &str, size: u32) -> Option<Self> {
        // Read mimetypes.conf to find the icon name for this extension
        let conf = anyos_std::fs::read_to_vec("/System/media/icons/mimetypes.conf").ok()?;
        let icon_name = parse_mimetype_conf(&conf, ext.as_bytes())?;

        // Load the ICO file
        let mut path_buf = [0u8; 128];
        let prefix = b"/System/media/icons/";
        let suffix = b".ico";
        if prefix.len() + icon_name.len() + suffix.len() >= path_buf.len() {
            return None;
        }
        path_buf[..prefix.len()].copy_from_slice(prefix);
        path_buf[prefix.len()..prefix.len() + icon_name.len()].copy_from_slice(icon_name);
        path_buf[prefix.len() + icon_name.len()..prefix.len() + icon_name.len() + suffix.len()].copy_from_slice(suffix);
        let path_len = prefix.len() + icon_name.len() + suffix.len();
        let path = core::str::from_utf8(&path_buf[..path_len]).ok()?;

        Self::load(path, size)
    }

    /// Load an icon for an application by name (e.g., "terminal", "files").
    ///
    /// Looks in `/System/media/icons/apps/<name>.ico`.
    pub fn for_application(name: &str) -> Option<Self> {
        Self::for_application_sized(name, 32)
    }

    /// Load an application icon at a specific size.
    pub fn for_application_sized(name: &str, size: u32) -> Option<Self> {
        let mut path_buf = [0u8; 128];
        let prefix = b"/System/media/icons/apps/";
        let name_bytes = name.as_bytes();
        let suffix = b".ico";
        if prefix.len() + name_bytes.len() + suffix.len() >= path_buf.len() {
            return None;
        }
        path_buf[..prefix.len()].copy_from_slice(prefix);
        path_buf[prefix.len()..prefix.len() + name_bytes.len()].copy_from_slice(name_bytes);
        path_buf[prefix.len() + name_bytes.len()..prefix.len() + name_bytes.len() + suffix.len()].copy_from_slice(suffix);
        let path_len = prefix.len() + name_bytes.len() + suffix.len();
        let path = core::str::from_utf8(&path_buf[..path_len]).ok()?;

        Self::load(path, size)
    }

    /// Load an icon from an ICO file at a preferred size.
    pub fn load(path: &str, preferred_size: u32) -> Option<Self> {
        let data = anyos_std::fs::read_to_vec(path).ok()?;
        Self::from_ico_bytes(&data, preferred_size)
    }

    /// Decode an ICO from raw bytes at a preferred size.
    pub fn from_ico_bytes(data: &[u8], preferred_size: u32) -> Option<Self> {
        let info = libimage_client::probe_ico_size(data, preferred_size)?;
        let pixel_count = (info.width as usize) * (info.height as usize);
        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        libimage_client::decode_ico_size(data, preferred_size, &mut pixels, &mut scratch).ok()?;
        Some(Self {
            pixels,
            width: info.width,
            height: info.height,
        })
    }

    /// Decode any image format (BMP, PNG, JPEG, GIF, ICO) from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let info = libimage_client::probe(data)?;
        let pixel_count = (info.width as usize) * (info.height as usize);
        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        libimage_client::decode(data, &mut pixels, &mut scratch).ok()?;
        Some(Self {
            pixels,
            width: info.width,
            height: info.height,
        })
    }

    /// Create an ImageView control from this icon.
    pub fn into_image_view(self, display_w: u32, display_h: u32) -> ImageView {
        let iv = ImageView::new(display_w, display_h);
        iv.set_pixels(&self.pixels, self.width, self.height);
        iv
    }

    /// Set this icon's pixels into an existing ImageView.
    pub fn apply_to(&self, image_view: &ImageView) {
        image_view.set_pixels(&self.pixels, self.width, self.height);
    }
}

/// Parse mimetypes.conf to find icon name for an extension.
///
/// Format: each line is `extension=iconname` (or lines starting with `#` are comments).
fn parse_mimetype_conf<'a>(data: &'a [u8], ext: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i < data.len() {
        // Find end of line
        let line_start = i;
        while i < data.len() && data[i] != b'\n' {
            i += 1;
        }
        let line = &data[line_start..i];
        if i < data.len() { i += 1; } // skip \n

        // Skip empty lines and comments
        if line.is_empty() || line[0] == b'#' {
            continue;
        }

        // Find '=' separator
        if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];
            // Trim trailing \r
            let value = if value.last() == Some(&b'\r') {
                &value[..value.len() - 1]
            } else {
                value
            };
            // Case-insensitive comparison for the extension
            if key.len() == ext.len() && key.iter().zip(ext.iter()).all(|(&a, &b)| a.to_ascii_lowercase() == b.to_ascii_lowercase()) {
                return Some(value);
            }
        }
    }
    None
}
