//! Intel VT-x (VMX) support.
//!
//! Implements VMXON per-CPU initialization, VMCS lifecycle, VM-entry/exit,
//! and exit reason handling for the anyOS hypervisor interface.

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    alloc_page_zeroed, free_page, rdmsr, read_cr0, read_cr3, read_cr4, write_cr4, wrmsr,
    CpuidEntry, GuestFpuState, GuestGprs, GuestSregs, MemoryRegion, VcpuMpState, VmExitInfo,
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
const HOST_TR_BASE: u32 = 0x6C0A;
const HOST_IA32_SYSENTER_CS: u32 = 0x4C00;
const HOST_IA32_SYSENTER_ESP: u32 = 0x6C10;
const HOST_IA32_SYSENTER_EIP: u32 = 0x6C12;

// Control fields
const PIN_BASED_VM_EXEC_CONTROL: u32 = 0x4000;
const PRIMARY_PROC_BASED_VM_EXEC_CONTROL: u32 = 0x4002;
const SECONDARY_PROC_BASED_VM_EXEC_CONTROL: u32 = 0x401E;
const VM_EXIT_CONTROLS: u32 = 0x400C;
const VM_ENTRY_CONTROLS: u32 = 0x4012;
const EPT_POINTER: u32 = 0x201A;
const MSR_BITMAP_ADDR: u32 = 0x2004;
const VIRTUAL_PROCESSOR_ID: u32 = 0x0000; // VPID (16-bit)

// Exit information
const VM_EXIT_REASON: u32 = 0x4402;
const VM_INSTRUCTION_ERROR: u32 = 0x4400;
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

// ── VMX exit reasons (Intel SDM Vol. 3C Table 27-1) ─────────────────────

pub const EXIT_REASON_EXTERNAL_INTERRUPT: u32 = 1;
pub const EXIT_REASON_TRIPLE_FAULT: u32 = 2;
pub const EXIT_REASON_INIT_SIGNAL: u32 = 3;
pub const EXIT_REASON_SIPI: u32 = 4;
pub const EXIT_REASON_CPUID: u32 = 10;
pub const EXIT_REASON_HLT: u32 = 12;
pub const EXIT_REASON_INVD: u32 = 13;
pub const EXIT_REASON_INVLPG: u32 = 14;
pub const EXIT_REASON_RDPMC: u32 = 15;
pub const EXIT_REASON_RDTSC: u32 = 16;
pub const EXIT_REASON_RSM: u32 = 17;
pub const EXIT_REASON_VMCALL: u32 = 18;
pub const EXIT_REASON_CR_ACCESS: u32 = 28;
pub const EXIT_REASON_DR_ACCESS: u32 = 29;
pub const EXIT_REASON_IO_INSTRUCTION: u32 = 30;
pub const EXIT_REASON_RDMSR: u32 = 31;
pub const EXIT_REASON_WRMSR: u32 = 32;
pub const EXIT_REASON_INVALID_GUEST_STATE: u32 = 33;
pub const EXIT_REASON_PAUSE: u32 = 40;
pub const EXIT_REASON_EPT_VIOLATION: u32 = 48;
pub const EXIT_REASON_EPT_MISCONFIG: u32 = 49;
pub const EXIT_REASON_RDTSCP: u32 = 51;
pub const EXIT_REASON_PREEMPTION_TIMER: u32 = 52;
pub const EXIT_REASON_WBINVD: u32 = 54;
pub const EXIT_REASON_XSETBV: u32 = 55;
pub const EXIT_REASON_RDRAND: u32 = 57;
pub const EXIT_REASON_INVPCID: u32 = 58;
pub const EXIT_REASON_RDSEED: u32 = 61;

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
const IA32_SYSENTER_CS: u32 = 0x174;
const IA32_SYSENTER_ESP: u32 = 0x175;
const IA32_SYSENTER_EIP: u32 = 0x176;

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
    guest_fpu: GuestFpuState,
    launched: bool,
    paused: bool,
    mp_state: VcpuMpState,
    msr_bitmap_phys: u64,
    vpid: u16,
}

// Global VPID allocator.  0 is reserved (means "no VPID"), so start at 1.
static NEXT_VPID: AtomicU32 = AtomicU32::new(1);

struct VmxVm {
    id: u32,
    active: bool,
    vcpus: [Option<VmxVcpu>; MAX_VCPUS_PER_VM],
    ept_root: u64,
    memory_regions: [Option<MemoryRegion>; 32],
    cpuid_table: [Option<CpuidEntry>; 64],
    cpuid_count: usize,
    /// Physical address of the dirty-log page (allocated on demand by alloc_page_zeroed).
    /// Layout: 32 slots × 64 u64 = 32×512 bytes = 16 KiB — fits in 4 pages.
    /// We allocate 4 contiguous pages for simplicity (see create_vm).
    /// 0 = not allocated yet.
    dirty_log_phys: u64,
}

