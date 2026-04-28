use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;
use ui::IconType;

use crate::logic::git::{ChangedFile, FileStatus, GitState};

const STYLE_BOLD: u32 = 1;

/// VS Code-style Source Control panel.
pub struct GitPanel {
    pub panel: ui::View,
    pub branch_label: ui::Label,
    pub repo_label: ui::Label,
    pub changes_label: ui::Label,
    pub commit_field: ui::TextField,
    pub btn_commit: ui::PlainButton,
    pub btn_stage_all: ui::PlainButton,
    pub btn_init: ui::PlainButton,
    pub btn_push: ui::PlainButton,
    pub btn_pull: ui::PlainButton,
    pub btn_refresh: ui::PlainButton,
    pub tree: ui::TreeView,
    pub timeline_tree: ui::TreeView,
    file_paths: Vec<String>,
}

impl GitPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        // Header
        let t = anyos_std::i18n::t;
        let header = ui::Label::new(t("SOURCE CONTROL"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        let repo_bar = ui::FlowPanel::new();
        repo_bar.set_dock(ui::DOCK_TOP);
        repo_bar.set_size(200, 30);
        repo_bar.set_color(tc.sidebar_bg);
        repo_bar.set_padding(6, 0, 6, 0);
        panel.add(&repo_bar);

        let repo_label = ui::Label::new(t("Repository"));
        repo_label.set_size(88, 26);
        repo_label.set_font_size(11);
        repo_label.set_text_color(tc.text_secondary);
        repo_bar.add(&repo_label);

        let btn_refresh = icon_button("refresh", tc.text_secondary, t("Refresh"));
        repo_bar.add(&btn_refresh);

        let btn_pull = icon_button("arrow-down-to-line", tc.text_secondary, t("Pull"));
        repo_bar.add(&btn_pull);

        let btn_push = icon_button("arrow-up-from-line", tc.text_secondary, t("Push"));
        repo_bar.add(&btn_push);

        // Branch label
        let branch_label = ui::Label::new("");
        branch_label.set_dock(ui::DOCK_TOP);
        branch_label.set_size(200, 20);
        branch_label.set_font_size(11);
        branch_label.set_text_color(tc.text);
        branch_label.set_margin(8, 0, 0, 5);
        panel.add(&branch_label);

        // Commit message field
        let commit_field = ui::TextField::new();
        commit_field.set_dock(ui::DOCK_TOP);
        commit_field.set_size(200, 28);
        commit_field.set_margin(4, 2, 4, 2);
        commit_field.set_placeholder(t("Commit message..."));
        panel.add(&commit_field);

        // Button row
        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 32);
        btn_bar.set_color(tc.sidebar_bg);
        btn_bar.set_padding(6, 0, 4, 0);
        panel.add(&btn_bar);

        let btn_commit = action_button(
            "git-commit",
            t("Commit"),
            tc.accent,
            78,
            t("Commit staged changes"),
        );
        btn_commit.set_tooltip(t("Commit staged changes"));
        btn_bar.add(&btn_commit);

        let btn_stage_all =
            action_button("plus", t("Stage All"), tc.text, 92, t("Stage all changes"));
        btn_stage_all.set_tooltip(t("Stage all changes"));
        btn_bar.add(&btn_stage_all);

        let changes_label = ui::Label::new(t("CHANGES"));
        changes_label.set_dock(ui::DOCK_TOP);
        changes_label.set_size(200, 20);
        changes_label.set_font_size(10);
        changes_label.set_text_color(tc.text_secondary);
        changes_label.set_margin(8, 6, 0, 0);
        panel.add(&changes_label);

        let init_bar = ui::FlowPanel::new();
        init_bar.set_dock(ui::DOCK_TOP);
        init_bar.set_size(200, 30);
        init_bar.set_color(tc.sidebar_bg);
        init_bar.set_padding(6, 0, 6, 0);
        panel.add(&init_bar);

        let btn_init = action_button(
            "folder-git-2",
            t("Initialize Repository"),
            tc.accent,
            178,
            t("Create a Git repository in the current project root"),
        );
        init_bar.add(&btn_init);

        // Repository timeline graph at the bottom of source control.
        let timeline_panel = ui::View::new();
        timeline_panel.set_dock(ui::DOCK_BOTTOM);
        timeline_panel.set_size(200, 170);
        timeline_panel.set_color(tc.sidebar_bg);
        panel.add(&timeline_panel);

        let timeline_header = ui::Label::new(t("TIMELINE"));
        timeline_header.set_dock(ui::DOCK_TOP);
        timeline_header.set_size(200, 20);
        timeline_header.set_font_size(11);
        timeline_header.set_text_color(tc.text_secondary);
        timeline_header.set_margin(8, 6, 0, 2);
        timeline_panel.add(&timeline_header);

        let timeline_tree = ui::TreeView::new(200, 140);
        timeline_tree.set_dock(ui::DOCK_FILL);
        timeline_tree.set_indent_width(12);
        timeline_tree.set_row_height(20);
        timeline_panel.add(&timeline_tree);

        // Tree view for changed files
        let tree = ui::TreeView::new(200, 300);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        panel.add(&tree);

        Self {
            panel,
            branch_label,
            repo_label,
            changes_label,
            commit_field,
            btn_commit,
            btn_stage_all,
            btn_init,
            btn_push,
            btn_pull,
            btn_refresh,
            tree,
            timeline_tree,
            file_paths: Vec::new(),
        }
    }

    /// Update the panel from a GitState snapshot.
    pub fn update(&mut self, state: &GitState) {
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;

        // Branch name
        if state.branch.is_empty() {
            self.branch_label.set_text(t("No branch"));
        } else {
            let text = format!("{}: {}", t("Branch"), state.branch);
            self.branch_label.set_text(&text);
        }
        self.repo_label.set_text(if state.is_repo {
            t("Repository")
        } else {
            t("No repository")
        });
        self.btn_init.set_enabled(!state.is_repo);
        self.btn_commit.set_enabled(state.is_repo);
        self.btn_stage_all.set_enabled(state.is_repo);
        self.btn_pull.set_enabled(state.is_repo);
        self.btn_push.set_enabled(state.is_repo);
        self.btn_refresh.set_enabled(state.is_repo);

        // Rebuild tree
        self.tree.clear();
        self.file_paths.clear();

        let staged: Vec<&ChangedFile> = state.changed_files.iter().filter(|f| f.staged).collect();
        let unstaged: Vec<&ChangedFile> =
            state.changed_files.iter().filter(|f| !f.staged).collect();
        self.changes_label
            .set_text(&format!("{}  {}", t("CHANGES"), state.changed_files.len()));

        // Staged changes section
        if !staged.is_empty() {
            let label = format!("{} ({})", t("Staged Changes"), staged.len());
            let root = self.tree.add_root(&label);
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, tc.text);
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
            let label = format!("{} ({})", t("Changes"), unstaged.len());
            let root = self.tree.add_root(&label);
            self.tree.set_node_style(root, STYLE_BOLD);
            self.tree.set_node_text_color(root, tc.text);
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
            let node = self.tree.add_root(t("No changes detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.file_paths.push(String::new());
        }

        self.update_timeline(state);
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
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;
        self.branch_label.set_text(t("git not found"));
        self.repo_label.set_text(t("Source Control"));
        self.changes_label.set_text(t("CHANGES"));
        self.btn_init.set_enabled(false);
        self.btn_commit.set_enabled(false);
        self.btn_stage_all.set_enabled(false);
        self.btn_pull.set_enabled(false);
        self.btn_push.set_enabled(false);
        self.btn_refresh.set_enabled(false);
        self.tree.clear();
        self.file_paths.clear();
        let node = self
            .tree
            .add_root(t("Install git to enable source control"));
        self.tree.set_node_text_color(node, tc.text_secondary);
        self.file_paths.push(String::new());
        self.timeline_tree.clear();
        let tnode = self.timeline_tree.add_root(t("No timeline available"));
        self.timeline_tree
            .set_node_text_color(tnode, tc.text_secondary);
    }

    /// Show a "no repo" message.
    pub fn show_no_repo(&mut self) {
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;
        self.branch_label.set_text(t("No repository"));
        self.repo_label.set_text(t("No repository"));
        self.changes_label.set_text(t("CHANGES"));
        self.btn_init.set_enabled(true);
        self.btn_commit.set_enabled(false);
        self.btn_stage_all.set_enabled(false);
        self.btn_pull.set_enabled(false);
        self.btn_push.set_enabled(false);
        self.btn_refresh.set_enabled(false);
        self.tree.clear();
        self.file_paths.clear();
        let node = self
            .tree
            .add_root(t("Initialize this folder or open a Git repository"));
        self.tree.set_node_text_color(node, tc.text_secondary);
        self.file_paths.push(String::new());
        self.timeline_tree.clear();
        let tnode = self.timeline_tree.add_root(t("No timeline available"));
        self.timeline_tree
            .set_node_text_color(tnode, tc.text_secondary);
    }

    fn update_timeline(&mut self, state: &GitState) {
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;
        self.timeline_tree.clear();
        if !state.is_repo {
            let node = self.timeline_tree.add_root(t("No timeline available"));
            self.timeline_tree
                .set_node_text_color(node, tc.text_secondary);
            return;
        }
        if state.timeline.is_empty() {
            let node = self.timeline_tree.add_root(t("No commits detected"));
            self.timeline_tree
                .set_node_text_color(node, tc.text_secondary);
            return;
        }

        for entry in &state.timeline {
            let node = self.timeline_tree.add_root(&entry.line);
            self.timeline_tree.set_node_text_color(node, tc.text);
        }
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

fn action_button(icon: &str, text: &str, color: u32, width: u32, tooltip: &str) -> ui::PlainButton {
    let btn = ui::PlainButton::new(text);
    btn.set_size(width, 24);
    btn.set_system_icon(icon, IconType::Outline, color, 15);
    btn.set_text_color(color);
    btn.set_tooltip(tooltip);
    btn
}

fn icon_button(icon: &str, color: u32, tooltip: &str) -> ui::PlainButton {
    let btn = ui::PlainButton::new("");
    btn.set_size(24, 24);
    btn.set_system_icon(icon, IconType::Outline, color, 17);
    btn.set_tooltip(tooltip);
    btn
}

fn status_color(s: FileStatus) -> u32 {
    let tc = ui::theme::colors();
    match s {
        FileStatus::Modified => tc.warning,
        FileStatus::Added => tc.success,
        FileStatus::Deleted => tc.destructive,
        FileStatus::Renamed => tc.warning,
        FileStatus::Untracked => tc.text_secondary,
        FileStatus::Conflicted => tc.destructive,
    }
}
