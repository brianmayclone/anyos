use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::app;
use crate::logic::{
    ai, build, crates, debug_session, designer, diagnostics, file_manager, git, intellisense,
    language, language_service, live_analysis, project, search, storyboard, symbols, tasks,
};
use crate::ui::problems_panel::ProblemFilter;
use crate::util::path;
use crate::AppState;

// ════════════════════════════════════════════════════════════════
//  IDE commands — each function implements one user action
// ════════════════════════════════════════════════════════════════

pub fn new_file() {
    crate::ui::new_item_dialog::show();
}

pub fn new_text_file() {
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
    if s.current_project.is_none() {
        s.status
            .set_analysis_status("Open a project before building");
        update_action_state();
        return;
    }
    // Try task manager first
    if let Some(task) = s.task_mgr.selected_build() {
        let task_clone = task.clone();
        execute_task_direct(&task_clone);
        return;
    }
    if !can_use_legacy_build_fallback(s) {
        s.status
            .set_analysis_status("No build task: configure the project toolchain");
        s.output.show_output();
        s.output
            .append_line("[Build] No valid build task was detected for this project.");
        s.output
            .append_line("[Build] Open Settings > Toolchains and configure ccargo/crust/cc.");
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
        s.active_task_category = Some(tasks::TaskCategory::Build);
        if s.build_process.is_some() {
            crate::start_build_timer();
        }
        update_action_state();
    }
}

pub fn run() {
    let s = app();
    if s.current_project.is_none() {
        s.status
            .set_analysis_status("Open a project before running");
        update_action_state();
        return;
    }
    if let Some(task) = s.task_mgr.selected_run() {
        let task_clone = task.clone();
        execute_task_direct(&task_clone);
        return;
    }
    s.status.set_analysis_status("No run target selected");
    if !s.task_mgr.tasks.is_empty() {
        return;
    }
    if !can_use_legacy_build_fallback(s) {
        s.output.show_output();
        s.output
            .append_line("[Run] No runnable target was detected for this project.");
        s.output
            .append_line("[Run] Select or configure a Run task in the Run and Debug panel.");
        return;
    }
    // Legacy fallback for old workspaces without detected tasks.
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
        s.active_task_category = Some(tasks::TaskCategory::Run);
        if s.build_process.is_some() {
            crate::start_build_timer();
        }
        update_action_state();
    }
}

pub fn test() {
    let s = app();
    if s.current_project.is_none() {
        s.status
            .set_analysis_status("Open a project before running tests");
        update_action_state();
        return;
    }
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
    } else {
        s.status.set_analysis_status("No test task detected");
        s.output.show_output();
        s.output
            .append_line("[Test] No test task was detected for this project.");
        s.output
            .append_line("[Test] Run task discovery or configure a ccargo test target.");
        update_action_state();
    }
}

pub fn check() {
    let s = app();
    if s.current_project.is_none() {
        s.status
            .set_analysis_status("Open a project before checking");
        update_action_state();
        return;
    }
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
    } else {
        s.status.set_analysis_status("No check task detected");
        s.output.show_output();
        s.output
            .append_line("[Check] No check task was detected for this project.");
        s.output
            .append_line("[Check] Configure ccargo/crust in Settings > Toolchains.");
        update_action_state();
    }
}

pub fn clean() {
    let s = app();
    if s.current_project.is_none() {
        s.status
            .set_analysis_status("Open a project before cleaning");
        update_action_state();
        return;
    }
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
    } else {
        s.status.set_analysis_status("No clean task detected");
        s.output.show_output();
        s.output
            .append_line("[Clean] No clean task was detected for this project.");
        update_action_state();
    }
}

pub fn stop() {
    let s = app();
    let had_debug = s.debug_backend.is_attached()
        || s.debug_session.status != debug_session::DebugSessionStatus::Idle;
    let had_build = s.build_process.is_some();
    let had_live_check = s.live_check_process.is_some();
    if s.debug_backend.is_attached() {
        s.debug_backend.detach();
    }
    if let Some(ref mut proc) = s.build_process {
        proc.kill();
        s.output.append_line("\n[Process killed]");
    }
    if let Some(ref mut proc) = s.live_check_process {
        proc.kill();
        s.status.set_analysis_status("Live check stopped");
    }
    s.build_process = None;
    s.active_task_category = None;
    s.live_check_process = None;
    s.live_check.reset();
    s.debug_session.stop();
    s.output.append_debug_line("stop");
    s.run_panel.update_debug_session(&s.debug_session);
    crate::stop_build_timer();
    crate::stop_debug_timer();
    crate::stop_live_check_timer();
    if !had_debug && !had_build && !had_live_check {
        s.status.set_analysis_status("Nothing is running");
    }
    update_action_state();
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
        "anyOS Code v2.0\n\nRust-first professional IDE for anyOS\n\nSupports: Rust, ccargo, crust, anyrc\n\nFeatures:\n- Rust project and target discovery\n- Live diagnostics\n- Symbol outline\n- Codex integration\n- Native plugin foundation\n- Git integration\n- Project-wide search & replace",
        Some("OK"),
    );
}

pub fn open_settings() {
    crate::ui::settings_dialog::show();
}

pub fn configure_run_profiles() {
    crate::ui::run_config_dialog::show();
}

pub fn save_run_configuration(
    name: String,
    target: String,
    kind_index: u32,
    profile_index: u32,
    args: String,
    working_dir: String,
    package: String,
) {
    let s = app();
    if name.trim().is_empty() || target.trim().is_empty() {
        s.status
            .set_analysis_status("Run configuration needs a name and Cargo target");
        return;
    }
    let Some(ref mut proj) = s.current_project else {
        s.status.set_analysis_status("No Cargo workspace open");
        return;
    };
    if proj.project_type != project::ProjectType::Cargo
        && proj.project_type != project::ProjectType::RustFolder
    {
        s.status
            .set_analysis_status("Run configurations are stored in Cargo.toml");
        return;
    }
    let manifest_root = if proj.project_type == project::ProjectType::RustFolder {
        match find_run_config_project_root(proj, &target, &package) {
            Some(root) => root,
            None => {
                s.status
                    .set_analysis_status("Select a Cargo target from a discovered Rust project");
                return;
            }
        }
    } else {
        proj.root.clone()
    };
    let cfg = project::CargoRunConfig {
        id: project::run_config_id(&name),
        name,
        target,
        kind: match kind_index {
            1 => project::TargetKind::Example,
            2 => project::TargetKind::Test,
            3 => project::TargetKind::Bench,
            _ => project::TargetKind::Binary,
        },
        profile: if profile_index == 1 {
            project::BuildConfiguration::Release
        } else {
            project::BuildConfiguration::Debug
        },
        args,
        working_dir,
        package,
    };
    match project::save_cargo_run_config(&manifest_root, &cfg) {
        Ok(()) => {
            proj.refresh();
            s.task_mgr.detect_from_project(proj, &s.config);
            s.test_explorer.refresh_from_project(proj);
            s.task_mgr.select_run_by_name(&cfg.name);
            s.run_panel.update(&s.task_mgr);
            s.run_panel.update_tests(&s.test_explorer);
            s.run_panel.update_debug_session(&s.debug_session);
            s.sidebar.populate_project(proj, &s.task_mgr);
            refresh_run_config_selector();
            s.status
                .set_analysis_status(&format!("Run configuration saved: {}", cfg.name));
        }
        Err(err) => s.status.set_analysis_status(err),
    }
}

