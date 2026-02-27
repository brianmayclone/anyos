use alloc::format;
use libanyui_client as ui;

/// Status bar at the bottom of the window.
pub struct StatusBar {
    pub panel: ui::View,
    file_label: ui::Label,
    cursor_label: ui::Label,
    language_label: ui::Label,
    branch_label: ui::Label,
    encoding_label: ui::Label,
}

impl StatusBar {
    /// Create the status bar view with all labels.
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_color(tc.tab_border_active);
        panel.set_size(900, 22);

        let file_lbl = ui::Label::new("No file open");
        file_lbl.set_position(8, 3);
        file_lbl.set_font_size(11);
        file_lbl.set_text_color(tc.check_mark);
        panel.add(&file_lbl);

        let cursor_lbl = ui::Label::new("Ln 1, Col 1");
        cursor_lbl.set_position(300, 3);
        cursor_lbl.set_font_size(11);
        cursor_lbl.set_text_color(tc.check_mark);
        panel.add(&cursor_lbl);

        let lang_lbl = ui::Label::new("Plain Text");
        lang_lbl.set_position(450, 3);
        lang_lbl.set_font_size(11);
        lang_lbl.set_text_color(tc.check_mark);
        panel.add(&lang_lbl);

        let branch_lbl = ui::Label::new("");
        branch_lbl.set_position(580, 3);
        branch_lbl.set_font_size(11);
        branch_lbl.set_text_color(tc.check_mark);
        panel.add(&branch_lbl);

        let enc_lbl = ui::Label::new("UTF-8");
        enc_lbl.set_position(720, 3);
        enc_lbl.set_font_size(11);
        enc_lbl.set_text_color(tc.check_mark);
        panel.add(&enc_lbl);

        Self {
            panel,
            file_label: file_lbl,
            cursor_label: cursor_lbl,
            language_label: lang_lbl,
            branch_label: branch_lbl,
            encoding_label: enc_lbl,
        }
    }

    /// Update the filename display.
    pub fn set_filename(&self, name: &str) {
        self.file_label.set_text(name);
    }

    /// Update cursor position display.
    pub fn set_cursor(&self, line: u32, col: u32) {
        let text = format!("Ln {}, Col {}", line + 1, col + 1);
        self.cursor_label.set_text(&text);
    }

    /// Update language display.
    pub fn set_language(&self, lang: &str) {
        self.language_label.set_text(lang);
    }

    /// Update the git branch display.
    pub fn set_branch(&self, branch: &str) {
        if branch.is_empty() {
            self.branch_label.set_text("");
        } else {
            let text = format!("$(git) {}", branch);
            self.branch_label.set_text(&text);
        }
    }
}
