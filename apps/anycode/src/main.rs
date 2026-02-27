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
use anyui::Widget;

use crate::logic::{build, config, file_manager, git, plugin, project};
use crate::ui::{activity_bar, editor_view, git_panel, output_panel, sidebar, status_bar, toolbar};
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
    build_rules: build::BuildRules,
    build_timer_id: u32,
    // Git integration
    git_state: git::GitState,
    git_process: Option<git::GitProcess>,
    git_pending_op: Option<git::GitOp>,
    git_timer_id: u32,
    // Activity bar + panel IDs for view switching
    activity_bar: activity_bar::ActivityBar,
    explorer_panel_id: u32,
    git_panel_id: u32,
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
    let win = anyui::Window::new("anyOS Code", -1, -1, 900, 650);

    // ── Toolbar (DOCK_TOP) ──
    let tb = toolbar::AppToolbar::new(&win);
    win.add(&tb.toolbar);

    // ── Status bar (DOCK_BOTTOM) ──
    let status = status_bar::StatusBar::new();
    status.panel.set_dock(anyui::DOCK_BOTTOM);
    win.add(&status.panel);

    // ── Activity bar (DOCK_LEFT, narrow) ──
    let activity_bar = activity_bar::ActivityBar::new();
    win.add(&activity_bar.panel);

    // ── Main split: sidebar | editor area ──
    let main_split = anyui::SplitView::new();
    main_split.set_dock(anyui::DOCK_FILL);
    main_split.set_split_ratio(config.sidebar_width);
    main_split.set_min_split(15);
    main_split.set_max_split(40);
    win.add(&main_split);

    // ── Sidebar container (left pane — holds explorer + git panels) ──
    let sidebar_container = anyui::View::new();
    sidebar_container.set_color(0xFF252526);

    // ── Sidebar (explorer view) ──
    let mut sidebar = sidebar::Sidebar::new();
    sidebar_container.add(&sidebar.panel);

    // ── Git panel (inside sidebar container) ──
    let git_panel = git_panel::GitPanel::new();
    sidebar_container.add(&git_panel.panel);

    let explorer_panel_id = sidebar.panel.id();
    let git_panel_id = git_panel.panel.id();

    // Initially show explorer, hide git panel
    sidebar.panel.set_visible(true);
    git_panel.panel.set_visible(false);

    main_split.add(&sidebar_container);

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

    // ── Load build rules from bundle ──
    let build_rules = build::BuildRules::load(&config::bundle_path("build.conf"));

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
            build_rules,
            build_timer_id: 0,
            git_state,
            git_process: None,
            git_pending_op: None,
            git_timer_id: 0,
            activity_bar,
            explorer_panel_id,
            git_panel_id,
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

    // ── Start terminal shell (if project open) ──
    {
        let s = app();
        if let Some(ref proj) = s.current_project {
            s.output.start_shell(&proj.root);
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring — all interactions via callbacks
    // ════════════════════════════════════════════════════════════════

    // ── Activity bar: Files ──
    app().activity_bar.btn_files.on_click(|_| {
        switch_sidebar_view(0);
    });

    // ── Activity bar: Git ──
    app().activity_bar.btn_git.on_click(|_| {
        switch_sidebar_view(1);
    });

    // ── Activity bar: Search (future — for now just show explorer) ──
    app().activity_bar.btn_search.on_click(|_| {
        switch_sidebar_view(0);
    });

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
            // Try build rules first, fallback to legacy
            let active_file = s.file_mgr.active_file().map(|f| f.path.as_str()).unwrap_or("");
            let (cmd, args) = if let Some(ca) = s.build_rules.build_command(active_file, &proj.root, &s.config) {
                ca
            } else {
                build::build_command(proj.build_type, &s.config)
            };
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
            let active_file = s.file_mgr.active_file().map(|f| f.path.as_str()).unwrap_or("");
            let (cmd, args) = if let Some(ca) = s.build_rules.run_command(active_file, &proj.root, &s.config) {
                ca
            } else {
                build::run_command(proj.build_type, &s.config)
            };
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
            // Start terminal shell for the new project
            s.output.start_shell(&folder);
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

    // ── Tree context menu: New File / New Folder / Delete ──
    app().sidebar.context_menu.on_item_click(|e| {
        let s = app();
        let dir = match s.sidebar.selected_dir() {
            Some(d) => d,
            None => return,
        };
        match e.index {
            0 => {
                // New File
                let new_path = path::join(&dir, "untitled.txt");
                let _ = anyos_std::fs::write_bytes(&new_path, b"");
                if let Some(ref proj) = s.current_project {
                    s.sidebar.refresh(&proj.root);
                }
            }
            1 => {
                // New Folder
                let new_path = path::join(&dir, "new_folder");
                let _ = anyos_std::fs::mkdir(&new_path);
                if let Some(ref proj) = s.current_project {
                    s.sidebar.refresh(&proj.root);
                }
            }
            3 => {
                // Delete selected
                let sel = s.sidebar.tree.selected();
                if sel != u32::MAX {
                    if let Some(p) = s.sidebar.path_for_node(sel) {
                        let owned = alloc::string::String::from(p);
                        anyos_std::fs::unlink(&owned);
                        if let Some(ref proj) = s.current_project {
                            s.sidebar.refresh(&proj.root);
                        }
                    }
                }
            }
            _ => {}
        }
    });

    // ── Tree: Enter key triggers inline rename ──
    app().sidebar.tree.on_enter(|_e| {
        let s = app();
        s.sidebar.start_rename();
    });

    // ── Rename field: submit completes rename ──
    app().sidebar.rename_field.on_submit(|_| {
        let s = app();
        s.sidebar.finish_rename();
        if let Some(ref proj) = s.current_project {
            s.sidebar.refresh(&proj.root);
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

    // ── Tab bar: close tab ──
    app().editor_view.tab_bar.on_tab_close(|e| {
        close_tab(e.index as usize);
    });

    // ── Terminal: handle command input (Enter) ──
    app().output.terminal_input.on_submit(|_| {
        let s = app();
        let mut buf = [0u8; 512];
        let len = s.output.terminal_input.get_text(&mut buf);
        if len > 0 {
            if let Ok(cmd) = core::str::from_utf8(&buf[..len as usize]) {
                s.output.send_to_shell(cmd);
            }
        }
        s.output.terminal_input.set_text("");
    });

    // ── Cursor position timer (500ms) ──
    anyui::set_timer(500, || {
        let s = app();
        if s.file_mgr.count() > 0 {
            let (row, col) = s.editor_view.get_cursor(s.file_mgr.active);
            s.status.set_cursor(row, col);
        }
    });

    // ── Terminal output poll timer (200ms) ──
    anyui::set_timer(200, || {
        let s = app();
        if s.output.shell_tid != 0 {
            s.output.poll_shell_output();
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

/// Switch sidebar between Explorer (0) and Git (1).
fn switch_sidebar_view(index: u32) {
    let s = app();
    let show_explorer = index == 0;
    anyui::Control::from_id(s.explorer_panel_id).set_visible(show_explorer);
    anyui::Control::from_id(s.git_panel_id).set_visible(!show_explorer);
    s.activity_bar.set_active(index);
}

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

fn close_tab(index: usize) {
    let s = app();
    if index >= s.file_mgr.count() {
        return;
    }
    s.editor_view.remove_editor(index);
    let new_active = s.file_mgr.remove(index);
    if s.file_mgr.count() > 0 {
        s.editor_view.set_active(new_active);
        s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), new_active);
    } else {
        s.editor_view.update_tab_labels("", 0);
    }
    update_status();
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
