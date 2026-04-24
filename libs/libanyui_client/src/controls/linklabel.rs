use crate::events::ClickEvent;
use crate::{events, lib, Control, Widget, KIND_LINK_LABEL};

leaf_control!(LinkLabel, KIND_LINK_LABEL);

impl LinkLabel {
    /// Create a clickable text link.
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_LINK_LABEL, text.as_ptr(), text.len() as u32);
        Self {
            ctrl: Control { id },
        }
    }

    /// Register a closure to be called when the link is activated.
    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
