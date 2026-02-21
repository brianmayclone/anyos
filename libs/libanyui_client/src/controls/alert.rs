use crate::{Container, Control, Widget, lib, KIND_ALERT};

container_control!(Alert, KIND_ALERT);

impl Alert {
    pub fn new(message: &str) -> Self {
        let id = (lib().create_control)(KIND_ALERT, message.as_ptr(), message.len() as u32);
        Self { container: Container { ctrl: Control { id } } }
    }
}
