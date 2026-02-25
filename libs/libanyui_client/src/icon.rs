//! Icon — class-based wrapper around libimage's icon decoding functions.
//!
//! # Usage
//! ```rust
//! let icon = Icon::for_filetype("txt").unwrap();
//! let image_view = icon.into_image_view(32, 32);
//!
//! let app_icon = Icon::for_application("terminal").unwrap();
//! let sys_icon = Icon::system("heart", IconType::Filled, 0xFF007AFF, 32).unwrap();
//! ```

use alloc::vec;
use alloc::vec::Vec;
use crate::controls::ImageView;

/// Icon style variant.
#[derive(Copy, Clone, PartialEq)]
pub enum IconType {
    Filled,
    Outline,
}

/// A decoded icon with its pixel data.
pub struct Icon {
    /// Decoded ARGB pixel buffer.
    pub pixels: Vec<u32>,
    /// Icon width in pixels.
    pub width: u32,
    /// Icon height in pixels.
    pub height: u32,
}

// ── ico.pak cache ─────────────────────────────────────

const ICO_PAK_PATH: &str = "/System/media/ico.pak";

static mut PAK_DATA: Option<Vec<u8>> = None;

fn pak_data() -> Option<&'static [u8]> {
    unsafe {
        if PAK_DATA.is_none() {
            PAK_DATA = Some(anyos_std::fs::read_to_vec(ICO_PAK_PATH).ok()?);
        }
        PAK_DATA.as_deref()
    }
}

// ── Rendered icon cache (fixed-size ring buffer) ──────

const ICON_CACHE_SIZE: usize = 64;

struct CacheEntry {
    key: u64,
    pixels: Vec<u32>,
    size: u32,
}

static mut ICON_CACHE: Option<Vec<CacheEntry>> = None;

fn icon_cache_key(name: &str, filled: bool, size: u32, color: u32) -> u64 {
    let mut h: u64 = if filled { 0x100000000 } else { 0 };
    h ^= (size as u64) << 40;
    h ^= color as u64;
    // FNV-1a hash of name
    let mut fnv: u64 = 0xcbf29ce484222325;
    for &b in name.as_bytes() {
        fnv ^= b as u64;
        fnv = fnv.wrapping_mul(0x100000001b3);
    }
    h ^= fnv << 8;
    h
}

fn icon_cache_lookup(key: u64) -> Option<(Vec<u32>, u32)> {
    unsafe {
        let cache = ICON_CACHE.as_ref()?;
        for entry in cache.iter() {
            if entry.key == key {
                return Some((entry.pixels.clone(), entry.size));
            }
        }
        None
    }
}

fn icon_cache_insert(key: u64, pixels: Vec<u32>, size: u32) {
    unsafe {
        if ICON_CACHE.is_none() {
            ICON_CACHE = Some(Vec::new());
        }
        let cache = ICON_CACHE.as_mut().unwrap();
        // Evict oldest if full
        if cache.len() >= ICON_CACHE_SIZE {
            cache.remove(0);
        }
        cache.push(CacheEntry { key, pixels, size });
    }
}

impl Icon {
    /// Load a system icon from ico.pak by name, type, color, and size.
    ///
    /// Icons are cached after first render — repeated calls with the same
    /// parameters return cached pixel data.
    ///
    /// # Example
    /// ```rust
    /// let icon = Icon::system("heart", IconType::Filled, 0xFF007AFF, 32).unwrap();
    /// ```
    pub fn system(name: &str, icon_type: IconType, color: u32, size: u32) -> Option<Self> {
        let filled = icon_type == IconType::Filled;
        let key = icon_cache_key(name, filled, size, color);

        // Check cache first
        if let Some((pixels, sz)) = icon_cache_lookup(key) {
            return Some(Self { pixels, width: sz, height: sz });
        }

        // Load pak and render
        let pak = pak_data()?;
        let pixel_count = (size as usize) * (size as usize);
        let mut pixels = vec![0u32; pixel_count];
        libimage_client::iconpack_render(pak, name, filled, size, color, &mut pixels).ok()?;

        // Cache the result
        icon_cache_insert(key, pixels.clone(), size);

        Some(Self { pixels, width: size, height: size })
    }

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

