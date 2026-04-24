use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

use crate::logic::project::{Project, TargetKind};
use crate::logic::tasks::{TaskCategory, TaskManager};
use crate::util::{path, syntax_map};

const STYLE_BOLD: u32 = 1;

/// Simple icon cache: stores decoded 16x16 ARGB icons keyed by file path.
struct IconCache {
    entries: Vec<(String, Vec<u32>, u32, u32)>, // (icon_path, pixels, w, h)
}

impl IconCache {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Get or load an icon from an ICO file. Returns (pixels, w, h) or None.
    fn get_or_load(&mut self, icon_path: &str) -> Option<(&[u32], u32, u32)> {
        // Check cache first
        if let Some(idx) = self.entries.iter().position(|e| e.0 == icon_path) {
            let e = &self.entries[idx];
            return Some((&e.1, e.2, e.3));
        }
        // Load from disk
        if let Some(icon) = ui::Icon::load(icon_path, 16) {
            self.entries.push((
                String::from(icon_path),
                icon.pixels,
                icon.width,
                icon.height,
            ));
            let e = self.entries.last().unwrap();
            Some((&e.1, e.2, e.3))
        } else {
            None
        }
    }
}

/// Sidebar panel with Explorer (tree view) — tab switching done via activity bar.
pub struct Sidebar {
    pub panel: ui::View,
    pub explorer_panel: ui::View,
    pub search: ui::SearchField,
    pub tree: ui::TreeView,
    pub context_menu: ui::ContextMenu,
    pub rename_field: ui::TextField,
    pub rename_node: u32,
    pub paths: Vec<String>,
    pub virtual_nodes: Vec<bool>,
    mime_db: anyos_std::icons::MimeDb,
    icon_cache: IconCache,
}

impl Sidebar {
    /// Create the sidebar panel with explorer view.
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        // Explorer panel (header + search + tree)
        let explorer_panel = ui::View::new();
        explorer_panel.set_dock(ui::DOCK_FILL);
        explorer_panel.set_color(tc.sidebar_bg);
        panel.add(&explorer_panel);

        let t = anyos_std::i18n::t;
        let header = ui::Label::new(t("EXPLORER"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 4, 0, 2);
        explorer_panel.add(&header);

        let search = ui::SearchField::new();
        search.set_dock(ui::DOCK_TOP);
        search.set_size(200, 28);
        search.set_margin(4, 0, 4, 4);
        search.set_placeholder(t("Filter files..."));
        explorer_panel.add(&search);

        let tree = ui::TreeView::new(200, 400);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        tree.set_draggable(true);
        tree.set_drop_target(true);
        explorer_panel.add(&tree);

        // Context menu for folders
        let ctx_items = alloc::format!("{}|{}|-|{}", t("New File"), t("New Folder"), t("Delete"));
        let context_menu = ui::ContextMenu::new(&ctx_items);
        tree.set_context_menu(&context_menu);
        explorer_panel.add(&context_menu);

        // Inline rename field (hidden by default, DOCK_TOP overlay)
        let rename_field = ui::TextField::new();
        rename_field.set_size(200, 22);
        rename_field.set_font_size(12);
        rename_field.set_color(tc.control_bg);
        rename_field.set_text_color(tc.text);
        rename_field.set_visible(false);
        explorer_panel.add(&rename_field);

        Self {
            panel,
            explorer_panel,
            search,
            tree,
            context_menu,
            rename_field,
            rename_node: u32::MAX,
            paths: Vec::new(),
            virtual_nodes: Vec::new(),
            mime_db: anyos_std::icons::MimeDb::load(),
            icon_cache: IconCache::new(),
        }
    }

