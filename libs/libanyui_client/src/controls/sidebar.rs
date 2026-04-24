use crate::events::SelectionChangedEvent;
use crate::{events, lib, Container, Control, Widget, KIND_SIDEBAR};

container_control!(Sidebar, KIND_SIDEBAR);

impl Sidebar {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SIDEBAR, core::ptr::null(), 0);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }

    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
