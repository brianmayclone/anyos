use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::config::{self, Config};
use crate::logic::diagnostics::Diagnostic;
use crate::logic::language::{self, LanguageId};
use crate::logic::live_analysis::{self, CheckCommand};
use crate::logic::project::Project;
use crate::logic::tasks::{TaskCategory, TaskManager};
use crate::util::path;

pub struct ServiceContext<'a> {
    pub config: &'a Config,
    pub project: Option<&'a Project>,
    pub task_mgr: &'a TaskManager,
}

pub fn analyze_document(file_path: &str, text: &str) -> Vec<Diagnostic> {
    let filename = path::basename(file_path);
    let lang = language::language_for_filename(filename);
    live_analysis::analyze_buffer(file_path, text, lang.id)
}

pub fn check_command(file_path: &str, ctx: &ServiceContext<'_>) -> Option<CheckCommand> {
    if let Some(task) = ctx
        .task_mgr
        .tasks
        .iter()
        .find(|task| task.category == TaskCategory::Check)
    {
        return Some(CheckCommand {
            command: task.command.clone(),
            args: task.args.clone(),
            working_dir: task.working_dir.clone(),
            label: task.display_label.clone(),
        });
    }

    let filename = path::basename(file_path);
    let lang = language::language_for_filename(filename);
    match lang.id {
        LanguageId::C | LanguageId::Cpp => {
            if ctx.config.cc_path.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: ctx.config.cc_path.clone(),
                args: format!("-fsyntax-only {}", file_path),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("cc -fsyntax-only"),
            })
        }
        LanguageId::Python => {
            let python = find_first_tool(&["python3", "python"]);
            if python.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: python,
                args: format!("-m py_compile {}", file_path),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("python py_compile"),
            })
        }
        LanguageId::Shell => {
            let shell = find_first_tool(&["sh", "bash"]);
            if shell.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: shell,
                args: format!("-n {}", file_path),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("shell syntax"),
            })
        }
        LanguageId::JavaScript => {
            let node = config::find_tool("node");
            if node.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: node,
                args: format!("--check {}", file_path),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("node --check"),
            })
        }
        LanguageId::TypeScript => {
            let tsc = config::find_tool("tsc");
            if tsc.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: tsc,
                args: format!("--noEmit {}", file_path),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("tsc --noEmit"),
            })
        }
        _ => None,
    }
}

fn find_first_tool(names: &[&str]) -> String {
    for name in names {
        let path = config::find_tool(name);
        if !path.is_empty() {
            return path;
        }
    }
    String::new()
}

fn project_root_or_parent(file_path: &str, project: Option<&Project>) -> String {
    if let Some(project) = project {
        return project.root.clone();
    }
    String::from(path::parent(file_path))
}
