use crate::{lib, Container, Control, Widget, KIND_VIEW};

container_control!(View, KIND_VIEW);

impl View {
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
        let id = (lib().create_control)(KIND_VIEW, core::ptr::null(), 0);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }
}
