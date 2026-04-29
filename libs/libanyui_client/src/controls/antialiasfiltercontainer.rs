use crate::{lib, Container, Control, Widget, KIND_ANTI_ALIAS_FILTER_CONTAINER};

container_control!(AntiAliasFilterContainer, KIND_ANTI_ALIAS_FILTER_CONTAINER);

impl AntiAliasFilterContainer {
    pub fn new() -> Self {
        let id = (lib().create_control)(
            KIND_ANTI_ALIAS_FILTER_CONTAINER,
            core::ptr::null(),
            0,
        );
        Self {
            container: Container {
                ctrl: Control { id },
            },
        }
    }
}
