use crate::{Control, Widget, lib, events, KIND_BUTTON};
use crate::events::ClickEvent;

leaf_control!(Button, KIND_BUTTON);

impl Button {
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_BUTTON, text.as_ptr(), text.len() as u32);
        Self { ctrl: Control { id } }
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
