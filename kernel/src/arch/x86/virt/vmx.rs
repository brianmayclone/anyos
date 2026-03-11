//! Intel VT-x (VMX) support.
//!
//! Implements VMXON per-CPU initialization, VMCS lifecycle, VM-entry/exit,
//! and exit reason handling for the anyOS hypervisor interface.

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    alloc_page_zeroed, free_page, rdmsr, wrmsr, read_cr0, read_cr3, read_cr4, write_cr4,
    CpuidEntry, GuestGprs, GuestSregs, MemoryRegion, VmExitInfo,
};
use crate::sync::spinlock::Spinlock;

// ── VMCS field encodings (Intel SDM Vol. 3D, Appendix B) ─────────────────

// Guest 16-bit selector fields
const GUEST_ES_SELECTOR: u32 = 0x0800;
const GUEST_CS_SELECTOR: u32 = 0x0802;
const GUEST_SS_SELECTOR: u32 = 0x0804;
const GUEST_DS_SELECTOR: u32 = 0x0806;
const GUEST_FS_SELECTOR: u32 = 0x0808;
const GUEST_GS_SELECTOR: u32 = 0x080A;
const GUEST_LDTR_SELECTOR: u32 = 0x080C;
const GUEST_TR_SELECTOR: u32 = 0x080E;

// Guest 32-bit limit fields
const GUEST_ES_LIMIT: u32 = 0x4800;
const GUEST_CS_LIMIT: u32 = 0x4802;
const GUEST_SS_LIMIT: u32 = 0x4804;
const GUEST_DS_LIMIT: u32 = 0x4806;
const GUEST_FS_LIMIT: u32 = 0x4808;
const GUEST_GS_LIMIT: u32 = 0x480A;
const GUEST_LDTR_LIMIT: u32 = 0x480C;
const GUEST_TR_LIMIT: u32 = 0x480E;
const GUEST_GDTR_LIMIT: u32 = 0x4810;
const GUEST_IDTR_LIMIT: u32 = 0x4812;

// Guest 32-bit access rights fields
const GUEST_ES_AR_BYTES: u32 = 0x4814;
const GUEST_CS_AR_BYTES: u32 = 0x4816;
const GUEST_SS_AR_BYTES: u32 = 0x4818;
const GUEST_DS_AR_BYTES: u32 = 0x481A;
const GUEST_FS_AR_BYTES: u32 = 0x481C;
const GUEST_GS_AR_BYTES: u32 = 0x481E;
const GUEST_LDTR_AR_BYTES: u32 = 0x4820;
const GUEST_TR_AR_BYTES: u32 = 0x4822;

// Guest natural-width base fields
const GUEST_ES_BASE: u32 = 0x6806;
const GUEST_CS_BASE: u32 = 0x6808;
const GUEST_SS_BASE: u32 = 0x680A;
const GUEST_DS_BASE: u32 = 0x680C;
const GUEST_FS_BASE: u32 = 0x680E;
const GUEST_GS_BASE: u32 = 0x6810;
const GUEST_LDTR_BASE: u32 = 0x6812;
const GUEST_TR_BASE: u32 = 0x6814;
const GUEST_GDTR_BASE: u32 = 0x6816;
const GUEST_IDTR_BASE: u32 = 0x6818;

// Guest state
const GUEST_CR0: u32 = 0x6800;
const GUEST_CR3: u32 = 0x6802;
const GUEST_CR4: u32 = 0x6804;
const GUEST_RIP: u32 = 0x681E;
const GUEST_RSP: u32 = 0x681C;
const GUEST_RFLAGS: u32 = 0x6820;
const GUEST_IA32_EFER: u32 = 0x2806;
const VMCS_LINK_POINTER: u32 = 0x2800;

// Host state
const HOST_CS_SELECTOR: u32 = 0x0C02;
const HOST_SS_SELECTOR: u32 = 0x0C04;
const HOST_DS_SELECTOR: u32 = 0x0C06;
const HOST_ES_SELECTOR: u32 = 0x0C00;
const HOST_FS_SELECTOR: u32 = 0x0C08;
const HOST_GS_SELECTOR: u32 = 0x0C0A;
const HOST_TR_SELECTOR: u32 = 0x0C0C;
const HOST_CR0: u32 = 0x6C00;
const HOST_CR3: u32 = 0x6C02;
const HOST_CR4: u32 = 0x6C04;
const HOST_RIP: u32 = 0x6C16;
const HOST_RSP: u32 = 0x6C14;
const HOST_IA32_EFER: u32 = 0x2C02;
const HOST_GDTR_BASE: u32 = 0x6C0C;
const HOST_IDTR_BASE: u32 = 0x6C0E;
const HOST_FS_BASE: u32 = 0x6C06;
const HOST_GS_BASE: u32 = 0x6C08;

// Control fields
const PIN_BASED_VM_EXEC_CONTROL: u32 = 0x4000;
const PRIMARY_PROC_BASED_VM_EXEC_CONTROL: u32 = 0x4002;
const SECONDARY_PROC_BASED_VM_EXEC_CONTROL: u32 = 0x401E;
const VM_EXIT_CONTROLS: u32 = 0x400C;
const VM_ENTRY_CONTROLS: u32 = 0x4012;
const EPT_POINTER: u32 = 0x201A;
const MSR_BITMAP_ADDR: u32 = 0x2004;

// Exit information
const VM_EXIT_REASON: u32 = 0x4402;
const EXIT_QUALIFICATION: u32 = 0x6400;
const VM_EXIT_INSTR_LEN: u32 = 0x440C;
const GUEST_PHYSICAL_ADDRESS: u32 = 0x2400;

