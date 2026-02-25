use crate::{Container, Control, Widget, lib, events, KIND_WINDOW, EVENT_CLOSE, EVENT_RESIZE, EVENT_KEY};
use crate::events::{EventArgs, ClickEvent};
use crate::KeyEvent;

container_control!(Window, KIND_WINDOW);

// ── Window flag constants ───────────────────────────────────────────

pub const WIN_FLAG_BORDERLESS: u32 = 0x01;
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;
pub const WIN_FLAG_NO_CLOSE: u32 = 0x08;
pub const WIN_FLAG_NO_MINIMIZE: u32 = 0x10;
pub const WIN_FLAG_NO_MAXIMIZE: u32 = 0x20;
pub const WIN_FLAG_SHADOW: u32 = 0x40;

impl Window {
    /// Create a new window at position (x, y) with default flags.
    /// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
    pub fn new(title: &str, x: i32, y: i32, w: u32, h: u32) -> Self {
        let id = (lib().create_window)(title.as_ptr(), title.len() as u32, x, y, w, h, 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    /// Create a new window with explicit flags (borderless, shadow, etc.).
    pub fn new_with_flags(title: &str, x: i32, y: i32, w: u32, h: u32, flags: u32) -> Self {
        let id = (lib().create_window)(title.as_ptr(), title.len() as u32, x, y, w, h, flags);
        Self { container: Container { ctrl: Control { id } } }
    }

    /// Set the window title after creation.
    pub fn set_title(&self, title: &str) {
        (lib().set_title)(self.container.ctrl.id, title.as_ptr(), title.len() as u32);
    }

    pub fn destroy(&self) {
        (lib().destroy_window)(self.container.ctrl.id);
    }

    /// Register a closure to be called when the window background is clicked.
    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.container.ctrl.id, thunk, ud);
    }

    pub fn on_close(&self, mut f: impl FnMut(&EventArgs) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&EventArgs { id }));
        (lib().on_event_fn)(self.container.ctrl.id, EVENT_CLOSE, thunk, ud);
    }

    pub fn on_resize(&self, mut f: impl FnMut(&EventArgs) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&EventArgs { id }));
        (lib().on_event_fn)(self.container.ctrl.id, EVENT_RESIZE, thunk, ud);
    }

    /// Register a typed key-down handler on this window.
    /// The closure receives a `KeyEvent` with keycode, char_code, and modifiers.
    /// This fires for unhandled key events that bubble up to the window.
    pub fn on_key_down(&self, mut f: impl FnMut(&KeyEvent) + 'static) {
        let (thunk, ud) = events::register(move |_id, _| {
            let ke = crate::get_key_info();
            f(&ke);
        });
        (lib().on_event_fn)(self.container.ctrl.id, EVENT_KEY, thunk, ud);
    }
}
