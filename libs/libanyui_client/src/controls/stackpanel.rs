use crate::{Container, Control, Widget, lib, KIND_STACK_PANEL, ORIENTATION_VERTICAL, ORIENTATION_HORIZONTAL};

container_control!(StackPanel, KIND_STACK_PANEL);

impl StackPanel {
    pub fn new(orientation: u32) -> Self {
        let id = (lib().create_control)(KIND_STACK_PANEL, core::ptr::null(), 0);
        (lib().set_orientation)(id, orientation);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn vertical() -> Self { Self::new(ORIENTATION_VERTICAL) }
    pub fn horizontal() -> Self { Self::new(ORIENTATION_HORIZONTAL) }

    pub fn set_orientation(&self, orientation: u32) {
        (lib().set_orientation)(self.container.ctrl.id, orientation);
    }
}
