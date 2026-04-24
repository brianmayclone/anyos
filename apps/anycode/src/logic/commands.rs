use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::app;
use crate::logic::{
    ai, build, diagnostics, file_manager, git, language, language_service, live_analysis, project,
    search, symbols, tasks,
};
use crate::util::path;
use crate::AppState;

// ════════════════════════════════════════════════════════════════
//  IDE commands — each function implements one user action
// ════════════════════════════════════════════════════════════════

pub fn new_file() {
    let s = app();
    let (_idx, ref p) = s.file_mgr.add_untitled(&s.config.temp_dir);
    s.editor_view.create_editor(p, None, &s.config);
    let count = s.file_mgr.count();
    s.editor_view.set_active(count - 1);
    s.file_mgr.set_active(count - 1);
    s.editor_view
        .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    update_status();
}

pub fn open_folder() {
    if let Some(folder) = libanyui_client::FileDialog::open_folder() {
        open_workspace(&folder, true);
    }
}

pub fn save() {
    let s = app();
    let active = s.file_mgr.active;
    save_current(s);
    s.editor_view
        .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    schedule_live_check(active);
}

pub fn save_all() {
    let s = app();
    let count = s.file_mgr.count();
    save_all_files(s);
    s.editor_view
        .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    for idx in 0..count {
        schedule_live_check(idx);
    }
}

pub fn build() {
    let s = app();
    // Try task manager first
    if let Some(task) = s.task_mgr.selected_build() {
        let task_clone = task.clone();
        execute_task_direct(&task_clone);
        return;
    }
    // Legacy fallback
    if let Some(ref proj) = s.current_project {
        s.output.clear();
        s.diagnostics.clear();
        s.build_output_buffer.clear();
        let active_file = s
            .file_mgr
            .active_file()
            .map(|f| f.path.as_str())
            .unwrap_or("");
        let (cmd, args) = if let Some(ca) =
            s.build_rules
                .build_command(active_file, &proj.root, &s.config)
        {
            ca
        } else {
            build::build_command(proj.build_type, &s.config)
        };
        let msg = format!("$ {}", path::basename(&cmd));
        s.output.append_line(&msg);
        s.output.show_output();
        anyos_std::fs::chdir(&proj.root);
        s.build_process = build::BuildProcess::spawn(&cmd, &args);
        if s.build_process.is_some() {
            crate::start_build_timer();
        }
    }
}

pub fn run() {
    let s = app();
    if let Some(task) = s.task_mgr.selected_run() {
        let task_clone = task.clone();
        execute_task_direct(&task_clone);
        return;
    }
    // Legacy fallback
    if let Some(ref proj) = s.current_project {
        s.output.clear();
        let active_file = s
            .file_mgr
            .active_file()
            .map(|f| f.path.as_str())
            .unwrap_or("");
        let (cmd, args) = if let Some(ca) =
            s.build_rules
                .run_command(active_file, &proj.root, &s.config)
        {
            ca
        } else {
            build::run_command(proj.build_type, &s.config)
        };
        let msg = format!("$ {}", path::basename(&cmd));
        s.output.append_line(&msg);
        s.output.show_output();
        anyos_std::fs::chdir(&proj.root);
        s.build_process = build::BuildProcess::spawn(&cmd, &args);
        if s.build_process.is_some() {
            crate::start_build_timer();
        }
    }
}

pub fn test() {
    let s = app();
    let test_tasks: Vec<usize> = s
        .task_mgr
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.category == tasks::TaskCategory::Test)
        .map(|(i, _)| i)
        .collect();
    if let Some(&idx) = test_tasks.first() {
        execute_task(idx);
    }
}

pub fn check() {
    let s = app();
    let check_tasks: Vec<usize> = s
        .task_mgr
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.category == tasks::TaskCategory::Check)
        .map(|(i, _)| i)
        .collect();
    if let Some(&idx) = check_tasks.first() {
        execute_task(idx);
    }
}

pub fn clean() {
    let s = app();
    let clean_tasks: Vec<usize> = s
        .task_mgr
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.category == tasks::TaskCategory::Clean)
        .map(|(i, _)| i)
        .collect();
    if let Some(&idx) = clean_tasks.first() {
        execute_task(idx);
    }
}

