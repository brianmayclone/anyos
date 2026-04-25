use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
#[allow(unused_imports)]
use libanyui_client as ui;

use crate::logic::diagnostics::{DiagnosticSet, Severity};

const STYLE_BOLD: u32 = 1;

#[derive(Clone, Copy, PartialEq)]
pub enum ProblemFilter {
    All,
    Errors,
    Warnings,
    CurrentFile,
}

#[derive(Clone)]
struct ProblemLocation {
    file_path: String,
    line: u32,
    column: u32,
}

// ════════════════════════════════════════════════════════════════
//  Problems panel — displays compiler errors and warnings
// ════════════════════════════════════════════════════════════════

pub struct ProblemsPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub summary_label: ui::Label,
    pub error_label: ui::Label,
    pub warning_label: ui::Label,
    pub visible_label: ui::Label,
    pub btn_all: ui::Button,
    pub btn_errors: ui::Button,
    pub btn_warnings: ui::Button,
    pub btn_current_file: ui::Button,
    filter: ProblemFilter,
    file_locations: Vec<ProblemLocation>, // indexed by TreeView node id
    current_file: String,
}

impl ProblemsPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.editor_bg);

        // Header row with error/warning counts
        let header = ui::View::new();
        header.set_dock(ui::DOCK_TOP);
        header.set_size(400, 52);
        header.set_color(tc.toolbar_bg);
        panel.add(&header);

        let accent = ui::View::new();
        accent.set_dock(ui::DOCK_LEFT);
        accent.set_size(3, 52);
        accent.set_color(tc.accent);
        header.add(&accent);

        let error_label = ui::Label::new("0 Errors");
        error_label.set_position(12, 4);
        error_label.set_font_size(11);
        error_label.set_text_color(0xFFF44747); // Red
        header.add(&error_label);

        let warning_label = ui::Label::new("0 Warnings");
        warning_label.set_position(124, 4);
        warning_label.set_font_size(11);
        warning_label.set_text_color(0xFFCCA700); // Yellow
        header.add(&warning_label);

        let visible_label = ui::Label::new("0 Visible");
        visible_label.set_position(244, 4);
        visible_label.set_font_size(11);
        visible_label.set_text_color(tc.text);
        header.add(&visible_label);

        let summary_label = ui::Label::new("No problems");
        summary_label.set_position(340, 4);
        summary_label.set_font_size(11);
        summary_label.set_text_color(tc.text_secondary);
        header.add(&summary_label);

        let btn_all = filter_button("All", 12);
        header.add(&btn_all);

        let btn_errors = filter_button("Errors", 76);
        header.add(&btn_errors);

        let btn_warnings = filter_button("Warnings", 152);
        header.add(&btn_warnings);

        let btn_current_file = filter_button("Current File", 248);
        btn_current_file.set_size(92, 20);
        header.add(&btn_current_file);

        // Tree view for diagnostics
        let tree = ui::TreeView::new(400, 200);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(20);
        panel.add(&tree);

        Self {
            panel,
            tree,
            summary_label,
            error_label,
            warning_label,
            visible_label,
            btn_all,
            btn_errors,
            btn_warnings,
            btn_current_file,
            filter: ProblemFilter::All,
            file_locations: Vec::new(),
            current_file: String::new(),
        }
    }

    pub fn set_filter(&mut self, filter: ProblemFilter) {
        self.filter = filter;
        self.refresh_filter_buttons();
    }

    pub fn filter(&self) -> ProblemFilter {
        self.filter
    }

    pub fn set_current_file(&mut self, file_path: Option<&str>) {
        self.current_file.clear();
        if let Some(path) = file_path {
            self.current_file.push_str(path);
        }
    }

    /// Update the problems panel from the diagnostic set.
    pub fn update(&mut self, diagnostics: &DiagnosticSet) {
        let tc = ui::theme::colors();
        self.tree.clear();
        self.file_locations.clear();
        self.refresh_filter_buttons();

        let errors = diagnostics.error_count();
        let warnings = diagnostics.warning_count();
        let visible_count = diagnostics
            .diagnostics
            .iter()
            .filter(|d| self.filter_accepts(d.severity, &d.file_path))
            .count();

        self.error_label.set_text(&format!(
            "{} Error{}",
            errors,
            if errors == 1 { "" } else { "s" }
        ));
        self.warning_label.set_text(&format!(
            "{} Warning{}",
            warnings,
            if warnings == 1 { "" } else { "s" }
        ));
        self.visible_label
            .set_text(&format!("{} Visible", visible_count,));

        if diagnostics.diagnostics.is_empty() {
            self.summary_label.set_text("No problems");
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No problems detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.remember_location(node, "", 0, 0);
            return;
        }

        if visible_count == 0 {
            self.summary_label.set_text("No matching problems");
            let node = self.tree.add_root("No problems match the current filter");
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.remember_location(node, "", 0, 0);
            return;
        }

        self.summary_label.set_text(&format!(
            "{} - {}",
            diagnostics.summary(),
            self.filter_label()
        ));

        let globals = diagnostics.global();
        let mut visible_globals: Vec<_> = globals
            .into_iter()
            .filter(|diag| self.filter_accepts(diag.severity, &diag.file_path))
            .collect();
        visible_globals.sort_by(|a, b| {
            severity_rank(a.severity)
                .cmp(&severity_rank(b.severity))
                .then(a.source.cmp(&b.source))
                .then(a.message.cmp(&b.message))
        });
        if !visible_globals.is_empty() {
            let root = self
                .tree
                .add_root(&format!("Build messages ({})", visible_globals.len()));
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, tc.text);
            self.remember_location(root, "", 0, 0);
            self.tree.set_expanded(root, true);

            for diag in visible_globals {
                let code_str = match &diag.code {
                    Some(c) => format!(" [{}]", c),
                    None => String::new(),
                };
                let label = format!(
                    "{} [{}] {}{}",
                    severity_marker(diag.severity),
                    diag.source,
                    diag.message,
                    code_str,
                );
                let node = self.tree.add_child(root, &label);
                self.tree
                    .set_node_text_color(node, diag.severity.icon_color());
                self.remember_location(node, "", 0, 0);
            }
        }

        let mut file_nodes: Vec<(String, u32)> = Vec::new();
        let mut visible_file_diags: Vec<_> = diagnostics
            .diagnostics
            .iter()
            .filter(|diag| {
                diag.has_location() && self.filter_accepts(diag.severity, &diag.file_path)
            })
            .collect();
        visible_file_diags.sort_by(|a, b| {
            severity_rank(a.severity)
                .cmp(&severity_rank(b.severity))
                .then(a.file_path.cmp(&b.file_path))
                .then(a.line.cmp(&b.line))
                .then(a.column.cmp(&b.column))
                .then(a.source.cmp(&b.source))
                .then(a.message.cmp(&b.message))
        });

        for diag in &visible_file_diags {
            let file_node = match file_nodes.iter().position(|(p, _)| p == &diag.file_path) {
                Some(idx) => file_nodes[idx].1,
                None => {
                    let basename = crate::util::path::basename(&diag.file_path);
                    let file_diag_count = visible_file_diags
                        .iter()
                        .filter(|d| d.file_path == diag.file_path)
                        .count();
                    let label = format!("{} ({})", basename, file_diag_count);
                    let node = self.tree.add_root(&label);
                    self.tree.set_node_style(node, STYLE_BOLD);
                    self.tree.set_node_text_color(node, tc.text);
                    self.remember_location(node, &diag.file_path, 0, 0);
                    self.tree.set_expanded(node, true);
                    file_nodes.push((diag.file_path.clone(), node));
                    node
                }
            };

            // Build diagnostic line
            let code_str = match &diag.code {
                Some(c) => format!(" [{}]", c),
                None => String::new(),
            };
            let label = format!(
                "{} {} [{}] - {}{}",
                severity_marker(diag.severity),
                range_label(diag.line, diag.column, diag.end_line, diag.end_column),
                diag.source,
                diag.message,
                code_str,
            );
            let node = self.tree.add_child(file_node, &label);
            self.tree
                .set_node_text_color(node, diag.severity.icon_color());
            self.remember_location(node, &diag.file_path, diag.line, diag.column);
        }
    }

    /// Clear all problems.
    pub fn clear(&mut self) {
        self.tree.clear();
        self.file_locations.clear();
        self.error_label.set_text("0 Errors");
        self.warning_label.set_text("0 Warnings");
        self.visible_label.set_text("0 Visible");
        self.summary_label.set_text("No problems");
    }

    /// Get the file path and line for a tree node selection.
    pub fn location_for_node(&self, index: u32) -> Option<(&str, u32, u32)> {
        self.file_locations
            .get(index as usize)
            .filter(|loc| !loc.file_path.is_empty())
            .map(|loc| (loc.file_path.as_str(), loc.line, loc.column))
    }

    fn remember_location(&mut self, node: u32, file_path: &str, line: u32, column: u32) {
        let idx = node as usize;
        while self.file_locations.len() <= idx {
            self.file_locations.push(ProblemLocation {
                file_path: String::new(),
                line: 0,
                column: 0,
            });
        }
        self.file_locations[idx] = ProblemLocation {
            file_path: String::from(file_path),
            line,
            column,
        };
    }

    fn filter_label(&self) -> &'static str {
        match self.filter {
            ProblemFilter::All => "All",
            ProblemFilter::Errors => "Errors",
            ProblemFilter::Warnings => "Warnings",
            ProblemFilter::CurrentFile => "Current File",
        }
    }

    fn filter_accepts(&self, severity: Severity, file_path: &str) -> bool {
        match self.filter {
            ProblemFilter::All => true,
            ProblemFilter::Errors => severity == Severity::Error,
            ProblemFilter::Warnings => severity == Severity::Warning,
            ProblemFilter::CurrentFile => {
                !self.current_file.is_empty()
                    && (file_path == self.current_file
                        || crate::util::path::basename(file_path)
                            == crate::util::path::basename(&self.current_file))
            }
        }
    }

    fn refresh_filter_buttons(&self) {
        mark_filter(&self.btn_all, self.filter == ProblemFilter::All);
        mark_filter(&self.btn_errors, self.filter == ProblemFilter::Errors);
        mark_filter(&self.btn_warnings, self.filter == ProblemFilter::Warnings);
        mark_filter(
            &self.btn_current_file,
            self.filter == ProblemFilter::CurrentFile,
        );
    }
}

fn filter_button(label: &str, x: i32) -> ui::Button {
    let tc = ui::theme::colors();
    let button = ui::Button::new(label);
    button.set_position(x, 24);
    button.set_size(60, 20);
    button.set_font_size(10);
    button.set_color(tc.tab_inactive_bg);
    button.set_text_color(tc.text);
    button
}

fn mark_filter(button: &ui::Button, active: bool) {
    let tc = ui::theme::colors();
    button.set_color(if active {
        tc.accent
    } else {
        tc.tab_inactive_bg
    });
    button.set_text_color(if active { tc.window_bg } else { tc.text });
}

fn severity_marker(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
        Severity::Hint => 3,
    }
}

fn range_label(line: u32, column: u32, end_line: u32, end_column: u32) -> String {
    if end_line > 0 && (end_line != line || end_column != column) {
        format!("Ln {}, Col {}-{}:{}", line, column, end_line, end_column)
    } else {
        format!("Ln {}, Col {}", line, column)
    }
}
