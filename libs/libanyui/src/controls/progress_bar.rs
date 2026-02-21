use crate::control::{Control, ControlBase, ControlKind};

pub struct ProgressBar {
    pub(crate) base: ControlBase,
}

impl ProgressBar {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for ProgressBar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ProgressBar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.base.w, self.base.h, self.base.h / 2, tc.control_bg);
        let val = self.base.state.min(100);
        let fill_w = (self.base.w as u64 * val as u64 / 100) as u32;
        if fill_w > 0 {
            crate::draw::fill_rounded_rect(surface, x, y, fill_w, self.base.h, self.base.h / 2, tc.accent);
        }
    }
}