pub fn stop() {
    let s = app();
    if let Some(ref mut proc) = s.build_process {
        proc.kill();
        s.output.append_line("\n[Process killed]");
    }
    if let Some(ref mut proc) = s.live_check_process {
        proc.kill();
        s.status.set_analysis_status("Live check stopped");
    }
    s.build_process = None;
    s.live_check_process = None;
    s.live_check.reset();
    crate::stop_build_timer();
    crate::stop_live_check_timer();
}

pub fn search_in_project() {
    let s = app();
    let query = s.search_panel.get_query();
    if query.is_empty() {
        return;
    }
    if let Some(ref proj) = s.current_project {
        let results = search::search_in_project(&proj.root, &query, 200);
        s.search_panel.show_results(&results, &query);
    }
}

pub fn about() {
    libanyui_client::MessageBox::show(
        libanyui_client::MessageBoxType::Info,
        "anyOS Code v2.0\n\nProfessional IDE for anyOS\n\nSupports: Rust, C, C++, Python, JavaScript,\nTypeScript, Shell, Makefile\n\nFeatures:\n- Multi-project detection\n- Automatic task/target discovery\n- Symbol outline\n- Diagnostics parsing\n- Git integration\n- Plugin system\n- Project-wide search & replace",
        Some("OK"),
    );
}

pub fn open_settings() {
    crate::ui::settings_dialog::show();
}

// ════════════════════════════════════════════════════════════════
//  AI commands
// ════════════════════════════════════════════════════════════════

/// Send a chat message to the AI.
pub fn ai_chat() {
    let s = app();
    let input = s.ai_panel.get_input();
    if input.is_empty() {
        return;
    }
    s.ai_panel.clear_input();
    s.ai_panel.append_user_message(&input);
    s.ai_panel.set_status("Thinking...");

    match s.ai_client.chat(&input) {
        Ok(response) => {
            s.ai_panel.append_ai_response(&response);
            s.ai_panel.set_status("");
        }
        Err(err) => {
            s.ai_panel.append_error(&err);
            s.ai_panel.set_status("Error");
        }
    }
}

/// Execute an AI code action on the current editor content.
pub fn ai_action(action: ai::CodeAction) {
    let s = app();

    // Get current file content (or selection — full file for now)
    if s.file_mgr.count() == 0 {
        s.ai_panel.append_error("No file open.");
        return;
    }

    let filename = path::basename(&s.file_mgr.active_file().unwrap().path);
    let lang = language::language_for_filename(filename);

    let mut buf = vec![0u8; 64 * 1024];
    let len = s.editor_view.get_editor_text(s.file_mgr.active, &mut buf);
    if len == 0 {
        s.ai_panel.append_error("Editor is empty.");
        return;
    }

    let code = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(s) => s,
        Err(_) => {
            s.ai_panel.append_error("Invalid UTF-8 in editor.");
            return;
        }
    };

    // Switch to AI panel
    switch_sidebar_view(5);
    s.ai_panel.set_status(&format!("{}...", action.label()));

    match s
        .ai_client
        .code_action(action, code, lang.id.display_name())
    {
        Ok(response) => {
            s.ai_panel
                .append_user_message(&format!("[{}] {}", action.label(), filename));
            s.ai_panel.append_ai_response(&response);
            s.ai_panel.set_status("");
        }
        Err(err) => {
            s.ai_panel.append_error(&err);
            s.ai_panel.set_status("Error");
        }
    }
}

/// Open the AI settings dialog.
pub fn ai_settings() {
    crate::ui::ai_settings_dialog::AiSettingsDialog::show();
}

// ════════════════════════════════════════════════════════════════
//  File operations
// ════════════════════════════════════════════════════════════════

pub fn open_file(file_path: &str) {
    let s = app();
    // Hide welcome tab when opening a file
    s.welcome.hide();

    if let Some(idx) = s.file_mgr.find_open(file_path) {
        s.file_mgr.set_active(idx);
        s.editor_view.set_active(idx);
        schedule_live_check(idx);
        return;
    }
    let content = file_manager::read_file(file_path);
    let idx = s.file_mgr.add_file(file_path);
    s.editor_view
        .create_editor(file_path, content.as_deref(), &s.config);
    s.file_mgr.set_active(idx);
    s.editor_view.set_active(idx);
    s.editor_view
        .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    refresh_editor_diagnostics();
    schedule_live_check(idx);
    persist_session();
}

