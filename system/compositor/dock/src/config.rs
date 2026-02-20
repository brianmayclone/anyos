//! Dock configuration loading — programs.conf and icon loading.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::fs;
use anyos_std::icons;

use crate::theme::DOCK_ICON_SIZE;
use crate::types::{DockItem, Icon};

const CONFIG_PATH: &str = "/System/dock/programs.conf";

/// Load dock items from /System/dock/programs.conf.
///
/// Format: one item per line: `name|path`
/// Lines starting with '#' are comments, empty lines are skipped.
pub fn load_dock_config() -> Vec<DockItem> {
    let mut items = Vec::new();

    let mut stat_buf = [0u32; 7];
    if fs::stat(CONFIG_PATH, &mut stat_buf) != 0 {
        return items;
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 4096 {
        return items;
    }

    let fd = fs::open(CONFIG_PATH, 0);
    if fd == u32::MAX {
        return items;
    }

    let mut data = vec![0u8; file_size];
    let bytes_read = fs::read(fd, &mut data) as usize;
    fs::close(fd);

    if bytes_read == 0 {
        return items;
    }

    let text = match core::str::from_utf8(&data[..bytes_read]) {
        Ok(s) => s,
        Err(_) => return items,
    };

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
