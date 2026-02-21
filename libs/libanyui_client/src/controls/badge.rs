use crate::{Control, Widget, lib, KIND_BADGE};

leaf_control!(Badge, KIND_BADGE);

impl Badge {
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_BADGE, text.as_ptr(), text.len() as u32);
        Self { ctrl: Control { id } }
    }
}
