use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;

use crate::app;
use crate::logic::{commands, git, tasks};
use crate::ui::toolbar::AppToolbar;
use crate::util::path;

// ════════════════════════════════════════════════════════════════
//  Event wiring — connects UI widgets to command functions
// ════════════════════════════════════════════════════════════════

/// Wire all activity bar button click events.
pub fn wire_activity_bar() {
    app().activity_bar.btn_files.on_click(|_| commands::switch_sidebar_view(0));
    app().activity_bar.btn_git.on_click(|_| commands::switch_sidebar_view(1));
    app().activity_bar.btn_search.on_click(|_| commands::switch_sidebar_view(2));
    app().activity_bar.btn_run.on_click(|_| commands::switch_sidebar_view(3));
    app().activity_bar.btn_outline.on_click(|_| {
        commands::refresh_symbols();
        commands::switch_sidebar_view(4);
    });
    app().activity_bar.btn_extensions.on_click(|_| commands::switch_sidebar_view(5));
}

/// Wire all toolbar button click events.
pub fn wire_toolbar(tb: &AppToolbar) {
    tb.btn_new.on_click(|_| commands::new_file());
    tb.btn_open.on_click(|_| commands::open_folder());
    tb.btn_save.on_click(|_| commands::save());
    tb.btn_save_all.on_click(|_| commands::save_all());
    tb.btn_build.on_click(|_| commands::build());
    tb.btn_run.on_click(|_| commands::run());
    tb.btn_stop.on_click(|_| commands::stop());
    tb.btn_settings.on_click(|_| commands::open_settings());
}

/// Wire the menu bar events.
pub fn wire_menu(menu: &anyui::MenuBar) {
    menu.on_item(|e| {
        match e.item_id {
            1 => commands::new_file(),
            2 => commands::open_folder(),
            3 => commands::save(),
            4 => commands::save_all(),
            5 => anyui::quit(),
            10..=13 => {} // Handled by editor widget
            14 => commands::switch_sidebar_view(2), // Find in Files
            20 => commands::switch_sidebar_view(0),
            21 => commands::switch_sidebar_view(1),
            22 => commands::switch_sidebar_view(2),
            23 => commands::switch_sidebar_view(3),
            24 => commands::switch_sidebar_view(4),
            25 => commands::switch_sidebar_view(5),
            26 => app().output.show_output(),
            27 => app().output.show_problems(),
            28 => app().output.show_terminal(),
            30 => commands::build(),
            31 => commands::run(),
            32 => commands::test(),
            33 => commands::check(),
            34 => commands::stop(),
            35 => commands::clean(),
            40 => commands::about(),
            _ => {}
        }
    });
}

/// Wire sidebar (explorer) events.
pub fn wire_sidebar() {
    // Tree view: selection opens file
    app().sidebar.tree.on_selection_changed(|e| {
        let s = app();
        let idx = e.index;
        if idx != u32::MAX && !s.sidebar.is_directory(idx) {
            if let Some(p) = s.sidebar.path_for_node(idx) {
                let owned = String::from(p);
                commands::open_file(&owned);
                commands::update_status();
            }
        }
    });

    // Context menu: New File / New Folder / Delete
    app().sidebar.context_menu.on_item_click(|e| {
        let s = app();
        let dir = match s.sidebar.selected_dir() {
            Some(d) => d,
            None => return,
        };
        match e.index {
            0 => {
                let new_path = path::join(&dir, "untitled.txt");
                let _ = anyos_std::fs::write_bytes(&new_path, b"");
                if let Some(ref proj) = s.current_project {
                    s.sidebar.refresh(&proj.root);
                }
            }
            1 => {
                let new_path = path::join(&dir, "new_folder");
                let _ = anyos_std::fs::mkdir(&new_path);
                if let Some(ref proj) = s.current_project {
                    s.sidebar.refresh(&proj.root);
                }
            }
            3 => {
                let sel = s.sidebar.tree.selected();
                if sel != u32::MAX {
                    if let Some(p) = s.sidebar.path_for_node(sel) {
                        let owned = String::from(p);
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

    // Enter key triggers inline rename
    app().sidebar.tree.on_enter(|_| {
        app().sidebar.start_rename();
    });

    // Rename field submit
    app().sidebar.rename_field.on_submit(|_| {
        let s = app();
        s.sidebar.finish_rename();
        if let Some(ref proj) = s.current_project {
            s.sidebar.refresh(&proj.root);
        }
    });
}

/// Wire search panel events.
pub fn wire_search_panel() {
    app().search_panel.btn_search.on_click(|_| commands::search_in_project());
    app().search_panel.search_field.on_submit(|_| commands::search_in_project());

    app().search_panel.results_tree.on_selection_changed(|e| {
        let s = app();
        if let Some((file_path, _line)) = s.search_panel.path_for_node(e.index) {
            let owned = String::from(file_path);
            commands::open_file(&owned);
            commands::update_status();
        }
    });
}

/// Wire run panel events.
pub fn wire_run_panel() {
    app().run_panel.btn_run.on_click(|_| commands::run());
    app().run_panel.btn_build.on_click(|_| commands::build());
    app().run_panel.btn_test.on_click(|_| commands::test());
    app().run_panel.btn_stop.on_click(|_| commands::stop());

    // Task tree click — select task
    app().run_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(task_idx) = s.run_panel.task_index_for_node(e.index) {
            if let Some(task) = s.task_mgr.tasks.get(task_idx) {
                match task.category {
                    tasks::TaskCategory::Run => s.task_mgr.selected_run_task = task_idx,
                    tasks::TaskCategory::Build => s.task_mgr.selected_build_task = task_idx,
                    _ => {}
                }
            }
            s.run_panel.update(&s.task_mgr);
        }
    });

    // Double-click (Enter) executes task
    app().run_panel.tree.on_enter(|_| {
        let s = app();
        let sel = s.run_panel.tree.selected();
        if sel != u32::MAX {
            if let Some(task_idx) = s.run_panel.task_index_for_node(sel) {
                commands::execute_task(task_idx);
            }
        }
    });
}

/// Wire symbols panel events.
pub fn wire_symbols_panel() {
    app().symbols_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(_line) = s.symbols_panel.line_for_node(e.index) {
            // TODO: navigate editor to line when TextEditor supports set_cursor_line()
        }
    });
}

/// Wire extensions panel events.
pub fn wire_extensions_panel() {
    app().extensions_panel.btn_refresh.on_click(|_| {
        let s = app();
        s.plugin_mgr.scan_and_load();
        s.extensions_panel.update(&s.plugin_mgr);
    });

    app().extensions_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(name) = s.extensions_panel.plugin_name_for_node(e.index) {
            let name_owned = String::from(name);
            s.extensions_panel.show_detail(&s.plugin_mgr, &name_owned);
        }
    });
}

