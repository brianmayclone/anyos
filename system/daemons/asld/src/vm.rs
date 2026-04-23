use alloc::format;
use alloc::string::String;

use crate::errors::AsldError;
use crate::model::DistroConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmInstance {
    pub vm_id: u32,
    pub vcpu_id: u32,
    pub backend: String,
    pub console_pipe_name: String,
}

pub fn start_vm(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    start_vm_impl(config)
}

pub fn stop_vm(instance: &VmInstance) -> Result<(), AsldError> {
    stop_vm_impl(instance)
}

#[cfg(target_os = "linux")]
fn start_vm_impl(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    Ok(VmInstance {
        vm_id: 1,
        vcpu_id: 0,
        backend: String::from("host-stub"),
        console_pipe_name: format!("asl-console-{}", config.name),
    })
}

#[cfg(target_os = "linux")]
fn stop_vm_impl(_instance: &VmInstance) -> Result<(), AsldError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn start_vm_impl(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    const SYS_VM_CREATE: u32 = 401;
    const SYS_VM_DESTROY: u32 = 402;
    const SYS_VM_HW_INFO: u32 = 405;
    const SYS_VCPU_CREATE: u32 = 407;

    let hw = libsyscall::syscall0(SYS_VM_HW_INFO) as u32;
    if hw == 0 {
        return Err(AsldError::BackendUnavailable("hardware virtualization not available"));
    }

    let vm_id = libsyscall::syscall0(SYS_VM_CREATE) as u32;
    if vm_id == 0 || vm_id == u32::MAX {
        return Err(AsldError::BackendUnavailable("vm_create failed"));
    }

    let vcpu_id = 0u32;
    let create_vcpu_result =
        libsyscall::syscall2(SYS_VCPU_CREATE, vm_id as u64, vcpu_id as u64) as u32;
    if create_vcpu_result == u32::MAX {
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(AsldError::BackendUnavailable("vcpu_create failed"));
    }

    Ok(VmInstance {
        vm_id,
        vcpu_id,
        backend: if hw == 1 {
            String::from("kernel-vmx")
        } else {
            String::from("kernel-svm")
        },
        console_pipe_name: format!("asl-console-{}", config.name),
    })
}

#[cfg(not(target_os = "linux"))]
fn stop_vm_impl(instance: &VmInstance) -> Result<(), AsldError> {
    const SYS_VM_DESTROY: u32 = 402;
    let rc = libsyscall::syscall1(SYS_VM_DESTROY, instance.vm_id as u64) as u32;
    if rc == u32::MAX {
        return Err(AsldError::BackendUnavailable("vm_destroy failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use crate::model::{
        AgentPolicy, DistroConfig, DistroMetadata, LifecyclePolicy, NetworkPolicy, Resources,
        StorageSpec,
    };

    use super::{start_vm, stop_vm};

    #[test]
    fn vm_start_returns_backend_instance() {
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
        let instance = start_vm(&cfg).unwrap();
        assert_eq!(instance.vcpu_id, 0);
        assert!(instance.console_pipe_name.contains("ubuntu"));
        stop_vm(&instance).unwrap();
    }
}
