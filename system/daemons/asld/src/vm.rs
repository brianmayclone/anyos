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
    pub guest_memory_addr: usize,
    pub guest_memory_size: usize,
}

pub fn start_vm(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    start_vm_impl(config)
}

pub fn stop_vm(instance: &VmInstance) -> Result<(), AsldError> {
    stop_vm_impl(instance)
}

#[cfg(target_os = "linux")]
fn start_vm_impl(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    ensure_pipe(&format!("asl-console-{}", config.name))?;
    Ok(VmInstance {
        vm_id: 1,
        vcpu_id: 0,
        backend: String::from("host-stub"),
        console_pipe_name: format!("asl-console-{}", config.name),
        guest_memory_addr: 0,
        guest_memory_size: align_guest_memory_size(config.resources.memory_mb),
    })
}

#[cfg(target_os = "linux")]
fn stop_vm_impl(_instance: &VmInstance) -> Result<(), AsldError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn start_vm_impl(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    const SYS_VM_CREATE: u32 = 600;
    const SYS_VM_DESTROY: u32 = 601;
    const SYS_VM_SET_MEMORY: u32 = 602;
    const SYS_VCPU_CREATE: u32 = 603;
    const SYS_VM_HW_INFO: u32 = 613;

    #[repr(C)]
    struct MemRegionDesc {
        guest_phys: u64,
        size: u64,
        host_phys: u64,
    }

    let hw = libsyscall::syscall0(SYS_VM_HW_INFO) as u32;
    if hw == 0 {
        return Err(AsldError::BackendUnavailable("hardware virtualization not available"));
    }

    let vm_id = libsyscall::syscall0(SYS_VM_CREATE) as u32;
    if vm_id == 0 || vm_id == u32::MAX {
        return Err(AsldError::BackendUnavailable("vm_create failed"));
    }

    let guest_memory_size = align_guest_memory_size(config.resources.memory_mb);
    let guest_memory = anyos_std::process::mmap(guest_memory_size);
    if guest_memory.is_null() {
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(AsldError::BackendUnavailable("guest memory allocation failed"));
    }
    unsafe {
        *guest_memory = 0xf4;
    }

    let memory_desc = MemRegionDesc {
        guest_phys: 0,
        size: guest_memory_size as u64,
        host_phys: guest_memory as u64,
    };
    let map_result = libsyscall::syscall3(
        SYS_VM_SET_MEMORY,
        vm_id as u64,
        0,
        (&memory_desc as *const MemRegionDesc) as u64,
    ) as u32;
    if map_result == u32::MAX {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(AsldError::BackendUnavailable("vm_set_memory failed"));
    }

    let vcpu_id = 0u32;
    let create_vcpu_result =
        libsyscall::syscall2(SYS_VCPU_CREATE, vm_id as u64, vcpu_id as u64) as u32;
    if create_vcpu_result == u32::MAX {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(AsldError::BackendUnavailable("vcpu_create failed"));
    }

    let console_pipe_name = format!("asl-console-{}", config.name);
    if let Err(err) = ensure_pipe(&console_pipe_name) {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(err);
    }

    Ok(VmInstance {
        vm_id,
        vcpu_id,
        backend: if hw == 1 {
            String::from("kernel-vmx")
        } else {
            String::from("kernel-svm")
        },
        console_pipe_name,
        guest_memory_addr: guest_memory as usize,
        guest_memory_size,
    })
}

#[cfg(not(target_os = "linux"))]
fn stop_vm_impl(instance: &VmInstance) -> Result<(), AsldError> {
    const SYS_VM_DESTROY: u32 = 601;
    if instance.guest_memory_addr != 0 && instance.guest_memory_size != 0 {
        let _ = anyos_std::process::munmap(instance.guest_memory_addr as *mut u8, instance.guest_memory_size);
    }
    let rc = libsyscall::syscall1(SYS_VM_DESTROY, instance.vm_id as u64) as u32;
    if rc == u32::MAX {
        return Err(AsldError::BackendUnavailable("vm_destroy failed"));
    }
    Ok(())
}

fn align_guest_memory_size(memory_mb: u32) -> usize {
    const MIN_MB: usize = 16;
    const PAGE_SIZE: usize = 0x1000;
    let requested = (memory_mb as usize).max(MIN_MB) * 1024 * 1024;
    (requested + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1)
}

fn ensure_pipe(pipe_name: &str) -> Result<(), AsldError> {
    let existing = anyos_std::ipc::pipe_open(pipe_name);
    if existing != 0 && existing != u32::MAX {
        let _ = anyos_std::ipc::pipe_close(existing);
    }
    let created = anyos_std::ipc::pipe_create(pipe_name);
    if created == 0 || created == u32::MAX {
        return Err(AsldError::BackendUnavailable("console pipe provisioning failed"));
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
        assert!(instance.guest_memory_size >= 16 * 1024 * 1024);
        stop_vm(&instance).unwrap();
    }
}
