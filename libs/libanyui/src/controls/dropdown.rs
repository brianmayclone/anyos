//! DropDown — combobox-style control with a pop-up item list.
//!
//! Items are stored as a pipe-separated string (e.g. "Option A|Option B|Option C").
//! `base.state` holds the selected index.  An internal `open` flag controls whether
//! the drop-down list is visible.

use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};
use crate::control::{KEY_UP, KEY_DOWN, KEY_ENTER, KEY_ESCAPE};

const ITEM_HEIGHT: u32 = 28;
const CORNER: u32 = 6;

pub struct DropDown {
    pub(crate) text_base: TextControlBase,
    pub(crate) open: bool,
    pub(crate) hover_index: i32,
}

impl DropDown {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base, open: false, hover_index: -1 }
    }

    fn item_count(&self) -> usize {
        if self.text_base.text.is_empty() { return 0; }
        self.text_base.text.iter().filter(|&&b| b == b'|').count() + 1
    }

    fn item_label(&self, index: usize) -> &[u8] {
        let text = &self.text_base.text;
        let mut seg = 0;
        let mut start = 0;
        for i in 0..text.len() {
            if text[i] == b'|' {
                if seg == index { return &text[start..i]; }
                seg += 1;
                start = i + 1;
            }
        }
        if seg == index { &text[start..] } else { &[] }
    }

    fn popup_height(&self) -> u32 {
        let n = self.item_count() as u32;
        n * ITEM_HEIGHT + 4 // 2px padding top + bottom
    }
}

