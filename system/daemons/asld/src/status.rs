use alloc::string::String;

use crate::model::{AgentState, DistroHealth, DistroState, DistroStatus, NetworkPolicy, Resources};

pub fn stopped_status(name: &str, resources: Resources, network: NetworkPolicy) -> DistroStatus {
    DistroStatus {
        name: String::from(name),
        state: DistroState::Stopped,
        health: DistroHealth::Stopped,
        agent_state: AgentState::NotPresent,
        uptime_ms: 0,
        last_error: None,
        resources,
        network,
    }
}

pub fn degraded_status(
    name: &str,
    resources: Resources,
    network: NetworkPolicy,
    error: &str,
) -> DistroStatus {
    DistroStatus {
        name: String::from(name),
        state: DistroState::Degraded,
        health: DistroHealth::Degraded,
        agent_state: AgentState::Degraded,
        uptime_ms: 0,
        last_error: Some(String::from(error)),
        resources,
        network,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{NetworkPolicy, Resources};

    use super::{degraded_status, stopped_status};

    #[test]
    fn builds_stopped_status() {
        let status = stopped_status("ubuntu", Resources::default(), NetworkPolicy::default());
        assert_eq!(status.state.as_str(), "stopped");
        assert_eq!(status.health.as_str(), "stopped");
    }

    #[test]
    fn builds_degraded_status() {
        let status = degraded_status(
            "ubuntu",
            Resources::default(),
            NetworkPolicy::default(),
            "vm backend not implemented",
        );
        assert_eq!(status.state.as_str(), "degraded");
        assert!(status.last_error.is_some());
    }
}
