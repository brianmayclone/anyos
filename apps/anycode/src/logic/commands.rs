use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::app;
use crate::AppState;
use crate::logic::{build, file_manager, git, language, project, search, symbols, tasks};
use crate::util::path;

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
    s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
    update_status();
}

pub fn open_folder() {
    if let Some(folder) = libanyui_client::FileDialog::open_folder() {
        let s = app();
        s.sidebar.populate(&folder);
        let proj = project::Project::open(&folder);
        s.status.set_project_type(proj.project_type.display_name());

        // Detect tasks
        s.task_mgr.detect_from_project(&proj);
        s.run_panel.update(&s.task_mgr);

        // Git
        s.git_state.is_repo = git::is_git_repo(&folder);
        if s.git_state.is_repo {
            crate::trigger_git_refresh();
        }
        s.status.set_branch("");

        // Shell
        s.output.start_shell(&folder);

        s.current_project = Some(proj);
        update_status();
    }
}

pub fn save() {
    let s = app();
    save_current(s);
    s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
}

pub fn save_all() {
    let s = app();
    save_all_files(s);
    s.editor_view.update_tab_labels(&s.file_mgr.tab_labels(), s.file_mgr.active);
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
        let active_file = s.file_mgr.active_file().map(|f| f.path.as_str()).unwrap_or("");
        let (cmd, args) = if let Some(ca) = s.build_rules.build_command(active_file, &proj.root, &s.config) {
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
        let active_file = s.file_mgr.active_file().map(|f| f.path.as_str()).unwrap_or("");
        let (cmd, args) = if let Some(ca) = s.build_rules.run_command(active_file, &proj.root, &s.config) {
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
    let test_tasks: Vec<usize> = s.task_mgr.tasks.iter()
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
    let check_tasks: Vec<usize> = s.task_mgr.tasks.iter()
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
    let clean_tasks: Vec<usize> = s.task_mgr.tasks.iter()
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
    s.build_process = None;
    crate::stop_build_timer();
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
    let s = app();
    let settings = s.config.settings_path.clone();
    open_file(&settings);
    update_status();
}

// ════════════════════════════════════════════════════════════════
//  File operations
// ════════════════════════════════════════════════════════════════

pub fn open_file(file_path: &str) {
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

pub fn close_tab(index: usize) {
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

fn save_all_files(s: &mut AppState) {
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
    } else {
        s.status.set_filename("No file open");
        s.status.set_language("Plain Text");
    }
    s.status.set_branch(&s.git_state.branch);
    s.status.set_problems(s.diagnostics.error_count(), s.diagnostics.warning_count());
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
