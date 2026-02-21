use crate::{Control, Widget, lib, events, KIND_SEGMENTED};
use crate::events::SelectionChangedEvent;

leaf_control!(SegmentedControl, KIND_SEGMENTED);

impl SegmentedControl {
    /// Create a segmented control. Labels are pipe-separated, e.g. "Tab 1|Tab 2|Tab 3".
    pub fn new(labels: &str) -> Self {
        let id = (lib().create_control)(KIND_SEGMENTED, labels.as_ptr(), labels.len() as u32);
        Self { ctrl: Control { id } }
    }

    /// Register a callback for when the active segment changes.
    pub fn on_active_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Connect panel views to this segmented control for automatic tab switching.
    /// Shows panels[active_index], hides all others. Call after adding panels to parent.
    pub fn connect_panels(&self, panels: &[&impl crate::Widget]) {
        let ids: alloc::vec::Vec<u32> = panels.iter().map(|p| p.id()).collect();
        // Show first panel, hide rest
        for (i, &pid) in ids.iter().enumerate() {
            Control::from_id(pid).set_visible(i == 0);
        }
        // Register change handler to auto-switch
        let (thunk, ud) = events::register(move |id, _| {
            let active = Control::from_id(id).get_state() as usize;
            for (i, &pid) in ids.iter().enumerate() {
                Control::from_id(pid).set_visible(i == active);
            }
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
