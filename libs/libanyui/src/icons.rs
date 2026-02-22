//! Pixel-art icon rendering using fill_rect primitives.
//!
//! Each icon is 16x16 and drawn at a given position and color.
//! Icons are identified by ID constants.

use crate::draw::{Surface, fill_rect};

// ── Icon IDs ────────────────────────────────────────────────────────

pub const ICON_NEW_FILE: u32 = 1;
pub const ICON_FOLDER_OPEN: u32 = 2;
pub const ICON_SAVE: u32 = 3;
pub const ICON_SAVE_ALL: u32 = 4;
pub const ICON_BUILD: u32 = 5;
pub const ICON_PLAY: u32 = 6;
pub const ICON_STOP: u32 = 7;
pub const ICON_SETTINGS: u32 = 8;
pub const ICON_FILES: u32 = 9;
pub const ICON_GIT_BRANCH: u32 = 10;
pub const ICON_SEARCH: u32 = 11;

/// Draw a 16x16 icon at (x, y) with the given color.
pub fn draw_icon(s: &Surface, x: i32, y: i32, icon_id: u32, color: u32) {
    match icon_id {
        ICON_NEW_FILE => draw_new_file(s, x, y, color),
        ICON_FOLDER_OPEN => draw_folder_open(s, x, y, color),
        ICON_SAVE => draw_save(s, x, y, color),
        ICON_SAVE_ALL => draw_save_all(s, x, y, color),
        ICON_BUILD => draw_build(s, x, y, color),
        ICON_PLAY => draw_play(s, x, y, color),
        ICON_STOP => draw_stop(s, x, y, color),
        ICON_SETTINGS => draw_settings(s, x, y, color),
        ICON_FILES => draw_files(s, x, y, color),
        ICON_GIT_BRANCH => draw_git_branch(s, x, y, color),
        ICON_SEARCH => draw_search(s, x, y, color),
        _ => {}
    }
}

// ── New file: document with folded corner and "+" ───────────────────

fn draw_new_file(s: &Surface, x: i32, y: i32, c: u32) {
    let d = dim(c);
    // Document body — left portion
    fill_rect(s, x + 3, y + 1, 7, 14, c);
    // Right portion below fold
    fill_rect(s, x + 10, y + 5, 3, 10, c);
    // Fold triangle (stepped diagonal)
    fill_rect(s, x + 10, y + 4, 3, 1, c);
    fill_rect(s, x + 11, y + 3, 2, 1, c);
    fill_rect(s, x + 12, y + 2, 1, 1, c);
    // Fold face (darker)
    fill_rect(s, x + 10, y + 1, 1, 3, d);
    fill_rect(s, x + 11, y + 1, 1, 2, d);
    fill_rect(s, x + 12, y + 1, 1, 1, d);
    // "+" sign centered
    fill_rect(s, x + 5, y + 8, 5, 1, d);
    fill_rect(s, x + 7, y + 6, 1, 5, d);
}

// ── Folder open ─────────────────────────────────────────────────────

fn draw_folder_open(s: &Surface, x: i32, y: i32, c: u32) {
    // Tab on top-left
    fill_rect(s, x + 1, y + 3, 5, 2, c);
    // Back of folder
    fill_rect(s, x + 1, y + 5, 13, 2, c);
    // Open front flap (lighter, shifted left)
    fill_rect(s, x + 0, y + 7, 13, 6, lighten(c));
    // Inner shadow
    fill_rect(s, x + 1, y + 8, 11, 4, dim(c));
}

// ── Save (floppy disk) ──────────────────────────────────────────────

fn draw_save(s: &Surface, x: i32, y: i32, c: u32) {
    // Disk body
    fill_rect(s, x + 2, y + 1, 12, 14, c);
    // Metal shutter (top, darker)
    fill_rect(s, x + 4, y + 1, 7, 5, dim(c));
    // Shutter slider hole
    fill_rect(s, x + 8, y + 2, 2, 3, c);
    // Label area (bottom, lighter)
    fill_rect(s, x + 4, y + 9, 8, 5, lighten(c));
    // Label line
    fill_rect(s, x + 5, y + 11, 6, 1, dim(c));
}

