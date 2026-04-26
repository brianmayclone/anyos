use alloc::vec::Vec;
use libanyui_client as ui;

use crate::logic::config::Config;
use crate::logic::designer;
use crate::ui::designer_surface::DesignerSurface;
use crate::util::{path, syntax_map};

/// UI wrapper for a single open editor tab.
enum EditorTab {
    Text { editor: ui::TextEditor },
    Designer { surface: DesignerSurface },
}

/// Manages the tab bar and TextEditor instances (UI layer).
pub struct EditorView {
    pub tab_bar: ui::TabBar,
    pub panel: ui::View,
    pub context_menu: ui::ContextMenu,
    pub completion_panel: ui::View,
    pub completion_list: ui::ListBox,
    pub completion_detail: ui::Label,
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
        tab_bar.set_color(tc.toolbar_bg);
        tab_bar.set_style(ui::STYLE_BG, tc.toolbar_bg);
        tab_bar.set_style(ui::STYLE_BORDER, tc.separator);
        tab_bar.set_style(ui::STYLE_ACTIVE_BG, tc.editor_bg);
        tab_bar.set_style(ui::STYLE_ACTIVE_TEXT, tc.text);
        tab_bar.set_style(ui::STYLE_INACTIVE_BG, tc.toolbar_bg);
        tab_bar.set_style(ui::STYLE_INACTIVE_TEXT, tc.text_secondary);
        tab_bar.set_style(ui::STYLE_HOVER_BG, tc.sidebar_bg);
        tab_bar.set_style(ui::STYLE_ACCENT, tc.accent);
        tab_bar.set_style(ui::STYLE_RADIUS, 2);
        panel.add(&tab_bar);

        let breadcrumb_bar = ui::View::new();
        breadcrumb_bar.set_dock(ui::DOCK_TOP);
        breadcrumb_bar.set_size(600, 24);
        breadcrumb_bar.set_color(tc.toolbar_bg);
        panel.add(&breadcrumb_bar);

        let breadcrumb_accent = ui::View::new();
        breadcrumb_accent.set_dock(ui::DOCK_BOTTOM);
        breadcrumb_accent.set_size(600, 1);
        breadcrumb_accent.set_color(tc.separator);
        breadcrumb_bar.add(&breadcrumb_accent);

        let breadcrumb_label = ui::Label::new("No file open");
        breadcrumb_label.set_position(12, 4);
        breadcrumb_label.set_size(560, 15);
        breadcrumb_label.set_font_size(11);
        breadcrumb_label.set_text_color(tc.text_secondary);
        breadcrumb_bar.add(&breadcrumb_label);

        let context_menu = ui::ContextMenu::new(
            "Complete Word|Go to Definition|Peek Symbol|Fold Block|Cut|Copy|Paste|Select All|Explain with AI",
        );
        panel.add(&context_menu);

        let completion_panel = ui::View::new();
        completion_panel.set_position(54, 62);
        completion_panel.set_size(380, 252);
        completion_panel.set_color(tc.sidebar_bg);
        completion_panel.set_visible(false);
        panel.add(&completion_panel);

        let completion_accent = ui::View::new();
        completion_accent.set_dock(ui::DOCK_LEFT);
        completion_accent.set_size(3, 252);
        completion_accent.set_color(tc.accent);
        completion_panel.add(&completion_accent);

        let completion_title = ui::Label::new("Rust Intelligence");
        completion_title.set_dock(ui::DOCK_TOP);
        completion_title.set_size(380, 30);
        completion_title.set_font_size(12);
        completion_title.set_text_color(tc.text);
        completion_title.set_margin(12, 7, 0, 0);
        completion_panel.add(&completion_title);

        let completion_detail = ui::Label::new("");
        completion_detail.set_dock(ui::DOCK_BOTTOM);
        completion_detail.set_size(380, 48);
        completion_detail.set_font_size(11);
        completion_detail.set_text_color(tc.text_secondary);
        completion_detail.set_margin(14, 5, 12, 7);
        completion_panel.add(&completion_detail);

        let completion_detail_rule = ui::View::new();
        completion_detail_rule.set_dock(ui::DOCK_BOTTOM);
        completion_detail_rule.set_size(380, 1);
        completion_detail_rule.set_color(tc.separator);
        completion_panel.add(&completion_detail_rule);

        let completion_list = ui::ListBox::new("");
        completion_list.set_dock(ui::DOCK_FILL);
        completion_list.set_color(tc.editor_bg);
        completion_panel.add(&completion_list);

