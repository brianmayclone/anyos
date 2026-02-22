use crate::{Container, Control, Widget, lib, events, KIND_WINDOW, EVENT_CLOSE, EVENT_RESIZE};
use crate::events::EventArgs;

container_control!(Window, KIND_WINDOW);

impl Window {
    /// Create a new window at position (x, y).
    /// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
    pub fn new(title: &str, x: i32, y: i32, w: u32, h: u32) -> Self {
        let id = (lib().create_window)(title.as_ptr(), title.len() as u32, x, y, w, h);
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
