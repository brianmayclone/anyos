use crate::{Container, Control, Widget, lib, KIND_VIEW};

container_control!(View, KIND_VIEW);

impl View {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_VIEW, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }
}