// VM-entry interrupt injection
const VM_ENTRY_INTR_INFO: u32 = 0x4016;
const VM_ENTRY_EXCEPTION_ERROR_CODE: u32 = 0x4018;
const VM_ENTRY_INSTRUCTION_LEN: u32 = 0x401A;

// Exception bitmap
const EXCEPTION_BITMAP: u32 = 0x4004;

// CR0/CR4 guest/host masks and shadows
const CR0_GUEST_HOST_MASK: u32 = 0x6000;
const CR0_READ_SHADOW: u32 = 0x6004;
const CR4_GUEST_HOST_MASK: u32 = 0x6002;
const CR4_READ_SHADOW: u32 = 0x6006;

// ── VMX exit reasons ─────────────────────────────────────────────────────

pub const EXIT_REASON_EXTERNAL_INTERRUPT: u32 = 1;
pub const EXIT_REASON_TRIPLE_FAULT: u32 = 2;
pub const EXIT_REASON_CPUID: u32 = 10;
pub const EXIT_REASON_HLT: u32 = 12;
pub const EXIT_REASON_VMCALL: u32 = 18;
pub const EXIT_REASON_IO_INSTRUCTION: u32 = 30;
pub const EXIT_REASON_RDMSR: u32 = 31;
pub const EXIT_REASON_WRMSR: u32 = 32;
pub const EXIT_REASON_EPT_VIOLATION: u32 = 48;
pub const EXIT_REASON_EPT_MISCONFIG: u32 = 49;

// ── MSR addresses ────────────────────────────────────────────────────────

const IA32_FEATURE_CONTROL: u32 = 0x3A;
const IA32_VMX_BASIC: u32 = 0x480;
const IA32_VMX_PINBASED_CTLS: u32 = 0x481;
const IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
const IA32_VMX_EXIT_CTLS: u32 = 0x483;
const IA32_VMX_ENTRY_CTLS: u32 = 0x484;
const IA32_VMX_PROCBASED_CTLS2: u32 = 0x48B;
const IA32_VMX_TRUE_PINBASED_CTLS: u32 = 0x48D;
const IA32_VMX_TRUE_PROCBASED_CTLS: u32 = 0x48E;
const IA32_VMX_TRUE_EXIT_CTLS: u32 = 0x48F;
const IA32_VMX_TRUE_ENTRY_CTLS: u32 = 0x490;
const IA32_EFER: u32 = 0xC0000080;

// ── Per-CPU VMXON state ──────────────────────────────────────────────────

const MAX_CPUS: usize = 64;
static mut VMXON_REGIONS: [u64; MAX_CPUS] = [0; MAX_CPUS];
static mut VMXON_ACTIVE: [bool; MAX_CPUS] = [false; MAX_CPUS];

// ── VM storage ───────────────────────────────────────────────────────────

static NEXT_VM_ID: AtomicU32 = AtomicU32::new(1);
const MAX_VMS: usize = 64;
const MAX_VCPUS_PER_VM: usize = 16;

struct VmxVcpu {
    vmcs_phys: u64,
    guest_gprs: GuestGprs,
    launched: bool,
    msr_bitmap_phys: u64,
}

struct VmxVm {
    id: u32,
    active: bool,
    vcpus: [Option<VmxVcpu>; MAX_VCPUS_PER_VM],
    ept_root: u64,
    memory_regions: [Option<MemoryRegion>; 32],
    cpuid_table: [Option<CpuidEntry>; 64],
    cpuid_count: usize,
}

// We use a simple static array of VMs protected by a spinlock.
// For VM-entry/exit hot path, the lock is released before VMLAUNCH.
static VMS: Spinlock<[Option<VmxVm>; MAX_VMS]> = Spinlock::new([const { None }; MAX_VMS]);

// ── VMX instruction wrappers ─────────────────────────────────────────────

#[inline]
unsafe fn vmxon(phys_addr: u64) -> bool {
    let success: u8;
    core::arch::asm!(
        "vmxon [{}]",
        in(reg) &phys_addr,
        options(nostack),
    );
    // If we reach here without fault, VMXON succeeded
    true
}

#[inline]
unsafe fn vmclear(phys_addr: u64) {
    core::arch::asm!(
        "vmclear [{}]",
        in(reg) &phys_addr,
        options(nostack),
    );
}

#[inline]
unsafe fn vmptrld(phys_addr: u64) {
    core::arch::asm!(
        "vmptrld [{}]",
        in(reg) &phys_addr,
        options(nostack),
    );
}

#[inline]
unsafe fn vmwrite(field: u32, value: u64) {
    core::arch::asm!(
        "vmwrite {}, {}",
        in(reg) field as u64,
        in(reg) value,
        options(nostack),
    );
}

#[inline]
unsafe fn vmread(field: u32) -> u64 {
    let value: u64;
    core::arch::asm!(
        "vmread {}, {}",
        out(reg) value,
        in(reg) field as u64,
        options(nostack),
    );
    value
}

/// Adjust a VMX control field value according to the allowed-0/allowed-1 MSR.
#[inline]
unsafe fn adjust_control(value: u32, msr: u32) -> u32 {
    let cap = rdmsr(msr);
    let allowed_0 = cap as u32;          // bits that MUST be 1
    let allowed_1 = (cap >> 32) as u32;  // bits that CAN be 1
    (value | allowed_0) & allowed_1
}

// ── Global init ──────────────────────────────────────────────────────────