fn find_run_config_project_root(
    proj: &project::Project,
    target: &str,
    package: &str,
) -> Option<String> {
    for cargo_project in &proj.cargo_projects {
        if !package.is_empty() && cargo_project.name != package && cargo_project.rel_path != package
        {
            continue;
        }
        if cargo_project.targets.iter().any(|t| t.name == target) {
            return Some(cargo_project.root.clone());
        }
    }
    proj.cargo_projects
        .first()
        .map(|cargo_project| cargo_project.root.clone())
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

    let active_path = s.file_mgr.active_file().unwrap().path.clone();
    let filename = path::basename(&active_path);
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

    let diagnostics = ai_diagnostics_context(&active_path);
    let symbols = ai_symbols_context(&active_path);
    let project = s
        .current_project
        .as_ref()
        .map(|p| p.display_context())
        .unwrap_or_default();

    match s.ai_client.code_action_with_context(ai::AiCodeContext {
        action,
        code,
        language: lang.id.display_name(),
        file_path: &active_path,
        diagnostics: &diagnostics,
        symbols: &symbols,
        project: &project,
    }) {
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

fn ai_diagnostics_context(file_path: &str) -> String {
    let s = app();
    let mut out = String::new();
    for diag in s.diagnostics.for_file(file_path).into_iter().take(20) {
        if !out.is_empty() {
            out.push('\n');
        }
        let code = diag.code.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{}:{}:{} [{} {}] {}",
            path::basename(&diag.file_path),
            diag.line,
            diag.column,
            diag.source,
            code,
            diag.message
        ));
    }
    out
}

fn ai_symbols_context(file_path: &str) -> String {
    let s = app();
    let mut out = String::new();
    for symbol in s
        .symbol_index
        .symbols
        .iter()
        .filter(|symbol| symbol.file_path == file_path)
        .take(40)
    {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "{} {} at line {}",
            symbol.kind.label(),
            symbol.name,
            symbol.line + 1
        ));
    }
    out
}

/// Open the AI settings dialog.
pub fn ai_settings() {
    crate::ui::ai_settings_dialog::AiSettingsDialog::show();
}

// ════════════════════════════════════════════════════════════════
//  Editor intelligence
// ════════════════════════════════════════════════════════════════

pub fn show_completion_list() {
    let s = app();
    let (file_path, text, row, col) = match active_editor_context() {
        Some(ctx) => ctx,
        None => return,
    };

    let set = intellisense::completions_for_cursor(&file_path, &text, row, col, &s.symbol_index);
    if set.items.is_empty() {
        s.editor_view.hide_completions();
        s.status.set_analysis_status("IntelliSense: no suggestions");
        return;
    }

    let detail = String::from(
        set.items
            .first()
            .map(|item| item.detail.as_str())
            .unwrap_or(""),
    );
    let list_text = intellisense::completion_list_text(&set.items);
    s.active_completions = set.items;
    s.active_completion_prefix = set.prefix;
    s.editor_view.show_completions(&list_text, &detail);
    s.status.set_analysis_status(&format!(
        "IntelliSense: {} suggestions",
        s.active_completions.len()
    ));
}

pub fn accept_completion(index: usize) {
    let s = app();
    if s.file_mgr.count() == 0 || index >= s.active_completions.len() {
        return;
    }

    let insert_text = s.active_completions[index].insert_text.clone();
    let prefix_len = s.active_completion_prefix.len();
    let suffix = if insert_text.len() >= prefix_len
        && insert_text[..prefix_len].eq_ignore_ascii_case(&s.active_completion_prefix)
    {
        String::from(&insert_text[prefix_len..])
    } else {
        insert_text
    };

    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.insert_text(&suffix);
        editor.focus();
    }
    s.editor_view.hide_completions();
    s.active_completions.clear();
    s.active_completion_prefix.clear();
    schedule_live_check(s.file_mgr.active);
}

pub fn update_completion_detail(index: usize) {
    let s = app();
    if let Some(item) = s.active_completions.get(index) {
        s.editor_view.set_completion_detail(&item.detail);
    }
}

pub fn update_editor_hover() {
    let s = app();
    let (file_path, text, row, col) = match active_editor_context() {
        Some(ctx) => ctx,
        None => return,
    };
    let hover = intellisense::hover_for_cursor(&file_path, &text, row, col, &s.symbol_index);
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.set_tooltip(&hover);
    }
}

pub fn go_to_definition_at_cursor() {
    let s = app();
    let (file_path, text, row, col) = match active_editor_context() {
        Some(ctx) => ctx,
        None => return,
    };
    let word = intellisense::word_at_cursor(&text, row, col);
    if word.is_empty() {
        s.status.set_analysis_status("No symbol under cursor");
        return;
    }
    let Some(symbol) = intellisense::best_symbol_for_word(&file_path, &word, &s.symbol_index)
    else {
        s.status
            .set_analysis_status(&format!("No definition found for {}", word));
        return;
    };

    let target_name = symbol.name.clone();
    let target_path = symbol.file_path.clone();
    let target_line = symbol.line;
    open_file(&target_path);
    if let Some(editor) = app().editor_view.editor_widget(app().file_mgr.active) {
        editor.set_cursor(target_line, 0);
        editor.ensure_line_visible(target_line);
    }
    app().status.set_analysis_status(&format!(
        "Definition: {} in {}",
        target_name,
        path::basename(&target_path)
    ));
}

pub fn peek_symbol_at_cursor() {
    let s = app();
    let (file_path, text, row, col) = match active_editor_context() {
        Some(ctx) => ctx,
        None => return,
    };
    let word = intellisense::word_at_cursor(&text, row, col);
    if let Some(symbol) = intellisense::best_symbol_for_word(&file_path, &word, &s.symbol_index) {
        s.output.show_output();
        s.output.append_line(&format!(
            "[Symbol] {} {} at {}:{}",
            symbol.kind.label(),
            symbol.detail,
            symbol.file_path,
            symbol.line + 1
        ));
    }
}

pub fn fold_block_at_cursor() {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.toggle_fold_at_cursor();
        editor.focus();
    }
}

pub fn editor_cut() {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.cut();
    }
}

pub fn editor_copy() {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.copy();
    }
}

pub fn editor_paste() {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.paste();
    }
}

pub fn editor_select_all() {
    let s = app();
    if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
        editor.select_all();
    }
}

fn active_editor_context() -> Option<(String, String, u32, u32)> {
    let s = app();
    let file = s.file_mgr.active_file()?;
    let mut buf = vec![0u8; 128 * 1024];
    let len = s.editor_view.get_editor_text(s.file_mgr.active, &mut buf);
    let text = core::str::from_utf8(&buf[..len as usize]).ok()?;
    let (row, col) = s.editor_view.get_cursor(s.file_mgr.active);
    Some((file.path.clone(), String::from(text), row, col))
}

// ════════════════════════════════════════════════════════════════
//  File operations
// ════════════════════════════════════════════════════════════════

