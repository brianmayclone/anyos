use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;

use crate::logic::tasks::{TaskManager, TaskCategory};

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Run panel — shows available tasks and run configurations
// ════════════════════════════════════════════════════════════════

pub struct RunPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub btn_run: ui::Button,
    pub btn_build: ui::Button,
    pub btn_test: ui::Button,
    pub btn_stop: ui::Button,
    pub run_config_label: ui::Label,
    task_indices: Vec<Option<usize>>, // maps tree node → task index
}

impl RunPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        // Header
        let header = ui::Label::new(t("RUN AND DEBUG"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        // Run configuration label
        let run_config_label = ui::Label::new(t("No run configuration"));
        run_config_label.set_dock(ui::DOCK_TOP);
        run_config_label.set_size(200, 18);
        run_config_label.set_font_size(12);
        run_config_label.set_text_color(tc.text);
        run_config_label.set_margin(8, 2, 0, 4);
        panel.add(&run_config_label);

        // Action buttons
        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 34);
        btn_bar.set_color(tc.sidebar_bg);
        panel.add(&btn_bar);

        let btn_run = ui::Button::new(t("Run"));
        btn_run.set_size(50, 26);
        btn_run.set_color(tc.success);
        btn_bar.add(&btn_run);

        let btn_build = ui::Button::new(t("Build"));
        btn_build.set_size(50, 26);
        btn_build.set_color(tc.accent);
        btn_bar.add(&btn_build);

        let btn_test = ui::Button::new(t("Test"));
        btn_test.set_size(45, 26);
        btn_bar.add(&btn_test);

        let btn_stop = ui::Button::new(t("Stop"));
        btn_stop.set_size(45, 26);
        btn_stop.set_color(tc.destructive);
        btn_bar.add(&btn_stop);

        // Separator
        let sep = ui::View::new();
        sep.set_dock(ui::DOCK_TOP);
        sep.set_size(200, 1);
        sep.set_color(tc.tab_border_active);
        sep.set_margin(0, 4, 0, 4);
        panel.add(&sep);

        // Tasks tree
        let tree = ui::TreeView::new(200, 300);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        panel.add(&tree);

        Self {
            panel,
            tree,
            btn_run,
            btn_build,
            btn_test,
            btn_stop,
            run_config_label,
            task_indices: Vec::new(),
        }
    }

    /// Update the task tree from the task manager.
    pub fn update(&mut self, task_mgr: &TaskManager) {
        let tc = ui::theme::colors();
        self.tree.clear();
        self.task_indices.clear();

        if task_mgr.tasks.is_empty() {
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No tasks detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.task_indices.push(None);
            self.run_config_label.set_text(t("No run configuration"));
            return;
        }

        // Update run config label
        if let Some(run_task) = task_mgr.selected_run() {
            self.run_config_label.set_text(&format!("\u{25B6} {}", run_task.name));
        }

        // Group tasks by category
        let categories = [
            TaskCategory::Build,
            TaskCategory::Run,
            TaskCategory::Test,
            TaskCategory::Check,
            TaskCategory::Clean,
            TaskCategory::Custom,
        ];

        for category in &categories {
            let tasks: Vec<(usize, &crate::logic::tasks::Task)> = task_mgr.tasks.iter()
                .enumerate()
                .filter(|(_, t)| t.category == *category)
                .collect();

            if tasks.is_empty() {
                continue;
            }

            let label = format!("{} ({})", category.label(), tasks.len());
            let cat_node = self.tree.add_root(&label);
            self.tree.set_node_style(cat_node, STYLE_BOLD);
            self.tree.set_node_text_color(cat_node, tc.text);
            self.task_indices.push(None);
            self.tree.set_expanded(cat_node, true);

            for (task_idx, task) in &tasks {
                let is_selected = match task.category {
                    TaskCategory::Run => *task_idx == task_mgr.selected_run_task,
                    TaskCategory::Build => *task_idx == task_mgr.selected_build_task,
                    _ => false,
                };

                let prefix = if is_selected { "\u{25B6} " } else { "  " };
                let label = format!("{}{}", prefix, task.display_label);
                let node = self.tree.add_child(cat_node, &label);

                let color = match task.category {
                    TaskCategory::Run => tc.success,
                    TaskCategory::Build => tc.accent,
                    TaskCategory::Test => tc.warning,
                    _ => tc.text_secondary,
                };
                self.tree.set_node_text_color(node, if is_selected { color } else { tc.text_secondary });
                self.task_indices.push(Some(*task_idx));
            }
        }
    }

    /// Get the task index for a tree node selection.
    pub fn task_index_for_node(&self, node: u32) -> Option<usize> {
        self.task_indices.get(node as usize).copied().flatten()
    }

    /// Show a "no project" message.
    pub fn show_no_project(&mut self) {
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;
        self.tree.clear();
        self.task_indices.clear();
        let node = self.tree.add_root(t("Open a folder to detect tasks"));
        self.tree.set_node_text_color(node, tc.text_secondary);
        self.task_indices.push(None);
        self.run_config_label.set_text(t("No run configuration"));
    }
}
