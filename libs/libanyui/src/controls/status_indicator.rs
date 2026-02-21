use crate::control::{Control, ControlBase, ControlKind};

pub struct StatusIndicator {
    pub(crate) base: ControlBase,
}

impl StatusIndicator {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for StatusIndicator {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::StatusIndicator }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_status(win, x, y, self.base.state as u8, &self.base.text);
    }
}
