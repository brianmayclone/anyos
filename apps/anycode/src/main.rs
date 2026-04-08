//! anyOS Code — Professional IDE for anyOS
//!
//! Architecture:
//!   logic/    — Business logic (no UI imports)
//!   ui/       — UI layer (libanyui_client widgets)
//!   util/     — Shared utilities (path, syntax mapping)
//!
//! Key files:
//!   logic/commands.rs  — All IDE commands (build, run, open, save, etc.)
//!   ui/events.rs       — Event wiring (connects UI → commands)
//!   ui/splash.rs       — Splash screen with prerequisite check

#![no_std]
#![no_main]

mod logic;
mod ui;
mod util;

use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;
use anyui::Widget;

use crate::logic::{ai, build, config, diagnostics, file_manager, git, plugin, project, tasks};
use crate::ui::{activity_bar, ai_panel, command_palette, editor_view, events, extensions_panel,
                git_panel, output_panel, problems_panel, run_panel, search_panel, sidebar,
                splash, status_bar, symbols_panel, toolbar, welcome_tab};
use crate::util::path;

// ════════════════════════════════════════════════════════════════
//  Global application state
// ════════════════════════════════════════════════════════════════

struct AppState {
    // Core
    file_mgr: file_manager::FileManager,
    config: config::Config,
    current_project: Option<project::Project>,

    // Subsystems
    task_mgr: tasks::TaskManager,
    diagnostics: diagnostics::DiagnosticSet,
    plugin_mgr: plugin::PluginManager,
    ai_client: ai::AiClient,

    // Build
    build_process: Option<build::BuildProcess>,
    build_rules: build::BuildRules,
    build_timer_id: u32,
    build_output_buffer: String,

    // Git
    git_state: git::GitState,
    git_process: Option<git::GitProcess>,
    git_pending_op: Option<git::GitOp>,
    git_timer_id: u32,

    // UI panels
    editor_view: editor_view::EditorView,
    side_editor_view: editor_view::EditorView,
    welcome: welcome_tab::WelcomeTab,
    sidebar: sidebar::Sidebar,
    git_panel: git_panel::GitPanel,
    search_panel: search_panel::SearchPanel,
    run_panel: run_panel::RunPanel,
    symbols_panel: symbols_panel::SymbolsPanel,
    ai_panel: ai_panel::AiPanel,
    extensions_panel: extensions_panel::ExtensionsPanel,
    output: output_panel::OutputPanel,
    problems_panel: problems_panel::ProblemsPanel,
    status: status_bar::StatusBar,
    activity_bar: activity_bar::ActivityBar,
    command_palette: command_palette::CommandPalette,
    editor_groups_split: anyui::SplitView,
    side_file_mgr: file_manager::FileManager,
    split_visible: bool,
    panel_ids: [u32; 7], // Explorer, Git, Search, Run, Outline, AI, Extensions
}

anyos_std::global_app_state!(AppState);

// ════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════

anyos_std::entry!(main);

fn main() {
    if !anyui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }
    anyos_std::i18n::init();

    // ── Splash screen ──
    if !splash::show(&build::check_prerequisites()) {
        return;
    }

    // ── Configuration ──
    let config = config::Config::load();
    let plugin_mgr = plugin::load_plugins(&config.plugin_dir);

    // ── Detect project from command-line args ──
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let open_folder = if !args.is_empty() && path::is_directory(args) {
        Some(String::from(args))
    } else if config.reopen_last_project
        && !config.last_project.is_empty()
        && path::is_directory(&config.last_project)
    {
        Some(config.last_project.clone())
    } else {
        None
    };

    // ── Build UI, init state, wire events, run ──
    build_and_run(config, plugin_mgr, open_folder);
}

