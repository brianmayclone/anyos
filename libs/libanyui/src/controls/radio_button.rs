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
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let selected = self.text_base.base.state != 0;
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let focused = self.text_base.base.focused;
        let sz = crate::theme::RADIO_SIZE;
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
            crate::draw::fill_rounded_rect(surface, x + 5, y + 5, 8, 8, 4, dot_color);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, sz, sz, r, tc.accent);
        }

        // Label text
        let text_color = if disabled { tc.text_disabled } else { tc.text };
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, x + sz as i32 + 6, y + 2, text_color, &self.text_base.text, self.text_base.text_style.font_size);
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
