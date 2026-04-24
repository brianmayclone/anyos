use crate::events::TextChangedEvent;
use crate::{events, lib, Control, Widget, KIND_TEXT_AREA};

leaf_control!(TextArea, KIND_TEXT_AREA);

impl TextArea {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TEXT_AREA, core::ptr::null(), 0);
        Self {
            ctrl: Control { id },
        }
    }

    pub fn set_read_only(&self, enabled: bool) {
        (lib().textarea_set_read_only)(self.ctrl.id, enabled as u32);
    }

    pub fn select_all(&self) {
        (lib().textarea_select_all)(self.ctrl.id);
    }

    pub fn set_cursor(&self, pos: u32) {
        (lib().textarea_set_cursor)(self.ctrl.id, pos);
    }

    pub fn cursor(&self) -> u32 {
        (lib().textarea_get_cursor)(self.ctrl.id)
    }

    pub fn set_selection(&self, start: u32, end: u32) {
        (lib().textarea_set_selection)(self.ctrl.id, start, end);
    }

    pub fn selection(&self) -> Option<(u32, u32)> {
        let mut start = 0;
        let mut end = 0;
        let ok = (lib().textarea_get_selection)(self.ctrl.id, &mut start, &mut end);
        if ok != 0 {
            Some((start, end))
        } else {
            None
        }
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
