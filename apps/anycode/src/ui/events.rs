use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;

use crate::app;
use crate::logic::{ai, commands, git, tasks};
use crate::ui::toolbar::AppToolbar;
use crate::util::path;

// ════════════════════════════════════════════════════════════════
//  Event wiring — connects UI widgets to command functions
// ════════════════════════════════════════════════════════════════

// ── Keyboard shortcuts (Window-level) ──────────────────────────

pub fn wire_keyboard(win: &anyui::Window) {
    win.on_key_down(|e| {
        let s = app();

        // Quick Open: Ctrl+P
        if e.ctrl() && !e.shift() && e.keycode == b'P' as u32 {
            let root = s.current_project.as_ref().map(|p| p.root.as_str());
            s.command_palette.show_files(root);
            return;
        }

        // Command palette: Ctrl+Shift+P
        if e.ctrl() && e.shift() && e.keycode == b'P' as u32 {
            s.command_palette.show_commands();
            return;
        }

        // Split editor: Ctrl+\
        if e.ctrl() && e.keycode == b'\\' as u32 {
            commands::toggle_editor_split();
            return;
        }

        // Close command palette on Escape
        if e.keycode == anyui::KEY_ESCAPE && s.command_palette.visible {
            s.command_palette.hide();
            return;
        }

        // Save: Ctrl+S
        if e.ctrl() && !e.shift() && e.keycode == b'S' as u32 {
            commands::save();
            return;
        }

        // Save All: Ctrl+Shift+S
        if e.ctrl() && e.shift() && e.keycode == b'S' as u32 {
            commands::save_all();
            return;
        }

        // Find in Files: Ctrl+Shift+F
        if e.ctrl() && e.shift() && e.keycode == b'F' as u32 {
            commands::switch_sidebar_view(2);
            app().search_panel.search_field.focus();
            return;
        }

        // AI Assistant: Ctrl+Shift+A
        if e.ctrl() && e.shift() && e.keycode == b'A' as u32 {
            commands::switch_sidebar_view(5);
            return;
        }

        // Build: F7
        if e.keycode == anyui::KEY_F7 {
            commands::build();
            return;
        }

        // Run: F5
        if e.keycode == anyui::KEY_F5 {
            commands::run();
            return;
        }

        // Stop: Shift+F5
        if e.shift() && e.keycode == anyui::KEY_F5 {
            commands::stop();
            return;
        }

        // Toggle breakpoint: F9
        if e.keycode == anyui::KEY_F9 {
            commands::toggle_breakpoint_at_cursor();
            return;
        }

        // New File: Ctrl+N
        if e.ctrl() && e.keycode == b'N' as u32 {
            commands::new_file();
            return;
        }

        // Close Tab: Ctrl+W
        if e.ctrl() && e.keycode == b'W' as u32 {
            let s = app();
            if s.file_mgr.count() > 0 {
                commands::close_tab(s.file_mgr.active);
            }
            return;
        }

        // Toggle Terminal: Ctrl+` (backtick = 0x60)
        if e.ctrl() && e.keycode == 0x60 {
            app().output.show_terminal();
            return;
        }
    });
}

// ── Activity bar ───────────────────────────────────────────────

pub fn wire_activity_bar() {
    app()
        .activity_bar
        .btn_files
        .on_click(|_| commands::switch_sidebar_view(0));
    app()
        .activity_bar
        .btn_git
        .on_click(|_| commands::switch_sidebar_view(1));
    app()
        .activity_bar
        .btn_search
        .on_click(|_| commands::switch_sidebar_view(2));
    app()
        .activity_bar
        .btn_run
        .on_click(|_| commands::switch_sidebar_view(3));
    app().activity_bar.btn_outline.on_click(|_| {
        commands::refresh_symbols();
        commands::switch_sidebar_view(4);
    });
    app().activity_bar.btn_ai.on_click(|_| {
        let s = app();
        if !s.ai_client.config.is_configured() {
            s.ai_panel.show_setup_needed();
        }
        s.ai_panel.set_provider(s.ai_client.config.provider);
        commands::switch_sidebar_view(5);
    });
    app()
        .activity_bar
        .btn_extensions
        .on_click(|_| commands::switch_sidebar_view(6));
}

// ── Toolbar ────────────────────────────────────────────────────

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

// ── Menu bar ───────────────────────────────────────────────────

