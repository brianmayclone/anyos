use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

use crate::util::{path, syntax_map};

const STYLE_BOLD: u32 = 1;
const DIR_COLOR: u32 = 0xFFCCCCCC;
const FILE_COLOR: u32 = 0xFFBBBBBB;

/// Sidebar panel with Explorer (tree view) — tab switching done via activity bar.
pub struct Sidebar {
    pub panel: ui::View,
    pub explorer_panel: ui::View,
    pub search: ui::SearchField,
    pub tree: ui::TreeView,
    paths: Vec<String>,
}

impl Sidebar {
    /// Create the sidebar panel with explorer view.
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(0xFF252526);

        // Explorer panel (header + search + tree)
        let explorer_panel = ui::View::new();
        explorer_panel.set_dock(ui::DOCK_FILL);
        explorer_panel.set_color(0xFF252526);
        panel.add(&explorer_panel);

        let header = ui::Label::new("EXPLORER");
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(0xFF969696);
        header.set_margin(8, 4, 0, 2);
        explorer_panel.add(&header);

        let search = ui::SearchField::new();
        search.set_dock(ui::DOCK_TOP);
        search.set_size(200, 28);
        search.set_margin(4, 0, 4, 4);
        search.set_placeholder("Filter files...");
        explorer_panel.add(&search);

        let tree = ui::TreeView::new(200, 400);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        explorer_panel.add(&tree);

        Self {
            panel,
            explorer_panel,
            search,
            tree,
            paths: Vec::new(),
        }
    }

    /// Populate the tree from a root directory.
    pub fn populate(&mut self, root: &str) {
        self.tree.clear();
        self.paths.clear();

        let dir_name = path::basename(root);
        let root_node = self.tree.add_root(dir_name);
        self.paths.push(String::from(root));
        self.tree.set_node_style(root_node, STYLE_BOLD);
        self.tree.set_node_text_color(root_node, DIR_COLOR);

        self.add_dir_entries(root_node, root, 0);
        self.tree.set_expanded(root_node, true);
    }

    /// Refresh the tree from the same root.
    pub fn refresh(&mut self, root: &str) {
        self.populate(root);
    }

    /// Get the file path for a given node index.
    pub fn path_for_node(&self, index: u32) -> Option<&str> {
        self.paths.get(index as usize).map(|s| s.as_str())
    }

    /// Check if the given node index is a directory.
    pub fn is_directory(&self, index: u32) -> bool {
        match self.path_for_node(index) {
            Some(p) => path::is_directory(p),
            None => false,
        }
    }

    fn add_dir_entries(&mut self, parent_node: u32, dir_path: &str, depth: u32) {
        if depth > 8 {
            return;
        }

        let entries = match anyos_std::fs::read_dir(dir_path) {
            Ok(rd) => rd,
            Err(_) => return,
        };

        let mut dirs: Vec<(String, String)> = Vec::new();
        let mut files: Vec<(String, String)> = Vec::new();

        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            let full = path::join(dir_path, &entry.name);
            if entry.is_dir() {
                dirs.push((entry.name.clone(), full));
            } else {
                files.push((entry.name.clone(), full));
            }
        }

        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, full_path) in &dirs {
            let node = self.tree.add_child(parent_node, name);
            self.paths.push(full_path.clone());
            self.tree.set_node_style(node, STYLE_BOLD);
            self.tree.set_node_text_color(node, DIR_COLOR);
            self.add_dir_entries(node, full_path, depth + 1);
        }

        for (name, full_path) in &files {
            let node = self.tree.add_child(parent_node, name);
            self.paths.push(full_path.clone());

            let icon_color = language_icon_color(syntax_map::language_for_filename(name));
            if icon_color != 0 {
                self.tree.set_node_text_color(node, icon_color);
            } else {
                self.tree.set_node_text_color(node, FILE_COLOR);
            }
        }
    }
}

fn language_icon_color(lang: &str) -> u32 {
    match lang {
        "C" => 0xFF569CD6,
        "Rust" => 0xFFDEA584,
        "Python" => 0xFF4EC9B0,
        "JSON" => 0xFFCE9178,
        "Shell" => 0xFF6A9955,
        "Makefile" => 0xFFDCDCAA,
        "Markdown" => 0xFF9CDCFE,
        _ => 0,
    }
}
