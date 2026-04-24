use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::tasks::Task;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DebugSessionStatus {
    Idle,
    Launching,
    Running,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct Breakpoint {
    pub file_path: String,
    pub line: u32,
    pub enabled: bool,
}

pub struct DebugSession {
    pub status: DebugSessionStatus,
    pub launch_name: String,
    pub target: String,
    pub breakpoints: Vec<Breakpoint>,
}

impl DebugSession {
    pub fn new() -> Self {
        Self {
            status: DebugSessionStatus::Idle,
            launch_name: String::new(),
            target: String::new(),
            breakpoints: Vec::new(),
        }
    }

    pub fn start_launch(&mut self, task: &Task) {
        self.status = DebugSessionStatus::Launching;
        self.launch_name = task.name.clone();
        self.target = if task.args.is_empty() {
            task.command.clone()
        } else {
            format!("{} {}", task.command, task.args)
        };
    }

    pub fn mark_running(&mut self) {
        self.status = DebugSessionStatus::Running;
    }

    pub fn stop(&mut self) {
        self.status = DebugSessionStatus::Stopped;
        self.launch_name.clear();
        self.target.clear();
    }

    pub fn reset(&mut self) {
        self.status = DebugSessionStatus::Idle;
        self.launch_name.clear();
        self.target.clear();
        self.breakpoints.clear();
    }

    pub fn toggle_breakpoint(&mut self, file_path: &str, line: u32) -> bool {
        if let Some(idx) = self
            .breakpoints
            .iter()
            .position(|bp| bp.file_path == file_path && bp.line == line)
        {
            self.breakpoints.remove(idx);
            false
        } else {
            self.breakpoints.push(Breakpoint {
                file_path: String::from(file_path),
                line,
                enabled: true,
            });
            true
        }
    }

    pub fn breakpoint_count(&self) -> usize {
        self.breakpoints.iter().filter(|bp| bp.enabled).count()
    }

    pub fn status_label(&self) -> String {
        match self.status {
            DebugSessionStatus::Idle => String::from("Debugger: Idle"),
            DebugSessionStatus::Launching => format!("Debugger: Launching {}", self.launch_name),
            DebugSessionStatus::Running => format!("Debugger: Running {}", self.launch_name),
            DebugSessionStatus::Stopped => String::from("Debugger: Stopped"),
        }
    }
}
