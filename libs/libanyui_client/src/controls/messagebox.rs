use crate::lib;

/// Message box type — determines the icon and accent color.
#[derive(Clone, Copy)]
pub enum MessageBoxType {
    /// Red exclamation mark — for errors and critical alerts.
    Alert = 0,
    /// Blue "i" icon — for informational messages.
    Info = 1,
    /// Yellow exclamation mark — for warnings.
    Warning = 2,
}

/// A modal message dialog that blocks until the user clicks the button.
pub struct MessageBox;

impl MessageBox {
    /// Show a modal message box and block until dismissed.
    ///
    /// # Arguments
    /// * `msg_type` — `Alert`, `Info`, or `Warning` (controls icon/color).
    /// * `text` — The message to display.
    /// * `button_text` — Label for the dismiss button. Defaults to `"OK"` if `None`.
    pub fn show(msg_type: MessageBoxType, text: &str, button_text: Option<&str>) {
        let btn = button_text.unwrap_or("OK");
        (lib().message_box)(
            msg_type as u32,
            text.as_ptr(), text.len() as u32,
            btn.as_ptr(), btn.len() as u32,
        );
    }
}
