//! Simple pixel-art icon rendering using fill_rect primitives.
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

// New file: document with folded corner and "+"
fn draw_new_file(s: &Surface, x: i32, y: i32, c: u32) {
    // Document body
    fill_rect(s, x + 2, y + 0, 9, 16, c);
    // Fold corner cut (top-right)
    fill_rect(s, x + 8, y + 0, 3, 4, 0x00000000); // transparent cut
    // Fold triangle
    fill_rect(s, x + 8, y + 3, 3, 1, c);
    fill_rect(s, x + 10, y + 1, 1, 3, c);
    // "+" sign in the middle
    fill_rect(s, x + 5, y + 7, 5, 1, darken(c));
    fill_rect(s, x + 7, y + 5, 1, 5, darken(c));
}

// Folder open
fn draw_folder_open(s: &Surface, x: i32, y: i32, c: u32) {
    // Folder tab
    fill_rect(s, x + 1, y + 2, 5, 2, c);
    // Folder body
    fill_rect(s, x + 1, y + 4, 13, 9, c);
    // Folder front (open flap)
    fill_rect(s, x + 0, y + 6, 14, 7, lighten(c));
    // Inner dark area
    fill_rect(s, x + 2, y + 7, 10, 4, darken(c));
}

// Save (floppy disk)
fn draw_save(s: &Surface, x: i32, y: i32, c: u32) {
    // Disk body
    fill_rect(s, x + 1, y + 1, 14, 14, c);
    // Label area (bottom center)
    fill_rect(s, x + 3, y + 8, 10, 6, lighten(c));
    // Metal slider (top)
    fill_rect(s, x + 4, y + 1, 8, 5, darken(c));
    fill_rect(s, x + 8, y + 2, 2, 3, c);
}

// Save all (two stacked floppy disks)
fn draw_save_all(s: &Surface, x: i32, y: i32, c: u32) {
    // Back disk (offset)
    fill_rect(s, x + 3, y + 0, 12, 12, darken(c));
    // Front disk
    fill_rect(s, x + 1, y + 3, 12, 12, c);
    // Label area
    fill_rect(s, x + 3, y + 9, 8, 5, lighten(c));
    // Metal slider
    fill_rect(s, x + 4, y + 3, 6, 4, darken(c));
    fill_rect(s, x + 7, y + 4, 2, 2, c);
}

// Build (hammer/wrench)
fn draw_build(s: &Surface, x: i32, y: i32, c: u32) {
    // Wrench handle (diagonal, simplified as rectangles)
    fill_rect(s, x + 2, y + 10, 6, 3, c);
    fill_rect(s, x + 4, y + 8, 3, 2, c);
    fill_rect(s, x + 6, y + 6, 3, 2, c);
    fill_rect(s, x + 8, y + 4, 3, 2, c);
    // Wrench head
    fill_rect(s, x + 9, y + 1, 5, 5, c);
    fill_rect(s, x + 11, y + 2, 1, 3, darken(c));
}

// Play triangle
fn draw_play(s: &Surface, x: i32, y: i32, c: u32) {
    // Play triangle approximated with rectangles
    fill_rect(s, x + 4, y + 2, 2, 12, c);
    fill_rect(s, x + 6, y + 3, 2, 10, c);
    fill_rect(s, x + 8, y + 4, 2, 8, c);
    fill_rect(s, x + 10, y + 5, 2, 6, c);
    fill_rect(s, x + 12, y + 6, 1, 4, c);
}

// Stop square
fn draw_stop(s: &Surface, x: i32, y: i32, c: u32) {
    fill_rect(s, x + 3, y + 3, 10, 10, c);
}

// Settings gear (simplified)
fn draw_settings(s: &Surface, x: i32, y: i32, c: u32) {
    // Center circle
    fill_rect(s, x + 5, y + 5, 6, 6, c);
    // Center hole
    fill_rect(s, x + 7, y + 7, 2, 2, darken(c));
    // Gear teeth (cross pattern)
    fill_rect(s, x + 6, y + 1, 4, 3, c); // top
    fill_rect(s, x + 6, y + 12, 4, 3, c); // bottom
    fill_rect(s, x + 1, y + 6, 3, 4, c); // left
    fill_rect(s, x + 12, y + 6, 3, 4, c); // right
    // Diagonal teeth
    fill_rect(s, x + 2, y + 3, 3, 2, c);
    fill_rect(s, x + 11, y + 3, 3, 2, c);
    fill_rect(s, x + 2, y + 11, 3, 2, c);
    fill_rect(s, x + 11, y + 11, 3, 2, c);
}

// Files (explorer icon - two documents)
fn draw_files(s: &Surface, x: i32, y: i32, c: u32) {
    // Back document
    fill_rect(s, x + 4, y + 0, 9, 13, darken(c));
    // Front document
    fill_rect(s, x + 2, y + 3, 9, 13, c);
    // Lines on front document
    fill_rect(s, x + 4, y + 6, 5, 1, darken(c));
    fill_rect(s, x + 4, y + 9, 5, 1, darken(c));
    fill_rect(s, x + 4, y + 12, 3, 1, darken(c));
}

// Git branch icon
fn draw_git_branch(s: &Surface, x: i32, y: i32, c: u32) {
    // Vertical line (trunk)
    fill_rect(s, x + 5, y + 2, 2, 12, c);
    // Branch dot top-right
    fill_rect(s, x + 10, y + 3, 3, 3, c);
    // Branch line from dot to trunk
    fill_rect(s, x + 7, y + 4, 3, 2, c);
    // Trunk dot bottom
    fill_rect(s, x + 4, y + 12, 4, 3, c);
    // Trunk dot top
    fill_rect(s, x + 4, y + 1, 4, 3, c);
}

// Search (magnifying glass)
fn draw_search(s: &Surface, x: i32, y: i32, c: u32) {
    // Glass circle (as a square ring approximation)
    fill_rect(s, x + 3, y + 1, 8, 2, c); // top
    fill_rect(s, x + 3, y + 9, 8, 2, c); // bottom
    fill_rect(s, x + 1, y + 3, 2, 6, c); // left
    fill_rect(s, x + 11, y + 3, 2, 6, c); // right
    fill_rect(s, x + 3, y + 3, 2, 2, c); // top-left corner
    fill_rect(s, x + 9, y + 3, 2, 2, c); // top-right corner
    fill_rect(s, x + 3, y + 7, 2, 2, c); // bottom-left corner
    fill_rect(s, x + 9, y + 7, 2, 2, c); // bottom-right corner
    // Handle
    fill_rect(s, x + 11, y + 10, 2, 2, c);
    fill_rect(s, x + 12, y + 11, 2, 2, c);
    fill_rect(s, x + 13, y + 12, 2, 2, c);
}

// ── Color helpers ───────────────────────────────────────────────────

fn darken(color: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = ((color >> 16) & 0xFF).saturating_sub(40);
    let g = ((color >> 8) & 0xFF).saturating_sub(40);
    let b = (color & 0xFF).saturating_sub(40);
    a | (r << 16) | (g << 8) | b
}

fn lighten(color: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = (((color >> 16) & 0xFF) + 30).min(255);
    let g = (((color >> 8) & 0xFF) + 30).min(255);
    let b = ((color & 0xFF) + 30).min(255);
    a | (r << 16) | (g << 8) | b
}
