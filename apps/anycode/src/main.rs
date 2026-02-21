//! anyOS Code — VS Code-like IDE for anyOS.
//!
//! Architecture:
//!   logic/ — Business logic (no UI imports)
//!   ui/    — UI layer (libanyui_client widgets)
//!   util/  — Shared utilities (path, syntax mapping)

#![no_std]
#![no_main]

mod logic;
mod ui;
mod util;

use alloc::format;
use alloc::vec;
use libanyui_client as anyui;

use crate::logic::{build, config, file_manager, plugin, project};
use crate::ui::{editor_view, output_panel, sidebar, status_bar, toolbar};
use crate::util::{path, syntax_map};

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
    let mut editor_view = editor_view::EditorView::new();
    editor_split.add(&editor_view.panel);

    // ── Output panel (bottom of right pane) ──
    let output = output_panel::OutputPanel::new();
    editor_split.add(&output.panel);

    // ── Core state ──
    let mut file_mgr = file_manager::FileManager::new();
    let mut current_project: Option<project::Project> = None;
    let mut build_process: Option<build::BuildProcess> = None;

    // ── Check for command-line args (open folder) ──
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    if !args.is_empty() && path::is_directory(args) {
        current_project = Some(project::Project::open(args));
        sidebar.populate(args);
    }

    // ════════════════════════════════════════════════════════════════
    //  Event wiring
    // ════════════════════════════════════════════════════════════════

    // Tree view: double-click to open file
    sidebar.tree.on_node_clicked(move |_e| {
        // Event handling will be done in the run_once loop via polling
    });

    // Tab bar: switch active tab
    editor_view.tab_bar.on_active_changed(move |_e| {
        // Will be handled in run_once loop
    });

    // Toolbar buttons (using closures that set flags)
    // Since we can't easily share mutable state with closures in no_std,
    // we use a polling approach in the run_once loop.

    // ════════════════════════════════════════════════════════════════
    //  Main event loop with polling
    // ════════════════════════════════════════════════════════════════

    // We use run_once() so we can poll build output and handle events
    let mut last_tab_state = u32::MAX;
    let mut last_tree_sel = u32::MAX;

    loop {
        if !anyui::run_once() {
            break;
        }

        // ── Poll build process output ──
        if let Some(ref mut proc) = build_process {
            let mut buf = [0u8; 1024];
            while let Some(n) = proc.poll_output(&mut buf) {
                if let Ok(text) = core::str::from_utf8(&buf[..n]) {
                    output.append(text);
                }
            }
            if let Some(exit_code) = proc.check_finished() {
                let msg = format!("\n[Process exited with code {}]\n", exit_code);
                output.append(&msg);
                build_process = None;
            }
        }

        // ── Poll tab bar changes ──
        let tab_state = editor_view.tab_bar.get_state();
        if tab_state != last_tab_state && tab_state != u32::MAX {
            last_tab_state = tab_state;
            let idx = tab_state as usize;
            if idx < file_mgr.count() {
                file_mgr.set_active(idx);
                editor_view.set_active(idx);
                update_status_bar(&status, &file_mgr, &editor_view);
            }
        }

        // ── Poll tree view selection (double-click opens file) ──
        let tree_sel = sidebar.tree.selected();
        if tree_sel != last_tree_sel && tree_sel != u32::MAX {
            last_tree_sel = tree_sel;
            if !sidebar.is_directory(tree_sel) {
                if let Some(file_path) = sidebar.path_for_node(tree_sel) {
                    open_file(file_path, &mut file_mgr, &mut editor_view, &config);
                    update_status_bar(&status, &file_mgr, &editor_view);
                }
            }
        }

        // ── Poll toolbar buttons via state ──
        // New
        if tb.btn_new.get_state() == 1 {
            tb.btn_new.set_state(0);
            let (_idx, _path) = file_mgr.add_untitled();
            editor_view.create_editor(&_path, None, &config);
            editor_view.set_active(file_mgr.count() - 1);
            file_mgr.set_active(file_mgr.count() - 1);
            editor_view.update_tab_labels(&file_mgr.tab_labels(), file_mgr.active);
            update_status_bar(&status, &file_mgr, &editor_view);
        }

        // Save
        if tb.btn_save.get_state() == 1 {
            tb.btn_save.set_state(0);
            save_current(&mut file_mgr, &editor_view);
            editor_view.update_tab_labels(&file_mgr.tab_labels(), file_mgr.active);
        }

        // Save All
        if tb.btn_save_all.get_state() == 1 {
            tb.btn_save_all.set_state(0);
            save_all(&mut file_mgr, &editor_view);
            editor_view.update_tab_labels(&file_mgr.tab_labels(), file_mgr.active);
        }

        // Build
        if tb.btn_build.get_state() == 1 {
            tb.btn_build.set_state(0);
            if let Some(ref proj) = current_project {
                output.clear();
                output.append_line("$ make");
                let (cmd, args) = build::build_command(proj.build_type);
                anyos_std::fs::chdir(&proj.root);
                build_process = build::BuildProcess::spawn(cmd, args);
            }
        }

        // Run
        if tb.btn_run.get_state() == 1 {
            tb.btn_run.set_state(0);
            if let Some(ref proj) = current_project {
                output.clear();
                output.append_line("$ run");
                let (cmd, args) = build::run_command(proj.build_type);
                anyos_std::fs::chdir(&proj.root);
                build_process = build::BuildProcess::spawn(cmd, args);
            }
        }

        // Stop
        if tb.btn_stop.get_state() == 1 {
            tb.btn_stop.set_state(0);
            if let Some(ref mut proc) = build_process {
                proc.kill();
                output.append_line("\n[Process killed]");
            }
            build_process = None;
        }

        // Open Folder
        if tb.btn_open.get_state() == 1 {
            tb.btn_open.set_state(0);
            // Show a simple input dialog
            anyui::MessageBox::show(
                anyui::MessageBoxType::Info,
                "Open Folder: Enter path in terminal args",
                None,
            );
        }

        // Settings
        if tb.btn_settings.get_state() == 1 {
            tb.btn_settings.set_state(0);
            open_file(
                "/Users/settings/anycode.json",
                &mut file_mgr,
                &mut editor_view,
                &config,
            );
            update_status_bar(&status, &file_mgr, &editor_view);
        }

        // ── Update cursor position in status bar (every frame) ──
        if file_mgr.count() > 0 {
            let (row, col) = editor_view.get_cursor(file_mgr.active);
            status.set_cursor(row, col);
        }

        anyos_std::process::sleep(8); // ~120 Hz
    }
}

