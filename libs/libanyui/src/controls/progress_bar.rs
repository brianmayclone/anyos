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
        let w = self.base.w;
        let h = self.base.h;
        let tc = crate::theme::colors();
        let r = h / 2;

        // Track with subtle inner shadow (1px darker top line)
        crate::draw::fill_rounded_rect(surface, x, y, w, h, r, tc.control_bg);
        crate::draw::draw_top_highlight(surface, x, y, w, r, crate::theme::darken(tc.control_bg, 8));

        // Filled portion with accent
        let val = self.base.state.min(100);
        let fill_w = (w as u64 * val as u64 / 100) as u32;
        if fill_w > 0 {
            crate::draw::fill_rounded_rect(surface, x, y, fill_w, h, r, tc.accent);
            // Subtle highlight on the filled portion
            crate::draw::draw_top_highlight(surface, x, y, fill_w, r, crate::theme::lighten(tc.accent, 20));
        }
    }
}
