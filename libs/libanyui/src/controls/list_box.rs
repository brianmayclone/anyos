//! ListBox — scrollable multi-line selection list.
//!
//! Displays a list of items (pipe-separated) in a scrollable box.
//! Supports single-select and multi-select mode.
//!
//! ## Value encoding
//!
//! - `base.state` — selected index for single-select mode (0-based).
//! - Multi-select: each item index is stored as a bit in a separate field.
//!   For simplicity, multi-select state is tracked in `selected_mask` (u64,
//!   supports up to 64 items).
//!
//! ## Text format
//!
//! Items are pipe-separated: `"Item A|Item B|Item C"`.
//! Mode is determined by a prefix: `"multi:"` enables multi-select.

use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};
use crate::control::{KEY_DOWN, KEY_ENTER, KEY_SPACE, KEY_UP};

const ITEM_HEIGHT: u32 = 22;
pub struct ListBox {
    pub(crate) text_base: TextControlBase,
    /// Bitfield for multi-select (up to 64 items).
    pub(crate) selected_mask: u64,
    /// Scroll offset in pixels.
    pub(crate) scroll_y: i32,
    /// Whether multi-select mode is enabled.
    pub(crate) multi: bool,
}

impl ListBox {
    pub fn new(text_base: TextControlBase) -> Self {
        // Check for "multi:" prefix.
        let multi = text_base.text.starts_with(b"multi:");
        Self {
            text_base,
            selected_mask: 0,
            scroll_y: 0,
            multi,
        }
    }

    /// Number of items.
    pub fn item_count(&self) -> usize {
        let text = self.item_text();
        if text.is_empty() {
            return 0;
        }
        text.iter().filter(|&&b| b == b'|').count() + 1
    }

    /// The item data (without "multi:" prefix).
    fn item_text(&self) -> &[u8] {
        let t = &self.text_base.text;
        if t.starts_with(b"multi:") {
            &t[6..]
        } else {
            t
        }
    }

    /// Get the label of item at index.
    pub fn item_label(&self, index: usize) -> &[u8] {
        let text = self.item_text();
        let mut seg = 0;
        let mut start = 0;
        for i in 0..text.len() {
            if text[i] == b'|' {
                if seg == index {
                    return &text[start..i];
                }
                seg += 1;
                start = i + 1;
            }
        }
        if seg == index {
            &text[start..]
        } else {
            &[]
        }
    }

    /// Check if an item is selected (works for both modes).
    pub fn is_selected(&self, index: usize) -> bool {
        if self.multi {
            index < 64 && (self.selected_mask & (1u64 << index)) != 0
        } else {
            self.text_base.base.state == index as u32
        }
    }

    /// Toggle selection of an item (multi-select mode).
    fn toggle_item(&mut self, index: usize) {
        if index < 64 {
            self.selected_mask ^= 1u64 << index;
        }
    }

    /// Total content height in pixels.
    fn content_height(&self) -> i32 {
        let ih = crate::theme::scale(ITEM_HEIGHT) as i32;
        self.item_count() as i32 * ih
    }

    /// Ensure selected index is visible by adjusting scroll.
    fn scroll_to_visible(&mut self, index: usize) {
        let ih = crate::theme::scale(ITEM_HEIGHT) as i32;
        let visible_h = self.text_base.base.h as i32;
        let item_y = index as i32 * ih;
        if item_y < self.scroll_y {
            self.scroll_y = item_y;
        } else if item_y + ih > self.scroll_y + visible_h {
            self.scroll_y = item_y + ih - visible_h;
        }
    }
}

impl Control for ListBox {
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
        ControlKind::ListBox
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::input_corner();

        // Background.
        let palette =
            crate::controls::chrome::flat_field_palette(0, ctx.hovered, ctx.focused, ctx.disabled);
        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        // Clip to inner area.
        let pad = crate::theme::scale_i32(2);
        let inner_x = x + pad;
        let inner_y = y + pad;
        let inner_w = w as i32 - pad * 2;
        let inner_h = h as i32 - pad * 2;