/// Wire problems panel events.
pub fn wire_problems_panel() {
    app().problems_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some((file_path, _line)) = s.problems_panel.location_for_node(e.index) {
            let owned = String::from(file_path);
            if !owned.is_empty() {
                let full = if owned.starts_with('/') {
                    owned
                } else if let Some(ref proj) = s.current_project {
                    path::join(&proj.root, &owned)
                } else {
                    owned
                };
                commands::open_file(&full);
                commands::update_status();
            }
        }
    });
}

/// Wire git panel events.
pub fn wire_git_panel() {
    app().git_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(rel_path) = s.git_panel.path_for_node(e.index) {
            if let Some(ref proj) = s.current_project {
                let full = path::join(&proj.root, rel_path);
                commands::open_file(&full);
                commands::update_status();
            }
        }
    });

    app().git_panel.btn_refresh.on_click(|_| crate::trigger_git_refresh());

    app().git_panel.btn_stage_all.on_click(|_| {
        let s = app();
        if s.git_process.is_some() { return; }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "add -A");
            s.git_pending_op = Some(git::GitOp::Add);
        }
    });

    app().git_panel.btn_commit.on_click(|_| {
        let s = app();
        if s.git_process.is_some() { return; }
        let mut msg_buf = [0u8; 512];
        let len = s.git_panel.commit_field.get_text(&mut msg_buf);
        if len == 0 { return; }
        let msg = match core::str::from_utf8(&msg_buf[..len as usize]) {
            Ok(m) => m,
            Err(_) => return,
        };
        if msg.trim().is_empty() { return; }
        if let Some(ref proj) = s.current_project {
            let args = format!("commit -m \"{}\"", msg.trim());
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, &args);
            s.git_pending_op = Some(git::GitOp::Commit);
            s.git_panel.commit_field.set_text("");
        }
    });

    app().git_panel.btn_push.on_click(|_| {
        let s = app();
        if s.git_process.is_some() { return; }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "push");
            s.git_pending_op = Some(git::GitOp::Push);
            s.output.clear();
            s.output.append_line("$ git push");
        }
    });

    app().git_panel.btn_pull.on_click(|_| {
        let s = app();
        if s.git_process.is_some() { return; }
        if let Some(ref proj) = s.current_project {
            anyos_std::fs::chdir(&proj.root);
            s.git_process = git::GitProcess::spawn(&s.config.git_path, "pull");
            s.git_pending_op = Some(git::GitOp::Pull);
            s.output.clear();
            s.output.append_line("$ git pull");
        }
    });
}

/// Wire editor tab bar events.
pub fn wire_editor() {
    app().editor_view.tab_bar.on_active_changed(|e| {
        let s = app();
        let idx = e.index as usize;
        if idx < s.file_mgr.count() {
            s.file_mgr.set_active(idx);
            s.editor_view.set_active(idx);
            commands::update_status();
            commands::refresh_symbols();
        }
    });

    app().editor_view.tab_bar.on_tab_close(|e| {
        commands::close_tab(e.index as usize);
    });
}

/// Wire terminal input.
pub fn wire_terminal() {
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
}

/// Wire all timers.
pub fn wire_timers() {
    // Cursor position timer (500ms)
    anyui::set_timer(500, || {
        let s = app();
        if s.file_mgr.count() > 0 {
            let (row, col) = s.editor_view.get_cursor(s.file_mgr.active);
            s.status.set_cursor(row, col);
        }
    });

    // Terminal output poll timer (200ms)
    anyui::set_timer(200, || {
        let s = app();
        if s.output.shell_tid != 0 {
            s.output.poll_shell_output();
        }
    });
}