// ════════════════════════════════════════════════════════════════
//  Helper functions
// ════════════════════════════════════════════════════════════════

fn open_file(
    file_path: &str,
    file_mgr: &mut file_manager::FileManager,
    editor_view: &mut editor_view::EditorView,
    config: &config::Config,
) {
    // Check if already open
    if let Some(idx) = file_mgr.find_open(file_path) {
        file_mgr.set_active(idx);
        editor_view.set_active(idx);
        return;
    }

    // Read file
    let content = file_manager::read_file(file_path);
    let idx = file_mgr.add_file(file_path);
    editor_view.create_editor(file_path, content.as_deref(), config);
    file_mgr.set_active(idx);
    editor_view.set_active(idx);
    editor_view.update_tab_labels(&file_mgr.tab_labels(), file_mgr.active);
}

fn save_current(
    file_mgr: &mut file_manager::FileManager,
    editor_view: &editor_view::EditorView,
) {
    if file_mgr.count() == 0 {
        return;
    }
    let idx = file_mgr.active;
    let mut buf = vec![0u8; 128 * 1024];
    let len = editor_view.get_editor_text(idx, &mut buf);
    if let Some(f) = file_mgr.files.get(idx) {
        if file_manager::write_file(&f.path, &buf[..len as usize]) {
            file_mgr.mark_saved(idx);
        }
    }
}

fn save_all(
    file_mgr: &mut file_manager::FileManager,
    editor_view: &editor_view::EditorView,
) {
    for i in 0..file_mgr.count() {
        if file_mgr.files[i].modified {
            let mut buf = vec![0u8; 128 * 1024];
            let len = editor_view.get_editor_text(i, &mut buf);
            if file_manager::write_file(&file_mgr.files[i].path, &buf[..len as usize]) {
                file_mgr.mark_saved(i);
            }
        }
    }
}

fn update_status_bar(
    status: &status_bar::StatusBar,
    file_mgr: &file_manager::FileManager,
    editor_view: &editor_view::EditorView,
) {
    if let Some(f) = file_mgr.active_file() {
        let filename = path::basename(&f.path);
        status.set_filename(filename);
        status.set_language(syntax_map::language_for_filename(filename));
    } else {
        status.set_filename("No file open");
        status.set_language("Plain Text");
    }
}
