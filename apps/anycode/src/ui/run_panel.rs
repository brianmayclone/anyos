use alloc::format;
use alloc::vec::Vec;
use libanyui_client as ui;
use ui::IconType;

use crate::logic::debug_session::{DebugSession, DebugSessionStatus};
use crate::logic::tasks::{TaskCategory, TaskManager};
use crate::logic::test_explorer::{TestExplorerState, TestStatus};
use crate::util::path;

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Run panel — shows available tasks and run configurations
// ════════════════════════════════════════════════════════════════

pub struct RunPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub btn_run: ui::PlainButton,
    pub btn_debug: ui::PlainButton,
    pub btn_continue: ui::PlainButton,
    pub btn_pause: ui::PlainButton,
    pub btn_step_over: ui::PlainButton,
    pub btn_build: ui::PlainButton,
    pub btn_test: ui::PlainButton,
    pub btn_stop: ui::PlainButton,
    pub btn_configure: ui::LinkLabel,
    pub run_config_label: ui::Label,
    pub run_command_label: ui::Label,
    pub toolchain_label: ui::Label,
    pub debug_status_label: ui::Label,
    pub breakpoint_label: ui::Label,
    pub debug_tree: ui::TreeView,
    pub test_status_label: ui::Label,
    pub test_tree: ui::TreeView,
    task_indices: Vec<Option<usize>>, // maps tree node → task index
}

