//! Syscall handlers for hardware virtualization (sys_vm_* / sys_vcpu_*).
//!
//! These are called from the syscall dispatch table and delegate to the
//! appropriate VMX or SVM backend based on detected hardware.

use super::{CpuidEntry, GuestGprs, GuestSregs, VirtType, VmExitInfo};

/// Create a new VM. Returns VM ID or 0 on failure.
pub fn sys_vm_create() -> u32 {
    match super::virt_type() {
        VirtType::Vmx => super::vmx::create_vm(),
        VirtType::Svm => super::svm::create_vm(),
        VirtType::None => 0,
    }
}

/// Destroy a VM.
pub fn sys_vm_destroy(vm_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::destroy_vm(vm_id),
        VirtType::Svm => super::svm::destroy_vm(vm_id),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Set a memory region for a VM.
/// arg1=vm_id, arg2=slot, arg3=ptr to {gpa: u64, size: u64, uva: u64} struct.
pub fn sys_vm_set_memory(vm_id: u32, slot: u32, desc_ptr: u32) -> u32 {
    // Read the descriptor from userspace
    let desc = desc_ptr as *const MemRegionDesc;
    let (gpa, size, hpa) = unsafe {
        if desc.is_null() {
            return u32::MAX;
        }
        let d = &*desc;
        // The userspace passes a user virtual address which, for anyOS,
        // maps to a physical address. We use it directly as host physical.
        (d.guest_phys, d.size, d.host_phys)
    };

    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_memory(vm_id, slot, gpa, size, hpa),
        VirtType::Svm => super::svm::set_memory(vm_id, slot, gpa, size, hpa),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Create a vCPU within a VM.
pub fn sys_vcpu_create(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::create_vcpu(vm_id, vcpu_id),
        VirtType::Svm => super::svm::create_vcpu(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Run a vCPU until a VM-exit occurs. Copies exit info to user pointer.
pub fn sys_vcpu_run(vm_id: u32, vcpu_id: u32, exit_info_ptr: u32) -> u32 {
    let exit = match super::virt_type() {
        VirtType::Vmx => super::vmx::vcpu_run(vm_id, vcpu_id),
        VirtType::Svm => super::svm::vcpu_run(vm_id, vcpu_id),
        VirtType::None => None,
    };

    match exit {
        Some(info) => {
            if exit_info_ptr != 0 {
                unsafe {
                    let ptr = exit_info_ptr as *mut VmExitInfo;
                    *ptr = info;
                }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Get guest GPRs. Copies to user pointer.
pub fn sys_vcpu_get_regs(vm_id: u32, vcpu_id: u32, regs_ptr: u32) -> u32 {
    let gprs = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_regs(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_regs(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match gprs {
        Some(g) => {
            if regs_ptr != 0 {
                unsafe { *(regs_ptr as *mut GuestGprs) = g; }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Set guest GPRs from user pointer.
pub fn sys_vcpu_set_regs(vm_id: u32, vcpu_id: u32, regs_ptr: u32) -> u32 {
    if regs_ptr == 0 {
        return u32::MAX;
    }
    let gprs = unsafe { &*(regs_ptr as *const GuestGprs) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_regs(vm_id, vcpu_id, gprs),
        VirtType::Svm => super::svm::set_regs(vm_id, vcpu_id, gprs),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Get guest segment/control registers. Copies to user pointer.
pub fn sys_vcpu_get_sregs(vm_id: u32, vcpu_id: u32, sregs_ptr: u32) -> u32 {
    let sregs = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_sregs(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_sregs(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match sregs {
        Some(s) => {
            if sregs_ptr != 0 {
                unsafe { *(sregs_ptr as *mut GuestSregs) = s; }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Set guest segment/control registers from user pointer.
pub fn sys_vcpu_set_sregs(vm_id: u32, vcpu_id: u32, sregs_ptr: u32) -> u32 {
    if sregs_ptr == 0 {
        return u32::MAX;
    }
    let sregs = unsafe { &*(sregs_ptr as *const GuestSregs) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_sregs(vm_id, vcpu_id, sregs),
        VirtType::Svm => super::svm::set_sregs(vm_id, vcpu_id, sregs),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Inject an external interrupt into a vCPU.
pub fn sys_vcpu_inject_irq(vm_id: u32, vcpu_id: u32, vector: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::inject_irq(vm_id, vcpu_id, vector as u8),
        VirtType::Svm => super::svm::inject_irq(vm_id, vcpu_id, vector as u8),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Inject an exception into a vCPU.
/// arg3 encodes vector (low 8 bits) and error code (bits 8-31 shifted, or separate arg).
pub fn sys_vcpu_inject_exception(vm_id: u32, vcpu_id: u32, info: u32) -> u32 {
    let vector = (info & 0xFF) as u8;
    let error_code = info >> 8;
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::inject_exception(vm_id, vcpu_id, vector, error_code),
        VirtType::Svm => super::svm::inject_exception(vm_id, vcpu_id, vector, error_code),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Inject an NMI into a vCPU.
pub fn sys_vcpu_inject_nmi(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::inject_nmi(vm_id, vcpu_id),
        VirtType::Svm => super::svm::inject_nmi(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

/// Set CPUID emulation table for a VM.
/// arg2 = pointer to array of CpuidEntry, arg3 = count.
pub fn sys_vm_set_cpuid(vm_id: u32, entries_ptr: u32, count: u32) -> u32 {
    if entries_ptr == 0 || count == 0 {
        return u32::MAX;
    }
    let entries = unsafe {
        core::slice::from_raw_parts(entries_ptr as *const CpuidEntry, count as usize)
    };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_cpuid(vm_id, entries),
        VirtType::Svm => super::svm::set_cpuid(vm_id, entries),
        VirtType::None => false,
    };
    if ok { 0 } else { u32::MAX }
}

// ── Memory region descriptor (passed from userspace) ─────────────────────

#[repr(C)]
struct MemRegionDesc {
    guest_phys: u64,
    size: u64,
    host_phys: u64,
}