pub fn global_init() {
    // Nothing needed globally; per-CPU init does VMXON.
}

// ── Per-CPU init ─────────────────────────────────────────────────────────

pub fn per_cpu_init() {
    let cpu_id = crate::arch::x86::apic::lapic_id() as usize;
    if cpu_id >= MAX_CPUS {
        return;
    }

    unsafe {
        // Check IA32_FEATURE_CONTROL
        let feat_ctrl = rdmsr(IA32_FEATURE_CONTROL);
        if feat_ctrl & 1 == 0 {
            // Not locked — set lock bit + VMXON-outside-SMX
            wrmsr(IA32_FEATURE_CONTROL, feat_ctrl | 1 | (1 << 2));
        } else if feat_ctrl & (1 << 2) == 0 {
            // Locked but VMXON-outside-SMX not enabled
            crate::serial_println!("VMX: CPU {} — VMXON locked out by BIOS", cpu_id);
            return;
        }

        // Enable VMX in CR4
        let cr4 = read_cr4();
        write_cr4(cr4 | (1 << 13));

        // Allocate VMXON region (4KB aligned, zeroed)
        let page = match alloc_page_zeroed() {
            Some(p) => p,
            None => {
                crate::serial_println!("VMX: CPU {} — failed to allocate VMXON region", cpu_id);
                return;
            }
        };

        // Write VMCS revision ID to first 4 bytes
        let vmx_basic = rdmsr(IA32_VMX_BASIC);
        let revision = (vmx_basic & 0x7FFF_FFFF) as u32;
        *(page as *mut u32) = revision;

        // Execute VMXON
        vmxon(page);

        VMXON_REGIONS[cpu_id] = page;
        VMXON_ACTIVE[cpu_id] = true;

        crate::serial_verbose_println!("VMX: CPU {} — VMXON enabled (rev={:#x})", cpu_id, revision);
    }
}

// ── VM lifecycle ─────────────────────────────────────────────────────────

/// Create a new VM. Returns the VM ID or 0 on failure.
pub fn create_vm() -> u32 {
    let id = NEXT_VM_ID.fetch_add(1, Ordering::Relaxed);

    // Create EPT root
    let ept_root = match super::ept::create_ept_root() {
        Some(r) => r,
        None => return 0,
    };

    let vm = VmxVm {
        id,
        active: true,
        vcpus: [const { None }; MAX_VCPUS_PER_VM],
        ept_root,
        memory_regions: [const { None }; 32],
        cpuid_table: [const { None }; 64],
        cpuid_count: 0,
    };

    let mut vms = VMS.lock();
    for slot in vms.iter_mut() {
        if slot.is_none() {
            *slot = Some(vm);
            return id;
        }
    }

    // No free slot
    0
}

/// Destroy a VM, freeing all resources.
pub fn destroy_vm(vm_id: u32) -> bool {
    let mut vms = VMS.lock();
    for slot in vms.iter_mut() {
        if let Some(vm) = slot {
            if vm.id == vm_id {
                // Free vCPU resources
                for vcpu_opt in vm.vcpus.iter_mut() {
                    if let Some(vcpu) = vcpu_opt.take() {
                        unsafe { vmclear(vcpu.vmcs_phys); }
                        free_page(vcpu.vmcs_phys);
                        if vcpu.msr_bitmap_phys != 0 {
                            free_page(vcpu.msr_bitmap_phys);
                        }
                    }
                }
                // Free EPT
                super::ept::destroy_ept(vm.ept_root);
                *slot = None;
                return true;
            }
        }
    }
    false
}

/// Set a memory region mapping guest physical addresses to host physical addresses.
pub fn set_memory(vm_id: u32, slot: u32, gpa: u64, size: u64, hpa: u64) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(vm) => vm,
        None => return false,
    };

    let region = MemoryRegion {
        slot,
        guest_phys: gpa,
        size,
        host_phys: hpa,
    };

    // Store region
    let slot_idx = slot as usize;
    if slot_idx >= vm.memory_regions.len() {
        return false;
    }
    vm.memory_regions[slot_idx] = Some(region);

    // Map in EPT
    super::ept::ept_map_range(vm.ept_root, gpa, hpa, size, true, true);

    true
}

/// Set CPUID table for a VM.
pub fn set_cpuid(vm_id: u32, entries: &[CpuidEntry]) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(vm) => vm,
        None => return false,
    };

    let count = entries.len().min(vm.cpuid_table.len());
    for i in 0..count {
        vm.cpuid_table[i] = Some(entries[i]);
    }
    vm.cpuid_count = count;
    true
}

/// Create a vCPU within a VM. Returns true on success.
pub fn create_vcpu(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(vm) => vm,
        None => return false,
    };

    let idx = vcpu_id as usize;
    if idx >= MAX_VCPUS_PER_VM || vm.vcpus[idx].is_some() {
        return false;
    }

    unsafe {
        // Allocate VMCS region
        let vmcs_phys = match alloc_page_zeroed() {
            Some(p) => p,
            None => return false,
        };

        // Allocate MSR bitmap (all 1s = exit on all MSR access)
        let msr_bitmap_phys = match alloc_page_zeroed() {
            Some(p) => p,
            None => {
                free_page(vmcs_phys);
                return false;
            }
        };
        // Fill MSR bitmap with 1s (intercept all MSRs)
        core::ptr::write_bytes(msr_bitmap_phys as *mut u8, 0xFF, 4096);

        // Write revision ID
        let vmx_basic = rdmsr(IA32_VMX_BASIC);
        let revision = (vmx_basic & 0x7FFF_FFFF) as u32;
        *(vmcs_phys as *mut u32) = revision;

        // VMCLEAR then VMPTRLD
        vmclear(vmcs_phys);
        vmptrld(vmcs_phys);

        // Initialize VMCS fields
        init_vmcs(vm.ept_root, msr_bitmap_phys);

        vm.vcpus[idx] = Some(VmxVcpu {
            vmcs_phys,
            guest_gprs: GuestGprs::default(),
            launched: false,
            msr_bitmap_phys,
        });
    }

    true
}

