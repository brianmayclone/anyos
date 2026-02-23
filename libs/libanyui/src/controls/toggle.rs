use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Toggle {
    pub(crate) text_base: TextControlBase,
}

impl Toggle {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Toggle {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Toggle }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let on = self.text_base.base.state != 0;
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let focused = self.text_base.base.focused;

        let track_color = if disabled {
            tc.toggle_off
        } else if on {
            if hovered { crate::theme::lighten(tc.toggle_on, 15) } else { tc.toggle_on }
        } else {
            if hovered { crate::theme::lighten(tc.toggle_off, 10) } else { tc.toggle_off }
        };

        // Track
        let tw = crate::theme::TOGGLE_WIDTH;
        let th = crate::theme::TOGGLE_HEIGHT;
        crate::draw::fill_rounded_rect(surface, x, y, tw, th, th / 2, track_color);

        // Thumb with subtle bottom shadow
        let thumb_sz = crate::theme::TOGGLE_THUMB_SIZE;
        let thumb_x = if on { x + (tw - thumb_sz - 2) as i32 } else { x + 2 };
        let thumb_y = y + 2;
        let thumb_color = if disabled { crate::theme::darken(tc.toggle_thumb, 30) } else { tc.toggle_thumb };

        // 1px shadow under thumb
        crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y + 1, thumb_sz, thumb_sz, thumb_sz / 2, crate::theme::with_alpha(0xFF000000, 25));
        // Thumb
        crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y, thumb_sz, thumb_sz, thumb_sz / 2, thumb_color);

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, tw, th, th / 2, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }
}
