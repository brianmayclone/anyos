use crate::{Container, Control, Widget, lib, KIND_NAVIGATION_BAR};

container_control!(NavigationBar, KIND_NAVIGATION_BAR);

impl NavigationBar {
    pub fn new(title: &str) -> Self {
        let id = (lib().create_control)(KIND_NAVIGATION_BAR, title.as_ptr(), title.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }
}