/// Initialize all VMCS fields for a new vCPU.
unsafe fn init_vmcs(ept_root: u64, msr_bitmap_phys: u64) {
    // Check for TRUE controls support
    let vmx_basic = rdmsr(IA32_VMX_BASIC);
    let use_true_ctls = (vmx_basic >> 55) & 1 != 0;

    // Pin-based controls: external-interrupt exiting
    let pin_msr = if use_true_ctls { IA32_VMX_TRUE_PINBASED_CTLS } else { IA32_VMX_PINBASED_CTLS };
    let pin_based = adjust_control(
        1 << 0,  // External-interrupt exiting
        pin_msr,
    );
    vmwrite(PIN_BASED_VM_EXEC_CONTROL, pin_based as u64);

    // Primary proc-based controls
    let proc_msr = if use_true_ctls { IA32_VMX_TRUE_PROCBASED_CTLS } else { IA32_VMX_PROCBASED_CTLS };
    let primary = adjust_control(
        (1 << 7)   // HLT exiting
        | (1 << 24) // Unconditional I/O exiting
        | (1 << 28) // Use MSR bitmaps
        | (1 << 31), // Activate secondary controls
        proc_msr,
    );
    vmwrite(PRIMARY_PROC_BASED_VM_EXEC_CONTROL, primary as u64);

    // Secondary proc-based controls
    let secondary = adjust_control(
        (1 << 1)  // Enable EPT
        | (1 << 7), // Unrestricted guest
        IA32_VMX_PROCBASED_CTLS2,
    );
    vmwrite(SECONDARY_PROC_BASED_VM_EXEC_CONTROL, secondary as u64);

    // VM-exit controls
    let exit_msr = if use_true_ctls { IA32_VMX_TRUE_EXIT_CTLS } else { IA32_VMX_EXIT_CTLS };
    let exit_ctls = adjust_control(
        (1 << 9)   // Host address-space size (64-bit host)
        | (1 << 20) // Save IA32_EFER
        | (1 << 21), // Load IA32_EFER
        exit_msr,
    );
    vmwrite(VM_EXIT_CONTROLS, exit_ctls as u64);

    // VM-entry controls
    let entry_msr = if use_true_ctls { IA32_VMX_TRUE_ENTRY_CTLS } else { IA32_VMX_ENTRY_CTLS };
    let entry_ctls = adjust_control(
        1 << 15, // Load IA32_EFER
        entry_msr,
    );
    vmwrite(VM_ENTRY_CONTROLS, entry_ctls as u64);

    // EPT pointer: WB memory type (6), 4-level walk (3 << 3)
    let eptp = ept_root | (6) | (3 << 3);
    vmwrite(EPT_POINTER, eptp);

    // MSR bitmap
    vmwrite(MSR_BITMAP_ADDR, msr_bitmap_phys);

    // VMCS link pointer (required, set to -1 = no VMCS shadowing)
    vmwrite(VMCS_LINK_POINTER, 0xFFFF_FFFF_FFFF_FFFF);

    // Exception bitmap: intercept nothing by default
    vmwrite(EXCEPTION_BITMAP, 0);

    // CR0/CR4 guest/host mask and shadow: let guest control all bits
    vmwrite(CR0_GUEST_HOST_MASK, 0);
    vmwrite(CR0_READ_SHADOW, 0);
    vmwrite(CR4_GUEST_HOST_MASK, 0);
    vmwrite(CR4_READ_SHADOW, 0);

    // Guest state: real mode defaults
    vmwrite(GUEST_CS_SELECTOR, 0xF000);
    vmwrite(GUEST_CS_BASE, 0xFFFF_0000);
    vmwrite(GUEST_CS_LIMIT, 0xFFFF);
    vmwrite(GUEST_CS_AR_BYTES, 0x009B); // present, R/W code, accessed
    vmwrite(GUEST_DS_SELECTOR, 0);
    vmwrite(GUEST_DS_BASE, 0);
    vmwrite(GUEST_DS_LIMIT, 0xFFFF);
    vmwrite(GUEST_DS_AR_BYTES, 0x0093);
    vmwrite(GUEST_ES_SELECTOR, 0);
    vmwrite(GUEST_ES_BASE, 0);
    vmwrite(GUEST_ES_LIMIT, 0xFFFF);
    vmwrite(GUEST_ES_AR_BYTES, 0x0093);
    vmwrite(GUEST_FS_SELECTOR, 0);
    vmwrite(GUEST_FS_BASE, 0);
    vmwrite(GUEST_FS_LIMIT, 0xFFFF);
    vmwrite(GUEST_FS_AR_BYTES, 0x0093);
    vmwrite(GUEST_GS_SELECTOR, 0);
    vmwrite(GUEST_GS_BASE, 0);
    vmwrite(GUEST_GS_LIMIT, 0xFFFF);
    vmwrite(GUEST_GS_AR_BYTES, 0x0093);
    vmwrite(GUEST_SS_SELECTOR, 0);
    vmwrite(GUEST_SS_BASE, 0);
    vmwrite(GUEST_SS_LIMIT, 0xFFFF);
    vmwrite(GUEST_SS_AR_BYTES, 0x0093);
    vmwrite(GUEST_TR_SELECTOR, 0);
    vmwrite(GUEST_TR_BASE, 0);
    vmwrite(GUEST_TR_LIMIT, 0xFFFF);
    vmwrite(GUEST_TR_AR_BYTES, 0x008B); // present, busy TSS
    vmwrite(GUEST_LDTR_SELECTOR, 0);
    vmwrite(GUEST_LDTR_BASE, 0);
    vmwrite(GUEST_LDTR_LIMIT, 0xFFFF);
    vmwrite(GUEST_LDTR_AR_BYTES, 0x0082); // present, LDT

    vmwrite(GUEST_GDTR_BASE, 0);
    vmwrite(GUEST_GDTR_LIMIT, 0xFFFF);
    vmwrite(GUEST_IDTR_BASE, 0);
    vmwrite(GUEST_IDTR_LIMIT, 0xFFFF);

    vmwrite(GUEST_CR0, 0x0000_0030); // ET + NE (required for VMX)
    vmwrite(GUEST_CR3, 0);
    vmwrite(GUEST_CR4, 0);
    vmwrite(GUEST_IA32_EFER, 0);
    vmwrite(GUEST_RIP, 0xFFF0); // Reset vector
    vmwrite(GUEST_RSP, 0);
    vmwrite(GUEST_RFLAGS, 0x2); // Reserved bit 1 must be set

    // Host state
    let cs: u16;
    let ss: u16;
    let ds: u16;
    let es: u16;
    let fs: u16;
    let gs: u16;
    let tr: u16;
    core::arch::asm!("mov {:x}, cs", out(reg) cs, options(nostack, nomem));
    core::arch::asm!("mov {:x}, ss", out(reg) ss, options(nostack, nomem));
    core::arch::asm!("mov {:x}, ds", out(reg) ds, options(nostack, nomem));
    core::arch::asm!("mov {:x}, es", out(reg) es, options(nostack, nomem));
    core::arch::asm!("mov {:x}, fs", out(reg) fs, options(nostack, nomem));
    core::arch::asm!("mov {:x}, gs", out(reg) gs, options(nostack, nomem));
    core::arch::asm!("str {:x}", out(reg) tr, options(nostack, nomem));

    vmwrite(HOST_CS_SELECTOR, cs as u64);
    vmwrite(HOST_SS_SELECTOR, ss as u64);
    vmwrite(HOST_DS_SELECTOR, ds as u64);
    vmwrite(HOST_ES_SELECTOR, es as u64);
    vmwrite(HOST_FS_SELECTOR, fs as u64);
    vmwrite(HOST_GS_SELECTOR, gs as u64);
    vmwrite(HOST_TR_SELECTOR, tr as u64);

    vmwrite(HOST_CR0, read_cr0());
    vmwrite(HOST_CR3, read_cr3());
    vmwrite(HOST_CR4, read_cr4());
    vmwrite(HOST_IA32_EFER, rdmsr(IA32_EFER));

    // GDT and IDT base
    let mut gdtr: [u8; 10] = [0; 10];
    let mut idtr: [u8; 10] = [0; 10];
    core::arch::asm!("sgdt [{}]", in(reg) gdtr.as_mut_ptr(), options(nostack));
    core::arch::asm!("sidt [{}]", in(reg) idtr.as_mut_ptr(), options(nostack));
    let gdt_base = u64::from_le_bytes([gdtr[2], gdtr[3], gdtr[4], gdtr[5], gdtr[6], gdtr[7], gdtr[8], gdtr[9]]);
    let idt_base = u64::from_le_bytes([idtr[2], idtr[3], idtr[4], idtr[5], idtr[6], idtr[7], idtr[8], idtr[9]]);
    vmwrite(HOST_GDTR_BASE, gdt_base);
    vmwrite(HOST_IDTR_BASE, idt_base);

    vmwrite(HOST_FS_BASE, rdmsr(0xC0000100)); // IA32_FS_BASE
    vmwrite(HOST_GS_BASE, rdmsr(0xC0000101)); // IA32_GS_BASE

    // HOST_RIP and HOST_RSP are set dynamically before each VM-entry
    vmwrite(HOST_RIP, vmx_vmexit_stub as u64);
    vmwrite(HOST_RSP, 0); // Will be patched before VM-entry
}

