use alloc::vec::Vec;

use crate::agent::inferred_agent_state;
use crate::config::{ensure_distro_tree, list_distros, load_distro, ConfigStore};
use crate::distro::build_distro_config;
use crate::errors::AsldError;
use crate::status::{degraded_status, stopped_status};
use crate::store::RuntimeStore;
use crate::vm;
use crate::model::{DistroStatus, DistroState};

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
}
