use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::image::validate_image_ref;
use crate::model::{
    default_storage_for, is_valid_distro_name, AgentPolicy, DistroConfig, DistroMetadata,
    LifecyclePolicy, MountSpec, NetworkPolicy, PortForwardSpec, Resources,
};

pub fn build_distro_config(
    name: &str,
    base_image_ref: &str,
    owner: &str,
) -> Result<DistroConfig, AsldError> {
    build_distro_config_with_kernel_profile(name, base_image_ref, owner, "linux-x86_64-generic")
}

pub fn build_distro_config_with_kernel_profile(
    name: &str,
    base_image_ref: &str,
    owner: &str,
    kernel_profile: &str,
) -> Result<DistroConfig, AsldError> {
    if !is_valid_distro_name(name) {
        return Err(AsldError::InvalidName);
    }
    if owner.is_empty() {
        return Err(AsldError::MissingField("owner"));
    }
    validate_image_ref(base_image_ref)?;
    if !is_valid_kernel_profile(kernel_profile) {
        return Err(AsldError::InvalidArgument("kernel_profile"));
    }

    Ok(DistroConfig {
        schema_version: 1,
        id: format!("distro-{name}"),
        name: String::from(name),
        owner: String::from(owner),
        base_image_ref: String::from(base_image_ref),
        kernel_profile: String::from(kernel_profile),
        resources: Resources::default(),
        storage: default_storage_for(name),
        network: NetworkPolicy::default(),
        mounts: Vec::<MountSpec>::new(),
        port_forwards: Vec::<PortForwardSpec>::new(),
        agent: AgentPolicy::default(),
        lifecycle: LifecyclePolicy::default(),
        metadata: DistroMetadata::default(),
    })
}

fn is_valid_kernel_profile(profile: &str) -> bool {
    !profile.is_empty()
        && profile
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::{build_distro_config, build_distro_config_with_kernel_profile};

    #[test]
    fn builds_default_distro_config() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        assert_eq!(cfg.name, "ubuntu-dev");
        assert_eq!(cfg.network.mode, "nat");
        assert_eq!(cfg.resources.memory_mb, 2048);
    }

    #[test]
    fn rejects_invalid_name() {
        assert!(build_distro_config("ubuntu/dev", "ubuntu", "strati").is_err());
    }

    #[test]
    fn supports_explicit_kernel_profile() {
        let cfg = build_distro_config_with_kernel_profile(
            "pcboot",
            "ubuntu-24.04-x86_64-v1",
            "strati",
            "seabios-x86_64",
        )
        .unwrap();
        assert_eq!(cfg.kernel_profile, "seabios-x86_64");
    }
}