        Self {
            tab_bar,
            panel,
            context_menu,
            completion_panel,
            completion_list,
            completion_detail,
            breadcrumb_bar,
            breadcrumb_label,
            editors: Vec::new(),
        }
    }

    /// Create a new TextEditor for an opened file and add it to the panel.
    /// Returns the editor index.
    pub fn create_editor(
        &mut self,
        file_path: &str,
        content: Option<&[u8]>,
        config: &Config,
    ) -> usize {
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
            last.set_visible(false);
        }

        if designer::is_designer_file(file_path) {
            if let Some(doc) = designer::load_designer(file_path) {
                let surface = DesignerSurface::new(file_path, doc);
                self.panel.add(&surface.panel);
                let idx = self.editors.len();
                self.editors.push(EditorTab::Designer { surface });
                return idx;
            }
        }

        let editor = ui::TextEditor::new(600, 400);
        editor.set_dock(ui::DOCK_FILL);
        config.apply_to_editor(&editor);
        editor.set_read_only(read_only);
        editor.set_context_menu(&self.context_menu);

        // Load content
        if let Some(data) = content {
            editor.set_text_bytes(data);
        }

        // Load syntax highlighting
        let filename = path::basename(file_path);
        anyos_std::println!(
            "[SYNTAX] syntax_dir='{}', filename='{}'",
            config.syntax_dir,
            filename
        );
        if let Some(syn_path) = syntax_map::syntax_for_filename(&config.syntax_dir, filename) {
            anyos_std::println!("[SYNTAX] loading syn file: '{}'", syn_path);
            editor.load_syntax(&syn_path);
            anyos_std::println!("[SYNTAX] load_syntax() returned");
        } else {
            anyos_std::println!("[SYNTAX] no syntax mapping for '{}'", filename);
        }

        self.panel.add(&editor);

        let idx = self.editors.len();
        self.editors.push(EditorTab::Text { editor });

        // Wire text-changed event for modification tracking
        if !read_only {
            crate::ui::events::wire_editor_text_changed(idx);
        }

        idx
    }

    /// Switch to a specific editor tab (hide others, show this one).
    pub fn set_active(&self, index: usize) {
        for (i, tab) in self.editors.iter().enumerate() {
            tab.set_visible(i == index);
        }
        self.tab_bar.set_state(index as u32);
    }

    /// Remove an editor tab at the given index.
    pub fn remove_editor(&mut self, index: usize) {
        if index < self.editors.len() {
            self.editors[index].remove();
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
            if let EditorTab::Text { editor } = tab {
                config.apply_to_editor(editor);
            }
        }
    }

    pub fn show_completions(&self, items: &str, detail: &str) {
        self.completion_list.set_items(items);
        self.completion_detail.set_text(detail);
        self.completion_panel.set_visible(true);
        self.completion_panel.bring_to_front();
    }

    pub fn hide_completions(&self) {
        self.completion_panel.set_visible(false);
    }

    pub fn set_completion_detail(&self, detail: &str) {
        self.completion_detail.set_text(detail);
    }

    /// Get the text content of an editor at the given index.
    pub fn get_editor_text(&self, index: usize, buf: &mut [u8]) -> u32 {
        if let Some(tab) = self.editors.get(index) {
            match tab {
                EditorTab::Text { editor } => editor.get_text(buf),
                EditorTab::Designer { .. } => 0,
            }
        } else {
            0
        }
    }

    /// Get cursor position of the active editor.
    pub fn get_cursor(&self, index: usize) -> (u32, u32) {
        if let Some(tab) = self.editors.get(index) {
            match tab {
                EditorTab::Text { editor } => editor.cursor(),
                EditorTab::Designer { .. } => (0, 0),
            }
        } else {
            (0, 0)
        }
    }

    /// Get the active editor's TextEditor widget (for event wiring).
    pub fn editor_widget(&self, index: usize) -> Option<&ui::TextEditor> {
        self.editors.get(index).and_then(|tab| match tab {
            EditorTab::Text { editor } => Some(editor),
            EditorTab::Designer { .. } => None,
        })
    }

    /// Get the count of editor tabs.
    pub fn count(&self) -> usize {
        self.editors.len()
    }

    pub fn select_designer_control(&self, file_path: &str, control_name: &str) {
        for tab in &self.editors {
            if let EditorTab::Designer { surface } = tab {
                if surface.file_path() == file_path {
                    surface.render(Some(control_name));
                }
            }
        }
    }

    pub fn update_designer_document(
        &mut self,
        file_path: &str,
        doc: crate::logic::designer::DesignerDocument,
        selected_control: Option<&str>,
    ) {
        for tab in &mut self.editors {
            if let EditorTab::Designer { surface } = tab {
                if surface.file_path() == file_path {
                    surface.set_document(doc, selected_control);
                    break;
                }
            }
        }
    }
}

impl EditorTab {
    fn set_visible(&self, visible: bool) {
        match self {
            EditorTab::Text { editor } => editor.set_visible(visible),
            EditorTab::Designer { surface } => surface.set_visible(visible),
        }
    }

    fn remove(&self) {
        match self {
            EditorTab::Text { editor } => editor.remove(),
            EditorTab::Designer { surface } => surface.remove(),
        }
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
