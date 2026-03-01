//! Memory hex dump view using TextEditor.

use libanyui_client as ui;
use ui::Widget;
use crate::logic::memory;

/// Memory view panel.
pub struct MemoryView {
    pub editor: ui::TextEditor,
    /// Current base address being displayed.
    pub base_addr: u64,
}

impl MemoryView {
    /// Create the memory view.
    pub fn new(_parent: &impl Widget) -> Self {
        let editor = ui::TextEditor::new(800, 600);
        editor.set_dock(ui::DOCK_FILL);
        editor.set_read_only(true);
        // Use monospace font (font_id 4 = Andale Mono)
        editor.set_font(4);

        Self {
            editor,
            base_addr: 0,
        }
    }

    /// Update the hex dump with memory data.
    pub fn update(&mut self, addr: u64, data: &[u8]) {
        self.base_addr = addr;
        let text = memory::format_hex_dump(addr, data);
        self.editor.set_text(&text);
    }

    /// Show a message when memory is not readable.
    pub fn show_message(&self, msg: &str) {
        self.editor.set_text(msg);
    }
}
