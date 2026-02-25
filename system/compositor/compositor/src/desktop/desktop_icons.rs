//! Desktop drive icons — macOS-style mounted volume icons on the desktop.
//!
//! Renders device icons directly onto the background layer (wallpaper).
//! Supports drag, double-click to open Finder, right-click context menu,
//! and persistent icon positions.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::compositor::{alpha_blend, Compositor, Rect};
use crate::desktop::drawing::{fill_rect, fill_rounded_rect, draw_rounded_rect_outline};
use crate::menu::rendering::draw_rect_outline;

// ── Constants ──────────────────────────────────────────────────────────────

/// Preferred icon size when loading ICO files.
const ICON_SIZE: u32 = 48;
/// Total width of one icon cell (icon + padding for label).
const CELL_WIDTH: u32 = 90;
/// Total height of one icon cell (icon + gap + label + shadow).
const CELL_HEIGHT: u32 = 90;
/// Right margin from screen edge for default placement.
const MARGIN_RIGHT: i32 = 20;
/// Top offset below menubar for first icon.
const MARGIN_TOP: i32 = 60;
/// Vertical spacing between icon cells.
const CELL_SPACING: i32 = 95;
/// Font ID for labels (system font).
const LABEL_FONT_ID: u16 = 0;
/// Font size for icon labels.
const LABEL_FONT_SIZE: u16 = 11;
/// How often to poll mounts (milliseconds).
const MOUNT_POLL_INTERVAL_MS: u32 = 3000;

// ── Context Menu Constants ─────────────────────────────────────────────────

const CTX_ITEM_OPEN: u32 = 1;
const CTX_ITEM_EJECT: u32 = 2;
const CTX_ITEM_INFO: u32 = 3;

const CTX_ITEM_HEIGHT: u32 = 24;
const CTX_SEPARATOR_HEIGHT: u32 = 9;
const CTX_PADDING: i32 = 4;
const CTX_COLOR_BG: u32 = 0xF0303035;
const CTX_COLOR_BORDER: u32 = 0xFF505055;
const CTX_COLOR_HOVER: u32 = 0xFF0058D0;
const CTX_COLOR_TEXT: u32 = 0xFFE0E0E0;
const CTX_COLOR_DISABLED: u32 = 0xFF707075;
const CTX_COLOR_SEPARATOR: u32 = 0xFF505055;

/// Selection highlight color (macOS-like blue, semi-transparent)
const SELECTION_BG: u32 = 0x50307AD8;
const SELECTION_BORDER: u32 = 0xA0307AD8;
const SELECTION_RADIUS: u32 = 6;

// ── Desktop Icon ───────────────────────────────────────────────────────────

pub struct DesktopIcon {
    /// Mount path, e.g. "/", "/mnt/usb0"
    pub mount_path: [u8; 64],
    pub mount_path_len: usize,
    /// Display label
    pub label: [u8; 32],
    pub label_len: usize,
    /// Filesystem type string, e.g. "exfat", "fat", "iso9660"
    pub fs_type: [u8; 16],
    pub fs_type_len: usize,
    /// Desktop position (top-left of the icon cell)
    pub x: i32,
    pub y: i32,
    /// Decoded icon pixels (ARGB)
    pub icon_pixels: Vec<u32>,
    pub icon_w: u32,
    pub icon_h: u32,
    /// Whether this drive can be ejected (USB, CD-ROM, network)
    pub is_ejectable: bool,
}

impl DesktopIcon {
    pub(crate) fn mount_path_str(&self) -> &str {
        core::str::from_utf8(&self.mount_path[..self.mount_path_len]).unwrap_or("")
    }

    fn label_str(&self) -> &str {
        core::str::from_utf8(&self.label[..self.label_len]).unwrap_or("")
    }

    fn fs_type_str(&self) -> &str {
        core::str::from_utf8(&self.fs_type[..self.fs_type_len]).unwrap_or("")
    }
}

// ── Context Menu ───────────────────────────────────────────────────────────

pub struct ContextMenuItem {
    pub id: u32,
    pub label: &'static str,
    pub enabled: bool,
    pub is_separator: bool,
}

