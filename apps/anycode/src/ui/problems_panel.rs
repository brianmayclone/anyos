use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
#[allow(unused_imports)]
use libanyui_client as ui;

use crate::logic::diagnostics::{DiagnosticSet, Severity};

const STYLE_BOLD: u32 = 1;

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
    file_locations: Vec<ProblemLocation>, // indexed by TreeView node id
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
        header.set_size(400, 22);
        header.set_color(tc.sidebar_bg);
        panel.add(&header);

        let error_label = ui::Label::new("0 Errors");
        error_label.set_position(8, 3);
        error_label.set_font_size(11);
        error_label.set_text_color(0xFFF44747); // Red
        header.add(&error_label);

        let warning_label = ui::Label::new("0 Warnings");
        warning_label.set_position(120, 3);
        warning_label.set_font_size(11);
        warning_label.set_text_color(0xFFCCA700); // Yellow
        header.add(&warning_label);

        let summary_label = ui::Label::new("No problems");
        summary_label.set_position(260, 3);
        summary_label.set_font_size(11);
        summary_label.set_text_color(tc.text_secondary);
        header.add(&summary_label);

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
            file_locations: Vec::new(),
        }
    }

    /// Update the problems panel from the diagnostic set.
    pub fn update(&mut self, diagnostics: &DiagnosticSet) {
        let tc = ui::theme::colors();
        self.tree.clear();
        self.file_locations.clear();

        let errors = diagnostics.error_count();
        let warnings = diagnostics.warning_count();

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

        if diagnostics.diagnostics.is_empty() {
            self.summary_label.set_text("No problems");
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No problems detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.remember_location(node, "", 0, 0);
            return;
        }

        self.summary_label.set_text(&diagnostics.summary());

        let globals = diagnostics.global();
        if !globals.is_empty() {
            let root = self
                .tree
                .add_root(&format!("Build messages ({})", globals.len()));
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, tc.text);
            self.remember_location(root, "", 0, 0);
            self.tree.set_expanded(root, true);

            for diag in globals {
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

        for diag in &diagnostics.diagnostics {
            // Skip diagnostics without file location
            if !diag.has_location() {
                continue;
            }

            let file_node = match file_nodes.iter().position(|(p, _)| p == &diag.file_path) {
                Some(idx) => file_nodes[idx].1,
                None => {
                    let basename = crate::util::path::basename(&diag.file_path);
                    let file_diag_count = diagnostics
                        .diagnostics
                        .iter()
                        .filter(|d| d.file_path == diag.file_path && d.has_location())
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
}

fn severity_marker(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

fn range_label(line: u32, column: u32, end_line: u32, end_column: u32) -> String {
    if end_line > 0 && (end_line != line || end_column != column) {
        format!("Ln {}, Col {}-{}:{}", line, column, end_line, end_column)
    } else {
        format!("Ln {}, Col {}", line, column)
    }
}
