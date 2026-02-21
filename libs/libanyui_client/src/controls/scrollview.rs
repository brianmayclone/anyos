use crate::{Container, Control, Widget, lib, events, KIND_SCROLL_VIEW};
use crate::events::ScrollChangedEvent;

container_control!(ScrollView, KIND_SCROLL_VIEW);

impl ScrollView {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SCROLL_VIEW, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn on_scroll(&self, mut f: impl FnMut(&ScrollChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let offset = Control::from_id(id).get_state();
            f(&ScrollChangedEvent { id, offset });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
