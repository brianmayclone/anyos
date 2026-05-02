use crate::logic::config::Config;
use crate::logic::project::BuildType;
use crate::logic::tasks::Task;
use crate::util::path;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A running build/run process with pipe output capture.
pub struct BuildProcess {
    pub tid: u32,
    pub pipe_id: u32,
    pub finished: bool,
}

impl BuildProcess {
    /// Spawn a build or run command with stdout piped.
    pub fn spawn(cmd: &str, args: &str) -> Option<Self> {
        let pipe_id = anyos_std::ipc::pipe_create("anycode:build");
        if pipe_id == 0 {
            return None;
        }
        let full_args = full_argv_string(cmd, args);
        let tid = anyos_std::process::spawn_piped(cmd, &full_args, pipe_id);
        if tid == u32::MAX {
            anyos_std::ipc::pipe_close(pipe_id);
            return None;
        }
        Some(Self {
            tid,
            pipe_id,
            finished: false,
        })
    }

    /// Spawn a task from the task manager.
    pub fn spawn_task(task: &Task) -> Option<Self> {
        if !task.working_dir.is_empty() {
            anyos_std::fs::chdir(&task.working_dir);
        }
        Self::spawn(&task.command, &task.args)
    }

    /// Poll for new output from the pipe. Returns any available data.
    pub fn poll_output(&mut self, buf: &mut [u8]) -> Option<usize> {
        if self.finished {
            return None;
        }
        let n = anyos_std::ipc::pipe_read(self.pipe_id, buf);
        if n == 0 || n == u32::MAX {
            return None;
        }
        Some(n as usize)
    }

    /// Check if the process has finished. Returns Some(exit_code) if done.
    pub fn check_finished(&mut self) -> Option<u32> {
        if self.finished {
            return Some(0);
        }
        let status = anyos_std::process::try_waitpid(self.tid);
        if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
            self.finished = true;
            anyos_std::ipc::pipe_close(self.pipe_id);
            Some(status)
        } else {
            None
        }
    }

    /// Kill the running process.
    pub fn kill(&mut self) {
        if !self.finished {
            anyos_std::process::kill(self.tid);
            self.finished = true;
            anyos_std::ipc::pipe_close(self.pipe_id);
        }
    }
}

fn full_argv_string(cmd: &str, args: &str) -> String {
    let argv0 = path::basename(cmd);
    if args.trim().is_empty() {
        String::from(argv0)
    } else {
        format!("{} {}", argv0, args)
    }
}

/// A single build rule: pattern → command.
///
/// Pattern syntax:
///   `Makefile**` — file named "Makefile" exists in the project (glob)
///   `*.c` — active file has .c extension
///   `*.rs` — active file has .rs extension
struct BuildRule {
    pattern: String,
    build_cmd: String,
    run_cmd: String,
}

/// Build rule set loaded from build.conf.
pub struct BuildRules {
    rules: Vec<BuildRule>,
}

impl BuildRules {
    /// Load rules from the build.conf file in the app bundle.
    pub fn load(conf_path: &str) -> Self {
        let mut rules = Vec::new();
        if let Ok(data) = anyos_std::fs::read_to_string(conf_path) {
            for line in data.split('\n') {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let parts: Vec<&str> = line.splitn(3, ':').collect();
                if parts.len() >= 2 {
                    rules.push(BuildRule {
                        pattern: String::from(parts[0]),
                        build_cmd: String::from(parts[1]),
                        run_cmd: if parts.len() > 2 {
                            String::from(parts[2])
                        } else {
                            String::new()
                        },
                    });
                }
            }
        }
        Self { rules }
    }

    /// Find a matching build rule for the given active file.
    fn find_match(&self, active_file: &str, project_root: &str) -> Option<&BuildRule> {
        let filename = path::basename(active_file);
        let ext = path::extension(active_file).unwrap_or("");

        for rule in &self.rules {
            if rule.pattern.ends_with("**") {
                let name = &rule.pattern[..rule.pattern.len() - 2];
                let check_path = path::join(project_root, name);
                if path::exists(&check_path) {
                    return Some(rule);
                }
            } else if rule.pattern.starts_with("*.") {
                let pat_ext = &rule.pattern[2..];
                if ext == pat_ext {
                    return Some(rule);
                }
            } else if filename == rule.pattern {
                return Some(rule);
            }
        }
        None
    }

    /// Expand variables in a command template.
    fn expand_cmd(template: &str, active_file: &str) -> String {
        let filename = path::basename(active_file);
        let out_name = match filename.rfind('.') {
            Some(i) if i > 0 => &filename[..i],
            _ => filename,
        };
        let mut result = String::from(template);
        if result.contains("$FILE") {
            result = String::from(result.replace("$FILE", active_file).as_str());
        }
        if result.contains("$OUT") {
            result = String::from(result.replace("$OUT", out_name).as_str());
        }
        result
    }

