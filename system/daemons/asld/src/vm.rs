use crate::errors::AsldError;
use crate::model::DistroConfig;

pub fn start_vm(_config: &DistroConfig) -> Result<(), AsldError> {
    Err(AsldError::NotImplemented("vm backend not implemented"))
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use crate::model::{
        AgentPolicy, DistroConfig, DistroMetadata, LifecyclePolicy, NetworkPolicy, Resources,
        StorageSpec,
    };

    use super::start_vm;

    #[test]
    fn vm_start_is_explicit_stub() {
        let cfg = DistroConfig {
            schema_version: 1,
            id: String::from("d1"),
            name: String::from("ubuntu"),
            owner: String::from("root"),
            base_image_ref: String::from("ubuntu"),
            kernel_profile: String::from("linux-x86_64-generic"),
            resources: Resources::default(),
            storage: StorageSpec {
                layout: String::from("layered-v1"),
                base_image_path: String::from("/base"),
                overlay_image_path: String::from("/overlay"),
                state_image_path: String::from("/state"),
                state_image_enabled: true,
            },
            network: NetworkPolicy::default(),
            mounts: Vec::new(),
            port_forwards: Vec::new(),
            agent: AgentPolicy::default(),
            lifecycle: LifecyclePolicy::default(),
            metadata: DistroMetadata::default(),
        };
        assert_eq!(
            start_vm(&cfg).unwrap_err().code(),
            "NOT_IMPLEMENTED"
        );
    }
}
