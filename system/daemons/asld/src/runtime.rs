use alloc::vec::Vec;
use alloc::{format, string::String};

use crate::agent::{inferred_agent_state, provision_exec_io, provision_shell_io};
use crate::config::{
    add_mount, add_port_forward, clone_distro, delete_distro, ensure_distro_tree, list_distros,
    load_distro, remove_mount, remove_port_forward, update_network, update_resources, ConfigStore,
};
use crate::distro::build_distro_config;
use crate::errors::AsldError;
use crate::model::{
    AgentState, DistroConfig, DistroHealth, DistroState, DistroStatus, ExecInvocation, MountSpec,
    MountValidation, NetworkPolicy, NetworkValidation, PortForwardSpec, SessionMode, ShellSession,
    StorageValidation, VmExitEvent, VmStatusSummary,
};
use crate::status::{degraded_status, stopped_status};
use crate::store::RuntimeStore;
use crate::vm;

#[derive(Clone)]
struct RuntimeBackend {
    name: String,
    vm: vm::VmInstance,
    boot_summary: String,
    exit_history: Vec<VmExitEvent>,
    total_exits: u64,
    shell_sessions: Vec<ShellSession>,
    execs: Vec<ExecInvocation>,
}

pub struct RuntimeService {
    store: RuntimeStore,
    backends: Vec<RuntimeBackend>,
    next_shell_seq: u32,
    next_exec_seq: u32,
    next_exit_seq: u64,
}

impl RuntimeService {
    pub fn new() -> Self {
        Self {
            store: RuntimeStore::new(),
            backends: Vec::new(),
            next_shell_seq: 1,
            next_exec_seq: 1,
            next_exit_seq: 1,
        }
    }

    pub fn tick(&mut self) {
        for index in 0..self.backends.len() {
            let name = self.backends[index].name.clone();
            let poll_result = {
                let backend = &mut self.backends[index];
                vm::poll_runtime(&mut backend.vm)
            };
            match poll_result {
                Ok(Some(exit)) => self.record_vm_exit(index, &name, exit),
                Ok(None) => {}
                Err(err) => {
                    if let Some(status) = self.store.get_mut(&name) {
                        status.state = DistroState::Degraded;
                        status.health = DistroHealth::Degraded;
                        status.agent_state = crate::model::AgentState::Degraded;
                        status.last_error = Some(err.message());
                    }
                }
            }
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

    pub fn status<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<DistroStatus, AsldError> {
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

    pub fn delete<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        force: bool,
    ) -> Result<DistroConfig, AsldError> {
        let cfg = load_distro(store, name)?;
        if let Some(index) = self
            .backends
            .iter()
            .position(|backend| backend.name == name)
        {
            if !force {
                return Err(AsldError::InvalidState("distro is running"));
            }
            let instance = self.backends[index].vm.clone();
            vm::stop_vm(&instance)?;
            self.backends.remove(index);
        }
        let deleted = delete_distro(store, &cfg.name)?;
        let _ = crate::broker::clear_distro(&cfg.name);
        let _ = self.store.remove(&cfg.name);
        Ok(deleted)
    }

    pub fn clone<S: ConfigStore>(
        &mut self,
        store: &mut S,
        source_name: &str,
        target_name: &str,
        owner: Option<&str>,
        include_mounts: bool,
    ) -> Result<Vec<String>, AsldError> {
        if let Some(status) = self.store.get(source_name) {
            if matches!(
                status.state,
                DistroState::Ready
                    | DistroState::Starting
                    | DistroState::Booting
                    | DistroState::Degraded
                    | DistroState::Stopping
            ) {
                return Err(AsldError::InvalidState(
                    "clone requires a stopped source distro",
                ));
            }
        }
        let cfg = clone_distro(
            store,
            source_name,
            target_name,
            owner,
            include_mounts,
            false,
        )?;
        self.store.upsert(stopped_status(
            &cfg.name,
            cfg.resources.clone(),
            cfg.network.clone(),
        ));
        Ok(format_config_lines(&cfg))
    }

    pub fn config_lines<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<String>, AsldError> {
        let cfg = load_distro(store, name)?;
        Ok(format_config_lines(&cfg))
    }

    pub fn export_lines<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<String>, AsldError> {
        let cfg = load_distro(store, name)?;
        Ok(crate::storage::export_manifest_lines(&cfg))
    }

    pub fn validate_storage<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<StorageValidation>, AsldError> {
        let cfg = load_distro(store, name)?;
        Ok(crate::storage::validate_storage(&cfg.name, &cfg.storage))
    }

    pub fn update_resources<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        memory_mb: Option<u32>,
        vcpu_count: Option<u16>,
    ) -> Result<Vec<String>, AsldError> {
        if let Some(status) = self.store.get(name) {
            if matches!(
                status.state,
                DistroState::Ready
                    | DistroState::Starting
                    | DistroState::Booting
                    | DistroState::Degraded
            ) {
                return Err(AsldError::InvalidState(
                    "resource changes require a stopped distro",
                ));
            }
        }
        let cfg = update_resources(store, name, memory_mb, vcpu_count)?;
        self.store.upsert(stopped_status(
            &cfg.name,
            cfg.resources.clone(),
            cfg.network.clone(),
        ));
        Ok(format_config_lines(&cfg))
    }

    pub fn network_policy<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<NetworkPolicy, AsldError> {
        Ok(load_distro(store, name)?.network)
    }

    pub fn update_network<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        mode: Option<&str>,
        dns_mode: Option<&str>,
        allow_outbound: Option<bool>,
    ) -> Result<Vec<String>, AsldError> {
        if let Some(status) = self.store.get(name) {
            if matches!(
                status.state,
                DistroState::Ready
                    | DistroState::Starting
                    | DistroState::Booting
                    | DistroState::Degraded
            ) {
                return Err(AsldError::InvalidState(
                    "network changes require a stopped distro",
                ));
            }
        }
        let cfg = update_network(store, name, mode, dns_mode, allow_outbound)?;
        self.store.upsert(stopped_status(
            &cfg.name,
            cfg.resources.clone(),
            cfg.network.clone(),
        ));
        Ok(format_config_lines(&cfg))
    }

