use crate::{Container, Control, Widget, lib, KIND_TOOLBAR};

container_control!(Toolbar, KIND_TOOLBAR);

impl Toolbar {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TOOLBAR, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }
}