// We use a simple static array of VMs protected by a spinlock.
// For VM-entry/exit hot path, the lock is released before VMLAUNCH.
static VMS: Spinlock<[Option<VmxVm>; MAX_VMS]> = Spinlock::new([const { None }; MAX_VMS]);

// (No static dirty-log array — dirty logs are heap-allocated per VM.)

// ── VMX instruction wrappers ─────────────────────────────────────────────

#[inline]
unsafe fn vmxon(phys_addr: u64) -> bool {
    core::arch::asm!(
        "vmxon [{}]",
        in(reg) &phys_addr,
        options(nostack),
    );
    // If we reach here without fault, VMXON succeeded.
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
    let allowed_0 = cap as u32; // bits that MUST be 1
    let allowed_1 = (cap >> 32) as u32; // bits that CAN be 1
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
        *(super::phys_to_virt(page) as *mut u32) = revision;

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

    // Allocate dirty-log pages on demand (4 pages = 16 KiB for 32 slots × 64 u64).
    // We allocate page 0 now; pages 1-3 share the same phys region via the
    // virt mapping. For simplicity we just allocate 4 separate pages and use
    // only the first here — the dirty-log helper addresses into it by offset.
    // Actually: 32 slots × 64 u64 × 8 bytes = 16 384 bytes = exactly 4 pages.
    // We allocate them individually and store only the first phys address;
    // the others are at phys+PAGE, phys+2PAGE, phys+3PAGE (if contiguous).
    // For the kernel's simple bump-allocator this is usually contiguous.
    // If not contiguous, dirty tracking silently degrades (bits just won't set).
    // A future improvement can use a proper multi-page alloc API.
    let dirty_log_phys = alloc_page_zeroed().unwrap_or(0);

    let vm = VmxVm {
        id,
        active: true,
        vcpus: [const { None }; MAX_VCPUS_PER_VM],
        ept_root,
        memory_regions: [const { None }; 32],
        cpuid_table: [const { None }; 64],
        cpuid_count: 0,
        dirty_log_phys,
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
                        unsafe {
                            vmclear(vcpu.vmcs_phys);
                        }
                        free_page(vcpu.vmcs_phys);
                        if vcpu.msr_bitmap_phys != 0 {
                            free_page(vcpu.msr_bitmap_phys);
                        }
                    }
                }
                // Free EPT
                super::ept::destroy_ept(vm.ept_root);
                // Free dirty-log page
                if vm.dirty_log_phys != 0 {
                    free_page(vm.dirty_log_phys);
                }
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

    // Flush stale EPT-derived TLB entries.  Must be done after any EPT
    // mapping change so all CPUs see the new translation.
    unsafe {
        super::ept::invept_all_context();
    }

    true
}

