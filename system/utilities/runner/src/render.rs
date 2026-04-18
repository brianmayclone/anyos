// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Canvas rendering — rounded background, category headers, result rows with icons.

use libanyui_client as ui;
use crate::apps::{AppEntry, ICON_SIZE};
use crate::search::{Category, SearchResult};
use crate::searchd;

// ── Layout constants ─────────────────────────────────────────────────────────

pub const WIN_WIDTH: u32 = 580;
pub const SEARCH_HEIGHT: u32 = 64;
pub const CORNER_RADIUS: u32 = 16;
/// Single-line row (apps).
pub const ROW_HEIGHT: u32 = 32;
/// Two-line row (files: name + path/size).
pub const FILE_ROW_HEIGHT: u32 = 44;
pub const CATEGORY_HEIGHT: u32 = 24;
pub const PADDING_X: u32 = 20;
pub const RESULT_PADDING_TOP: u32 = 4;
pub const RESULT_PADDING_BOTTOM: u32 = 12;
const DIVIDER_Y_OFFSET: u32 = 2;
const ICON_LEFT: u32 = PADDING_X;
const TEXT_LEFT: u32 = PADDING_X + ICON_SIZE + 8;

// ── Colors ───────────────────────────────────────────────────────────────────

pub const BG_COLOR: u32 = 0xE6_1E_1E_1E;
const CATEGORY_COLOR: u32 = 0xFF_8B_8B_8B;
const RESULT_COLOR: u32 = 0xFF_E8_E8_E8;
const SELECTED_BG: u32 = 0xFF_0A_64_D2;
const SELECTED_TEXT: u32 = 0xFF_FF_FF_FF;
const DIVIDER_COLOR: u32 = 0x40_FF_FF_FF;
const PATH_COLOR: u32 = 0xFF_6B_6B_6B;
const PATH_SELECTED: u32 = 0xCC_FF_FF_FF;

