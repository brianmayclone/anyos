use crate::model::{AgentPolicy, AgentState, DistroState};

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

#[cfg(test)]
mod tests {
    use crate::model::{AgentPolicy, DistroState};

    use super::inferred_agent_state;

    #[test]
    fn disabled_policy_means_not_present() {
        let mut policy = AgentPolicy::default();
        policy.enabled = false;
        assert_eq!(
            inferred_agent_state(&policy, DistroState::Ready).as_str(),
            "not_present"
        );
    }
}