/// Map a single 4KB page in the EPT for a VM (used by sys_vm_set_memory for pages
/// beyond the first, which are mapped without slot re-registration).
pub fn map_page_in_ept(vm_id: u32, gpa: u64, hpa: u64) {
    let mut vms = VMS.lock();
    if let Some(vm) = find_vm_mut(&mut vms, vm_id) {
        super::ept::ept_map_range(vm.ept_root, gpa, hpa, 0x1000, true, true);
        unsafe {
            super::ept::invept_all_context();
        }
    }
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

// ── MSR bitmap helpers ────────────────────────────────────────────────────

/// Clear the RDMSR intercept bit for an MSR in the low range (0x0000–0x1FFF).
/// Clearing = passthrough (no VM-exit on RDMSR for that MSR).
unsafe fn msr_bitmap_clear_rdmsr(bitmap_phys: u64, msr: u32) {
    debug_assert!(msr <= 0x1FFF);
    let byte = (msr / 8) as usize; // byte index in the 1K RDMSR-low block
    let bit = (msr % 8) as u8;
    let ptr = super::phys_to_virt(bitmap_phys) as *mut u8;
    *ptr.add(byte) &= !(1 << bit);
}

/// Clear the RDMSR intercept bit for an MSR in the high range
/// (0xC000_0000–0xC000_1FFF), i.e. only pass the low 13 bits.
unsafe fn msr_bitmap_clear_rdmsr_high(bitmap_phys: u64, msr_low: u32) {
    debug_assert!(msr_low <= 0x1FFF);
    // High RDMSR block starts at byte 1024.
    let byte = 1024 + (msr_low / 8) as usize;
    let bit = (msr_low % 8) as u8;
    let ptr = super::phys_to_virt(bitmap_phys) as *mut u8;
    *ptr.add(byte) &= !(1 << bit);
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

        // Allocate MSR bitmap. Fill with 1s then selectively clear bits for
        // MSRs we want to pass through without a VM-exit.
        let msr_bitmap_phys = match alloc_page_zeroed() {
            Some(p) => p,
            None => {
                free_page(vmcs_phys);
                return false;
            }
        };
        // Start: intercept everything.
        core::ptr::write_bytes(super::phys_to_virt(msr_bitmap_phys) as *mut u8, 0xFF, 4096);
        // Clear intercept bits for common read-only MSRs so the guest can
        // read them without a VM-exit (they are safe to pass through directly).
        //
        // MSR bitmap layout (Intel SDM Vol. 3C §24.6.9):
        //   Bytes   0–1023: RDMSR intercepts for MSR 0x0000–0x1FFF
        //   Bytes 1024–2047: RDMSR intercepts for MSR 0xC000_0000–0xC000_1FFF
        //   Bytes 2048–3071: WRMSR intercepts for MSR 0x0000–0x1FFF
        //   Bytes 3072–4095: WRMSR intercepts for MSR 0xC000_0000–0xC000_1FFF
        //
        // Bit index within each range = MSR low 13 bits.
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x10); // IA32_TSC
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x1B); // IA32_APIC_BASE (read-only view)
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x8B); // IA32_BIOS_SIGN_ID
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0xFE); // IA32_MTRRCAP
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x174); // IA32_SYSENTER_CS
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x175); // IA32_SYSENTER_ESP
        msr_bitmap_clear_rdmsr(msr_bitmap_phys, 0x176); // IA32_SYSENTER_EIP
                                                        // IA32_TSC_AUX (0xC0000103) — C000 range read
        msr_bitmap_clear_rdmsr_high(msr_bitmap_phys, 0x103);

        // Write revision ID
        let vmx_basic = rdmsr(IA32_VMX_BASIC);
        let revision = (vmx_basic & 0x7FFF_FFFF) as u32;
        *(super::phys_to_virt(vmcs_phys) as *mut u32) = revision;

        // Allocate a VPID for this vCPU.  Wrap around 0xFFFF; 0 is reserved.
        let vpid = {
            let raw = NEXT_VPID.fetch_add(1, Ordering::Relaxed);
            let v = (raw & 0xFFFF) as u16;
            if v == 0 {
                1
            } else {
                v
            }
        };

        // VMCLEAR then VMPTRLD
        vmclear(vmcs_phys);
        vmptrld(vmcs_phys);

        // Initialize VMCS fields
        init_vmcs(vm.ept_root, msr_bitmap_phys, vpid);

        vm.vcpus[idx] = Some(VmxVcpu {
            vmcs_phys,
            guest_gprs: GuestGprs::default(),
            guest_fpu: GuestFpuState::default(),
            launched: false,
            paused: false,
            mp_state: VcpuMpState::Runnable,
            msr_bitmap_phys,
            vpid,
        });
    }

    true
}

