use crate::events::SelectionChangedEvent;
use crate::{Control, Widget, events, lib, KIND_COMBO_BOX};

leaf_control!(ComboBox, KIND_COMBO_BOX);

impl ComboBox {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_COMBO_BOX, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn set_items(&self, items: &str) {
        (lib().combobox_set_items)(self.ctrl.id, items.as_ptr(), items.len() as u32);
    }

    pub fn set_placeholder(&self, text: &str) {
        (lib().combobox_set_placeholder)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    pub fn set_editable(&self, editable: bool) {
        (lib().combobox_set_editable)(self.ctrl.id, editable as u32);
    }

    pub fn editable(&self) -> bool {
        (lib().combobox_get_editable)(self.ctrl.id) != 0
    }

    pub fn set_selected_index(&self, idx: Option<u32>) {
        (lib().combobox_set_selected_index)(self.ctrl.id, idx.unwrap_or(u32::MAX));
    }

    pub fn selected_index(&self) -> Option<u32> {
        let idx = (lib().combobox_get_selected_index)(self.ctrl.id);
        if idx == u32::MAX { None } else { Some(idx) }
    }

    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            f(&SelectionChangedEvent {
                id,
                index: (lib().combobox_get_selected_index)(id),
            })
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
