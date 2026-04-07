use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
#[allow(unused_imports)]
use libanyui_client as ui;

use crate::logic::diagnostics::{DiagnosticSet, Severity};

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Problems panel — displays compiler errors and warnings
// ════════════════════════════════════════════════════════════════

pub struct ProblemsPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub summary_label: ui::Label,
    pub error_label: ui::Label,
    pub warning_label: ui::Label,
    file_locations: Vec<(String, u32)>, // (file_path, line) per tree node
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

        self.error_label.set_text(&format!("{} Error{}", errors, if errors == 1 { "" } else { "s" }));
        self.warning_label.set_text(&format!("{} Warning{}", warnings, if warnings == 1 { "" } else { "s" }));

        if diagnostics.diagnostics.is_empty() {
            self.summary_label.set_text("No problems");
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No problems detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.file_locations.push((String::new(), 0));
            return;
        }

        self.summary_label.set_text(&diagnostics.summary());

        // Group by file
        let mut current_file = String::new();
        let mut file_node: u32 = 0;

        for diag in &diagnostics.diagnostics {
            // Skip diagnostics without file location
            if diag.file_path.is_empty() && diag.line == 0 {
                continue;
            }

            if diag.file_path != current_file {
                current_file = diag.file_path.clone();
                let basename = if current_file.is_empty() {
                    "<unknown>"
                } else {
                    crate::util::path::basename(&current_file)
                };
                let file_diags: Vec<_> = diagnostics.diagnostics.iter()
                    .filter(|d| d.file_path == current_file)
                    .collect();
                let label = format!("{} ({})", basename, file_diags.len());
                file_node = self.tree.add_root(&label);
                self.tree.set_node_style(file_node, STYLE_BOLD);
                self.tree.set_node_text_color(file_node, tc.text);
                self.file_locations.push((current_file.clone(), 0));
                self.tree.set_expanded(file_node, true);
            }

            // Build diagnostic line
            let severity_icon = match diag.severity {
                Severity::Error => "\u{2716}", // ✖
                Severity::Warning => "\u{26A0}", // ⚠
                Severity::Info => "\u{2139}", // ℹ
                Severity::Hint => "\u{2022}", // •
            };
            let code_str = match &diag.code {
                Some(c) => format!(" [{}]", c),
                None => String::new(),
            };
            let label = format!("{} Ln {}, Col {} — {}{}",
                severity_icon,
                diag.line, diag.column,
                diag.message,
                code_str,
            );
            let node = self.tree.add_child(file_node, &label);
            self.tree.set_node_text_color(node, diag.severity.icon_color());
            self.file_locations.push((diag.file_path.clone(), diag.line));
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
    pub fn location_for_node(&self, index: u32) -> Option<(&str, u32)> {
        self.file_locations
            .get(index as usize)
            .filter(|(p, _)| !p.is_empty())
            .map(|(p, l)| (p.as_str(), *l))
    }
}