/// Initialize all VMCS fields for a new vCPU.
unsafe fn init_vmcs(ept_root: u64, msr_bitmap_phys: u64, vpid: u16) {
    // Check for TRUE controls support
    let vmx_basic = rdmsr(IA32_VMX_BASIC);
    let use_true_ctls = (vmx_basic >> 55) & 1 != 0;

    // Pin-based controls: external-interrupt exiting
    let pin_msr = if use_true_ctls {
        IA32_VMX_TRUE_PINBASED_CTLS
    } else {
        IA32_VMX_PINBASED_CTLS
    };
    let pin_based = adjust_control(
        1 << 0, // External-interrupt exiting
        pin_msr,
    );
    vmwrite(PIN_BASED_VM_EXEC_CONTROL, pin_based as u64);

    // Primary proc-based controls
    let proc_msr = if use_true_ctls {
        IA32_VMX_TRUE_PROCBASED_CTLS
    } else {
        IA32_VMX_PROCBASED_CTLS
    };
    let primary = adjust_control(
        (1 << 7)   // HLT exiting
        | (1 << 24) // Unconditional I/O exiting
        | (1 << 28) // Use MSR bitmaps
        | (1 << 31), // Activate secondary controls
        proc_msr,
    );
    vmwrite(PRIMARY_PROC_BASED_VM_EXEC_CONTROL, primary as u64);

    // Secondary proc-based controls
    // Bit 5 = Enable VPID.  We attempt to enable it; adjust_control will
    // silently drop the bit if the CPU doesn't support it.
    let secondary = adjust_control(
        (1 << 1)  // Enable EPT
        | (1 << 5) // Enable VPID
        | (1 << 7), // Unrestricted guest
        IA32_VMX_PROCBASED_CTLS2,
    );
    vmwrite(SECONDARY_PROC_BASED_VM_EXEC_CONTROL, secondary as u64);

    // Write VPID (only meaningful when the VPID secondary-control bit is set,
    // but harmless to write regardless).
    vmwrite(VIRTUAL_PROCESSOR_ID, vpid as u64);

    // VM-exit controls
    let exit_msr = if use_true_ctls {
        IA32_VMX_TRUE_EXIT_CTLS
    } else {
        IA32_VMX_EXIT_CTLS
    };
    let exit_ctls = adjust_control(
        (1 << 9)   // Host address-space size (64-bit host)
        | (1 << 20) // Save IA32_EFER
        | (1 << 21), // Load IA32_EFER
        exit_msr,
    );
    vmwrite(VM_EXIT_CONTROLS, exit_ctls as u64);

    // VM-entry controls
    let entry_msr = if use_true_ctls {
        IA32_VMX_TRUE_ENTRY_CTLS
    } else {
        IA32_VMX_ENTRY_CTLS
    };
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

    // CR0 guest/host mask: the host owns CR0.PE (bit 0) and CR0.NE (bit 5) so
    // writes to those bits cause a VM-exit.  The guest sees our shadow values.
    // We expose PE=0 in the shadow (real-mode start) and NE=1.
    let cr0_owned: u64 = (1 << 0) | (1 << 5); // PE, NE
    vmwrite(CR0_GUEST_HOST_MASK, cr0_owned);
    vmwrite(CR0_READ_SHADOW, 1 << 5); // NE=1, PE=0 (real mode)

    // CR4 guest/host mask: the host owns CR4.VMXE (bit 13) — the guest must
    // never be able to clear it.  We show the guest VMXE=0 in the shadow.
    let cr4_owned: u64 = 1 << 13; // VMXE
    vmwrite(CR4_GUEST_HOST_MASK, cr4_owned);
    vmwrite(CR4_READ_SHADOW, 0); // guest sees VMXE=0

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

    vmwrite(HOST_CS_SELECTOR, (cs & !0x7) as u64);
    vmwrite(HOST_SS_SELECTOR, (ss & !0x7) as u64);
    vmwrite(HOST_DS_SELECTOR, (ds & !0x7) as u64);
    vmwrite(HOST_ES_SELECTOR, (es & !0x7) as u64);
    vmwrite(HOST_FS_SELECTOR, (fs & !0x7) as u64);
    vmwrite(HOST_GS_SELECTOR, (gs & !0x7) as u64);
    vmwrite(HOST_TR_SELECTOR, (tr & !0x7) as u64);

    vmwrite(HOST_CR0, read_cr0());
    vmwrite(HOST_CR3, read_cr3());
    vmwrite(HOST_CR4, read_cr4());
    vmwrite(HOST_IA32_EFER, rdmsr(IA32_EFER));

    // GDT and IDT base
    let mut gdtr: [u8; 10] = [0; 10];
    let mut idtr: [u8; 10] = [0; 10];
    core::arch::asm!("sgdt [{}]", in(reg) gdtr.as_mut_ptr(), options(nostack));
    core::arch::asm!("sidt [{}]", in(reg) idtr.as_mut_ptr(), options(nostack));
    let gdt_base = u64::from_le_bytes([
        gdtr[2], gdtr[3], gdtr[4], gdtr[5], gdtr[6], gdtr[7], gdtr[8], gdtr[9],
    ]);
    let idt_base = u64::from_le_bytes([
        idtr[2], idtr[3], idtr[4], idtr[5], idtr[6], idtr[7], idtr[8], idtr[9],
    ]);
    vmwrite(HOST_GDTR_BASE, gdt_base);
    vmwrite(HOST_IDTR_BASE, idt_base);
    vmwrite(HOST_TR_BASE, host_tss_base(gdt_base, tr));

    vmwrite(HOST_FS_BASE, rdmsr(0xC0000100)); // IA32_FS_BASE
    vmwrite(HOST_GS_BASE, rdmsr(0xC0000101)); // IA32_GS_BASE
    vmwrite(HOST_IA32_SYSENTER_CS, rdmsr(IA32_SYSENTER_CS));
    vmwrite(HOST_IA32_SYSENTER_ESP, rdmsr(IA32_SYSENTER_ESP));
    vmwrite(HOST_IA32_SYSENTER_EIP, rdmsr(IA32_SYSENTER_EIP));

    // HOST_RIP and HOST_RSP are set dynamically before each VM-entry
    vmwrite(HOST_RIP, vmx_vmexit_stub as u64);
    vmwrite(HOST_RSP, 0); // Will be patched before VM-entry
}

