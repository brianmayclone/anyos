use crate::control::{Control, ControlBase, ControlKind};

pub struct Divider {
    pub(crate) base: ControlBase,
}

impl Divider {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Divider {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Divider }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        if self.base.w > self.base.h {
            crate::uisys::render_divider_h(win, x, y, self.base.w);
        } else {
            crate::uisys::render_divider_v(win, x, y, self.base.h);
        }
    }
}
