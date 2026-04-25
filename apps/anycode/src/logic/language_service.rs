use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::config::{self, Config};
use crate::logic::diagnostics::Diagnostic;
use crate::logic::diagnostics::Severity;
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

pub fn analyze_document_with_config(
    file_path: &str,
    text: &str,
    config: &Config,
) -> Vec<Diagnostic> {
    let filename = path::basename(file_path);
    let lang = language::language_for_filename(filename);
    let mut diagnostics = live_analysis::analyze_buffer(file_path, text, lang.id);
    if lang.id == LanguageId::Rust && config.rust_use_anyrc_library {
        diagnostics.extend(analyze_rust_with_anyrc(file_path, text));
    }
    diagnostics
}

fn analyze_rust_with_anyrc(file_path: &str, text: &str) -> Vec<Diagnostic> {
    let mut opts = anyrc::driver::CompileOptions::default();
    opts.input = String::from(file_path);
    opts.output = String::from("/tmp/anycode-anyrc-check.o");
    opts.emit = anyrc::driver::EmitKind::Hir;
    opts.src_dir = Some(String::from(path::parent(file_path)));

    match anyrc::driver::compile(text, file_path, &opts) {
        Ok(_) => Vec::new(),
        Err(errors) => {
            let sm =
                anyrc::diagnostics::SourceMap::new(String::from(file_path), String::from(text));
            let mut out = Vec::new();
            for err in errors {
                let (line, col) = sm.line_col(err.span);
                let severity = match err.level {
                    anyrc::diagnostics::Level::Error => Severity::Error,
                    anyrc::diagnostics::Level::Warning => Severity::Warning,
                    anyrc::diagnostics::Level::Note => Severity::Info,
                };
                out.push(Diagnostic {
                    severity,
                    file_path: String::from(file_path),
                    line,
                    column: col,
                    end_line: line,
                    end_column: col.saturating_add(err.span.len().max(1)),
                    message: err.message,
                    code: None,
                    source: String::from("anyrc"),
                });
            }
            out
        }
    }
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
        LanguageId::Rust => {
            if !ctx.config.rust_check_on_save || ctx.config.ccargo_path.is_empty() {
                return None;
            }
            Some(CheckCommand {
                command: ctx.config.ccargo_path.clone(),
                args: String::from("check"),
                working_dir: project_root_or_parent(file_path, ctx.project),
                label: String::from("ccargo check"),
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
        LanguageId::C | LanguageId::Cpp | LanguageId::JavaScript | LanguageId::TypeScript => None,
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
