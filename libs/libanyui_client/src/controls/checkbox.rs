use crate::{Control, Widget, lib, events, KIND_CHECKBOX};
use crate::events::CheckedChangedEvent;

leaf_control!(Checkbox, KIND_CHECKBOX);

impl Checkbox {
    pub fn new(label: &str) -> Self {
        let id = (lib().create_control)(KIND_CHECKBOX, label.as_ptr(), label.len() as u32);
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
