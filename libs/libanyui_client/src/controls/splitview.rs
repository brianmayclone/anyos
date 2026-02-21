use crate::{Container, Control, Widget, lib, events, KIND_SPLIT_VIEW};
use crate::events::ValueChangedEvent;

container_control!(SplitView, KIND_SPLIT_VIEW);

impl SplitView {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SPLIT_VIEW, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn set_orientation(&self, orientation: u32) {
        (lib().set_orientation)(self.container.ctrl.id, orientation);
    }

    pub fn set_split_ratio(&self, ratio: u32) {
        (lib().set_split_ratio)(self.container.ctrl.id, ratio);
    }

    pub fn set_min_split(&self, min_ratio: u32) {
        (lib().set_min_split)(self.container.ctrl.id, min_ratio);
    }

    pub fn set_max_split(&self, max_ratio: u32) {
        (lib().set_max_split)(self.container.ctrl.id, max_ratio);
    }

    pub fn on_split_changed(&self, mut f: impl FnMut(&ValueChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let value = Control::from_id(id).get_state();
            f(&ValueChangedEvent { id, value });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