// ── VM-entry/exit assembly ───────────────────────────────────────────────

core::arch::global_asm!(
    ".global vmx_vcpu_run",
    "vmx_vcpu_run:",
    // rdi = *mut GuestGprs
    // sil = launched (u8)
    //
    // Save callee-saved host registers
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "push rdi",          // save GuestGprs pointer for vmexit stub

    // Set HOST_RSP to current RSP (after all pushes, pointing at GuestGprs ptr)
    "mov rax, 0x6C14",  // HOST_RSP field encoding
    "mov rdx, rsp",     // current RSP after all pushes
    "vmwrite rax, rdx",

    // Load guest GPRs from struct
    "mov rax, [rdi + 0x00]",
    "mov rbx, [rdi + 0x08]",
    "mov rcx, [rdi + 0x10]",
    "mov rdx, [rdi + 0x18]",
    // rsi has launched flag — save it; load guest rsi last
    "mov rbp, [rdi + 0x30]",
    "mov r8,  [rdi + 0x38]",
    "mov r9,  [rdi + 0x40]",
    "mov r10, [rdi + 0x48]",
    "mov r11, [rdi + 0x50]",
    "mov r12, [rdi + 0x58]",
    "mov r13, [rdi + 0x60]",
    "mov r14, [rdi + 0x68]",
    "mov r15, [rdi + 0x70]",

    // Check launched flag (sil) before clobbering rsi/rdi
    "test sil, sil",

    // Load guest rsi and rdi last
    "mov rsi, [rdi + 0x20]",
    "mov rdi, [rdi + 0x28]",

    // VMLAUNCH or VMRESUME
    "jnz 2f",
    "vmlaunch",
    "jmp 3f",           // VM-entry failed
    "2:",
    "vmresume",
    "jmp 3f",           // VM-entry failed

    // VM-exit handler — HOST_RIP points here
    ".global vmx_vmexit_stub",
    "vmx_vmexit_stub:",
    // On VM-exit, CPU loaded host state from VMCS.
    // RSP = HOST_RSP (set to point at the pushed rdi on the stack).
    // Guest GPRs are in physical registers, need to save them.
    // rdi was guest rdi; the GuestGprs pointer is at [rsp].
    "xchg rdi, [rsp]",  // rdi = GuestGprs ptr, [rsp] = guest rdi

    // Save all guest GPRs to struct
    "mov [rdi + 0x00], rax",
    "mov [rdi + 0x08], rbx",
    "mov [rdi + 0x10], rcx",
    "mov [rdi + 0x18], rdx",
    "mov [rdi + 0x20], rsi",
    // guest rdi is on stack
    "pop rax",
    "mov [rdi + 0x28], rax",
    "mov [rdi + 0x30], rbp",
    "mov [rdi + 0x38], r8",
    "mov [rdi + 0x40], r9",
    "mov [rdi + 0x48], r10",
    "mov [rdi + 0x50], r11",
    "mov [rdi + 0x58], r12",
    "mov [rdi + 0x60], r13",
    "mov [rdi + 0x68], r14",
    "mov [rdi + 0x70], r15",

    // Restore host callee-saved registers
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",

    // Return 0 (success — VM-exit handled)
    "xor eax, eax",
    "ret",

    // VM-entry failure path
    "3:",
    "pop rdi",   // discard GuestGprs pointer
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "mov eax, 1", // return 1 (failure)
    "ret",
);

