use crate::{Control, Widget, lib, KIND_LABEL};

/// Text alignment constants (set via `set_state()`).
pub const TEXT_ALIGN_LEFT: u32 = 0;
pub const TEXT_ALIGN_CENTER: u32 = 1;
pub const TEXT_ALIGN_RIGHT: u32 = 2;

leaf_control!(Label, KIND_LABEL);

impl Label {
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_LABEL, text.as_ptr(), text.len() as u32);
        Self { ctrl: Control { id } }
    }

    /// Set text alignment: TEXT_ALIGN_LEFT (0), TEXT_ALIGN_CENTER (1), TEXT_ALIGN_RIGHT (2).
    pub fn set_text_align(&self, align: u32) {
        self.set_state(align);
    }
}
