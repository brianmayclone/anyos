//! MenuBar — macOS-like global menu bar with per-app menus, dropdowns, and status icons.

use alloc::string::String;
use alloc::vec::Vec;

use crate::compositor::{alpha_blend, Compositor, Rect};

// ── Theme Constants ──────────────────────────────────────────────────────────

const FONT_ID: u16 = 0;
const FONT_SIZE: u16 = 13;

pub const MENUBAR_HEIGHT: u32 = 24;

const COLOR_MENUBAR_TEXT: u32 = 0xFFE0E0E0;
const COLOR_MENUBAR_HIGHLIGHT: u32 = 0x40FFFFFF;
const COLOR_DROPDOWN_BG: u32 = 0xF0303035;
const COLOR_DROPDOWN_BORDER: u32 = 0xFF505055;
const COLOR_HOVER_BG: u32 = 0xFF0058D0;
const COLOR_SEPARATOR: u32 = 0xFF505055;
const COLOR_DISABLED_TEXT: u32 = 0xFF707075;
const COLOR_CHECK: u32 = 0xFF0A84FF;

const ITEM_HEIGHT: u32 = 24;
const SEPARATOR_HEIGHT: u32 = 9;
const DROPDOWN_PADDING: i32 = 4;
const MENU_TITLE_START_X: i32 = 60; // after "anyOS" text + gap

// ── Menu Item Flags ──────────────────────────────────────────────────────────

pub const MENU_FLAG_DISABLED: u32 = 0x01;
pub const MENU_FLAG_SEPARATOR: u32 = 0x02;
pub const MENU_FLAG_CHECKED: u32 = 0x04;

// ── Data Structures ──────────────────────────────────────────────────────────

pub struct MenuItem {
    pub item_id: u32,
    pub flags: u32,
    pub label: String,
}

impl MenuItem {
    pub fn is_disabled(&self) -> bool {
        self.flags & MENU_FLAG_DISABLED != 0
    }
    pub fn is_separator(&self) -> bool {
        self.flags & MENU_FLAG_SEPARATOR != 0
    }
    pub fn is_checked(&self) -> bool {
        self.flags & MENU_FLAG_CHECKED != 0
    }
}

pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
}

pub struct MenuBarDef {
    pub menus: Vec<Menu>,
}

struct MenuTitleLayout {
    x: i32,
    width: u32,
    menu_idx: usize,
}

pub struct OpenDropdown {
    pub menu_idx: usize,
    pub owner_window_id: u32,
    pub layer_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hover_idx: Option<usize>,
    items_y: Vec<i32>,
}

pub struct StatusIcon {
    pub owner_tid: u32,
    pub icon_id: u32,
    pub pixels: [u32; 256],
}

pub enum MenuBarHit {
    None,
    MenuTitle { menu_idx: usize },
    StatusIcon { owner_tid: u32, icon_id: u32 },
}

// ── SHM Binary Format Parsing ────────────────────────────────────────────────

const MENU_MAGIC: u32 = 0x4D454E55; // 'MENU'

impl MenuBarDef {
    /// Parse a menu bar definition from raw SHM bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let magic = read_u32(data, 0);
        if magic != MENU_MAGIC {
            return None;
        }

        let num_menus = read_u32(data, 4) as usize;
        if num_menus > 16 {
            return None;
        }

        let mut offset = 8usize;
        let mut menus = Vec::with_capacity(num_menus);

        for _ in 0..num_menus {
            if offset + 4 > data.len() {
                return None;
            }
            let title_len = read_u32(data, offset) as usize;
            offset += 4;
            if offset + title_len > data.len() || title_len > 64 {
                return None;
            }
            let title = core::str::from_utf8(&data[offset..offset + title_len]).ok()?;
            offset += title_len;
            offset = align4(offset);

            if offset + 4 > data.len() {
                return None;
            }
            let num_items = read_u32(data, offset) as usize;
            offset += 4;
            if num_items > 32 {
                return None;
            }

            let mut items = Vec::with_capacity(num_items);
            for _ in 0..num_items {
                if offset + 12 > data.len() {
                    return None;
                }
                let item_id = read_u32(data, offset);
                let flags = read_u32(data, offset + 4);
                let label_len = read_u32(data, offset + 8) as usize;
                offset += 12;
                if offset + label_len > data.len() || label_len > 64 {
                    return None;
                }
                let label = core::str::from_utf8(&data[offset..offset + label_len]).ok()?;
                offset += label_len;
                offset = align4(offset);

                items.push(MenuItem {
                    item_id,
                    flags,
                    label: String::from(label),
                });
            }

            menus.push(Menu {
                title: String::from(title),
                items,
            });
        }

