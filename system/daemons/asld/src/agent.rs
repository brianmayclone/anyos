use alloc::format;
use alloc::string::String;

use crate::errors::AsldError;
use crate::model::{AgentPolicy, AgentState, DistroState, SessionMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellIo {
    pub console_pipe_name: String,
    pub stdin_pipe_name: String,
    pub attached_pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecIo {
    pub stdout_pipe_name: String,
    pub stdin_pipe_name: String,
    pub attached_pid: u32,
}

pub fn inferred_agent_state(policy: &AgentPolicy, distro_state: DistroState) -> AgentState {
    if !policy.enabled {
        return AgentState::NotPresent;
    }
    match distro_state {
        DistroState::Ready => AgentState::Disconnected,
        DistroState::Degraded => AgentState::Degraded,
        DistroState::Stopped | DistroState::Created => AgentState::NotPresent,
        _ => AgentState::Starting,
    }
}

pub fn provision_shell_io(session_id: &str, mode: SessionMode, console_pipe_name: &str) -> Result<ShellIo, AsldError> {
    let stdin_pipe_name = format!("asl-shell-stdin-{}", session_id);
    ensure_pipe(&stdin_pipe_name)?;

    let output_pipe_name = match mode {
        SessionMode::Agent => {
            let pipe_name = format!("asl-agent-shell-{}", session_id);
            ensure_pipe(&pipe_name)?;
            pipe_name
        }
        SessionMode::FallbackConsole => {
            ensure_pipe(console_pipe_name)?;
            String::from(console_pipe_name)
        }
    };

    Ok(ShellIo {
        console_pipe_name: output_pipe_name,
        stdin_pipe_name,
        attached_pid: 0,
    })
}

pub fn provision_exec_io(exec_id: &str, mode: SessionMode) -> Result<ExecIo, AsldError> {
    let stdout_pipe_name = match mode {
        SessionMode::Agent => format!("asl-agent-exec-stdout-{}", exec_id),
        SessionMode::FallbackConsole => format!("asl-console-exec-stdout-{}", exec_id),
    };
    let stdin_pipe_name = format!("asl-exec-stdin-{}", exec_id);

    ensure_pipe(&stdout_pipe_name)?;
    ensure_pipe(&stdin_pipe_name)?;

    Ok(ExecIo {
        stdout_pipe_name,
        stdin_pipe_name,
        attached_pid: 0,
    })
}

fn ensure_pipe(pipe_name: &str) -> Result<(), AsldError> {
    let existing = anyos_std::ipc::pipe_open(pipe_name);
    if existing != 0 && existing != u32::MAX {
        let _ = anyos_std::ipc::pipe_close(existing);
    }
    let created = anyos_std::ipc::pipe_create(pipe_name);
    if created == 0 || created == u32::MAX {
        return Err(AsldError::BackendUnavailable("failed to create agent or console pipe"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::model::{AgentPolicy, DistroState, SessionMode};

    use super::{inferred_agent_state, provision_exec_io, provision_shell_io};

    #[test]
    fn disabled_policy_means_not_present() {
        let mut policy = AgentPolicy::default();
        policy.enabled = false;
        assert_eq!(
            inferred_agent_state(&policy, DistroState::Ready).as_str(),
            "not_present"
        );
    }

    #[test]
    fn fallback_console_shell_uses_console_pipe() {
        let io = provision_shell_io("sh-00000001", SessionMode::FallbackConsole, "asl-console-ubuntu").unwrap();
        assert_eq!(io.console_pipe_name, "asl-console-ubuntu");
        assert!(io.stdin_pipe_name.contains("sh-00000001"));
    }

    #[test]
    fn exec_provisions_dedicated_stdout_and_stdin_pipes() {
        let io = provision_exec_io("exec-00000001", SessionMode::Agent).unwrap();
        assert!(io.stdout_pipe_name.contains("exec-00000001"));
        assert!(io.stdin_pipe_name.contains("exec-00000001"));
    }
}