pub fn open_file(file_path: &str) {
    clear_designer_drag_state();

    if let Some(designer_path) = designer::try_create_designer_from_rust_ui(file_path) {
        let s = app();
        s.welcome.hide();
        s.status
            .set_analysis_status("Created Designer metadata from Rust UI");
        open_file(&designer_path);
        return;
    }

    let s = app();
    // Hide welcome tab when opening a file
    s.welcome.hide();

    if let Some(idx) = s.file_mgr.find_open(file_path) {
        s.file_mgr.set_active(idx);
        s.editor_view.set_active(idx);
        refresh_inspector_for_file(file_path);
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
    refresh_inspector_for_file(file_path);
    schedule_live_check(idx);
    persist_session();
}

fn refresh_inspector_for_file(file_path: &str) {
    let s = app();
    if designer::is_designer_file(file_path) {
        if let Some(doc) = designer::load_designer(file_path) {
            s.inspector_panel.show_designer(&doc);
            return;
        }
    }
    s.inspector_panel.show_file(file_path);
}

const DESIGNER_MIN_DRAG_SIZE: i32 = 8;
const DESIGNER_SNAP: i32 = 8;

pub fn designer_pointer_down_at(file_path: &str, x: i32, y: i32) {
    let s = app();
    s.designer_drag_moved = false;
    let doc = match designer::load_designer(file_path) {
        Some(doc) => doc,
        None => return,
    };
    let hit = crate::ui::designer_surface::hit_test_resize_handle(&doc, x, y).or_else(|| {
        crate::ui::designer_surface::hit_test_doc(&doc, x, y)
            .map(|name| (name, crate::ui::designer_surface::DESIGNER_DRAG_MOVE))
    });
    if let Some((control_name, drag_mode)) = hit {
        let Some(control) = doc
            .controls
            .iter()
            .find(|control| control.name == control_name)
        else {
            return;
        };
        s.selected_designer_file = String::from(file_path);
        s.selected_designer_control = control_name.clone();
        s.designer_drag_file = String::from(file_path);
        s.designer_drag_control = control_name.clone();
        s.designer_drag_mode = drag_mode;
        s.designer_drag_start_x = x;
        s.designer_drag_start_y = y;
        s.designer_drag_orig_x = control.x;
        s.designer_drag_orig_y = control.y;
        s.designer_drag_orig_w = control.width;
        s.designer_drag_orig_h = control.height;
        s.designer_drag_moved = false;
        s.inspector_panel.show_designer_control(&doc, &control_name);
        s.editor_view
            .select_designer_control(file_path, &control_name);
        s.status
            .set_analysis_status(&format!("Selected {}", control_name));
    } else {
        s.selected_designer_file = String::from(file_path);
        s.selected_designer_control.clear();
        s.designer_drag_file.clear();
        s.designer_drag_control.clear();
        s.designer_drag_mode = crate::ui::designer_surface::DESIGNER_DRAG_NONE;
        s.designer_drag_moved = false;
        s.inspector_panel.show_designer(&doc);
        s.status.set_analysis_status("Selected form surface");
    }
}

pub fn designer_pointer_move_at(file_path: &str, x: i32, y: i32) {
    update_designer_drag(file_path, x, y, false);
}

pub fn designer_pointer_up_at(file_path: &str, x: i32, y: i32) {
    update_designer_drag(file_path, x, y, true);
    let s = app();
    s.designer_drag_file.clear();
    s.designer_drag_control.clear();
    s.designer_drag_mode = crate::ui::designer_surface::DESIGNER_DRAG_NONE;
}

pub fn designer_zoom(file_path: &str, delta: i32) {
    let s = app();
    let selected =
        if s.selected_designer_file == file_path && !s.selected_designer_control.is_empty() {
            Some(s.selected_designer_control.clone())
        } else {
            None
        };
    s.editor_view
        .zoom_designer(file_path, delta, selected.as_deref());
}

pub fn designer_drop_tool_at(file_path: &str, x: i32, y: i32, payload: &str) {
    let Some(control_kind) = payload.strip_prefix("anycode-control:") else {
        return;
    };
    if control_kind.is_empty() {
        return;
    }
    let s = app();
    let mut doc = match designer::load_designer(file_path) {
        Some(doc) => doc,
        None => return,
    };
    let (form_x, form_y) = crate::ui::designer_surface::canvas_to_form(x, y);
    let parent_name = crate::ui::designer_surface::hit_test_container(&doc, x, y);
    let page_index = parent_name
        .as_ref()
        .and_then(|parent| doc.control_parent_page_index_at_form(parent, form_x, form_y))
        .or_else(|| crate::ui::designer_surface::hit_test_page_index(&doc, x, y));
    let (local_x, local_y) = if let Some(ref parent_name) = parent_name {
        if let Some((parent_x, parent_y)) = doc.control_parent_client_origin(parent_name) {
            (form_x - parent_x, form_y - parent_y)
        } else {
            (form_x, form_y)
        }
    } else {
        (form_x, form_y)
    };
    let local_x = snap_i32(local_x.max(0), DESIGNER_SNAP);
    let local_y = snap_i32(local_y.max(0), DESIGNER_SNAP);
    let control_name = match doc.add_control(
        control_kind,
        local_x,
        local_y,
        parent_name.as_deref(),
        page_index,
    ) {
        Ok(name) => name,
        Err(err) => {
            s.status.set_analysis_status(err);
            return;
        }
    };
    if let Err(err) = designer::save_designer(file_path, &doc) {
        s.status.set_analysis_status(err);
        return;
    }
    s.selected_designer_file = String::from(file_path);
    s.selected_designer_control = control_name.clone();
    s.editor_view
        .update_designer_document(file_path, doc.clone(), Some(&control_name));
    s.inspector_panel.show_designer_control(&doc, &control_name);
    s.status
        .set_analysis_status(&format!("Added {}", control_name));
}

fn update_designer_drag(file_path: &str, x: i32, y: i32, persist: bool) {
    let s = app();
    if s.designer_drag_mode == crate::ui::designer_surface::DESIGNER_DRAG_NONE
        || s.designer_drag_file != file_path
        || s.designer_drag_control.is_empty()
    {
        return;
    }
    let control_name = s.designer_drag_control.clone();
    let drag_mode = s.designer_drag_mode;
    let start_x = s.designer_drag_start_x;
    let start_y = s.designer_drag_start_y;
    let orig_x = s.designer_drag_orig_x;
    let orig_y = s.designer_drag_orig_y;
    let orig_w = s.designer_drag_orig_w;
    let orig_h = s.designer_drag_orig_h;

    let mut doc = match designer::load_designer(file_path) {
        Some(doc) => doc,
        None => return,
    };
    let dx = x - start_x;
    let dy = y - start_y;
    if dx.abs() > 2 || dy.abs() > 2 {
        s.designer_drag_moved = true;
    }
    let (new_x, new_y, new_w, new_h) =
        designer_drag_bounds(drag_mode, orig_x, orig_y, orig_w, orig_h, dx, dy);
    if let Err(err) = doc.set_control_bounds(&control_name, new_x, new_y, new_w, new_h) {
        s.status.set_analysis_status(err);
        return;
    }
    if persist {
        if let Err(err) = designer::save_designer(file_path, &doc) {
            s.status.set_analysis_status(err);
            return;
        }
    }
    s.editor_view
        .update_designer_document(file_path, doc.clone(), Some(&control_name));
    s.inspector_panel.show_designer_control(&doc, &control_name);
    if persist {
        s.status
            .set_analysis_status(&format!("Updated layout for {}", control_name));
    }
}

fn designer_drag_bounds(
    mode: u32,
    orig_x: i32,
    orig_y: i32,
    orig_w: u32,
    orig_h: u32,
    dx: i32,
    dy: i32,
) -> (i32, i32, u32, u32) {
    let w = orig_w as i32;
    let h = orig_h as i32;
    match mode {
        crate::ui::designer_surface::DESIGNER_DRAG_MOVE => (
            snap_i32(orig_x + dx, DESIGNER_SNAP),
            snap_i32(orig_y + dy, DESIGNER_SNAP),
            orig_w,
            orig_h,
        ),
        crate::ui::designer_surface::DESIGNER_DRAG_RESIZE_NW => {
            let adx = dx.min(w - DESIGNER_MIN_DRAG_SIZE);
            let ady = dy.min(h - DESIGNER_MIN_DRAG_SIZE);
            let new_x = snap_i32(orig_x + adx, DESIGNER_SNAP);
            let new_y = snap_i32(orig_y + ady, DESIGNER_SNAP);
            (
                new_x,
                new_y,
                (orig_x + w - new_x).max(DESIGNER_MIN_DRAG_SIZE) as u32,
                (orig_y + h - new_y).max(DESIGNER_MIN_DRAG_SIZE) as u32,
            )
        }
        crate::ui::designer_surface::DESIGNER_DRAG_RESIZE_NE => {
            let ady = dy.min(h - DESIGNER_MIN_DRAG_SIZE);
            let new_y = snap_i32(orig_y + ady, DESIGNER_SNAP);
            let nw = snap_i32((w + dx).max(DESIGNER_MIN_DRAG_SIZE), DESIGNER_SNAP)
                .max(DESIGNER_MIN_DRAG_SIZE) as u32;
            (
                orig_x,
                new_y,
                nw,
                (orig_y + h - new_y).max(DESIGNER_MIN_DRAG_SIZE) as u32,
            )
        }
        crate::ui::designer_surface::DESIGNER_DRAG_RESIZE_SW => {
            let adx = dx.min(w - DESIGNER_MIN_DRAG_SIZE);
            let new_x = snap_i32(orig_x + adx, DESIGNER_SNAP);
            let nh = snap_i32((h + dy).max(DESIGNER_MIN_DRAG_SIZE), DESIGNER_SNAP)
                .max(DESIGNER_MIN_DRAG_SIZE) as u32;
            (
                new_x,
                orig_y,
                (orig_x + w - new_x).max(DESIGNER_MIN_DRAG_SIZE) as u32,
                nh,
            )
        }
        crate::ui::designer_surface::DESIGNER_DRAG_RESIZE_SE => {
            let nw = snap_i32((w + dx).max(DESIGNER_MIN_DRAG_SIZE), DESIGNER_SNAP)
                .max(DESIGNER_MIN_DRAG_SIZE) as u32;
            let nh = snap_i32((h + dy).max(DESIGNER_MIN_DRAG_SIZE), DESIGNER_SNAP)
                .max(DESIGNER_MIN_DRAG_SIZE) as u32;
            (orig_x, orig_y, nw, nh)
        }
        _ => (orig_x, orig_y, orig_w, orig_h),
    }
}

fn snap_i32(value: i32, grid: i32) -> i32 {
    if grid <= 1 {
        return value;
    }
    let half = grid / 2;
    if value >= 0 {
        ((value + half) / grid) * grid
    } else {
        ((value - half) / grid) * grid
    }
}

pub fn designer_double_click_at(file_path: &str, x: i32, y: i32) {
    let s = app();
    if s.designer_drag_moved {
        clear_designer_drag_state();
        s.designer_drag_moved = false;
        s.status
            .set_analysis_status("Ignored double-click while moving designer control");
        return;
    }
    let doc = match designer::load_designer(file_path) {
        Some(doc) => doc,
        None => return,
    };
    let control_name = match crate::ui::designer_surface::hit_test_doc(&doc, x, y) {
        Some(name) => name,
        None => return,
    };
    let control = match doc
        .controls
        .iter()
        .find(|control| control.name == control_name)
    {
        Some(control) => control,
        None => return,
    };
    match designer::ensure_event_handler(file_path, control) {
        Ok(events_path) => {
            s.selected_designer_file = String::from(file_path);
            s.selected_designer_control = control_name.clone();
            s.inspector_panel.show_designer_control(&doc, &control_name);
            clear_designer_drag_state();
            open_file(&events_path);
            jump_to_text_in_active_editor(&format!("pub fn {}()", control.event_name()));
            s.status
                .set_analysis_status(&format!("Opened handler {}", control.event_name()));
        }
        Err(err) => s.status.set_analysis_status(err),
    }
}

fn clear_designer_drag_state() {
    let s = app();
    s.designer_drag_file.clear();
    s.designer_drag_control.clear();
    s.designer_drag_mode = crate::ui::designer_surface::DESIGNER_DRAG_NONE;
    s.designer_drag_start_x = 0;
    s.designer_drag_start_y = 0;
    s.designer_drag_orig_x = 0;
    s.designer_drag_orig_y = 0;
    s.designer_drag_orig_w = 0;
    s.designer_drag_orig_h = 0;
}

fn jump_to_text_in_active_editor(needle: &str) {
    let s = app();
    let mut buf = vec![0u8; 128 * 1024];
    let len = s.editor_view.get_editor_text(s.file_mgr.active, &mut buf);
    let Ok(text) = core::str::from_utf8(&buf[..len as usize]) else {
        return;
    };
    let mut line = 0u32;
    for current in text.split('\n') {
        if current.contains(needle) {
            if let Some(editor) = s.editor_view.editor_widget(s.file_mgr.active) {
                editor.set_cursor(line, 0);
                editor.ensure_line_visible(line);
            }
            s.status.set_cursor(line, 0);
            return;
        }
        line = line.saturating_add(1);
    }
}

pub fn designer_property_selection_changed() {
    let s = app();
    if s.selected_designer_file.is_empty() {
        return;
    }
    let Some(doc) = designer::load_designer(&s.selected_designer_file) else {
        return;
    };
    s.inspector_panel
        .update_property_value_from_selection(&doc, &s.selected_designer_control);
}

pub fn open_designer_color_picker() {
    let s = app();
    let Some((property_name, value)) = selected_designer_property(s) else {
        s.status
            .set_analysis_status("Select a designer color property first");
        return;
    };
    if !is_color_property(&property_name) {
        s.status
            .set_analysis_status("Select TextColor, BackgroundColor or BorderColor first");
        return;
    }
    libanyui_client::ColorPickerDialog::show(&value, apply_designer_property_value);
}

pub fn apply_designer_property() {
    let value = app().inspector_panel.property_value_text();
    apply_designer_property_value(value);
}

pub fn apply_designer_property_from_grid() {
    let value = app().inspector_panel.property_grid_value_text();
    apply_designer_property_value(value);
}

pub fn apply_designer_event_from_grid() {
    let s = app();
    if s.selected_designer_file.is_empty() {
        s.status
            .set_analysis_status("Select a designer surface or control first");
        return;
    }
    let file_path = s.selected_designer_file.clone();
    let control_name = s.selected_designer_control.clone();
    let mut handler = s.inspector_panel.event_grid_handler_text();
    let event_index = s.inspector_panel.selected_event_index();
    if event_index == u32::MAX {
        return;
    }
    let Some(doc) = designer::load_designer(&file_path) else {
        s.status.set_analysis_status("Could not load designer file");
        return;
    };

    if control_name.is_empty() {
        let event_name = doc.form_event_name_at(event_index);
        if handler.is_empty() {
            handler = doc.form_event_value(&event_name);
        }
        match designer::ensure_named_event_handler(&file_path, &handler) {
            Ok(events_path) => {
                open_file(&events_path);
                jump_to_text_in_active_editor(&format!("pub fn {}()", handler));
                s.status
                    .set_analysis_status(&format!("Opened handler {}", handler));
            }
            Err(err) => s.status.set_analysis_status(err),
        }
        return;
    }

    let Some(control) = doc
        .controls
        .iter()
        .find(|control| control.name == control_name)
    else {
        s.status.set_analysis_status("Designer control not found");
        return;
    };
    let event_property = match event_index {
        1 => "OnDoubleClick",
        2 => "OnChanged",
        3 => "OnSubmit",
        _ => "OnClick",
    };
    if handler.is_empty() {
        handler = control.property_value(event_property);
    }
    match designer::ensure_named_event_handler(&file_path, &handler) {
        Ok(events_path) => {
            open_file(&events_path);
            jump_to_text_in_active_editor(&format!("pub fn {}()", handler));
            s.status
                .set_analysis_status(&format!("Opened handler {}", handler));
        }
        Err(err) => s.status.set_analysis_status(err),
    }
}

pub fn apply_designer_property_value(value: String) -> bool {
    let s = app();
    if s.selected_designer_file.is_empty() {
        s.status
            .set_analysis_status("Select a designer surface or control first");
        return false;
    }
    let file_path = s.selected_designer_file.clone();
    let control_name = s.selected_designer_control.clone();
    let mut doc = match designer::load_designer(&file_path) {
        Some(doc) => doc,
        None => {
            s.status.set_analysis_status("Could not load designer file");
            return false;
        }
    };
    let property_index = s.inspector_panel.selected_property_index();
    if control_name.is_empty() {
        let property_name = doc.form_property_name_at(property_index);
        if let Err(err) = doc.update_form_property(&property_name, &value) {
            s.status.set_analysis_status(err);
            return false;
        }
        if let Err(err) = designer::save_designer(&file_path, &doc) {
            s.status.set_analysis_status(err);
            return false;
        }
        s.editor_view
            .update_designer_document(&file_path, doc.clone(), None);
        s.inspector_panel.show_designer(&doc);
        s.status
            .set_analysis_status(&format!("Updated form.{}", property_name));
        return true;
    }
    let property_name = doc.control_property_name_at(&control_name, property_index);
    if property_name.starts_with("On") {
        let Some(control) = doc
            .controls
            .iter()
            .find(|control| control.name == control_name)
        else {
            s.status.set_analysis_status("Designer control not found");
            return false;
        };
        match designer::ensure_control_event_handler(&file_path, control, &property_name) {
            Ok((events_path, handler)) => {
                open_file(&events_path);
                jump_to_text_in_active_editor(&format!("pub fn {}()", handler));
                s.status
                    .set_analysis_status(&format!("Opened handler {}", handler));
            }
            Err(err) => s.status.set_analysis_status(err),
        }
        return true;
    }
    if let Err(err) = doc.update_control_property(&control_name, &property_name, &value) {
        s.status.set_analysis_status(err);
        return false;
    }
    if let Err(err) = designer::save_designer(&file_path, &doc) {
        s.status.set_analysis_status(err);
        return false;
    }
    s.editor_view
        .update_designer_document(&file_path, doc.clone(), Some(&control_name));
    s.inspector_panel.show_designer_control(&doc, &control_name);
    s.status
        .set_analysis_status(&format!("Updated {}.{}", control_name, property_name));
    true
}

fn selected_designer_property(s: &crate::AppState) -> Option<(String, String)> {
    if s.selected_designer_file.is_empty() {
        return None;
    }
    let doc = designer::load_designer(&s.selected_designer_file)?;
    let property_index = s.inspector_panel.selected_property_index();
    if s.selected_designer_control.is_empty() {
        let property_name = doc.form_property_name_at(property_index);
        let value = doc.form_property_value(&property_name);
        Some((property_name, value))
    } else {
        let property_name =
            doc.control_property_name_at(&s.selected_designer_control, property_index);
        let value = doc.control_property_value(&s.selected_designer_control, &property_name);
        Some((property_name, value))
    }
}

fn is_color_property(property_name: &str) -> bool {
    matches!(
        property_name.to_ascii_lowercase().as_str(),
        "textcolor" | "backgroundcolor" | "bordercolor"
    )
}

pub fn delete_selected_designer_control() {
    let s = app();
    if s.selected_designer_file.is_empty() || s.selected_designer_control.is_empty() {
        s.status
            .set_analysis_status("Select a designer control first");
        return;
    }
    let file_path = s.selected_designer_file.clone();
    let control_name = s.selected_designer_control.clone();
    let mut doc = match designer::load_designer(&file_path) {
        Some(doc) => doc,
        None => {
            s.status.set_analysis_status("Could not load designer file");
            return;
        }
    };
    if let Err(err) = doc.remove_control(&control_name) {
        s.status.set_analysis_status(err);
        return;
    }
    if let Err(err) = designer::save_designer(&file_path, &doc) {
        s.status.set_analysis_status(err);
        return;
    }
    s.selected_designer_control.clear();
    s.editor_view
        .update_designer_document(&file_path, doc.clone(), None);
    s.inspector_panel.show_designer(&doc);
    s.status
        .set_analysis_status(&format!("Deleted control {}", control_name));
}

pub fn toggle_inspector() {
    let s = app();
    s.config.inspector_visible = !s.config.inspector_visible;
    s.inspector_panel
        .panel
        .set_visible(s.config.inspector_visible);
    s.config.save();
}

pub fn show_new_ui_form_dialog() {
    let s = app();
    let root = match s.current_project.as_ref() {
        Some(project) => project.root.clone(),
        None => {
            s.status.set_analysis_status("Open a Rust project first");
            return;
        }
    };
    let default_name = designer::next_form_name(&root, "MainForm");
    crate::ui::new_form_dialog::show(&default_name);
}

pub fn create_storyboard() {
    let s = app();
    let root = match s.current_project.as_ref() {
        Some(project) => project.root.clone(),
        None => {
            s.status.set_analysis_status("Open a Rust project first");
            return;
        }
    };
    let ui_dir = format!("{}/src/ui", root);
    let _ = anyos_std::fs::mkdir(&format!("{}/src", root));
    let _ = anyos_std::fs::mkdir(&ui_dir);
    let mut name_idx = 1u32;
    let mut path = format!("{}/Main.Storyboard", ui_dir);
    while crate::util::path::exists(&path) {
        name_idx += 1;
        path = format!("{}/Main{}.Storyboard", ui_dir, name_idx);
    }
    let doc = storyboard::StoryboardDocument {
        name: String::from(crate::util::path::basename(&path).trim_end_matches(".Storyboard")),
        scenes: Vec::new(),
        segues: Vec::new(),
    };
    if let Err(err) = storyboard::save_storyboard(&path, &doc) {
        s.status.set_analysis_status(err);
        return;
    }
    open_file(&path);
    s.status.set_analysis_status("Created Storyboard");
}

pub fn manage_crates() {
    let s = app();
    if s.current_project.is_none() {
        s.status.set_analysis_status("Open a Rust project first");
        return;
    }
    crate::ui::crate_manager_dialog::show();
}

pub fn manage_connected_services() {
    let s = app();
    if s.current_project.is_none() {
        s.status.set_analysis_status("Open a Rust project first");
        return;
    }
    crate::ui::connected_services_dialog::show();
}

pub fn add_connected_service(
    name: String,
    endpoint: String,
    module_name: String,
    kind_index: u32,
) -> Result<String, String> {
    let s = app();
    let Some(ref project) = s.current_project else {
        return Err(String::from("Open a Rust project first"));
    };
    let kind = crate::logic::connected_services::ConnectedServiceKind::from_index(kind_index);
    match crate::logic::connected_services::add_service(
        project,
        &name,
        &endpoint,
        &module_name,
        kind,
    ) {
        Ok(service) => {
            if let Some(ref proj) = s.current_project {
                s.sidebar.populate_project(proj, &s.task_mgr);
            }
            s.status
                .set_analysis_status(&format!("Generated connected service {}", service.name));
            s.output.append_line(&format!(
                "[Connected Services] Generated {} in {}",
                service.name, service.output_dir
            ));
            Ok(format!("Generated {}", service.name))
        }
        Err(err) => {
            s.status.set_analysis_status(err);
            Err(String::from(err))
        }
    }
}

pub fn regenerate_connected_services() -> Result<String, String> {
    let s = app();
    let Some(ref project) = s.current_project else {
        return Err(String::from("Open a Rust project first"));
    };
    let services = crate::logic::connected_services::services_for_project(project);
    let mut count = 0usize;
    for service in &services {
        match crate::logic::connected_services::regenerate_service(project, &service.module_name) {
            Ok(_) => count += 1,
            Err(err) => {
                s.status.set_analysis_status(err);
                return Err(String::from(err));
            }
        }
    }
    match crate::logic::connected_services::regenerate_all(project) {
        Ok(_) => {
            if let Some(ref proj) = s.current_project {
                s.sidebar.populate_project(proj, &s.task_mgr);
            }
            let msg = format!("Regenerated {} connected services", count);
            s.status.set_analysis_status(&msg);
            s.output
                .append_line(&format!("[Connected Services] {}", msg));
            Ok(msg)
        }
        Err(err) => {
            s.status.set_analysis_status(err);
            Err(String::from(err))
        }
    }
}

pub fn remove_first_connected_service() -> Result<String, String> {
    let s = app();
    let Some(ref project) = s.current_project else {
        return Err(String::from("Open a Rust project first"));
    };
    let Some(module_name) = crate::logic::connected_services::first_service_module(project) else {
        return Err(String::from("No connected service to remove"));
    };
    match crate::logic::connected_services::remove_service(project, &module_name) {
        Ok(()) => {
            if let Some(ref proj) = s.current_project {
                s.sidebar.populate_project(proj, &s.task_mgr);
            }
            let msg = format!("Removed connected service {}", module_name);
            s.status.set_analysis_status(&msg);
            s.output
                .append_line(&format!("[Connected Services] {}", msg));
            Ok(msg)
        }
        Err(err) => {
            s.status.set_analysis_status(err);
            Err(String::from(err))
        }
    }
}

pub fn show_project_properties() {
    let s = app();
    if s.current_project.is_none() {
        s.status.set_analysis_status("Open a Rust project first");
        return;
    }
    crate::ui::project_properties_dialog::show();
}

pub fn check_crate_updates_on_open() {
    let s = app();
    let Some(ref project) = s.current_project else {
        return;
    };
    let deps = crates::dependencies_for_project(project);
    if deps.is_empty() {
        return;
    }
    s.output.append_line(&format!(
        "[Crates] {}",
        crates::update_check_message(deps.len())
    ));
}

pub fn add_crate_dependency(name: String, version: String, kind_index: u32) {
    let s = app();
    let Some(ref project) = s.current_project else {
        s.status.set_analysis_status("Open a Rust project first");
        return;
    };
    let kind = crates::DependencyKind::from_index(kind_index);
    match crates::add_dependency(project, &name, &version, kind) {
        Ok(()) => {
            refresh_project_metadata();
            app()
                .status
                .set_analysis_status(&format!("Added crate dependency {}", name.trim()));
        }
        Err(err) => s.status.set_analysis_status(err),
    }
}

pub fn update_crate_dependency(name: String, version: String, kind_index: u32) {
    let s = app();
    let Some(ref project) = s.current_project else {
        s.status.set_analysis_status("Open a Rust project first");
        return;
    };
    let kind = crates::DependencyKind::from_index(kind_index);
    match crates::update_dependency(project, &name, &version, kind) {
        Ok(()) => {
            refresh_project_metadata();
            app()
                .status
                .set_analysis_status(&format!("Updated crate dependency {}", name.trim()));
        }
        Err(err) => s.status.set_analysis_status(err),
    }
}

fn refresh_project_metadata() {
    let s = app();
    if let Some(ref mut project) = s.current_project {
        project.refresh();
        s.task_mgr.detect_from_project(project, &s.config);
        s.test_explorer.refresh_from_project(project);
        s.run_panel.update(&s.task_mgr);
        s.run_panel.update_tests(&s.test_explorer);
        s.run_panel.update_debug_session(&s.debug_session);
        s.sidebar.populate_project(project, &s.task_mgr);
        s.status.set_project_type(&project.display_context());
        refresh_run_config_selector();
    }
}

pub fn create_ui_form_named(form_name: String) -> bool {
    let s = app();
    let root = match s.current_project.as_ref() {
        Some(project) => project.root.clone(),
        None => {
            s.status.set_analysis_status("Open a Rust project first");
            return false;
        }
    };
    if !designer::is_valid_form_name(&form_name) {
        s.status
            .set_analysis_status("Use a valid Rust type name, for example MainForm");
        return false;
    }
    if designer::form_exists(&root, &form_name) {
        s.status
            .set_analysis_status("A UI form with this name already exists");
        return false;
    }
    match designer::create_form_files(&root, &form_name) {
        Ok(()) => {
            let designer_path = designer::designer_file_path(&root, &form_name);
            s.status
                .set_analysis_status("Created Rust UI form and designer files");
            open_file(&designer_path);
            if let Some(ref project) = s.current_project {
                s.sidebar.populate_project(project, &s.task_mgr);
            }
            true
        }
        Err(err) => {
            s.status.set_analysis_status(err);
            false
        }
    }
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
    if !s.editor_view.is_text_editor(index) {
        s.status
            .set_analysis_status("This viewer does not edit text content");
        return;
    }
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
    let s = app();
    if !s.editor_view.is_text_editor(editor_index) {
        if let Some(file) = s.file_mgr.files.get(editor_index) {
            s.diagnostics
                .remove_source_for_file(live_analysis::LIVE_SOURCE, &file.path);
            s.diagnostics
                .remove_source_for_file(live_analysis::LIVE_CHECK_SOURCE, &file.path);
            refresh_problem_views();
        }
        return;
    }
    run_static_analysis_for_editor(editor_index);

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
        s.diagnostics
            .remove_source(live_analysis::LIVE_CHECK_SOURCE);
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

pub fn start_debugging() {
    let task = {
        let s = app();
        if s.current_project.is_none() {
            s.status
                .set_analysis_status("Open a project before debugging");
            update_action_state();
            return;
        }
        match s.task_mgr.selected_run() {
            Some(task) => task.clone(),
            None => {
                s.status
                    .set_analysis_status("No run configuration selected");
                update_action_state();
                return;
            }
        }
    };

    let s = app();
    s.output.clear();
    s.output.clear_debug_console();
    s.diagnostics.clear();
    s.build_output_buffer.clear();
    s.debug_session.start_launch(&task);
    s.run_panel.update_debug_session(&s.debug_session);
    s.output
        .append_line(&format!("[Debug] Launching {}", task.display_label));
    s.output
        .append_debug_line(&format!("launch {}", task.display_label));
    if s.debug_session.breakpoint_count() > 0 {
        s.output.append_debug_line(&format!(
            "{} breakpoint(s) armed",
            s.debug_session.breakpoint_count()
        ));
    }
    s.output.show_output();

    if !task.working_dir.is_empty() {
        anyos_std::fs::chdir(&task.working_dir);
    }

    s.build_process = build::BuildProcess::spawn(&task.command, &task.args);
    s.active_task_category = Some(tasks::TaskCategory::Run);
    if let Some(ref proc) = s.build_process {
        let tid = proc.tid;
        s.output
            .append_debug_line(&format!("process started tid={}", tid));
        if s.debug_backend.attach(tid) {
            s.debug_session.mark_attached(tid, &s.debug_backend.regs);
            refresh_debug_snapshot(s);
            s.output.append_debug_line(&format!(
                "attached tid={} rip={}",
                tid,
                anyos_std::fmt::hex64(s.debug_backend.regs.rip)
            ));
            if s.debug_session.breakpoint_count() > 0 {
                s.output.append_debug_line(
                    "source breakpoints recorded; address binding is pending symbol mapping",
                );
            }
            if s.debug_backend.resume() {
                s.debug_session.mark_running();
                s.status
                    .set_analysis_status("Debug session attached and running");
                s.output.append_debug_line("target resumed");
            } else {
                s.status
                    .set_analysis_status("Debug session attached and paused");
                s.output.append_debug_line("target paused at attach");
            }
            crate::start_debug_timer();
        } else {
            s.debug_session.mark_running();
            s.status
                .set_analysis_status("Debug attach failed; process is running");
            s.output.append_debug_line("debug attach failed");
        }
        crate::start_build_timer();
    } else {
        s.debug_session.stop();
        s.status.set_analysis_status("Debug launch failed");
        s.output.append_debug_line("launch failed");
    }
    s.run_panel.update_debug_session(&s.debug_session);
    update_action_state();
}

pub fn debug_continue() {
    let s = app();
    if s.debug_backend.is_attached() {
        if s.debug_backend.resume() {
            s.debug_session.continue_execution();
            s.status.set_analysis_status("Debug continue");
            s.output.append_debug_line("continue");
            crate::start_debug_timer();
        } else {
            s.status.set_analysis_status("Debug continue unavailable");
            s.output.append_debug_line("continue failed");
        }
    } else {
        s.debug_session.continue_execution();
        s.status.set_analysis_status("Debug continue");
        s.output.append_debug_line("continue");
    }
    s.run_panel.update_debug_session(&s.debug_session);
    s.output.show_debug_console();
}

pub fn debug_pause() {
    let s = app();
    if s.debug_backend.is_attached() {
        if s.debug_backend.suspend() {
            s.debug_session.pause("user pause");
            refresh_debug_snapshot(s);
            s.status.set_analysis_status("Debug paused");
            s.output.append_debug_line(&format!(
                "pause rip={}",
                anyos_std::fmt::hex64(s.debug_backend.regs.rip)
            ));
        } else {
            s.status.set_analysis_status("Debug pause unavailable");
            s.output.append_debug_line("pause failed");
        }
    } else {
        s.debug_session.pause("user pause");
        s.status.set_analysis_status("Debug paused");
        s.output.append_debug_line("pause");
    }
    s.run_panel.update_debug_session(&s.debug_session);
    s.output.show_debug_console();
}

pub fn debug_step_over() {
    let s = app();
    if s.debug_backend.is_attached() {
        if s.debug_backend.is_suspended() && s.debug_backend.single_step() {
            s.debug_session.step_started();
            s.status.set_analysis_status("Debug step over");
            s.output.append_debug_line("step over");
            crate::start_debug_timer();
        } else {
            s.status
                .set_analysis_status("Debug step requires a paused target");
            s.output.append_debug_line("step over unavailable");
        }
    } else {
        s.debug_session.step_started();
        s.status.set_analysis_status("Debug step over");
        s.output.append_debug_line("step over");
    }
    s.run_panel.update_debug_session(&s.debug_session);
    s.output.show_debug_console();
}

pub fn refresh_debug_snapshot(s: &mut AppState) {
    if !s.debug_backend.is_attached() {
        return;
    }

    s.debug_backend.refresh_regs();
    let regs = s.debug_backend.regs;
    let disassembly = disassembly_preview(&s.debug_backend, regs.rip);
    let memory_rows = memory_preview(&s.debug_backend, regs.rsp);

    s.debug_session.apply_registers(&regs);
    s.debug_session.disassembly = disassembly;
    s.debug_session.memory_rows = memory_rows;
}

fn disassembly_preview(
    backend: &crate::logic::debug_backend::DebugBackend,
    rip: u64,
) -> Vec<debug_session::DisassemblyLine> {
    let mut buf = [0u8; 96];
    let read = backend.read_mem(rip, &mut buf);
    let mut lines = Vec::new();
    let mut offset = 0usize;

    while offset < read && lines.len() < 10 {
        let addr = rip + offset as u64;
        let (len, text) = decode_simple_instr(&buf[offset..read], addr);
        let len = len.max(1).min(read - offset);
        lines.push(debug_session::DisassemblyLine {
            address: anyos_std::fmt::hex64(addr),
            bytes: format_bytes(&buf[offset..offset + len]),
            text,
            current: offset == 0,
        });
        offset += len;
    }

    lines
}

fn memory_preview(
    backend: &crate::logic::debug_backend::DebugBackend,
    rsp: u64,
) -> Vec<debug_session::MemoryRow> {
    let mut buf = [0u8; 64];
    let read = backend.read_mem(rsp, &mut buf);
    let mut rows = Vec::new();
    let mut offset = 0usize;

    while offset < read && rows.len() < 8 {
        let end = (offset + 8).min(read);
        rows.push(debug_session::MemoryRow {
            address: anyos_std::fmt::hex64(rsp + offset as u64),
            bytes: format_bytes(&buf[offset..end]),
            ascii: ascii_preview(&buf[offset..end]),
        });
        offset += 8;
    }

    rows
}

fn decode_simple_instr(bytes: &[u8], rip: u64) -> (usize, String) {
    if bytes.is_empty() {
        return (1, String::from("db ?"));
    }

    match bytes[0] {
        0x55 => (1, String::from("push rbp")),
        0x5D => (1, String::from("pop rbp")),
        0x90 => (1, String::from("nop")),
        0xC3 => (1, String::from("ret")),
        0xCC => (1, String::from("int3")),
        0xE8 if bytes.len() >= 5 => {
            let rel = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let target = (rip as i64 + 5 + rel as i64) as u64;
            (5, format!("call {}", anyos_std::fmt::hex64(target)))
        }
        0xE9 if bytes.len() >= 5 => {
            let rel = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let target = (rip as i64 + 5 + rel as i64) as u64;
            (5, format!("jmp {}", anyos_std::fmt::hex64(target)))
        }
        0xEB if bytes.len() >= 2 => {
            let rel = bytes[1] as i8 as i64;
            let target = (rip as i64 + 2 + rel) as u64;
            (2, format!("jmp {}", anyos_std::fmt::hex64(target)))
        }
        0x0F if bytes.len() >= 2 && bytes[1] == 0x05 => (2, String::from("syscall")),
        0x48 if bytes.len() >= 3 && bytes[1] == 0x89 && bytes[2] == 0xE5 => {
            (3, String::from("mov rbp, rsp"))
        }
        0x48 if bytes.len() >= 4 && bytes[1] == 0x83 && bytes[2] == 0xEC => {
            (4, format!("sub rsp, {:#x}", bytes[3]))
        }
        0x48 if bytes.len() >= 4 && bytes[1] == 0x83 && bytes[2] == 0xC4 => {
            (4, format!("add rsp, {:#x}", bytes[3]))
        }
        0x48 if bytes.len() >= 10 && (0xB8..=0xBF).contains(&bytes[1]) => {
            let imm = u64::from_le_bytes([
                bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
            ]);
            let reg = reg_name((bytes[1] - 0xB8) as usize);
            (10, format!("mov {}, {}", reg, anyos_std::fmt::hex64(imm)))
        }
        0xB8..=0xBF if bytes.len() >= 5 => {
            let imm = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let reg = reg32_name((bytes[0] - 0xB8) as usize);
            (5, format!("mov {}, 0x{:08x}", reg, imm))
        }
        b => (1, format!("db 0x{:02x}", b)),
    }
}

fn reg_name(idx: usize) -> &'static str {
    match idx {
        0 => "rax",
        1 => "rcx",
        2 => "rdx",
        3 => "rbx",
        4 => "rsp",
        5 => "rbp",
        6 => "rsi",
        7 => "rdi",
        _ => "r?",
    }
}

fn reg32_name(idx: usize) -> &'static str {
    match idx {
        0 => "eax",
        1 => "ecx",
        2 => "edx",
        3 => "ebx",
        4 => "esp",
        5 => "ebp",
        6 => "esi",
        7 => "edi",
        _ => "e?",
    }
}