pub fn wire_menu(menu: &anyui::MenuBar) {
    menu.on_item(|e| match e.item_id {
        1 => commands::new_file(),
        2 => commands::open_folder(),
        3 => commands::save(),
        4 => commands::save_all(),
        5 => anyui::quit(),
        10..=13 => {}
        14 => commands::switch_sidebar_view(2),
        20 => commands::switch_sidebar_view(0),
        21 => commands::switch_sidebar_view(1),
        22 => commands::switch_sidebar_view(2),
        23 => commands::switch_sidebar_view(3),
        24 => commands::switch_sidebar_view(4),
        25 => commands::switch_sidebar_view(6),
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
        41 => app().command_palette.show_commands(),
        50 => commands::switch_sidebar_view(5),
        51 => commands::ai_action(ai::CodeAction::Explain),
        52 => commands::ai_action(ai::CodeAction::Refactor),
        53 => commands::ai_action(ai::CodeAction::Fix),
        54 => commands::ai_action(ai::CodeAction::Generate),
        55 => commands::ai_action(ai::CodeAction::Test),
        56 => commands::ai_action(ai::CodeAction::Review),
        57 => commands::ai_settings(),
        _ => {}
    });
}

// ── Sidebar (file explorer) ────────────────────────────────────

pub fn wire_sidebar() {
    app().sidebar.tree.on_drag_start(|_| {
        app().sidebar.begin_drag_from_selection();
    });

    app().sidebar.tree.on_drop(|_| {
        let s = app();
        if s.sidebar.move_drag_payload_to_hovered_dir().is_some() {
            if let Some(ref proj) = s.current_project {
                s.sidebar.populate_project(proj, &s.task_mgr);
            }
            commands::update_status();
        }
    });

    // File tree selection opens file
    app().sidebar.tree.on_selection_changed(|e| {
        let s = app();
        let idx = e.index;
        if idx != u32::MAX && s.sidebar.is_file_node(idx) {
            if let Some(p) = s.sidebar.path_for_node(idx) {
                let owned = String::from(p);
                commands::open_file(&owned);
                commands::update_status();
            }
        }
    });

    // Search field: live filter as user types
    app().sidebar.search.on_text_changed(|_| {
        let s = app();
        let mut buf = [0u8; 128];
        let len = s.sidebar.search.get_text(&mut buf);
        if len == 0 {
            // Empty filter → refresh full tree
            if let Some(ref proj) = s.current_project {
                s.sidebar.populate_project(proj, &s.task_mgr);
            }
        } else if let Ok(filter) = core::str::from_utf8(&buf[..len as usize]) {
            s.sidebar.filter_tree(filter);
        }
    });

    // Context menu
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
                    s.sidebar.populate_project(proj, &s.task_mgr);
                }
            }
            1 => {
                let new_path = path::join(&dir, "new_folder");
                let _ = anyos_std::fs::mkdir(&new_path);
                if let Some(ref proj) = s.current_project {
                    s.sidebar.populate_project(proj, &s.task_mgr);
                }
            }
            3 => {
                let sel = s.sidebar.tree.selected();
                if sel != u32::MAX {
                    if let Some(p) = s.sidebar.path_for_node(sel) {
                        let owned = String::from(p);
                        anyos_std::fs::unlink(&owned);
                        if let Some(ref proj) = s.current_project {
                            s.sidebar.populate_project(proj, &s.task_mgr);
                        }
                    }
                }
            }
            _ => {}
        }
    });

    app().sidebar.tree.on_enter(|_| {
        app().sidebar.start_rename();
    });

    app().sidebar.rename_field.on_submit(|_| {
        let s = app();
        s.sidebar.finish_rename();
        if let Some(ref proj) = s.current_project {
            s.sidebar.populate_project(proj, &s.task_mgr);
        }
    });
}

// ── Search panel ───────────────────────────────────────────────

pub fn wire_search_panel() {
    app()
        .search_panel
        .btn_search
        .on_click(|_| commands::search_in_project());
    app()
        .search_panel
        .search_field
        .on_submit(|_| commands::search_in_project());

    app().search_panel.results_tree.on_selection_changed(|e| {
        let s = app();
        if let Some((file_path, _line)) = s.search_panel.path_for_node(e.index) {
            let owned = String::from(file_path);
            commands::open_file(&owned);
            commands::update_status();
        }
    });
}

// ── Run panel ──────────────────────────────────────────────────

pub fn wire_run_panel() {
    app().run_panel.btn_run.on_click(|_| commands::run());
    app().run_panel.btn_debug.on_click(|_| commands::start_debugging());
    app().run_panel.btn_build.on_click(|_| commands::build());
    app().run_panel.btn_test.on_click(|_| commands::test());
    app().run_panel.btn_stop.on_click(|_| commands::stop());

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
            s.run_panel.update_debug_session(&s.debug_session);
        }
    });

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

