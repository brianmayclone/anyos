use crate::{Control, Widget, lib, events, KIND_LIST_BOX};
use crate::events::SelectionChangedEvent;

leaf_control!(ListBox, KIND_LIST_BOX);

impl ListBox {
    /// Create a list box.  Items are pipe-separated, e.g. `"Item A|Item B|Item C"`.
    /// For multi-select mode, prefix with `"multi:"`, e.g. `"multi:Item A|Item B"`.
    pub fn new(items: &str) -> Self {
        let id = (lib().create_control)(KIND_LIST_BOX, items.as_ptr(), items.len() as u32);
        Self { ctrl: Control { id } }
    }

    /// Replace the item list (pipe-separated).
    pub fn set_items(&self, items: &str) {
        self.ctrl.set_text(items);
    }

    /// Get the currently selected index (0-based, single-select mode).
    pub fn selected_index(&self) -> u32 {
        self.ctrl.get_state()
    }

    /// Set the selected index (single-select mode).
    pub fn set_selected_index(&self, idx: u32) {
        self.ctrl.set_state(idx);
    }

    /// Register a callback for when the selection changes.
    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