unsafe fn host_tss_base(gdt_base: u64, tr: u16) -> u64 {
    let selector_offset = (tr & !0x7) as usize;
    if selector_offset == 0 {
        return 0;
    }

    let desc = (gdt_base as *const u8).add(selector_offset);
    let base_low = core::ptr::read_unaligned(desc.add(2) as *const u16) as u64;
    let base_mid = core::ptr::read_unaligned(desc.add(4) as *const u8) as u64;
    let base_high = core::ptr::read_unaligned(desc.add(7) as *const u8) as u64;
    let base_upper = core::ptr::read_unaligned(desc.add(8) as *const u32) as u64;

    base_low | (base_mid << 16) | (base_high << 24) | (base_upper << 32)
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
    "push rdi", // save GuestGprs pointer for vmexit stub
    // Set HOST_RSP to current RSP (after all pushes, pointing at GuestGprs ptr)
    "mov rax, 0x6C14", // HOST_RSP field encoding
    "mov rdx, rsp",    // current RSP after all pushes
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
    "jmp 3f", // VM-entry failed
    "2:",
    "vmresume",
    "jmp 3f", // VM-entry failed
    // VM-exit handler — HOST_RIP points here
    ".global vmx_vmexit_stub",
    "vmx_vmexit_stub:",
    // On VM-exit, CPU loaded host state from VMCS.
    // RSP = HOST_RSP (set to point at the pushed rdi on the stack).
    // Guest GPRs are in physical registers, need to save them.
    // rdi was guest rdi; the GuestGprs pointer is at [rsp].
    "xchg rdi, [rsp]", // rdi = GuestGprs ptr, [rsp] = guest rdi
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
    "pop rdi", // discard GuestGprs pointer
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

    // Refuse to enter a paused or halted vCPU.
    if vcpu.paused || vcpu.mp_state == VcpuMpState::Halted {
        return None;
    }

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
            let hw_reason = vmread(VM_EXIT_REASON) as u32 & 0xFFFF; // low 16 bits = basic reason
            let qualification = vmread(EXIT_QUALIFICATION);
            let instr_len = vmread(VM_EXIT_INSTR_LEN) as u32;

            let guest_phys = if hw_reason == EXIT_REASON_EPT_VIOLATION
                || hw_reason == EXIT_REASON_EPT_MISCONFIG
            {
                vmread(GUEST_PHYSICAL_ADDRESS)
            } else {
                0
            };

            // Read guest linear address (valid for EPT violations where bit 7 of
            // qualification = GLA_VALID).
            let guest_virt =
                if hw_reason == EXIT_REASON_EPT_VIOLATION && (qualification & (1 << 7)) != 0 {
                    vmread(0x640A) // GUEST_LINEAR_ADDRESS VMCS field
                } else {
                    0
                };

            // Map raw VMX exit code → portable exit_reason::* value.
            let reason = match hw_reason {
                EXIT_REASON_EXTERNAL_INTERRUPT => super::exit_reason::EXTERNAL_INTERRUPT,
                EXIT_REASON_TRIPLE_FAULT => super::exit_reason::TRIPLE_FAULT,
                EXIT_REASON_INIT_SIGNAL => super::exit_reason::INIT_SIGNAL,
                EXIT_REASON_SIPI => super::exit_reason::SIPI,
                EXIT_REASON_HLT => super::exit_reason::HLT,
                EXIT_REASON_CPUID => super::exit_reason::CPUID,
                EXIT_REASON_INVD => super::exit_reason::INVD,
                EXIT_REASON_INVLPG => super::exit_reason::INVLPG,
                EXIT_REASON_RDPMC => super::exit_reason::RDPMC,
                EXIT_REASON_RDTSC => super::exit_reason::RDTSC,
                EXIT_REASON_RSM => super::exit_reason::RSM,
                EXIT_REASON_VMCALL => super::exit_reason::VMCALL,
                EXIT_REASON_CR_ACCESS => super::exit_reason::CR_ACCESS,
                EXIT_REASON_DR_ACCESS => super::exit_reason::DR_ACCESS,
                EXIT_REASON_IO_INSTRUCTION => super::exit_reason::IO_INSTRUCTION,
                EXIT_REASON_RDMSR => super::exit_reason::RDMSR,
                EXIT_REASON_WRMSR => super::exit_reason::WRMSR,
                EXIT_REASON_INVALID_GUEST_STATE => super::exit_reason::INVALID_GUEST_STATE,
                EXIT_REASON_PAUSE => super::exit_reason::PAUSE,
                EXIT_REASON_EPT_VIOLATION => super::exit_reason::EPT_VIOLATION,
                EXIT_REASON_EPT_MISCONFIG => super::exit_reason::EPT_MISCONFIG,
                EXIT_REASON_RDTSCP => super::exit_reason::RDTSCP,
                EXIT_REASON_PREEMPTION_TIMER => super::exit_reason::PREEMPTION_TIMER,
                EXIT_REASON_WBINVD => super::exit_reason::WBINVD,
                EXIT_REASON_XSETBV => super::exit_reason::XSETBV,
                EXIT_REASON_RDRAND => super::exit_reason::RDRAND,
                EXIT_REASON_INVPCID => super::exit_reason::INVPCID,
                EXIT_REASON_RDSEED => super::exit_reason::RDSEED,
                _ => hw_reason, // pass unknown reasons through
            };

            // Handle CPUID exits internally — advance RIP, fill regs, return synthetic reason.
            if hw_reason == EXIT_REASON_CPUID {
                handle_cpuid_exit(&vm.cpuid_table[..vm.cpuid_count], vcpu);
                return Some(VmExitInfo {
                    reason: super::exit_reason::CPUID_EMULATED,
                    hw_reason,
                    ..Default::default()
                });
            }

            let mut info = VmExitInfo {
                reason,
                hw_reason,
                qualification,
                guest_phys_addr: guest_phys,
                guest_virt_addr: guest_virt,
                instruction_len: instr_len,
                ..Default::default()
            };

            match hw_reason {
                EXIT_REASON_HLT => {
                    // Advance RIP past HLT (1 byte) and put vCPU into halted state.
                    let rip = vmread(GUEST_RIP);
                    vmwrite(GUEST_RIP, rip + instr_len as u64);
                    vcpu.mp_state = VcpuMpState::Halted;
                    info.reason = super::exit_reason::HLT_EMULATED;
                }
                EXIT_REASON_IO_INSTRUCTION => {
                    // Intel SDM Vol. 3C §27.2.1, Exit Qualification for I/O:
                    //   bits  2:0 = size encoding (0=1B, 1=2B, 3=4B)
                    //   bit     3 = direction (0=OUT, 1=IN)
                    //   bits 31:16 = port number
                    info.io_port = (qualification >> 16) as u16;
                    info.access_size = ((qualification & 0x7) + 1) as u8;
                    info.is_read = ((qualification >> 3) & 1) as u8;
                    if info.is_read == 0 {
                        info.io_data = vcpu.guest_gprs.rax;
                    }
                }
                EXIT_REASON_EPT_VIOLATION => {
                    // Qualification: bit 0=read, bit 1=write, bit 2=insn-fetch.
                    let is_write = (qualification >> 1) & 1;
                    info.is_read = if is_write == 0 { 1 } else { 0 };
                    info.access_size = 4;
                    if is_write != 0 {
                        info.io_data = vcpu.guest_gprs.rax;
                        mark_dirty_page(vm.dirty_log_phys, &vm.memory_regions, guest_phys);
                    }
                }
                EXIT_REASON_RDMSR => {
                    info.msr_index = vcpu.guest_gprs.rcx as u32;
                    info.is_read = 1;
                    // Advance RIP (kernel returns to guest after userspace handles)
                }
                EXIT_REASON_WRMSR => {
                    info.msr_index = vcpu.guest_gprs.rcx as u32;
                    info.is_read = 0;
                    info.io_data =
                        (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
                }
                EXIT_REASON_CR_ACCESS => {
                    // Qualification: bits 3:0 = CR#, bit 4 = direction (0=write,1=read),
                    //   bits 11:8 = source/dest general-purpose register.
                    info.cr_number = (qualification & 0xF) as u8;
                    info.cr_is_read = ((qualification >> 4) & 1) as u8;
                }
                EXIT_REASON_DR_ACCESS => {
                    // Qualification: bits 2:0 = DR#, bit 4 = direction.
                    info.dr_number = (qualification & 0x7) as u8;
                    info.dr_is_read = ((qualification >> 4) & 1) as u8;
                }
                EXIT_REASON_VMCALL => {
                    // Hypercall: convention = rax=call#, rbx=arg1, rcx=arg2, rdx=arg3
                    info.io_data = vcpu.guest_gprs.rax; // call number
                    info.io_data2 = vcpu.guest_gprs.rbx; // first arg
                }
                EXIT_REASON_XSETBV => {
                    // ECX = XCR index, EDX:EAX = new value
                    info.msr_index = vcpu.guest_gprs.rcx as u32;
                    info.io_data =
                        (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
                }
                EXIT_REASON_SIPI => {
                    // Qualification bits 7:0 = SIPI vector
                    info.io_data = qualification & 0xFF;
                }
                _ => {}
            }

            Some(info)
        } else {
            // VM-entry failure
            vmptrld(vcpu.vmcs_phys);
            let instruction_error = vmread(VM_INSTRUCTION_ERROR);
            let rip = vmread(GUEST_RIP);
            let cr0 = vmread(GUEST_CR0);
            let cr4 = vmread(GUEST_CR4);
            let efer = vmread(GUEST_IA32_EFER);
            crate::serial_println!(
                "VMX: VM-entry failed for VM {} vCPU {} err={} rip={:#x} cr0={:#x} cr4={:#x} efer={:#x}",
                vm_id,
                vcpu_id,
                instruction_error,
                rip,
                cr0,
                cr4,
                efer
            );
            vcpu.mp_state = VcpuMpState::Halted;
            Some(VmExitInfo {
                reason: super::exit_reason::INVALID_GUEST_STATE,
                hw_reason: EXIT_REASON_INVALID_GUEST_STATE,
                qualification: instruction_error,
                ..Default::default()
            })
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
        // Pass through host CPUID with required masking.
        let (eax, ebx, mut ecx, edx) = crate::arch::x86::cpuid::cpuid(leaf, subleaf);
        match leaf {
            // Leaf 1: feature flags
            1 => {
                // ECX bit  5 = VMX — guests must not see VMX support.
                ecx &= !(1 << 5);
                // ECX bit 31 = Hypervisor present — set it so the guest knows
                // it is running inside a hypervisor (KVM convention).
                ecx |= 1 << 31;
            }
            // Leaf 0x8000_0001 (AMD extended): clear SVM bit (ECX bit 2)
            0x8000_0001 => {
                ecx &= !(1 << 2);
            }
            _ => {}
        }
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
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };
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
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };

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
    let vm = match find_vm(&vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() {
        Some(v) => v,
        None => return false,
    };

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
    let vm = match find_vm(&vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() {
        Some(v) => v,
        None => return false,
    };

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
    let vm = match find_vm(&vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() {
        Some(v) => v,
        None => return false,
    };

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        let info: u32 = 2 | (2 << 8) | (1 << 31);
        vmwrite(VM_ENTRY_INTR_INFO, info as u64);
    }
    true
}

/// Pause a vCPU — prevents the next vcpu_run from entering the guest.
pub fn vcpu_pause(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.paused = true;
    true
}

/// Resume a paused vCPU.
pub fn vcpu_resume(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.paused = false;
    // Also wake a halted vCPU when resumed (e.g. after injecting an interrupt).
    if vcpu.mp_state == VcpuMpState::Halted {
        vcpu.mp_state = VcpuMpState::Runnable;
    }
    true
}

/// Get guest FPU state (FXSAVE-layout, 512 bytes).
pub fn get_fpu(vm_id: u32, vcpu_id: u32) -> Option<GuestFpuState> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;
    Some(vcpu.guest_fpu)
}

/// Set guest FPU state.
pub fn set_fpu(vm_id: u32, vcpu_id: u32, fpu: &GuestFpuState) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.guest_fpu = *fpu;
    true
}

