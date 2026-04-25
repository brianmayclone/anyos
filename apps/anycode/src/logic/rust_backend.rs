use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::config::Config;
use crate::logic::ide_model::{
    BuildProfile, BuildTarget, Range, RunConfig, RustWorkspace, TestCase, Workspace,
};
use crate::logic::project::{BuildConfiguration, Project, ProjectType, TargetKind};
use crate::logic::tasks::{Task, TaskCategory};
use crate::util::path;

pub const RUST_BACKEND_ID: &str = "rust.ccargo";

pub struct RustBuildBackend {
    pub ccargo_path: String,
    pub crust_path: String,
    pub anyrc_path: String,
}

impl RustBuildBackend {
    pub fn from_config(config: &Config) -> Self {
        Self {
            ccargo_path: config.ccargo_path.clone(),
            crust_path: config.crust_path.clone(),
            anyrc_path: config.crust_path.clone(),
        }
    }

    pub fn is_available(&self) -> bool {
        !self.ccargo_path.is_empty() || !self.crust_path.is_empty() || !self.anyrc_path.is_empty()
    }

    pub fn workspace_from_project(project: &Project) -> Option<Workspace> {
        if project.project_type != ProjectType::Cargo {
            return None;
        }
        let mut package_names = Vec::new();
        if !project.name.is_empty() {
            package_names.push(project.name.clone());
        }
        for member in &project.workspace_members {
            if !package_names.iter().any(|name| name == &member.name) {
                package_names.push(member.name.clone());
            }
        }
        Some(Workspace {
            root: project.root.clone(),
            name: project.name.clone(),
            rust: RustWorkspace {
                manifest_path: format!("{}/Cargo.toml", project.root),
                is_cargo_workspace: project.is_workspace,
                package_names,
            },
        })
    }

    pub fn discover_targets(project: &Project) -> Vec<BuildTarget> {
        let mut targets = Vec::new();
        for profile in [BuildProfile::Debug, BuildProfile::Release] {
            targets.push(BuildTarget {
                id: format!("{}:workspace:{:?}", RUST_BACKEND_ID, profile),
                display_name: match profile {
                    BuildProfile::Debug => String::from("Build workspace (debug)"),
                    BuildProfile::Release => String::from("Build workspace (release)"),
                },
                package: None,
                target: None,
                profile,
            });
        }
        for target in &project.cargo_targets {
            if matches!(
                target.kind,
                TargetKind::Binary | TargetKind::Example | TargetKind::Test
            ) {
                targets.push(BuildTarget {
                    id: format!("{}:target:{}", RUST_BACKEND_ID, target.name),
                    display_name: format!("Rust {}", target.name),
                    package: None,
                    target: Some(target.name.clone()),
                    profile: BuildProfile::Debug,
                });
            }
        }
        for member in &project.workspace_members {
            targets.push(BuildTarget {
                id: format!("{}:package:{}", RUST_BACKEND_ID, member.name),
                display_name: format!("Build package {}", member.name),
                package: Some(member.name.clone()),
                target: None,
                profile: BuildProfile::Debug,
            });
        }
        targets
    }

    pub fn task_for_check(&self, project: &Project) -> Option<Task> {
        if self.ccargo_path.is_empty() {
            return None;
        }
        let mut task = Task::new(
            "Rust Check",
            TaskCategory::Check,
            &self.ccargo_path,
            "check",
            &project.root,
        );
        task.display_label = String::from("ccargo check");
        Some(task)
    }

    pub fn task_for_build(&self, project: &Project, profile: BuildConfiguration) -> Option<Task> {
        if self.ccargo_path.is_empty() {
            return None;
        }
        let args = match profile {
            BuildConfiguration::Debug => "build",
            BuildConfiguration::Release => "build --release",
        };
        let mut task = Task::new(
            "Rust Build",
            TaskCategory::Build,
            &self.ccargo_path,
            args,
            &project.root,
        );
        task.display_label = format!("ccargo {}", args);
        Some(task)
    }

    pub fn run_configs(project: &Project, ccargo_path: &str) -> Vec<RunConfig> {
        let mut configs = Vec::new();
        if ccargo_path.is_empty() {
            return configs;
        }
        for target in &project.cargo_targets {
            let args = match target.kind {
                TargetKind::Binary => format!("run --bin {}", target.name),
                TargetKind::Example => format!("run --example {}", target.name),
                _ => continue,
            };
            configs.push(RunConfig {
                id: format!("{}:run:{}", RUST_BACKEND_ID, target.name),
                display_name: target.name.clone(),
                command: String::from(ccargo_path),
                args,
                working_dir: project.root.clone(),
                build_before_run: false,
            });
        }
        configs
    }

    pub fn discover_tests(root: &str) -> Vec<TestCase> {
        let mut tests = Vec::new();
        scan_tests_dir(root, 0, &mut tests);
        tests
    }
}

fn scan_tests_dir(dir: &str, depth: u32, tests: &mut Vec<TestCase>) {
    if depth > 10 || tests.len() > 512 {
        return;
    }
    let entries = match anyos_std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let full = path::join(dir, &entry.name);
        if entry.is_dir() {
            if matches!(
                entry.name.as_str(),
                "target" | ".git" | "build" | "node_modules"
            ) {
                continue;
            }
            scan_tests_dir(&full, depth + 1, tests);
        } else if path::extension(&entry.name) == Some("rs") {
            scan_tests_file(&full, tests);
        }
    }
}

fn scan_tests_file(file_path: &str, tests: &mut Vec<TestCase>) {
    let text = match anyos_std::fs::read_to_string(file_path) {
        Ok(text) => text,
        Err(_) => return,
    };
    let mut saw_test_attr = false;
    for (idx, line) in text.split('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[test]") || trimmed.starts_with("#[anyrc::test]") {
            saw_test_attr = true;
            continue;
        }
        if saw_test_attr {
            if let Some(name) = extract_test_fn_name(trimmed) {
                tests.push(TestCase {
                    id: format!("{}:{}", file_path, name),
                    display_name: name,
                    file_path: String::from(file_path),
                    range: Range::new((idx + 1) as u32, 1, (idx + 1) as u32, 1),
                });
            }
            saw_test_attr = false;
        }
    }
}

fn extract_test_fn_name(line: &str) -> Option<String> {
    let fn_pos = line.find("fn ")?;
    let rest = &line[fn_pos + 3..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(String::from(&rest[..end]))
    }
}
