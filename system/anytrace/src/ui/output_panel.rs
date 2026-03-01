//! Output/log panel using TextArea.

use libanyui_client as ui;
use ui::Widget;

/// Output panel for debug log messages.
pub struct OutputPanel {
    pub text_area: ui::TextArea,
    /// Accumulated log text.
    text_buf: alloc::string::String,
}

impl OutputPanel {
    /// Create the output panel.
    pub fn new(_parent: &impl Widget) -> Self {
        let text_area = ui::TextArea::new();
        text_area.set_dock(ui::DOCK_FILL);
        Self {
            text_area,
            text_buf: alloc::string::String::new(),
        }
    }

    /// Append a log message.
    pub fn log(&mut self, msg: &str) {
        self.text_buf.push_str(msg);
        self.text_buf.push('\n');
        self.text_area.set_text(&self.text_buf);
    }

    /// Clear all messages.
    pub fn clear(&mut self) {
        self.text_buf.clear();
        self.text_area.set_text("");
    }
}
