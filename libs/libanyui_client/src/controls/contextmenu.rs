use crate::{Container, Control, Widget, lib, events, KIND_CONTEXT_MENU};
use crate::events::SelectionChangedEvent;

container_control!(ContextMenu, KIND_CONTEXT_MENU);

impl ContextMenu {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_CONTEXT_MENU, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn on_item_click(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_click_fn)(self.container.ctrl.id, thunk, ud);
    }
}
