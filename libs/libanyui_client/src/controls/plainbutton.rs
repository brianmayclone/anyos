use crate::{Control, Widget, lib, events, KIND_PLAIN_BUTTON};
use crate::events::ClickEvent;
use crate::icon::IconType;

leaf_control!(PlainButton, KIND_PLAIN_BUTTON);

impl PlainButton {
    /// Create a new borderless plain button (no background, icon-only).
    pub fn new(text: &str) -> Self {
        let id = (lib().create_control)(KIND_PLAIN_BUTTON, text.as_ptr(), text.len() as u32);
        Self { ctrl: Control { id } }
    }

    /// Set a system SVG icon by name from ico.pak.
    pub fn set_system_icon(&self, name: &str, icon_type: IconType, color: u32, size: u32) {
        if let Some(icon) = crate::icon::Icon::system(name, icon_type, color, size) {
            (lib().iconbutton_set_pixels)(self.ctrl.id, icon.pixels.as_ptr(), icon.width, icon.height);
        }
    }

    /// Set raw ARGB pixel data as the icon.
    pub fn set_pixels(&self, pixels: &[u32], w: u32, h: u32) {
        (lib().iconbutton_set_pixels)(self.ctrl.id, pixels.as_ptr(), w, h);
    }

    /// Set which built-in pixel-art icon to display (legacy compatibility).
    pub fn set_icon(&self, icon_id: u32) {
        self.ctrl.set_state(icon_id);
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
