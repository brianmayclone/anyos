use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};

/// Height of a normal menu item in pixels.
const ITEM_H: i32 = 28;
/// Height of a divider separator in pixels.
const DIVIDER_H: i32 = 9;
/// Top/bottom padding inside the menu.
const MENU_PAD: i32 = 4;

/// Icon size in logical pixels for menu items with icons.
const ICON_SZ: i32 = 20;
/// Hidden prefix for disabled items. Format: "\x1DLabel".
const DISABLED_PREFIX: u8 = 0x1D;

/// A divider item is exactly the text "-".
fn is_divider(item: &[u8]) -> bool {
    item == b"-"
}

/// Extract icon path and label from an item that may contain \x1F separator.
/// Format: "icon_path\x1Flabel" → (icon_path, label). No \x1F → (empty, full item).
fn split_icon_label(item: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = item.iter().position(|&b| b == 0x1F) {
        (&item[..pos], &item[pos + 1..])
    } else {
        (&[], item)
    }
}

fn strip_disabled_prefix(item: &[u8]) -> (&[u8], bool) {
    if item.first().copied() == Some(DISABLED_PREFIX) {
        (&item[1..], true)
    } else {
        (item, false)
    }
}

/// Load an .ico file at a preferred size, returning (pixels, w, h).
fn load_icon_pixels(path: &str, preferred_size: u32) -> (alloc::vec::Vec<u32>, u32, u32) {
    let data = match read_file_raw(path) {
        Some(d) => d,
        None => return (alloc::vec::Vec::new(), 0, 0),
    };
    // Try ICO format first
    if let Some(info) = libimage_client::probe_ico_size(&data, preferred_size) {
        let count = (info.width * info.height) as usize;
        let mut pixels = alloc::vec![0u32; count];
        let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode_ico_size(&data, preferred_size, &mut pixels, &mut scratch)
            .is_ok()
        {
            return (pixels, info.width, info.height);
        }
    }
    // Fallback: try as regular image (PNG/BMP)
    if let Some(info) = libimage_client::probe(&data) {
        let count = (info.width * info.height) as usize;
        let mut pixels = alloc::vec![0u32; count];
        let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok() {
            return (pixels, info.width, info.height);
        }
    }
    (alloc::vec::Vec::new(), 0, 0)
}

/// Read a reasonably small icon file into memory.
fn read_file_raw(path: &str) -> Option<alloc::vec::Vec<u8>> {
    use anyos_std::fs::{File, Read};

    let mut file = File::open(path).ok()?;
    let size = file
        .metadata()
        .ok()
        .map(|meta| meta[1] as usize)
        .unwrap_or(0);
    if size == 0 || size > 256 * 1024 {
        return None;
    }
    let mut data = alloc::vec![0u8; size];
    let n = file.read(&mut data).ok()?;
    if n > 0 && n <= size {
        data.truncate(n);
        Some(data)
    } else {
        None
    }
}

pub struct ContextMenu {
    pub(crate) text_base: TextControlBase,
    hovered_item: u32,
    /// Whether any item has an icon (detected on recompute_size).
    has_icons: bool,
    /// Cached icon pixels: Vec of (pixels, w, h) per item. Empty vec = no icon.
    icon_cache: alloc::vec::Vec<(alloc::vec::Vec<u32>, u32, u32)>,
}

impl ContextMenu {
    pub fn new(text_base: TextControlBase) -> Self {
        let mut cm = Self {
            text_base,
            hovered_item: u32::MAX,
            has_icons: false,
            icon_cache: alloc::vec::Vec::new(),
        };
        cm.text_base.base.visible = false;
        cm.recompute_size();
        cm
    }

    /// Recompute w/h from pipe-separated item text. Detect and load icons.
    fn recompute_size(&mut self) {
        let items: alloc::vec::Vec<&[u8]> = self.text_base.text.split(|&b| b == b'|').collect();
        self.has_icons = false;
        self.icon_cache.clear();
        let mut max_w = 0u32;
        let mut total_h = MENU_PAD * 2;
        for item in &items {
            if is_divider(item) {
                total_h += DIVIDER_H;
                self.icon_cache.push((alloc::vec::Vec::new(), 0, 0));
            } else {
                let (item, _disabled) = strip_disabled_prefix(item);
                let (icon_path, label) = split_icon_label(item);
                let (tw, _) = crate::draw::text_size(label);
                if tw > max_w {
                    max_w = tw;
                }
                total_h += ITEM_H;
                // Load icon if path is present
                if !icon_path.is_empty() {
                    self.has_icons = true;
                    if let Ok(path_str) = core::str::from_utf8(icon_path) {
                        let pixels = load_icon_pixels(path_str, ICON_SZ as u32);
                        self.icon_cache.push(pixels);
                    } else {
                        self.icon_cache.push((alloc::vec::Vec::new(), 0, 0));
                    }
                } else {
                    self.icon_cache.push((alloc::vec::Vec::new(), 0, 0));
                }
            }
        }
        let icon_extra = if self.has_icons {
            ICON_SZ as u32 + 8
        } else {
            0
        };
        self.text_base.base.w = (max_w + icon_extra + 24).max(120);
        self.text_base.base.h = total_h.max(MENU_PAD * 2) as u32;
    }

