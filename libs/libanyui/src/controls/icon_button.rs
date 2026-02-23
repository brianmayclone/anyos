use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct IconButton {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl IconButton {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base, pressed: false } }
}

impl Control for IconButton {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::IconButton }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let icon_id = self.text_base.base.state;
        let custom = self.text_base.base.color;
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let focused = self.text_base.base.focused;
        let corner = crate::theme::BUTTON_CORNER;

        // Background: hover highlight, pressed darken, custom color
        if self.pressed && !disabled {
            let bg = if custom != 0 { crate::theme::darken(custom, 30) } else { tc.control_pressed };
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        } else if hovered && !disabled {
            let bg = if custom != 0 { crate::theme::lighten(custom, 12) } else { tc.control_hover };
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        } else if custom != 0 {
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, custom);
        }

        // Draw icon if icon_id is set
        if icon_id > 0 {
            let icon_color = if disabled {
                tc.text_disabled
            } else if self.text_base.text_style.text_color != 0 {
                self.text_base.text_style.text_color
            } else {
                tc.text_secondary
            };
            let ix = x + (w as i32 - 16) / 2;
            let iy = y + (h as i32 - 16) / 2;
            crate::icons::draw_icon(surface, ix, iy, icon_id, icon_color);
        }

        // Text label
        if !self.text_base.text.is_empty() {
            let text_color = if disabled {
                tc.text_disabled
            } else if self.text_base.text_style.text_color != 0 {
                self.text_base.text_style.text_color
            } else {
                tc.text
            };
            let font_size = self.text_base.text_style.font_size;
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, font_size);
            let tx = x + (w as i32 - tw as i32) / 2;
            let ty = if icon_id > 0 {
                y + (h as i32 - 16) / 2 + 16 + 1
            } else {
                y + (h as i32 - font_size as i32) / 2
            };
            crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

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
}
