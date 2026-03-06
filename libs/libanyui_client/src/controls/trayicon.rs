use crate::{lib, events};

/// A system tray icon (16×16 ARGB, displayed in the menu bar).
///
/// Unlike normal controls, TrayIcon does not have a control ID or parent window.
/// It communicates directly with the compositor via IPC.
pub struct TrayIcon {
    icon_id: u32,
}

impl TrayIcon {
    /// Create and register a tray icon with the given ID and 16×16 ARGB pixel data.
    /// `pixels` must be exactly 256 elements (16×16).
    pub fn new(icon_id: u32, pixels: &[u32; 256]) -> Self {
        (lib().add_status_icon_fn)(icon_id, pixels.as_ptr());
        Self { icon_id }
    }

    /// Update the tray icon's pixel data.
    pub fn set_pixels(&self, pixels: &[u32; 256]) {
        (lib().add_status_icon_fn)(self.icon_id, pixels.as_ptr());
    }

    /// Register a callback for when this tray icon is clicked.
    pub fn on_click(&self, mut f: impl FnMut() + 'static) {
        let (thunk, ud) = events::register(move |_id, _evt| f());
        (lib().on_tray_click_fn)(self.icon_id, thunk, ud);
    }

    /// Remove this tray icon from the menu bar.
    pub fn remove(&self) {
        (lib().remove_status_icon_fn)(self.icon_id);
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
    }
}
