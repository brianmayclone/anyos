// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Application scanning, metadata, and icon loading.

use anyos_std::{String, Vec};
use anyos_std::icons;

/// Icon size to load (pixels).
pub const ICON_SIZE: u32 = 20;

/// Metadata for an installed application bundle.
pub struct AppEntry {
    pub name: String,
    pub path: String,
    /// Decoded ARGB8888 icon pixels (ICON_SIZE x ICON_SIZE), or empty if missing.
    pub icon: Vec<u32>,
}

/// Scans `/Applications/` for `.app` bundles, returning them sorted by name.
pub fn scan_apps() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    for path in icons::collect_app_bundles() {
        let folder_name = path.rsplit('/').next().unwrap_or(path.as_str());
        let display_name = read_app_name(&path, folder_name);
        let icon = load_icon(&path);

        apps.push(AppEntry { name: display_name, path, icon });
    }
    apps.sort_by(|a, b| {
        let la = a.name.as_bytes();
        let lb = b.name.as_bytes();
        for i in 0..la.len().min(lb.len()) {
            let ca = la[i].to_ascii_lowercase();
            let cb = lb[i].to_ascii_lowercase();
            if ca != cb {
                return ca.cmp(&cb);
            }
        }
        la.len().cmp(&lb.len())
    });
    apps
}

/// Reads the display name from `Info.conf` inside a bundle, falling back to
/// the folder name (minus `.app` suffix).
fn read_app_name(bundle_path: &str, folder_name: &str) -> String {
    let mut conf = String::from(bundle_path);
    conf.push_str("/Info.conf");
    let fd = anyos_std::fs::open(&conf, 0);
    if fd != u32::MAX {
        let mut fbuf = [0u8; 512];
        let n = anyos_std::fs::read(fd, &mut fbuf);
        anyos_std::fs::close(fd);
        if n > 0 && n != u32::MAX {
            if let Ok(text) = core::str::from_utf8(&fbuf[..n as usize]) {
                for line in text.split('\n') {
                    let line = line.trim();
                    if let Some(rest) = line.strip_prefix("name=") {
                        let rest = rest.trim();
                        if !rest.is_empty() {
                            return String::from(rest);
                        }
                    }
                }
            }
        }
    }
    let base = &folder_name[..folder_name.len().saturating_sub(4)];
    String::from(base)
}

/// Load and decode the Icon.ico from an app bundle at ICON_SIZE.
fn load_icon(bundle_path: &str) -> Vec<u32> {
    let mut ico_path = String::from(bundle_path);
    ico_path.push_str("/Icon.ico");

    let data = match anyos_std::fs::read_to_vec(&ico_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    if data.is_empty() {
        return Vec::new();
    }

    let info = match libimage_client::probe_ico_size(&data, ICON_SIZE) {
        Some(i) => i,
        None => {
            // Fallback: try generic probe (might be PNG or BMP, not ICO)
            match libimage_client::probe(&data) {
                Some(i) => i,
                None => return Vec::new(),
            }
        }
    };

    let pixel_count = (info.width * info.height) as usize;
    let mut pixels = Vec::new();
    pixels.resize(pixel_count, 0u32);
    let mut scratch = Vec::new();
    scratch.resize(info.scratch_needed as usize, 0u8);

    // Try ICO decode first, then generic
    if libimage_client::decode_ico_size(&data, ICON_SIZE, &mut pixels, &mut scratch).is_ok() {
        // Scale down if decoded size > ICON_SIZE
        if info.width != ICON_SIZE || info.height != ICON_SIZE {
            let mut scaled = Vec::new();
            scaled.resize((ICON_SIZE * ICON_SIZE) as usize, 0u32);
            if libimage_client::scale_image(
                &pixels, info.width, info.height,
                &mut scaled, ICON_SIZE, ICON_SIZE,
                libimage_client::MODE_CONTAIN,
            ){
                return scaled;
            }
        }
        pixels
    } else if libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok() {
        if info.width != ICON_SIZE || info.height != ICON_SIZE {
            let mut scaled = Vec::new();
            scaled.resize((ICON_SIZE * ICON_SIZE) as usize, 0u32);
            if libimage_client::scale_image(
                &pixels, info.width, info.height,
                &mut scaled, ICON_SIZE, ICON_SIZE,
                libimage_client::MODE_CONTAIN,
            ){
                return scaled;
            }
        }
        pixels
    } else {
        Vec::new()
    }
}
