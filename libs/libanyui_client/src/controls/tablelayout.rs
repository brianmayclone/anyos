use crate::{Container, Control, Widget, lib, KIND_TABLE_LAYOUT};

container_control!(TableLayout, KIND_TABLE_LAYOUT);

impl TableLayout {
    pub fn new(columns: u32) -> Self {
        let id = (lib().create_control)(KIND_TABLE_LAYOUT, core::ptr::null(), 0);
        (lib().set_columns)(id, columns);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn set_columns(&self, columns: u32) {
        (lib().set_columns)(self.container.ctrl.id, columns);
    }

    pub fn set_row_height(&self, row_height: u32) {
        (lib().set_row_height)(self.container.ctrl.id, row_height);
    }
}