        Some(MenuBarDef { menus })
    }
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

// ── MenuBar ──────────────────────────────────────────────────────────────────

pub struct MenuBar {
    /// Per-window menu definitions: (window_id, MenuBarDef)
    window_menus: Vec<(u32, MenuBarDef)>,

    /// Layout of current menu titles (recomputed on focus change)
    title_layouts: Vec<MenuTitleLayout>,

    /// Currently open dropdown (if any)
    pub open_dropdown: Option<OpenDropdown>,

    /// Registered status icons (right side of menubar, left of clock)
    status_icons: Vec<StatusIcon>,

    /// Layout positions of status icons (computed when icons change)
    status_icon_x: Vec<i32>,

    /// The window_id whose menus are currently displayed
    active_window_id: Option<u32>,
}

impl MenuBar {
    pub fn new() -> Self {
        MenuBar {
            window_menus: Vec::new(),
            title_layouts: Vec::new(),
            open_dropdown: None,
            status_icons: Vec::with_capacity(8),
            status_icon_x: Vec::new(),
            active_window_id: None,
        }
    }

    // ── Menu Registration ────────────────────────────────────────────────

    /// Register/update menu bar for a window.
    pub fn set_menu(&mut self, window_id: u32, def: MenuBarDef) {
        if let Some(entry) = self.window_menus.iter_mut().find(|(id, _)| *id == window_id) {
            entry.1 = def;
        } else {
            self.window_menus.push((window_id, def));
        }
        if self.active_window_id == Some(window_id) {
            self.recompute_title_layout();
        }
    }

    /// Remove all menus associated with a window.
    pub fn remove_menu(&mut self, window_id: u32) {
        self.window_menus.retain(|(id, _)| *id != window_id);
        if self.active_window_id == Some(window_id) {
            self.active_window_id = None;
            self.title_layouts.clear();
        }
    }

    /// Update the flags for a specific item_id in a window's menu.
    /// Returns true if the item was found and flags changed.
    pub fn update_item_flags(&mut self, window_id: u32, item_id: u32, new_flags: u32) -> bool {
        if let Some(entry) = self.window_menus.iter_mut().find(|(id, _)| *id == window_id) {
            for menu in &mut entry.1.menus {
                if let Some(item) = menu.items.iter_mut().find(|i| i.item_id == item_id) {
                    if item.flags != new_flags {
                        item.flags = new_flags;
                        return true;
                    }
                    return false;
                }
            }
        }
        false
    }

    /// Called when focused window changes. Returns true if menubar needs redraw.
    pub fn on_focus_change(&mut self, window_id: Option<u32>) -> bool {
        if self.active_window_id == window_id {
            return false;
        }
        self.active_window_id = window_id;
        self.recompute_title_layout();
        true
    }

    /// Get the active MenuBarDef (for the focused window), if any.
    fn active_def(&self) -> Option<&MenuBarDef> {
        let wid = self.active_window_id?;
        self.window_menus
            .iter()
            .find(|(id, _)| *id == wid)
            .map(|(_, def)| def)
    }