pub struct ContextMenu {
    pub layer_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub items: Vec<ContextMenuItem>,
    pub items_y: Vec<i32>,
    pub hover_idx: Option<usize>,
    pub target_icon_idx: usize,
}

// ── Drag State ─────────────────────────────────────────────────────────────

pub struct DragIconState {
    pub icon_idx: usize,
    pub offset_x: i32,
    pub offset_y: i32,
}

// ── Desktop Icon Manager ───────────────────────────────────────────────────

pub struct DesktopIconManager {
    pub icons: Vec<DesktopIcon>,
    pub last_mount_check: u32,
    pub context_menu: Option<ContextMenu>,
    pub dragging: Option<DragIconState>,
    /// Currently selected icon index (highlighted with selection rect).
    pub selected_icon: Option<usize>,
    /// Whether mouse is pressed on an icon (for drag-start on move).
    pub mouse_down_icon: Option<(usize, i32, i32)>,
    /// Cached wallpaper pixels for icon region restore during drag.
    /// We store per-icon the wallpaper pixels behind the icon cell.
    wallpaper_cache: Vec<Vec<u32>>,
}

impl DesktopIconManager {
    pub fn new() -> Self {
        DesktopIconManager {
            icons: Vec::new(),
            last_mount_check: 0,
            context_menu: None,
            dragging: None,
            selected_icon: None,
            mouse_down_icon: None,
            wallpaper_cache: Vec::new(),
        }
    }

    /// Initialize: scan current mounts and load icons.
    pub fn init(&mut self, screen_width: u32) {
        self.scan_mounts(screen_width);
        self.load_positions(screen_width);
    }

    // ── Mount Scanning ─────────────────────────────────────────────────

    /// Poll mounts and update icons if changed. Returns true if icons changed.
    pub fn poll_mounts(&mut self, screen_width: u32) -> bool {
        let now = anyos_std::sys::uptime_ms();
        if now.wrapping_sub(self.last_mount_check) < MOUNT_POLL_INTERVAL_MS {
            return false;
        }
        self.last_mount_check = now;

        let mut buf = [0u8; 512];
        let n = anyos_std::fs::list_mounts(&mut buf);
        if n == u32::MAX || n == 0 {
            return false;
        }

        let data = &buf[..n as usize];

        // Parse mount entries: "path\tfstype\n"
        let mut new_mounts: Vec<([u8; 64], usize, [u8; 16], usize)> = Vec::new();
        let mut pos = 0usize;
        while pos < data.len() {
            let line_end = data[pos..].iter().position(|&b| b == b'\n')
                .map(|p| pos + p).unwrap_or(data.len());
            let line = &data[pos..line_end];
            pos = line_end + 1;

            if line.is_empty() { continue; }

            // Split on tab
            if let Some(tab) = line.iter().position(|&b| b == b'\t') {
                let path = &line[..tab];
                let fstype = &line[tab + 1..];

                // Skip /dev mount
                if path == b"/dev" { continue; }

                let mut mp = [0u8; 64];
                let mp_len = path.len().min(63);
                mp[..mp_len].copy_from_slice(&path[..mp_len]);

                let mut ft = [0u8; 16];
                let ft_len = fstype.len().min(15);
                ft[..ft_len].copy_from_slice(&fstype[..ft_len]);

                new_mounts.push((mp, mp_len, ft, ft_len));
            }
        }

        // Check if anything changed
        let same = new_mounts.len() == self.icons.len()
            && new_mounts.iter().zip(self.icons.iter()).all(|((mp, ml, _, _), icon)| {
                *ml == icon.mount_path_len && mp[..*ml] == icon.mount_path[..icon.mount_path_len]
            });

        if same {
            return false;
        }

        // Rebuild icon list, preserving positions for existing mounts
        let mut new_icons: Vec<DesktopIcon> = Vec::with_capacity(new_mounts.len());
        let mut idx = 0usize;

        for (mp, mp_len, ft, ft_len) in &new_mounts {
            // Check if we already have this mount
            let existing = self.icons.iter().find(|icon| {
                icon.mount_path_len == *mp_len && icon.mount_path[..*mp_len] == mp[..*mp_len]
            });

            let (x, y) = if let Some(ex) = existing {
                (ex.x, ex.y)
            } else {
                Self::default_position(idx, screen_width)
            };

            let mount_str = core::str::from_utf8(&mp[..*mp_len]).unwrap_or("");
            let fs_str = core::str::from_utf8(&ft[..*ft_len]).unwrap_or("");

            let (icon_pixels, icon_w, icon_h) = load_device_icon(fs_str, mount_str);

            let label = derive_label(mount_str, fs_str);
            let mut label_buf = [0u8; 32];
            let label_len = label.len().min(31);
            label_buf[..label_len].copy_from_slice(&label.as_bytes()[..label_len]);

            let is_ejectable = is_mount_ejectable(mount_str, fs_str);

            new_icons.push(DesktopIcon {
                mount_path: *mp,
                mount_path_len: *mp_len,
                label: label_buf,
                label_len,
                fs_type: *ft,
                fs_type_len: *ft_len,
                x, y,
                icon_pixels,
                icon_w, icon_h,
                is_ejectable,
            });
            idx += 1;
        }

        self.icons = new_icons;
        self.wallpaper_cache.clear();
        // Apply saved positions from disk (overrides defaults for known mounts)
        self.load_positions(screen_width);
        true
    }

