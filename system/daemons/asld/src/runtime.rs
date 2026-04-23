use alloc::vec::Vec;
use alloc::{format, string::String};

use crate::agent::{inferred_agent_state, provision_exec_io, provision_shell_io};
use crate::config::{
    add_mount, add_port_forward, ensure_distro_tree, list_distros, load_distro, remove_mount,
    remove_port_forward, ConfigStore,
};
use crate::distro::build_distro_config;
use crate::errors::AsldError;
use crate::model::{
    DistroHealth, DistroState, DistroStatus, ExecInvocation, MountSpec, PortForwardSpec,
    SessionMode, ShellSession,
};
use crate::status::{degraded_status, stopped_status};
use crate::store::RuntimeStore;
use crate::vm;

#[derive(Clone)]
struct RuntimeBackend {
    name: String,
    vm: vm::VmInstance,
    shell_sessions: Vec<ShellSession>,
    execs: Vec<ExecInvocation>,
}

pub struct RuntimeService {
    store: RuntimeStore,
    backends: Vec<RuntimeBackend>,
    next_shell_seq: u32,
    next_exec_seq: u32,
}

impl RuntimeService {
    pub fn new() -> Self {
        Self {
            store: RuntimeStore::new(),
            backends: Vec::new(),
            next_shell_seq: 1,
            next_exec_seq: 1,
        }
    }

    pub fn list<S: ConfigStore>(&mut self, store: &mut S) -> Result<Vec<DistroStatus>, AsldError> {
        let mut out = Vec::new();
        for name in list_distros(store)? {
            if let Some(status) = self.store.get(&name) {
                out.push(status.clone());
                continue;
            }
            let cfg = load_distro(store, &name)?;
            out.push(stopped_status(&cfg.name, cfg.resources, cfg.network));
        }
        Ok(out)
    }

    pub fn status<S: ConfigStore>(&mut self, store: &mut S, name: &str) -> Result<DistroStatus, AsldError> {
        if let Some(status) = self.store.get(name) {
            return Ok(status.clone());
        }
        let cfg = load_distro(store, name)?;
        Ok(stopped_status(&cfg.name, cfg.resources, cfg.network))
    }

