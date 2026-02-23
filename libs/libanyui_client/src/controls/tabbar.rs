use crate::{Container, Control, Widget, lib, events, KIND_TAB_BAR};
use crate::events::SelectionChangedEvent;

container_control!(TabBar, KIND_TAB_BAR);

impl TabBar {
    pub fn new(labels: &str) -> Self {
        let id = (lib().create_control)(KIND_TAB_BAR, labels.as_ptr(), labels.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }

    pub fn on_active_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }

    /// Called when a tab's close button (×) is clicked. `index` is the 0-based tab position.
    pub fn on_tab_close(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_submit_fn)(self.container.ctrl.id, thunk, ud);
    }

    /// Connect panel views to this tab bar for automatic tab switching.
    pub fn connect_panels(&self, panels: &[&impl crate::Widget]) {
        let ids: alloc::vec::Vec<u32> = panels.iter().map(|p| p.id()).collect();
        for (i, &pid) in ids.iter().enumerate() {
            Control::from_id(pid).set_visible(i == 0);
        }
        let (thunk, ud) = events::register(move |id, _| {
            let active = Control::from_id(id).get_state() as usize;
            for (i, &pid) in ids.iter().enumerate() {
                Control::from_id(pid).set_visible(i == active);
            }
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
