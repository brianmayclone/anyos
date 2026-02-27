//! DropDown — combobox-style control with a pop-up item list.
//!
//! Items are stored as a pipe-separated string (e.g. "Option A|Option B|Option C").
//! `base.state` holds the selected index.  When clicked, the event loop opens a
//! popup compositor window (reusing the ContextMenu popup infrastructure) to show
//! the item list on top of everything.

use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};
use crate::control::{KEY_UP, KEY_DOWN, KEY_ENTER, KEY_ESCAPE};

const CORNER: u32 = 6;

pub struct DropDown {
    pub(crate) text_base: TextControlBase,
    /// Set to true when the user clicks the header; the event loop reads
    /// this flag to open a popup and immediately clears it.
    pub(crate) open: bool,
    pub(crate) hover_index: i32,
}

impl DropDown {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base, open: false, hover_index: -1 }
    }

    pub fn item_count(&self) -> usize {
        if self.text_base.text.is_empty() { return 0; }
        self.text_base.text.iter().filter(|&&b| b == b'|').count() + 1
    }

    pub fn item_label(&self, index: usize) -> &[u8] {
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

        // ── Background ──────────────────────────────────────────────
        let bg = if disabled {
            crate::theme::darken(tc.control_bg, 10)
        } else if hovered {
            tc.control_hover
        } else {
            tc.input_bg
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, CORNER, bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, CORNER, tc.input_border);

        // ── Selected item text ──────────────────────────────────────
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

        // ── Chevron ▼ (wide at top, narrow at bottom) ───────────────
        let chevron_x = x + w as i32 - 20;
        let chevron_y = y + (h as i32 / 2) - 2;
        let chevron_color = if disabled { tc.text_disabled } else { tc.text_secondary };
        for row in 0..5i32 {
            let half = 4 - row; // row 0 → half=4 (widest), row 4 → half=0 (narrowest)
            let cx = chevron_x + (4 - half);
            let cw = 1 + half * 2;
            crate::draw::fill_rect(surface, cx, chevron_y + row, cw as u32, 1, chevron_color);
        }

        // ── Focus ring ──────────────────────────────────────────────
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, CORNER, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Toggle the popup request flag; the event loop will open
        // a popup compositor window when it sees open == true.
        self.open = !self.open;
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let n = self.item_count();
        if n == 0 { return EventResponse::IGNORED; }

        match keycode {
            KEY_DOWN => {
                let cur = self.text_base.base.state;
                if (cur as usize) < n - 1 {
                    self.text_base.base.state = cur + 1;
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_UP => {
                let cur = self.text_base.base.state;
                if cur > 0 {
                    self.text_base.base.state = cur - 1;
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_ENTER => {
                // Open the popup
                self.open = true;
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_ESCAPE => EventResponse::IGNORED,
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_blur(&mut self) {
        self.base_mut().focused = false;
        self.base_mut().mark_dirty();
    }
}
