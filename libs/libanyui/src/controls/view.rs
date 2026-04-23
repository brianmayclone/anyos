use crate::control::{Control, ControlBase, ControlKind};

pub struct View {
    pub(crate) base: ControlBase,
}

impl View {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for View {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::View
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        if self.base.color != 0 {
            let b = self.base();
            let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
            crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, b.color);
        }
        if self.base.drop_hover {
            draw_drop_hover_border(surface, ax, ay, self.base());
        }
    }
}

/// Draws a 2-pixel accent border around the control's bounds to indicate
/// it is the active drop target during a drag. Placed here (rather than on
/// `ControlBase`) so every container that wants to opt in can call it from
/// its own `render` override without paying the cost when no drag is active.
pub fn draw_drop_hover_border(
    surface: &crate::draw::Surface,
    ax: i32,
    ay: i32,
    b: &ControlBase,
) {
    const BORDER: u32 = 0xFF2E8BFF; // accent blue, fully opaque
    const THICKNESS: u32 = 2;
    let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
    // Top + bottom edges.
    crate::draw::fill_rect(surface, p.x, p.y, p.w, THICKNESS, BORDER);
    if p.h > THICKNESS {
        crate::draw::fill_rect(
            surface,
            p.x,
            p.y + p.h as i32 - THICKNESS as i32,
            p.w,
            THICKNESS,
            BORDER,
        );
    }
    // Left + right edges.
    crate::draw::fill_rect(surface, p.x, p.y, THICKNESS, p.h, BORDER);
    if p.w > THICKNESS {
        crate::draw::fill_rect(
            surface,
            p.x + p.w as i32 - THICKNESS as i32,
            p.y,
            THICKNESS,
            p.h,
            BORDER,
        );
    }
}
