use crate::control::{Control, ControlBase, ControlKind, TextControlBase};

pub struct Window {
    pub(crate) tb: TextControlBase,
}

impl Window {
    pub fn new(base: ControlBase) -> Self {
        Self {
            tb: TextControlBase::new(base),
        }
    }

    pub fn new_with_title(base: ControlBase, title: &[u8]) -> Self {
        Self {
            tb: TextControlBase::new(base).with_text(title),
        }
    }
}

impl Control for Window {
    fn base(&self) -> &ControlBase {
        &self.tb.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.tb.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Window
    }
    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.tb)
    }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.tb)
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let bg = if b.color_set {
            b.color
        } else {
            crate::theme::colors().window_bg
        };
        if (bg >> 24) != 0 {
            crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, bg);
        }
    }
}
