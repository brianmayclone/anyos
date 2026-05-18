use crate::events::SelectionChangedEvent;
use crate::{events, lib, Container, Control, Widget, KIND_TAB_BAR};

container_control!(TabBar, KIND_TAB_BAR);

impl TabBar {
    pub fn new(labels: &str) -> Self {
        let id = (lib().create_control)(KIND_TAB_BAR, labels.as_ptr(), labels.len() as u32);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
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

    /// Show or hide the "+" (new-tab) button on the right side of the tab bar.
    pub fn show_plus(&self, show: bool) {
        (lib().tabbar_show_plus)(self.container.ctrl.id, if show { 1 } else { 0 });
    }

    /// Set an ARGB icon for a tab. Pass a 16x16 favicon for browser-style tabs.
    pub fn set_tab_icon(&self, index: u32, pixels: &[u32], w: u32, h: u32) {
        if pixels.is_empty() || w == 0 || h == 0 {
            (lib().tabbar_set_tab_icon)(self.container.ctrl.id, index, core::ptr::null(), 0, 0);
            return;
        }
        (lib().tabbar_set_tab_icon)(self.container.ctrl.id, index, pixels.as_ptr(), w, h);
    }

    /// Clear the icon for a tab.
    pub fn clear_tab_icon(&self, index: u32) {
        (lib().tabbar_set_tab_icon)(self.container.ctrl.id, index, core::ptr::null(), 0, 0);
    }

    /// Set one visual style property. Keys are `STYLE_*` constants.
    pub fn set_style(&self, key: u32, value: u32) {
        self.container.ctrl.set_style(key, value);
    }

    /// Register a callback for double-click on a tab (e.g. for rename).
    pub fn on_double_click(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        self.container.ctrl.on_double_click_raw(thunk, ud);
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