impl RunPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        let header = ui::Label::new(t("RUN AND DEBUG"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 24);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(10, 8, 0, 2);
        panel.add(&header);

        let active_heading = section_label(t("ACTIVE TARGET"));
        panel.add(&active_heading);

        let run_config_label = ui::Label::new(t("Select a run target"));
        run_config_label.set_dock(ui::DOCK_TOP);
        run_config_label.set_size(200, 22);
        run_config_label.set_font_size(13);
        run_config_label.set_text_color(tc.text);
        run_config_label.set_margin(10, 2, 8, 0);
        panel.add(&run_config_label);

        let run_command_label = ui::Label::new("");
        run_command_label.set_dock(ui::DOCK_TOP);
        run_command_label.set_size(200, 22);
        run_command_label.set_font_size(10);
        run_command_label.set_text_color(tc.text_secondary);
        run_command_label.set_margin(10, 0, 8, 2);
        panel.add(&run_command_label);

        let toolchain_label = ui::Label::new(t("Toolchain: not configured"));
        toolchain_label.set_dock(ui::DOCK_TOP);
        toolchain_label.set_size(200, 18);
        toolchain_label.set_font_size(10);
        toolchain_label.set_text_color(tc.text_secondary);
        toolchain_label.set_margin(10, 0, 8, 4);
        panel.add(&toolchain_label);

        let configure_bar = ui::FlowPanel::new();
        configure_bar.set_dock(ui::DOCK_TOP);
        configure_bar.set_size(200, 28);
        configure_bar.set_color(tc.sidebar_bg);
        configure_bar.set_padding(6, 0, 6, 0);
        panel.add(&configure_bar);

        let btn_configure = ui::LinkLabel::new(t("Configure Toolchain..."));
        btn_configure.set_size(168, 24);
        btn_configure.set_font_size(11);
        configure_bar.add(&btn_configure);

        let action_heading = section_label(t("ACTIONS"));
        panel.add(&action_heading);

        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 34);
        btn_bar.set_color(tc.sidebar_bg);
        btn_bar.set_padding(6, 0, 6, 0);
        panel.add(&btn_bar);

        let btn_run = plain_icon_button("player-play", tc.success, t("Run selected target"));
        btn_run.set_tooltip(t("Run selected target"));
        btn_bar.add(&btn_run);

        let btn_build = plain_icon_button("hammer", tc.accent, t("Build selected target"));
        btn_build.set_tooltip(t("Build selected target"));
        btn_bar.add(&btn_build);

        let btn_test = plain_icon_button("flask-2", tc.warning, t("Run tests"));
        btn_test.set_tooltip(t("Run tests"));
        btn_bar.add(&btn_test);

        let btn_stop = plain_icon_button("player-stop", tc.destructive, t("Stop current process"));
        btn_stop.set_tooltip(t("Stop current process"));
        btn_bar.add(&btn_stop);

        let debug_heading = section_label(t("DEBUGGER"));
        panel.add(&debug_heading);

        let debug_status_label = ui::Label::new("Debugger: Idle");
        debug_status_label.set_dock(ui::DOCK_TOP);
        debug_status_label.set_size(200, 20);
        debug_status_label.set_font_size(11);
        debug_status_label.set_text_color(tc.text_secondary);
        debug_status_label.set_margin(10, 0, 8, 0);
        panel.add(&debug_status_label);

        let debug_bar = ui::FlowPanel::new();
        debug_bar.set_dock(ui::DOCK_TOP);
        debug_bar.set_size(200, 30);
        debug_bar.set_color(tc.sidebar_bg);
        debug_bar.set_padding(6, 0, 6, 0);
        panel.add(&debug_bar);

        let btn_debug = plain_icon_button("bug", tc.accent, t("Start debugging"));
        btn_debug.set_tooltip(t("Start debugging"));
        debug_bar.add(&btn_debug);

        let breakpoint_label = ui::Label::new("0 breakpoints");
        breakpoint_label.set_size(120, 24);
        breakpoint_label.set_font_size(11);
        breakpoint_label.set_text_color(tc.text_secondary);
        debug_bar.add(&breakpoint_label);

        let control_bar = ui::FlowPanel::new();
        control_bar.set_dock(ui::DOCK_TOP);
        control_bar.set_size(200, 30);
        control_bar.set_color(tc.sidebar_bg);
        control_bar.set_padding(6, 0, 6, 0);
        panel.add(&control_bar);

        let btn_continue = plain_icon_button("player-skip-forward", tc.text, t("Continue"));
        control_bar.add(&btn_continue);

        let btn_pause = plain_icon_button("player-pause", tc.text, t("Pause"));
        control_bar.add(&btn_pause);

        let btn_step_over = plain_icon_button("corner-down-right", tc.text, t("Step Over"));
        control_bar.add(&btn_step_over);

        let content_split = ui::SplitView::new();
        content_split.set_dock(ui::DOCK_FILL);
        content_split.set_orientation(ui::ORIENTATION_VERTICAL);
        content_split.set_split_ratio(48);
        content_split.set_min_split(35);
        content_split.set_max_split(82);
        panel.add(&content_split);

        let debug_section = ui::View::new();
        debug_section.set_color(tc.sidebar_bg);
        content_split.add(&debug_section);

        let debug_details_heading = section_label(t("DEBUG DETAILS"));
        debug_section.add(&debug_details_heading);

        let debug_tree = ui::TreeView::new(200, 320);
        debug_tree.set_dock(ui::DOCK_FILL);
        debug_tree.set_indent_width(16);
        debug_tree.set_row_height(18);
        debug_section.add(&debug_tree);

        let lower_split = ui::SplitView::new();
        lower_split.set_orientation(ui::ORIENTATION_VERTICAL);
        lower_split.set_split_ratio(50);
        lower_split.set_min_split(25);
        lower_split.set_max_split(75);
        content_split.add(&lower_split);

        let test_section = ui::View::new();
        test_section.set_color(tc.sidebar_bg);
        lower_split.add(&test_section);

        let test_heading = section_label(t("TEST EXPLORER"));
        test_section.add(&test_heading);

        let test_status_label = ui::Label::new(t("No tests discovered"));
        test_status_label.set_dock(ui::DOCK_TOP);
        test_status_label.set_size(200, 20);
        test_status_label.set_font_size(11);
        test_status_label.set_text_color(tc.text_secondary);
        test_status_label.set_margin(10, 0, 8, 2);
        test_section.add(&test_status_label);

        let test_tree = ui::TreeView::new(200, 220);
        test_tree.set_dock(ui::DOCK_FILL);
        test_tree.set_indent_width(16);
        test_tree.set_row_height(19);
        test_section.add(&test_tree);

        let task_section = ui::View::new();
        task_section.set_color(tc.sidebar_bg);
        lower_split.add(&task_section);

        let task_heading = section_label(t("TASKS"));
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
            btn_configure,
            run_config_label,
            run_command_label,
            toolchain_label,
            debug_status_label,
            breakpoint_label,
            debug_tree,
            test_status_label,
            test_tree,
            task_indices: Vec::new(),
        }
    }

    /// Update the task tree from the task manager.
    pub fn update(&mut self, task_mgr: &TaskManager) {
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;
        self.tree.clear();
        self.task_indices.clear();

        if task_mgr.tasks.is_empty() {
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No tasks detected"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.task_indices.push(None);
            self.run_config_label.set_text(t("Select a run target"));
            self.run_command_label
                .set_text(t("Configure Build Tools in Settings."));
            self.toolchain_label.set_text(t("Toolchain: missing"));
            return;
        }

        // Update run config label
        if let Some(run_task) = task_mgr.selected_run() {
            self.run_config_label
                .set_text(&format!("Run: {}", run_task.name));
            self.run_command_label.set_text(&run_task.display_label);
            self.toolchain_label.set_text(&format!(
                "Toolchain: {} ({})",
                run_task.toolchain_label(),
                path::basename(&run_task.command)
            ));
        } else {
            self.run_config_label.set_text(t("Select a run target"));
            if let Some(build_task) = task_mgr.selected_build() {
                self.run_command_label
                    .set_text(&format!("Build: {}", build_task.display_label));
                self.toolchain_label.set_text(&format!(
                    "Toolchain: {} ({})",
                    build_task.toolchain_label(),
                    path::basename(&build_task.command)
                ));
            } else {
                self.run_command_label
                    .set_text(t("No runnable target detected."));
                self.toolchain_label
                    .set_text(t("Toolchain: not configured"));
            }
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
            let tasks: Vec<(usize, &crate::logic::tasks::Task)> = task_mgr
                .tasks
                .iter()
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
            set_node_icon(
                &self.tree,
                cat_node,
                icon_for_task_category(*category),
                tc.text_secondary,
            );
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
                self.tree.set_node_text_color(
                    node,
                    if is_selected {
                        color
                    } else {
                        tc.text_secondary
                    },
                );
                set_node_icon(
                    &self.tree,
                    node,
                    icon_for_task_category(task.category),
                    color,
                );
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
        set_node_icon(&self.debug_tree, session, "bug", status_color);
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
        set_node_icon(&self.debug_tree, bps, "circle-dot", tc.destructive);
        self.debug_tree.set_expanded(bps, true);
        for bp in &debug.breakpoints {
            let marker = if bp.enabled { "enabled" } else { "disabled" };
            let label = format!(
                "{}:{} ({})",
                path::basename(&bp.file_path),
                bp.line + 1,
                marker
            );
            let node = self.debug_tree.add_child(bps, &label);
            set_node_icon(&self.debug_tree, node, "circle", tc.destructive);
        }

        let frames = self
            .debug_tree
            .add_root(&format!("Call Stack ({})", debug.call_stack.len()));
        self.debug_tree.set_node_style(frames, STYLE_BOLD);
        set_node_icon(&self.debug_tree, frames, "layers", tc.text_secondary);
        self.debug_tree.set_expanded(frames, true);
        for frame in &debug.call_stack {
            let label = format!(
                "{} - {}:{}",
                frame.function,
                path::basename(&frame.file_path),
                frame.line + 1
            );
            let node = self.debug_tree.add_child(frames, &label);
            set_node_icon(
                &self.debug_tree,
                node,
                "corner-down-right",
                tc.text_secondary,
            );
        }

        let vars = self
            .debug_tree
            .add_root(&format!("Variables ({})", debug.variables.len()));
        self.debug_tree.set_node_style(vars, STYLE_BOLD);
        set_node_icon(&self.debug_tree, vars, "braces", tc.text_secondary);
        for var in &debug.variables {
            let node = self.debug_tree.add_child(
                vars,
                &format!("{}: {} = {}", var.name, var.type_name, var.value),
            );
            set_node_icon(&self.debug_tree, node, "equal", tc.text_secondary);
        }

        let regs = self
            .debug_tree
            .add_root(&format!("Registers ({})", debug.registers.len()));
        self.debug_tree.set_node_style(regs, STYLE_BOLD);
        set_node_icon(&self.debug_tree, regs, "cpu", tc.text_secondary);
        for reg in &debug.registers {
            let node = self
                .debug_tree
                .add_child(regs, &format!("{} = {}", reg.name, reg.value));
            set_node_icon(&self.debug_tree, node, "binary", tc.text_secondary);
        }

        let disasm = self
            .debug_tree
            .add_root(&format!("Disassembly ({})", debug.disassembly.len()));
        self.debug_tree.set_node_style(disasm, STYLE_BOLD);
        set_node_icon(&self.debug_tree, disasm, "terminal", tc.text_secondary);
        self.debug_tree.set_expanded(disasm, true);
        for line in &debug.disassembly {
            let marker = if line.current { "=>" } else { "  " };
            let node = self.debug_tree.add_child(
                disasm,
                &format!(
                    "{} {}  {:<14} {}",
                    marker,
                    compact_hex(&line.address),
                    line.bytes,
                    line.text
                ),
            );
            if line.current {
                self.debug_tree.set_node_text_color(node, tc.accent);
            }
        }

        let memory = self
            .debug_tree
            .add_root(&format!("Stack Memory ({})", debug.memory_rows.len()));
        self.debug_tree.set_node_style(memory, STYLE_BOLD);
        set_node_icon(&self.debug_tree, memory, "memory-stick", tc.text_secondary);
        for row in &debug.memory_rows {
            let node = self.debug_tree.add_child(
                memory,
                &format!(
                    "{}  {:<24} {}",
                    compact_hex(&row.address),
                    row.bytes,
                    row.ascii
                ),
            );
            set_node_icon(&self.debug_tree, node, "rows-3", tc.text_secondary);
        }
    }

    pub fn update_tests(&self, tests: &TestExplorerState) {
        let tc = ui::theme::colors();
        self.test_tree.clear();

        let total = tests.total_tests();
        let failed = tests.failed_count();
        let status_color = match tests.last_status {
            TestStatus::Passed => tc.success,
            TestStatus::Failed => tc.destructive,
            TestStatus::NotRun => tc.text_secondary,
        };
        self.test_status_label.set_text(&format!(
            "{} tests | {} | {} failed",
            total,
            tests.last_status.label(),
            failed
        ));
        self.test_status_label.set_text_color(status_color);

        if tests.projects.is_empty() {
            let node = self.test_tree.add_root("Open a Rust project to discover tests");
            self.test_tree.set_node_text_color(node, tc.text_secondary);
            return;
        }

        for project in &tests.projects {
            let root = self
                .test_tree
                .add_root(&format!("{} ({})", project.name, project.cases.len()));
            self.test_tree.set_node_style(root, STYLE_BOLD);
            set_node_icon(&self.test_tree, root, "folder-code", tc.text_secondary);
            self.test_tree.set_expanded(root, true);

            if project.cases.is_empty() {
                let node = self.test_tree.add_child(root, "No tests discovered");
                self.test_tree.set_node_text_color(node, tc.text_secondary);
                continue;
            }

            for case in &project.cases {
                let node = self.test_tree.add_child(root, &case.display_name);
                set_node_icon(&self.test_tree, node, "flask-2", tc.warning);
                self.test_tree.set_node_text_color(node, tc.text_secondary);
            }
        }

        if let Some(last) = tests.history.last() {
            let history = self.test_tree.add_root(&format!(
                "Last Run: exit {} | {} passed | {} failed",
                last.exit_code, last.passed, last.failed
            ));
            self.test_tree.set_node_style(history, STYLE_BOLD);
            set_node_icon(&self.test_tree, history, "history", status_color);
            for line in last.output_excerpt.split('\n') {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let node = self.test_tree.add_child(history, trimmed);
                self.test_tree.set_node_text_color(node, tc.text_secondary);
            }
            self.test_tree.set_expanded(history, failed > 0);
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
        self.run_config_label.set_text(t("Select a run target"));
        self.run_command_label
            .set_text(t("Open a workspace or configure build tools."));
        self.toolchain_label
            .set_text(t("Toolchain: not configured"));
        self.debug_status_label.set_text("Debugger: Idle");
        self.breakpoint_label.set_text("0 breakpoints");
        self.debug_tree.clear();
        self.test_status_label.set_text(t("No tests discovered"));
        self.test_tree.clear();
    }
}

fn compact_hex(value: &str) -> &str {
    if value.len() > 10 {
        &value[value.len() - 8..]
    } else {
        value
    }
}

fn section_label(text: &str) -> ui::Label {
    let tc = ui::theme::colors();
    let label = ui::Label::new(text);
    label.set_dock(ui::DOCK_TOP);
    label.set_size(200, 20);
    label.set_font_size(10);
    label.set_text_color(tc.text_secondary);
    label.set_margin(10, 8, 0, 0);
    label
}

fn plain_icon_button(icon: &str, color: u32, tooltip: &str) -> ui::PlainButton {
    let btn = ui::PlainButton::new("");
    btn.set_size(26, 24);
    btn.set_system_icon(icon, IconType::Outline, color, 17);
    btn.set_tooltip(tooltip);
    btn
}

fn set_node_icon(tree: &ui::TreeView, node: u32, icon: &str, color: u32) {
    if let Some(icon) = ui::Icon::system(icon, IconType::Outline, color, 16) {
        tree.set_node_icon(node, &icon.pixels, icon.width, icon.height);
    }
}

fn icon_for_task_category(category: TaskCategory) -> &'static str {
    match category {
        TaskCategory::Build => "hammer",
        TaskCategory::Run => "player-play",
        TaskCategory::Test => "flask-2",
        TaskCategory::Check => "check",
        TaskCategory::Clean => "trash",
        TaskCategory::Custom => "terminal",
    }
}
