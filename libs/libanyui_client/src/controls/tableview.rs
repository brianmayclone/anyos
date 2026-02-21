use crate::{Container, Control, Widget, lib, events, KIND_TABLE_VIEW};
use crate::events::SelectionChangedEvent;

container_control!(TableView, KIND_TABLE_VIEW);

impl TableView {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TABLE_VIEW, core::ptr::null(), 0);
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
