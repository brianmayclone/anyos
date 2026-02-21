use crate::control::{Control, ControlBase, ControlKind};

pub struct NavigationBar {
    pub(crate) base: ControlBase,
}

impl NavigationBar {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for NavigationBar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::NavigationBar }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, 0xFF2C2C2E);
        if !self.base.text.is_empty() {
            crate::uisys::render_label(win, x + 12, y + 8, &self.base.text, crate::uisys::color_text());
        }
    }
}
