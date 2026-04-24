//! PlainButton — a borderless icon button that blends with its background.
//!
//! Normal state: only the icon is visible, no border, no fill.
//! Hover state:  a subtle rounded highlight rectangle appears behind the icon.
//! Pressed state: a slightly darker highlight.
//!
//! Used in activity bars, toolbars, and anywhere a flat icon-only button is needed.

use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};
use alloc::vec::Vec;

pub struct PlainButton {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
    pub(crate) icon_pixels: Vec<u32>,
    pub(crate) icon_w: u32,
    pub(crate) icon_h: u32,
}

impl PlainButton {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            pressed: false,
            icon_pixels: Vec::new(),
            icon_w: 0,
            icon_h: 0,
        }
    }

    pub fn set_icon_pixels(&mut self, data: &[u32], w: u32, h: u32) {
        let expected = (w as usize) * (h as usize);
        if data.len() < expected {
            return;
        }
        self.icon_pixels.clear();
        self.icon_pixels.extend_from_slice(&data[..expected]);
        self.icon_w = w;
        self.icon_h = h;
        self.text_base.base.mark_dirty();
    }
}

impl Control for PlainButton {
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
        ControlKind::PlainButton
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let disabled = b.disabled;
        let hovered = b.hovered;
        let corner = 6u32; // subtle rounded corners on hover

        // ── Background: ONLY on hover or pressed (otherwise invisible) ──
        if !disabled && (hovered || self.pressed) {
            let bg = if self.pressed {
                0x30FFFFFF // pressed: slightly brighter
            } else {
                0x18FFFFFF // hover: very subtle white overlay
            };
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        }

        // ── Focus ring ──
        if b.focused && !disabled {
            crate::draw::draw_rounded_border(surface, x, y, w, h, corner, 0xFF2D8CFF);
        }

        // ── Icon or text ──
        let tc = crate::theme::colors();
        let icon_opacity = if disabled { 96u8 } else { 255u8 };

        if !self.icon_pixels.is_empty() {
            // Center icon in button area
            let ix = x + (w as i32 - self.icon_w as i32) / 2;
            let iy = y + (h as i32 - self.icon_h as i32) / 2;
            super::icon_button::blit_alpha_opacity(
                surface,
                ix,
                iy,
                self.icon_w,
                self.icon_h,
                &self.icon_pixels,
                icon_opacity,
            );
        } else if b.state > 0 {
            // Legacy pixel-art icon
            let default_icon_sz = crate::theme::scale_i32(16);
            let icon_color = if disabled {
                tc.text_disabled
            } else {
                tc.text_secondary
            };
            let ix = x + (w as i32 - default_icon_sz) / 2;
            let iy = y + (h as i32 - default_icon_sz) / 2;
            crate::icons::draw_icon(surface, ix, iy, b.state, icon_color);
        } else if !self.text_base.text.is_empty() {
            // Text-only
            let text_color = if disabled {
                tc.text_disabled
            } else if self.text_base.text_style.text_color != 0 {
                self.text_base.text_style.text_color
            } else {
                tc.text
            };
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, font_size);
            let tx = x + (w as i32 - tw as i32) / 2;
            let ty = y + (h as i32 - font_size as i32) / 2;
            crate::draw::draw_text_sized(
                surface,
                tx,
                ty,
                text_color,
                &self.text_base.text,
                font_size,
            );
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        if keycode == crate::control::KEY_SPACE || keycode == crate::control::KEY_ENTER {
            EventResponse::CLICK
        } else {
            EventResponse::IGNORED
        }
    }
}
