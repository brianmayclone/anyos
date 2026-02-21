use crate::control::{Control, ControlBase, ControlKind};

pub struct Window {
    pub(crate) base: ControlBase,
}

impl Window {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Window {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Window }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let bg = crate::uisys::color_window_bg();
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, bg);
    }
}
