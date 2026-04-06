//! DropDown — combobox-style control with a pop-up item list.
//!
//! Items are stored as a pipe-separated string (e.g. "Option A|Option B|Option C").
//! `base.state` holds the selected index.  When clicked, the event loop opens a
//! popup compositor window (reusing the ContextMenu popup infrastructure) to show
//! the item list on top of everything.

use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};
use crate::control::{KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_UP};

const CORNER: u32 = 6;

pub struct DropDown {
    pub(crate) text_base: TextControlBase,
    /// Set to true when the user clicks the header; the event loop reads
    /// this flag to open a popup and immediately clears it.
    pub(crate) open: bool,
}

impl DropDown {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            open: false,
        }
    }

    pub fn item_count(&self) -> usize {
        if self.text_base.text.is_empty() {
            return 0;
        }
        self.text_base.text.iter().filter(|&&b| b == b'|').count() + 1
    }

    pub fn item_label(&self, index: usize) -> &[u8] {
        let text = &self.text_base.text;
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
}

impl Control for DropDown {
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
        ControlKind::DropDown
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::scale(CORNER);

        let palette =
            crate::controls::chrome::flat_field_palette(0, ctx.hovered, ctx.focused, ctx.disabled);
        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        // ── Selected item text ──────────────────────────────────────
        let selected = b.state as usize;
        let label = self.item_label(selected);
        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        let text_color = if ctx.disabled {
            tc.text_disabled
        } else {
            tc.text
        };
        if !label.is_empty() {
            let ty = y + (h as i32 - font_size as i32) / 2;
            crate::draw::draw_text_sized(
                surface,
                x + crate::theme::scale_i32(10),
                ty,
                text_color,
                label,
                font_size,
            );
        }

        // ── Chevron (wide at top, narrow at bottom) ─────────────────
        let chevron_rows = crate::theme::scale_i32(5);
        let chevron_x = x + w as i32 - crate::theme::scale_i32(20);
        let chevron_y = y + (h as i32 / 2) - crate::theme::scale_i32(2);
        let chevron_color = if ctx.disabled {
            tc.text_disabled
        } else {
            tc.text_secondary
        };
        let half_max = chevron_rows - 1;
        for row in 0..chevron_rows {
            let half = half_max - row;
            let cx = chevron_x + (half_max - half);
            let cw = 1 + half * 2;
            crate::draw::fill_rect(surface, cx, chevron_y + row, cw as u32, 1, chevron_color);
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Toggle the popup request flag; the event loop will open
        // a popup compositor window when it sees open == true.
        self.open = !self.open;
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let n = self.item_count();
        if n == 0 {
            return EventResponse::IGNORED;
        }

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
        self.text_base.base.focused = false;
        self.base_mut().mark_dirty();
    }
}
