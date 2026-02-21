use crate::{Control, Widget, lib, events, KIND_TOGGLE};
use crate::events::CheckedChangedEvent;

leaf_control!(Toggle, KIND_TOGGLE);

impl Toggle {
    pub fn new(on: bool) -> Self {
        let id = (lib().create_control)(KIND_TOGGLE, core::ptr::null(), 0);
        if on {
            (lib().set_state)(id, 1);
        }
        Self { ctrl: Control { id } }
    }

    pub fn on_checked_changed(&self, mut f: impl FnMut(&CheckedChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let checked = Control::from_id(id).get_state() != 0;
            f(&CheckedChangedEvent { id, checked });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
