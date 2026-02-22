use crate::{Control, Widget, lib, events, KIND_ICON_BUTTON};
use crate::events::ClickEvent;

// ── Icon ID constants ────────────────────────────────────────────────
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

    /// Set which built-in icon to display (use ICON_* constants).
    pub fn set_icon(&self, icon_id: u32) {
        self.ctrl.set_state(icon_id);
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
