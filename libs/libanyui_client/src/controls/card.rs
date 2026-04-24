use crate::{lib, Container, Control, Widget, KIND_CARD};

container_control!(Card, KIND_CARD);

impl Card {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_CARD, core::ptr::null(), 0);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }
}
