// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Settings and bookmark persistence for Surf.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libconf_schema::{default_bool, default_string, manifest, RegistryScope, ServiceSchema};

const SURF_DIRS: &[&str] = &["config"];
const SURF_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("config/homepage", ""),
    default_string("config/bookmarks_json", "[]"),
    default_bool("config/js_enabled", true),
];
const SURF_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const SURF_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/surf",
    RegistryScope::User,
    1,
    SURF_DIRS,
    SURF_DEFAULTS,
    SURF_MIGRATIONS,
);
const SURF_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("surf", &SURF_MANIFEST);

// ═══════════════════════════════════════════════════════════
// Settings
// ═══════════════════════════════════════════════════════════

pub struct SurfConfig {
    pub homepage: String,
    pub js_enabled: bool,
}

impl SurfConfig {
    pub fn default() -> Self {
        Self {
            homepage: String::new(),
            js_enabled: true,
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Bookmarks
// ═══════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct BookmarkItem {
    pub title: String,
    pub url: String,
    pub is_folder: bool,
    pub children: Vec<BookmarkItem>,
    /// Favicon pixel data (ARGB8888, 16x16).
    pub icon: Vec<u32>,
    pub icon_w: u32,
    pub icon_h: u32,
}

pub struct BookmarkStore {
    pub roots: Vec<BookmarkItem>,
}

impl BookmarkStore {
    pub fn new() -> Self {
        Self { roots: Vec::new() }
    }
}

// ═══════════════════════════════════════════════════════════
// JSON serialization
// ═══════════════════════════════════════════════════════════

fn bookmark_to_json(item: &BookmarkItem) -> Value {
    let mut obj = Value::new_object();
    obj.set("title", Value::String(item.title.clone()));
    obj.set("url", Value::String(item.url.clone()));
    obj.set("is_folder", Value::Bool(item.is_folder));
    let mut children_arr = Value::new_array();
    for child in &item.children {
        children_arr.push(bookmark_to_json(child));
    }
    obj.set("children", children_arr);
    // Save favicon pixels as a JSON array of u32 values.
    if !item.icon.is_empty() && item.icon_w > 0 {
        let mut px_arr = Value::new_array();
        for &px in &item.icon {
            px_arr.push((px as i64).into());
        }
        obj.set("icon", px_arr);
        obj.set("icon_w", (item.icon_w as i64).into());
        obj.set("icon_h", (item.icon_h as i64).into());
    }
    obj
}

fn bookmark_from_json(val: &Value) -> BookmarkItem {
    let title = val["title"].as_str().unwrap_or("");
    let url = val["url"].as_str().unwrap_or("");
    let is_folder = val["is_folder"].as_bool().unwrap_or(false);
    let mut children = Vec::new();
    if let Some(arr) = val["children"].as_array() {
        for child_val in arr {
            children.push(bookmark_from_json(child_val));
        }
    }
    // Restore favicon pixels from JSON array.
    let mut icon = Vec::new();
    let icon_w = val["icon_w"].as_i64().unwrap_or(0) as u32;
    let icon_h = val["icon_h"].as_i64().unwrap_or(0) as u32;
    if let Some(px_arr) = val["icon"].as_array() {
        for px_val in px_arr {
            icon.push(px_val.as_i64().unwrap_or(0) as u32);
        }
    }
    BookmarkItem {
        title: String::from(title),
        url: String::from(url),
        is_folder,
        children,
        icon,
        icon_w,
        icon_h,
    }
}

// ═══════════════════════════════════════════════════════════
// Load / Save
// ═══════════════════════════════════════════════════════════

pub fn load() -> (SurfConfig, BookmarkStore) {
    let _ = SURF_SCHEMA.register();
    let homepage = SURF_SCHEMA
        .read_string("config/homepage")
        .unwrap_or_default();
    let js_enabled = SURF_SCHEMA.read_bool("config/js_enabled").unwrap_or(true);
    let mut store = BookmarkStore::new();
    if let Some(json) = SURF_SCHEMA.read_string("config/bookmarks_json") {
        if let Ok(arr) = Value::parse(&json) {
            if let Some(bookmarks) = arr.as_array() {
                for item_val in bookmarks {
                    store.roots.push(bookmark_from_json(item_val));
                }
            }
        }
    }
    (
        SurfConfig {
            homepage,
            js_enabled,
        },
        store,
    )
}

pub fn save(config: &SurfConfig, store: &BookmarkStore) {
    let mut bookmarks_arr = Value::new_array();
    for item in &store.roots {
        bookmarks_arr.push(bookmark_to_json(item));
    }
    let json = bookmarks_arr.to_json_string_pretty();
    let _ = SURF_SCHEMA.write_string("config/homepage", &config.homepage);
    let _ = SURF_SCHEMA.write_string("config/bookmarks_json", &json);
    let _ = SURF_SCHEMA.write_bool("config/js_enabled", config.js_enabled);
}

// ═══════════════════════════════════════════════════════════
// Favicon fetching
// ═══════════════════════════════════════════════════════════

/// Favicon target size in pixels.
const FAVICON_SIZE: u32 = 16;

/// Try to fetch and decode `/favicon.ico` from the bookmark's domain.
/// Returns (pixels, w, h) on success.
pub fn fetch_favicon(bookmark_url: &str) -> Option<(Vec<u32>, u32, u32)> {
    let parsed = crate::http::parse_url(bookmark_url).ok()?;
    let favicon_path = alloc::format!(
        "{}://{}:{}/favicon.ico",
        parsed.scheme,
        parsed.host,
        parsed.port
    );
    let favicon_url = crate::http::parse_url(&favicon_path).ok()?;

    let st = crate::state();
    let resp = crate::http::fetch(&favicon_url, &mut st.cookies, &mut st.conn_pool).ok()?;
    if resp.status != 200 || resp.body.is_empty() {
        return None;
    }

    // Try ICO-specific decoding first (picks best size from multi-res .ico).
    if let Some(info) = libimage_client::probe_ico_size(&resp.body, FAVICON_SIZE) {
        let w = info.width.min(64);
        let h = info.height.min(64);
        if w == 0 || h == 0 {
            return None;
        }
        let mut pixels = alloc::vec![0u32; (w * h) as usize];
        let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode_ico_size(&resp.body, FAVICON_SIZE, &mut pixels, &mut scratch)
            .is_ok()
        {
            let (pixels, w, h) = scale_icon(&pixels, w, h, FAVICON_SIZE);
            return Some((pixels, w, h));
        }
    }

    // Fallback: generic image probe (PNG, etc.).
    if let Some(info) = libimage_client::probe(&resp.body) {
        let w = info.width.min(256);
        let h = info.height.min(256);
        if w == 0 || h == 0 {
            return None;
        }
        let mut pixels = alloc::vec![0u32; (w * h) as usize];
        let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode(&resp.body, &mut pixels, &mut scratch).is_ok() {
            let (pixels, w, h) = scale_icon(&pixels, w, h, FAVICON_SIZE);
            return Some((pixels, w, h));
        }
    }

    None
}

/// Simple nearest-neighbor downscale to target_size x target_size.
fn scale_icon(src: &[u32], sw: u32, sh: u32, target: u32) -> (Vec<u32>, u32, u32) {
    if sw == target && sh == target {
        return (Vec::from(src), sw, sh);
    }
    let tw = target;
    let th = target;
    let mut dst = alloc::vec![0u32; (tw * th) as usize];
    for y in 0..th {
        let sy = (y * sh / th).min(sh - 1);
        for x in 0..tw {
            let sx = (x * sw / tw).min(sw - 1);
            dst[(y * tw + x) as usize] = src[(sy * sw + sx) as usize];
        }
    }
    (dst, tw, th)
}

/// Read the text content of a TextField.
pub fn get_field_text(field: &libanyui_client::TextField) -> String {
    let mut buf = [0u8; 2048];
    let len = field.get_text(&mut buf);
    if len > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return String::from(s);
        }
    }
    String::new()
}
