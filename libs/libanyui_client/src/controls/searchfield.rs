use crate::{Control, Widget, lib, events, KIND_SEARCH_FIELD};
use crate::events::{TextChangedEvent, SubmitEvent};

leaf_control!(SearchField, KIND_SEARCH_FIELD);

impl SearchField {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SEARCH_FIELD, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn set_placeholder(&self, text: &str) {
        (lib().textfield_set_placeholder)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    /// Called when text content changes.
    pub fn on_text_changed(&self, mut f: impl FnMut(&TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&TextChangedEvent { id }));
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Called when the user presses Enter (submits the search).
    pub fn on_submit(&self, mut f: impl FnMut(&SubmitEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&SubmitEvent { id }));
        (lib().on_submit_fn)(self.ctrl.id, thunk, ud);
    }
}
