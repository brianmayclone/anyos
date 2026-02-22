use alloc::string::String;
use crate::logic::config::Config;
use crate::logic::project::BuildType;

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

/// Get the build command for the detected build system.
pub fn build_command(bt: BuildType, config: &Config) -> (String, String) {
    match bt {
        BuildType::Make => (config.make_path.clone(), String::new()),
        BuildType::SingleFile => (config.cc_path.clone(), String::from("main.c -o main")),
    }
}

/// Get the run command for the detected build system.
pub fn run_command(bt: BuildType, config: &Config) -> (String, String) {
    match bt {
        BuildType::Make => (config.make_path.clone(), String::from("run")),
        BuildType::SingleFile => (String::from("./main"), String::new()),
    }
}
