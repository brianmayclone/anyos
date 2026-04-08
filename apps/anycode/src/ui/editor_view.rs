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
    breadcrumb_bar: ui::View,
    breadcrumb_label: ui::Label,
    editors: Vec<EditorTab>,
}

impl EditorView {
    /// Create the editor view with a tab bar and editor panel.
    pub fn new(_config: &Config) -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_color(tc.editor_bg);

        let tab_bar = ui::TabBar::new("");
        tab_bar.set_dock(ui::DOCK_TOP);
        tab_bar.set_size(600, 32);
        tab_bar.set_color(tc.tab_inactive_bg);
        panel.add(&tab_bar);

        let breadcrumb_bar = ui::View::new();
        breadcrumb_bar.set_dock(ui::DOCK_TOP);
        breadcrumb_bar.set_size(600, 24);
        breadcrumb_bar.set_color(tc.sidebar_bg);
        panel.add(&breadcrumb_bar);

        let breadcrumb_label = ui::Label::new("No file open");
        breadcrumb_label.set_position(10, 4);
        breadcrumb_label.set_font_size(11);
        breadcrumb_label.set_text_color(tc.text_secondary);
        breadcrumb_bar.add(&breadcrumb_label);

        Self {
            tab_bar,
            panel,
            breadcrumb_bar,
            breadcrumb_label,
            editors: Vec::new(),
        }
    }

    /// Create a new TextEditor for an opened file and add it to the panel.
    /// Returns the editor index.
    pub fn create_editor(&mut self, file_path: &str, content: Option<&[u8]>, config: &Config) -> usize {
        self.create_editor_with_mode(file_path, content, config, false)
    }

    pub fn create_editor_with_mode(
        &mut self,
        file_path: &str,
        content: Option<&[u8]>,
        config: &Config,
        read_only: bool,
    ) -> usize {
        // Hide current active editor
        if let Some(last) = self.editors.last() {
            last.editor.set_visible(false);
        }

        let editor = ui::TextEditor::new(600, 400);
        editor.set_dock(ui::DOCK_FILL);
        config.apply_to_editor(&editor);
        editor.set_read_only(read_only);

        // Load content
        if let Some(data) = content {
            editor.set_text_bytes(data);
        }

        // Load syntax highlighting
        let filename = path::basename(file_path);
        anyos_std::println!("[SYNTAX] syntax_dir='{}', filename='{}'", config.syntax_dir, filename);
        if let Some(syn_path) = syntax_map::syntax_for_filename(&config.syntax_dir, filename) {
            anyos_std::println!("[SYNTAX] loading syn file: '{}'", syn_path);
            editor.load_syntax(&syn_path);
            anyos_std::println!("[SYNTAX] load_syntax() returned");
        } else {
            anyos_std::println!("[SYNTAX] no syntax mapping for '{}'", filename);
        }

        self.panel.add(&editor);

        let idx = self.editors.len();
        self.editors.push(EditorTab { editor });

        // Wire text-changed event for modification tracking
        if !read_only {
            crate::ui::events::wire_editor_text_changed(idx);
        }

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

    pub fn set_breadcrumb(&self, text: &str) {
        self.breadcrumb_label.set_text(text);
        self.breadcrumb_bar.set_visible(true);
    }

    pub fn apply_config(&self, config: &Config) {
        for tab in &self.editors {
            config.apply_to_editor(&tab.editor);
        }
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
        editor.set_line_height(self.line_height);
        editor.set_tab_width(self.tab_width);
        editor.set_show_line_numbers(self.show_line_numbers);
    }
}