    pub fn create<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        image_ref: &str,
        owner: &str,
    ) -> Result<DistroStatus, AsldError> {
        let cfg = build_distro_config(name, image_ref, owner)?;
        ensure_distro_tree(store, &cfg)?;
        let status = stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
        self.store.upsert(status.clone());
        Ok(status)
    }

    pub fn start<S: ConfigStore>(&mut self, store: &mut S, name: &str) -> Result<DistroStatus, AsldError> {
        let cfg = load_distro(store, name)?;
        if let Some(existing) = self.store.get(name) {
            if matches!(existing.state, DistroState::Ready | DistroState::Starting | DistroState::Booting) {
                return Err(AsldError::InvalidState("already running or starting"));
            }
        }

        match vm::start_vm(&cfg) {
            Ok(vm_instance) => {
                let mut status = stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
                status.state = DistroState::Ready;
                status.health = DistroHealth::Ready;
                status.agent_state = inferred_agent_state(&cfg.agent, DistroState::Ready);
                self.upsert_backend(&cfg.name, vm_instance);
                self.store.upsert(status.clone());
                Ok(status)
            }
            Err(err) => {
                let status = degraded_status(
                    &cfg.name,
                    cfg.resources.clone(),
                    cfg.network.clone(),
                    &err.message(),
                );
                self.store.upsert(status.clone());
                Ok(status)
            }
        }
    }

    pub fn stop<S: ConfigStore>(&mut self, store: &mut S, name: &str) -> Result<DistroStatus, AsldError> {
        let cfg = load_distro(store, name)?;
        if let Some(index) = self.backends.iter().position(|backend| backend.name == name) {
            let instance = self.backends[index].vm.clone();
            vm::stop_vm(&instance)?;
            self.backends.remove(index);
        }
        let status = stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
        self.store.upsert(status.clone());
        Ok(status)
    }

    pub fn list_mounts<S: ConfigStore>(&mut self, store: &mut S, name: &str) -> Result<Vec<MountSpec>, AsldError> {
        Ok(load_distro(store, name)?.mounts)
    }

    pub fn show_mount<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        mount_id: &str,
    ) -> Result<MountSpec, AsldError> {
        load_distro(store, name)?
            .mounts
            .into_iter()
            .find(|mount| mount.id == mount_id)
            .ok_or(AsldError::NotFound)
    }

    pub fn add_mount<S: ConfigStore>(
        &mut self,
        store: &mut S,
        distro_name: &str,
        mount: &MountSpec,
    ) -> Result<Vec<MountSpec>, AsldError> {
        add_mount(store, distro_name, mount)?;
        self.list_mounts(store, distro_name)
    }

    pub fn remove_mount<S: ConfigStore>(
        &mut self,
        store: &mut S,
        distro_name: &str,
        mount_id: &str,
    ) -> Result<Vec<MountSpec>, AsldError> {
        let _ = self.show_mount(store, distro_name, mount_id)?;
        remove_mount(store, distro_name, mount_id)?;
        self.list_mounts(store, distro_name)
    }

    pub fn list_port_forwards<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<PortForwardSpec>, AsldError> {
        Ok(load_distro(store, name)?.port_forwards)
    }

    pub fn add_port_forward<S: ConfigStore>(
        &mut self,
        store: &mut S,
        distro_name: &str,
        rule: &PortForwardSpec,
    ) -> Result<Vec<PortForwardSpec>, AsldError> {
        add_port_forward(store, distro_name, rule)?;
        self.list_port_forwards(store, distro_name)
    }

    pub fn remove_port_forward<S: ConfigStore>(
        &mut self,
        store: &mut S,
        distro_name: &str,
        rule_id: &str,
    ) -> Result<Vec<PortForwardSpec>, AsldError> {
        let exists = self
            .list_port_forwards(store, distro_name)?
            .into_iter()
            .any(|rule| rule.id == rule_id);
        if !exists {
            return Err(AsldError::NotFound);
        }
        remove_port_forward(store, distro_name, rule_id)?;
        self.list_port_forwards(store, distro_name)
    }

    pub fn open_shell_session<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        session_name: Option<&str>,
        fallback_console: bool,
    ) -> Result<ShellSession, AsldError> {
        let cfg = load_distro(store, name)?;
        let _ = self.require_running(name)?;
        let mode = select_session_mode(&cfg.agent, fallback_console)?;
        let requested_name = String::from(session_name.unwrap_or("default"));
        if let Some(existing) = self
            .backends
            .iter()
            .find(|backend| backend.name == name)
            .and_then(|backend| {
                backend
                    .shell_sessions
                    .iter()
                    .find(|session| session.session_name == requested_name && session.mode == mode)
            })
        {
            let mut reused = existing.clone();
            reused.reused = true;
            return Ok(reused);
        }

        let next_shell_id = format!("sh-{:08x}", self.next_shell_seq);
        self.next_shell_seq = self.next_shell_seq.wrapping_add(1);
        let backend = self
            .backend_mut(name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;

        let session = ShellSession {
            session_id: next_shell_id,
            session_name: requested_name,
            mode,
            console_pipe_name: String::new(),
            stdin_pipe_name: String::new(),
            attached_pid: 0,
            reused: false,
        };
        let io = provision_shell_io(&session.session_id, session.mode, &backend.vm.console_pipe_name)?;
        let session = ShellSession {
            console_pipe_name: io.console_pipe_name,
            stdin_pipe_name: io.stdin_pipe_name,
            attached_pid: io.attached_pid,
            ..session
        };
        backend.shell_sessions.push(session.clone());
        Ok(session)
    }

    pub fn exec_command<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        argv: &[String],
        cwd: Option<&str>,
        env: &[(&str, &str)],
        fallback_console: bool,
    ) -> Result<ExecInvocation, AsldError> {
        if argv.is_empty() {
            return Err(AsldError::InvalidArgument("argv"));
        }
        let cfg = load_distro(store, name)?;
        let _ = self.require_running(name)?;
        let mode = select_session_mode(&cfg.agent, fallback_console)?;
        let next_exec_id = format!("exec-{:08x}", self.next_exec_seq);
        self.next_exec_seq = self.next_exec_seq.wrapping_add(1);
        let backend = self
            .backend_mut(name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        let mut exec = ExecInvocation {
            exec_id: next_exec_id,
            mode,
            cwd: String::from(cwd.unwrap_or("/")),
            env_count: env.len(),
            command_line: join_command(argv),
            stdout_pipe_name: String::new(),
            stdin_pipe_name: String::new(),
            attached_pid: 0,
        };
        let io = provision_exec_io(&exec.exec_id, exec.mode, &exec.cwd, env, argv)?;
        exec.stdout_pipe_name = io.stdout_pipe_name;
        exec.stdin_pipe_name = io.stdin_pipe_name;
        exec.attached_pid = io.attached_pid;
        backend.execs.push(exec.clone());
        Ok(exec)
    }

    fn upsert_backend(&mut self, name: &str, vm: vm::VmInstance) {
        if let Some(existing) = self.backends.iter_mut().find(|backend| backend.name == name) {
            existing.vm = vm;
            existing.shell_sessions.clear();
            existing.execs.clear();
            return;
        }
        self.backends.push(RuntimeBackend {
            name: String::from(name),
            vm,
            shell_sessions: Vec::new(),
            execs: Vec::new(),
        });
    }

    fn backend_mut(&mut self, name: &str) -> Option<&mut RuntimeBackend> {
        self.backends.iter_mut().find(|backend| backend.name == name)
    }

    fn require_running(&self, name: &str) -> Result<&DistroStatus, AsldError> {
        let status = self.store.get(name).ok_or(AsldError::NotFound)?;
        if !matches!(status.state, DistroState::Ready | DistroState::Degraded) {
            return Err(AsldError::InvalidState("distro is not running"));
        }
        Ok(status)
    }
}

