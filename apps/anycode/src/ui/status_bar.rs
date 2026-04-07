use alloc::format;
use libanyui_client as ui;

/// Status bar at the bottom of the window — professional IDE look.
pub struct StatusBar {
    pub panel: ui::View,
    file_label: ui::Label,
    cursor_label: ui::Label,
    language_label: ui::Label,
    branch_label: ui::Label,
    encoding_label: ui::Label,
    project_label: ui::Label,
    problems_label: ui::Label,
}

impl StatusBar {
    /// Create the status bar view with all labels.
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_color(tc.tab_border_active);
        panel.set_size(900, 22);

        let t = anyos_std::i18n::t;

        // Left group: branch, problems
        let branch_lbl = ui::Label::new("");
        branch_lbl.set_position(8, 3);
        branch_lbl.set_font_size(11);
        branch_lbl.set_text_color(tc.check_mark);
        panel.add(&branch_lbl);

        let problems_lbl = ui::Label::new("");
        problems_lbl.set_position(140, 3);
        problems_lbl.set_font_size(11);
        problems_lbl.set_text_color(tc.check_mark);
        panel.add(&problems_lbl);

        // Center: cursor position
        let cursor_lbl = ui::Label::new("Ln 1, Col 1");
        cursor_lbl.set_position(320, 3);
        cursor_lbl.set_font_size(11);
        cursor_lbl.set_text_color(tc.check_mark);
        panel.add(&cursor_lbl);

        // Right group: encoding, language, project type, filename
        let enc_lbl = ui::Label::new("UTF-8");
        enc_lbl.set_position(450, 3);
        enc_lbl.set_font_size(11);
        enc_lbl.set_text_color(tc.check_mark);
        panel.add(&enc_lbl);

        let lang_lbl = ui::Label::new(t("Plain Text"));
        lang_lbl.set_position(510, 3);
        lang_lbl.set_font_size(11);
        lang_lbl.set_text_color(tc.check_mark);
        panel.add(&lang_lbl);

        let project_lbl = ui::Label::new("");
        project_lbl.set_position(630, 3);
        project_lbl.set_font_size(11);
        project_lbl.set_text_color(tc.check_mark);
        panel.add(&project_lbl);

        let file_lbl = ui::Label::new(t("No file open"));
        file_lbl.set_position(750, 3);
        file_lbl.set_font_size(11);
        file_lbl.set_text_color(tc.check_mark);
        panel.add(&file_lbl);

        Self {
            panel,
            file_label: file_lbl,
            cursor_label: cursor_lbl,
            language_label: lang_lbl,
            branch_label: branch_lbl,
            encoding_label: enc_lbl,
            project_label: project_lbl,
            problems_label: problems_lbl,
        }
    }

    /// Update the filename display.
    pub fn set_filename(&self, name: &str) {
        self.file_label.set_text(name);
    }

    /// Update cursor position display.
    pub fn set_cursor(&self, line: u32, col: u32) {
        let t = anyos_std::i18n::t;
        let text = format!("{} {}, {} {}", t("Ln"), line + 1, t("Col"), col + 1);
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
            let text = format!("\u{2387} {}", branch); // ⎇ branch symbol
            self.branch_label.set_text(&text);
        }
    }

    /// Update the project type display.
    pub fn set_project_type(&self, project_type: &str) {
        if project_type.is_empty() {
            self.project_label.set_text("");
        } else {
            self.project_label.set_text(project_type);
        }
    }

    /// Update the problems count display.
    pub fn set_problems(&self, errors: usize, warnings: usize) {
        if errors == 0 && warnings == 0 {
            self.problems_label.set_text("");
        } else {
            let text = format!("\u{2716} {} \u{26A0} {}", errors, warnings);
            self.problems_label.set_text(&text);
        }
    }
}