        let ih = crate::theme::scale(ITEM_HEIGHT) as i32;
        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        let text_pad = crate::theme::scale_i32(6);

        let n = self.item_count();
        for i in 0..n {
            let item_y = inner_y + (i as i32 * ih) - self.scroll_y;
            // Skip items outside visible area.
            if item_y + ih <= inner_y || item_y >= inner_y + inner_h {
                continue;
            }

            let selected = self.is_selected(i);

            // Selection highlight.
            if selected {
                let hy = item_y.max(inner_y);
                let hh = (item_y + ih).min(inner_y + inner_h) - hy;
                if hh > 0 {
                    let sel_color = if ctx.focused {
                        tc.accent
                    } else {
                        crate::theme::darken(tc.control_bg, 12)
                    };
                    crate::draw::fill_rect(
                        surface,
                        inner_x,
                        hy,
                        inner_w as u32,
                        hh as u32,
                        sel_color,
                    );
                }
            }

            // Item text.
            let label = self.item_label(i);
            if !label.is_empty() {
                let ty = item_y + (ih - font_size as i32) / 2;
                if ty >= inner_y - font_size as i32 && ty < inner_y + inner_h {
                    let text_color = if selected && ctx.focused {
                        0xFFFFFFFF // white on accent
                    } else if ctx.disabled {
                        tc.text_disabled
                    } else {
                        tc.text
                    };
                    crate::draw::draw_text_sized(
                        surface,
                        inner_x + text_pad,
                        ty,
                        text_color,
                        label,
                        font_size,
                    );
                }
            }
        }

        // Scrollbar thumb (if content overflows).
        let content_h = self.content_height();
        if content_h > h as i32 {
            let track_h = inner_h as f32;
            let thumb_h = ((h as f32 / content_h as f32) * track_h).max(16.0) as i32;
            let max_scroll = content_h - h as i32;
            let thumb_y = if max_scroll > 0 {
                inner_y + ((self.scroll_y as f32 / max_scroll as f32) * (track_h - thumb_h as f32)) as i32
            } else {
                inner_y
            };
            let sb_x = x + w as i32 - crate::theme::scale_i32(6);
            crate::draw::fill_rect(
                surface,
                sb_x,
                thumb_y,
                crate::theme::scale(4),
                thumb_h as u32,
                crate::theme::with_alpha(tc.text_secondary, 80),
            );
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        let ih = crate::theme::scale(ITEM_HEIGHT) as i32;
        let pad = crate::theme::scale_i32(2);
        let clicked_index = ((ly - pad + self.scroll_y) / ih) as usize;
        if clicked_index >= self.item_count() {
            return EventResponse::CONSUMED;
        }

        if self.multi {
            self.toggle_item(clicked_index);
        } else {
            self.text_base.base.state = clicked_index as u32;
        }
        self.text_base.base.mark_dirty();
        EventResponse::CHANGED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let n = self.item_count();
        if n == 0 {
            return EventResponse::IGNORED;
        }

        match keycode {
            KEY_DOWN => {
                let cur = self.text_base.base.state as usize;
                if cur < n - 1 {
                    self.text_base.base.state = (cur + 1) as u32;
                    self.scroll_to_visible(cur + 1);
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_UP => {
                let cur = self.text_base.base.state as usize;
                if cur > 0 {
                    self.text_base.base.state = (cur - 1) as u32;
                    self.scroll_to_visible(cur - 1);
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_SPACE | KEY_ENTER => {
                if self.multi {
                    let cur = self.text_base.base.state as usize;
                    self.toggle_item(cur);
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let max_scroll = (self.content_height() - self.text_base.base.h as i32).max(0);
        self.scroll_y = (self.scroll_y - delta * 20).clamp(0, max_scroll);
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }
}
