use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;

use crate::logic::git::{GitState, FileStatus, ChangedFile};

const COLOR_MODIFIED: u32 = 0xFFE2B93D;
const COLOR_ADDED: u32 = 0xFF73C991;
const COLOR_DELETED: u32 = 0xFFE06C75;
const COLOR_UNTRACKED: u32 = 0xFF808080;
const COLOR_CONFLICTED: u32 = 0xFFE06C75;
const STYLE_BOLD: u32 = 1;

/// VS Code-style Source Control panel.
pub struct GitPanel {
    pub panel: ui::View,
    pub branch_label: ui::Label,
    pub commit_field: ui::TextField,
    pub btn_commit: ui::Button,
    pub btn_stage_all: ui::Button,
    pub btn_push: ui::Button,
    pub btn_pull: ui::Button,
    pub btn_refresh: ui::Button,
    pub tree: ui::TreeView,
    file_paths: Vec<String>,
}

impl GitPanel {
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(0xFF252526);

        // Header
        let header = ui::Label::new("SOURCE CONTROL");
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(0xFF969696);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        // Branch label
        let branch_label = ui::Label::new("");
        branch_label.set_dock(ui::DOCK_TOP);
        branch_label.set_size(200, 18);
        branch_label.set_font_size(11);
        branch_label.set_text_color(0xFFCCCCCC);
        branch_label.set_margin(8, 2, 0, 4);
        panel.add(&branch_label);

        // Commit message field
        let commit_field = ui::TextField::new();
        commit_field.set_dock(ui::DOCK_TOP);
        commit_field.set_size(200, 28);
        commit_field.set_margin(4, 2, 4, 2);
        commit_field.set_placeholder("Commit message...");
        panel.add(&commit_field);

        // Button row
        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 32);
        btn_bar.set_color(0xFF252526);
        panel.add(&btn_bar);

        let btn_commit = ui::Button::new("Commit");
        btn_commit.set_size(55, 24);
        btn_commit.set_color(0xFF0E639C);
        btn_bar.add(&btn_commit);

        let btn_stage_all = ui::Button::new("Stage All");
        btn_stage_all.set_size(60, 24);
        btn_bar.add(&btn_stage_all);

        let btn_push = ui::Button::new("Push");
        btn_push.set_size(40, 24);
        btn_bar.add(&btn_push);

        let btn_pull = ui::Button::new("Pull");
        btn_pull.set_size(40, 24);
        btn_bar.add(&btn_pull);

        let btn_refresh = ui::Button::new("Refresh");
        btn_refresh.set_size(55, 24);
        btn_bar.add(&btn_refresh);

        // Tree view for changed files
        let tree = ui::TreeView::new(200, 300);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        panel.add(&tree);

        Self {
            panel,
            branch_label,
            commit_field,
            btn_commit,
            btn_stage_all,
            btn_push,
            btn_pull,
            btn_refresh,
            tree,
            file_paths: Vec::new(),
        }
    }

    /// Update the panel from a GitState snapshot.
    pub fn update(&mut self, state: &GitState) {
        // Branch name
        if state.branch.is_empty() {
            self.branch_label.set_text("No branch");
        } else {
            let text = format!("Branch: {}", state.branch);
            self.branch_label.set_text(&text);
        }

        // Rebuild tree
        self.tree.clear();
        self.file_paths.clear();

        let staged: Vec<&ChangedFile> = state.changed_files.iter().filter(|f| f.staged).collect();
        let unstaged: Vec<&ChangedFile> = state.changed_files.iter().filter(|f| !f.staged).collect();

        // Staged changes section
        if !staged.is_empty() {
            let label = format!("Staged Changes ({})", staged.len());
            let root = self.tree.add_root(&label);
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, 0xFFCCCCCC);
            self.file_paths.push(String::new());

            for f in &staged {
                let ch = status_char(f.status);
                let label = format!("{} {}", ch, f.path);
                let node = self.tree.add_child(root, &label);
                self.tree.set_node_text_color(node, status_color(f.status));
                self.file_paths.push(f.path.clone());
            }
            self.tree.set_expanded(root, true);
        }

        // Changes (unstaged) section
        if !unstaged.is_empty() {
            let label = format!("Changes ({})", unstaged.len());
            let root = self.tree.add_root(&label);
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, 0xFFCCCCCC);
            self.file_paths.push(String::new());

            for f in &unstaged {
                let ch = status_char(f.status);
                let label = format!("{} {}", ch, f.path);
                let node = self.tree.add_child(root, &label);
                self.tree.set_node_text_color(node, status_color(f.status));
                self.file_paths.push(f.path.clone());
            }
            self.tree.set_expanded(root, true);
        }

        if staged.is_empty() && unstaged.is_empty() && state.is_repo {
            let node = self.tree.add_root("No changes detected");
            self.tree.set_node_text_color(node, 0xFF808080);
            self.file_paths.push(String::new());
        }
    }

    /// Get the file path for a tree node index.
    pub fn path_for_node(&self, index: u32) -> Option<&str> {
        self.file_paths
            .get(index as usize)
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
    }

    /// Show a "git not installed" message.
    pub fn show_not_installed(&mut self) {
        self.branch_label.set_text("git not found");
        self.tree.clear();
        self.file_paths.clear();
        let node = self.tree.add_root("Install git to enable source control");
        self.tree.set_node_text_color(node, 0xFF808080);
        self.file_paths.push(String::new());
    }

    /// Show a "no repo" message.
    pub fn show_no_repo(&mut self) {
        self.branch_label.set_text("No repository");
        self.tree.clear();
        self.file_paths.clear();
        let node = self.tree.add_root("Open a folder with a git repository");
        self.tree.set_node_text_color(node, 0xFF808080);
        self.file_paths.push(String::new());
    }
}

fn status_char(s: FileStatus) -> char {
    match s {
        FileStatus::Modified => 'M',
        FileStatus::Added => 'A',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
        FileStatus::Untracked => '?',
        FileStatus::Conflicted => 'U',
    }
}

fn status_color(s: FileStatus) -> u32 {
    match s {
        FileStatus::Modified => COLOR_MODIFIED,
        FileStatus::Added => COLOR_ADDED,
        FileStatus::Deleted => COLOR_DELETED,
        FileStatus::Renamed => COLOR_MODIFIED,
        FileStatus::Untracked => COLOR_UNTRACKED,
        FileStatus::Conflicted => COLOR_CONFLICTED,
    }
}
