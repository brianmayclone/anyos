use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Slider {
    pub(crate) base: ControlBase,
    dragging: bool,
}

impl Slider {
    pub fn new(base: ControlBase) -> Self { Self { base, dragging: false } }

    fn value_from_x(&self, local_x: i32) -> u32 {
        let w = self.base.w as i32;
        if w <= 0 { return 0; }
        let clamped = local_x.max(0).min(w);
        ((clamped as u32) * 100) / (w as u32)
    }
}

impl Control for Slider {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Slider }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let focused = b.focused;
        let track_h = crate::theme::scale(4);
        let track_r = crate::theme::scale(2);
        let track_y = y + (h as i32 - track_h as i32) / 2;

        // Track background
        crate::draw::fill_rounded_rect(surface, x, track_y, w, track_h, track_r, tc.control_bg);

        // Filled portion
        let val = b.state.min(100);
        let fill_w = (w as u64 * val as u64 / 100) as u32;
        if fill_w > 0 {
            let accent = if disabled { tc.toggle_off } else { tc.accent };
            crate::draw::fill_rounded_rect(surface, x, track_y, fill_w, track_h, track_r, accent);
        }

        // Thumb
        let thumb_sz = crate::theme::scale(18);
        let thumb_r = crate::theme::scale(9);
        let thumb_x = x + fill_w as i32 - thumb_r as i32;
        let thumb_y = y + (h as i32 - thumb_sz as i32) / 2;
        let thumb_color = if disabled { crate::theme::darken(tc.toggle_thumb, 30) } else { tc.toggle_thumb };

        // 1px shadow under thumb
        if !disabled {
            crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y + 1, thumb_sz, thumb_sz, thumb_r, crate::theme::with_alpha(0xFF000000, 20));
        }
        crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y, thumb_sz, thumb_sz, thumb_r, thumb_color);

        // Subtle thumb border
        if !disabled {
            crate::draw::draw_rounded_border(surface, thumb_x, thumb_y, thumb_sz, thumb_sz, thumb_r, crate::theme::with_alpha(0xFF000000, 15));
        }

        // Focus ring on thumb
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, thumb_x, thumb_y, thumb_sz, thumb_sz, thumb_r, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.base.disabled }

    fn handle_mouse_down(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.dragging = true;
        self.base.state = self.value_from_x(lx);
        EventResponse::CHANGED
    }

    fn handle_mouse_move(&mut self, lx: i32, _ly: i32) -> EventResponse {
        if self.dragging {
            self.base.state = self.value_from_x(lx);
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_up(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if self.dragging {
            self.dragging = false;
            self.base.state = self.value_from_x(lx);
            EventResponse::CHANGED
        } else {
            EventResponse::CONSUMED
        }
    }
}