pub fn open_file_to_side(file_path: &str) {
    let s = app();
    ensure_split_visible();

    if let Some(idx) = s.side_file_mgr.find_open(file_path) {
        s.side_file_mgr.set_active(idx);
        s.side_editor_view.set_active(idx);
        s.side_editor_view
            .update_tab_labels(&s.side_file_mgr.tab_labels(), s.side_file_mgr.active);
        return;
    }

    let content = file_manager::read_file(file_path);
    let idx = s.side_file_mgr.add_file(file_path);
    s.side_editor_view
        .create_editor_with_mode(file_path, content.as_deref(), &s.config, true);
    s.side_file_mgr.set_active(idx);
    s.side_editor_view.set_active(idx);
    s.side_editor_view.set_breadcrumb(file_path);
    s.side_editor_view
        .update_tab_labels(&s.side_file_mgr.tab_labels(), s.side_file_mgr.active);
}

pub fn open_active_file_to_side() {
    let s = app();
    if let Some(file) = s.file_mgr.active_file() {
        let owned = file.path.clone();
        open_file_to_side(&owned);
    }
}

pub fn close_tab(index: usize) {
    let s = app();
    if index >= s.file_mgr.count() {
        return;
    }
    s.editor_view.remove_editor(index);
    let new_active = s.file_mgr.remove(index);
    if s.file_mgr.count() > 0 {
        s.editor_view.set_active(new_active);
        s.editor_view
            .update_tab_labels(&s.file_mgr.tab_labels(), new_active);
    } else {
        s.editor_view.update_tab_labels("", 0);
    }
    update_status();
    persist_session();
}

pub fn close_side_tab(index: usize) {
    let s = app();
    if index >= s.side_file_mgr.count() {
        return;
    }
    s.side_editor_view.remove_editor(index);
    let new_active = s.side_file_mgr.remove(index);
    if s.side_file_mgr.count() > 0 {
        s.side_editor_view.set_active(new_active);
        s.side_editor_view
            .update_tab_labels(&s.side_file_mgr.tab_labels(), new_active);
    } else {
        s.side_editor_view.update_tab_labels("", 0);
        s.side_editor_view
            .set_breadcrumb("Open a file to the side for reference");
    }
}

pub fn toggle_editor_split() {
    let s = app();
    s.split_visible = !s.split_visible;
    if s.split_visible {
        s.editor_groups_split.set_split_ratio(58);
        s.side_editor_view.panel.set_visible(true);
    } else {
        s.editor_groups_split.set_split_ratio(100);
        s.side_editor_view.panel.set_visible(false);
    }
}

fn save_current(s: &mut AppState) {
    if s.file_mgr.count() == 0 {
        return;
    }
    save_index(s, s.file_mgr.active);
}

fn save_all_files(s: &mut AppState) {
    for i in 0..s.file_mgr.count() {
        if s.file_mgr.files[i].modified {
            save_index(s, i);
        }
    }
}

fn save_index(s: &mut AppState, index: usize) {
    let mut buf = vec![0u8; 128 * 1024];
    let len = s.editor_view.get_editor_text(index, &mut buf);
    if let Some(f) = s.file_mgr.files.get(index) {
        if file_manager::write_file(&f.path, &buf[..len as usize]) {
            s.file_mgr.mark_saved(index);
        }
    }
}

pub fn autosave_editor(index: usize) {
    let s = app();
    if !s.config.auto_save || index >= s.file_mgr.count() {
        return;
    }
    save_index(s, index);
    s.editor_view
        .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
}

// ════════════════════════════════════════════════════════════════
//  Live analysis
// ════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct ProblemTarget {
    file_path: String,
    line: u32,
    column: u32,
    message: String,
}

pub fn schedule_live_check(editor_index: usize) {
    run_static_analysis_for_editor(editor_index);

    let s = app();
    let file = match s.file_mgr.files.get(editor_index) {
        Some(file) => file,
        None => return,
    };
    let file_path = file.path.clone();
    let file_version = file.version;
    s.diagnostics
        .remove_source_for_file(live_analysis::LIVE_CHECK_SOURCE, &file_path);
    s.live_check.queue(editor_index, file_version, 3);
    refresh_problem_views();
    crate::start_live_check_timer();
}

pub fn analyze_active_file() {
    let active = {
        let s = app();
        if s.file_mgr.count() == 0 {
            s.status.set_analysis_status("No file to analyze");
            return;
        }
        s.file_mgr.active
    };
    schedule_live_check(active);
    app().output.show_problems();
    update_status();
}