    pub fn populate_project(&mut self, project: &Project, task_mgr: &TaskManager) {
        self.tree.clear();
        self.paths.clear();
        self.virtual_nodes.clear();

        let tc = ui::theme::colors();
        let solution_label = format!("Solution '{}'", project.name);
        let solution = self.tree.add_root(&solution_label);
        self.remember_virtual(solution);
        self.tree.set_node_style(solution, STYLE_BOLD);
        self.tree.set_node_text_color(solution, tc.text);
        self.tree.set_expanded(solution, true);

        let project_label = format!("{} ({})", project.name, project.project_type.display_name());
        let project_node = self.tree.add_child(solution, &project_label);
        self.remember_virtual(project_node);
        self.tree.set_node_style(project_node, STYLE_BOLD);
        self.tree.set_node_text_color(project_node, tc.text);
        self.tree.set_expanded(project_node, true);
        self.set_folder_icon(project_node);

        self.add_config_nodes(project_node, project);
        self.add_target_nodes(project_node, project);
        self.add_task_nodes(project_node, task_mgr);

        let files_node = self.tree.add_child(project_node, "Files");
        self.remember_virtual(files_node);
        self.tree.set_node_style(files_node, STYLE_BOLD);
        self.tree.set_node_text_color(files_node, tc.text);
        self.set_folder_icon(files_node);
        self.tree.set_expanded(files_node, true);

        let root_name = path::basename(&project.root);
        let root_node = self.tree.add_child(files_node, root_name);
        self.remember_path(root_node, &project.root, false);
        self.tree.set_node_style(root_node, STYLE_BOLD);
        self.tree.set_node_text_color(root_node, tc.text);
        self.set_folder_icon(root_node);
        self.add_dir_entries(root_node, &project.root, 0);
        self.tree.set_expanded(root_node, true);
    }

