use crate::control::{Control, ControlBase, ControlId, TextControlBase, ControlKind, EventResponse};

pub struct RadioButton {
    pub(crate) text_base: TextControlBase,
    /// The RadioGroup this button belongs to (0 = none).
    pub(crate) group: ControlId,
}

impl RadioButton {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base, group: 0 } }
}

impl Control for RadioButton {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::RadioButton }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y) = (p.x, p.y);
        let tc = crate::theme::colors();
        let selected = b.state != 0;
        let disabled = b.disabled;
        let hovered = b.hovered;
        let focused = b.focused;
        let sz = crate::theme::scale(crate::theme::radio_size());
        let r = sz / 2;

        // Background with hover feedback
        let bg = if disabled {
            crate::theme::darken(tc.control_bg, 10)
        } else if hovered {
            tc.control_hover
        } else {
            tc.control_bg
        };
        crate::draw::fill_rounded_rect(surface, x, y, sz, sz, r, bg);

        // Border (accent when selected, lighter on hover)
        let border_color = if selected {
            tc.accent
        } else if hovered && !disabled {
            tc.accent
        } else {
            tc.input_border
        };
        crate::draw::draw_rounded_border(surface, x, y, sz, sz, r, border_color);

        // Inner dot when selected
        if selected {
            let dot_color = if disabled { tc.text_disabled } else { tc.accent };
            let dot_inset = crate::theme::scale_i32(5);
            let dot_sz = crate::theme::scale(8);
            let dot_r = crate::theme::scale(4);
            crate::draw::fill_rounded_rect(surface, x + dot_inset, y + dot_inset, dot_sz, dot_sz, dot_r, dot_color);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, sz, sz, r, tc.accent);
        }

        // Label text
        let text_color = if disabled { tc.text_disabled } else { tc.text };
        if !self.text_base.text.is_empty() {
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            crate::draw::draw_text_sized(surface, x + sz as i32 + crate::theme::scale_i32(6), y + crate::theme::scale_i32(2), text_color, &self.text_base.text, font_size);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn set_radio_group(&mut self, group_id: ControlId) {
        self.group = group_id;
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.state = 1;
        // If inside a RadioGroup, post deferred deselection of siblings
        if self.group != 0 {
            crate::controls::radio_group::post_deselect(self.group, self.text_base.base.id);
        }
        EventResponse::CHANGED
    }
}
