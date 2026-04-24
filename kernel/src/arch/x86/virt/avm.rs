//! AVM (anyOS Virtual Machine) KVM-style ABI.
//!
//! AVM exposes one ioctl-like syscall. Userspace talks to a system handle,
//! VM handles, and vCPU handles. VMX/SVM remain raw backend internals.

use super::{CpuidEntry, DirtyLogRequest, GuestFpuState, GuestGprs, GuestSregs, TranslateRequest};

pub const AVM_API_VERSION: u32 = 1;
pub const AVM_SYSTEM_HANDLE: u64 = 0;

const HANDLE_KIND_SHIFT: u64 = 60;
const HANDLE_KIND_MASK: u64 = 0xF << HANDLE_KIND_SHIFT;
const HANDLE_KIND_VM: u64 = 1;
const HANDLE_KIND_VCPU: u64 = 2;
const HANDLE_ID_MASK: u64 = 0x0FFF_FFFF;

pub const AVM_EXT_DIRTY_LOG: u32 = 1;
pub const AVM_EXT_MP_STATE: u32 = 2;
pub const AVM_EXT_GVA_TRANSLATE: u32 = 3;
pub const AVM_EXT_FPU_STATE: u32 = 4;
pub const AVM_EXT_IRQ_INJECTION: u32 = 5;

pub const AVMIO_GET_API_VERSION: u32 = 0xAE00;
pub const AVMIO_CHECK_EXTENSION: u32 = 0xAE01;
pub const AVMIO_GET_BACKEND_INFO: u32 = 0xAE02;
pub const AVMIO_CREATE_VM: u32 = 0xAE03;

pub const AVMIO_SET_USER_MEMORY_REGION: u32 = 0xAE40;
pub const AVMIO_CREATE_VCPU: u32 = 0xAE41;
pub const AVMIO_SET_CPUID2: u32 = 0xAE42;
pub const AVMIO_GET_DIRTY_LOG: u32 = 0xAE43;
pub const AVMIO_DESTROY_VM: u32 = 0xAE44;

