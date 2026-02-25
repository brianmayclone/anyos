use crate::{Container, Control, Widget, lib, events, KIND_RADIO_GROUP};
use crate::events::SelectionChangedEvent;

container_control!(RadioGroup, KIND_RADIO_GROUP);

impl RadioGroup {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_RADIO_GROUP, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
