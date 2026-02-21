use crate::{Container, Control, Widget, lib, KIND_GROUP_BOX};

container_control!(GroupBox, KIND_GROUP_BOX);

impl GroupBox {
    pub fn new(title: &str) -> Self {
        let id = (lib().create_control)(KIND_GROUP_BOX, title.as_ptr(), title.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }
}