pub fn restart_live_analysis() {
    let active = {
        let s = app();
        if s.file_mgr.count() == 0 {
            s.status.set_analysis_status("No file to analyze");
            return;
        }
        if let Some(proc) = s.live_check_process.as_mut() {
            proc.kill();
        }
        s.live_check_process = None;
        s.live_check.reset();
        s.diagnostics.remove_source(live_analysis::LIVE_SOURCE);
        s.diagnostics.remove_source(live_analysis::LIVE_CHECK_SOURCE);
        s.status.set_analysis_status("Live analysis restarted");
        s.file_mgr.active
    };
    crate::stop_live_check_timer();
    refresh_problem_views();
    schedule_live_check(active);
    app().output.show_problems();
    update_status();
}

pub fn clear_problems() {
    {
        let s = app();
        if let Some(proc) = s.live_check_process.as_mut() {
            proc.kill();
        }
        s.live_check_process = None;
        s.diagnostics.clear();
        s.live_check.reset();
        s.status.set_analysis_status("Problems cleared");
    }
    crate::stop_live_check_timer();
    refresh_problem_views();
    update_status();
    app().output.show_problems();
}

pub fn next_problem() {
    navigate_problem(ProblemDirection::Next);
}

pub fn previous_problem() {
    navigate_problem(ProblemDirection::Previous);
}

#[derive(Clone, Copy)]
enum ProblemDirection {
    Next,
    Previous,
}

fn navigate_problem(direction: ProblemDirection) {
    let (targets, active_file, cursor_line, cursor_col) = {
        let s = app();
        let project_root = s.current_project.as_ref().map(|p| p.root.as_str());
        let mut targets = Vec::new();
        for diag in &s.diagnostics.diagnostics {
            if !diag.has_location() {
                continue;
            }
            targets.push(ProblemTarget {
                file_path: resolve_diagnostic_path(&diag.file_path, project_root),
                line: diag.line,
                column: diag.column,
                message: diag.message.clone(),
            });
        }
        targets.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line.cmp(&b.line))
                .then(a.column.cmp(&b.column))
        });

        let active_file = s.file_mgr.active_file().map(|f| f.path.clone());
        let (row, col) = if active_file.is_some() {
            s.editor_view.get_cursor(s.file_mgr.active)
        } else {
            (0, 0)
        };
        (targets, active_file, row + 1, col + 1)
    };

    if targets.is_empty() {
        let s = app();
        s.status.set_analysis_status("No problems");
        s.output.show_problems();
        return;
    }

    let idx = select_problem_target(&targets, active_file.as_deref(), cursor_line, cursor_col, direction);
    open_problem_target(&targets[idx]);
}

fn select_problem_target(
    targets: &[ProblemTarget],
    active_file: Option<&str>,
    cursor_line: u32,
    cursor_col: u32,
    direction: ProblemDirection,
) -> usize {
    let active_file = match active_file {
        Some(file) => file,
        None => {
            return match direction {
                ProblemDirection::Next => 0,
                ProblemDirection::Previous => targets.len() - 1,
            };
        }
    };

    match direction {
        ProblemDirection::Next => targets
            .iter()
            .position(|target| problem_after(target, active_file, cursor_line, cursor_col))
            .unwrap_or(0),
        ProblemDirection::Previous => targets
            .iter()
            .rposition(|target| problem_before(target, active_file, cursor_line, cursor_col))
            .unwrap_or(targets.len() - 1),
    }
}

fn problem_after(target: &ProblemTarget, file_path: &str, line: u32, column: u32) -> bool {
    target.file_path.as_str() > file_path
        || (target.file_path == file_path
            && (target.line > line || (target.line == line && target.column > column)))
}

fn problem_before(target: &ProblemTarget, file_path: &str, line: u32, column: u32) -> bool {
    target.file_path.as_str() < file_path
        || (target.file_path == file_path
            && (target.line < line || (target.line == line && target.column < column)))
}

fn open_problem_target(target: &ProblemTarget) {
    open_file(&target.file_path);
    {
        let s = app();
        if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
            editor.set_cursor(target.line.saturating_sub(1), target.column.saturating_sub(1));
        }
        let label = format!("Problem: {}", target.message);
        s.status.set_analysis_status(&label);
        s.output.show_problems();
    }
    update_status();
}

