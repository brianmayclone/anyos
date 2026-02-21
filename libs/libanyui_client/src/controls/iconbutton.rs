use crate::{Control, Widget, lib, events, KIND_ICON_BUTTON};
use crate::events::ClickEvent;

leaf_control!(IconButton, KIND_ICON_BUTTON);

impl IconButton {
    pub fn new(icon_text: &str) -> Self {
        let id = (lib().create_control)(KIND_ICON_BUTTON, icon_text.as_ptr(), icon_text.len() as u32);
        Self { ctrl: Control { id } }
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
