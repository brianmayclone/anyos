use crate::{Container, Control, Widget, lib, events, KIND_CONTEXT_MENU};
use crate::events::SelectionChangedEvent;

container_control!(ContextMenu, KIND_CONTEXT_MENU);

impl ContextMenu {
    /// Create a context menu with pipe-separated item labels, e.g. `"Cut|Copy|Paste"`.
    pub fn new(items: &str) -> Self {
        let id = (lib().create_control)(KIND_CONTEXT_MENU, items.as_ptr(), items.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }

    /// Called when a menu item is clicked. `index` is the 0-based item position.
    pub fn on_item_click(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_click_fn)(self.container.ctrl.id, thunk, ud);
    }
}
