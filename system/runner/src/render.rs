// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Canvas rendering — rounded background, category headers, result rows with icons.

use libanyui_client as ui;
use crate::apps::{AppEntry, ICON_SIZE};
use crate::search::{Category, SearchResult};

// ── Layout constants ─────────────────────────────────────────────────────────

pub const WIN_WIDTH: u32 = 580;
pub const SEARCH_HEIGHT: u32 = 64;
pub const CORNER_RADIUS: u32 = 16;
pub const ROW_HEIGHT: u32 = 32;
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

// ── Public API ───────────────────────────────────────────────────────────────

/// Calculate total window height for the given results.
pub fn calc_height(results: &[SearchResult]) -> u32 {
    if results.is_empty() {
        return SEARCH_HEIGHT;
    }

    let mut h = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    let mut last_cat: Option<Category> = None;

    for r in results.iter() {
        if last_cat != Some(r.category) {
            if last_cat.is_some() {
                h += DIVIDER_Y_OFFSET;
            }
            h += CATEGORY_HEIGHT;
            last_cat = Some(r.category);
        }
        h += ROW_HEIGHT;
    }

    h + RESULT_PADDING_BOTTOM
}

/// Determine which result index was clicked at canvas coordinate `cy`.
/// Returns `None` if the click is in the search bar or outside results.
pub fn hit_test(results: &[SearchResult], cy: i32) -> Option<usize> {
    if cy < SEARCH_HEIGHT as i32 || results.is_empty() {
        return None;
    }

    let mut y = (SEARCH_HEIGHT + RESULT_PADDING_TOP) as i32;
    let mut last_cat: Option<Category> = None;

    for (idx, r) in results.iter().enumerate() {
        if last_cat != Some(r.category) {
            if last_cat.is_some() {
                y += DIVIDER_Y_OFFSET as i32;
            }
            y += CATEGORY_HEIGHT as i32;
            last_cat = Some(r.category);
        }

        if cy >= y && cy < y + ROW_HEIGHT as i32 {
            return Some(idx);
        }
        y += ROW_HEIGHT as i32;
    }

    None
}

/// Redraw the entire canvas: rounded background + results with icons.
pub fn draw(
    canvas: &ui::Canvas, w: u32, h: u32,
    results: &[SearchResult], apps: &[AppEntry], selected: usize,
) {
    // ── Pass 1: pixel-level drawing ─────────────────────────────────────
    let buf = canvas.get_buffer();
    if buf.is_null() {
        return;
    }
    let stride = canvas.get_stride().max(w);
    let buf_h = canvas.get_height().max(h);
    let total = (stride * buf_h) as usize;
    let pixels = unsafe { core::slice::from_raw_parts_mut(buf, total) };

    // Clear to transparent
    for p in pixels.iter_mut() {
        *p = 0x00_000000;
    }

    // Rounded background
    fill_rounded_rect(pixels, stride, 0, 0, w, h, CORNER_RADIUS, BG_COLOR);

    // Divider between search bar and results
    if !results.is_empty() {
        let dy = SEARCH_HEIGHT - 1;
        for x in PADDING_X..w.saturating_sub(PADDING_X) {
            let idx = (dy * stride + x) as usize;
            if idx < pixels.len() {
                pixels[idx] = DIVIDER_COLOR;
            }
        }
    }

    // Selection highlights + icons
    let mut y = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    let mut last_cat: Option<Category> = None;

    for (idx, r) in results.iter().enumerate() {
        if last_cat != Some(r.category) {
            if last_cat.is_some() {
                y += DIVIDER_Y_OFFSET;
            }
            y += CATEGORY_HEIGHT;
            last_cat = Some(r.category);
        }

        // Selection highlight
        if idx == selected {
            fill_rounded_rect(
                pixels, stride,
                PADDING_X - 6, y,
                w - (PADDING_X - 6) * 2, ROW_HEIGHT,
                8, SELECTED_BG,
            );
        }

        // Blit icon
        let app = &apps[r.app_idx];
        if !app.icon.is_empty() {
            let icon_y = y + (ROW_HEIGHT.saturating_sub(ICON_SIZE)) / 2;
            blit_icon(pixels, stride, ICON_LEFT, icon_y, ICON_SIZE, &app.icon);
        }

        y += ROW_HEIGHT;
    }

    // Flush pixel changes
    canvas.fill_rect(0, 0, 0, 0, 0);

    // ── Pass 2: text rendering via canvas API ───────────────────────────
    y = SEARCH_HEIGHT + RESULT_PADDING_TOP;
    last_cat = None;

    for (idx, r) in results.iter().enumerate() {
        if last_cat != Some(r.category) {
            if last_cat.is_some() {
                y += DIVIDER_Y_OFFSET;
            }
            canvas.draw_text(
                PADDING_X as i32, (y + 5) as i32,
                CATEGORY_COLOR, 1, 11,
                r.category.label(),
            );
            y += CATEGORY_HEIGHT;
            last_cat = Some(r.category);
        }

        let text_color = if idx == selected { SELECTED_TEXT } else { RESULT_COLOR };
        canvas.draw_text(
            TEXT_LEFT as i32, (y + 7) as i32,
            text_color, 0, 14,
            &apps[r.app_idx].name,
        );

        y += ROW_HEIGHT;
    }
}

// ── Drawing primitives ───────────────────────────────────────────────────────

/// Alpha-blend a source pixel over a destination pixel.
fn blend_over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 {
        return dst;
    }
    if sa == 255 {
        return src;
    }
    let inv = 255 - sa;
    let r = (((src >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv) / 255;
    let g = (((src >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv) / 255;
    let b = ((src & 0xFF) * sa + (dst & 0xFF) * inv) / 255;
    let a = sa + ((dst >> 24) & 0xFF) * inv / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Blit ARGB8888 icon pixels onto the canvas buffer with alpha blending.
fn blit_icon(pixels: &mut [u32], stride: u32, x: u32, y: u32, size: u32, icon: &[u32]) {
    let icon_w = size;
    for row in 0..size {
        for col in 0..size {
            let si = (row * icon_w + col) as usize;
            if si >= icon.len() {
                continue;
            }
            let src = icon[si];
            if (src >> 24) == 0 {
                continue;
            }
            let di = ((y + row) * stride + (x + col)) as usize;
            if di < pixels.len() {
                pixels[di] = blend_over(pixels[di], src);
            }
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
                if i < pixels.len() {
                    pixels[i] = color;
                }
            }
        }
    }
}