/// Build the full UI, initialize state, wire events, and run.
fn build_and_run(
    config: config::Config,
    plugin_mgr: plugin::PluginManager,
    open_folder: Option<String>,
) {
    let t = anyos_std::i18n::t;
    let tc = anyui::theme::colors();

    // ── Window ──
    let win = anyui::Window::new(t("anyOS Code"), -1, -1, 1024, 700);

    // ── Toolbar (DOCK_TOP) ──
    let tb = toolbar::AppToolbar::new(&win, &config);
    win.add(&tb.toolbar);

    // ── Status bar (DOCK_BOTTOM) ──
    let status = status_bar::StatusBar::new();
    status.panel.set_dock(anyui::DOCK_BOTTOM);
    win.add(&status.panel);

    // ── Activity bar (DOCK_LEFT) ──
    let activity_bar = activity_bar::ActivityBar::new(&config);
    win.add(&activity_bar.panel);

    // ── Main split: sidebar | editor ──
    let main_split = anyui::SplitView::new();
    main_split.set_dock(anyui::DOCK_FILL);
    main_split.set_split_ratio(config.sidebar_width);
    main_split.set_min_split(15);
    main_split.set_max_split(40);
    win.add(&main_split);

    // ── Sidebar container ──
    let sidebar_container = anyui::View::new();
    sidebar_container.set_color(tc.sidebar_bg);

    let mut sidebar = sidebar::Sidebar::new();
    sidebar_container.add(&sidebar.panel);

    let git_panel = git_panel::GitPanel::new();
    sidebar_container.add(&git_panel.panel);

    let search_panel = search_panel::SearchPanel::new();
    sidebar_container.add(&search_panel.panel);

    let run_panel = run_panel::RunPanel::new();
    sidebar_container.add(&run_panel.panel);

    let symbols_panel = symbols_panel::SymbolsPanel::new();
    sidebar_container.add(&symbols_panel.panel);

    let ai_panel = ai_panel::AiPanel::new();
    sidebar_container.add(&ai_panel.panel);

    let extensions_panel = extensions_panel::ExtensionsPanel::new();
    sidebar_container.add(&extensions_panel.panel);

    let panel_ids = [
        sidebar.panel.id(),
        git_panel.panel.id(),
        search_panel.panel.id(),
        run_panel.panel.id(),
        symbols_panel.panel.id(),
        ai_panel.panel.id(),
        extensions_panel.panel.id(),
    ];

    // Show only explorer
    sidebar.panel.set_visible(true);
    git_panel.panel.set_visible(false);
    search_panel.panel.set_visible(false);
    run_panel.panel.set_visible(false);
    symbols_panel.panel.set_visible(false);
    ai_panel.panel.set_visible(false);
    extensions_panel.panel.set_visible(false);

    main_split.add(&sidebar_container);

    // ── Editor + Output split ──
    let editor_split = anyui::SplitView::new();
    editor_split.set_orientation(anyui::ORIENTATION_VERTICAL);
    editor_split.set_split_ratio(100 - config.output_height);
    editor_split.set_min_split(50);
    editor_split.set_max_split(95);
    main_split.add(&editor_split);

    let editor_groups_split = anyui::SplitView::new();
    editor_groups_split.set_dock(anyui::DOCK_FILL);
    editor_groups_split.set_split_ratio(100);
    editor_groups_split.set_min_split(35);
    editor_groups_split.set_max_split(100);
    editor_split.add(&editor_groups_split);

    let editor_view = editor_view::EditorView::new(&config);
    editor_groups_split.add(&editor_view.panel);

    // Welcome tab (inside editor panel, shown when no files open)
    let welcome = welcome_tab::WelcomeTab::new(&config);
    editor_view.panel.add(&welcome.panel);
    welcome.panel.set_visible(open_folder.is_none());

    let side_editor_view = editor_view::EditorView::new(&config);
    side_editor_view.panel.set_visible(false);
    side_editor_view.set_breadcrumb("Open a file to the side for reference");
    editor_groups_split.add(&side_editor_view.panel);

    let output = output_panel::OutputPanel::new(&config);
    editor_split.add(&output.panel);

    let problems_panel = problems_panel::ProblemsPanel::new();
    output.problems_panel_view.add(&problems_panel.panel);

    // ── Project setup ──
    let current_project = open_folder.as_deref().map(|folder| {
        sidebar.populate(folder);
        project::Project::open(folder)
    });

    let mut git_state = git::GitState::empty();
    if let Some(ref proj) = current_project {
        git_state.is_repo = git::is_git_repo(&proj.root);
    }

    let build_rules = build::BuildRules::load(&config::bundle_path("build.conf"));

    let mut task_mgr = tasks::TaskManager::new();
    if let Some(ref proj) = current_project {
        task_mgr.detect_from_project(proj);
    }

    // ── Init global state ──
    unsafe {
        APP = Some(AppState {
            file_mgr: file_manager::FileManager::new(),
            config,
            current_project,
            task_mgr,
            diagnostics: diagnostics::DiagnosticSet::new(),
            plugin_mgr,
            ai_client: ai::AiClient::new(),
            build_process: None,
            build_rules,
            build_timer_id: 0,
            build_output_buffer: String::new(),
            git_state,
            git_process: None,
            git_pending_op: None,
            git_timer_id: 0,
            editor_view,
            side_editor_view,
            welcome,
            sidebar,
            git_panel,
            search_panel,
            run_panel,
            symbols_panel,
            ai_panel,
            extensions_panel,
            output,
            problems_panel,
            status,
            activity_bar,
            command_palette: command_palette::CommandPalette::new(&win),
            editor_groups_split,
            side_file_mgr: file_manager::FileManager::new(),
            split_visible: false,
            panel_ids,
        });
    }

    if let Some(ref proj) = app().current_project {
        app().config.push_recent_project(&proj.root);
        app().config.save();
        app().status.set_project_type(proj.project_type.display_name());
        app().output.start_shell(&proj.root);
    } else {
        app().status.set_project_type("");
    }

    // ── Menu bar ──
    let mut mb = anyui::MenuBarBuilder::new()
        .menu(t("File"))
            .item(1, t("New"), 0)
            .item(2, t("Open Folder..."), 0)
            .separator()
            .item(3, t("Save"), 0)
            .item(4, t("Save All"), 0)
            .separator()
            .item(5, t("Quit"), 0)
        .end_menu()
        .menu(t("Edit"))
            .item(10, t("Cut"), 0)
            .item(11, t("Copy"), 0)
            .item(12, t("Paste"), 0)
            .separator()
            .item(13, t("Select All"), 0)
            .item(14, t("Find in Files..."), 0)
        .end_menu()
        .menu(t("View"))
            .item(20, t("Explorer"), 0)
            .item(21, t("Source Control"), 0)
            .item(22, t("Search"), 0)
            .item(23, t("Run and Debug"), 0)
            .item(24, t("Outline"), 0)
            .item(25, t("Extensions"), 0)
            .separator()
            .item(26, t("Output"), 0)
            .item(27, t("Problems"), 0)
            .item(28, t("Terminal"), 0)
        .end_menu()
        .menu(t("Build"))
            .item(30, t("Build"), 0)
            .item(31, t("Run"), 0)
            .item(32, t("Test"), 0)
            .item(33, t("Check"), 0)
            .separator()
            .item(34, t("Stop"), 0)
            .item(35, t("Clean"), 0)
        .end_menu()
        .menu(t("AI"))
            .item(50, t("AI Assistant"), 0)
            .separator()
            .item(51, t("Explain Code"), 0)
            .item(52, t("Refactor Code"), 0)
            .item(53, t("Fix Code"), 0)
            .item(54, t("Generate Code"), 0)
            .item(55, t("Generate Tests"), 0)
            .item(56, t("Review Code"), 0)
            .separator()
            .item(57, t("AI Settings..."), 0)
        .end_menu()
        .menu(t("Help"))
            .item(40, t("About anyOS Code"), 0)
            .item(41, t("Command Palette"), 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = anyui::MenuBar::set(win.id(), menu_data);

    // ── Wire all events ──
    events::wire_keyboard(&win);
    events::wire_menu(&menu);
    events::wire_toolbar(&tb);
    events::wire_activity_bar();
    events::wire_sidebar();
    events::wire_search_panel();
    events::wire_run_panel();
    events::wire_symbols_panel();
    events::wire_extensions_panel();
    events::wire_problems_panel();
    events::wire_git_panel();
    events::wire_ai_panel();
    events::wire_editor();
    events::wire_welcome_tab();
    events::wire_command_palette();
    events::wire_terminal();
    events::wire_timers();

    // ── Initial panel states ──
    {
        let s = app();

        if !s.config.has_git() {
            s.git_panel.show_not_installed();
        } else if !s.git_state.is_repo {
            s.git_panel.show_no_repo();
        } else {
            trigger_git_refresh();
            s.git_timer_id = anyui::set_timer(5000, poll_git);
        }

        if s.current_project.is_some() {
            s.run_panel.update(&s.task_mgr);
        } else {
            s.run_panel.show_no_project();
        }

        s.extensions_panel.update(&s.plugin_mgr);

        if let Some(ref proj) = s.current_project {
            s.status.set_project_type(proj.project_type.display_name());
            s.output.start_shell(&proj.root);
        }

        if s.current_project.is_some() && s.config.reopen_last_project {
            logic::commands::restore_session();
        }

        if s.file_mgr.count() == 0 {
            s.welcome.show();
        }
    }

    // ── Run ──
    anyui::run();
}

// ════════════════════════════════════════════════════════════════
//  Build timer (called from commands.rs)
// ════════════════════════════════════════════════════════════════

pub fn start_build_timer() {
    let s = app();
    if s.build_timer_id == 0 {
        s.build_timer_id = anyui::set_timer(100, poll_build_output);
    }
}

pub fn stop_build_timer() {
    let s = app();
    if s.build_timer_id != 0 {
        anyui::kill_timer(s.build_timer_id);
        s.build_timer_id = 0;
    }
}

fn poll_build_output() {
    let s = app();
    if let Some(ref mut proc) = s.build_process {
        let mut buf = [0u8; 1024];
        while let Some(n) = proc.poll_output(&mut buf) {
            if let Ok(text) = core::str::from_utf8(&buf[..n]) {
                s.output.append(text);
                s.build_output_buffer.push_str(text);
            }
        }
        if let Some(exit_code) = proc.check_finished() {
            let msg = format!("\n[Process exited with code {}]\n", exit_code);
            s.output.append(&msg);

            // Parse diagnostics
            s.diagnostics.parse_output(&s.build_output_buffer);
            for line in s.build_output_buffer.split('\n') {
                diagnostics::try_parse_location(line, &mut s.diagnostics.diagnostics);
            }
            s.problems_panel.update(&s.diagnostics);
            logic::commands::update_status();

            if s.diagnostics.error_count() > 0 {
                s.output.show_problems();
            }

            s.build_process = None;
            stop_build_timer();
        }
    } else {
        stop_build_timer();
    }
}

// ════════════════════════════════════════════════════════════════
//  Git timer (called from events.rs)
// ════════════════════════════════════════════════════════════════

pub fn trigger_git_refresh() {
    let s = app();
    if s.git_process.is_some() || !s.git_state.is_repo || !s.config.has_git() {
        return;
    }
    if let Some(ref proj) = s.current_project {
        anyos_std::fs::chdir(&proj.root);
        s.git_process = git::GitProcess::spawn(&s.config.git_path, "branch --show-current");
        s.git_pending_op = Some(git::GitOp::Branch);
    }
}

fn poll_git() {
    let s = app();

    if let Some(ref mut proc) = s.git_process {
        proc.poll();
        if let Some(_exit_code) = proc.check_finished() {
            let output = String::from(proc.output_str());
            let op = s.git_pending_op.take().unwrap_or(git::GitOp::Status);
            s.git_process = None;

            match op {
                git::GitOp::Branch => {
                    s.git_state.branch = git::parse_branch(&output);
                    s.status.set_branch(&s.git_state.branch);
                    if let Some(ref proj) = s.current_project {
                        anyos_std::fs::chdir(&proj.root);
                        s.git_process = git::GitProcess::spawn(
                            &s.config.git_path, "status --porcelain",
                        );
                        s.git_pending_op = Some(git::GitOp::Status);
                    }
                }
                git::GitOp::Status => {
                    s.git_state.changed_files = git::parse_status_porcelain(&output);
                    s.git_panel.update(&s.git_state);
                }
                git::GitOp::Add | git::GitOp::Commit => {
                    trigger_git_refresh();
                }
                git::GitOp::Push | git::GitOp::Pull => {
                    if !output.is_empty() {
                        s.output.append(&output);
                    }
                    s.output.append_line("\n[Done]");
                    trigger_git_refresh();
                }
            }
        }
    } else if s.git_state.is_repo && s.config.has_git() {
        trigger_git_refresh();
    }
}
