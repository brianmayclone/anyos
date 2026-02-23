use crate::{Container, Control, Widget, lib, events, KIND_WINDOW, EVENT_CLOSE, EVENT_RESIZE};
use crate::events::EventArgs;

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

    pub fn destroy(&self) {
        (lib().destroy_window)(self.container.ctrl.id);
    }

    pub fn on_close(&self, mut f: impl FnMut(&EventArgs) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&EventArgs { id }));
        (lib().on_event_fn)(self.container.ctrl.id, EVENT_CLOSE, thunk, ud);
    }

    pub fn on_resize(&self, mut f: impl FnMut(&EventArgs) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&EventArgs { id }));
        (lib().on_event_fn)(self.container.ctrl.id, EVENT_RESIZE, thunk, ud);
    }
}