extern "C" {
    fn vmx_vcpu_run(gprs: *mut GuestGprs, launched: u8) -> u32;
    fn vmx_vmexit_stub();
}

/// Run a vCPU. Returns exit info to the caller.
pub fn vcpu_run(vm_id: u32, vcpu_id: u32) -> Option<VmExitInfo> {
    // We need to grab the VM data, extract what we need, drop the lock,
    // then do the actual VM-entry (which is blocking).

    let mut vms = VMS.lock();
    let vm = find_vm_mut(&mut vms, vm_id)?;
    let idx = vcpu_id as usize;
    let vcpu = vm.vcpus[idx].as_mut()?;

    unsafe {
        // Make this VMCS current
        vmptrld(vcpu.vmcs_phys);

        // HOST_RSP is set inside vmx_vcpu_run assembly, after all pushes,
        // via VMWRITE with the exact RSP value at that point.

        let launched = vcpu.launched;
        let gprs_ptr = &mut vcpu.guest_gprs as *mut GuestGprs;

        // Drop the lock before entering the guest
        drop(vms);

        let result = vmx_vcpu_run(gprs_ptr, launched as u8);

        // Re-acquire lock to update state
        let mut vms = VMS.lock();
        let vm = find_vm_mut(&mut vms, vm_id)?;
        let vcpu = vm.vcpus[idx].as_mut()?;

        if result == 0 {
            vcpu.launched = true;

            // Read exit information from VMCS
            vmptrld(vcpu.vmcs_phys);
            let reason = vmread(VM_EXIT_REASON) as u32 & 0xFFFF; // low 16 bits
            let qualification = vmread(EXIT_QUALIFICATION);
            let instr_len = vmread(VM_EXIT_INSTR_LEN) as u32;
            let guest_phys = if reason == EXIT_REASON_EPT_VIOLATION || reason == EXIT_REASON_EPT_MISCONFIG {
                vmread(GUEST_PHYSICAL_ADDRESS)
            } else {
                0
            };

            // Handle CPUID exits internally
            if reason == EXIT_REASON_CPUID {
                handle_cpuid_exit(&vm.cpuid_table[..vm.cpuid_count], vcpu);
                return Some(VmExitInfo {
                    reason,
                    ..Default::default()
                });
            }

            let mut info = VmExitInfo {
                reason,
                qualification,
                guest_phys_addr: guest_phys,
                instruction_len: instr_len,
                ..Default::default()
            };

            match reason {
                EXIT_REASON_IO_INSTRUCTION => {
                    info.io_port = (qualification >> 16) as u16;
                    info.access_size = ((qualification & 0x7) + 1) as u8;
                    info.is_read = ((qualification >> 3) & 1) as u8;
                    if info.is_read == 0 {
                        info.io_data = vcpu.guest_gprs.rax;
                    }
                }
                EXIT_REASON_EPT_VIOLATION => {
                    info.is_read = if (qualification >> 1) & 1 != 0 { 0 } else { 1 };
                    info.access_size = 4; // default; exact size requires instruction decode
                    if info.is_read == 0 {
                        info.io_data = vcpu.guest_gprs.rax;
                    }
                }
                EXIT_REASON_RDMSR => {
                    info.msr_index = vcpu.guest_gprs.rcx as u32;
                    info.is_read = 1;
                }
                EXIT_REASON_WRMSR => {
                    info.msr_index = vcpu.guest_gprs.rcx as u32;
                    info.io_data = (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
                }
                _ => {}
            }

            Some(info)
        } else {
            // VM-entry failure
            crate::serial_println!("VMX: VM-entry failed for VM {} vCPU {}", vm_id, vcpu_id);
            None
        }
    }
}

/// Handle a CPUID VM-exit by emulating the instruction.
unsafe fn handle_cpuid_exit(cpuid_table: &[Option<CpuidEntry>], vcpu: &mut VmxVcpu) {
    let leaf = vcpu.guest_gprs.rax as u32;
    let subleaf = vcpu.guest_gprs.rcx as u32;

    // Check VM's CPUID table first
    let mut found = false;
    for entry_opt in cpuid_table {
        if let Some(entry) = entry_opt {
            if entry.function == leaf && entry.index == subleaf {
                vcpu.guest_gprs.rax = entry.eax as u64;
                vcpu.guest_gprs.rbx = entry.ebx as u64;
                vcpu.guest_gprs.rcx = entry.ecx as u64;
                vcpu.guest_gprs.rdx = entry.edx as u64;
                found = true;
                break;
            }
        }
    }

    if !found {
        // Pass through host CPUID (with some masking)
        let (eax, ebx, ecx, edx) = crate::arch::x86::cpuid::cpuid(leaf, subleaf);
        vcpu.guest_gprs.rax = eax as u64;
        vcpu.guest_gprs.rbx = ebx as u64;
        vcpu.guest_gprs.rcx = ecx as u64;
        vcpu.guest_gprs.rdx = edx as u64;
    }

    // Advance guest RIP past CPUID instruction (2 bytes: 0F A2)
    let rip = vmread(GUEST_RIP);
    let instr_len = vmread(VM_EXIT_INSTR_LEN);
    vmwrite(GUEST_RIP, rip + instr_len);
}

/// Get guest general-purpose registers.
pub fn get_regs(vm_id: u32, vcpu_id: u32) -> Option<GuestGprs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;
    Some(vcpu.guest_gprs)
}

