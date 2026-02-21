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

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_progress(win, x, y, self.base.w, self.base.h, self.base.state, 100);
    }
}
