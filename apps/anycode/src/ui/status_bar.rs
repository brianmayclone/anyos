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
    project_label: ui::Label,
    problems_label: ui::Label,
    analysis_label: ui::Label,
}

impl StatusBar {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_color(tc.tab_border_active);
        panel.set_size(900, 26);

        let left = ui::View::new();
        left.set_dock(ui::DOCK_LEFT);
        left.set_size(560, 26);
        left.set_color(tc.tab_border_active);
        panel.add(&left);

        let left_flow = ui::FlowPanel::new();
        left_flow.set_dock(ui::DOCK_FILL);
        left_flow.set_color(tc.tab_border_active);
        left_flow.set_padding(8, 4, 8, 4);
        left.add(&left_flow);

        let right = ui::View::new();
        right.set_dock(ui::DOCK_RIGHT);
        right.set_size(340, 26);
        right.set_color(tc.tab_border_active);
        panel.add(&right);

        let right_flow = ui::FlowPanel::new();
        right_flow.set_dock(ui::DOCK_FILL);
        right_flow.set_color(tc.tab_border_active);
        right_flow.set_padding(8, 4, 8, 4);
        right.add(&right_flow);

        let branch_lbl = make_status_label("", tc.check_mark);
        branch_lbl.set_margin(0, 0, 14, 0);
        left_flow.add(&branch_lbl);

        let problems_lbl = make_status_label("", tc.check_mark);
        problems_lbl.set_margin(0, 0, 14, 0);
        left_flow.add(&problems_lbl);

        let project_lbl = make_status_label("", tc.check_mark);
        project_lbl.set_margin(0, 0, 14, 0);
        left_flow.add(&project_lbl);

        let analysis_lbl = make_status_label("", tc.check_mark);
        analysis_lbl.set_margin(0, 0, 14, 0);
        left_flow.add(&analysis_lbl);

        let cursor_lbl = make_status_label("Ln 1, Col 1", tc.check_mark);
        cursor_lbl.set_margin(0, 0, 0, 0);
        left_flow.add(&cursor_lbl);

        let enc_lbl = make_status_label("UTF-8", tc.check_mark);
        enc_lbl.set_margin(0, 0, 12, 0);
        right_flow.add(&enc_lbl);

        let lang_lbl = make_status_label("Plain Text", tc.check_mark);
        lang_lbl.set_margin(0, 0, 12, 0);
        right_flow.add(&lang_lbl);

        let file_lbl = make_status_label("No file open", tc.check_mark);
        file_lbl.set_margin(0, 0, 0, 0);
        right_flow.add(&file_lbl);

        Self {
            panel,
            file_label: file_lbl,
            cursor_label: cursor_lbl,
            language_label: lang_lbl,
            branch_label: branch_lbl,
            encoding_label: enc_lbl,
            project_label: project_lbl,
            problems_label: problems_lbl,
            analysis_label: analysis_lbl,
        }
    }

    pub fn set_filename(&self, name: &str) {
        self.file_label.set_text(name);
    }

    pub fn set_cursor(&self, line: u32, col: u32) {
        let t = anyos_std::i18n::t;
        let text = format!("{} {}, {} {}", t("Ln"), line + 1, t("Col"), col + 1);
        self.cursor_label.set_text(&text);
    }

    pub fn set_language(&self, lang: &str) {
        self.language_label.set_text(lang);
    }

    pub fn set_branch(&self, branch: &str) {
        if branch.is_empty() {
            self.branch_label.set_text("");
        } else {
            self.branch_label.set_text(&format!("\u{2387} {}", branch));
        }
    }

    pub fn set_project_type(&self, project_type: &str) {
        if project_type.is_empty() {
            self.project_label.set_text("");
        } else {
            self.project_label.set_text(project_type);
        }
    }

    pub fn set_problems(&self, errors: usize, warnings: usize) {
        if errors == 0 && warnings == 0 {
            self.problems_label.set_text("");
        } else {
            self.problems_label
                .set_text(&format!("\u{2716} {}  \u{26A0} {}", errors, warnings));
        }
    }

    pub fn set_analysis_status(&self, text: &str) {
        self.analysis_label.set_text(text);
    }
}

fn make_status_label(text: &str, color: u32) -> ui::Label {
    let lbl = ui::Label::new(text);
    lbl.set_size(120, 18);
    lbl.set_font_size(11);
    lbl.set_text_color(color);
    lbl
}