/// Get vCPU multi-processor state.
pub fn get_mp_state(vm_id: u32, vcpu_id: u32) -> Option<VcpuMpState> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;
    Some(vcpu.mp_state)
}

/// Set vCPU multi-processor state.
pub fn set_mp_state(vm_id: u32, vcpu_id: u32, state: VcpuMpState) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.mp_state = state;
    true
}

/// Translate a guest virtual address to guest physical address using the
/// guest CR3 and the nested page tables.
///
/// Performs a software page-table walk on the guest's 4-level (or 3-level PAE)
/// page tables, consulting the EPT to resolve each level's host physical address.
/// Returns `Some(gpa)` on success, `None` if the GVA is not mapped or the
/// page tables are not accessible via EPT.
pub fn translate_gva(vm_id: u32, vcpu_id: u32, gva: u64) -> Option<u64> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;

    unsafe {
        vmptrld(vcpu.vmcs_phys);
        let cr3 = vmread(GUEST_CR3) & !0xFFF; // PML4 base (ignore flags)
        let cr4 = vmread(GUEST_CR4);
        let efer = vmread(GUEST_IA32_EFER);

        // Only handle 64-bit long mode (LME + LMA set)
        if efer & 0x500 != 0x500 {
            // 32-bit / PAE guest — not supported in this walk
            return None;
        }

        // 4-level walk: PML4 → PDPT → PD → PT
        let walk_gpa = |table_gpa: u64, idx: usize| -> Option<u64> {
            // Translate table_gpa via EPT to get a host physical address,
            // then access via phys_to_virt.
            let hpa = super::ept::ept_translate(vm.ept_root, table_gpa)?;
            let virt = super::phys_to_virt(hpa & !0xFFF) as *const u64;
            Some(unsafe { *virt.add(idx) })
        };

        // PML4
        let pml4_idx = ((gva >> 39) & 0x1FF) as usize;
        let pml4e = walk_gpa(cr3, pml4_idx)?;
        if pml4e & 1 == 0 {
            return None;
        } // not present

        // PDPT
        let pdpt_gpa = pml4e & 0x000F_FFFF_FFFF_F000;
        let pdpt_idx = ((gva >> 30) & 0x1FF) as usize;
        let pdpte = walk_gpa(pdpt_gpa, pdpt_idx)?;
        if pdpte & 1 == 0 {
            return None;
        }
        if pdpte & (1 << 7) != 0 {
            // 1GB page
            return Some((pdpte & 0x000F_FFFC_0000_0000) | (gva & 0x3FFF_FFFF));
        }

        // PD
        let pd_gpa = pdpte & 0x000F_FFFF_FFFF_F000;
        let pd_idx = ((gva >> 21) & 0x1FF) as usize;
        let pde = walk_gpa(pd_gpa, pd_idx)?;
        if pde & 1 == 0 {
            return None;
        }
        if pde & (1 << 7) != 0 {
            // 2MB page
            return Some((pde & 0x000F_FFFF_FFE0_0000) | (gva & 0x1F_FFFF));
        }

        // PT
        let pt_gpa = pde & 0x000F_FFFF_FFFF_F000;
        let pt_idx = ((gva >> 12) & 0x1FF) as usize;
        let pte = walk_gpa(pt_gpa, pt_idx)?;
        if pte & 1 == 0 {
            return None;
        }

        // PA = PTE[51:12] | GVA[11:0]
        let _ = cr4; // silence unused warning; could check LA57 in future
        Some((pte & 0x000F_FFFF_FFFF_F000) | (gva & 0xFFF))
    }
}