fn resolve_diagnostic_path(file_path: &str, project_root: Option<&str>) -> String {
    if file_path.starts_with('/') {
        return String::from(file_path);
    }
    if let Some(root) = project_root {
        return path::join(root, file_path);
    }
    String::from(file_path)
}

pub fn poll_live_check() {
    let mut finished_output: Option<String> = None;

    {
        let s = app();
        if let Some(ref mut proc) = s.live_check_process {
            let mut buf = [0u8; 1024];
            while let Some(n) = proc.poll_output(&mut buf) {
                if let Ok(text) = core::str::from_utf8(&buf[..n]) {
                    s.live_check.output_buffer.push_str(text);
                }
            }
            if proc.check_finished().is_some() {
                finished_output = Some(s.live_check.output_buffer.clone());
                s.live_check_process = None;
                s.live_check.output_buffer.clear();
            }
        }
    }

    if let Some(output) = finished_output {
        finish_external_live_check(&output);
    }

    let s = app();
    if s.live_check_process.is_some() {
        return;
    }

    if s.live_check.debounce_ticks > 0 {
        s.live_check.debounce_ticks -= 1;
        return;
    }

    if let Some(check) = s.live_check.take_pending() {
        if s.build_process.is_some() {
            s.live_check.requeue(check, 2);
            s.status.set_analysis_status("Analysis waiting");
            return;
        }
        if start_external_live_check(check.editor_index, check.version) {
            return;
        }
    }

    if s.live_check.pending.is_none() && s.live_check_process.is_none() {
        crate::stop_live_check_timer();
    }
}

fn run_static_analysis_for_editor(editor_index: usize) {
    let s = app();
    let file = match s.file_mgr.files.get(editor_index) {
        Some(file) => file,
        None => return,
    };
    let file_path = file.path.clone();

    let mut buf = vec![0u8; 256 * 1024];
    let len = s.editor_view.get_editor_text(editor_index, &mut buf);
    let text = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(text) => text,
        Err(_) => {
            s.diagnostics
                .remove_source_for_file(live_analysis::LIVE_SOURCE, &file_path);
            s.diagnostics.diagnostics.push(diagnostics::Diagnostic {
                severity: diagnostics::Severity::Error,
                file_path,
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
                message: String::from("file is not valid UTF-8"),
                code: None,
                source: String::from(live_analysis::LIVE_SOURCE),
            });
            refresh_problem_views();
            return;
        }
    };

    let live_diags = language_service::analyze_document(&file_path, text);
    s.diagnostics
        .remove_source_for_file(live_analysis::LIVE_SOURCE, &file_path);
    s.diagnostics.append_many(live_diags);
    refresh_problem_views();
}

fn start_external_live_check(editor_index: usize, expected_version: u32) -> bool {
    let s = app();
    let file = match s.file_mgr.files.get(editor_index) {
        Some(file) => file,
        None => return false,
    };
    if file.version != expected_version {
        s.live_check.queue(editor_index, file.version, 2);
        s.status.set_analysis_status("Analysis queued");
        crate::start_live_check_timer();
        return true;
    }
    if file.is_untitled {
        s.status.set_analysis_status("Live analysis");
        return false;
    }
    if file.modified {
        s.status.set_analysis_status("Live analysis: unsaved");
        return false;
    }

    let ctx = language_service::ServiceContext {
        config: &s.config,
        project: s.current_project.as_ref(),
        task_mgr: &s.task_mgr,
    };
    let cmd = match language_service::check_command(&file.path, &ctx) {
        Some(cmd) => cmd,
        None => {
            s.status.set_analysis_status("Live analysis");
            return false;
        }
    };

    if !cmd.working_dir.is_empty() {
        anyos_std::fs::chdir(&cmd.working_dir);
    }
    s.live_check
        .begin_running(&file.path, file.version, &cmd.label);
    s.live_check_process = build::BuildProcess::spawn(&cmd.command, &cmd.args);
    if s.live_check_process.is_some() {
        s.status
            .set_analysis_status(&format!("Checking: {}", cmd.label));
        true
    } else {
        s.status.set_analysis_status("Live check failed");
        s.live_check.finish_running();
        false
    }
}

