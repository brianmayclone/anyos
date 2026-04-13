use crate::{Control, Widget, lib, events, KIND_TEXT_AREA};
use crate::events::TextChangedEvent;

leaf_control!(TextArea, KIND_TEXT_AREA);

impl TextArea {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TEXT_AREA, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn set_read_only(&self, enabled: bool) {
        (lib().textarea_set_read_only)(self.ctrl.id, enabled as u32);
    }

    pub fn set_cursor(&self, pos: u32) {
        (lib().textarea_set_cursor)(self.ctrl.id, pos);
    }

    pub fn cursor(&self) -> u32 {
        (lib().textarea_get_cursor)(self.ctrl.id)
    }

    pub fn on_text_changed(&self, mut f: impl FnMut(&TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&TextChangedEvent { id }));
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Set the maximum text length in bytes (0 = unlimited).
    pub fn set_max_length(&self, max_len: u32) {
        (lib().textarea_set_max_length)(self.ctrl.id, max_len);
    }
}
