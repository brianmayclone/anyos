use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::debug::DebugRegs;
use anyos_std::fmt::hex64;

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

#[derive(Clone, Debug)]
pub struct StackFrame {
    pub function: String,
    pub file_path: String,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct DebugVariable {
    pub name: String,
    pub type_name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct RegisterValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct DisassemblyLine {
    pub address: String,
    pub bytes: String,
    pub text: String,
    pub current: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryRow {
    pub address: String,
    pub bytes: String,
    pub ascii: String,
}

pub struct DebugSession {
    pub status: DebugSessionStatus,
    pub target_tid: u32,
    pub launch_name: String,
    pub target: String,
    pub breakpoints: Vec<Breakpoint>,
    pub call_stack: Vec<StackFrame>,
    pub variables: Vec<DebugVariable>,
    pub registers: Vec<RegisterValue>,
    pub disassembly: Vec<DisassemblyLine>,
    pub memory_rows: Vec<MemoryRow>,
    pub paused_reason: String,
}

impl DebugSession {
    pub fn new() -> Self {
        Self {
            status: DebugSessionStatus::Idle,
            target_tid: 0,
            launch_name: String::new(),
            target: String::new(),
            breakpoints: Vec::new(),
            call_stack: Vec::new(),
            variables: Vec::new(),
            registers: Vec::new(),
            disassembly: Vec::new(),
            memory_rows: Vec::new(),
            paused_reason: String::new(),
        }
    }

    pub fn start_launch(&mut self, task: &Task) {
        self.status = DebugSessionStatus::Launching;
        self.target_tid = 0;
        self.launch_name = task.name.clone();
        self.target = if task.args.is_empty() {
            task.command.clone()
        } else {
            format!("{} {}", task.command, task.args)
        };
        self.paused_reason.clear();
        self.refresh_preview_state();
    }

    pub fn mark_running(&mut self) {
        self.status = DebugSessionStatus::Running;
        self.paused_reason.clear();
        if self.registers.is_empty() {
            self.refresh_preview_state();
        }
    }

    pub fn mark_attached(&mut self, tid: u32, regs: &DebugRegs) {
        self.target_tid = tid;
        self.status = DebugSessionStatus::Stopped;
        self.paused_reason = String::from("attached");
        self.apply_registers(regs);
    }

    pub fn pause(&mut self, reason: &str) {
        if self.status == DebugSessionStatus::Running {
            self.status = DebugSessionStatus::Stopped;
            self.paused_reason = String::from(reason);
            if self.registers.is_empty() {
                self.refresh_preview_state();
            }
        }
    }

    pub fn continue_execution(&mut self) {
        if self.status == DebugSessionStatus::Stopped {
            self.status = DebugSessionStatus::Running;
            self.paused_reason.clear();
        }
    }

    pub fn step_started(&mut self) {
        if self.status == DebugSessionStatus::Stopped {
            self.status = DebugSessionStatus::Running;
            self.paused_reason = String::from("step");
        }
    }

    pub fn stop(&mut self) {
        self.status = DebugSessionStatus::Stopped;
        self.target_tid = 0;
        self.launch_name.clear();
        self.target.clear();
        self.paused_reason = String::from("terminated");
        self.call_stack.clear();
        self.variables.clear();
        self.registers.clear();
        self.disassembly.clear();
        self.memory_rows.clear();
    }

    pub fn reset(&mut self) {
        self.status = DebugSessionStatus::Idle;
        self.target_tid = 0;
        self.launch_name.clear();
        self.target.clear();
        self.breakpoints.clear();
        self.call_stack.clear();
        self.variables.clear();
        self.registers.clear();
        self.disassembly.clear();
        self.memory_rows.clear();
        self.paused_reason.clear();
    }

    pub fn toggle_breakpoint(&mut self, file_path: &str, line: u32) -> bool {
        if let Some(idx) = self
            .breakpoints
            .iter()
            .position(|bp| bp.file_path == file_path && bp.line == line)
        {
            self.breakpoints.remove(idx);
            self.refresh_preview_state();
            false
        } else {
            self.breakpoints.push(Breakpoint {
                file_path: String::from(file_path),
                line,
                enabled: true,
            });
            self.refresh_preview_state();
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

    pub fn apply_registers(&mut self, regs: &DebugRegs) {
        self.call_stack.clear();
        self.variables.clear();
        self.registers.clear();

        self.call_stack.push(StackFrame {
            function: String::from("rip"),
            file_path: hex64(regs.rip),
            line: 0,
        });

        self.variables.push(DebugVariable {
            name: String::from("tid"),
            type_name: String::from("u32"),
            value: format!("{}", self.target_tid),
        });
        self.variables.push(DebugVariable {
            name: String::from("rip"),
            type_name: String::from("u64"),
            value: hex64(regs.rip),
        });
        self.variables.push(DebugVariable {
            name: String::from("rsp"),
            type_name: String::from("u64"),
            value: hex64(regs.rsp),
        });

        self.push_register("RAX", regs.rax);
        self.push_register("RBX", regs.rbx);
        self.push_register("RCX", regs.rcx);
        self.push_register("RDX", regs.rdx);
        self.push_register("RSI", regs.rsi);
        self.push_register("RDI", regs.rdi);
        self.push_register("RBP", regs.rbp);
        self.push_register("R8", regs.r8);
        self.push_register("R9", regs.r9);
        self.push_register("R10", regs.r10);
        self.push_register("R11", regs.r11);
        self.push_register("R12", regs.r12);
        self.push_register("R13", regs.r13);
        self.push_register("R14", regs.r14);
        self.push_register("R15", regs.r15);
        self.push_register("RSP", regs.rsp);
        self.push_register("RIP", regs.rip);
        self.push_register("RFLAGS", regs.rflags);
        self.push_register("CR3", regs.cr3);
    }

    fn push_register(&mut self, name: &str, value: u64) {
        self.registers.push(RegisterValue {
            name: String::from(name),
            value: hex64(value),
        });
    }

    fn refresh_preview_state(&mut self) {
        self.call_stack.clear();
        self.variables.clear();
        self.registers.clear();

        if let Some(bp) = self.breakpoints.iter().find(|bp| bp.enabled) {
            self.call_stack.push(StackFrame {
                function: String::from("main"),
                file_path: bp.file_path.clone(),
                line: bp.line,
            });
        } else if !self.target.is_empty() {
            self.call_stack.push(StackFrame {
                function: self.launch_name.clone(),
                file_path: self.target.clone(),
                line: 0,
            });
        }

        self.variables.push(DebugVariable {
            name: String::from("argc"),
            type_name: String::from("usize"),
            value: String::from("pending"),
        });
        self.variables.push(DebugVariable {
            name: String::from("status"),
            type_name: String::from("&str"),
            value: self.status_label(),
        });

        self.registers.push(RegisterValue {
            name: String::from("rip"),
            value: String::from("pending backend"),
        });
        self.registers.push(RegisterValue {
            name: String::from("rsp"),
            value: String::from("pending backend"),
        });
        self.registers.push(RegisterValue {
            name: String::from("rax"),
            value: String::from("pending backend"),
        });
    }
}
