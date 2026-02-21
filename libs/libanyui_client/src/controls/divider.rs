use crate::{Control, Widget, lib, KIND_DIVIDER};

leaf_control!(Divider, KIND_DIVIDER);

impl Divider {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_DIVIDER, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }
}
