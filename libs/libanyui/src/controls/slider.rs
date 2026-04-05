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
        let track_h = crate::theme::scale(6);
        let track_r = crate::theme::scale(3);
        let track_y = y + (h as i32 - track_h as i32) / 2;

        crate::controls::chrome::draw_surface(
            surface,
            x,
            track_y,
            w,
            track_h,
            track_r,
            crate::controls::chrome::field_palette(0, b.hovered, false, disabled),
        );

        let val = b.state.min(100);
        let fill_w = (w.saturating_sub(4) as u64 * val as u64 / 100) as u32;
        if fill_w > 0 {
            let accent = if disabled { tc.toggle_off } else { tc.accent };
            crate::controls::chrome::draw_surface(
                surface,
                x + 2,
                track_y + 2,
                fill_w,
                track_h.saturating_sub(4),
                track_r.saturating_sub(2),
                crate::controls::chrome::accent_palette(accent, false, false, disabled),
            );
        }

        let thumb_w = crate::theme::scale(14);
        let thumb_h = crate::theme::scale(14);
        let thumb_r = crate::theme::scale(5);
        let usable = w.saturating_sub(thumb_w);
        let thumb_x = x + (usable as u64 * val as u64 / 100) as i32;
        let thumb_y = y + (h as i32 - thumb_h as i32) / 2;
        let thumb_palette = crate::controls::chrome::neutral_palette(b.hovered || self.dragging, self.dragging, disabled);

        if focused && !disabled {
            crate::controls::chrome::draw_focus(surface, thumb_x, thumb_y, thumb_w, thumb_h, thumb_r, thumb_palette);
        }
        crate::controls::chrome::draw_surface(surface, thumb_x, thumb_y, thumb_w, thumb_h, thumb_r, thumb_palette);
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

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let step = 5u32;
        match keycode {
            crate::control::KEY_LEFT | crate::control::KEY_DOWN => {
                self.base.state = self.base.state.saturating_sub(step);
                EventResponse::CHANGED
            }
            crate::control::KEY_RIGHT | crate::control::KEY_UP => {
                self.base.state = (self.base.state + step).min(100);
                EventResponse::CHANGED
            }
            _ => EventResponse::IGNORED,
        }
    }
}