    /// Get the build command based on rules and active file.
    pub fn build_command(
        &self,
        active_file: &str,
        project_root: &str,
        config: &Config,
    ) -> Option<(String, String)> {
        let rule = self.find_match(active_file, project_root)?;
        if rule.build_cmd.is_empty() {
            return None;
        }
        let expanded = Self::expand_cmd(&rule.build_cmd, active_file);
        let (cmd, args) = match expanded.find(' ') {
            Some(i) => (&expanded[..i], &expanded[i + 1..]),
            None => (expanded.as_str(), ""),
        };
        let cmd_path = resolve_tool(cmd, config);
        if cmd_path.is_empty() {
            return None;
        }
        Some((cmd_path, String::from(args)))
    }

    /// Get the run command based on rules and active file.
    pub fn run_command(
        &self,
        active_file: &str,
        project_root: &str,
        config: &Config,
    ) -> Option<(String, String)> {
        let rule = self.find_match(active_file, project_root)?;
        if rule.run_cmd.is_empty() {
            return None;
        }
        let expanded = Self::expand_cmd(&rule.run_cmd, active_file);
        let (cmd, args) = match expanded.find(' ') {
            Some(i) => (&expanded[..i], &expanded[i + 1..]),
            None => (expanded.as_str(), ""),
        };
        let cmd_path = resolve_tool(cmd, config);
        if cmd_path.is_empty() {
            return None;
        }
        Some((cmd_path, String::from(args)))
    }
}

/// Resolve a tool name to a full path using known config paths.
fn resolve_tool(name: &str, config: &Config) -> String {
    match name {
        "make" => config.make_path.clone(),
        "cc" | "gcc" | "clang" => config.cc_path.clone(),
        "c++" | "g++" | "clang++" => config.cxx_path.clone(),
        "crust" | "rustc" | "anyrc" => config.crust_path.clone(),
        "ccargo" | "cargo" | "acargo" => config.ccargo_path.clone(),
        "git" => config.git_path.clone(),
        "node" => config.node_path.clone(),
        "npm" => config.npm_path.clone(),
        "eslint" => config.eslint_path.clone(),
        _ => {
            if name.starts_with('/') || name.starts_with("./") {
                String::from(name)
            } else {
                crate::logic::config::find_tool(name)
            }
        }
    }
}

/// Legacy build_command fallback (when no build rules match).
pub fn build_command(bt: BuildType, config: &Config) -> (String, String) {
    match bt {
        BuildType::Make => (config.make_path.clone(), String::new()),
        BuildType::SingleFile => (config.cc_path.clone(), String::from("main.c -o main")),
    }
}

/// Legacy run_command fallback.
pub fn run_command(bt: BuildType, config: &Config) -> (String, String) {
    match bt {
        BuildType::Make => (config.make_path.clone(), String::from("run")),
        BuildType::SingleFile => (String::from("./main"), String::new()),
    }
}

// ════════════════════════════════════════════════════════════════
//  Prerequisite check — verify required tools are available
// ════════════════════════════════════════════════════════════════

/// Tool availability info for the splash screen check.
pub struct ToolStatus {
    pub name: &'static str,
    pub description: &'static str,
    pub path: String,
    pub available: bool,
}

/// Check which development tools are installed.
pub fn check_prerequisites() -> Vec<ToolStatus> {
    let mut results = Vec::new();
    let tools: [(&str, &str, &[&str]); 11] = [
        ("crust", "Rust Compiler", &["crust", "rustc"]),
        (
            "ccargo",
            "Cargo Build System",
            &["ccargo", "cargo", "acargo"],
        ),
        ("anyrc", "anyRC Compiler Library", &["anyrc"]),
        ("cc", "C Compiler", &["cc", "gcc", "clang"]),
        ("c++", "C++ Compiler", &["c++", "g++", "clang++"]),
        ("make", "Make Build Tool", &["make"]),
        ("git", "Git Version Control", &["git", "agit"]),
        ("node", "Node.js Runtime", &["node"]),
        ("npm", "NPM Package Manager", &["npm"]),
        ("eslint", "JavaScript Linter", &["eslint", "npx"]),
        ("nasm", "NASM Assembler", &["nasm"]),
    ];

    for (name, desc, aliases) in tools {
        let path = find_first_available_tool(aliases);
        let available = !path.is_empty();
        results.push(ToolStatus {
            name,
            description: desc,
            path,
            available,
        });
    }
    results
}

/// Check if the essential Rust-first tools (crust, ccargo, anyrc) are all available.
pub fn has_essential_tools(statuses: &[ToolStatus]) -> bool {
    let essential = ["crust", "ccargo", "anyrc"];
    essential
        .iter()
        .all(|name| statuses.iter().any(|s| s.name == *name && s.available))
}

fn find_first_available_tool(names: &[&str]) -> String {
    for name in names {
        let path = crate::logic::config::find_tool(name);
        if !path.is_empty() {
            return path;
        }
    }
    String::new()
}