    /// Populate the tree from a root directory.
    pub fn populate(&mut self, root: &str) {
        self.tree.clear();
        self.paths.clear();
        self.virtual_nodes.clear();

        let dir_name = path::basename(root);
        let tc = ui::theme::colors();
        let root_node = self.tree.add_root(dir_name);
        self.remember_path(root_node, root, false);
        self.tree.set_node_style(root_node, STYLE_BOLD);
        self.tree.set_node_text_color(root_node, tc.text);
        // Folder icon for root
        self.set_folder_icon(root_node);

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

    pub fn is_virtual_node(&self, index: u32) -> bool {
        self.virtual_nodes
            .get(index as usize)
            .copied()
            .unwrap_or(true)
    }

    pub fn is_file_node(&self, index: u32) -> bool {
        !self.is_virtual_node(index)
            && self
                .path_for_node(index)
                .map(|p| !p.is_empty() && !path::is_directory(p))
                .unwrap_or(false)
    }

    /// Check if the given node index is a directory.
    pub fn is_directory(&self, index: u32) -> bool {
        if self.is_virtual_node(index) {
            return false;
        }
        match self.path_for_node(index) {
            Some(p) => path::is_directory(p),
            None => false,
        }
    }

    /// Get the directory path for context menu actions.
    /// If selected node is a file, returns its parent directory.
    pub fn selected_dir(&self) -> Option<String> {
        let sel = self.tree.selected();
        if sel == u32::MAX {
            return None;
        }
        match self.path_for_node(sel) {
            Some(p) => {
                if self.is_virtual_node(sel) {
                    return None;
                }
                if path::is_directory(p) {
                    Some(String::from(p))
                } else {
                    Some(String::from(path::parent(p)))
                }
            }
            None => None,
        }
    }

    /// Path of the currently hovered node during drag-and-drop.
    pub fn hovered_path(&self) -> Option<String> {
        let hovered = self.tree.hovered();
        if hovered == u32::MAX {
            return None;
        }
        self.path_for_node(hovered).map(String::from)
    }

    /// Resolve the current drag target directory from the hovered node.
    /// Directories accept drops directly; files route to their parent directory.
    pub fn hovered_drop_dir(&self) -> Option<String> {
        let hovered = self.tree.hovered();
        if hovered == u32::MAX {
            return None;
        }
        match self.path_for_node(hovered) {
            Some(_) if self.is_virtual_node(hovered) => None,
            Some(p) if path::is_directory(p) => Some(String::from(p)),
            Some(p) => Some(String::from(path::parent(p))),
            None => None,
        }
    }

    pub fn begin_drag_from_selection(&self) {
        let sel = self.tree.selected();
        if sel == u32::MAX {
            ui::drag_set_text("");
            return;
        }
        if let Some(p) = self.path_for_node(sel) {
            if self.is_virtual_node(sel) {
                ui::drag_set_text("");
            } else {
                ui::drag_set_text(p);
            }
        } else {
            ui::drag_set_text("");
        }
    }

    pub fn move_drag_payload_to_hovered_dir(&self) -> Option<(String, String)> {
        let src = ui::drag_get_text();
        if src.is_empty() {
            return None;
        }

        let dest_dir = self.hovered_drop_dir()?;
        if src == dest_dir {
            return None;
        }

        let name = path::basename(&src);
        let dest_path = path::join(&dest_dir, name);
        if dest_path == src {
            return None;
        }
        if dest_path.starts_with(&src) && path::is_directory(&src) {
            return None;
        }
        if path::exists(&dest_path) {
            return None;
        }

        if anyos_std::fs::rename(&src, &dest_path) == 0 {
            Some((src, dest_path))
        } else {
            None
        }
    }

    /// Start inline rename for the currently selected node.
    pub fn start_rename(&mut self) {
        let sel = self.tree.selected();
        if sel == u32::MAX {
            return;
        }
        let name = match self.path_for_node(sel) {
            Some(p) if !self.is_virtual_node(sel) => String::from(path::basename(p)),
            None => return,
            _ => return,
        };
        self.rename_node = sel;
        self.rename_field.set_text(&name);
        self.rename_field.set_visible(true);
        self.rename_field.focus();
    }

    /// Complete inline rename — called when the rename field is submitted.
    /// Returns the new full path if rename succeeded, or None.
    pub fn finish_rename(&mut self) -> Option<(String, String)> {
        let node = self.rename_node;
        self.rename_node = u32::MAX;
        self.rename_field.set_visible(false);

        if node == u32::MAX {
            return None;
        }

        let mut buf = [0u8; 256];
        let len = self.rename_field.get_text(&mut buf);
        if len == 0 {
            return None;
        }
        let new_name = match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => s.trim(),
            Err(_) => return None,
        };
        if new_name.is_empty() {
            return None;
        }

        let old_path = match self.path_for_node(node) {
            Some(p) => String::from(p),
            None => return None,
        };
        let dir = path::parent(&old_path);
        let new_path = path::join(dir, new_name);

        if anyos_std::fs::rename(&old_path, &new_path) == 0 {
            Some((old_path, new_path))
        } else {
            None
        }
    }

    /// Filter the tree to show only files matching the filter string.
    pub fn filter_tree(&mut self, filter: &str) {
        if self.paths.is_empty() {
            return;
        }
        // Get root path (index 0)
        let root = self.paths[0].clone();
        self.tree.clear();
        self.paths.clear();
        self.virtual_nodes.clear();

        let tc = ui::theme::colors();
        let dir_name = path::basename(&root);
        let root_node = self.tree.add_root(dir_name);
        self.remember_path(root_node, &root, false);
        self.tree.set_node_style(root_node, STYLE_BOLD);
        self.tree.set_node_text_color(root_node, tc.text);
        self.set_folder_icon(root_node);

        // Recursively add only files matching filter
        self.add_filtered_entries(root_node, &root, filter, 0);
        self.tree.set_expanded(root_node, true);
    }