    fn scan_mounts(&mut self, screen_width: u32) {
        self.last_mount_check = anyos_std::sys::uptime_ms();
        self.poll_mounts(screen_width);
        // Force poll even if interval hasn't passed
        self.last_mount_check = 0;
        self.poll_mounts(screen_width);
    }

    // ── Rendering ──────────────────────────────────────────────────────

    /// Render all desktop icons onto the background layer pixels.
    /// Call after loading wallpaper or when icons change/move.
    pub fn render_icons(&self, bg_pixels: &mut [u32], stride: u32, buf_h: u32) {
        for (i, icon) in self.icons.iter().enumerate() {
            let selected = self.selected_icon == Some(i);
            self.render_one_icon(icon, selected, bg_pixels, stride, buf_h);
        }
    }

    fn render_one_icon(&self, icon: &DesktopIcon, selected: bool, pixels: &mut [u32], stride: u32, buf_h: u32) {
        let cell_x = icon.x;
        let cell_y = icon.y;

        // Draw selection highlight behind icon if selected
        if selected {
            // Selection rect covers icon + label area
            let sel_x = cell_x;
            let sel_y = cell_y - 2;
            let sel_w = CELL_WIDTH;
            let sel_h = CELL_HEIGHT;
            fill_rounded_rect(pixels, stride, buf_h, sel_x, sel_y, sel_w, sel_h, SELECTION_RADIUS, SELECTION_BG);
            // Draw border around selection
            draw_rounded_rect_outline(pixels, stride, buf_h, sel_x, sel_y, sel_w, sel_h, SELECTION_RADIUS, SELECTION_BORDER);
        }

        // Center the icon within the cell
        let icon_x = cell_x + (CELL_WIDTH as i32 - icon.icon_w as i32) / 2;
        let icon_y = cell_y;

        // Draw the icon with alpha blending
        for row in 0..icon.icon_h {
            let py = icon_y + row as i32;
            if py < 0 || py >= buf_h as i32 { continue; }
            for col in 0..icon.icon_w {
                let px = icon_x + col as i32;
                if px < 0 || px >= stride as i32 { continue; }
                let src = icon.icon_pixels[(row * icon.icon_w + col) as usize];
                let a = (src >> 24) & 0xFF;
                if a == 0 { continue; }
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

        // Draw label below icon
        let label = icon.label_str();
        if label.is_empty() { return; }

        let (tw, th) = anyos_std::ui::window::font_measure(LABEL_FONT_ID, LABEL_FONT_SIZE, label);
        let label_x = cell_x + (CELL_WIDTH as i32 - tw as i32) / 2;
        let label_y = cell_y + icon.icon_h as i32 + 4;

        // Draw shadow (inverted/dark text offset 1px down-right)
        let shadow_color: u32 = 0xC0000000; // semi-transparent black
        anyos_std::ui::window::font_render_buf(
            LABEL_FONT_ID, LABEL_FONT_SIZE,
            pixels, stride, buf_h,
            label_x + 1, label_y + 1,
            shadow_color, label,
        );

        // Draw label text in white
        anyos_std::ui::window::font_render_buf(
            LABEL_FONT_ID, LABEL_FONT_SIZE,
            pixels, stride, buf_h,
            label_x, label_y,
            0xFFFFFFFF, label,
        );
    }

    /// Get the bounding rect of an icon cell for damage tracking.
    fn icon_damage_rect(&self, idx: usize) -> Rect {
        if idx >= self.icons.len() { return Rect::new(0, 0, 0, 0); }
        let icon = &self.icons[idx];
        // Include some extra space for label
        Rect::new(icon.x - 2, icon.y - 2, CELL_WIDTH + 4, CELL_HEIGHT + 4)
    }

    // ── Hit Testing ────────────────────────────────────────────────────

    /// Test if a point is over any desktop icon. Returns the icon index.
    pub fn hit_test(&self, mx: i32, my: i32) -> Option<usize> {
        for (i, icon) in self.icons.iter().enumerate() {
            let x0 = icon.x;
            let y0 = icon.y;
            let x1 = x0 + CELL_WIDTH as i32;
            let y1 = y0 + CELL_HEIGHT as i32;
            if mx >= x0 && mx < x1 && my >= y0 && my < y1 {
                return Some(i);
            }
        }
        None
    }

    // ── Default Positioning ────────────────────────────────────────────

    fn default_position(index: usize, screen_width: u32) -> (i32, i32) {
        let x = screen_width as i32 - CELL_WIDTH as i32 - MARGIN_RIGHT;
        let y = MARGIN_TOP + (index as i32) * CELL_SPACING;
        (x, y)
    }

    // ── Dragging ───────────────────────────────────────────────────────

    pub fn start_drag(&mut self, icon_idx: usize, mouse_x: i32, mouse_y: i32) {
        if icon_idx >= self.icons.len() { return; }
        let icon = &self.icons[icon_idx];
        self.dragging = Some(DragIconState {
            icon_idx,
            offset_x: mouse_x - icon.x,
            offset_y: mouse_y - icon.y,
        });
    }

    pub fn update_drag(&mut self, mouse_x: i32, mouse_y: i32) -> Option<(Rect, Rect)> {
        let drag = self.dragging.as_ref()?;
        let idx = drag.icon_idx;
        if idx >= self.icons.len() { return None; }

        let old_rect = self.icon_damage_rect(idx);
        let new_x = mouse_x - drag.offset_x;
        let new_y = mouse_y - drag.offset_y;

        self.icons[idx].x = new_x;
        self.icons[idx].y = new_y;

        let new_rect = self.icon_damage_rect(idx);
        Some((old_rect, new_rect))
    }

    pub fn end_drag(&mut self) -> bool {
        if self.dragging.take().is_some() {
            self.save_positions();
            true
        } else {
            false
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    // ── Context Menu ───────────────────────────────────────────────────

    pub fn open_context_menu(
        &mut self,
        icon_idx: usize,
        mx: i32,
        my: i32,
        compositor: &mut Compositor,
    ) {
        // Close existing menu first
        self.close_context_menu(compositor);

        if icon_idx >= self.icons.len() { return; }
        let ejectable = self.icons[icon_idx].is_ejectable;

        let mut items: Vec<ContextMenuItem> = Vec::new();
        let mut items_y: Vec<i32> = Vec::new();

        let mut total_h: i32 = CTX_PADDING;

        // "Open"
        items_y.push(total_h);
        items.push(ContextMenuItem { id: CTX_ITEM_OPEN, label: "Open", enabled: true, is_separator: false });
        total_h += CTX_ITEM_HEIGHT as i32;

        // Separator
        items_y.push(total_h);
        items.push(ContextMenuItem { id: 0, label: "", enabled: false, is_separator: true });
        total_h += CTX_SEPARATOR_HEIGHT as i32;

        // "Eject" (only if ejectable)
        if ejectable {
            items_y.push(total_h);
            items.push(ContextMenuItem { id: CTX_ITEM_EJECT, label: "Eject", enabled: true, is_separator: false });
            total_h += CTX_ITEM_HEIGHT as i32;

            // Separator
            items_y.push(total_h);
            items.push(ContextMenuItem { id: 0, label: "", enabled: false, is_separator: true });
            total_h += CTX_SEPARATOR_HEIGHT as i32;
        }

        // "Get Info" (disabled for now)
        items_y.push(total_h);
        items.push(ContextMenuItem { id: CTX_ITEM_INFO, label: "Get Info", enabled: false, is_separator: false });
        total_h += CTX_ITEM_HEIGHT as i32;

        total_h += CTX_PADDING;

        let dd_width: u32 = 160;
        let dd_height = total_h as u32;

        let layer_id = compositor.add_layer(mx, my, dd_width, dd_height, false);
        compositor.raise_layer(layer_id);

        self.context_menu = Some(ContextMenu {
            layer_id,
            x: mx, y: my,
            width: dd_width,
            height: dd_height,
            items,
            items_y,
            hover_idx: None,
            target_icon_idx: icon_idx,
        });

        self.render_context_menu(compositor);
    }

    pub fn render_context_menu(&self, compositor: &mut Compositor) {
        let cm = match &self.context_menu {
            Some(c) => c,
            None => return,
        };

        if let Some(pixels) = compositor.layer_pixels(cm.layer_id) {
            let w = cm.width;
            let h = cm.height;

            // Clear to transparent
            for p in pixels.iter_mut() { *p = 0x00000000; }

            // Background
            fill_rounded_rect(pixels, w, h, 0, 0, w, h, 6, CTX_COLOR_BG);
            draw_rect_outline(pixels, w, 0, 0, w, h, CTX_COLOR_BORDER);

            for (i, item) in cm.items.iter().enumerate() {
                let iy = cm.items_y[i];

                if item.is_separator {
                    let line_y = iy + CTX_SEPARATOR_HEIGHT as i32 / 2;
                    if line_y >= 0 && (line_y as u32) < h {
                        for x in 8i32..(w as i32 - 8) {
                            if x >= 0 && (x as u32) < w {
                                let idx = (line_y as u32 * w + x as u32) as usize;
                                if idx < pixels.len() { pixels[idx] = CTX_COLOR_SEPARATOR; }
                            }
                        }
                    }
                    continue;
                }

                // Hover highlight
                if cm.hover_idx == Some(i) && item.enabled {
                    fill_rect(pixels, w, h, 4, iy, w - 8, CTX_ITEM_HEIGHT, CTX_COLOR_HOVER);
                }

                // Text
                let text_x = 16i32;
                let (_, th) = anyos_std::ui::window::font_measure(
                    LABEL_FONT_ID, 13, item.label,
                );
                let text_y = iy + ((CTX_ITEM_HEIGHT as i32 - th as i32) / 2).max(0);
                let text_color = if !item.enabled {
                    CTX_COLOR_DISABLED
                } else if cm.hover_idx == Some(i) {
                    0xFFFFFFFF
                } else {
                    CTX_COLOR_TEXT
                };
                anyos_std::ui::window::font_render_buf(
                    LABEL_FONT_ID, 13, pixels, w, h, text_x, text_y, text_color, item.label,
                );
            }
        }

        compositor.mark_layer_dirty(cm.layer_id);
        let bounds = Rect::new(cm.x, cm.y, cm.width, cm.height);
        compositor.add_damage(bounds);
    }

    /// Update context menu hover. Returns true if redraw needed.
    pub fn update_context_hover(&mut self, mx: i32, my: i32) -> bool {
        let cm = match &mut self.context_menu {
            Some(c) => c,
            None => return false,
        };

        if mx < cm.x || mx >= cm.x + cm.width as i32
            || my < cm.y || my >= cm.y + cm.height as i32
        {
            if cm.hover_idx.is_some() {
                cm.hover_idx = None;
                return true;
            }
            return false;
        }

        let local_y = my - cm.y;
        let mut new_hover = None;
        for (i, &iy) in cm.items_y.iter().enumerate() {
            if !cm.items[i].is_separator && cm.items[i].enabled {
                if local_y >= iy && local_y < iy + CTX_ITEM_HEIGHT as i32 {
                    new_hover = Some(i);
                    break;
                }
            }
        }

        if new_hover != cm.hover_idx {
            cm.hover_idx = new_hover;
            true
        } else {
            false
        }
    }

    /// Hit test context menu click. Returns (item_id, icon_idx) if hit.
    pub fn hit_test_context_menu(&self, mx: i32, my: i32) -> Option<(u32, usize)> {
        let cm = self.context_menu.as_ref()?;

        if mx < cm.x || mx >= cm.x + cm.width as i32
            || my < cm.y || my >= cm.y + cm.height as i32
        {
            return None;
        }

        let local_y = my - cm.y;
        for (i, item) in cm.items.iter().enumerate() {
            let iy = cm.items_y[i];
            let ih = if item.is_separator { CTX_SEPARATOR_HEIGHT } else { CTX_ITEM_HEIGHT };
            if local_y >= iy && local_y < iy + ih as i32 {
                if item.enabled && !item.is_separator {
                    return Some((item.id, cm.target_icon_idx));
                }
                return None;
            }
        }
        None
    }

    /// Check if a point is inside the context menu.
    pub fn is_in_context_menu(&self, mx: i32, my: i32) -> bool {
        if let Some(ref cm) = self.context_menu {
            mx >= cm.x && mx < cm.x + cm.width as i32
                && my >= cm.y && my < cm.y + cm.height as i32
        } else {
            false
        }
    }

    pub fn has_context_menu(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn close_context_menu(&mut self, compositor: &mut Compositor) {
        if let Some(cm) = self.context_menu.take() {
            let bounds = Rect::new(cm.x, cm.y, cm.width, cm.height);
            compositor.remove_layer(cm.layer_id);
            compositor.add_damage(bounds);
        }
    }

    /// Handle a context menu item action. Returns action to perform.
    pub fn handle_context_action(&self, item_id: u32, icon_idx: usize) -> ContextAction {
        match item_id {
            CTX_ITEM_OPEN => {
                if icon_idx < self.icons.len() {
                    let path = self.icons[icon_idx].mount_path_str();
                    ContextAction::OpenFinder(String::from(path))
                } else {
                    ContextAction::None
                }
            }
            CTX_ITEM_EJECT => {
                if icon_idx < self.icons.len() {
                    let path = self.icons[icon_idx].mount_path_str();
                    ContextAction::Eject(String::from(path))
                } else {
                    ContextAction::None
                }
            }
            _ => ContextAction::None,
        }
    }

    // ── Position Persistence ───────────────────────────────────────────

    fn save_positions(&self) {
        use anyos_std::fs;
        use anyos_std::process;

        let uid = process::getuid() as u32;
        let mut buf = [0u8; 1024];
        let mut pos = 0usize;

        for icon in &self.icons {
            // Format: UID:mount_path:x:y\n
            pos += write_u32(&mut buf[pos..], uid);
            if pos < buf.len() { buf[pos] = b':'; pos += 1; }
            let mp_len = icon.mount_path_len.min(buf.len() - pos - 20);
            buf[pos..pos + mp_len].copy_from_slice(&icon.mount_path[..mp_len]);
            pos += mp_len;
            if pos < buf.len() { buf[pos] = b':'; pos += 1; }
            pos += write_i32(&mut buf[pos..], icon.x);
            if pos < buf.len() { buf[pos] = b':'; pos += 1; }
            pos += write_i32(&mut buf[pos..], icon.y);
            if pos < buf.len() { buf[pos] = b'\n'; pos += 1; }
        }

        let fd = fs::open(
            "/System/users/desktop_icons",
            fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC,
        );
        if fd != u32::MAX {
            fs::write(fd, &buf[..pos]);
            fs::close(fd);
        }
    }

    fn load_positions(&mut self, screen_width: u32) {
        use anyos_std::fs;
        use anyos_std::process;

        let uid = process::getuid() as u32;

        let fd = fs::open("/System/users/desktop_icons", 0);
        if fd == u32::MAX { return; }

        let mut buf = [0u8; 1024];
        let n = fs::read(fd, &mut buf) as usize;
        fs::close(fd);
        if n == 0 { return; }

        let data = &buf[..n];
        let mut pos = 0usize;

        while pos < data.len() {
            let line_end = data[pos..].iter().position(|&b| b == b'\n')
                .map(|p| pos + p).unwrap_or(data.len());
            let line = &data[pos..line_end];
            pos = line_end + 1;

            if line.is_empty() { continue; }

            // Parse: UID:mount_path:x:y
            let parts: Vec<&[u8]> = split_bytes(line, b':');
            if parts.len() < 4 { continue; }

            let parsed_uid = parse_u32(parts[0]);
            if parsed_uid != uid { continue; }

            let mount_path = parts[1];
            let x = parse_i32(parts[2]);
            let y = parse_i32(parts[3]);

            // Find matching icon and update position
            for icon in &mut self.icons {
                if icon.mount_path_len == mount_path.len()
                    && icon.mount_path[..icon.mount_path_len] == *mount_path
                {
                    icon.x = x;
                    icon.y = y;
                    break;
                }
            }
        }
    }
}

// ── Context Action ─────────────────────────────────────────────────────────

pub enum ContextAction {
    None,
    OpenFinder(String),
    Eject(String),
}

// ── Helper Functions ───────────────────────────────────────────────────────

/// Load a device icon from /System/media/icons/devices/.
/// Returns (pixels, width, height). Falls back to generic.ico.
fn load_device_icon(fs_type: &str, mount_path: &str) -> (Vec<u32>, u32, u32) {
    // Determine icon path based on fs type and mount path.
    // Available icons: generic.ico, cdrom.ico, usb/storage.ico, usb/cdrom.ico
    let icon_path = if mount_path.starts_with("/mnt/usb") || mount_path.starts_with("/mnt/USB") {
        if fs_type == "iso9660" {
            "/System/media/icons/devices/usb/cdrom.ico"
        } else {
            "/System/media/icons/devices/usb/storage.ico"
        }
    } else if fs_type == "iso9660" {
        "/System/media/icons/devices/cdrom.ico"
    } else {
        // Root partition, internal drives, network, etc. — use generic
        "/System/media/icons/devices/generic.ico"
    };

    if let Some(result) = try_load_ico(icon_path) {
        return result;
    }

    // Fallback to generic
    if icon_path != "/System/media/icons/devices/generic.ico" {
        if let Some(result) = try_load_ico("/System/media/icons/devices/generic.ico") {
            return result;
        }
    }

    // Ultimate fallback: generate a simple colored rectangle
    generate_fallback_icon()
}

fn try_load_ico(path: &str) -> Option<(Vec<u32>, u32, u32)> {
    use anyos_std::fs;

    let fd = fs::open(path, 0);
    if fd == u32::MAX { return None; }

    let mut stat_buf = [0u32; 4];
    if fs::fstat(fd, &mut stat_buf) == u32::MAX {
        fs::close(fd);
        return None;
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 256 * 1024 {
        fs::close(fd);
        return None;
    }

    let mut data = vec![0u8; file_size];
    let bytes_read = fs::read(fd, &mut data) as usize;
    fs::close(fd);
    if bytes_read == 0 { return None; }

    let info = libimage_client::probe_ico_size(&data[..bytes_read], ICON_SIZE)?;
    let pixel_count = (info.width * info.height) as usize;
    if pixel_count == 0 || pixel_count > 262144 { return None; }

    let mut pixels = vec![0u32; pixel_count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    if libimage_client::decode_ico_size(
        &data[..bytes_read], ICON_SIZE, &mut pixels, &mut scratch,
    ).is_err() {
        return None;
    }

    // Scale to ICON_SIZE if the decoded image is a different size
    if info.width != ICON_SIZE || info.height != ICON_SIZE {
        let dst_count = (ICON_SIZE * ICON_SIZE) as usize;
        let mut scaled = vec![0u32; dst_count];
        libimage_client::scale_image(
            &pixels, info.width, info.height,
            &mut scaled, ICON_SIZE, ICON_SIZE,
            libimage_client::MODE_CONTAIN,
        );
        Some((scaled, ICON_SIZE, ICON_SIZE))
    } else {
        Some((pixels, info.width, info.height))
    }
}

/// Generate a simple fallback icon (colored rounded rectangle).
fn generate_fallback_icon() -> (Vec<u32>, u32, u32) {
    let w = ICON_SIZE;
    let h = ICON_SIZE;
    let mut pixels = vec![0x00000000u32; (w * h) as usize];

    // Draw a simple gray rounded rectangle with a drive shape
    fill_rounded_rect(&mut pixels, w, h, 4, 4, w - 8, h - 8, 6, 0xFF808080);

    // Draw a small rectangle inside (representing a drive)
    fill_rect(&mut pixels, w, h, 12, 16, w as u32 - 24, 4, 0xFFA0A0A0);
    fill_rect(&mut pixels, w, h, 12, 24, w as u32 - 24, 4, 0xFFA0A0A0);

    (pixels, w, h)
}

/// Derive a display label from mount path and filesystem type.
fn derive_label(mount_path: &str, fs_type: &str) -> String {
    match mount_path {
        "/" => String::from("System"),
        _ => {
            // Extract last path component
            let name = mount_path.rsplit('/').next().unwrap_or(mount_path);
            if name.is_empty() {
                // Use fs type as fallback
                match fs_type {
                    "iso9660" => String::from("CD-ROM"),
                    "smb" => String::from("Network"),
                    _ => String::from("Drive"),
                }
            } else {
                // Prettify common names
                if name.starts_with("usb") {
                    String::from("USB Drive")
                } else if name.starts_with("cdrom") {
                    String::from("CD-ROM")
                } else {
                    String::from(name)
                }
            }
        }
    }
}

/// Determine if a mount is ejectable.
fn is_mount_ejectable(mount_path: &str, fs_type: &str) -> bool {
    // Root is never ejectable
    if mount_path == "/" { return false; }
    // Everything under /mnt is potentially ejectable
    if mount_path.starts_with("/mnt/") { return true; }
    // SMB is ejectable
    if fs_type == "smb" { return true; }
    false
}

// ── Byte-level helpers (no_std compatible) ─────────────────────────────────

fn write_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        if !buf.is_empty() { buf[0] = b'0'; }
        return 1;
    }
    let mut digits = [0u8; 10];
    let mut d = 0;
    let mut n = val;
    while n > 0 {
        digits[d] = b'0' + (n % 10) as u8;
        d += 1;
        n /= 10;
    }
    let len = d.min(buf.len());
    for i in 0..len {
        buf[i] = digits[d - 1 - i];
    }
    len
}

fn write_i32(buf: &mut [u8], val: i32) -> usize {
    if val < 0 {
        if !buf.is_empty() {
            buf[0] = b'-';
            return 1 + write_u32(&mut buf[1..], (-val) as u32);
        }
        return 0;
    }
    write_u32(buf, val as u32)
}

fn parse_u32(data: &[u8]) -> u32 {
    let mut val: u32 = 0;
    for &b in data {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
        }
    }
    val
}

fn parse_i32(data: &[u8]) -> i32 {
    if data.first() == Some(&b'-') {
        -(parse_u32(&data[1..]) as i32)
    } else {
        parse_u32(data) as i32
    }
}

fn split_bytes<'a>(data: &'a [u8], sep: u8) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == sep {
            parts.push(&data[start..i]);
            start = i + 1;
        }
    }
    parts.push(&data[start..]);
    parts
}
