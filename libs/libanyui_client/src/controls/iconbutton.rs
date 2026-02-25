use crate::{Control, Widget, lib, events, KIND_ICON_BUTTON};
use crate::events::ClickEvent;
use crate::icon::IconType;

// ── Legacy Icon ID constants (kept for backwards compatibility) ──────
pub const ICON_NEW_FILE: u32 = 1;
pub const ICON_FOLDER_OPEN: u32 = 2;
pub const ICON_SAVE: u32 = 3;
pub const ICON_SAVE_ALL: u32 = 4;
pub const ICON_BUILD: u32 = 5;
pub const ICON_PLAY: u32 = 6;
pub const ICON_STOP: u32 = 7;
pub const ICON_SETTINGS: u32 = 8;
pub const ICON_FILES: u32 = 9;
pub const ICON_GIT_BRANCH: u32 = 10;
pub const ICON_SEARCH: u32 = 11;
pub const ICON_REFRESH: u32 = 12;

leaf_control!(IconButton, KIND_ICON_BUTTON);

impl IconButton {
    pub fn new(icon_text: &str) -> Self {
        let id = (lib().create_control)(KIND_ICON_BUTTON, icon_text.as_ptr(), icon_text.len() as u32);
        Self { ctrl: Control { id } }
    }

    /// Set a system SVG icon by name from ico.pak.
    ///
    /// Renders the icon at the given size and color via libimage's SVG rasterizer,
    /// with caching so repeated calls are free.
    ///
    /// # Example
    /// ```rust
    /// btn.set_system_icon("device-floppy", IconType::Outline, 0xFFCCCCCC, 18);
    /// ```
    pub fn set_system_icon(&self, name: &str, icon_type: IconType, color: u32, size: u32) {
        if let Some(icon) = crate::icon::Icon::system(name, icon_type, color, size) {
            (lib().iconbutton_set_pixels)(self.ctrl.id, icon.pixels.as_ptr(), icon.width, icon.height);
        }
    }

    /// Set raw ARGB pixel data as the icon.
    ///
    /// This calls the server-side IconButton's `set_icon_pixels`, so the icon
    /// gets proper hover, pressed, disabled, and focus-ring rendering for free.
    pub fn set_pixels(&self, pixels: &[u32], w: u32, h: u32) {
        (lib().iconbutton_set_pixels)(self.ctrl.id, pixels.as_ptr(), w, h);
    }

    /// Set which built-in pixel-art icon to display (legacy, use ICON_* constants).
    pub fn set_icon(&self, icon_id: u32) {
        self.ctrl.set_state(icon_id);
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
