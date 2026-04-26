use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::ide_model::{Range, TestCase};
use crate::logic::project::{Project, ProjectType};
use crate::util::path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TestStatus {
    NotRun,
    Passed,
    Failed,
}

impl TestStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotRun => "not run",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestProject {
    pub name: String,
    pub root: String,
    pub cases: Vec<TestCase>,
}

#[derive(Clone, Debug)]
pub struct TestRunSummary {
    pub exit_code: u32,
    pub passed: usize,
    pub failed: usize,
    pub output_excerpt: String,
}

pub struct TestExplorerState {
    pub projects: Vec<TestProject>,
    pub history: Vec<TestRunSummary>,
    pub last_status: TestStatus,
}

impl TestExplorerState {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            history: Vec::new(),
            last_status: TestStatus::NotRun,
        }
    }

    pub fn refresh_from_project(&mut self, project: &Project) {
        self.projects.clear();
        match project.project_type {
            ProjectType::Cargo => {
                self.projects.push(TestProject {
                    name: project.name.clone(),
                    root: project.root.clone(),
                    cases: discover_tests(&project.root),
                });
                for member in &project.workspace_members {
                    let root = format!("{}/{}", project.root, member.path);
                    self.projects.push(TestProject {
                        name: member.name.clone(),
                        root: root.clone(),
                        cases: discover_tests(&root),
                    });
                }
            }
            ProjectType::RustFolder => {
                for cargo_project in &project.cargo_projects {
                    self.projects.push(TestProject {
                        name: cargo_project.name.clone(),
                        root: cargo_project.root.clone(),
                        cases: discover_tests(&cargo_project.root),
                    });
                }
            }
            _ => {}
        }
    }

    pub fn total_tests(&self) -> usize {
        self.projects
            .iter()
            .map(|project| project.cases.len())
            .sum::<usize>()
    }

    pub fn failed_count(&self) -> usize {
        self.history.last().map(|run| run.failed).unwrap_or(0)
    }

    pub fn record_run(&mut self, exit_code: u32, output: &str) {
        let failed = count_failed_tests(output);
        let passed = count_passed_tests(output);
        self.last_status = if exit_code == 0 && failed == 0 {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };
        self.history.push(TestRunSummary {
            exit_code,
            passed,
            failed,
            output_excerpt: output_excerpt(output),
        });
        if self.history.len() > 12 {
            self.history.remove(0);
        }
    }
}

pub fn discover_tests(root: &str) -> Vec<TestCase> {
    let mut tests = Vec::new();
    scan_tests_dir(root, root, 0, &mut tests);
    tests
}

fn scan_tests_dir(root: &str, dir: &str, depth: u32, tests: &mut Vec<TestCase>) {
    if depth > 10 || tests.len() > 1024 {
        return;
    }
    let Ok(entries) = anyos_std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let full = path::join(dir, &entry.name);
        if entry.is_dir() {
            if is_ignored_dir(&entry.name) {
                continue;
            }
            scan_tests_dir(root, &full, depth.saturating_add(1), tests);
        } else if path::extension(&entry.name) == Some("rs") {
            scan_tests_file(root, &full, tests);
        }
    }
}

fn scan_tests_file(root: &str, file_path: &str, tests: &mut Vec<TestCase>) {
    let Ok(text) = anyos_std::fs::read_to_string(file_path) else {
        return;
    };
    let module = module_name(root, file_path);
    let mut saw_test_attr = false;
    for (idx, line) in text.split('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[test]")
            || trimmed.starts_with("#[anyrc::test]")
            || trimmed.starts_with("#[tokio::test]")
        {
            saw_test_attr = true;
            continue;
        }
        if saw_test_attr {
            if let Some(name) = extract_test_fn_name(trimmed) {
                let display_name = if module.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", module, name)
                };
                tests.push(TestCase {
                    id: format!("{}:{}", file_path, name),
                    display_name,
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
        .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(String::from(&rest[..end]))
    }
}

fn module_name(root: &str, file_path: &str) -> String {
    let prefix = format!("{}/", root);
    let rel = file_path.strip_prefix(&prefix).unwrap_or(file_path);
    let rel = rel.strip_suffix(".rs").unwrap_or(rel);
    let rel = rel.strip_prefix("src/").unwrap_or(rel);
    if rel == "lib" || rel == "main" {
        return String::new();
    }
    let rel = rel.strip_prefix("tests/").unwrap_or(rel);
    rel.replace('/', "::")
}

fn count_failed_tests(output: &str) -> usize {
    let mut count = 0;
    for line in output.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("test ") && trimmed.ends_with("FAILED") {
            count += 1;
        }
    }
    count
}

fn count_passed_tests(output: &str) -> usize {
    let mut count = 0;
    for line in output.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("test ") && trimmed.ends_with("ok") {
            count += 1;
        }
    }
    count
}

fn output_excerpt(output: &str) -> String {
    let mut out = String::new();
    for line in output.split('\n').rev().take(12).collect::<Vec<&str>>().iter().rev() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "target" | ".git" | "build" | "node_modules" | ".cache" | ".idea"
    )
}
