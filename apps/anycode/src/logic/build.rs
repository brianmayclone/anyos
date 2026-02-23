use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::logic::config::Config;
use crate::logic::project::BuildType;
use crate::util::path;

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
        let tid = anyos_std::process::spawn_piped(cmd, args, pipe_id);
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
///
/// File format (one rule per line):
///   pattern:build_command:run_command
///
/// Examples:
///   Makefile**:make:make run
///   *.c:cc $FILE -o $OUT:$OUT
///   *.rs:rustc $FILE -o $OUT:$OUT
///   *.py::python $FILE
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
                // Format: pattern:build_cmd:run_cmd
                let parts: Vec<&str> = line.splitn(3, ':').collect();
                if parts.len() >= 2 {
                    rules.push(BuildRule {
                        pattern: String::from(parts[0]),
                        build_cmd: String::from(parts[1]),
                        run_cmd: if parts.len() > 2 { String::from(parts[2]) } else { String::new() },
                    });
                }
            }
        }
        Self { rules }
    }

    /// Find a matching build rule for the given active file.
    /// `project_root` is used for Makefile** patterns.
    fn find_match(&self, active_file: &str, project_root: &str) -> Option<&BuildRule> {
        let filename = path::basename(active_file);
        let ext = path::extension(active_file).unwrap_or("");

        for rule in &self.rules {
            if rule.pattern.ends_with("**") {
                // Glob pattern: check if file exists in project root
                let name = &rule.pattern[..rule.pattern.len() - 2];
                let check_path = path::join(project_root, name);
                if path::exists(&check_path) {
                    return Some(rule);
                }
            } else if rule.pattern.starts_with("*.") {
                // Extension match
                let pat_ext = &rule.pattern[2..];
                if ext == pat_ext {
                    return Some(rule);
                }
            } else {
                // Exact filename match
                if filename == rule.pattern {
                    return Some(rule);
                }
            }
        }
        None
    }

    /// Expand variables in a command template.
    /// $FILE → active file path, $OUT → output name (filename without ext)
    fn expand_cmd(template: &str, active_file: &str) -> String {
        let filename = path::basename(active_file);
        let out_name = match filename.rfind('.') {
            Some(i) if i > 0 => &filename[..i],
            _ => filename,
        };
        let mut result = String::from(template);
        // Simple string replacement
        if result.contains("$FILE") {
            result = String::from(result.replace("$FILE", active_file).as_str());
        }
        if result.contains("$OUT") {
            result = String::from(result.replace("$OUT", out_name).as_str());
        }
        result
    }

    /// Get the build command based on rules and active file.
    /// Returns (command, args) or None if no rule matches.
    pub fn build_command(&self, active_file: &str, project_root: &str, config: &Config) -> Option<(String, String)> {
        let rule = self.find_match(active_file, project_root)?;
        if rule.build_cmd.is_empty() {
            return None;
        }
        let expanded = Self::expand_cmd(&rule.build_cmd, active_file);
        // Split into command + args at first space
        let (cmd, args) = match expanded.find(' ') {
            Some(i) => (&expanded[..i], &expanded[i + 1..]),
            None => (expanded.as_str(), ""),
        };
        // Resolve command path
        let cmd_path = resolve_tool(cmd, config);
        Some((cmd_path, String::from(args)))
    }

    /// Get the run command based on rules and active file.
    pub fn run_command(&self, active_file: &str, project_root: &str, config: &Config) -> Option<(String, String)> {
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
        Some((cmd_path, String::from(args)))
    }
}

/// Resolve a tool name to a full path using known config paths.
fn resolve_tool(name: &str, config: &Config) -> String {
    match name {
        "make" => config.make_path.clone(),
        "cc" | "gcc" => config.cc_path.clone(),
        "git" => config.git_path.clone(),
        _ => {
            // If it starts with / or ./, it's already a path
            if name.starts_with('/') || name.starts_with("./") {
                String::from(name)
            } else {
                // Try to find in PATH
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
