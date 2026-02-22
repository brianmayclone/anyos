//! anyOS Code — VS Code-like IDE for anyOS.
//!
//! Architecture:
//!   logic/ — Business logic (no UI imports)
//!   ui/    — UI layer (libanyui_client widgets)
//!   util/  — Shared utilities (path, syntax mapping)
//!
//! Event model:
//!   Uses anyui::run() (blocking event loop) with event callbacks
//!   and timers — no polling. Build output is polled via a 100ms
//!   timer that starts/stops dynamically.

#![no_std]
#![no_main]

mod logic;
mod ui;
mod util;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use libanyui_client as anyui;

use crate::logic::{build, config, file_manager, git, plugin, project};
use crate::ui::{editor_view, git_panel, output_panel, sidebar, status_bar, toolbar};
use crate::util::{path, syntax_map};

// ════════════════════════════════════════════════════════════════
//  Global application state (single-threaded, UI-thread only)
// ════════════════════════════════════════════════════════════════

struct AppState {
    file_mgr: file_manager::FileManager,
    editor_view: editor_view::EditorView,
    sidebar: sidebar::Sidebar,
    git_panel: git_panel::GitPanel,
    output: output_panel::OutputPanel,
    status: status_bar::StatusBar,
    config: config::Config,
    current_project: Option<project::Project>,
    build_process: Option<build::BuildProcess>,
    build_timer_id: u32,
    // Git integration
    git_state: git::GitState,
    git_process: Option<git::GitProcess>,
    git_pending_op: Option<git::GitOp>,
    git_timer_id: u32,
}

static mut APP: Option<AppState> = None;

/// Access the global app state. Safe because all callbacks run on the UI thread.
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("app not initialized") }
}

// ════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════

anyos_std::entry!(main);

