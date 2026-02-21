use crate::{Control, Widget, lib, KIND_IMAGE_VIEW};

leaf_control!(ImageView, KIND_IMAGE_VIEW);

impl ImageView {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_IMAGE_VIEW, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }
}
