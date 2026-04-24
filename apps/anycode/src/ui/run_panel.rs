use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;

use crate::logic::debug_session::{DebugSession, DebugSessionStatus};
use crate::logic::tasks::{TaskManager, TaskCategory};
use crate::util::path;

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Run panel — shows available tasks and run configurations
// ════════════════════════════════════════════════════════════════

pub struct RunPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub btn_run: ui::Button,
    pub btn_debug: ui::Button,
    pub btn_continue: ui::Button,
    pub btn_pause: ui::Button,
    pub btn_step_over: ui::Button,
    pub btn_build: ui::Button,
    pub btn_test: ui::Button,
    pub btn_stop: ui::Button,
    pub run_config_label: ui::Label,
    pub debug_status_label: ui::Label,
    pub breakpoint_label: ui::Label,
    pub debug_tree: ui::TreeView,
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

        let debug_status_label = ui::Label::new("Debugger: Idle");
        debug_status_label.set_dock(ui::DOCK_TOP);
        debug_status_label.set_size(200, 18);
        debug_status_label.set_font_size(11);
        debug_status_label.set_text_color(tc.text_secondary);
        debug_status_label.set_margin(8, 2, 0, 0);
        panel.add(&debug_status_label);

        let debug_bar = ui::FlowPanel::new();
        debug_bar.set_dock(ui::DOCK_TOP);
        debug_bar.set_size(200, 30);
        debug_bar.set_color(tc.sidebar_bg);
        panel.add(&debug_bar);

        let btn_debug = ui::Button::new(t("Debug"));
        btn_debug.set_size(62, 24);
        btn_debug.set_color(tc.accent);
        debug_bar.add(&btn_debug);

        let breakpoint_label = ui::Label::new("0 breakpoints");
        breakpoint_label.set_size(126, 24);
        breakpoint_label.set_font_size(11);
        breakpoint_label.set_text_color(tc.text_secondary);
        debug_bar.add(&breakpoint_label);

        let control_bar = ui::FlowPanel::new();
        control_bar.set_dock(ui::DOCK_TOP);
        control_bar.set_size(200, 30);
        control_bar.set_color(tc.sidebar_bg);
        panel.add(&control_bar);

        let btn_continue = ui::Button::new(t("Continue"));
        btn_continue.set_size(68, 24);
        control_bar.add(&btn_continue);

        let btn_pause = ui::Button::new(t("Pause"));
        btn_pause.set_size(54, 24);
        control_bar.add(&btn_pause);

        let btn_step_over = ui::Button::new(t("Step"));
        btn_step_over.set_size(50, 24);
        control_bar.add(&btn_step_over);

        let content_split = ui::SplitView::new();
        content_split.set_dock(ui::DOCK_FILL);
        content_split.set_orientation(ui::ORIENTATION_VERTICAL);
        content_split.set_split_ratio(64);
        content_split.set_min_split(35);
        content_split.set_max_split(82);
        panel.add(&content_split);

        let debug_section = ui::View::new();
        debug_section.set_color(tc.sidebar_bg);
        content_split.add(&debug_section);

        let debug_heading = ui::Label::new(t("DEBUG DETAILS"));
        debug_heading.set_dock(ui::DOCK_TOP);
        debug_heading.set_size(200, 20);
        debug_heading.set_font_size(10);
        debug_heading.set_text_color(tc.text_secondary);
        debug_heading.set_margin(8, 4, 0, 0);
        debug_section.add(&debug_heading);

        let debug_tree = ui::TreeView::new(200, 320);
        debug_tree.set_dock(ui::DOCK_FILL);
        debug_tree.set_indent_width(16);
        debug_tree.set_row_height(18);
        debug_section.add(&debug_tree);

        let task_section = ui::View::new();
        task_section.set_color(tc.sidebar_bg);
        content_split.add(&task_section);

        let task_heading = ui::Label::new(t("TASKS"));
        task_heading.set_dock(ui::DOCK_TOP);
        task_heading.set_size(200, 20);
        task_heading.set_font_size(10);
        task_heading.set_text_color(tc.text_secondary);
        task_heading.set_margin(8, 4, 0, 0);
        task_section.add(&task_heading);

        // Tasks tree
        let tree = ui::TreeView::new(200, 300);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(22);
        task_section.add(&tree);

        Self {
            panel,
            tree,
            btn_run,
            btn_debug,
            btn_continue,
            btn_pause,
            btn_step_over,
            btn_build,
            btn_test,
            btn_stop,
            run_config_label,
            debug_status_label,
            breakpoint_label,
            debug_tree,
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

    pub fn update_debug_session(&self, debug: &DebugSession) {
        let tc = ui::theme::colors();
        self.debug_status_label.set_text(&debug.status_label());
        let status_color = match debug.status {
            DebugSessionStatus::Running => tc.success,
            DebugSessionStatus::Launching => tc.warning,
            DebugSessionStatus::Stopped => tc.text,
            DebugSessionStatus::Idle => tc.text_secondary,
        };
        self.debug_status_label.set_text_color(status_color);
        self.breakpoint_label
            .set_text(&format!("{} breakpoints", debug.breakpoint_count()));

        self.debug_tree.clear();
        let session = self.debug_tree.add_root("Debug Session");
        self.debug_tree.set_node_style(session, STYLE_BOLD);
        self.debug_tree.set_node_text_color(session, status_color);
        self.debug_tree.set_expanded(session, true);
        self.debug_tree
            .add_child(session, &format!("State: {}", debug.status_label()));
        if !debug.paused_reason.is_empty() {
            self.debug_tree
                .add_child(session, &format!("Reason: {}", debug.paused_reason));
        }
        if !debug.target.is_empty() {
            self.debug_tree
                .add_child(session, &format!("Target: {}", debug.target));
        }
        if debug.target_tid != 0 {
            self.debug_tree
                .add_child(session, &format!("Thread: {}", debug.target_tid));
        }

        let bps = self
            .debug_tree
            .add_root(&format!("Breakpoints ({})", debug.breakpoint_count()));
        self.debug_tree.set_node_style(bps, STYLE_BOLD);
        self.debug_tree.set_expanded(bps, true);
        for bp in &debug.breakpoints {
            let marker = if bp.enabled { "enabled" } else { "disabled" };
            let label = format!("{}:{} ({})", path::basename(&bp.file_path), bp.line + 1, marker);
            self.debug_tree.add_child(bps, &label);
        }

        let frames = self
            .debug_tree
            .add_root(&format!("Call Stack ({})", debug.call_stack.len()));
        self.debug_tree.set_node_style(frames, STYLE_BOLD);
        self.debug_tree.set_expanded(frames, true);
        for frame in &debug.call_stack {
            let label = format!(
                "{} - {}:{}",
                frame.function,
                path::basename(&frame.file_path),
                frame.line + 1
            );
            self.debug_tree.add_child(frames, &label);
        }

        let vars = self
            .debug_tree
            .add_root(&format!("Variables ({})", debug.variables.len()));
        self.debug_tree.set_node_style(vars, STYLE_BOLD);
        for var in &debug.variables {
            self.debug_tree.add_child(
                vars,
                &format!("{}: {} = {}", var.name, var.type_name, var.value),
            );
        }

        let regs = self
            .debug_tree
            .add_root(&format!("Registers ({})", debug.registers.len()));
        self.debug_tree.set_node_style(regs, STYLE_BOLD);
        for reg in &debug.registers {
            self.debug_tree
                .add_child(regs, &format!("{} = {}", reg.name, reg.value));
        }

        let disasm = self
            .debug_tree
            .add_root(&format!("Disassembly ({})", debug.disassembly.len()));
        self.debug_tree.set_node_style(disasm, STYLE_BOLD);
        self.debug_tree.set_expanded(disasm, true);
        for line in &debug.disassembly {
            let marker = if line.current { "=>" } else { "  " };
            self.debug_tree.add_child(
                disasm,
                &format!(
                    "{} {}  {:<14} {}",
                    marker,
                    compact_hex(&line.address),
                    line.bytes,
                    line.text
                ),
            );
        }

        let memory = self
            .debug_tree
            .add_root(&format!("Stack Memory ({})", debug.memory_rows.len()));
        self.debug_tree.set_node_style(memory, STYLE_BOLD);
        for row in &debug.memory_rows {
            self.debug_tree.add_child(
                memory,
                &format!("{}  {:<24} {}", compact_hex(&row.address), row.bytes, row.ascii),
            );
        }
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
        self.debug_status_label.set_text("Debugger: Idle");
        self.breakpoint_label.set_text("0 breakpoints");
        self.debug_tree.clear();
    }
}

fn compact_hex(value: &str) -> &str {
    if value.len() > 10 {
        &value[value.len() - 8..]
    } else {
        value
    }
}
