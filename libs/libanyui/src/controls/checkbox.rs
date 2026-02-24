use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Checkbox {
    pub(crate) text_base: TextControlBase,
}

impl Checkbox {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Checkbox {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Checkbox }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let checked = self.text_base.base.state != 0;
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let focused = self.text_base.base.focused;
        let sz = crate::theme::CHECKBOX_SIZE;

        let bg = if disabled {
            crate::theme::darken(tc.control_bg, 10)
        } else if checked {
            if hovered { crate::theme::lighten(tc.accent, 15) } else { tc.accent }
        } else if hovered {
            tc.control_hover
        } else {
            tc.control_bg
        };

        // Checkbox box
        crate::draw::fill_rounded_rect(surface, x, y, sz, sz, 4, bg);
        if !checked {
            crate::draw::draw_rounded_border(surface, x, y, sz, sz, 4, if hovered && !disabled { tc.accent } else { tc.input_border });
        }

        // Checkmark — 2px thick diagonal lines forming a ✓ shape
        // Short leg: (4,9) → (6,11), Long leg: (6,11) → (13,4)
        if checked {
            let cm = tc.check_mark;
            // Short leg (bottom-left to center-bottom), 2px wide
            crate::draw::fill_rect(surface, x + 4, y + 8,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 5, y + 9,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 6, y + 10, 2, 2, cm);
            // Long leg (center-bottom to top-right), 2px wide
            crate::draw::fill_rect(surface, x + 7,  y + 9,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 8,  y + 8,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 9,  y + 7,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 10, y + 6,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 11, y + 5,  2, 2, cm);
            crate::draw::fill_rect(surface, x + 12, y + 4,  2, 2, cm);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, sz, sz, 4, tc.accent);
        }

        // Label text
        let text_color = if disabled { tc.text_disabled } else { tc.text };
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, x + sz as i32 + 6, y + 2, text_color, &self.text_base.text, self.text_base.text_style.font_size);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }
}
