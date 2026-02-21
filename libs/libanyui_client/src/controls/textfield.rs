use crate::{Control, Widget, lib, events, KIND_TEXTFIELD};
use crate::events::TextChangedEvent;

leaf_control!(TextField, KIND_TEXTFIELD);

impl TextField {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TEXTFIELD, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn set_placeholder(&self, text: &str) {
        (lib().textfield_set_placeholder)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    pub fn set_prefix_icon(&self, icon_code: u32) {
        (lib().textfield_set_prefix)(self.ctrl.id, icon_code);
    }

    pub fn set_postfix_icon(&self, icon_code: u32) {
        (lib().textfield_set_postfix)(self.ctrl.id, icon_code);
    }

    pub fn set_password_mode(&self, enabled: bool) {
        (lib().textfield_set_password)(self.ctrl.id, enabled as u32);
    }

    pub fn on_text_changed(&self, mut f: impl FnMut(&TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&TextChangedEvent { id }));
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
