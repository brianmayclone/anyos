use crate::{Control, Widget, lib, events, KIND_COLOR_WELL};
use crate::events::ColorSelectedEvent;

leaf_control!(ColorWell, KIND_COLOR_WELL);

impl ColorWell {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_COLOR_WELL, core::ptr::null(), 0);
        Self { ctrl: Control { id } }
    }

    pub fn set_selected_color(&self, color: u32) {
        (lib().set_state)(self.ctrl.id, color);
    }

    pub fn get_selected_color(&self) -> u32 {
        (lib().get_state)(self.ctrl.id)
    }

    pub fn on_color_selected(&self, mut f: impl FnMut(&ColorSelectedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let color = Control::from_id(id).get_state();
            f(&ColorSelectedEvent { id, color });
        });
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