fn select_session_mode(
    policy: &crate::model::AgentPolicy,
    fallback_console: bool,
) -> Result<SessionMode, AsldError> {
    if fallback_console {
        if policy.fallback_console_enabled {
            return Ok(SessionMode::FallbackConsole);
        }
        return Err(AsldError::PolicyDenied);
    }
    if policy.enabled {
        return Ok(SessionMode::Agent);
    }
    if policy.fallback_console_enabled {
        return Ok(SessionMode::FallbackConsole);
    }
    Err(AsldError::PolicyDenied)
}

fn join_command(argv: &[String]) -> String {
    let mut out = String::new();
    for part in argv {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::config::FakeStore;

    use super::RuntimeService;

    #[test]
    fn create_and_status_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let created = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        assert_eq!(created.state.as_str(), "stopped");

        let status = runtime.status(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(status.name, "ubuntu-dev");
    }

    #[test]
    fn start_creates_running_backend_status() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let status = runtime.start(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(status.state.as_str(), "ready");
        assert!(status.last_error.is_none());
    }

    #[test]
    fn shell_session_reuses_named_session() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let first = runtime
            .open_shell_session(&mut store, "ubuntu-dev", Some("dev"), false)
            .unwrap();
        let second = runtime
            .open_shell_session(&mut store, "ubuntu-dev", Some("dev"), false)
            .unwrap();
        assert_eq!(first.session_id, second.session_id);
        assert!(second.reused);
        assert!(first.stdin_pipe_name.contains("sh-"));
    }

    #[test]
    fn exec_command_tracks_mode_and_command_line() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let exec = runtime
            .exec_command(
                &mut store,
                "ubuntu-dev",
                &alloc::vec![
                    alloc::string::String::from("cargo"),
                    alloc::string::String::from("test"),
                ],
                Some("/workspace"),
                &[("RUST_BACKTRACE", "1")],
                false,
            )
            .unwrap();
        assert_eq!(exec.mode.as_str(), "agent");
        assert_eq!(exec.cwd, "/workspace");
        assert_eq!(exec.command_line, "cargo test");
        assert!(exec.stdout_pipe_name.contains("exec-"));
        assert_eq!(exec.attached_pid, 0);
    }

    #[test]
    fn mount_management_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mounts = runtime
            .add_mount(
                &mut store,
                "ubuntu-dev",
                &crate::model::MountSpec {
                    id: alloc::string::String::from("workspace"),
                    host_path: alloc::string::String::from("/Users/strati/work"),
                    guest_path: alloc::string::String::from("/mnt/work"),
                    mode: alloc::string::String::from("readwrite"),
                    metadata_mode: alloc::string::String::from("relaxed"),
                    case_mode: alloc::string::String::from("host-native"),
                    exec_policy: alloc::string::String::from("inherit"),
                    watch_policy: alloc::string::String::from("best-effort"),
                    description: alloc::string::String::from("Workspace"),
                },
            )
            .unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(runtime.show_mount(&mut store, "ubuntu-dev", "workspace").unwrap().guest_path, "/mnt/work");
        assert!(runtime.remove_mount(&mut store, "ubuntu-dev", "workspace").unwrap().is_empty());
    }

    #[test]
    fn port_forward_management_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let rules = runtime
            .add_port_forward(
                &mut store,
                "ubuntu-dev",
                &crate::model::PortForwardSpec {
                    id: alloc::string::String::from("web"),
                    listen_address: alloc::string::String::from("127.0.0.1"),
                    listen_port: 3000,
                    guest_port: 3000,
                    protocol: alloc::string::String::from("tcp"),
                    description: alloc::string::String::from("Web"),
                },
            )
            .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "web");
        assert!(runtime.remove_port_forward(&mut store, "ubuntu-dev", "web").unwrap().is_empty());
    }
}