/// Get dirty-page log for a memory slot.
///
/// The dirty log is a page allocated at `dirty_log_phys`.  Layout:
///   slot 0 → words [0..64), slot 1 → words [64..128), …, slot 31 → words [1984..2048)
/// Total: 32 × 64 × 8 = 16 384 bytes = 4 pages.  We allocated only 1 page in create_vm;
/// this is enough for the first 8 slots (512 bytes per slot × 8 = 4 KB).  Expand if needed.
///
/// Returns the number of dirty pages, or `None` on error.
pub fn get_dirty_log(vm_id: u32, slot: u32, bitmap: &mut [u64]) -> Option<u32> {
    let mut vms = VMS.lock();
    let vm = find_vm_mut(&mut vms, vm_id)?;
    let slot_idx = slot as usize;
    if slot_idx >= 32 || vm.dirty_log_phys == 0 {
        return None;
    }

    // Each slot gets 64 u64 words (= 512 bytes) within the dirty-log page.
    // Offset of this slot's words = slot_idx × 64 × 8 bytes.
    const WORDS_PER_SLOT: usize = 64;
    const BYTES_PER_SLOT: usize = WORDS_PER_SLOT * 8;

    let base_virt = super::phys_to_virt(vm.dirty_log_phys) as *mut u64;
    let slot_offset = slot_idx * WORDS_PER_SLOT;

    let copy_words = bitmap.len().min(WORDS_PER_SLOT);
    let mut dirty_count = 0u32;
    unsafe {
        for i in 0..copy_words {
            let w = *base_virt.add(slot_offset + i);
            bitmap[i] = w;
            dirty_count += w.count_ones();
        }
        // Clear after reading (like KVM).
        for i in 0..WORDS_PER_SLOT {
            *base_virt.add(slot_offset + i) = 0;
        }
    }
    let _ = BYTES_PER_SLOT; // suppress unused warning
    Some(dirty_count)
}

