use crate::{Control, Widget, lib, KIND_TEXT_EDITOR};
use crate::events;

leaf_control!(TextEditor, KIND_TEXT_EDITOR);

impl TextEditor {
    /// Create a new empty TextEditor with the given display size.
    pub fn new(w: u32, h: u32) -> Self {
        let id = (lib().create_control)(KIND_TEXT_EDITOR, core::ptr::null(), 0);
        (lib().set_size)(id, w, h);
        Self { ctrl: Control { id } }
    }

    /// Create a TextEditor and load text from a file.
    pub fn from_file(path: &str, w: u32, h: u32) -> Self {
        let te = Self::new(w, h);
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            te.set_text_bytes(&data);
        }
        te
    }

    /// Set the editor text.
    pub fn set_text(&self, text: &str) {
        (lib().texteditor_set_text)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    /// Set the editor text from raw bytes.
    pub fn set_text_bytes(&self, data: &[u8]) {
        (lib().texteditor_set_text)(self.ctrl.id, data.as_ptr(), data.len() as u32);
    }

    /// Get the editor text into a buffer. Returns the number of bytes written.
    pub fn get_text(&self, buf: &mut [u8]) -> u32 {
        (lib().texteditor_get_text)(self.ctrl.id, buf.as_mut_ptr(), buf.len() as u32)
    }

    /// Load syntax highlighting from a .syn file path.
    pub fn load_syntax(&self, path: &str) {
        anyos_std::println!("[SYNTAX-CLIENT] reading file: '{}'", path);
        match anyos_std::fs::read_to_vec(path) {
            Ok(data) => {
                anyos_std::println!("[SYNTAX-CLIENT] read {} bytes from '{}'", data.len(), path);
                self.load_syntax_from_bytes(&data);
                anyos_std::println!("[SYNTAX-CLIENT] sent to server");
            }
            Err(_) => {
                anyos_std::println!("[SYNTAX-CLIENT] FAILED to read file: '{}'", path);
            }
        }
    }

    /// Load syntax highlighting from raw bytes.
    pub fn load_syntax_from_bytes(&self, data: &[u8]) {
        (lib().texteditor_set_syntax)(self.ctrl.id, data.as_ptr(), data.len() as u32);
    }

    /// Set cursor position (row, col).
    pub fn set_cursor(&self, row: u32, col: u32) {
        (lib().texteditor_set_cursor)(self.ctrl.id, row, col);
    }

    /// Get cursor position as (row, col).
    pub fn cursor(&self) -> (u32, u32) {
        let mut row = 0u32;
        let mut col = 0u32;
        (lib().texteditor_get_cursor)(self.ctrl.id, &mut row, &mut col);
        (row, col)
    }

    /// Set line height in pixels (minimum 12).
    pub fn set_line_height(&self, h: u32) {
        (lib().texteditor_set_line_height)(self.ctrl.id, h);
    }

    /// Set tab width in spaces (minimum 1).
    pub fn set_tab_width(&self, w: u32) {
        (lib().texteditor_set_tab_width)(self.ctrl.id, w);
    }

    /// Show or hide line numbers.
    pub fn set_show_line_numbers(&self, show: bool) {
        (lib().texteditor_set_show_line_numbers)(self.ctrl.id, show as u32);
    }

    /// Set font (font_id, font_size).
    pub fn set_editor_font(&self, font_id: u32, size: u32) {
        (lib().texteditor_set_font)(self.ctrl.id, font_id, size);
    }

    /// Insert text at the current cursor position.
    pub fn insert_text(&self, text: &str) {
        (lib().texteditor_insert_text)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    /// Get the number of lines in the editor.
    pub fn line_count(&self) -> u32 {
        (lib().texteditor_get_line_count)(self.ctrl.id)
    }

    /// Copy selected text to system clipboard. Returns true if text was copied.
    pub fn copy(&self) -> bool {
        (lib().texteditor_copy)(self.ctrl.id) != 0
    }

    /// Cut selected text to system clipboard. Returns true if text was cut.
    pub fn cut(&self) -> bool {
        (lib().texteditor_cut)(self.ctrl.id) != 0
    }

    /// Paste from system clipboard at cursor position. Returns number of bytes pasted.
    pub fn paste(&self) -> u32 {
        (lib().texteditor_paste)(self.ctrl.id)
    }

    /// Select all text in the editor.
    pub fn select_all(&self) {
        (lib().texteditor_select_all)(self.ctrl.id);
    }

    /// Register a callback for when the text changes.
    pub fn on_text_changed(&self, mut f: impl FnMut(&crate::events::TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            f(&crate::events::TextChangedEvent { id });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