fn main() {
    if !anyui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    // ── Load configuration and plugins ──
    let config = config::Config::load();
    let _plugins = plugin::load_plugins(&config.plugin_dir);

    // ── Create window ──
    let win = anyui::Window::new("anyOS Code", 900, 650);

    // ── Toolbar (DOCK_TOP) ──
    let tb = toolbar::AppToolbar::new(&win);
    win.add(&tb.toolbar);

    // ── Status bar (DOCK_BOTTOM) ──
    let status = status_bar::StatusBar::new();
    status.panel.set_dock(anyui::DOCK_BOTTOM);
    win.add(&status.panel);

    // ── Main split: sidebar | editor area ──
    let main_split = anyui::SplitView::new();
    main_split.set_dock(anyui::DOCK_FILL);
    main_split.set_split_ratio(config.sidebar_width);
    main_split.set_min_split(15);
    main_split.set_max_split(40);
    win.add(&main_split);

    // ── Sidebar (left pane) ──
    let mut sidebar = sidebar::Sidebar::new();

    // ── Git panel (inside sidebar, connected to tab control) ──
    let git_panel = git_panel::GitPanel::new();
    sidebar.panel.add(&git_panel.panel);
    sidebar.tab_control.connect_panels(&[&sidebar.explorer_panel, &git_panel.panel]);

    main_split.add(&sidebar.panel);

    // ── Editor area (right pane) — split vertically: editor | output ──
    let editor_split = anyui::SplitView::new();
    editor_split.set_orientation(anyui::ORIENTATION_VERTICAL);
    editor_split.set_split_ratio(100 - config.output_height);
    editor_split.set_min_split(50);
    editor_split.set_max_split(95);
    main_split.add(&editor_split);

    // ── Editor panel (top of right pane) ──
    let editor_view = editor_view::EditorView::new();
    editor_split.add(&editor_view.panel);

    // ── Output panel (bottom of right pane) ──
    let output = output_panel::OutputPanel::new();
    editor_split.add(&output.panel);

    // ── Check for command-line args (open folder) ──
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let current_project = if !args.is_empty() && path::is_directory(args) {
        sidebar.populate(args);
        Some(project::Project::open(args))
    } else {
        None
    };

    // ── Git repo detection ──
    let mut git_state = git::GitState::empty();
    if let Some(ref proj) = current_project {
        git_state.is_repo = git::is_git_repo(&proj.root);
    }

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            file_mgr: file_manager::FileManager::new(),
            editor_view,
            sidebar,
            git_panel,
            output,
            status,
            config,
            current_project,
            build_process: None,
            build_timer_id: 0,
            git_state,
            git_process: None,
            git_pending_op: None,
            git_timer_id: 0,
        });
    }

    // ── Initial git panel state ──
    {
        let s = app();
        if !s.config.has_git() {
            s.git_panel.show_not_installed();
        } else if !s.git_state.is_repo {
            s.git_panel.show_no_repo();
        } else {
            // Start initial git refresh
            trigger_git_refresh();
            // Start periodic git timer (every 5 seconds)
            s.git_timer_id = anyui::set_timer(5000, poll_git);
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring — all interactions via callbacks
    // ════════════════════════════════════════════════════════════════

    // ── Toolbar: New ──
    tb.btn_new.on_click(|_| {
        let s = app();
        let (_idx, ref p) = s.file_mgr.add_untitled(&s.config.temp_dir);
        s.editor_view.create_editor(p, None, &s.config);
        let count = s.file_mgr.count();
        s.editor_view.set_active(count - 1);
        s.file_mgr.set_active(count - 1);
        s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
        update_status();
    });

    // ── Toolbar: Save ──
    tb.btn_save.on_click(|_| {
        let s = app();
        save_current(s);
        s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    });

    // ── Toolbar: Save All ──
    tb.btn_save_all.on_click(|_| {
        let s = app();
        save_all(s);
        s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    });

    // ── Toolbar: Build ──
    tb.btn_build.on_click(|_| {
        let s = app();
        if let Some(ref proj) = s.current_project {
            s.output.clear();
            let (cmd, args) = build::build_command(proj.build_type, &s.config);
            let msg = format!("$ {}", path::basename(&cmd));
            s.output.append_line(&msg);
            anyos_std::fs::chdir(&proj.root);
            s.build_process = build::BuildProcess::spawn(&cmd, &args);
            if s.build_process.is_some() {
                start_build_timer();
            }
        }
    });

    // ── Toolbar: Run ──
    tb.btn_run.on_click(|_| {
        let s = app();
        if let Some(ref proj) = s.current_project {
            s.output.clear();
            let (cmd, args) = build::run_command(proj.build_type, &s.config);
            let msg = format!("$ {}", path::basename(&cmd));
            s.output.append_line(&msg);
            anyos_std::fs::chdir(&proj.root);
            s.build_process = build::BuildProcess::spawn(&cmd, &args);
            if s.build_process.is_some() {
                start_build_timer();
            }
        }
    });

    // ── Toolbar: Stop ──
    tb.btn_stop.on_click(|_| {
        let s = app();
        if let Some(ref mut proc) = s.build_process {
            proc.kill();
            s.output.append_line("\n[Process killed]");
        }
        s.build_process = None;
        stop_build_timer();
    });

    // ── Toolbar: Open Folder ──
    tb.btn_open.on_click(|_| {
        if let Some(folder) = anyui::FileDialog::open_folder() {
            let s = app();
            s.sidebar.populate(&folder);
            s.current_project = Some(project::Project::open(&folder));
            s.git_state.is_repo = git::is_git_repo(&folder);
            if s.git_state.is_repo {
                trigger_git_refresh();
            }
            s.status.set_branch("");
            update_status();
        }
    });

    // ── Toolbar: Settings ──
    tb.btn_settings.on_click(|_| {
        let s = app();
        let settings = s.config.settings_path.clone();
        open_file(&settings);
        update_status();
    });

    // ── Tree view: selection opens file ──
    app().sidebar.tree.on_selection_changed(|e| {
        let s = app();
        let idx = e.index;
        if idx != u32::MAX && !s.sidebar.is_directory(idx) {
            if let Some(p) = s.sidebar.path_for_node(idx) {
                let owned = String::from(p);
                open_file(&owned);
                update_status();
            }
        }
    });

    // ── Git panel: tree selection opens file ──
    app().git_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(rel_path) = s.git_panel.path_for_node(e.index) {
            if let Some(ref proj) = s.current_project {
                let full = path::join(&proj.root, rel_path);
                open_file(&full);
                update_status();
            }
        }
    });

    // ── Git panel: Refresh ──
    app().git_panel.btn_refresh.on_click(|_| {
        trigger_git_refresh();
    });

    // ── Git panel: Stage All ──
    app().git_panel.btn_stage_all.on_click(|_| {
        let s = app();
        if s.git_process.is_some() {
            return;
        }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "add -A");
            s.git_pending_op = Some(git::GitOp::Add);
        }
    });

    // ── Git panel: Commit ──
    app().git_panel.btn_commit.on_click(|_| {
        let s = app();
        if s.git_process.is_some() {
            return;
        }
        let mut msg_buf = [0u8; 512];
        let len = s.git_panel.commit_field.get_text(&mut msg_buf);
        if len == 0 {
            return;
        }
        let msg = match core::str::from_utf8(&msg_buf[..len as usize]) {
            Ok(m) => m,
            Err(_) => return,
        };
        if msg.trim().is_empty() {
            return;
        }
        if let Some(ref proj) = s.current_project {
            let args = format!("commit -m \"{}\"", msg.trim());
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, &args);
            s.git_pending_op = Some(git::GitOp::Commit);
            s.git_panel.commit_field.set_text("");
        }
    });

    // ── Git panel: Push ──
    app().git_panel.btn_push.on_click(|_| {
        let s = app();
        if s.git_process.is_some() {
            return;
        }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "push");
            s.git_pending_op = Some(git::GitOp::Push);
            s.output.clear();
            s.output.append_line("$ git push");
        }
    });

    // ── Git panel: Pull ──
    app().git_panel.btn_pull.on_click(|_| {
        let s = app();
        if s.git_process.is_some() {
            return;
        }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "pull");
            s.git_pending_op = Some(git::GitOp::Pull);
            s.output.clear();
            s.output.append_line("$ git pull");
        }
    });

    // ── Tab bar: switch active tab ──
    app().editor_view.tab_bar.on_active_changed(|e| {
        let s = app();
        let idx = e.index as usize;
        if idx < s.file_mgr.count() {
            s.file_mgr.set_active(idx);
            s.editor_view.set_active(idx);
            update_status();
        }
    });

    // ── Cursor position timer (500ms) ──
    anyui::set_timer(500, || {
        let s = app();
        if s.file_mgr.count() > 0 {
            let (row, col) = s.editor_view.get_cursor(s.file_mgr.active);
            s.status.set_cursor(row, col);
        }
    });

    // ════════════════════════════════════════════════════════════════
    //  Run the event loop (blocking, ~60fps, fires timers + events)
    // ════════════════════════════════════════════════════════════════

    anyui::run();
}