fn finish_external_live_check(output: &str) {
    let s = app();
    let is_stale = if let Some(ref running) = s.live_check.running {
        s.file_mgr
            .files
            .iter()
            .any(|file| file.path == running.file_path && file.version != running.version)
    } else {
        false
    };
    if is_stale {
        s.live_check.finish_running();
        s.status.set_analysis_status("Stale check discarded");
        refresh_problem_views();
        return;
    }

    s.diagnostics.remove_source(live_analysis::LIVE_CHECK_SOURCE);
    if !output.trim().is_empty() {
        let mut parsed = diagnostics::DiagnosticSet::new();
        parsed.parse_output(output);
        for diag in &mut parsed.diagnostics {
            diag.source = String::from(live_analysis::LIVE_CHECK_SOURCE);
        }
        s.diagnostics.append_many(parsed.diagnostics);
    }

    let label = if s.live_check.label.is_empty() {
        String::from("Live analysis")
    } else {
        format!("Checked: {}", s.live_check.label)
    };
    s.status.set_analysis_status(&label);
    s.live_check.finish_running();
    refresh_problem_views();
}

fn refresh_problem_views() {
    let s = app();
    s.problems_panel.update(&s.diagnostics);
    refresh_editor_diagnostics();
    update_status();
}

// ════════════════════════════════════════════════════════════════
//  Task execution
// ════════════════════════════════════════════════════════════════

pub fn execute_task(task_idx: usize) {
    let s = app();
    if let Some(task) = s.task_mgr.tasks.get(task_idx) {
        let task_clone = task.clone();
        execute_task_direct(&task_clone);
    }
}

fn execute_task_direct(task: &tasks::Task) {
    let s = app();
    s.output.clear();
    s.diagnostics.clear();
    s.build_output_buffer.clear();

    let msg = format!("$ {}", task.display_label);
    s.output.append_line(&msg);
    s.output.show_output();

    if !task.working_dir.is_empty() {
        anyos_std::fs::chdir(&task.working_dir);
    }

    s.build_process = build::BuildProcess::spawn(&task.command, &task.args);
    if s.build_process.is_some() {
        crate::start_build_timer();
    }
}

// ════════════════════════════════════════════════════════════════
//  Status / symbol refresh
// ════════════════════════════════════════════════════════════════

pub fn update_status() {
    let s = app();
    if let Some(f) = s.file_mgr.active_file() {
        let filename = path::basename(&f.path);
        s.status.set_filename(filename);
        let lang = language::language_for_filename(filename);
        s.status.set_language(lang.id.display_name());
        s.editor_view.set_breadcrumb(&f.path);
    } else {
        s.status.set_filename("No file open");
        s.status.set_language("Plain Text");
        s.editor_view.set_breadcrumb("No file open");
    }
    s.status.set_branch(&s.git_state.branch);
    s.status
        .set_problems(s.diagnostics.error_count(), s.diagnostics.warning_count());
}

pub fn show_recent_projects() {
    let s = app();
    s.command_palette
        .show_recent_projects(&s.config.recent_projects);
}

pub fn open_workspace(folder: &str, should_restore_session: bool) {
    let s = app();
    reset_workspace_views();
    s.sidebar.populate(folder);
    let proj = project::Project::open(folder);
    s.status.set_project_type(proj.project_type.display_name());

    s.task_mgr.detect_from_project(&proj);
    s.run_panel.update(&s.task_mgr);

    s.git_state.is_repo = git::is_git_repo(folder);
    if s.git_state.is_repo {
        crate::trigger_git_refresh();
    }
    s.status.set_branch("");
    s.output.start_shell(folder);

    s.current_project = Some(proj);
    s.config.last_project = String::from(folder);
    s.config.push_recent_project(folder);
    s.config.save();
    update_status();

    if should_restore_session {
        restore_session();
    }

    let s = app();
    if s.file_mgr.count() == 0 {
        s.welcome.show();
    }
}

pub fn restore_session() {
    let s = app();
    let project_root = match s.current_project.as_ref() {
        Some(proj) => proj.root.clone(),
        None => return,
    };
    if s.config.session_project != project_root {
        return;
    }

    let files = s.config.session_files.clone();
    let active_file = s.config.session_active_file.clone();
    for file in files {
        if path::exists(&file) {
            open_file(&file);
        }
    }

    if !active_file.is_empty() {
        let s = app();
        if let Some(idx) = s.file_mgr.find_open(&active_file) {
            s.file_mgr.set_active(idx);
            s.editor_view.set_active(idx);
            s.editor_view
                .update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
        }
    }
    update_status();
}

