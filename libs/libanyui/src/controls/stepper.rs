use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

fn format_u32(mut v: u32) -> [u8; 10] {
    let mut buf = [0u8; 10];
    if v == 0 { buf[0] = b'0'; return buf; }
    let mut i = 9;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i -= 1; }
    let start = i + 1;
    let len = 10 - start;
    let mut out = [0u8; 10];
    out[..len].copy_from_slice(&buf[start..]);
    out
}

pub struct Stepper {
    pub(crate) text_base: TextControlBase,
}

impl Stepper {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Stepper {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Stepper }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let focused = b.focused;
        let corner = crate::theme::button_corner();

        // Overall background with depth
        let bg = if disabled { crate::theme::darken(tc.control_bg, 10) } else { tc.control_bg };
        crate::draw::draw_bottom_shadow(surface, x, y, w, h, corner, crate::theme::darken(bg, 20));
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        crate::draw::draw_top_highlight(surface, x, y, w, corner, crate::theme::lighten(bg, 10));

        let text_color = if disabled { tc.text_disabled } else { tc.text };
        let btn_color = if disabled { tc.text_disabled } else { tc.text_secondary };
        let fs = crate::draw::scale_font(14);
        let y_pad = crate::theme::scale_i32(6);

        // Minus button
        crate::draw::draw_text_sized(surface, x + crate::theme::scale_i32(10), y + y_pad, btn_color, b"\xe2\x88\x92", fs);

        // Value display
        let val_text = format_u32(b.state);
        let (vw, _) = crate::draw::text_size_at(&val_text, fs);
        let cx = x + (w as i32 - vw as i32) / 2;
        crate::draw::draw_text_sized(surface, cx, y + y_pad, text_color, &val_text, fs);

        // Plus button
        crate::draw::draw_text_sized(surface, x + w as i32 - crate::theme::scale_i32(18), y + y_pad, btn_color, b"+", fs);

        // Separators
        let sep_x_left = crate::theme::scale_i32(28);
        let sep_x_right = crate::theme::scale_i32(29);
        let sep_pad = crate::theme::scale_i32(4);
        let sep_h = if h > (sep_pad as u32 * 2) { h - sep_pad as u32 * 2 } else { 1 };
        crate::draw::fill_rect(surface, x + sep_x_left, y + sep_pad, 1, sep_h, tc.separator);
        crate::draw::fill_rect(surface, x + w as i32 - sep_x_right, y + sep_pad, 1, sep_h, tc.separator);

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let half = self.text_base.base.w as i32 / 2;
        if lx < half {
            if self.text_base.base.state > 0 { self.text_base.base.state -= 1; }
        } else {
            self.text_base.base.state += 1;
        }
        EventResponse::CHANGED
    }
}
