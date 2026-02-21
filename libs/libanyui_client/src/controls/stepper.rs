use crate::{Control, Widget, lib, events, KIND_STEPPER};
use crate::events::ValueChangedEvent;

leaf_control!(Stepper, KIND_STEPPER);

impl Stepper {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_STEPPER, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn on_value_changed(&self, mut f: impl FnMut(&ValueChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let value = Control::from_id(id).get_state();
            f(&ValueChangedEvent { id, value });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
