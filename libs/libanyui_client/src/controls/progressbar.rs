use crate::{Control, Widget, lib, KIND_PROGRESS_BAR};

leaf_control!(ProgressBar, KIND_PROGRESS_BAR);

impl ProgressBar {
    pub fn new(value: u32) -> Self {
        let id = (lib().create_control)(KIND_PROGRESS_BAR, core::ptr::null(), 0);
        (lib().set_state)(id, value);
        Self { ctrl: Control { id } }
    }
}
