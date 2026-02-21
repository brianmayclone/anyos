use crate::{Control, Widget, lib, events, KIND_TEXT_AREA};
use crate::events::TextChangedEvent;

leaf_control!(TextArea, KIND_TEXT_AREA);

impl TextArea {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TEXT_AREA, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn on_text_changed(&self, mut f: impl FnMut(&TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&TextChangedEvent { id }));
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
