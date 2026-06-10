use crate::events::ScrollChangedEvent;
use crate::{events, lib, Container, Control, Widget, KIND_SCROLL_VIEW};

container_control!(ScrollView, KIND_SCROLL_VIEW);

impl ScrollView {
    /// Wrap a raw control ID without creating a compositor control.
    ///
    /// `from_id(0)` yields a detached dummy handle for headless use;
    /// callers must not invoke UI methods on it.
    pub fn from_id(id: u32) -> Self {
        Self {
            container: Container {
                ctrl: Control::from_id(id),
            },
        }
    }

    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SCROLL_VIEW, core::ptr::null(), 0);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }

    pub fn on_scroll(&self, mut f: impl FnMut(&ScrollChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let offset = Control::from_id(id).get_state();
            f(&ScrollChangedEvent { id, offset });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }

    pub fn scroll_offsets(&self) -> (i32, i32) {
        let mut x = 0i32;
        let mut y = 0i32;
        (lib().scrollview_get_offsets)(self.container.ctrl.id, &mut x, &mut y);
        (x, y)
    }

    pub fn set_scroll_offsets(&self, x: i32, y: i32) {
        (lib().scrollview_set_offsets)(self.container.ctrl.id, x, y);
    }
}
