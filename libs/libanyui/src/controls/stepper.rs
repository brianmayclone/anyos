use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

fn format_u32(mut v: u32) -> [u8; 10] {
    let mut buf = [0u8; 10];
    if v == 0 { buf[0] = b'0'; return buf; }
    let mut i = 9;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i -= 1; }
    // Shift to start
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
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, crate::theme::BUTTON_CORNER, tc.control_bg);
        crate::draw::draw_text(surface, x + 6, y + 6, tc.text, b"-");
        let val_text = format_u32(self.text_base.base.state);
        let (vw, _) = crate::draw::text_size(&val_text);
        let cx = x + (self.text_base.base.w as i32 - vw as i32) / 2;
        crate::draw::draw_text(surface, cx, y + 6, tc.text, &val_text);
        crate::draw::draw_text(surface, x + self.text_base.base.w as i32 - 16, y + 6, tc.text, b"+");
    }

    fn is_interactive(&self) -> bool { true }

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
