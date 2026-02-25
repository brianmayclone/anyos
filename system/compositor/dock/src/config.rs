//! Dock configuration loading/saving — programs.conf and icon loading.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::fs;
use anyos_std::icons;

use crate::theme::DOCK_ICON_SIZE;
use crate::types::{DockItem, Icon};

const SYSTEM_CONFIG_PATH: &str = "/System/dock/programs.conf";

/// Get user-specific config path based on current UID: /Users/<username>/.dock.conf
fn user_config_path() -> Option<String> {
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 64];
    let len = anyos_std::process::getusername(uid, &mut name_buf);
    if len != u32::MAX && len > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..len as usize]) {
            return Some(format!("/Users/{}/.dock.conf", username));
        }
    }
    // Fallback to $HOME env var
    let mut home_buf = [0u8; 256];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    if hlen == u32::MAX || hlen == 0 {
        return None;
    }
    let home = core::str::from_utf8(&home_buf[..hlen as usize]).ok()?;
    Some(format!("{}/.dock.conf", home))
}

/// Parse dock config from text (shared by both system and user config).
fn parse_config(text: &str) -> Vec<DockItem> {
    let mut items = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.splitn(2, '|');
        let name = match parts.next() {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => continue,
        };
        let path = match parts.next() {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => continue,
        };

        items.push(DockItem {
            name: String::from(name),
            bin_path: String::from(path),
            icon: None,
            running: false,
            tid: 0,
            pinned: true,
        });
    }
    items
}

/// Read a config file and return its text content.
fn read_config_file(path: &str) -> Option<String> {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) != 0 {
        return None;
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 4096 {
        return None;
    }

    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut data = vec![0u8; file_size];
    let bytes_read = fs::read(fd, &mut data) as usize;
    fs::close(fd);

    if bytes_read == 0 {
        return None;
    }

    core::str::from_utf8(&data[..bytes_read]).ok().map(String::from)
}

/// Load dock items: try user config first, fall back to system config.
///
/// Format: one item per line: `name|path`
/// Lines starting with '#' are comments, empty lines are skipped.
pub fn load_dock_config() -> Vec<DockItem> {
    // Try user-specific config first
    if let Some(user_path) = user_config_path() {
        if let Some(text) = read_config_file(&user_path) {
            let items = parse_config(&text);
            if !items.is_empty() {
                return items;
            }
        }
    }

    // Fall back to system config
    if let Some(text) = read_config_file(SYSTEM_CONFIG_PATH) {
        return parse_config(&text);
    }

    Vec::new()
}

/// Save pinned dock items to user config ($HOME/.dock.conf).
pub fn save_dock_config(items: &[DockItem]) {
    let user_path = match user_config_path() {
        Some(p) => p,
        None => return,
    };

    let mut content = String::new();
    content.push_str("# Dock configuration\n");
    for item in items {
        if item.pinned {
            content.push_str(&item.name);
            content.push('|');
            content.push_str(&item.bin_path);
            content.push('\n');
        }
    }

    let _ = fs::write_bytes(&user_path, content.as_bytes());
}

const FINDER_NAME: &str = "Finder";
const FINDER_PATH: &str = "/Applications/Finder.app";

/// Ensure Finder is always present as the first pinned item.
pub fn ensure_finder(items: &mut Vec<DockItem>) {
    let has_finder = items.iter().any(|it| it.bin_path == FINDER_PATH);
    if !has_finder {
        items.insert(0, DockItem {
            name: String::from(FINDER_NAME),
            bin_path: String::from(FINDER_PATH),
            icon: None,
            running: false,
            tid: 0,
            pinned: true,
        });
    }
}

/// Check if a dock item is the Finder (cannot be removed).
pub fn is_finder(item: &DockItem) -> bool {
    item.bin_path == FINDER_PATH
}

/// Load and decode an ICO icon, scaling to DOCK_ICON_SIZE.
pub fn load_ico_icon(path: &str) -> Option<Icon> {
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

    let info = match libimage_client::probe_ico_size(&data, DOCK_ICON_SIZE) {
        Some(i) => i,
        None => match libimage_client::probe(&data) {
            Some(i) => i,
            None => return None,
        },
    };

    let src_w = info.width;
    let src_h = info.height;
    let src_pixels = (src_w as usize) * (src_h as usize);

    let mut pixels: Vec<u32> = Vec::new();
    pixels.resize(src_pixels, 0);
    let mut scratch: Vec<u8> = Vec::new();
    scratch.resize(info.scratch_needed as usize, 0);

    let decode_ok = if info.format == libimage_client::FMT_ICO {
        libimage_client::decode_ico_size(&data, DOCK_ICON_SIZE, &mut pixels, &mut scratch).is_ok()
    } else {
        libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok()
    };
    if !decode_ok {
        return None;
    }

    if src_w == DOCK_ICON_SIZE && src_h == DOCK_ICON_SIZE {
        return Some(Icon { width: DOCK_ICON_SIZE, height: DOCK_ICON_SIZE, pixels });
    }

    let dst_count = (DOCK_ICON_SIZE * DOCK_ICON_SIZE) as usize;
    let mut dst_pixels = vec![0u32; dst_count];
    libimage_client::scale_image(
        &pixels, src_w, src_h,
        &mut dst_pixels, DOCK_ICON_SIZE, DOCK_ICON_SIZE,
        libimage_client::MODE_SCALE,
    );

    Some(Icon { width: DOCK_ICON_SIZE, height: DOCK_ICON_SIZE, pixels: dst_pixels })
}

/// Load icons for all dock items (derives icon path from binary name).
pub fn load_icons(items: &mut [DockItem]) {
    for item in items.iter_mut() {
        let icon_path = icons::app_icon_path(&item.bin_path);
        item.icon = load_ico_icon(&icon_path);
    }
}
