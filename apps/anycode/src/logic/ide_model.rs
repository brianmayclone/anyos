use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::diagnostics::Severity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl Position {
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub const fn new(start_line: u32, start_column: u32, end_line: u32, end_column: u32) -> Self {
        Self {
            start: Position::new(start_line, start_column),
            end: Position::new(end_line, end_column),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextEdit {
    pub file_path: String,
    pub range: Range,
    pub replacement: String,
}

#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    pub source: String,
    pub edits: Vec<TextEdit>,
    pub requires_confirmation: bool,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub file_path: String,
    pub version: u32,
    pub dirty: bool,
    pub language_id: String,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: String,
    pub name: String,
    pub rust: RustWorkspace,
}

#[derive(Clone, Debug)]
pub struct RustWorkspace {
    pub manifest_path: String,
    pub is_cargo_workspace: bool,
    pub package_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    pub fn as_cargo_arg(&self) -> &'static str {
        match self {
            Self::Debug => "",
            Self::Release => "--release",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BuildTarget {
    pub id: String,
    pub display_name: String,
    pub package: Option<String>,
    pub target: Option<String>,
    pub profile: BuildProfile,
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub id: String,
    pub display_name: String,
    pub command: String,
    pub args: String,
    pub working_dir: String,
    pub build_before_run: bool,
}

#[derive(Clone, Debug)]
pub struct TestCase {
    pub id: String,
    pub display_name: String,
    pub file_path: String,
    pub range: Range,
}

#[derive(Clone, Debug)]
pub struct IdeDiagnostic {
    pub severity: Severity,
    pub file_path: String,
    pub range: Range,
    pub message: String,
    pub code: Option<String>,
    pub source: String,
    pub actions: Vec<CodeAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugState {
    Detached,
    Launching,
    Running,
    Paused,
    Exited,
}

#[derive(Clone, Debug)]
pub struct DebugSessionModel {
    pub target_tid: u32,
    pub state: DebugState,
    pub launch_config: Option<RunConfig>,
}