    fn add_filtered_entries(&mut self, parent_node: u32, dir_path: &str, filter: &str, depth: u32) {
        if depth > 8 {
            return;
        }
        let entries = match anyos_std::fs::read_dir(dir_path) {
            Ok(rd) => rd,
            Err(_) => return,
        };

        let filter_lower = ascii_lower(filter);
        let tc = ui::theme::colors();

        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            if entry.is_dir() && is_hidden_dir(&entry.name) {
                continue;
            }

            let full = path::join(dir_path, &entry.name);

            if entry.is_dir() {
                let node = self.tree.add_child(parent_node, &entry.name);
                self.remember_path(node, &full, false);
                self.tree.set_node_style(node, STYLE_BOLD);
                self.tree.set_node_text_color(node, tc.text);
                self.set_folder_icon(node);
                self.add_filtered_entries(node, &full, filter, depth + 1);
                self.tree.set_expanded(node, true);
            } else {
                let name_lower = ascii_lower(&entry.name);
                if name_lower.contains(filter_lower.as_str()) {
                    let node = self.tree.add_child(parent_node, &entry.name);
                    self.remember_path(node, &full, false);
                    let icon_color =
                        language_icon_color(syntax_map::language_for_filename(&entry.name));
                    if icon_color != 0 {
                        self.tree.set_node_text_color(node, icon_color);
                    } else {
                        self.tree.set_node_text_color(node, tc.text_secondary);
                    }
                    self.set_file_icon(node, &entry.name);
                }
            }
        }
    }

    /// Cancel inline rename.
    pub fn cancel_rename(&mut self) {
        self.rename_node = u32::MAX;
        self.rename_field.set_visible(false);
    }

    /// Set a folder icon on a tree node.
    fn set_folder_icon(&mut self, node: u32) {
        if let Some((pixels, w, h)) = self.icon_cache.get_or_load(anyos_std::icons::FOLDER_ICON) {
            self.tree.set_node_icon(node, pixels, w, h);
        }
    }

    /// Set a file icon on a tree node based on its filename extension.
    fn set_file_icon(&mut self, node: u32, filename: &str) {
        let ext = match filename.rsplit('.').next() {
            Some(e) if e != filename => e,
            _ => return, // no extension
        };
        let icon_path = self.mime_db.icon_for_ext(ext);
        if let Some((pixels, w, h)) = self.icon_cache.get_or_load(icon_path) {
            self.tree.set_node_icon(node, pixels, w, h);
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
            // Hide common build/VCS directories
            if entry.is_dir() && is_hidden_dir(&entry.name) {
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

        let tc = ui::theme::colors();
        for (name, full_path) in &dirs {
            let node = self.tree.add_child(parent_node, name);
            self.remember_path(node, full_path, false);
            self.tree.set_node_style(node, STYLE_BOLD);
            self.tree.set_node_text_color(node, tc.text);
            self.set_folder_icon(node);
            self.add_dir_entries(node, full_path, depth + 1);
        }

        for (name, full_path) in &files {
            let node = self.tree.add_child(parent_node, name);
            self.remember_path(node, full_path, false);

            let icon_color = language_icon_color(syntax_map::language_for_filename(name));
            if icon_color != 0 {
                self.tree.set_node_text_color(node, icon_color);
            } else {
                self.tree.set_node_text_color(node, tc.text_secondary);
            }
            self.set_file_icon(node, name);
        }
    }

    fn add_config_nodes(&mut self, project_node: u32, project: &Project) {
        let tc = ui::theme::colors();
        let root = self.tree.add_child(project_node, "Configurations");
        self.remember_virtual(root);
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.text);
        self.tree.set_expanded(root, true);

        for config in &project.configurations {
            let prefix = if *config == project.active_configuration {
                "* "
            } else {
                "  "
            };
            let label = format!("{}{}", prefix, config.display_name());
            let node = self.tree.add_child(root, &label);
            self.remember_virtual(node);
            self.tree.set_node_text_color(
                node,
                if *config == project.active_configuration {
                    tc.accent
                } else {
                    tc.text_secondary
                },
            );
        }
    }

    fn add_target_nodes(&mut self, project_node: u32, project: &Project) {
        let tc = ui::theme::colors();
        let count = project.target_count();
        let label = format!("Targets ({})", count);
        let root = self.tree.add_child(project_node, &label);
        self.remember_virtual(root);
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.text);
        self.tree.set_expanded(root, true);

        for target in &project.cargo_targets {
            let label = format!("{} {}", target.kind.label(), target.name);
            let node = self.tree.add_child(root, &label);
            self.remember_virtual(node);
            self.tree
                .set_node_text_color(node, target_color(&target.kind));
        }

        for member in &project.workspace_members {
            let member_label = format!("member {}", member.name);
            let member_node = self.tree.add_child(root, &member_label);
            self.remember_virtual(member_node);
            self.tree.set_node_style(member_node, STYLE_BOLD);
            self.tree
                .set_node_text_color(member_node, tc.text_secondary);
            for target in &member.targets {
                let label = format!("{} {}", target.kind.label(), target.name);
                let node = self.tree.add_child(member_node, &label);
                self.remember_virtual(node);
                self.tree
                    .set_node_text_color(node, target_color(&target.kind));
            }
            self.tree.set_expanded(member_node, true);
        }

        for target in &project.make_targets {
            let label = if target.is_phony {
                format!("make {} (phony)", target.name)
            } else {
                format!("make {}", target.name)
            };
            let node = self.tree.add_child(root, &label);
            self.remember_virtual(node);
            self.tree.set_node_text_color(node, tc.text_secondary);
        }

        for script in &project.npm_scripts {
            let label = format!("npm {} - {}", script.name, script.command);
            let node = self.tree.add_child(root, &label);
            self.remember_virtual(node);
            self.tree.set_node_text_color(node, tc.text_secondary);
        }
    }

    fn add_task_nodes(&mut self, project_node: u32, task_mgr: &TaskManager) {
        let tc = ui::theme::colors();
        let label = format!("Build & Run ({})", task_mgr.tasks.len());
        let root = self.tree.add_child(project_node, &label);
        self.remember_virtual(root);
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.text);
        self.tree.set_expanded(root, true);

        for category in [
            TaskCategory::Build,
            TaskCategory::Run,
            TaskCategory::Test,
            TaskCategory::Check,
            TaskCategory::Clean,
            TaskCategory::Custom,
        ] {
            let tasks = task_mgr.tasks_by_category(category);
            if tasks.is_empty() {
                continue;
            }
            let cat_label = format!("{} ({})", category.label(), tasks.len());
            let cat_node = self.tree.add_child(root, &cat_label);
            self.remember_virtual(cat_node);
            self.tree.set_node_style(cat_node, STYLE_BOLD);
            self.tree.set_node_text_color(cat_node, tc.text_secondary);
            self.tree.set_expanded(cat_node, true);
            for task in tasks {
                let node = self.tree.add_child(cat_node, &task.display_label);
                self.remember_virtual(node);
                self.tree.set_node_text_color(node, tc.text_secondary);
            }
        }
    }

    fn remember_path(&mut self, node: u32, path: &str, is_virtual: bool) {
        let idx = node as usize;
        while self.paths.len() <= idx {
            self.paths.push(String::new());
        }
        while self.virtual_nodes.len() <= idx {
            self.virtual_nodes.push(true);
        }
        self.paths[idx] = String::from(path);
        self.virtual_nodes[idx] = is_virtual;
    }

    fn remember_virtual(&mut self, node: u32) {
        self.remember_path(node, "", true);
    }
}

fn target_color(kind: &TargetKind) -> u32 {
    match kind {
        TargetKind::Binary => 0xFF4EC9B0,
        TargetKind::Library => 0xFF569CD6,
        TargetKind::Example => 0xFFDCDCAA,
        TargetKind::Test => 0xFFB5CEA8,
        TargetKind::Bench => 0xFFC586C0,
    }
}

fn ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            out.push((c as u8 + 32) as char);
        } else {
            out.push(c);
        }
    }
    out
}

/// Directories to hide from the explorer tree.
fn is_hidden_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | ".vscode"
            | ".idea"
            | "target"
            | "build"
            | "node_modules"
            | "__pycache__"
            | ".cache"
            | ".npm"
            | "dist"
            | ".next"
    )
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