    fn recompute_title_layout(&mut self) {
        self.title_layouts.clear();
        let wid = match self.active_window_id {
            Some(w) => w,
            None => return,
        };
        let def = match self.window_menus.iter().find(|(id, _)| *id == wid) {
            Some((_, d)) => d,
            None => return,
        };

        let mut x: i32 = MENU_TITLE_START_X;

        for (idx, menu) in def.menus.iter().enumerate() {
            let (tw, _) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &menu.title);
            let padding: u32 = 16;
            let total_w = tw + padding;
            self.title_layouts.push(MenuTitleLayout {
                x,
                width: total_w,
                menu_idx: idx,
            });
            x += total_w as i32;
        }
    }

    // ── Rendering ────────────────────────────────────────────────────────

    /// Render menu titles into the menubar pixel buffer.
    pub fn render_titles(&self, pixels: &mut [u32], stride: u32, height: u32) {
        let def = match self.active_def() {
            Some(d) => d,
            None => return,
        };

        let open_idx = self.open_dropdown.as_ref().map(|d| d.menu_idx);

        for layout in &self.title_layouts {
            let menu = &def.menus[layout.menu_idx];

            // Highlight background if this menu is open
            if open_idx == Some(layout.menu_idx) {
                for y in 0..MENUBAR_HEIGHT {
                    for x in layout.x.max(0)..(layout.x + layout.width as i32).min(stride as i32) {
                        let idx = (y * stride + x as u32) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = alpha_blend(COLOR_MENUBAR_HIGHLIGHT, pixels[idx]);
                        }
                    }
                }
            }

            // Render title text centered in its region
            let (tw, th) =
                anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &menu.title);
            let tx = layout.x + (layout.width as i32 - tw as i32) / 2;
            let ty = ((MENUBAR_HEIGHT as i32 - th as i32) / 2).max(0);
            anyos_std::ui::window::font_render_buf(
                FONT_ID,
                FONT_SIZE,
                pixels,
                stride,
                height,
                tx,
                ty,
                COLOR_MENUBAR_TEXT,
                &menu.title,
            );
        }
    }

    /// Render status icons into the menubar pixel buffer.
    pub fn render_status_icons(&self, pixels: &mut [u32], stride: u32) {
        for (i, icon) in self.status_icons.iter().enumerate() {
            let ix = match self.status_icon_x.get(i) {
                Some(&x) => x,
                None => continue,
            };
            let iy = ((MENUBAR_HEIGHT as i32 - 16) / 2).max(0);
            for row in 0..16i32 {
                let py = iy + row;
                if py < 0 || py >= MENUBAR_HEIGHT as i32 {
                    continue;
                }
                for col in 0..16i32 {
                    let px = ix + col;
                    if px < 0 || px >= stride as i32 {
                        continue;
                    }
                    let src = icon.pixels[(row * 16 + col) as usize];
                    let a = (src >> 24) & 0xFF;
                    if a == 0 {
                        continue;
                    }
                    let didx = (py as u32 * stride + px as u32) as usize;
                    if didx < pixels.len() {
                        if a >= 255 {
                            pixels[didx] = src;
                        } else {
                            pixels[didx] = alpha_blend(src, pixels[didx]);
                        }
                    }
                }
            }
        }
    }

    /// Render the dropdown layer content.
    pub fn render_dropdown(&self, compositor: &mut Compositor) {
        let dd = match &self.open_dropdown {
            Some(d) => d,
            None => return,
        };
        let def = match self.active_def() {
            Some(d) => d,
            None => return,
        };
        let menu = match def.menus.get(dd.menu_idx) {
            Some(m) => m,
            None => return,
        };

        if let Some(pixels) = compositor.layer_pixels(dd.layer_id) {
            let w = dd.width;
            let h = dd.height;

            // Clear to transparent
            for p in pixels.iter_mut() {
                *p = 0x00000000;
            }

            // Fill background with rounded corners
            fill_rounded_rect(pixels, w, h, 0, 0, w, h, 6, COLOR_DROPDOWN_BG);

            // 1px border
            draw_rect_outline(pixels, w, 0, 0, w, h, COLOR_DROPDOWN_BORDER);

            // Render each item
            for (i, item) in menu.items.iter().enumerate() {
                let iy = dd.items_y[i];

                if item.is_separator() {
                    let line_y = iy + SEPARATOR_HEIGHT as i32 / 2;
                    if line_y >= 0 && (line_y as u32) < h {
                        for x in 8i32..(w as i32 - 8) {
                            if x >= 0 && (x as u32) < w {
                                let idx = (line_y as u32 * w + x as u32) as usize;
                                if idx < pixels.len() {
                                    pixels[idx] = COLOR_SEPARATOR;
                                }
                            }
                        }
                    }
                    continue;
                }

                // Highlight hovered item
                if dd.hover_idx == Some(i) && !item.is_disabled() {
                    fill_rect_in(pixels, w, h, 4, iy, w - 8, ITEM_HEIGHT, COLOR_HOVER_BG);
                }

                // Checkmark if checked
                if item.is_checked() {
                    let check_x = 8i32;
                    let (_, ch) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, "✓");
                    let check_y = iy + ((ITEM_HEIGHT as i32 - ch as i32) / 2).max(0);
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID,
                        FONT_SIZE,
                        pixels,
                        w,
                        h,
                        check_x,
                        check_y,
                        if item.is_disabled() {
                            COLOR_DISABLED_TEXT
                        } else {
                            COLOR_CHECK
                        },
                        "✓",
                    );
                }

                // Item label
                let text_x = 28i32;
                let (_, th) =
                    anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                let text_y = iy + ((ITEM_HEIGHT as i32 - th as i32) / 2).max(0);
                let text_color = if item.is_disabled() {
                    COLOR_DISABLED_TEXT
                } else if dd.hover_idx == Some(i) {
                    0xFFFFFFFF
                } else {
                    COLOR_MENUBAR_TEXT
                };
                anyos_std::ui::window::font_render_buf(
                    FONT_ID,
                    FONT_SIZE,
                    pixels,
                    w,
                    h,
                    text_x,
                    text_y,
                    text_color,
                    &item.label,
                );
            }
        }

        compositor.mark_layer_dirty(dd.layer_id);
        let bounds = Rect::new(dd.x, dd.y, dd.width, dd.height);
        compositor.add_damage(bounds);
    }

    // ── Hit Testing ──────────────────────────────────────────────────────

    /// Hit test a click in the menubar region.
    pub fn hit_test_menubar(&self, mx: i32, my: i32) -> MenuBarHit {
        if my < 0 || my >= MENUBAR_HEIGHT as i32 {
            return MenuBarHit::None;
        }

        // Check status icons first
        for (i, icon) in self.status_icons.iter().enumerate() {
            if let Some(&ix) = self.status_icon_x.get(i) {
                let iy = ((MENUBAR_HEIGHT as i32 - 16) / 2).max(0);
                if mx >= ix && mx < ix + 16 && my >= iy && my < iy + 16 {
                    return MenuBarHit::StatusIcon {
                        owner_tid: icon.owner_tid,
                        icon_id: icon.icon_id,
                    };
                }
            }
        }

        // Check menu titles
        for layout in &self.title_layouts {
            if mx >= layout.x && mx < layout.x + layout.width as i32 {
                return MenuBarHit::MenuTitle {
                    menu_idx: layout.menu_idx,
                };
            }
        }

        MenuBarHit::None
    }

    /// Hit test within the open dropdown. Returns Some(item_id) if a clickable item was hit.
    pub fn hit_test_dropdown(&self, mx: i32, my: i32) -> Option<u32> {
        let dd = self.open_dropdown.as_ref()?;
        let def = self.active_def()?;
        let menu = def.menus.get(dd.menu_idx)?;

        if mx < dd.x || mx >= dd.x + dd.width as i32 {
            return None;
        }
        if my < dd.y || my >= dd.y + dd.height as i32 {
            return None;
        }

        let local_y = my - dd.y;
        for (i, item) in menu.items.iter().enumerate() {
            let item_y = dd.items_y.get(i).copied().unwrap_or(0);
            let item_h = if item.is_separator() {
                SEPARATOR_HEIGHT
            } else {
                ITEM_HEIGHT
            };
            if local_y >= item_y && local_y < item_y + item_h as i32 {
                if !item.is_disabled() && !item.is_separator() {
                    return Some(item.item_id);
                }
                return None;
            }
        }

        None
    }

    /// Check if a point is within the open dropdown bounds.
    pub fn is_in_dropdown(&self, mx: i32, my: i32) -> bool {
        if let Some(ref dd) = self.open_dropdown {
            mx >= dd.x
                && mx < dd.x + dd.width as i32
                && my >= dd.y
                && my < dd.y + dd.height as i32
        } else {
            false
        }
    }

    // ── Dropdown Management ──────────────────────────────────────────────

    /// Open a dropdown for the given menu index.
    pub fn open_menu(
        &mut self,
        menu_idx: usize,
        owner_window_id: u32,
        compositor: &mut Compositor,
    ) -> Option<u32> {
        // Close any existing dropdown first
        self.close_dropdown_with_compositor(compositor);

        let def = self.active_def()?;
        let menu = def.menus.get(menu_idx)?;
        let layout = self.title_layouts.iter().find(|l| l.menu_idx == menu_idx)?;

        // Compute dropdown dimensions
        let mut max_w: u32 = 0;
        let mut total_h: i32 = DROPDOWN_PADDING;
        let mut items_y = Vec::with_capacity(menu.items.len());

        for item in &menu.items {
            items_y.push(total_h);
            if item.is_separator() {
                total_h += SEPARATOR_HEIGHT as i32;
            } else {
                let (tw, _) =
                    anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                max_w = max_w.max(tw + 40); // label + padding + checkmark space
                total_h += ITEM_HEIGHT as i32;
            }
        }
        total_h += DROPDOWN_PADDING;
        let dd_width = max_w.max(120) + DROPDOWN_PADDING as u32 * 2;
        let dd_height = total_h as u32;

        let dd_x = layout.x;
        let dd_y = MENUBAR_HEIGHT as i32 + 1;

        // Create compositor layer (always on top = false, will be raised)
        let layer_id = compositor.add_layer(dd_x, dd_y, dd_width, dd_height, false);
        compositor.raise_layer(layer_id);

        self.open_dropdown = Some(OpenDropdown {
            menu_idx,
            owner_window_id,
            layer_id,
            x: dd_x,
            y: dd_y,
            width: dd_width,
            height: dd_height,
            hover_idx: None,
            items_y,
        });

        // Render the dropdown content
        self.render_dropdown(compositor);

        Some(layer_id)
    }

    /// Update hover state for mouse move. Returns true if dropdown needs redraw.
    pub fn update_hover(&mut self, mx: i32, my: i32) -> bool {
        let dd = match &mut self.open_dropdown {
            Some(d) => d,
            None => return false,
        };

        if mx < dd.x
            || mx >= dd.x + dd.width as i32
            || my < dd.y
            || my >= dd.y + dd.height as i32
        {
            if dd.hover_idx.is_some() {
                dd.hover_idx = None;
                return true;
            }
            return false;
        }

        let local_y = my - dd.y;
        let mut new_hover = None;

        for (i, &item_y) in dd.items_y.iter().enumerate() {
            if local_y >= item_y && local_y < item_y + ITEM_HEIGHT as i32 {
                new_hover = Some(i);
                break;
            }
        }

        if new_hover != dd.hover_idx {
            dd.hover_idx = new_hover;
            true
        } else {
            false
        }
    }

    /// Close the dropdown and remove its compositor layer.
    pub fn close_dropdown_with_compositor(&mut self, compositor: &mut Compositor) {
        if let Some(dd) = self.open_dropdown.take() {
            let bounds = Rect::new(dd.x, dd.y, dd.width, dd.height);
            compositor.remove_layer(dd.layer_id);
            compositor.add_damage(bounds);
        }
    }

    pub fn is_dropdown_open(&self) -> bool {
        self.open_dropdown.is_some()
    }

    // ── Status Icons ─────────────────────────────────────────────────────

    /// Add a status icon. Returns true if added.
    pub fn add_status_icon(
        &mut self,
        owner_tid: u32,
        icon_id: u32,
        pixel_data: &[u32],
        screen_width: u32,
    ) -> bool {
        if pixel_data.len() < 256 {
            return false;
        }
        if self.status_icons.len() >= 8 {
            return false;
        }
        // No duplicates
        if self
            .status_icons
            .iter()
            .any(|i| i.owner_tid == owner_tid && i.icon_id == icon_id)
        {
            return false;
        }

        let mut pixels = [0u32; 256];
        pixels.copy_from_slice(&pixel_data[..256]);

        self.status_icons.push(StatusIcon {
            owner_tid,
            icon_id,
            pixels,
        });
        self.recompute_status_positions(screen_width);
        true
    }

    /// Remove a status icon. Returns true if removed.
    pub fn remove_status_icon(
        &mut self,
        owner_tid: u32,
        icon_id: u32,
        screen_width: u32,
    ) -> bool {
        let before = self.status_icons.len();
        self.status_icons
            .retain(|i| !(i.owner_tid == owner_tid && i.icon_id == icon_id));
        if self.status_icons.len() != before {
            self.recompute_status_positions(screen_width);
            true
        } else {
            false
        }
    }

    fn recompute_status_positions(&mut self, screen_width: u32) {
        self.status_icon_x.clear();
        let clock_region = 60i32;
        let icon_spacing = 4i32;
        let icon_size = 16i32;
        let mut x = screen_width as i32 - clock_region - icon_spacing;

        // Layout right-to-left
        for _ in self.status_icons.iter().rev() {
            x -= icon_size;
            self.status_icon_x.push(x);
            x -= icon_spacing;
        }
        self.status_icon_x.reverse();
    }
}