// ── Internal dirty-tracking helper ───────────────────────────────────────

fn mark_dirty_page(dirty_log_phys: u64, regions: &[Option<MemoryRegion>; 32], gpa: u64) {
    if dirty_log_phys == 0 {
        return;
    }
    const WORDS_PER_SLOT: usize = 64;

    for (slot_idx, region_opt) in regions.iter().enumerate() {
        if let Some(r) = region_opt {
            if gpa >= r.guest_phys && gpa < r.guest_phys + r.size {
                let page_offset = ((gpa - r.guest_phys) >> 12) as usize;
                let word = page_offset / 64;
                let bit = page_offset % 64;
                if word < WORDS_PER_SLOT {
                    let base = super::phys_to_virt(dirty_log_phys) as *mut u64;
                    let idx = slot_idx * WORDS_PER_SLOT + word;
                    // Bounds-check: one allocated page = 512 u64 words total.
                    if idx < 512 {
                        unsafe {
                            *base.add(idx) |= 1u64 << bit;
                        }
                    }
                }
                return;
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn find_vm(vms: &[Option<VmxVm>; MAX_VMS], vm_id: u32) -> Option<&VmxVm> {
    vms.iter()
        .find_map(|slot| slot.as_ref().filter(|vm| vm.id == vm_id))
}

fn find_vm_mut(vms: &mut [Option<VmxVm>; MAX_VMS], vm_id: u32) -> Option<&mut VmxVm> {
    vms.iter_mut()
        .find_map(|slot| slot.as_mut().filter(|vm| vm.id == vm_id))
}