/// Set guest general-purpose registers.
pub fn set_regs(vm_id: u32, vcpu_id: u32, gprs: &GuestGprs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() { Some(v) => v, None => return false };
    vcpu.guest_gprs = *gprs;
    true
}

/// Get guest segment/control registers.
pub fn get_sregs(vm_id: u32, vcpu_id: u32) -> Option<GuestSregs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        Some(GuestSregs {
            cs_selector: vmread(GUEST_CS_SELECTOR) as u16,
            cs_base: vmread(GUEST_CS_BASE),
            cs_limit: vmread(GUEST_CS_LIMIT) as u32,
            cs_ar: vmread(GUEST_CS_AR_BYTES) as u32,
            ds_selector: vmread(GUEST_DS_SELECTOR) as u16,
            ds_base: vmread(GUEST_DS_BASE),
            ds_limit: vmread(GUEST_DS_LIMIT) as u32,
            ds_ar: vmread(GUEST_DS_AR_BYTES) as u32,
            es_selector: vmread(GUEST_ES_SELECTOR) as u16,
            es_base: vmread(GUEST_ES_BASE),
            es_limit: vmread(GUEST_ES_LIMIT) as u32,
            es_ar: vmread(GUEST_ES_AR_BYTES) as u32,
            fs_selector: vmread(GUEST_FS_SELECTOR) as u16,
            fs_base: vmread(GUEST_FS_BASE),
            fs_limit: vmread(GUEST_FS_LIMIT) as u32,
            fs_ar: vmread(GUEST_FS_AR_BYTES) as u32,
            gs_selector: vmread(GUEST_GS_SELECTOR) as u16,
            gs_base: vmread(GUEST_GS_BASE),
            gs_limit: vmread(GUEST_GS_LIMIT) as u32,
            gs_ar: vmread(GUEST_GS_AR_BYTES) as u32,
            ss_selector: vmread(GUEST_SS_SELECTOR) as u16,
            ss_base: vmread(GUEST_SS_BASE),
            ss_limit: vmread(GUEST_SS_LIMIT) as u32,
            ss_ar: vmread(GUEST_SS_AR_BYTES) as u32,
            tr_selector: vmread(GUEST_TR_SELECTOR) as u16,
            tr_base: vmread(GUEST_TR_BASE),
            tr_limit: vmread(GUEST_TR_LIMIT) as u32,
            tr_ar: vmread(GUEST_TR_AR_BYTES) as u32,
            ldtr_selector: vmread(GUEST_LDTR_SELECTOR) as u16,
            ldtr_base: vmread(GUEST_LDTR_BASE),
            ldtr_limit: vmread(GUEST_LDTR_LIMIT) as u32,
            ldtr_ar: vmread(GUEST_LDTR_AR_BYTES) as u32,
            gdtr_base: vmread(GUEST_GDTR_BASE),
            gdtr_limit: vmread(GUEST_GDTR_LIMIT) as u32,
            idtr_base: vmread(GUEST_IDTR_BASE),
            idtr_limit: vmread(GUEST_IDTR_LIMIT) as u32,
            cr0: vmread(GUEST_CR0),
            cr3: vmread(GUEST_CR3),
            cr4: vmread(GUEST_CR4),
            efer: vmread(GUEST_IA32_EFER),
            rip: vmread(GUEST_RIP),
            rsp: vmread(GUEST_RSP),
            rflags: vmread(GUEST_RFLAGS),
        })
    }
}