// ════════════════════════════════════════════════════════════════
//  Helper functions
// ════════════════════════════════════════════════════════════════

fn open_file(file_path: &str) {
    let s = app();
    if let Some(idx) = s.file_mgr.find_open(file_path) {
        s.file_mgr.set_active(idx);
        s.editor_view.set_active(idx);
        return;
    }
    let content = file_manager::read_file(file_path);
    let idx = s.file_mgr.add_file(file_path);
    s.editor_view.create_editor(file_path, content.as_deref(), &s.config);
    s.file_mgr.set_active(idx);
    s.editor_view.set_active(idx);
    s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
}

fn save_current(s: &mut AppState) {
    if s.file_mgr.count() == 0 {
        return;
    }
    let idx = s.file_mgr.active;
    let mut buf = vec![0u8; 128 * 1024];
    let len = s.editor_view.get_editor_text(idx, &mut buf);
    if let Some(f) = s.file_mgr.files.get(idx) {
        if file_manager::write_file(&f.path, &buf[..len as usize]) {
            s.file_mgr.mark_saved(idx);
        }
    }
}

fn save_all(s: &mut AppState) {
    for i in 0..s.file_mgr.count() {
        if s.file_mgr.files[i].modified {
            let mut buf = vec![0u8; 128 * 1024];
            let len = s.editor_view.get_editor_text(i, &mut buf);
            if file_manager::write_file(&s.file_mgr.files[i].path, &buf[..len as usize]) {
                s.file_mgr.mark_saved(i);
            }
        }
    }
}

fn update_status() {
    let s = app();
    if let Some(f) = s.file_mgr.active_file() {
        let filename = path::basename(&f.path);
        s.status.set_filename(filename);
        s.status.set_language(syntax_map::language_for_filename(filename));
    } else {
        s.status.set_filename("No file open");
        s.status.set_language("Plain Text");
    }
    s.status.set_branch(&s.git_state.branch);
}

// ── Build output timer ──────────────────────────────────────────

fn start_build_timer() {
    let s = app();
    if s.build_timer_id == 0 {
        s.build_timer_id = anyui::set_timer(100, poll_build_output);
    }
}

fn stop_build_timer() {
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
            }
        }
        if let Some(exit_code) = proc.check_finished() {
            let msg = format!("\n[Process exited with code {}]\n", exit_code);
            s.output.append(&msg);
            s.build_process = None;
            stop_build_timer();
        }
    } else {
        stop_build_timer();
    }
}

// ── Git integration ─────────────────────────────────────────────

/// Start a git status refresh (first queries branch, then status).
fn trigger_git_refresh() {
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

/// Polled by the git timer — checks running git processes, handles results.
fn poll_git() {
    let s = app();

    if let Some(ref mut proc) = s.git_process {
        proc.poll();
        if let Some(_exit_code) = proc.check_finished() {
            let output = String::from(proc.output_str());
            let op = s.git_pending_op.take().unwrap_or(git::GitOp::Status);

            // Clear the finished process
            s.git_process = None;

            match op {
                git::GitOp::Branch => {
                    // Store branch name, then chain to status query
                    s.git_state.branch = git::parse_branch(&output);
                    s.status.set_branch(&s.git_state.branch);
                    if let Some(ref proj) = s.current_project {
                        anyos_std::fs::chdir(&proj.root);
                        s.git_process = git::GitProcess::spawn(
                            &s.config.git_path,
                            "status --porcelain",
                        );
                        s.git_pending_op = Some(git::GitOp::Status);
                    }
                }
                git::GitOp::Status => {
                    // Parse and display changed files
                    s.git_state.changed_files = git::parse_status_porcelain(&output);
                    s.git_panel.update(&s.git_state);
                }
                git::GitOp::Add | git::GitOp::Commit => {
                    // After add/commit, refresh status
                    trigger_git_refresh();
                }
                git::GitOp::Push | git::GitOp::Pull => {
                    // Show output in output panel, then refresh
                    if !output.is_empty() {
                        s.output.append(&output);
                    }
                    s.output.append_line("\n[Done]");
                    trigger_git_refresh();
                }
            }
        }
    } else if s.git_state.is_repo && s.config.has_git() {
        // No process running — trigger periodic refresh
        trigger_git_refresh();
    }
}