fn format_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for (idx, byte) in bytes.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

fn ascii_preview(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        if (0x20..=0x7E).contains(byte) {
            out.push(*byte as char);
        } else {
            out.push('.');
        }
    }
    out
}

pub fn toggle_breakpoint_at_cursor() {
    let s = app();
    let file_path = match s.file_mgr.active_file() {
        Some(file) if !file.is_untitled => file.path.clone(),
        _ => {
            s.status.set_analysis_status("No saved file for breakpoint");
            return;
        }
    };
    let (line, _col) = s.editor_view.get_cursor(s.file_mgr.active);
    let enabled = s.debug_session.toggle_breakpoint(&file_path, line);
    let action = if enabled { "Added" } else { "Removed" };
    s.status
        .set_analysis_status(&format!("{} breakpoint at line {}", action, line + 1));
    s.output.append_line(&format!(
        "[Debug] {} breakpoint: {}:{}",
        action,
        path::basename(&file_path),
        line + 1
    ));
    s.output.append_debug_line(&format!(
        "{} breakpoint {}:{}",
        action,
        path::basename(&file_path),
        line + 1
    ));
    s.run_panel.update_debug_session(&s.debug_session);
}

pub fn set_problem_filter(filter: ProblemFilter) {
    {
        let s = app();
        s.problems_panel.set_filter(filter);
        s.status.set_analysis_status(problem_filter_status(filter));
    }
    refresh_problem_views();
    app().output.show_problems();
}

