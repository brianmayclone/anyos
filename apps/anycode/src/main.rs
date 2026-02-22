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

use crate::logic::{build, config, file_manager, plugin, project};
use crate::ui::{editor_view, output_panel, sidebar, status_bar, toolbar};
use crate::util::{path, syntax_map};

// ════════════════════════════════════════════════════════════════
//  Global application state (single-threaded, UI-thread only)
// ════════════════════════════════════════════════════════════════

struct AppState {
    file_mgr: file_manager::FileManager,
    editor_view: editor_view::EditorView,
    sidebar: sidebar::Sidebar,
    output: output_panel::OutputPanel,
    status: status_bar::StatusBar,
    config: config::Config,
    current_project: Option<project::Project>,
    build_process: Option<build::BuildProcess>,
    build_timer_id: u32,
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
    let _plugins = plugin::load_plugins();

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

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            file_mgr: file_manager::FileManager::new(),
            editor_view,
            sidebar,
            output,
            status,
            config,
            current_project,
            build_process: None,
            build_timer_id: 0,
        });
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring — all interactions via callbacks
    // ════════════════════════════════════════════════════════════════

    // ── Toolbar: New ──
    tb.btn_new.on_click(|_| {
        let s = app();
        let (_idx, ref p) = s.file_mgr.add_untitled();
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
            s.output.append_line("$ make");
            let (cmd, args) = build::build_command(proj.build_type);
            anyos_std::fs::chdir(&proj.root);
            s.build_process = build::BuildProcess::spawn(cmd, args);
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
            s.output.append_line("$ run");
            let (cmd, args) = build::run_command(proj.build_type);
            anyos_std::fs::chdir(&proj.root);
            s.build_process = build::BuildProcess::spawn(cmd, args);
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
        anyui::MessageBox::show(
            anyui::MessageBoxType::Info,
            "Open Folder: Enter path in terminal args",
            None,
        );
    });

    // ── Toolbar: Settings ──
    tb.btn_settings.on_click(|_| {
        open_file("/Users/settings/anycode.json");
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
