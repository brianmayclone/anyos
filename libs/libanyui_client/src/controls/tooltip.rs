use crate::{Container, Control, Widget, lib, KIND_TOOLTIP};

container_control!(Tooltip, KIND_TOOLTIP);

impl Tooltip {
    /// Create a tooltip container. The tooltip text is shown when hovering over children.
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_TOOLTIP, text.as_ptr(), text.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }
}