pub const AVMIO_RUN: u32 = 0xAE80;
pub const AVMIO_GET_REGS: u32 = 0xAE81;
pub const AVMIO_SET_REGS: u32 = 0xAE82;
pub const AVMIO_GET_SREGS: u32 = 0xAE83;
pub const AVMIO_SET_SREGS: u32 = 0xAE84;
pub const AVMIO_PAUSE: u32 = 0xAE85;
pub const AVMIO_RESUME: u32 = 0xAE86;
pub const AVMIO_GET_FPU: u32 = 0xAE87;
pub const AVMIO_SET_FPU: u32 = 0xAE88;
pub const AVMIO_GET_MP_STATE: u32 = 0xAE89;
pub const AVMIO_SET_MP_STATE: u32 = 0xAE8A;
pub const AVMIO_TRANSLATE: u32 = 0xAE8B;
pub const AVMIO_INJECT_IRQ: u32 = 0xAE8C;
pub const AVMIO_INJECT_EXCEPTION: u32 = 0xAE8D;
pub const AVMIO_INJECT_NMI: u32 = 0xAE8E;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AvmBackendInfo {
    pub api_version: u32,
    pub backend_kind: u32,
    pub feature_bits: u64,
    pub max_vcpus: u32,
    pub exit_info_size: u32,
    pub regs_size: u32,
    pub sregs_size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AvmUserspaceMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

#[repr(C)]
struct RawMemoryRegion {
    guest_phys: u64,
    size: u64,
    host_phys: u64,
}

pub fn sys_avm_ioctl(handle: u64, request: u32, arg: u64, flags: u64) -> u64 {
    match request {
        AVMIO_GET_API_VERSION if handle == AVM_SYSTEM_HANDLE => AVM_API_VERSION as u64,
        AVMIO_CHECK_EXTENSION if handle == AVM_SYSTEM_HANDLE => check_extension(arg as u32) as u64,
        AVMIO_GET_BACKEND_INFO if handle == AVM_SYSTEM_HANDLE => write_backend_info(arg),
        AVMIO_CREATE_VM if handle == AVM_SYSTEM_HANDLE => create_vm(flags),
        AVMIO_SET_USER_MEMORY_REGION => with_vm(handle, |vm_id| set_user_memory_region(vm_id, arg)),
        AVMIO_CREATE_VCPU => with_vm(handle, |vm_id| create_vcpu(vm_id, arg as u32)),
        AVMIO_SET_CPUID2 => with_vm(handle, |vm_id| set_cpuid(vm_id, arg, flags as u32)),
        AVMIO_GET_DIRTY_LOG => with_vm(handle, |vm_id| {
            super::syscalls::sys_vm_get_dirty_log(vm_id, arg) as u64
        }),
        AVMIO_DESTROY_VM => with_vm(handle, |vm_id| {
            super::syscalls::sys_vm_destroy(vm_id) as u64
        }),
        AVMIO_RUN => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_run(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_GET_REGS => with_vcpu(handle, |vm_id, vcpu_id| {
            let _ = core::mem::size_of::<GuestGprs>();
            super::syscalls::sys_vcpu_get_regs(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_SET_REGS => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_set_regs(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_GET_SREGS => with_vcpu(handle, |vm_id, vcpu_id| {
            let _ = core::mem::size_of::<GuestSregs>();
            super::syscalls::sys_vcpu_get_sregs(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_SET_SREGS => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_set_sregs(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_PAUSE => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_pause(vm_id, vcpu_id) as u64
        }),
        AVMIO_RESUME => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_resume(vm_id, vcpu_id) as u64
        }),
        AVMIO_GET_FPU => with_vcpu(handle, |vm_id, vcpu_id| {
            let _ = core::mem::size_of::<GuestFpuState>();
            super::syscalls::sys_vcpu_get_fpu(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_SET_FPU => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_set_fpu(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_GET_MP_STATE => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_get_mp_state(vm_id, vcpu_id) as u64
        }),
        AVMIO_SET_MP_STATE => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_set_mp_state(vm_id, vcpu_id, arg as u32) as u64
        }),
        AVMIO_TRANSLATE => with_vcpu(handle, |vm_id, vcpu_id| {
            let _ = core::mem::size_of::<TranslateRequest>();
            super::syscalls::sys_vcpu_translate(vm_id, vcpu_id, arg) as u64
        }),
        AVMIO_INJECT_IRQ => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_inject_irq(vm_id, vcpu_id, arg as u32) as u64
        }),
        AVMIO_INJECT_EXCEPTION => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_inject_exception(vm_id, vcpu_id, arg as u32) as u64
        }),
        AVMIO_INJECT_NMI => with_vcpu(handle, |vm_id, vcpu_id| {
            super::syscalls::sys_vcpu_inject_nmi(vm_id, vcpu_id) as u64
        }),
        _ => u64::MAX,
    }
}

fn create_vm(flags: u64) -> u64 {
    if flags != 0 {
        return u64::MAX;
    }
    let vm_id = super::syscalls::sys_vm_create();
    if vm_id == 0 || vm_id == u32::MAX {
        u64::MAX
    } else {
        encode_vm_handle(vm_id)
    }
}

fn create_vcpu(vm_id: u32, vcpu_id: u32) -> u64 {
    let rc = super::syscalls::sys_vcpu_create(vm_id, vcpu_id);
    if rc == 0 {
        encode_vcpu_handle(vm_id, vcpu_id)
    } else {
        u64::MAX
    }
}

fn set_user_memory_region(vm_id: u32, arg: u64) -> u64 {
    if arg == 0 {
        return u64::MAX;
    }
    let region = unsafe { &*(arg as *const AvmUserspaceMemoryRegion) };
    if region.memory_size == 0 || region.userspace_addr == 0 {
        return u64::MAX;
    }
    let raw = RawMemoryRegion {
        guest_phys: region.guest_phys_addr,
        size: region.memory_size,
        host_phys: region.userspace_addr,
    };
    super::syscalls::sys_vm_set_memory(vm_id, region.slot, (&raw as *const RawMemoryRegion) as u64)
        as u64
}

fn set_cpuid(vm_id: u32, arg: u64, count: u32) -> u64 {
    let _ = core::mem::size_of::<CpuidEntry>();
    super::syscalls::sys_vm_set_cpuid(vm_id, arg, count) as u64
}

fn write_backend_info(arg: u64) -> u64 {
    if arg == 0 {
        return u64::MAX;
    }
    let backend_kind = match super::virt_type() {
        super::VirtType::None => 0,
        super::VirtType::Vmx => 1,
        super::VirtType::Svm => 2,
    };
    let info = AvmBackendInfo {
        api_version: AVM_API_VERSION,
        backend_kind,
        feature_bits: feature_bits(),
        max_vcpus: 64,
        exit_info_size: core::mem::size_of::<super::VmExitInfo>() as u32,
        regs_size: core::mem::size_of::<GuestGprs>() as u32,
        sregs_size: core::mem::size_of::<GuestSregs>() as u32,
    };
    unsafe {
        *(arg as *mut AvmBackendInfo) = info;
    }
    0
}

fn feature_bits() -> u64 {
    (1u64 << AVM_EXT_DIRTY_LOG)
        | (1u64 << AVM_EXT_MP_STATE)
        | (1u64 << AVM_EXT_GVA_TRANSLATE)
        | (1u64 << AVM_EXT_FPU_STATE)
        | (1u64 << AVM_EXT_IRQ_INJECTION)
}

fn check_extension(ext: u32) -> u32 {
    match ext {
        AVM_EXT_DIRTY_LOG
        | AVM_EXT_MP_STATE
        | AVM_EXT_GVA_TRANSLATE
        | AVM_EXT_FPU_STATE
        | AVM_EXT_IRQ_INJECTION => 1,
        _ => 0,
    }
}

fn with_vm<F>(handle: u64, f: F) -> u64
where
    F: FnOnce(u32) -> u64,
{
    match decode_vm_handle(handle) {
        Some(vm_id) => f(vm_id),
        None => u64::MAX,
    }
}

fn with_vcpu<F>(handle: u64, f: F) -> u64
where
    F: FnOnce(u32, u32) -> u64,
{
    match decode_vcpu_handle(handle) {
        Some((vm_id, vcpu_id)) => f(vm_id, vcpu_id),
        None => u64::MAX,
    }
}

pub fn encode_vm_handle(vm_id: u32) -> u64 {
    (HANDLE_KIND_VM << HANDLE_KIND_SHIFT) | (vm_id as u64 & HANDLE_ID_MASK)
}

pub fn encode_vcpu_handle(vm_id: u32, vcpu_id: u32) -> u64 {
    (HANDLE_KIND_VCPU << HANDLE_KIND_SHIFT)
        | ((vm_id as u64 & HANDLE_ID_MASK) << 32)
        | vcpu_id as u64
}

pub fn decode_vm_handle(handle: u64) -> Option<u32> {
    if (handle & HANDLE_KIND_MASK) != (HANDLE_KIND_VM << HANDLE_KIND_SHIFT) {
        return None;
    }
    Some((handle & HANDLE_ID_MASK) as u32)
}

pub fn decode_vcpu_handle(handle: u64) -> Option<(u32, u32)> {
    if (handle & HANDLE_KIND_MASK) != (HANDLE_KIND_VCPU << HANDLE_KIND_SHIFT) {
        return None;
    }
    Some((((handle >> 32) & HANDLE_ID_MASK) as u32, handle as u32))
}