    pub fn restart_agent<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<String>, AsldError> {
        let cfg = load_distro(store, name)?;
        if !cfg.agent.enabled {
            return Err(AsldError::PolicyDenied);
        }
        let status = self
            .store
            .get_mut(name)
            .ok_or(AsldError::InvalidState("distro is not running"))?;
        if !matches!(status.state, DistroState::Ready | DistroState::Degraded) {
            return Err(AsldError::InvalidState("distro is not running"));
        }
        status.agent_state = crate::model::AgentState::Starting;
        status.last_error = None;
        Ok(alloc::vec![
            format!("agent\t{}", status.agent_state.as_str()),
            String::from("restart\trequested"),
            format!("fallback_console\t{}", cfg.agent.fallback_console_enabled),
        ])
    }

    pub fn update_agent_state<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
        next_state: AgentState,
        version: Option<&str>,
    ) -> Result<Vec<String>, AsldError> {
        let cfg = load_distro(store, name)?;
        if !cfg.agent.enabled {
            return Err(AsldError::PolicyDenied);
        }
        let status = self
            .store
            .get_mut(name)
            .ok_or(AsldError::InvalidState("distro is not running"))?;
        if !matches!(
            status.state,
            DistroState::Starting
                | DistroState::Booting
                | DistroState::Ready
                | DistroState::Degraded
        ) {
            return Err(AsldError::InvalidState("distro is not running"));
        }
        status.agent_state = next_state;
        let _ = crate::broker::record_observation(name, next_state.as_str());
        Ok(alloc::vec![
            format!("agent\t{}", status.agent_state.as_str()),
            format!("version\t{}", version.unwrap_or("-")),
            format!("fallback_console\t{}", cfg.agent.fallback_console_enabled),
        ])
    }

    pub fn start<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<DistroStatus, AsldError> {
        let cfg = load_distro(store, name)?;
        if let Some(existing) = self.store.get(name) {
            if matches!(
                existing.state,
                DistroState::Ready | DistroState::Starting | DistroState::Booting
            ) {
                return Err(AsldError::InvalidState("already running or starting"));
            }
        }

        match vm::start_vm(&cfg) {
            Ok(vm_instance) => {
                self.upsert_backend(&cfg.name, vm_instance);
                let boot_report = {
                    let backend = self
                        .backend_mut(&cfg.name)
                        .ok_or(AsldError::InvalidState("distro runtime missing"))?;
                    vm::boot_probe(&mut backend.vm)
                };

                let mut status =
                    stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
                match boot_report {
                    Ok(report) if report.ready => {
                        if let Some(backend) = self.backend_mut(&cfg.name) {
                            backend.boot_summary = report.summary.clone();
                        }
                        status.state = DistroState::Ready;
                        status.health = DistroHealth::Ready;
                        status.agent_state = inferred_agent_state(&cfg.agent, DistroState::Ready);
                    }
                    Ok(report) => {
                        if let Some(backend) = self.backend_mut(&cfg.name) {
                            backend.boot_summary = report.summary.clone();
                        }
                        status.state = DistroState::Degraded;
                        status.health = DistroHealth::Degraded;
                        status.agent_state =
                            inferred_agent_state(&cfg.agent, DistroState::Degraded);
                        status.last_error = Some(report.summary);
                    }
                    Err(err) => {
                        if let Some(backend) = self.backend_mut(&cfg.name) {
                            backend.boot_summary = err.message();
                        }
                        status.state = DistroState::Degraded;
                        status.health = DistroHealth::Degraded;
                        status.agent_state =
                            inferred_agent_state(&cfg.agent, DistroState::Degraded);
                        status.last_error = Some(err.message());
                    }
                }
                if let Err(err) = crate::broker::sync_distro(&cfg) {
                    status.state = DistroState::Degraded;
                    status.health = DistroHealth::Degraded;
                    status.last_error = Some(err.message());
                }
                let _ = crate::broker::record_observation(&cfg.name, status.health.as_str());
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

    pub fn restart<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<DistroStatus, AsldError> {
        let _ = self.stop(store, name)?;
        self.start(store, name)
    }

    pub fn stop<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<DistroStatus, AsldError> {
        let cfg = load_distro(store, name)?;
        if let Some(index) = self
            .backends
            .iter()
            .position(|backend| backend.name == name)
        {
            let instance = self.backends[index].vm.clone();
            vm::stop_vm(&instance)?;
            self.backends.remove(index);
        }
        let status = stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
        let _ = crate::broker::clear_distro(&cfg.name);
        let _ = crate::broker::record_observation(&cfg.name, "stopped");
        self.store.upsert(status.clone());
        Ok(status)
    }

    pub fn list_mounts<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<MountSpec>, AsldError> {
        Ok(load_distro(store, name)?.mounts)
    }

    pub fn validate_mounts<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<MountValidation>, AsldError> {
        let mounts = self.list_mounts(store, name)?;
        Ok(crate::mounts::validate_mount_set(&mounts))
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
        if self.is_active(distro_name) {
            let cfg = load_distro(store, distro_name)?;
            crate::broker::sync_filesystem(&cfg)?;
        }
        let _ = crate::broker::record_observation(distro_name, "mount_changed");
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
        if self.is_active(distro_name) {
            let cfg = load_distro(store, distro_name)?;
            crate::broker::sync_filesystem(&cfg)?;
        }
        let _ = crate::broker::record_observation(distro_name, "mount_changed");
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
        ensure_port_listener_available(store, distro_name, rule)?;
        add_port_forward(store, distro_name, rule)?;
        if self.is_active(distro_name) {
            let cfg = load_distro(store, distro_name)?;
            crate::broker::sync_network(&cfg)?;
        }
        let _ = crate::broker::record_observation(distro_name, "port_changed");
        self.list_port_forwards(store, distro_name)
    }

    pub fn validate_network<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<NetworkValidation>, AsldError> {
        let cfg = load_distro(store, name)?;
        let mut report = crate::network::validate_network_set(&cfg.network, &cfg.port_forwards);
        annotate_cross_distro_port_conflicts(store, name, &mut report, &cfg.port_forwards)?;
        Ok(report)
    }

    pub fn vm_status(&self, name: &str) -> Result<VmStatusSummary, AsldError> {
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.name == name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        let guest_memory_mb = (backend.vm.guest_memory_size / (1024 * 1024)) as u32;
        let last_exit_summary = backend
            .exit_history
            .last()
            .map(|event| event.summary.clone())
            .unwrap_or_default();
        Ok(VmStatusSummary {
            backend: backend.vm.backend.clone(),
            run_state: backend.vm.run_state,
            guest_memory_mb,
            boot_summary: backend.boot_summary.clone(),
            last_exit_summary,
            total_exits: backend.total_exits,
            recent_exit_count: backend.exit_history.len(),
        })
    }

    pub fn vm_events(&self, name: &str) -> Result<Vec<VmExitEvent>, AsldError> {
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.name == name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        Ok(backend.exit_history.clone())
    }

    pub fn vm_events_tail(&self, name: &str, limit: usize) -> Result<Vec<VmExitEvent>, AsldError> {
        let events = self.vm_events(name)?;
        if limit == 0 || events.len() <= limit {
            return Ok(events);
        }
        Ok(events[events.len() - limit..].to_vec())
    }

    pub fn clear_vm_events(&mut self, name: &str) -> Result<usize, AsldError> {
        let backend = self
            .backend_mut(name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        let cleared = backend.exit_history.len();
        backend.exit_history.clear();
        Ok(cleared)
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
        if self.is_active(distro_name) {
            let cfg = load_distro(store, distro_name)?;
            crate::broker::sync_network(&cfg)?;
        }
        let _ = crate::broker::record_observation(distro_name, "port_changed");
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
        if let Some(existing) =
            self.backends
                .iter()
                .find(|backend| backend.name == name)
                .and_then(|backend| {
                    backend.shell_sessions.iter().find(|session| {
                        session.session_name == requested_name && session.mode == mode
                    })
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
        let io = provision_shell_io(
            &session.session_id,
            session.mode,
            &backend.vm.console_pipe_name,
        )?;
        let session = ShellSession {
            console_pipe_name: io.console_pipe_name,
            stdin_pipe_name: io.stdin_pipe_name,
            attached_pid: io.attached_pid,
            ..session
        };
        crate::broker::attach_console_session(name, &session)?;
        let _ = crate::broker::record_observation(name, "shell_opened");
        backend.shell_sessions.push(session.clone());
        Ok(session)
    }

    pub fn list_shell_sessions(&self, name: &str) -> Result<Vec<ShellSession>, AsldError> {
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.name == name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        Ok(backend.shell_sessions.clone())
    }

    pub fn show_shell_session(
        &self,
        name: &str,
        session_id: &str,
    ) -> Result<ShellSession, AsldError> {
        self.list_shell_sessions(name)?
            .into_iter()
            .find(|session| session.session_id == session_id)
            .ok_or(AsldError::NotFound)
    }

    pub fn close_shell_session(
        &mut self,
        name: &str,
        session_id: &str,
    ) -> Result<Vec<ShellSession>, AsldError> {
        let backend = self
            .backend_mut(name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        let before = backend.shell_sessions.len();
        backend
            .shell_sessions
            .retain(|session| session.session_id != session_id);
        if backend.shell_sessions.len() == before {
            return Err(AsldError::NotFound);
        }
        Ok(backend.shell_sessions.clone())
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
        let _ = crate::broker::record_observation(name, "exec_opened");
        backend.execs.push(exec.clone());
        Ok(exec)
    }

    pub fn list_execs(&self, name: &str) -> Result<Vec<ExecInvocation>, AsldError> {
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.name == name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        Ok(backend.execs.clone())
    }

    pub fn show_exec(&self, name: &str, exec_id: &str) -> Result<ExecInvocation, AsldError> {
        self.list_execs(name)?
            .into_iter()
            .find(|exec| exec.exec_id == exec_id)
            .ok_or(AsldError::NotFound)
    }

    pub fn clear_execs(&mut self, name: &str) -> Result<usize, AsldError> {
        let backend = self
            .backend_mut(name)
            .ok_or(AsldError::InvalidState("distro runtime missing"))?;
        let cleared = backend.execs.len();
        backend.execs.clear();
        Ok(cleared)
    }

    pub fn diagnose<S: ConfigStore>(
        &mut self,
        store: &mut S,
        name: &str,
    ) -> Result<Vec<String>, AsldError> {
        let cfg = load_distro(store, name)?;
        let status = self.store.get(name).cloned().unwrap_or_else(|| {
            stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone())
        });
        let mount_report = crate::mounts::validate_mount_set(&cfg.mounts);
        let unhealthy_mounts = mount_report.iter().filter(|item| !item.valid).count();
        let storage_report = crate::storage::validate_storage(&cfg.name, &cfg.storage);
        let unhealthy_storage = storage_report.iter().filter(|item| !item.valid).count();
        let network_report = self.validate_network(store, name)?;
        let unhealthy_network = network_report.iter().filter(|item| !item.valid).count();
        let boot_plan = crate::boot::build_boot_plan(&cfg);
        let mut lines = alloc::vec![
            format!("name\t{}", status.name),
            format!("state\t{}", status.state.as_str()),
            format!("health\t{}", status.health.as_str()),
            format!("agent\t{}", status.agent_state.as_str()),
            format!(
                "resources\t{}vcpu\t{}MiB",
                status.resources.vcpu_count, status.resources.memory_mb
            ),
            format!(
                "network\t{}\tdns={}\toutbound={}",
                cfg.network.mode, cfg.network.dns_mode, cfg.network.allow_outbound
            ),
            format!("boot_mode\t{}", boot_plan.mode),
            format!("boot_ready\t{}", boot_plan.startable),
            format!("boot_kernel\t{}", boot_plan.kernel_path),
            format!("boot_initrd\t{}", boot_plan.initrd_path),
            format!("boot_cmdline\t{}", boot_plan.cmdline),
            format!("boot_message\t{}", boot_plan.message),
            format!(
                "network_broker\taslnetd\tmode={}\tdns={}\tforwards={}",
                cfg.network.mode,
                cfg.network.dns_mode,
                cfg.port_forwards.len()
            ),
            format!(
                "storage_layout\t{}\tunhealthy={}",
                cfg.storage.layout, unhealthy_storage
            ),
            format!(
                "fs_broker\taslfsd\texports={}\tunhealthy={}",
                cfg.mounts.len(),
                unhealthy_mounts
            ),
            String::from("console_broker\taslconsoled"),
            String::from("observability_broker\taslobsd"),
            format!(
                "mounts\t{}\tunhealthy={}",
                cfg.mounts.len(),
                unhealthy_mounts
            ),
            format!(
                "ports\t{}\tunhealthy={}",
                cfg.port_forwards.len(),
                unhealthy_network
            ),
        ];
        append_broker_status(&mut lines, "aslnetd");
        append_broker_status(&mut lines, "aslfsd");
        append_broker_status(&mut lines, "aslconsoled");
        append_broker_status(&mut lines, "aslobsd");

        if let Ok(vm) = self.vm_status(name) {
            let shells = self.list_shell_sessions(name).unwrap_or_default();
            let execs = self.list_execs(name).unwrap_or_default();
            let events = self.vm_events(name).unwrap_or_default();
            lines.extend(alloc::vec![
                format!("vm_backend\t{}", vm.backend),
                format!("vm_run_state\t{}", vm.run_state.as_str()),
                format!("vm_boot\t{}", vm.boot_summary),
                format!("shell_sessions\t{}", shells.len()),
                format!("execs\t{}", execs.len()),
                format!("vm_events\t{}", events.len()),
                format!("vm_total_exits\t{}", vm.total_exits),
            ]);
        } else {
            lines.push(String::from("vm_backend\tnone"));
            lines.push(String::from("vm_run_state\tstopped"));
        }

        Ok(lines)
    }

    fn upsert_backend(&mut self, name: &str, vm: vm::VmInstance) {
        if let Some(existing) = self
            .backends
            .iter_mut()
            .find(|backend| backend.name == name)
        {
            existing.vm = vm;
            existing.boot_summary.clear();
            existing.exit_history.clear();
            existing.total_exits = 0;
            existing.shell_sessions.clear();
            existing.execs.clear();
            return;
        }
        self.backends.push(RuntimeBackend {
            name: String::from(name),
            vm,
            boot_summary: String::new(),
            exit_history: Vec::new(),
            total_exits: 0,
            shell_sessions: Vec::new(),
            execs: Vec::new(),
        });
    }

    fn backend_mut(&mut self, name: &str) -> Option<&mut RuntimeBackend> {
        self.backends
            .iter_mut()
            .find(|backend| backend.name == name)
    }

    fn is_active(&self, name: &str) -> bool {
        self.store.get(name).is_some_and(|status| {
            matches!(
                status.state,
                DistroState::Ready
                    | DistroState::Starting
                    | DistroState::Booting
                    | DistroState::Degraded
            )
        })
    }

    fn require_running(&self, name: &str) -> Result<&DistroStatus, AsldError> {
        let status = self.store.get(name).ok_or(AsldError::NotFound)?;
        if !matches!(status.state, DistroState::Ready | DistroState::Degraded) {
            return Err(AsldError::InvalidState("distro is not running"));
        }
        Ok(status)
    }

    fn record_vm_exit(&mut self, index: usize, name: &str, exit: vm::VmRuntimeEvent) {
        let seq = self.next_exit_seq;
        self.next_exit_seq = self.next_exit_seq.wrapping_add(1);
        let event = VmExitEvent {
            seq,
            reason: exit.reason.clone(),
            summary: exit.summary.clone(),
            fatal: exit.fatal,
            qualification: exit.qualification,
            guest_phys_addr: exit.guest_phys_addr,
            guest_virt_addr: exit.guest_virt_addr,
        };

        let backend = &mut self.backends[index];
        backend.total_exits = backend.total_exits.saturating_add(1);
        backend.exit_history.push(event.clone());
        if backend.exit_history.len() > 32 {
            backend.exit_history.remove(0);
        }

        if let Some(status) = self.store.get_mut(name) {
            if exit.fatal {
                status.state = DistroState::Degraded;
                status.health = DistroHealth::Degraded;
                status.agent_state = crate::model::AgentState::Degraded;
                status.last_error = Some(exit.summary);
            } else {
                status.state = DistroState::Ready;
                status.health = DistroHealth::Ready;
                if exit.halted {
                    status.last_error = None;
                }
            }
        }
    }
}

fn ensure_port_listener_available<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    candidate: &PortForwardSpec,
) -> Result<(), AsldError> {
    for name in list_distros(store)? {
        let cfg = load_distro(store, &name)?;
        for existing in &cfg.port_forwards {
            if name == distro_name && existing.id == candidate.id {
                continue;
            }
            if crate::network::same_listener(existing, candidate) {
                return Err(AsldError::InvalidState("port listener already configured"));
            }
        }
    }
    Ok(())
}

fn annotate_cross_distro_port_conflicts<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    report: &mut [NetworkValidation],
    rules: &[PortForwardSpec],
) -> Result<(), AsldError> {
    for other_name in list_distros(store)? {
        if other_name == distro_name {
            continue;
        }
        let other = load_distro(store, &other_name)?;
        for rule in rules {
            if other
                .port_forwards
                .iter()
                .any(|existing| crate::network::same_listener(existing, rule))
            {
                if let Some(item) = report.iter_mut().find(|item| item.id == rule.id) {
                    item.valid = false;
                    item.message = format!("listener conflicts with distro {}", other_name);
                }
            }
        }
    }
    Ok(())
}

fn append_broker_status(lines: &mut Vec<String>, broker: &'static str) {
    match crate::broker::status(broker) {
        Ok(status_lines) => {
            lines.push(format!("broker\t{}\tavailable\ttrue", broker));
            for line in status_lines {
                lines.push(format!("broker_status\t{}\t{}", broker, line));
            }
        }
        Err(err) => {
            lines.push(format!(
                "broker\t{}\tavailable\tfalse\t{}",
                broker,
                err.message()
            ));
        }
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

fn format_config_lines(cfg: &DistroConfig) -> Vec<String> {
    let mut lines = alloc::vec![
        format!("schema_version\t{}", cfg.schema_version),
        format!("id\t{}", cfg.id),
        format!("name\t{}", cfg.name),
        format!("owner\t{}", cfg.owner),
        format!("base_image_ref\t{}", cfg.base_image_ref),
        format!("kernel_profile\t{}", cfg.kernel_profile),
        format!("resources.memory_mb\t{}", cfg.resources.memory_mb),
        format!("resources.vcpu_count\t{}", cfg.resources.vcpu_count),
        format!("resources.autostart\t{}", cfg.resources.autostart),
        format!("storage.layout\t{}", cfg.storage.layout),
        format!("storage.base_image_path\t{}", cfg.storage.base_image_path),
        format!(
            "storage.overlay_image_path\t{}",
            cfg.storage.overlay_image_path
        ),
        format!("storage.state_image_path\t{}", cfg.storage.state_image_path),
        format!(
            "storage.state_image_enabled\t{}",
            cfg.storage.state_image_enabled
        ),
        format!("network.mode\t{}", cfg.network.mode),
        format!("network.dns_mode\t{}", cfg.network.dns_mode),
        format!("network.allow_outbound\t{}", cfg.network.allow_outbound),
        format!("agent.enabled\t{}", cfg.agent.enabled),
        format!(
            "agent.required_for_rich_integration\t{}",
            cfg.agent.required_for_rich_integration
        ),
        format!(
            "agent.fallback_console_enabled\t{}",
            cfg.agent.fallback_console_enabled
        ),
        format!(
            "lifecycle.restart_on_failure\t{}",
            cfg.lifecycle.restart_on_failure
        ),
        format!(
            "lifecycle.shutdown_timeout_ms\t{}",
            cfg.lifecycle.shutdown_timeout_ms
        ),
        format!(
            "lifecycle.boot_timeout_ms\t{}",
            cfg.lifecycle.boot_timeout_ms
        ),
        format!("metadata.distro_family\t{}", cfg.metadata.distro_family),
        format!("metadata.distro_version\t{}", cfg.metadata.distro_version),
        format!("metadata.notes\t{}", cfg.metadata.notes),
        format!("mounts.count\t{}", cfg.mounts.len()),
        format!("port_forwards.count\t{}", cfg.port_forwards.len()),
    ];
    for mount in &cfg.mounts {
        lines.push(format!(
            "mount.{}\t{}\t{}\t{}",
            mount.id, mount.host_path, mount.guest_path, mount.mode
        ));
    }
    for rule in &cfg.port_forwards {
        lines.push(format!(
            "port.{}\t{}:{}\t{}\t{}",
            rule.id, rule.listen_address, rule.listen_port, rule.guest_port, rule.protocol
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use crate::config::FakeStore;

    use super::RuntimeService;

    #[test]
    fn create_and_status_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let created = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        assert_eq!(created.state.as_str(), "stopped");

        let status = runtime.status(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(status.name, "ubuntu-dev");
    }

    #[test]
    fn start_creates_running_backend_status() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let status = runtime.start(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(status.state.as_str(), "ready");
        assert!(status.last_error.is_none());
    }

    #[test]
    fn agent_restart_marks_agent_starting_without_restarting_distro() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();

        let lines = runtime.restart_agent(&mut store, "ubuntu-dev").unwrap();
        assert!(lines.iter().any(|line| line == "restart\trequested"));
        assert_eq!(
            runtime
                .status(&mut store, "ubuntu-dev")
                .unwrap()
                .agent_state
                .as_str(),
            "starting"
        );
    }

    #[test]
    fn shell_session_reuses_named_session() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
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
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
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
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
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
        assert_eq!(
            runtime
                .show_mount(&mut store, "ubuntu-dev", "workspace")
                .unwrap()
                .guest_path,
            "/mnt/work"
        );
        assert!(runtime
            .remove_mount(&mut store, "ubuntu-dev", "workspace")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn port_forward_management_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
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
        assert!(runtime
            .remove_port_forward(&mut store, "ubuntu-dev", "web")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn network_policy_update_and_validation_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();

        let lines = runtime
            .update_network(
                &mut store,
                "ubuntu-dev",
                Some("nat"),
                Some("host-broker"),
                Some(false),
            )
            .unwrap();
        assert!(lines
            .iter()
            .any(|line| line == "network.allow_outbound\tfalse"));

        let policy = runtime.network_policy(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(policy.mode, "nat");
        assert!(!policy.allow_outbound);

        let report = runtime.validate_network(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(report[0].id, "policy");
        assert!(report[0].valid);
    }

    #[test]
    fn clone_export_and_storage_validation_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();

        let cloned = runtime
            .clone(&mut store, "ubuntu-dev", "ubuntu-copy", Some("ops"), true)
            .unwrap();
        assert!(cloned.iter().any(|line| line == "name\tubuntu-copy"));
        assert!(cloned
            .iter()
            .any(|line| line.contains("/ubuntu-copy/images/overlay.img")));

        let export = runtime.export_lines(&mut store, "ubuntu-copy").unwrap();
        assert!(export.iter().any(|line| line == "format\tasl-export-v1"));

        let storage = runtime.validate_storage(&mut store, "ubuntu-copy").unwrap();
        assert!(storage.iter().all(|item| item.valid));
    }

    #[test]
    fn port_listener_conflict_is_rejected_across_distros() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime
            .create(&mut store, "debian-dev", "debian-13-x86_64-v1", "strati")
            .unwrap();
        let rule = crate::model::PortForwardSpec {
            id: alloc::string::String::from("web"),
            listen_address: alloc::string::String::from("127.0.0.1"),
            listen_port: 3000,
            guest_port: 3000,
            protocol: alloc::string::String::from("tcp"),
            description: alloc::string::String::from("Web"),
        };
        let _ = runtime
            .add_port_forward(&mut store, "ubuntu-dev", &rule)
            .unwrap();
        let result = runtime.add_port_forward(&mut store, "debian-dev", &rule);
        assert!(result.is_err());
    }

    #[test]
    fn vm_status_exposes_boot_summary() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let vm_status = runtime.vm_status("ubuntu-dev").unwrap();
        assert!(vm_status.backend.contains("stub") || vm_status.backend.contains("kernel"));
        assert!(vm_status.boot_summary.contains("boot"));
    }

    #[test]
    fn tick_keeps_host_stub_without_exit_history() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        runtime.tick();
        assert!(runtime.vm_events("ubuntu-dev").unwrap().is_empty());
    }

    #[test]
    fn shell_session_inventory_and_close_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let shell = runtime
            .open_shell_session(&mut store, "ubuntu-dev", Some("ops"), false)
            .unwrap();
        assert_eq!(runtime.list_shell_sessions("ubuntu-dev").unwrap().len(), 1);
        assert!(runtime
            .close_shell_session("ubuntu-dev", &shell.session_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn exec_inventory_and_event_clear_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let _ = runtime
            .exec_command(
                &mut store,
                "ubuntu-dev",
                &alloc::vec![alloc::string::String::from("uname")],
                None,
                &[],
                false,
            )
            .unwrap();
        assert_eq!(runtime.list_execs("ubuntu-dev").unwrap().len(), 1);
        assert_eq!(runtime.clear_vm_events("ubuntu-dev").unwrap(), 0);
        assert!(runtime
            .diagnose(&mut store, "ubuntu-dev")
            .unwrap()
            .iter()
            .any(|line| line.starts_with("vm_backend\t")));
    }

    #[test]
    fn diagnose_exposes_boot_plan() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();

        let lines = runtime.diagnose(&mut store, "ubuntu-dev").unwrap();
        assert!(lines.iter().any(|line| line == "boot_mode\tdirect-linux"));
        assert!(lines.iter().any(|line| line == "boot_ready\tfalse"));
        assert!(lines
            .iter()
            .any(|line| line == "boot_kernel\t/System/var/asl/distros/ubuntu-dev/boot/vmlinuz"));
        assert!(lines
            .iter()
            .any(|line| line.starts_with("boot_cmdline\tconsole=ttyS0")));
    }

    #[test]
    fn restart_rebuilds_running_backend() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let restarted = runtime.restart(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(restarted.state.as_str(), "ready");
    }

    #[test]
    fn shell_and_exec_show_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let shell = runtime
            .open_shell_session(&mut store, "ubuntu-dev", Some("ops"), false)
            .unwrap();
        let exec = runtime
            .exec_command(
                &mut store,
                "ubuntu-dev",
                &alloc::vec![alloc::string::String::from("uname")],
                None,
                &[],
                false,
            )
            .unwrap();
        assert_eq!(
            runtime
                .show_shell_session("ubuntu-dev", &shell.session_id)
                .unwrap()
                .session_name,
            "ops"
        );
        assert_eq!(
            runtime
                .show_exec("ubuntu-dev", &exec.exec_id)
                .unwrap()
                .command_line,
            "uname"
        );
    }

    #[test]
    fn vm_event_tail_and_exec_clear_roundtrip() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime
            .create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati")
            .unwrap();
        let _ = runtime.start(&mut store, "ubuntu-dev").unwrap();
        let _ = runtime
            .exec_command(
                &mut store,
                "ubuntu-dev",
                &alloc::vec![alloc::string::String::from("uname")],
                None,
                &[],
                false,
            )
            .unwrap();
        assert!(runtime.vm_events_tail("ubuntu-dev", 5).unwrap().is_empty());
        assert_eq!(runtime.clear_execs("ubuntu-dev").unwrap(), 1);
        assert!(runtime.list_execs("ubuntu-dev").unwrap().is_empty());
    }
}