// ── Drawing Helpers ──────────────────────────────────────────────────────────

fn fill_rounded_rect(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if r == 0 || w < r * 2 || h < r * 2 {
        fill_rect_in(pixels, stride, buf_h, x, y, w, h, color);
        return;
    }
    // Center body
    if h > r * 2 {
        fill_rect_in(pixels, stride, buf_h, x, y + r as i32, w, h - r * 2, color);
    }
    // Top and bottom bands
    if w > r * 2 {
        fill_rect_in(pixels, stride, buf_h, x + r as i32, y, w - r * 2, r, color);
        fill_rect_in(
            pixels,
            stride,
            buf_h,
            x + r as i32,
            y + h as i32 - r as i32,
            w - r * 2,
            r,
            color,
        );
    }
    // Corners
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            // Top-left
            fill_rect_in(pixels, stride, buf_h, x + fs, y + dy as i32, fill_width, 1, color);
            // Top-right
            fill_rect_in(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + dy as i32,
                fill_width,
                1,
                color,
            );
            // Bottom-left
            fill_rect_in(
                pixels,
                stride,
                buf_h,
                x + fs,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
            // Bottom-right
            fill_rect_in(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
        }
    }
}

fn fill_rect_in(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    let a = (color >> 24) & 0xFF;
    for row in 0..h as i32 {
        let py = y + row;
        if py < 0 || py >= buf_h as i32 {
            continue;
        }
        for col in 0..w as i32 {
            let px = x + col;
            if px < 0 || px >= stride as i32 {
                continue;
            }
            let idx = (py as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                if a >= 255 {
                    pixels[idx] = color;
                } else if a > 0 {
                    pixels[idx] = alpha_blend(color, pixels[idx]);
                }
            }
        }
    }
}

fn draw_rect_outline(
    pixels: &mut [u32],
    stride: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    // Top
    for col in 0..w as i32 {
        let px = x + col;
        if px >= 0 && (px as u32) < stride && y >= 0 {
            let idx = (y as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Bottom
    let by = y + h as i32 - 1;
    for col in 0..w as i32 {
        let px = x + col;
        if px >= 0 && (px as u32) < stride && by >= 0 {
            let idx = (by as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Left
    for row in 0..h as i32 {
        let py = y + row;
        if x >= 0 && py >= 0 {
            let idx = (py as u32 * stride + x as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Right
    let rx = x + w as i32 - 1;
    for row in 0..h as i32 {
        let py = y + row;
        if rx >= 0 && (rx as u32) < stride && py >= 0 {
            let idx = (py as u32 * stride + rx as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
}
