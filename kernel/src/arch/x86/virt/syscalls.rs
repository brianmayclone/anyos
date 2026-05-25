//! Syscall handlers for hardware virtualization (sys_vm_* / sys_vcpu_*).
//!
//! These are called from the syscall dispatch table and delegate to the
//! appropriate VMX or SVM backend based on detected hardware.

use super::{
    CpuidEntry, DirtyLogRequest, GuestFpuState, GuestGprs, GuestSregs, TranslateRequest,
    VcpuMpState, VirtType, VmExitInfo,
};

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
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Set a memory region for a VM.
/// arg1=vm_id, arg2=slot, arg3=ptr to {gpa: u64, size: u64, uva: u64} struct.
///
/// The `host_phys` field in the descriptor is actually a user virtual address.
/// We translate it page-by-page to real physical addresses using the current
/// process's page tables before programming the NPT/EPT entries.
pub fn sys_vm_set_memory(vm_id: u32, slot: u32, desc_ptr: u64) -> u32 {
    let desc = desc_ptr as *const MemRegionDesc;
    let (gpa, size, uva) = unsafe {
        if desc.is_null() {
            return u32::MAX;
        }
        let d = &*desc;
        (d.guest_phys, d.size, d.host_phys)
    };

    if size == 0 {
        return u32::MAX;
    }

    const PAGE_SIZE: u64 = 0x1000;
    if (gpa & (PAGE_SIZE - 1)) != 0 || (uva & (PAGE_SIZE - 1)) != 0 || (size & (PAGE_SIZE - 1)) != 0
    {
        return u32::MAX;
    }
    if gpa.checked_add(size).is_none() || uva.checked_add(size).is_none() {
        return u32::MAX;
    }

    let registered = match super::virt_type() {
        super::VirtType::Vmx => super::vmx::register_memory_region(vm_id, slot, gpa, size, 0),
        super::VirtType::Svm => super::svm::register_memory_region(vm_id, slot, gpa, size, 0),
        super::VirtType::None => false,
    };
    if !registered {
        return u32::MAX;
    }

    // mmap_large() may back guest RAM with non-contiguous host frames. Translate
    // and map each page independently, while the full slot remains registered
    // above for dirty logging and KVM-style memory-slot accounting.
    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
    for i in 0..pages {
        let page_uva = uva + i * PAGE_SIZE;
        let page_gpa = gpa + i * PAGE_SIZE;

        // Translate user virtual address → physical address via recursive mapping.
        let page_hpa = match crate::memory::virtual_mem::virt_to_phys(
            crate::memory::address::VirtAddr::new(page_uva),
        ) {
            Some(p) => p & !0xFFF, // page-aligned physical address
            None => {
                crate::serial_println!(
                    "[virt] sys_vm_set_memory: uva {:#x} not mapped (page {})",
                    page_uva,
                    i
                );
                return u32::MAX;
            }
        };

        let mapped = match super::virt_type() {
            super::VirtType::Vmx => super::vmx::map_page_in_ept(vm_id, page_gpa, page_hpa),
            super::VirtType::Svm => super::svm::map_page_in_npt(vm_id, page_gpa, page_hpa),
            super::VirtType::None => false,
        };
        if !mapped {
            return u32::MAX;
        }
    }

    0
}