    fn items(&self) -> alloc::vec::Vec<&[u8]> {
        self.text_base.text.split(|&b| b == b'|').collect()
    }

    fn item_enabled(item: &[u8]) -> bool {
        !is_divider(item) && !strip_disabled_prefix(item).1
    }

    fn first_selectable_index(&self) -> Option<u32> {
        self.items()
            .iter()
            .position(|item| Self::item_enabled(item))
            .map(|idx| idx as u32)
    }

    fn last_selectable_index(&self) -> Option<u32> {
        self.items()
            .iter()
            .rposition(|item| Self::item_enabled(item))
            .map(|idx| idx as u32)
    }

    fn move_selection(&self, current: u32, delta: i32) -> Option<u32> {
        let items = self.items();
        if items.is_empty() {
            return None;
        }
        let start = current.min(items.len().saturating_sub(1) as u32) as i32;
        let mut idx = start + delta;
        while idx >= 0 && (idx as usize) < items.len() {
            if Self::item_enabled(items[idx as usize]) {
                return Some(idx as u32);
            }
            idx += delta.signum();
        }
        None
    }

    /// Map a local Y coordinate to an item index, returning None for dividers or out-of-bounds.
    fn item_at_y(&self, ly: i32) -> Option<u32> {
        let items = self.items();
        let mut cur_y = MENU_PAD;
        for (i, item) in items.iter().enumerate() {
            let h = if is_divider(item) { DIVIDER_H } else { ITEM_H };
            if ly >= cur_y && ly < cur_y + h {
                return if !Self::item_enabled(item) {
                    None
                } else {
                    Some(i as u32)
                };
            }
            cur_y += h;
        }
        None
    }
}

