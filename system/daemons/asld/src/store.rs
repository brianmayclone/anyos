use alloc::vec::Vec;

use crate::model::DistroStatus;

#[derive(Default)]
pub struct RuntimeStore {
    statuses: Vec<DistroStatus>,
}

impl RuntimeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &[DistroStatus] {
        &self.statuses
    }

    pub fn get(&self, name: &str) -> Option<&DistroStatus> {
        self.statuses.iter().find(|s| s.name == name)
    }

    pub fn upsert(&mut self, status: DistroStatus) {
        if let Some(existing) = self.statuses.iter_mut().find(|s| s.name == status.name) {
            *existing = status;
        } else {
            self.statuses.push(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{AgentState, DistroHealth, DistroState, DistroStatus, NetworkPolicy, Resources};

    use super::RuntimeStore;

    #[test]
    fn upsert_replaces_existing_entry() {
        let mut store = RuntimeStore::new();
        let mut status = DistroStatus {
            name: alloc::string::String::from("ubuntu"),
            state: DistroState::Stopped,
            health: DistroHealth::Stopped,
            agent_state: AgentState::NotPresent,
            uptime_ms: 0,
            last_error: None,
            resources: Resources::default(),
            network: NetworkPolicy::default(),
        };
        store.upsert(status.clone());
        status.state = DistroState::Ready;
        store.upsert(status);
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get("ubuntu").unwrap().state.as_str(), "ready");
    }
}