impl Control for DropDown {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::DropDown }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let x = ax + b.x;
        let y = ay + b.y;
        let w = b.w;
        let h = b.h;
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let focused = b.focused;
        let hovered = b.hovered;

        // ── Closed button ─────────────────────────────────────────
        let bg = if disabled {
            crate::theme::darken(tc.control_bg, 10)
        } else if hovered && !self.open {
            tc.control_hover
        } else {
            tc.input_bg
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, CORNER, bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, CORNER, tc.input_border);

        // Selected item text
        let selected = b.state as usize;
        let label = self.item_label(selected);
        let font_size = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let text_color = if disabled { tc.text_disabled } else { tc.text };
        if !label.is_empty() {
            let ty = y + (h as i32 - font_size as i32) / 2;
            crate::draw::draw_text_sized(surface, x + 10, ty, text_color, label, font_size);
        }

        // Chevron (▼ triangle)
        let chevron_x = x + w as i32 - 20;
        let chevron_y = y + (h as i32 / 2) - 2;
        let chevron_color = if disabled { tc.text_disabled } else { tc.text_secondary };
        for row in 0..5u32 {
            let half = row as i32;
            let cx = chevron_x + (4 - half);
            let cw = 1 + half * 2;
            crate::draw::fill_rect(surface, cx, chevron_y + row as i32, cw as u32, 1, chevron_color);
        }

        // Focus ring
        if focused && !disabled && !self.open {
            crate::draw::draw_focus_ring(surface, x, y, w, h, CORNER, tc.accent);
        }

        // ── Open popup list ───────────────────────────────────────
        if self.open {
            let n = self.item_count();
            let popup_y = y + h as i32 + 2;
            let popup_h = self.popup_height();

            // Shadow
            crate::draw::draw_shadow_rounded_rect(surface, x, popup_y, w, popup_h, CORNER as i32, 0, 4, 8, 80);
            // Background
            crate::draw::fill_rounded_rect(surface, x, popup_y, w, popup_h, CORNER, tc.card_bg);
            crate::draw::draw_rounded_border(surface, x, popup_y, w, popup_h, CORNER, tc.card_border);

            for i in 0..n {
                let iy = popup_y + 2 + (i as u32 * ITEM_HEIGHT) as i32;
                let is_selected = i == selected;
                let is_hovered = i as i32 == self.hover_index;

                // Highlight
                if is_selected || is_hovered {
                    let hl_color = if is_selected { tc.accent } else { tc.control_hover };
                    crate::draw::fill_rounded_rect(surface, x + 2, iy, w - 4, ITEM_HEIGHT, 4, hl_color);
                }

                let il = self.item_label(i);
                if !il.is_empty() {
                    let ty = iy + (ITEM_HEIGHT as i32 - font_size as i32) / 2;
                    let item_text_color = if is_selected { 0xFFFFFFFF } else { tc.text };
                    crate::draw::draw_text_sized(surface, x + 10, ty, item_text_color, il, font_size);
                }

                // Checkmark for selected
                if is_selected {
                    let cx = x + w as i32 - 24;
                    let cy = iy + (ITEM_HEIGHT as i32 / 2) - 4;
                    // Simple checkmark: two lines
                    crate::draw::fill_rect(surface, cx, cy + 4, 2, 4, 0xFFFFFFFF);
                    crate::draw::fill_rect(surface, cx + 2, cy + 6, 2, 2, 0xFFFFFFFF);
                    crate::draw::fill_rect(surface, cx + 4, cy + 4, 2, 2, 0xFFFFFFFF);
                    crate::draw::fill_rect(surface, cx + 6, cy + 2, 2, 2, 0xFFFFFFFF);
                    crate::draw::fill_rect(surface, cx + 8, cy, 2, 2, 0xFFFFFFFF);
                }
            }
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        let header_h = self.text_base.base.h as i32;
        if ly < header_h {
            // Clicked the header: toggle open/close
            self.open = !self.open;
            self.hover_index = -1;
            self.text_base.base.mark_dirty();
            return EventResponse::CONSUMED;
        }

        // Clicked in popup area
        if self.open {
            let popup_top = header_h + 2 + 2; // +2 gap, +2 padding
            let item_idx = ((ly - popup_top) / ITEM_HEIGHT as i32).max(0) as usize;
            let n = self.item_count();
            if item_idx < n {
                let old = self.text_base.base.state;
                self.text_base.base.state = item_idx as u32;
                self.open = false;
                self.hover_index = -1;
                self.text_base.base.mark_dirty();
                if old != self.text_base.base.state {
                    return EventResponse::CHANGED;
                }
            }
            self.open = false;
            self.text_base.base.mark_dirty();
        }
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, _lx: i32, ly: i32) -> EventResponse {
        if !self.open { return EventResponse::IGNORED; }
        let b = &self.text_base.base;
        let popup_top = b.h as i32 + 2 + 2;
        let new_hover = ((ly - popup_top) / ITEM_HEIGHT as i32).max(-1);
        let n = self.item_count() as i32;
        let new_hover = if new_hover >= n { -1 } else { new_hover };
        if new_hover != self.hover_index {
            self.hover_index = new_hover;
            self.text_base.base.mark_dirty();
        }
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let n = self.item_count();
        if n == 0 { return EventResponse::IGNORED; }

        match keycode {
            KEY_DOWN => {
                if self.open {
                    self.hover_index = (self.hover_index + 1).min(n as i32 - 1);
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                } else {
                    // Move to next item without opening
                    let cur = self.text_base.base.state;
                    if (cur as usize) < n - 1 {
                        self.text_base.base.state = cur + 1;
                        self.text_base.base.mark_dirty();
                        EventResponse::CHANGED
                    } else {
                        EventResponse::CONSUMED
                    }
                }
            }
            KEY_UP => {
                if self.open {
                    self.hover_index = (self.hover_index - 1).max(0);
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                } else {
                    let cur = self.text_base.base.state;
                    if cur > 0 {
                        self.text_base.base.state = cur - 1;
                        self.text_base.base.mark_dirty();
                        EventResponse::CHANGED
                    } else {
                        EventResponse::CONSUMED
                    }
                }
            }
            KEY_ENTER => {
                if self.open && self.hover_index >= 0 && (self.hover_index as usize) < n {
                    let old = self.text_base.base.state;
                    self.text_base.base.state = self.hover_index as u32;
                    self.open = false;
                    self.hover_index = -1;
                    self.text_base.base.mark_dirty();
                    if old != self.text_base.base.state {
                        return EventResponse::CHANGED;
                    }
                    EventResponse::CONSUMED
                } else {
                    self.open = !self.open;
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                }
            }
            KEY_ESCAPE => {
                if self.open {
                    self.open = false;
                    self.hover_index = -1;
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                } else {
                    EventResponse::IGNORED
                }
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_blur(&mut self) {
        self.base_mut().focused = false;
        self.base_mut().mark_dirty();
        if self.open {
            self.open = false;
            self.hover_index = -1;
        }
    }
}