impl Control for ContextMenu {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::ContextMenu
    }

    fn set_text(&mut self, t: &[u8]) {
        if let Some(tb) = self.text_base_mut() {
            tb.set_text(t);
        }
        self.recompute_size();
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();

        let items = self.items();
        let corner = crate::theme::scale(6);
        let menu_pad = crate::theme::scale_i32(MENU_PAD);
        let item_h = crate::theme::scale_i32(ITEM_H);
        let divider_h = crate::theme::scale_i32(DIVIDER_H);
        let fs = crate::draw::scale_font(14);

        // Shadow for popup depth
        crate::draw::draw_shadow_rounded_rect(
            surface,
            x,
            y,
            w,
            h,
            corner as i32,
            0,
            crate::theme::scale_i32(3),
            crate::theme::scale_i32(12),
            80,
        );

        // Opaque background + border
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, tc.sidebar_bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, tc.card_border);

        // Render each item
        let item_pad_x = crate::theme::scale_i32(4);
        let text_pad_x = crate::theme::scale_i32(12);
        let text_pad_y = crate::theme::scale_i32(6);
        let divider_pad_x = crate::theme::scale_i32(8);
        let highlight_corner = crate::theme::scale(4);
        let mut iy = y + menu_pad;
        for (i, item_text) in items.iter().enumerate() {
            if is_divider(item_text) {
                // Draw a thin horizontal line as divider
                let line_y = iy + divider_h / 2;
                let line_w = if w > (divider_pad_x as u32 * 2) {
                    w - divider_pad_x as u32 * 2
                } else {
                    1
                };
                crate::draw::fill_rect(
                    surface,
                    x + divider_pad_x,
                    line_y,
                    line_w,
                    1,
                    tc.card_border,
                );
                iy += divider_h;
            } else {
                let (item_text, disabled) = strip_disabled_prefix(item_text);
                // Highlight hovered or keyboard-selected item
                let highlighted =
                    !disabled && (i as u32 == self.hovered_item || i as u32 == b.state);
                if highlighted {
                    let hl_w = if w > (item_pad_x as u32 * 2) {
                        w - item_pad_x as u32 * 2
                    } else {
                        1
                    };
                    crate::draw::fill_rounded_rect(
                        surface,
                        x + item_pad_x,
                        iy,
                        hl_w,
                        item_h as u32,
                        highlight_corner,
                        tc.accent,
                    );
                }

                // Split icon path and label
                let (_icon_path, label) = split_icon_label(item_text);
                let icon_offset = if self.has_icons {
                    crate::theme::scale_i32(ICON_SZ + 8)
                } else {
                    0
                };

                // Render icon if cached
                if i < self.icon_cache.len() {
                    let (ref pixels, iw, ih) = self.icon_cache[i];
                    if !pixels.is_empty() && iw > 0 && ih > 0 {
                        let icon_sz = crate::theme::scale(ICON_SZ as u32);
                        let icon_x = x + text_pad_x;
                        let icon_y = iy + (item_h - icon_sz as i32) / 2;
                        crate::draw::blit_buffer_scaled(
                            surface, icon_x, icon_y, icon_sz, icon_sz, iw, ih, pixels,
                        );
                    }
                }

                // Item text (offset by icon width if icons are present)
                if !label.is_empty() {
                    let text_color = if disabled {
                        tc.text_disabled
                    } else if highlighted {
                        0xFFFFFFFF
                    } else {
                        tc.text
                    };
                    crate::draw::draw_text_sized(
                        surface,
                        x + text_pad_x + icon_offset,
                        iy + text_pad_y,
                        text_color,
                        label,
                        fs,
                    );
                }
                iy += item_h;
            }
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_mouse_move(&mut self, _lx: i32, ly: i32) -> EventResponse {
        let new_hover = self.item_at_y(ly).unwrap_or(u32::MAX);
        if new_hover != self.hovered_item {
            self.hovered_item = new_hover;
            self.text_base.base.mark_dirty();
        }
        EventResponse::CONSUMED
    }

    fn handle_mouse_leave(&mut self) {
        if self.hovered_item != u32::MAX {
            self.hovered_item = u32::MAX;
            self.text_base.base.mark_dirty();
        }
    }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        if let Some(item_idx) = self.item_at_y(ly) {
            self.text_base.base.state = item_idx;
            // Hide after selection
            self.text_base.base.visible = false;
            self.hovered_item = u32::MAX;
            EventResponse::CLICK
        } else {
            // Clicked on divider or out of bounds — ignore
            EventResponse::CONSUMED
        }
    }

    fn handle_blur(&mut self) {
        // Hide context menu when focus leaves
        self.text_base.base.visible = false;
        self.hovered_item = u32::MAX;
        self.text_base.base.mark_dirty();
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn handle_focus(&mut self) {
        self.text_base.base.focused = true;
        if self.first_selectable_index().is_some() && self.hovered_item == u32::MAX {
            self.text_base.base.state = self
                .first_selectable_index()
                .unwrap_or(self.text_base.base.state);
        }
        self.text_base.base.mark_dirty();
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, _modifiers: u32) -> EventResponse {
        use crate::control::*;

        match keycode {
            KEY_UP => {
                let next = self
                    .move_selection(self.text_base.base.state, -1)
                    .or_else(|| self.first_selectable_index());
                if let Some(next) = next {
                    self.text_base.base.state = next;
                    self.hovered_item = u32::MAX;
                    self.text_base.base.mark_dirty();
                }
                EventResponse::CONSUMED
            }
            KEY_DOWN => {
                let next = self
                    .move_selection(self.text_base.base.state, 1)
                    .or_else(|| self.first_selectable_index());
                if let Some(next) = next {
                    self.text_base.base.state = next;
                    self.hovered_item = u32::MAX;
                    self.text_base.base.mark_dirty();
                }
                EventResponse::CONSUMED
            }
            KEY_HOME => {
                if let Some(first) = self.first_selectable_index() {
                    self.text_base.base.state = first;
                    self.hovered_item = u32::MAX;
                    self.text_base.base.mark_dirty();
                }
                EventResponse::CONSUMED
            }
            KEY_END => {
                if let Some(last) = self.last_selectable_index() {
                    self.text_base.base.state = last;
                    self.hovered_item = u32::MAX;
                    self.text_base.base.mark_dirty();
                }
                EventResponse::CONSUMED
            }
            KEY_ENTER => {
                let idx = self.text_base.base.state;
                let items = self.items();
                if let Some(item) = items.get(idx as usize) {
                    if Self::item_enabled(item) {
                        self.text_base.base.visible = false;
                        self.hovered_item = u32::MAX;
                        return EventResponse::CLICK;
                    }
                }
                EventResponse::CONSUMED
            }
            KEY_ESCAPE => {
                self.text_base.base.visible = false;
                self.hovered_item = u32::MAX;
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            _ if char_code == b' ' as u32 => {
                let idx = self.text_base.base.state;
                let items = self.items();
                if let Some(item) = items.get(idx as usize) {
                    if Self::item_enabled(item) {
                        self.text_base.base.visible = false;
                        self.hovered_item = u32::MAX;
                        return EventResponse::CLICK;
                    }
                }
                EventResponse::CONSUMED
            }
            _ => EventResponse::IGNORED,
        }
    }
}
