use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct ColorWell {
    pub(crate) base: ControlBase,
}

impl ColorWell {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for ColorWell {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::ColorWell
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let corner = crate::theme::scale(6);
        let color = if b.color != 0 { b.color } else { 0xFFFF0000 };
        let palette = crate::controls::chrome::neutral_palette(b.hovered, false, b.disabled);
        crate::controls::chrome::draw_surface(surface, p.x, p.y, p.w, p.h, corner, palette);
        if p.w > 6 && p.h > 6 {
            crate::draw::fill_rounded_rect(
                surface,
                p.x + 3,
                p.y + 3,
                p.w - 6,
                p.h - 6,
                corner.saturating_sub(2),
                color,
            );
            crate::draw::draw_rounded_border(
                surface,
                p.x + 3,
                p.y + 3,
                p.w - 6,
                p.h - 6,
                corner.saturating_sub(2),
                crate::theme::with_alpha(0xFFFFFFFF, 42),
            );
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
