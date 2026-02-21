use crate::{Control, Widget, lib, KIND_LABEL};

leaf_control!(Label, KIND_LABEL);

impl Label {
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_LABEL, text.as_ptr(), text.len() as u32);
        Self { ctrl: Control { id } }
    }
}