// ── Symbols panel ──────────────────────────────────────────────

pub fn wire_symbols_panel() {
    app().symbols_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some(line) = s.symbols_panel.line_for_node(e.index) {
            // Navigate editor to the symbol's line
            if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
                editor.set_cursor(line, 0);
                editor.ensure_line_visible(line);
            }
        }
    });
}

// ── AI panel ───────────────────────────────────────────────────

pub fn wire_ai_panel() {
    app().ai_panel.btn_send.on_click(|_| commands::ai_chat());
    app()
        .ai_panel
        .input_field
        .on_submit(|_| commands::ai_chat());

    app()
        .ai_panel
        .btn_explain
        .on_click(|_| commands::ai_action(ai::CodeAction::Explain));
    app()
        .ai_panel
        .btn_refactor
        .on_click(|_| commands::ai_action(ai::CodeAction::Refactor));
    app()
        .ai_panel
        .btn_fix
        .on_click(|_| commands::ai_action(ai::CodeAction::Fix));
    app()
        .ai_panel
        .btn_generate
        .on_click(|_| commands::ai_action(ai::CodeAction::Generate));
    app()
        .ai_panel
        .btn_test
        .on_click(|_| commands::ai_action(ai::CodeAction::Test));
    app()
        .ai_panel
        .btn_review
        .on_click(|_| commands::ai_action(ai::CodeAction::Review));

    app().ai_panel.btn_clear.on_click(|_| {
        let s = app();
        s.ai_client.clear_history();
        s.ai_panel.clear_chat();
    });

    app()
        .ai_panel
        .btn_settings
        .on_click(|_| commands::ai_settings());
}

// ── Extensions panel ───────────────────────────────────────────

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

// ── Problems panel ─────────────────────────────────────────────

pub fn wire_problems_panel() {
    app()
        .problems_panel
        .btn_all
        .on_click(|_| commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::All));
    app()
        .problems_panel
        .btn_errors
        .on_click(|_| commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::Errors));
    app()
        .problems_panel
        .btn_warnings
        .on_click(|_| commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::Warnings));
    app()
        .problems_panel
        .btn_current_file
        .on_click(|_| commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::CurrentFile));

    app().problems_panel.tree.on_selection_changed(|e| {
        let s = app();
        if let Some((file_path, line, column)) = s.problems_panel.location_for_node(e.index) {
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
                // Navigate to the error line
                if line > 0 {
                    let s = app();
                    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
                        editor.set_cursor(line.saturating_sub(1), column.saturating_sub(1));
                        editor.ensure_line_visible(line.saturating_sub(1));
                    }
                }
                commands::update_status();
            }
        }
    });
}

// ── Git panel ──────────────────────────────────────────────────

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

    app()
        .git_panel
        .btn_refresh
        .on_click(|_| crate::trigger_git_refresh());

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
}

// ── Editor tab bar + modification tracking ─────────────────────

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

    app().side_editor_view.tab_bar.on_active_changed(|e| {
        let s = app();
        let idx = e.index as usize;
        if idx < s.side_file_mgr.count() {
            s.side_file_mgr.set_active(idx);
            s.side_editor_view.set_active(idx);
        }
    });

    app().side_editor_view.tab_bar.on_tab_close(|e| {
        commands::close_side_tab(e.index as usize);
    });
}

/// Wire text-changed event on a newly created editor (called from editor_view).
/// This enables file-modified tracking and live symbol updates.
pub fn wire_editor_text_changed(editor_index: usize) {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(editor_index) {
        editor.on_text_changed(move |_| {
            let s = app();
            s.file_mgr.mark_modified(editor_index);
            s.editor_view
                .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
            if editor_index == s.file_mgr.active {
                commands::update_status();
                commands::refresh_symbols();
            }
            if s.config.auto_save {
                commands::autosave_editor(editor_index);
            }
            commands::schedule_live_check(editor_index);
        });
    }
}

// ── Welcome tab ────────────────────────────────────────────────

pub fn wire_welcome_tab() {
    app()
        .welcome
        .btn_new_file
        .on_click(|_| commands::new_file());
    app()
        .welcome
        .btn_open_folder
        .on_click(|_| commands::open_folder());
    app()
        .welcome
        .btn_open_recent
        .on_click(|_| commands::show_recent_projects());
    app()
        .welcome
        .btn_ai_setup
        .on_click(|_| commands::ai_settings());
}

// ── Command palette ────────────────────────────────────────────

