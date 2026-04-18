//! Dock configuration loading/saving — programs.conf and icon loading.

use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::fs;
use anyos_std::icons;
use anyos_std::vec;
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};

use crate::types::{DockItem, Icon};

const LEGACY_DOCK_DEFAULT: &str = "\
# Dock programs configuration\n\
# Format: name|path\n\
Finder|/Applications/Finder.app\n\
Surf|/Applications/Surf.app\n\
Terminal|/Applications/Terminal.app\n\
Activity Monitor|/Applications/Activity Monitor.app\n\
Settings|/Applications/Settings.app\n";

const DOCK_ITEMS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_string("config/pinned_items_blob", LEGACY_DOCK_DEFAULT)];
const DOCK_ITEMS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const DOCK_ITEMS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "profile/dock_items",
    RegistryScope::User,
    1,
    &["config"],
    DOCK_ITEMS_DEFAULTS,
    DOCK_ITEMS_MIGRATIONS,
);

fn dock_items_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("dock", &DOCK_ITEMS_MANIFEST)
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
            icon_hires: None,
            running: false,
            tid: 0,
            pinned: true,
        });
    }
    items
}

/// Check if the system booted from a live CD (root filesystem is ISO 9660).
fn is_live_cd_boot() -> bool {
    let mut buf = [0u8; 1024];
    let n = anyos_std::fs::list_mounts(&mut buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
        for line in text.split('\n') {
            let mut parts = line.splitn(3, '\t');
            let mount = parts.next().unwrap_or("");
            let fstype = parts.next().unwrap_or("");
            if mount == "/" && fstype == "iso9660" {
                return true;
            }
        }
    }
    false
}

const INSTALLER_NAME: &str = "Installer";
const INSTALLER_PATH: &str = "/Applications/Installer.app";

/// If booting from live CD, ensure the Installer is in the dock (after Finder).
fn ensure_installer_on_live_cd(items: &mut Vec<DockItem>) {
    if !is_live_cd_boot() {
        return;
    }
    // Don't add if already present
    if items.iter().any(|it| it.bin_path == INSTALLER_PATH) {
        return;
    }
    // Insert after Finder (index 1), or at the beginning if no Finder
    let pos = items.iter().position(|it| it.bin_path == FINDER_PATH)
        .map(|i| i + 1)
        .unwrap_or(0);
    items.insert(pos, DockItem {
        name: String::from(INSTALLER_NAME),
        bin_path: String::from(INSTALLER_PATH),
        icon: None,
        icon_hires: None,
        running: false,
        tid: 0,
        pinned: true,
    });
}

/// Load dock items from config file.
///
/// Format: one item per line: `name|path`
/// Lines starting with '#' are comments, empty lines are skipped.
pub fn load_dock_config() -> Vec<DockItem> {
    let _ = dock_items_schema().register();
    let raw = dock_items_schema()
        .read_string("config/pinned_items_blob")
        .unwrap_or_else(|| String::from(LEGACY_DOCK_DEFAULT));
    let mut items = parse_config(&raw);

    if items.is_empty() {
        items = parse_config(LEGACY_DOCK_DEFAULT);
        let _ = dock_items_schema().write_string("config/pinned_items_blob", LEGACY_DOCK_DEFAULT);
    }

    // On live CD boot, add Installer to the dock
    ensure_installer_on_live_cd(&mut items);

    items
}

/// Save pinned dock items to the dock config file.
pub fn save_dock_config(items: &[DockItem]) {
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

    let _ = dock_items_schema().write_string("config/pinned_items_blob", &content);
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
            icon_hires: None,
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

/// Read raw icon file data from disk.
fn read_icon_file(path: &str) -> Option<Vec<u8>> {
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

    if data.is_empty() { None } else { Some(data) }
}

/// Decode an icon from raw file data at the given target pixel size.
fn decode_icon_at_size(data: &[u8], target_size: u32) -> Option<Icon> {
    let info = match libimage_client::probe_ico_size(data, target_size) {
        Some(i) => i,
        None => match libimage_client::probe(data) {
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
        libimage_client::decode_ico_size(data, target_size, &mut pixels, &mut scratch).is_ok()
    } else {
        libimage_client::decode(data, &mut pixels, &mut scratch).is_ok()
    };
    if !decode_ok {
        return None;
    }

    let dst_count = (target_size * target_size) as usize;
    let mut dst_pixels = vec![0u32; dst_count];

    // Trim transparent borders and scale content to fill target_size.
    libimage_client::trim_and_scale(
        &pixels, src_w, src_h,
        &mut dst_pixels, target_size, target_size,
    );

    Some(Icon { width: target_size, height: target_size, pixels: dst_pixels })
}

/// Load and decode an ICO icon at the given target size.
pub fn load_ico_icon(path: &str, target_size: u32) -> Option<Icon> {
    let data = read_icon_file(path)?;
    decode_icon_at_size(&data, target_size)
}

/// Load icons for all dock items at base icon_size.
pub fn load_icons(items: &mut [DockItem], icon_size: u32) {
    for item in items.iter_mut() {
        let icon_path = icons::app_icon_path(&item.bin_path);
        item.icon = load_ico_icon(&icon_path, icon_size);
    }
}

/// Load high-resolution icons for magnification at mag_size.
pub fn load_icons_hires(items: &mut [DockItem], mag_size: u32) {
    for item in items.iter_mut() {
        let icon_path = icons::app_icon_path(&item.bin_path);
        item.icon_hires = load_ico_icon(&icon_path, mag_size);
    }
}
