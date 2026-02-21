use crate::{Container, Control, Widget, lib, events, KIND_EXPANDER};
use crate::events::CheckedChangedEvent;

container_control!(Expander, KIND_EXPANDER);

impl Expander {
    /// Create an expander with the given header title. Starts expanded.
    pub fn new(title: &str) -> Self {
        let id = (lib().create_control)(KIND_EXPANDER, title.as_ptr(), title.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }

    /// Whether the expander is currently expanded.
    pub fn is_expanded(&self) -> bool {
        self.get_state() != 0
    }

    /// Programmatically expand or collapse.
    pub fn set_expanded(&self, expanded: bool) {
        self.set_state(expanded as u32);
    }

    /// Called when the expander is toggled (expanded/collapsed).
    pub fn on_toggled(&self, mut f: impl FnMut(&CheckedChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let checked = Control::from_id(id).get_state() != 0;
            f(&CheckedChangedEvent { id, checked });
        });
        (lib().on_change_fn)(self.container.ctrl.id, thunk, ud);
    }
}