fn problem_filter_status(filter: ProblemFilter) -> &'static str {
    match filter {
        ProblemFilter::All => "Error List: all problems",
        ProblemFilter::Errors => "Error List: errors",
        ProblemFilter::Warnings => "Error List: warnings",
        ProblemFilter::CurrentFile => "Error List: current file",
    }
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
        let filter = s.problems_panel.filter();
        let active_path = s.file_mgr.active_file().map(|f| f.path.as_str());
        let mut targets = Vec::new();
        for diag in &s.diagnostics.diagnostics {
            if !diag.has_location() {
                continue;
            }
            let resolved = resolve_diagnostic_path(&diag.file_path, project_root);
            if !problem_filter_accepts(filter, diag.severity, &resolved, active_path) {
                continue;
            }
            targets.push(ProblemTarget {
                file_path: resolved,
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

    let idx = select_problem_target(
        &targets,
        active_file.as_deref(),
        cursor_line,
        cursor_col,
        direction,
    );
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
            editor.set_cursor(
                target.line.saturating_sub(1),
                target.column.saturating_sub(1),
            );
        }
        let label = format!("Problem: {}", target.message);
        s.status.set_analysis_status(&label);
        s.output.show_problems();
    }
    update_status();
}

fn problem_filter_accepts(
    filter: ProblemFilter,
    severity: diagnostics::Severity,
    file_path: &str,
    active_file: Option<&str>,
) -> bool {
    match filter {
        ProblemFilter::All => true,
        ProblemFilter::Errors => severity == diagnostics::Severity::Error,
        ProblemFilter::Warnings => severity == diagnostics::Severity::Warning,
        ProblemFilter::CurrentFile => active_file
            .map(|active| {
                file_path == active || path::basename(file_path) == path::basename(active)
            })
            .unwrap_or(false),
    }
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

    let live_diags = language_service::analyze_document_with_config(&file_path, text, &s.config);
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

    s.diagnostics
        .remove_source(live_analysis::LIVE_CHECK_SOURCE);
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
    let active_path = s.file_mgr.active_file().map(|f| f.path.clone());
    s.problems_panel.set_current_file(active_path.as_deref());
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

    if task.command.is_empty() {
        s.status
            .set_analysis_status("Task has no command; configure the toolchain");
        s.output
            .append_line("[Task] Command is empty. Open Settings > Toolchains.");
        return;
    }

    if !task.working_dir.is_empty() {
        anyos_std::fs::chdir(&task.working_dir);
    }

    s.build_process = build::BuildProcess::spawn(&task.command, &task.args);
    s.active_task_category = Some(task.category);
    if s.build_process.is_some() {
        crate::start_build_timer();
    }
    update_action_state();
}

