use crate::{Control, Widget, lib, events, KIND_SLIDER};
use crate::events::ValueChangedEvent;

leaf_control!(Slider, KIND_SLIDER);

impl Slider {
    pub fn new(value: u32) -> Self {
        let id = (lib().create_control)(KIND_SLIDER, core::ptr::null(), 0);
        (lib().set_state)(id, value);
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
