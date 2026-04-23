use alloc::vec::Vec;

use crate::agent::inferred_agent_state;
use crate::config::{
    add_mount, add_port_forward, ensure_distro_tree, list_distros, load_distro, remove_mount,
    remove_port_forward, ConfigStore,
};
use crate::distro::build_distro_config;
use crate::errors::AsldError;
use crate::model::{DistroStatus, DistroState, MountSpec, PortForwardSpec};
use crate::status::{degraded_status, stopped_status};
use crate::store::RuntimeStore;
use crate::vm;

pub struct RuntimeService {
    store: RuntimeStore,
}

impl RuntimeService {
    pub fn new() -> Self {
        Self {
            store: RuntimeStore::new(),
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
            Ok(()) => {
                let mut status = stopped_status(&cfg.name, cfg.resources.clone(), cfg.network.clone());
                status.state = DistroState::Ready;
                status.health = crate::model::DistroHealth::Ready;
                status.agent_state = inferred_agent_state(&cfg.agent, DistroState::Ready);
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
    fn start_enters_degraded_until_vm_backend_exists() {
        let mut store = FakeStore::default();
        let mut runtime = RuntimeService::new();
        let _ = runtime.create(&mut store, "ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let status = runtime.start(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(status.state.as_str(), "degraded");
        assert!(status.last_error.unwrap().contains("not implemented"));
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
