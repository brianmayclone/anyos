use libanyui_client as ui;

/// The output panel showing build/run output.
pub struct OutputPanel {
    pub panel: ui::View,
    pub text_area: ui::TextArea,
}

impl OutputPanel {
    /// Create the output panel with header and text area.
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_color(0xFF1E1E1E);

        let header = ui::Label::new("OUTPUT");
        header.set_dock(ui::DOCK_TOP);
        header.set_size(100, 20);
        header.set_font_size(11);
        header.set_text_color(0xFF969696);
        header.set_margin(8, 4, 0, 0);
        panel.add(&header);

        let ta = ui::TextArea::new();
        ta.set_dock(ui::DOCK_FILL);
        ta.set_font(4); // monospace
        ta.set_font_size(12);
        ta.set_color(0xFF1E1E1E);
        ta.set_text_color(0xFFCCCCCC);
        panel.add(&ta);

        Self {
            panel,
            text_area: ta,
        }
    }

    /// Clear all output.
    pub fn clear(&self) {
        self.text_area.set_text("");
    }

    /// Append text to the output (read existing + concat).
    pub fn append(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.text_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let total = existing + add;
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.text_area.set_text(full);
        }
    }

    /// Append a line to the output (with trailing newline).
    pub fn append_line(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.text_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing - 1);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let mut total = existing + add;
        if total < buf.len() {
            buf[total] = b'\n';
            total += 1;
        }
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.text_area.set_text(full);
        }
    }
}