pub fn persist_session() {
    let s = app();
    s.config.session_project = s
        .current_project
        .as_ref()
        .map(|proj| proj.root.clone())
        .unwrap_or_else(String::new);
    s.config.session_files.clear();
    for file in &s.file_mgr.files {
        if !file.is_untitled && path::exists(&file.path) {
            s.config.session_files.push(file.path.clone());
        }
    }
    s.config.session_active_file = s
        .file_mgr
        .active_file()
        .map(|f| f.path.clone())
        .unwrap_or_else(String::new);
    s.config.save();
}

fn ensure_split_visible() {
    let s = app();
    if !s.split_visible {
        s.split_visible = true;
        s.editor_groups_split.set_split_ratio(58);
        s.side_editor_view.panel.set_visible(true);
    }
}

fn reset_workspace_views() {
    let s = app();
    if let Some(ref mut proc) = s.live_check_process {
        proc.kill();
    }
    s.live_check_process = None;
    s.live_check.reset();
    s.diagnostics.remove_source(live_analysis::LIVE_SOURCE);
    s.diagnostics.remove_source(live_analysis::LIVE_CHECK_SOURCE);
    s.status.set_analysis_status("");
    crate::stop_live_check_timer();

    while s.file_mgr.count() > 0 {
        s.editor_view.remove_editor(s.file_mgr.count() - 1);
        s.file_mgr.remove(s.file_mgr.count() - 1);
    }
    s.editor_view.update_tab_labels("", 0);
    s.editor_view.set_breadcrumb("No file open");

    while s.side_file_mgr.count() > 0 {
        s.side_editor_view
            .remove_editor(s.side_file_mgr.count() - 1);
        s.side_file_mgr.remove(s.side_file_mgr.count() - 1);
    }
    s.side_editor_view.update_tab_labels("", 0);
    s.side_editor_view
        .set_breadcrumb("Open a file to the side for reference");
}

pub fn refresh_symbols() {
    let s = app();
    if let Some(f) = s.file_mgr.active_file() {
        let filename = path::basename(&f.path);
        let lang = language::language_for_filename(filename);

        let mut buf = vec![0u8; 128 * 1024];
        let len = s.editor_view.get_editor_text(s.file_mgr.active, &mut buf);
        if len > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..len as usize]) {
                let syms = symbols::extract_symbols(text, lang.id);
                s.symbols_panel.update(&syms, filename);
            }
        }
    } else {
        s.symbols_panel.clear();
    }
}

pub fn refresh_editor_diagnostics() {
    let s = app();
    let project_root = s.current_project.as_ref().map(|p| p.root.as_str());
    for (idx, file) in s.file_mgr.files.iter().enumerate() {
        let editor = match s.editor_view.editor_widget(idx) {
            Some(editor) => editor,
            None => continue,
        };
        editor.clear_diagnostics();
        for diag in &s.diagnostics.diagnostics {
            if !diag.has_location()
                || !diagnostic_matches_file(&diag.file_path, &file.path, project_root)
            {
                continue;
            }
            editor.add_diagnostic(
                diag.line.saturating_sub(1),
                diag.column.saturating_sub(1),
                diag.end_line.saturating_sub(1),
                diag.end_column.saturating_sub(1),
                severity_to_editor(diag.severity),
            );
        }
    }
}

fn diagnostic_matches_file(diag_path: &str, file_path: &str, project_root: Option<&str>) -> bool {
    if diag_path == file_path {
        return true;
    }
    if let Some(root) = project_root {
        if path::join(root, diag_path) == file_path {
            return true;
        }
    }
    false
}

fn severity_to_editor(severity: diagnostics::Severity) -> u32 {
    match severity {
        diagnostics::Severity::Error => 0,
        diagnostics::Severity::Warning => 1,
        diagnostics::Severity::Info => 2,
        diagnostics::Severity::Hint => 3,
    }
}

// ════════════════════════════════════════════════════════════════
//  Sidebar view switching
// ════════════════════════════════════════════════════════════════

pub fn switch_sidebar_view(index: u32) {
    let s = app();
    for (i, &panel_id) in s.panel_ids.iter().enumerate() {
        libanyui_client::Control::from_id(panel_id).set_visible(i as u32 == index);
    }
    s.activity_bar.set_active(index);
}