const ICON_DOC: u32 = 0xFF_4A_90_D9;
const ICON_IMG: u32 = 0xFF_7C_B3_42;
const ICON_DIR: u32 = 0xFF_E8_A8_38;
const ICON_OTHER: u32 = 0xFF_88_88_88;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn row_height(r: &SearchResult) -> u32 {
    match r {
        SearchResult::App { .. } => ROW_HEIGHT,
        SearchResult::File { .. } => FILE_ROW_HEIGHT,
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

pub fn calc_height(results: &[SearchResult]) -> u32 {
    if results.is_empty() {
        return SEARCH_HEIGHT;
    }
    let mut h = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    let mut last_cat: Option<Category> = None;
    for r in results.iter() {
        let cat = r.category();
        if last_cat != Some(cat) {
            if last_cat.is_some() { h += DIVIDER_Y_OFFSET; }
            h += CATEGORY_HEIGHT;
            last_cat = Some(cat);
        }
        h += row_height(r);
    }
    h + RESULT_PADDING_BOTTOM
}

pub fn hit_test(results: &[SearchResult], cy: i32) -> Option<usize> {
    if cy < SEARCH_HEIGHT as i32 || results.is_empty() {
        return None;
    }
    let mut y = (SEARCH_HEIGHT + RESULT_PADDING_TOP) as i32;
    let mut last_cat: Option<Category> = None;
    for (idx, r) in results.iter().enumerate() {
        let cat = r.category();
        if last_cat != Some(cat) {
            if last_cat.is_some() { y += DIVIDER_Y_OFFSET as i32; }
            y += CATEGORY_HEIGHT as i32;
            last_cat = Some(cat);
        }
        let rh = row_height(r) as i32;
        if cy >= y && cy < y + rh {
            return Some(idx);
        }
        y += rh;
    }
    None
}

pub fn draw(
    canvas: &ui::Canvas, w: u32, h: u32,
    results: &[SearchResult], apps: &[AppEntry], selected: usize,
) {
    let buf = canvas.get_buffer();
    if buf.is_null() { return; }
    let stride = canvas.get_stride().max(w);
    let buf_h = canvas.get_height().max(h);
    let total = (stride * buf_h) as usize;
    let pixels = unsafe { core::slice::from_raw_parts_mut(buf, total) };

    for p in pixels.iter_mut() { *p = 0x00_000000; }
    fill_rounded_rect(pixels, stride, 0, 0, w, h, CORNER_RADIUS, BG_COLOR);

    // Divider
    if !results.is_empty() {
        let dy = SEARCH_HEIGHT - 1;
        for x in PADDING_X..w.saturating_sub(PADDING_X) {
            let idx = (dy * stride + x) as usize;
            if idx < pixels.len() { pixels[idx] = DIVIDER_COLOR; }
        }
    }

    // ── Pass 1: backgrounds + icons (pixel level) ───────────────────────
    let mut y = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    let mut last_cat: Option<Category> = None;

    for (idx, r) in results.iter().enumerate() {
        let cat = r.category();
        if last_cat != Some(cat) {
            if last_cat.is_some() { y += DIVIDER_Y_OFFSET; }
            y += CATEGORY_HEIGHT;
            last_cat = Some(cat);
        }
        let rh = row_height(r);

        if idx == selected {
            fill_rounded_rect(pixels, stride,
                PADDING_X - 6, y, w - (PADDING_X - 6) * 2, rh, 8, SELECTED_BG);
        }

        let icon_y = y + (rh.saturating_sub(ICON_SIZE)) / 2;
        match r {
            SearchResult::App { app_idx } => {
                let app = &apps[*app_idx];
                if !app.icon.is_empty() {
                    blit_icon(pixels, stride, ICON_LEFT, icon_y, ICON_SIZE, &app.icon);
                }
            }
            SearchResult::File { kind, .. } => {
                let color = match kind.as_str() {
                    "document" | "text" | "script" | "config" => ICON_DOC,
                    "image" => ICON_IMG,
                    "directory" => ICON_DIR,
                    _ => ICON_OTHER,
                };
                fill_rounded_rect(pixels, stride,
                    ICON_LEFT, icon_y, ICON_SIZE, ICON_SIZE, 4, color);
            }
        }

        y += rh;
    }

    canvas.fill_rect(0, 0, 0, 0, 0);

    // ── Pass 2: text ────────────────────────────────────────────────────
    y = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    last_cat = None;

    for (idx, r) in results.iter().enumerate() {
        let cat = r.category();
        if last_cat != Some(cat) {
            if last_cat.is_some() { y += DIVIDER_Y_OFFSET; }
            canvas.draw_text(PADDING_X as i32, (y + 5) as i32,
                CATEGORY_COLOR, 1, 11, cat.label());
            y += CATEGORY_HEIGHT;
            last_cat = Some(cat);
        }

        let sel = idx == selected;
        let rh = row_height(r);
        let name = r.display_name(apps);
        let text_color = if sel { SELECTED_TEXT } else { RESULT_COLOR };

        match r {
            SearchResult::App { .. } => {
                canvas.draw_text(TEXT_LEFT as i32, (y + 7) as i32,
                    text_color, 0, 14, name);
            }
            SearchResult::File { path, size, .. } => {
                // Line 1: filename (bold)
                canvas.draw_text(TEXT_LEFT as i32, (y + 4) as i32,
                    text_color, 1, 13, name);
                // Line 2: path — size
                let path_color = if sel { PATH_SELECTED } else { PATH_COLOR };
                let size_str = searchd::fmt_size(*size);
                let mut detail = anyos_std::String::from(path.as_str());
                detail.push_str(" \u{2014} ");
                detail.push_str(&size_str);
                canvas.draw_text(TEXT_LEFT as i32, (y + 22) as i32,
                    path_color, 0, 10, &detail);
            }
        }

        y += rh;
    }
}

// ── Drawing primitives ───────────────────────────────────────────────────────

fn blend_over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 { return dst; }
    if sa == 255 { return src; }
    let inv = 255 - sa;
    let r = (((src >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv) / 255;
    let g = (((src >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv) / 255;
    let b = ((src & 0xFF) * sa + (dst & 0xFF) * inv) / 255;
    let a = sa + ((dst >> 24) & 0xFF) * inv / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

fn blit_icon(pixels: &mut [u32], stride: u32, x: u32, y: u32, size: u32, icon: &[u32]) {
    for row in 0..size {
        for col in 0..size {
            let si = (row * size + col) as usize;
            if si >= icon.len() { continue; }
            let src = icon[si];
            if (src >> 24) == 0 { continue; }
            let di = ((y + row) * stride + (x + col)) as usize;
            if di < pixels.len() { pixels[di] = blend_over(pixels[di], src); }
        }
    }
}

fn fill_rounded_rect(
    pixels: &mut [u32], stride: u32,
    rx: u32, ry: u32, rw: u32, rh: u32,
    radius: u32, color: u32,
) {
    let r = radius.min(rw / 2).min(rh / 2);
    let r_sq = (r * r) as i64;
    for py in ry..ry + rh {
        for px in rx..rx + rw {
            let lx = px - rx;
            let ly = py - ry;
            let inside = if ly < r && lx < r {
                let dx = r as i64 - 1 - lx as i64;
                let dy = r as i64 - 1 - ly as i64;
                dx * dx + dy * dy <= r_sq
            } else if ly < r && lx >= rw - r {
                let dx = lx as i64 - (rw - r) as i64;
                let dy = r as i64 - 1 - ly as i64;
                dx * dx + dy * dy <= r_sq
            } else if ly >= rh - r && lx < r {
                let dx = r as i64 - 1 - lx as i64;
                let dy = ly as i64 - (rh - r) as i64;
                dx * dx + dy * dy <= r_sq
            } else if ly >= rh - r && lx >= rw - r {
                let dx = lx as i64 - (rw - r) as i64;
                let dy = ly as i64 - (rh - r) as i64;
                dx * dx + dy * dy <= r_sq
            } else {
                true
            };
            if inside {
                let i = (py * stride + px) as usize;
                if i < pixels.len() { pixels[i] = color; }
            }
        }
    }
}
