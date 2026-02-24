use crate::{Container, Control, Widget, lib, KIND_RADIO_GROUP};

container_control!(RadioGroup, KIND_RADIO_GROUP);

impl RadioGroup {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_RADIO_GROUP, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }
}