// ── Save all (two stacked floppy disks) ─────────────────────────────

fn draw_save_all(s: &Surface, x: i32, y: i32, c: u32) {
    // Back disk shadow
    fill_rect(s, x + 4, y + 0, 11, 12, dim(c));
    // Front disk body
    fill_rect(s, x + 1, y + 3, 11, 12, c);
    // Metal shutter
    fill_rect(s, x + 3, y + 3, 7, 4, dim(c));
    // Slider hole
    fill_rect(s, x + 7, y + 4, 2, 2, c);
    // Label area
    fill_rect(s, x + 3, y + 10, 7, 4, lighten(c));
}

// ── Build (wrench) ──────────────────────────────────────────────────

fn draw_build(s: &Surface, x: i32, y: i32, c: u32) {
    // Wrench head (top-right circle-ish)
    fill_rect(s, x + 9, y + 1, 4, 1, c);
    fill_rect(s, x + 8, y + 2, 6, 1, c);
    fill_rect(s, x + 8, y + 3, 6, 1, c);
    fill_rect(s, x + 9, y + 4, 4, 1, c);
    // Jaw opening
    fill_rect(s, x + 10, y + 2, 2, 2, dim(c));
    // Handle (diagonal steps)
    fill_rect(s, x + 8, y + 5, 3, 1, c);
    fill_rect(s, x + 7, y + 6, 3, 1, c);
    fill_rect(s, x + 6, y + 7, 3, 1, c);
    fill_rect(s, x + 5, y + 8, 3, 1, c);
    fill_rect(s, x + 4, y + 9, 3, 1, c);
    fill_rect(s, x + 3, y + 10, 3, 1, c);
    fill_rect(s, x + 2, y + 11, 3, 2, c);
    // Handle end (wider)
    fill_rect(s, x + 1, y + 13, 4, 1, c);
}

// ── Play triangle ───────────────────────────────────────────────────

fn draw_play(s: &Surface, x: i32, y: i32, c: u32) {
    // Smooth triangle pointing right
    fill_rect(s, x + 4, y + 2, 1, 12, c);
    fill_rect(s, x + 5, y + 3, 1, 10, c);
    fill_rect(s, x + 6, y + 3, 1, 10, c);
    fill_rect(s, x + 7, y + 4, 1, 8, c);
    fill_rect(s, x + 8, y + 4, 1, 8, c);
    fill_rect(s, x + 9, y + 5, 1, 6, c);
    fill_rect(s, x + 10, y + 5, 1, 6, c);
    fill_rect(s, x + 11, y + 6, 1, 4, c);
    fill_rect(s, x + 12, y + 7, 1, 2, c);
}

// ── Stop square ─────────────────────────────────────────────────────

fn draw_stop(s: &Surface, x: i32, y: i32, c: u32) {
    fill_rect(s, x + 3, y + 3, 10, 10, c);
}

// ── Settings gear ───────────────────────────────────────────────────

fn draw_settings(s: &Surface, x: i32, y: i32, c: u32) {
    // Center body
    fill_rect(s, x + 5, y + 5, 6, 6, c);
    // Center hole
    fill_rect(s, x + 7, y + 7, 2, 2, dim(c));
    // Top tooth
    fill_rect(s, x + 6, y + 1, 4, 3, c);
    // Bottom tooth
    fill_rect(s, x + 6, y + 12, 4, 3, c);
    // Left tooth
    fill_rect(s, x + 1, y + 6, 3, 4, c);
    // Right tooth
    fill_rect(s, x + 12, y + 6, 3, 4, c);
    // Top-left diagonal tooth
    fill_rect(s, x + 3, y + 3, 2, 2, c);
    // Top-right diagonal tooth
    fill_rect(s, x + 11, y + 3, 2, 2, c);
    // Bottom-left diagonal tooth
    fill_rect(s, x + 3, y + 11, 2, 2, c);
    // Bottom-right diagonal tooth
    fill_rect(s, x + 11, y + 11, 2, 2, c);
}

// ── Files (explorer icon) ───────────────────────────────────────────

