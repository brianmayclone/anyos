use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;

use crate::logic::search::SearchResult;

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Search panel — project-wide search with results tree
// ════════════════════════════════════════════════════════════════

pub struct SearchPanel {
    pub panel: ui::View,
    pub search_field: ui::TextField,
    pub replace_field: ui::TextField,
    pub btn_search: ui::Button,
    pub btn_replace_all: ui::Button,
    pub results_tree: ui::TreeView,
    pub status_label: ui::Label,
    result_paths: Vec<(String, u32)>, // (file_path, line_number) per tree node
}

impl SearchPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        // Header
        let header = ui::Label::new(t("SEARCH"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        // Search field
        let search_field = ui::TextField::new();
        search_field.set_dock(ui::DOCK_TOP);
        search_field.set_size(200, 28);
        search_field.set_margin(4, 2, 4, 2);
        search_field.set_placeholder(t("Search..."));
        search_field.set_font_size(12);
        search_field.set_color(tc.control_bg);
        search_field.set_text_color(tc.text);
        panel.add(&search_field);

        // Replace field
        let replace_field = ui::TextField::new();
        replace_field.set_dock(ui::DOCK_TOP);
        replace_field.set_size(200, 28);
        replace_field.set_margin(4, 0, 4, 2);
        replace_field.set_placeholder(t("Replace..."));
        replace_field.set_font_size(12);
        replace_field.set_color(tc.control_bg);
        replace_field.set_text_color(tc.text);
        panel.add(&replace_field);

        // Button row
        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 30);
        btn_bar.set_color(tc.sidebar_bg);
        panel.add(&btn_bar);

        let btn_search = ui::Button::new(t("Search"));
        btn_search.set_size(65, 24);
        btn_search.set_color(tc.accent);
        btn_bar.add(&btn_search);

        let btn_replace_all = ui::Button::new(t("Replace All"));
        btn_replace_all.set_size(80, 24);
        btn_bar.add(&btn_replace_all);

        // Status label
        let status_label = ui::Label::new("");
        status_label.set_dock(ui::DOCK_TOP);
        status_label.set_size(200, 16);
        status_label.set_font_size(11);
        status_label.set_text_color(tc.text_secondary);
        status_label.set_margin(8, 2, 0, 2);
        panel.add(&status_label);

        // Results tree
        let results_tree = ui::TreeView::new(200, 300);
        results_tree.set_dock(ui::DOCK_FILL);
        results_tree.set_indent_width(16);
        results_tree.set_row_height(20);
        panel.add(&results_tree);

        Self {
            panel,
            search_field,
            replace_field,
            btn_search,
            btn_replace_all,
            results_tree,
            status_label,
            result_paths: Vec::new(),
        }
    }

    /// Display search results in the tree view.
    pub fn show_results(&mut self, results: &[SearchResult], query: &str) {
        let tc = ui::theme::colors();
        self.results_tree.clear();
        self.result_paths.clear();

        if results.is_empty() {
            self.status_label.set_text(&format!("No results for \"{}\"", query));
            return;
        }

        self.status_label.set_text(&format!("{} results in {} files",
            results.len(),
            count_unique_files(results),
        ));

        // Group results by file
        let mut current_file = String::new();
        let mut file_node: u32 = 0;
        let mut file_count: u32 = 0;

        for result in results {
            if result.file_path != current_file {
                // New file group
                current_file = result.file_path.clone();
                file_count = results.iter().filter(|r| r.file_path == current_file).count() as u32;

                let basename = crate::util::path::basename(&result.file_path);
                let label = format!("{} ({})", basename, file_count);
                file_node = self.results_tree.add_root(&label);
                self.results_tree.set_node_style(file_node, STYLE_BOLD);
                self.results_tree.set_node_text_color(file_node, tc.text);
                self.result_paths.push((result.file_path.clone(), 0));
                self.results_tree.set_expanded(file_node, true);
            }

            // Add match line
            let line_label = format!("  {}:  {}", result.line_number, result.line_text.trim());
            let node = self.results_tree.add_child(file_node, &line_label);
            self.results_tree.set_node_text_color(node, tc.text_secondary);
            self.result_paths.push((result.file_path.clone(), result.line_number));
        }
    }

    /// Clear all search results.
    pub fn clear(&mut self) {
        self.results_tree.clear();
        self.result_paths.clear();
        self.status_label.set_text("");
    }

    /// Get the file path and line number for a tree node selection.
    pub fn path_for_node(&self, index: u32) -> Option<(&str, u32)> {
        self.result_paths.get(index as usize).map(|(p, l)| (p.as_str(), *l))
    }

    /// Get the search query text.
    pub fn get_query(&self) -> String {
        let mut buf = [0u8; 256];
        let len = self.search_field.get_text(&mut buf);
        if len == 0 {
            return String::new();
        }
        match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => String::from(s),
            Err(_) => String::new(),
        }
    }

    /// Get the replace text.
    pub fn get_replace_text(&self) -> String {
        let mut buf = [0u8; 256];
        let len = self.replace_field.get_text(&mut buf);
        if len == 0 {
            return String::new();
        }
        match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => String::from(s),
            Err(_) => String::new(),
        }
    }
}

fn count_unique_files(results: &[SearchResult]) -> usize {
    let mut files: Vec<&str> = Vec::new();
    for r in results {
        if !files.contains(&r.file_path.as_str()) {
            files.push(&r.file_path);
        }
    }
    files.len()
}
