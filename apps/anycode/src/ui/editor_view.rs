use alloc::vec::Vec;
use libanyui_client as ui;

use crate::logic::config::Config;
use crate::util::{path, syntax_map};

/// UI wrapper for a single open editor tab.
struct EditorTab {
    editor: ui::TextEditor,
}

/// Manages the tab bar and TextEditor instances (UI layer).
pub struct EditorView {
    pub tab_bar: ui::TabBar,
    pub panel: ui::View,
    editors: Vec<EditorTab>,
}

impl EditorView {
    /// Create the editor view with a tab bar and editor panel.
    pub fn new() -> Self {
        let panel = ui::View::new();

        let tab_bar = ui::TabBar::new("");
        tab_bar.set_dock(ui::DOCK_TOP);
        tab_bar.set_size(600, 28);
        tab_bar.set_color(0xFF2D2D2D);
        panel.add(&tab_bar);

        Self {
            tab_bar,
            panel,
            editors: Vec::new(),
        }
    }

    /// Create a new TextEditor for an opened file and add it to the panel.
    /// Returns the editor index.
    pub fn create_editor(&mut self, file_path: &str, content: Option<&[u8]>, config: &Config) -> usize {
        // Hide current active editor
        if let Some(last) = self.editors.last() {
            last.editor.set_visible(false);
        }

        let editor = ui::TextEditor::new(600, 400);
        editor.set_dock(ui::DOCK_FILL);
        config.apply_to_editor(&editor);

        // Load content
        if let Some(data) = content {
            editor.set_text_bytes(data);
        }

        // Load syntax highlighting
        let filename = path::basename(file_path);
        if let Some(syn_path) = syntax_map::syntax_for_filename(&config.syntax_dir, filename) {
            editor.load_syntax(&syn_path);
        }

        self.panel.add(&editor);

        let idx = self.editors.len();
        self.editors.push(EditorTab { editor });
        idx
    }

    /// Switch to a specific editor tab (hide others, show this one).
    pub fn set_active(&self, index: usize) {
        for (i, tab) in self.editors.iter().enumerate() {
            tab.editor.set_visible(i == index);
        }
        self.tab_bar.set_state(index as u32);
    }

    /// Remove an editor tab at the given index.
    pub fn remove_editor(&mut self, index: usize) {
        if index < self.editors.len() {
            self.editors[index].editor.remove();
            self.editors.remove(index);
        }
    }

    /// Update the tab bar labels.
    pub fn update_tab_labels(&self, labels: &str, active: usize) {
        self.tab_bar.set_text(labels);
        self.tab_bar.set_state(active as u32);
    }

    /// Get the text content of an editor at the given index.
    pub fn get_editor_text(&self, index: usize, buf: &mut [u8]) -> u32 {
        if let Some(tab) = self.editors.get(index) {
            tab.editor.get_text(buf)
        } else {
            0
        }
    }

    /// Get cursor position of the active editor.
    pub fn get_cursor(&self, index: usize) -> (u32, u32) {
        if let Some(tab) = self.editors.get(index) {
            tab.editor.cursor()
        } else {
            (0, 0)
        }
    }

    /// Get the active editor's TextEditor widget (for event wiring).
    pub fn editor_widget(&self, index: usize) -> Option<&ui::TextEditor> {
        self.editors.get(index).map(|t| &t.editor)
    }

    /// Get the count of editor tabs.
    pub fn count(&self) -> usize {
        self.editors.len()
    }
}

impl Config {
    /// Apply configuration to a TextEditor instance.
    pub fn apply_to_editor(&self, editor: &ui::TextEditor) {
        editor.set_editor_font(self.font_id, self.font_size);
        editor.set_tab_width(self.tab_width);
        editor.set_show_line_numbers(self.show_line_numbers);
    }
}