pub fn wire_command_palette() {
    // Live filter as user types
    app().command_palette.input_field.on_text_changed(|_| {
        let s = app();
        let filter = s.command_palette.get_filter();
        s.command_palette.update_list(&filter);
    });

    // Enter executes selected command
    app().command_palette.input_field.on_submit(|_| {
        let s = app();
        if s.command_palette.is_file_mode() {
            if let Some(path) = s.command_palette.selected_file_path() {
                let owned = String::from(path);
                s.command_palette.hide();
                commands::open_file(&owned);
                commands::update_status();
            }
        } else if s.command_palette.is_project_mode() {
            if let Some(path) = s.command_palette.selected_project_path() {
                let owned = String::from(path);
                s.command_palette.hide();
                commands::open_workspace(&owned, true);
            }
        } else if let Some(cmd_id) = s.command_palette.selected_command_id() {
            s.command_palette.hide();
            execute_palette_command(cmd_id);
        }
    });

    // Click on list item executes command
    app().command_palette.list.on_enter(|_| {
        let s = app();
        if s.command_palette.is_file_mode() {
            if let Some(path) = s.command_palette.selected_file_path() {
                let owned = String::from(path);
                s.command_palette.hide();
                commands::open_file(&owned);
                commands::update_status();
            }
        } else if s.command_palette.is_project_mode() {
            if let Some(path) = s.command_palette.selected_project_path() {
                let owned = String::from(path);
                s.command_palette.hide();
                commands::open_workspace(&owned, true);
            }
        } else if let Some(cmd_id) = s.command_palette.selected_command_id() {
            s.command_palette.hide();
            execute_palette_command(cmd_id);
        }
    });
}

fn execute_palette_command(cmd_id: u32) {
    match cmd_id {
        100 => commands::new_file(),
        101 => commands::open_folder(),
        102 => commands::save(),
        103 => commands::save_all(),
        104 => {
            let s = app();
            if s.file_mgr.count() > 0 {
                commands::close_tab(s.file_mgr.active);
            }
        }
        105 => commands::toggle_editor_split(),
        106 => commands::open_active_file_to_side(),
        107 => commands::show_recent_projects(),
        110 => commands::build(),
        111 => commands::run(),
        112 => commands::test(),
        113 => commands::check(),
        114 => commands::clean(),
        115 => commands::stop(),
        141 => commands::set_build_configuration(crate::logic::project::BuildConfiguration::Debug),
        142 => commands::set_build_configuration(crate::logic::project::BuildConfiguration::Release),
        144 => commands::start_debugging(),
        145 => commands::toggle_breakpoint_at_cursor(),
        116 => commands::analyze_active_file(),
        117 => commands::restart_live_analysis(),
        118 => commands::clear_problems(),
        119 => commands::next_problem(),
        120 => commands::switch_sidebar_view(0),
        121 => commands::switch_sidebar_view(1),
        122 => commands::switch_sidebar_view(2),
        123 => commands::switch_sidebar_view(3),
        124 => commands::switch_sidebar_view(4),
        125 => commands::switch_sidebar_view(6),
        126 => commands::switch_sidebar_view(5),
        127 => app().output.show_output(),
        128 => app().output.show_problems(),
        129 => app().output.show_terminal(),
        130 => commands::ai_action(ai::CodeAction::Explain),
        131 => commands::ai_action(ai::CodeAction::Refactor),
        132 => commands::ai_action(ai::CodeAction::Fix),
        133 => commands::ai_action(ai::CodeAction::Generate),
        134 => commands::ai_action(ai::CodeAction::Test),
        135 => commands::ai_action(ai::CodeAction::Review),
        136 => commands::previous_problem(),
        137 => commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::All),
        138 => commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::Errors),
        139 => commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::Warnings),
        140 => commands::set_problem_filter(crate::ui::problems_panel::ProblemFilter::CurrentFile),
        143 => commands::rebuild_symbol_index(),
        160 => commands::open_settings(),
        161 => commands::ai_settings(),
        199 => commands::about(),
        _ => {}
    }
}

// ── Terminal ───────────────────────────────────────────────────

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

// ── Timers ─────────────────────────────────────────────────────

pub fn wire_timers() {
    // Cursor position update (500ms)
    anyui::set_timer(500, || {
        let s = app();
        if s.file_mgr.count() > 0 {
            let (row, col) = s.editor_view.get_cursor(s.file_mgr.active);
            s.status.set_cursor(row, col);
        }
    });

    // Terminal output poll (200ms)
    anyui::set_timer(200, || {
        let s = app();
        if s.output.shell_tid != 0 {
            s.output.poll_shell_output();
        }
    });
}
