use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct ColorWell {
    pub(crate) base: ControlBase,
}

impl ColorWell {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for ColorWell {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ColorWell }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let corner = crate::theme::scale(4);
        let color = if b.color != 0 { b.color } else { 0xFFFF0000 };
        crate::draw::fill_rounded_rect(surface, p.x, p.y, p.w, p.h, corner, color);
        crate::draw::draw_rounded_border(surface, p.x, p.y, p.w, p.h, corner, crate::theme::colors().card_border);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
