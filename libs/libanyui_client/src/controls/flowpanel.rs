use crate::{lib, Container, Control, Widget, KIND_FLOW_PANEL};

container_control!(FlowPanel, KIND_FLOW_PANEL);

impl FlowPanel {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_FLOW_PANEL, core::ptr::null(), 0);
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }
}