/// Create a vCPU within a VM.
pub fn sys_vcpu_create(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::create_vcpu(vm_id, vcpu_id),
        VirtType::Svm => super::svm::create_vcpu(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Run a vCPU until a VM-exit occurs. Copies exit info to user pointer.
pub fn sys_vcpu_run(vm_id: u32, vcpu_id: u32, exit_info_ptr: u64) -> u32 {
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
pub fn sys_vcpu_get_regs(vm_id: u32, vcpu_id: u32, regs_ptr: u64) -> u32 {
    let gprs = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_regs(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_regs(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match gprs {
        Some(g) => {
            if regs_ptr != 0 {
                unsafe {
                    *(regs_ptr as *mut GuestGprs) = g;
                }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Set guest GPRs from user pointer.
pub fn sys_vcpu_set_regs(vm_id: u32, vcpu_id: u32, regs_ptr: u64) -> u32 {
    if regs_ptr == 0 {
        return u32::MAX;
    }
    let gprs = unsafe { &*(regs_ptr as *const GuestGprs) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_regs(vm_id, vcpu_id, gprs),
        VirtType::Svm => super::svm::set_regs(vm_id, vcpu_id, gprs),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Get guest segment/control registers. Copies to user pointer.
pub fn sys_vcpu_get_sregs(vm_id: u32, vcpu_id: u32, sregs_ptr: u64) -> u32 {
    let sregs = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_sregs(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_sregs(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match sregs {
        Some(s) => {
            if sregs_ptr != 0 {
                unsafe {
                    *(sregs_ptr as *mut GuestSregs) = s;
                }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Set guest segment/control registers from user pointer.
pub fn sys_vcpu_set_sregs(vm_id: u32, vcpu_id: u32, sregs_ptr: u64) -> u32 {
    if sregs_ptr == 0 {
        return u32::MAX;
    }
    let sregs = unsafe { &*(sregs_ptr as *const GuestSregs) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_sregs(vm_id, vcpu_id, sregs),
        VirtType::Svm => super::svm::set_sregs(vm_id, vcpu_id, sregs),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Inject an external interrupt into a vCPU.
pub fn sys_vcpu_inject_irq(vm_id: u32, vcpu_id: u32, vector: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::inject_irq(vm_id, vcpu_id, vector as u8),
        VirtType::Svm => super::svm::inject_irq(vm_id, vcpu_id, vector as u8),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
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
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Inject an NMI into a vCPU.
pub fn sys_vcpu_inject_nmi(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::inject_nmi(vm_id, vcpu_id),
        VirtType::Svm => super::svm::inject_nmi(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Set CPUID emulation table for a VM.
/// arg2 = pointer to array of CpuidEntry, arg3 = count.
pub fn sys_vm_set_cpuid(vm_id: u32, entries_ptr: u64, count: u32) -> u32 {
    if entries_ptr == 0 || count == 0 {
        return u32::MAX;
    }
    let entries =
        unsafe { core::slice::from_raw_parts(entries_ptr as *const CpuidEntry, count as usize) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_cpuid(vm_id, entries),
        VirtType::Svm => super::svm::set_cpuid(vm_id, entries),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Query hardware virtualization availability.
/// Returns: 0 = none, 1 = Intel VT-x (VMX), 2 = AMD-V (SVM).
pub fn sys_vm_hw_info() -> u32 {
    match super::virt_type() {
        super::VirtType::Vmx => 1,
        super::VirtType::Svm => 2,
        super::VirtType::None => 0,
    }
}

/// Get dirty-page log for a memory slot.
/// arg1=vm_id, arg2=ptr to DirtyLogRequest { slot, _pad, bitmap_ptr, bitmap_size }.
pub fn sys_vm_get_dirty_log(vm_id: u32, req_ptr: u64) -> u32 {
    if req_ptr == 0 {
        return u32::MAX;
    }
    let req = unsafe { &*(req_ptr as *const DirtyLogRequest) };
    if req.bitmap_ptr == 0 || req.bitmap_size == 0 {
        return u32::MAX;
    }

    // bitmap_size is in bytes; we work in u64 words (8 bytes each)
    let words = (req.bitmap_size / 8) as usize;
    if words == 0 {
        return u32::MAX;
    }
    let bitmap = unsafe { core::slice::from_raw_parts_mut(req.bitmap_ptr as *mut u64, words) };

    let result = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_dirty_log(vm_id, req.slot, bitmap),
        VirtType::Svm => super::svm::get_dirty_log(vm_id, req.slot, bitmap),
        VirtType::None => None,
    };
    match result {
        Some(_) => 0,
        None => u32::MAX,
    }
}

/// Pause a vCPU.
pub fn sys_vcpu_pause(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::vcpu_pause(vm_id, vcpu_id),
        VirtType::Svm => super::svm::vcpu_pause(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Resume a paused vCPU.
pub fn sys_vcpu_resume(vm_id: u32, vcpu_id: u32) -> u32 {
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::vcpu_resume(vm_id, vcpu_id),
        VirtType::Svm => super::svm::vcpu_resume(vm_id, vcpu_id),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Get guest FPU/SSE/AVX state. Copies to user pointer (512 bytes, FXSAVE layout).
pub fn sys_vcpu_get_fpu(vm_id: u32, vcpu_id: u32, fpu_ptr: u64) -> u32 {
    let fpu = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_fpu(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_fpu(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match fpu {
        Some(f) => {
            if fpu_ptr != 0 {
                unsafe {
                    *(fpu_ptr as *mut GuestFpuState) = f;
                }
            }
            0
        }
        None => u32::MAX,
    }
}

/// Set guest FPU/SSE/AVX state from user pointer.
pub fn sys_vcpu_set_fpu(vm_id: u32, vcpu_id: u32, fpu_ptr: u64) -> u32 {
    if fpu_ptr == 0 {
        return u32::MAX;
    }
    let fpu = unsafe { &*(fpu_ptr as *const GuestFpuState) };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_fpu(vm_id, vcpu_id, fpu),
        VirtType::Svm => super::svm::set_fpu(vm_id, vcpu_id, fpu),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Get vCPU multi-processor state. Returns VcpuMpState value or u32::MAX on error.
pub fn sys_vcpu_get_mp_state(vm_id: u32, vcpu_id: u32) -> u32 {
    let state = match super::virt_type() {
        VirtType::Vmx => super::vmx::get_mp_state(vm_id, vcpu_id),
        VirtType::Svm => super::svm::get_mp_state(vm_id, vcpu_id),
        VirtType::None => None,
    };
    match state {
        Some(s) => s as u32,
        None => u32::MAX,
    }
}

/// Set vCPU multi-processor state.
pub fn sys_vcpu_set_mp_state(vm_id: u32, vcpu_id: u32, state_val: u32) -> u32 {
    let state = match VcpuMpState::from_u32(state_val) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let ok = match super::virt_type() {
        VirtType::Vmx => super::vmx::set_mp_state(vm_id, vcpu_id, state),
        VirtType::Svm => super::svm::set_mp_state(vm_id, vcpu_id, state),
        VirtType::None => false,
    };
    if ok {
        0
    } else {
        u32::MAX
    }
}

/// Translate guest virtual address to guest physical address.
/// arg3 = ptr to TranslateRequest { gva, out_gpa, out_valid }.
pub fn sys_vcpu_translate(vm_id: u32, vcpu_id: u32, req_ptr: u64) -> u32 {
    if req_ptr == 0 {
        return u32::MAX;
    }
    let req = unsafe { &mut *(req_ptr as *mut TranslateRequest) };

    let result = match super::virt_type() {
        VirtType::Vmx => super::vmx::translate_gva(vm_id, vcpu_id, req.gva),
        VirtType::Svm => super::svm::translate_gva(vm_id, vcpu_id, req.gva),
        VirtType::None => None,
    };

    match result {
        Some(gpa) => {
            req.out_gpa = gpa;
            req.out_valid = 1;
            0
        }
        None => {
            req.out_gpa = 0;
            req.out_valid = 0;
            0 // Not an error — just not mapped; caller checks out_valid
        }
    }
}

// ── Memory region descriptor (passed from userspace) ─────────────────────

#[repr(C)]
struct MemRegionDesc {
    guest_phys: u64,
    size: u64,
    host_phys: u64,
}