/// Set guest segment/control registers.
pub fn set_sregs(vm_id: u32, vcpu_id: u32, sregs: &GuestSregs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() { Some(v) => v, None => return false };

    unsafe {
        vmptrld(vcpu.vmcs_phys);

        vmwrite(GUEST_CS_SELECTOR, sregs.cs_selector as u64);
        vmwrite(GUEST_CS_BASE, sregs.cs_base);
        vmwrite(GUEST_CS_LIMIT, sregs.cs_limit as u64);
        vmwrite(GUEST_CS_AR_BYTES, sregs.cs_ar as u64);
        vmwrite(GUEST_DS_SELECTOR, sregs.ds_selector as u64);
        vmwrite(GUEST_DS_BASE, sregs.ds_base);
        vmwrite(GUEST_DS_LIMIT, sregs.ds_limit as u64);
        vmwrite(GUEST_DS_AR_BYTES, sregs.ds_ar as u64);
        vmwrite(GUEST_ES_SELECTOR, sregs.es_selector as u64);
        vmwrite(GUEST_ES_BASE, sregs.es_base);
        vmwrite(GUEST_ES_LIMIT, sregs.es_limit as u64);
        vmwrite(GUEST_ES_AR_BYTES, sregs.es_ar as u64);
        vmwrite(GUEST_FS_SELECTOR, sregs.fs_selector as u64);
        vmwrite(GUEST_FS_BASE, sregs.fs_base);
        vmwrite(GUEST_FS_LIMIT, sregs.fs_limit as u64);
        vmwrite(GUEST_FS_AR_BYTES, sregs.fs_ar as u64);
        vmwrite(GUEST_GS_SELECTOR, sregs.gs_selector as u64);
        vmwrite(GUEST_GS_BASE, sregs.gs_base);
        vmwrite(GUEST_GS_LIMIT, sregs.gs_limit as u64);
        vmwrite(GUEST_GS_AR_BYTES, sregs.gs_ar as u64);
        vmwrite(GUEST_SS_SELECTOR, sregs.ss_selector as u64);
        vmwrite(GUEST_SS_BASE, sregs.ss_base);
        vmwrite(GUEST_SS_LIMIT, sregs.ss_limit as u64);
        vmwrite(GUEST_SS_AR_BYTES, sregs.ss_ar as u64);
        vmwrite(GUEST_TR_SELECTOR, sregs.tr_selector as u64);
        vmwrite(GUEST_TR_BASE, sregs.tr_base);
        vmwrite(GUEST_TR_LIMIT, sregs.tr_limit as u64);
        vmwrite(GUEST_TR_AR_BYTES, sregs.tr_ar as u64);
        vmwrite(GUEST_LDTR_SELECTOR, sregs.ldtr_selector as u64);
        vmwrite(GUEST_LDTR_BASE, sregs.ldtr_base);
        vmwrite(GUEST_LDTR_LIMIT, sregs.ldtr_limit as u64);
        vmwrite(GUEST_LDTR_AR_BYTES, sregs.ldtr_ar as u64);
        vmwrite(GUEST_GDTR_BASE, sregs.gdtr_base);
        vmwrite(GUEST_GDTR_LIMIT, sregs.gdtr_limit as u64);
        vmwrite(GUEST_IDTR_BASE, sregs.idtr_base);
        vmwrite(GUEST_IDTR_LIMIT, sregs.idtr_limit as u64);
        vmwrite(GUEST_CR0, sregs.cr0);
        vmwrite(GUEST_CR3, sregs.cr3);
        vmwrite(GUEST_CR4, sregs.cr4);
        vmwrite(GUEST_IA32_EFER, sregs.efer);
        vmwrite(GUEST_RIP, sregs.rip);
        vmwrite(GUEST_RSP, sregs.rsp);
        vmwrite(GUEST_RFLAGS, sregs.rflags);
    }

    true
}

/// Inject an external interrupt into the guest.
pub fn inject_irq(vm_id: u32, vcpu_id: u32, vector: u8) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        let info = (vector as u32) | (0 << 8) | (1 << 31);
        vmwrite(VM_ENTRY_INTR_INFO, info as u64);
    }
    true
}

/// Inject an exception into the guest.
pub fn inject_exception(vm_id: u32, vcpu_id: u32, vector: u8, error_code: u32) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        let has_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
        let mut info = (vector as u32) | (3 << 8) | (1 << 31);
        if has_error {
            info |= 1 << 11;
            vmwrite(VM_ENTRY_EXCEPTION_ERROR_CODE, error_code as u64);
        }
        vmwrite(VM_ENTRY_INTR_INFO, info as u64);
    }
    true
}

/// Inject an NMI into the guest.
pub fn inject_nmi(vm_id: u32, vcpu_id: u32) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        let info: u32 = 2 | (2 << 8) | (1 << 31);
        vmwrite(VM_ENTRY_INTR_INFO, info as u64);
    }
    true
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn find_vm(vms: &[Option<VmxVm>; MAX_VMS], vm_id: u32) -> Option<&VmxVm> {
    vms.iter().find_map(|slot| {
        slot.as_ref().filter(|vm| vm.id == vm_id)
    })
}

fn find_vm_mut(vms: &mut [Option<VmxVm>; MAX_VMS], vm_id: u32) -> Option<&mut VmxVm> {
    vms.iter_mut().find_map(|slot| {
        slot.as_mut().filter(|vm| vm.id == vm_id)
    })
}