fn can_use_legacy_build_fallback(s: &AppState) -> bool {
    match s.current_project.as_ref().map(|proj| proj.project_type) {
        Some(project::ProjectType::Generic) => true,
        _ => false,
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
    update_action_state();
}

pub fn update_action_state() {
    let s = app();
    let has_project = s.current_project.is_some();
    let has_files = s.file_mgr.count() > 0;
    let process_running = s.build_process.is_some();
    let has_build =
        has_project && (s.task_mgr.selected_build().is_some() || can_use_legacy_build_fallback(s));
    let has_run = has_project
        && (s.task_mgr.selected_run().is_some()
            || (s.task_mgr.tasks.is_empty() && can_use_legacy_build_fallback(s)));
    let has_tests = has_project
        && s.task_mgr
            .tasks
            .iter()
            .any(|task| task.category == tasks::TaskCategory::Test);
    let debug_launching = s.debug_session.status == debug_session::DebugSessionStatus::Launching;
    let debug_running = s.debug_session.status == debug_session::DebugSessionStatus::Running;
    let debug_stopped = s.debug_session.status == debug_session::DebugSessionStatus::Stopped;
    let debug_active = debug_launching || debug_running || debug_stopped;
    let can_continue_debug = debug_stopped;
    let can_pause_debug = debug_launching || debug_running;
    let can_step_debug = debug_stopped;
    let can_stop_debug = process_running || debug_active;

    set_enabled(s.toolbar_save_id, has_files);
    set_enabled(s.toolbar_save_all_id, has_files);
    set_enabled(s.toolbar_build_id, has_build && !process_running);
    set_enabled(
        s.toolbar_run_config_button_id,
        has_project && !process_running,
    );
    set_enabled(s.run_config_dropdown_id, has_project && !process_running);
    set_enabled(s.debug_profile_dropdown_id, has_project && !process_running);
    set_enabled(s.toolbar_run_id, has_run && !process_running);
    set_enabled(s.toolbar_debug_id, has_run && !process_running);
    set_enabled(s.toolbar_debug_continue_id, can_continue_debug);
    set_enabled(s.toolbar_debug_pause_id, can_pause_debug);
    set_enabled(s.toolbar_debug_step_id, can_step_debug);
    set_enabled(s.toolbar_stop_id, can_stop_debug);

    s.run_panel
        .btn_build
        .set_enabled(has_build && !process_running);
    s.run_panel.btn_run.set_enabled(has_run && !process_running);
    s.run_panel
        .btn_test
        .set_enabled(has_tests && !process_running);
    s.run_panel
        .btn_debug
        .set_enabled(has_run && !process_running);
    s.run_panel.btn_continue.set_enabled(can_continue_debug);
    s.run_panel.btn_pause.set_enabled(can_pause_debug);
    s.run_panel.btn_step_over.set_enabled(can_step_debug);
    s.run_panel.btn_stop.set_enabled(can_stop_debug);
}

fn set_enabled(id: u32, enabled: bool) {
    libanyui_client::Control::from_id(id).set_enabled(enabled);
}

pub fn show_recent_projects() {
    let s = app();
    s.command_palette
        .show_recent_projects(&s.config.recent_projects);
}

pub fn set_build_configuration(config: project::BuildConfiguration) {
    let s = app();
    let project_context = {
        let proj = match s.current_project.as_mut() {
            Some(proj) => proj,
            None => {
                s.status.set_analysis_status("No workspace open");
                return;
            }
        };
        proj.set_active_configuration(config);
        proj.display_context()
    };

    if let Some(ref proj) = s.current_project {
        s.task_mgr.detect_from_project(proj, &s.config);
        s.test_explorer.refresh_from_project(proj);
        s.run_panel.update(&s.task_mgr);
        s.run_panel.update_tests(&s.test_explorer);
        s.run_panel.update_debug_session(&s.debug_session);
        s.sidebar.populate_project(proj, &s.task_mgr);
        s.status.set_project_type(&project_context);
        refresh_run_config_selector();
    }
}

pub fn refresh_run_config_selector() {
    let s = app();
    let dropdown = libanyui_client::Control::from_id(s.run_config_dropdown_id);
    if s.current_project.is_none() {
        dropdown.set_text("No project open");
        dropdown.set_state(0);
    } else {
        dropdown.set_text(&s.task_mgr.run_config_labels());
        dropdown.set_state(s.task_mgr.selected_run_dropdown_index());
    }
}

pub fn select_run_config_from_toolbar(index: usize) {
    let s = app();
    if s.current_project.is_none() {
        s.status.set_analysis_status("Open a project first");
        refresh_run_config_selector();
        update_action_state();
        return;
    }
    if let Some(task_idx) = s.task_mgr.run_task_index_for_dropdown(index) {
        s.task_mgr.selected_run_task = task_idx;
        s.run_panel.update(&s.task_mgr);
        s.run_panel.update_debug_session(&s.debug_session);
        refresh_run_config_selector();
        update_action_state();
    }
}

pub fn rebuild_symbol_index() {
    let root = {
        let s = app();
        match s.current_project.as_ref() {
            Some(proj) => proj.root.clone(),
            None => {
                s.status.set_analysis_status("No workspace open");
                return;
            }
        }
    };

    let count = {
        let s = app();
        s.status.set_analysis_status("Indexing workspace...");
        s.symbol_index.rebuild(&root);
        s.symbol_index.count()
    };

    let s = app();
    s.status
        .set_analysis_status(&format!("Symbol index: {} symbols", count));
    s.output
        .append_line(&format!("[IntelliSense] Indexed {} symbols", count));
}

pub fn open_workspace(folder: &str, should_restore_session: bool) {
    let s = app();
    reset_workspace_views();
    let proj = project::Project::open(folder);
    let solution = crate::logic::solution::SolutionMetadata::load(&proj);
    if !path::exists(&solution.path) {
        let _ = solution.save();
    }
    let project_context = proj.display_context();
    let workspace_root = proj.root.clone();

    s.task_mgr.detect_from_project(&proj, &s.config);
    s.test_explorer.refresh_from_project(&proj);
    s.run_panel.update(&s.task_mgr);
    s.run_panel.update_tests(&s.test_explorer);
    s.run_panel.update_debug_session(&s.debug_session);
    s.sidebar.populate_project(&proj, &s.task_mgr);
    refresh_run_config_selector();
    s.status.set_project_type(&project_context);

    s.status.set_branch("");
    s.output.start_shell(&workspace_root);

    s.solution = Some(solution);
    s.current_project = Some(proj);
    s.git_state = git::GitState::empty();
    if let Some(repo_root) = git::find_repository_root(&workspace_root) {
        s.git_state.is_repo = true;
        s.git_state.root = repo_root;
    }
    if !s.config.has_git() {
        s.git_panel.show_not_installed();
        s.activity_bar.set_git_change_count(0);
    } else if s.git_state.is_repo {
        crate::trigger_git_refresh();
    } else {
        s.git_panel.show_no_repo();
        s.activity_bar.set_git_change_count(0);
    }
    check_crate_updates_on_open();
    s.symbol_index.rebuild(&workspace_root);
    let indexed_symbols = s.symbol_index.count();
    s.status
        .set_analysis_status(&format!("Symbol index: {} symbols", indexed_symbols));
    s.config.last_project = workspace_root.clone();
    s.config.push_recent_project(&workspace_root);
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
    s.diagnostics
        .remove_source(live_analysis::LIVE_CHECK_SOURCE);
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
    s.symbol_index.clear();
    s.solution = None;
    s.active_task_category = None;
    if s.debug_backend.is_attached() {
        s.debug_backend.detach();
    }
    s.debug_session.reset();
    s.run_panel.update_debug_session(&s.debug_session);
    crate::stop_debug_timer();
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