    /// Load a control icon by name from `/System/media/icons/controls/`.
    ///
    /// Uses pre-sized file reading (via fstat) and optional scaling.
    /// Available icons: left, right, refresh, folder, check, clear, help, etc.
    ///
    /// # Example
    /// ```rust
    /// let icon = Icon::control("refresh", 16).unwrap();
    /// ```
    pub fn control(name: &str, size: u32) -> Option<Self> {
        const DIR: &str = "/System/media/icons/controls/";
        let mut path = [0u8; 128];
        let prefix = DIR.as_bytes();
        let name_b = name.as_bytes();
        let base_len = prefix.len() + name_b.len();
        if base_len + 4 >= path.len() { return None; }
        path[..prefix.len()].copy_from_slice(prefix);
        path[prefix.len()..base_len].copy_from_slice(name_b);

        // Try .png first, then .ico fallback
        path[base_len..base_len + 4].copy_from_slice(b".png");
        let png_path = core::str::from_utf8(&path[..base_len + 4]).ok()?;
        let icon = Self::load_file_sized(png_path).or_else(|| {
            path[base_len..base_len + 4].copy_from_slice(b".ico");
            let ico_path = core::str::from_utf8(&path[..base_len + 4]).ok()?;
            Self::load_ico_sized(ico_path, size)
        })?;

        if size > 0 && (icon.width != size || icon.height != size) {
            let mut scaled = vec![0u32; (size * size) as usize];
            if libimage_client::scale_image(
                &icon.pixels, icon.width, icon.height,
                &mut scaled, size, size,
                libimage_client::MODE_CONTAIN,
            ) {
                Some(Self { pixels: scaled, width: size, height: size })
            } else {
                Some(icon)
            }
        } else {
            Some(icon)
        }
    }

    /// Load an ICO file with pre-sized allocation and ICO-specific decoder.
    fn load_ico_sized(path: &str, preferred_size: u32) -> Option<Self> {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX { return None; }

        let mut stat = [0u32; 4];
        if anyos_std::fs::fstat(fd, &mut stat) != 0 {
            anyos_std::fs::close(fd);
            return None;
        }
        let file_size = stat[1] as usize;
        if file_size == 0 || file_size > 256 * 1024 {
            anyos_std::fs::close(fd);
            return None;
        }

        let mut data = vec![0u8; file_size];
        let mut read = 0usize;
        while read < file_size {
            let n = anyos_std::fs::read(fd, &mut data[read..]);
            if n == 0 || n == u32::MAX { break; }
            read += n as usize;
        }
        anyos_std::fs::close(fd);
        if read == 0 { return None; }

        Self::from_ico_bytes(&data[..read], preferred_size)
    }

    /// Load any image file with pre-sized allocation (fstat + exact read).
    /// Caps at 256 KB file size.
    fn load_file_sized(path: &str) -> Option<Self> {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX { return None; }

        let mut stat = [0u32; 4];
        if anyos_std::fs::fstat(fd, &mut stat) != 0 {
            anyos_std::fs::close(fd);
            return None;
        }
        let file_size = stat[1] as usize;
        if file_size == 0 || file_size > 256 * 1024 {
            anyos_std::fs::close(fd);
            return None;
        }

        let mut data = vec![0u8; file_size];
        let mut read = 0usize;
        while read < file_size {
            let n = anyos_std::fs::read(fd, &mut data[read..]);
            if n == 0 || n == u32::MAX { break; }
            read += n as usize;
        }
        anyos_std::fs::close(fd);
        if read == 0 { return None; }

        let info = libimage_client::probe(&data[..read])?;
        let pixel_count = (info.width as usize) * (info.height as usize);
        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        libimage_client::decode(&data[..read], &mut pixels, &mut scratch).ok()?;

        Some(Self { pixels, width: info.width, height: info.height })
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

    /// Recolor all non-transparent pixels to the given ARGB color,
    /// preserving the original alpha channel.
    pub fn recolor(&mut self, color: u32) {
        let rgb = color & 0x00FFFFFF;
        for px in self.pixels.iter_mut() {
            let a = *px & 0xFF000000;
            if a != 0 {
                *px = a | rgb;
            }
        }
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
