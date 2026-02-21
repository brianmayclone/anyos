use crate::{Container, Control, Widget, lib, KIND_TOOLBAR};

container_control!(Toolbar, KIND_TOOLBAR);

impl Toolbar {
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_TOOLBAR, core::ptr::null(), 0);
        Self { container: Container { ctrl: Control { id } } }
    }

    /// Create a Button with the given text label and add it to this toolbar.
    pub fn add_button(&self, text: &str) -> crate::controls::Button {
        let btn = crate::controls::Button::new(text);
        self.add(&btn);
        btn
    }

    /// Create a Label with the given text and add it to this toolbar.
    pub fn add_label(&self, text: &str) -> crate::controls::Label {
        let lbl = crate::controls::Label::new(text);
        self.add(&lbl);
        lbl
    }

    /// Create a vertical Divider separator (1x16) and add it to this toolbar.
    pub fn add_separator(&self) -> crate::controls::Divider {
        let div = crate::controls::Divider::new();
        div.set_size(1, 16);
        self.add(&div);
        div
    }

    /// Create an IconButton with the given icon text and add it to this toolbar.
    pub fn add_icon_button(&self, icon_text: &str) -> crate::controls::IconButton {
        let btn = crate::controls::IconButton::new(icon_text);
        self.add(&btn);
        btn
    }
}
