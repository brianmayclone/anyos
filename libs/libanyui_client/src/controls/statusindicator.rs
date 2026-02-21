use crate::{Control, Widget, lib, KIND_STATUS_INDICATOR};

leaf_control!(StatusIndicator, KIND_STATUS_INDICATOR);

impl StatusIndicator {
    pub fn new(label: &str) -> Self {
        let id = (lib().create_control)(KIND_STATUS_INDICATOR, label.as_ptr(), label.len() as u32);
        Self { ctrl: Control { id } }
    }
}