fn draw_files(s: &Surface, x: i32, y: i32, c: u32) {
    // Back document (darker, offset right+up)
    fill_rect(s, x + 5, y + 0, 8, 12, dim(c));
    // Front document (brighter, offset left+down)
    fill_rect(s, x + 2, y + 3, 8, 12, c);
    // Text lines on front document
    fill_rect(s, x + 4, y + 6, 4, 1, dim(c));
    fill_rect(s, x + 4, y + 9, 4, 1, dim(c));
    fill_rect(s, x + 4, y + 12, 3, 1, dim(c));
}

// ── Git branch icon ─────────────────────────────────────────────────

fn draw_git_branch(s: &Surface, x: i32, y: i32, c: u32) {
    // Main trunk (vertical line)
    fill_rect(s, x + 5, y + 3, 2, 10, c);
    // Top node (circle)
    fill_rect(s, x + 4, y + 1, 4, 1, c);
    fill_rect(s, x + 4, y + 2, 4, 2, c);
    fill_rect(s, x + 4, y + 4, 4, 1, c);
    // Bottom node (circle)
    fill_rect(s, x + 4, y + 11, 4, 1, c);
    fill_rect(s, x + 4, y + 12, 4, 2, c);
    fill_rect(s, x + 4, y + 14, 4, 1, c);
    // Branch node (right, circle)
    fill_rect(s, x + 10, y + 3, 3, 1, c);
    fill_rect(s, x + 10, y + 4, 3, 2, c);
    fill_rect(s, x + 10, y + 6, 3, 1, c);
    // Branch connector
    fill_rect(s, x + 7, y + 5, 3, 2, c);
}

// ── Search (magnifying glass) ───────────────────────────────────────

fn draw_search(s: &Surface, x: i32, y: i32, c: u32) {
    // Circle top edge
    fill_rect(s, x + 4, y + 1, 5, 1, c);
    // Circle top-left / top-right corners
    fill_rect(s, x + 3, y + 2, 1, 1, c);
    fill_rect(s, x + 9, y + 2, 1, 1, c);
    // Circle left / right sides
    fill_rect(s, x + 2, y + 3, 1, 4, c);
    fill_rect(s, x + 10, y + 3, 1, 4, c);
    // Circle bottom-left / bottom-right corners
    fill_rect(s, x + 3, y + 7, 1, 1, c);
    fill_rect(s, x + 9, y + 7, 1, 1, c);
    // Circle bottom edge
    fill_rect(s, x + 4, y + 8, 5, 1, c);
    // Glass interior fill (subtle)
    fill_rect(s, x + 3, y + 3, 7, 4, alpha_blend(c, 40));
    fill_rect(s, x + 4, y + 2, 5, 1, alpha_blend(c, 40));
    fill_rect(s, x + 4, y + 7, 5, 1, alpha_blend(c, 40));
    // Handle (diagonal)
    fill_rect(s, x + 9, y + 8, 2, 1, c);
    fill_rect(s, x + 10, y + 9, 2, 1, c);
    fill_rect(s, x + 11, y + 10, 2, 1, c);
    fill_rect(s, x + 12, y + 11, 2, 2, c);
}

// ── Color helpers ───────────────────────────────────────────────────

/// Darken a color by reducing RGB by 50.
fn dim(color: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = ((color >> 16) & 0xFF).saturating_sub(50);
    let g = ((color >> 8) & 0xFF).saturating_sub(50);
    let b = (color & 0xFF).saturating_sub(50);
    a | (r << 16) | (g << 8) | b
}

/// Lighten a color by adding 35 to each RGB channel.
fn lighten(color: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = (((color >> 16) & 0xFF) + 35).min(255);
    let g = (((color >> 8) & 0xFF) + 35).min(255);
    let b = ((color & 0xFF) + 35).min(255);
    a | (r << 16) | (g << 8) | b
}

/// Create a semi-transparent version of a color for glass fill.
fn alpha_blend(color: u32, alpha: u32) -> u32 {
    let r = (color >> 16) & 0xFF;
    let g = (color >> 8) & 0xFF;
    let b = color & 0xFF;
    (alpha << 24) | (r << 16) | (g << 8) | b
}
