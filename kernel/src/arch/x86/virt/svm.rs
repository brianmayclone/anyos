//! AMD-V (SVM) support.
//!
//! Implements VMCB management, VMRUN, and exit handling for AMD processors.

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    alloc_page_zeroed, free_page, rdmsr, wrmsr,
    CpuidEntry, GuestGprs, GuestSregs, MemoryRegion, VmExitInfo,
};
use crate::sync::spinlock::Spinlock;

// ── MSR addresses ────────────────────────────────────────────────────────

const MSR_EFER: u32 = 0xC000_0080;
const MSR_VM_HSAVE_PA: u32 = 0xC001_0117;

// ── SVM exit codes ───────────────────────────────────────────────────────

pub const VMEXIT_CPUID: u64 = 0x72;
pub const VMEXIT_HLT: u64 = 0x78;
pub const VMEXIT_IOIO: u64 = 0x7B;
pub const VMEXIT_MSR: u64 = 0x7C;
pub const VMEXIT_SHUTDOWN: u64 = 0x7F;
pub const VMEXIT_NPF: u64 = 0x400;

// ── VMCB structures ─────────────────────────────────────────────────────

/// VMCB control area (offset 0x000-0x3FF).
#[derive(Clone, Copy)]
#[repr(C)]
struct VmcbControl {
    intercept_cr_reads: u16,     // 0x000
    intercept_cr_writes: u16,    // 0x002
    intercept_dr_reads: u16,     // 0x004
    intercept_dr_writes: u16,    // 0x006
    intercept_exceptions: u32,   // 0x008
    intercepts: u64,             // 0x00C — misc intercepts (CPUID, HLT, I/O, MSR, etc.)
    _reserved1: [u8; 0x040 - 0x014],
    iopm_base_pa: u64,           // 0x040
    msrpm_base_pa: u64,          // 0x048
    tsc_offset: u64,             // 0x050
    guest_asid: u32,             // 0x058
    tlb_control: u8,             // 0x05C
    _reserved2: [u8; 0x060 - 0x05D],
    vintr: u64,                  // 0x060
    interrupt_shadow: u64,       // 0x068
    exit_code: u64,              // 0x070
    exit_info1: u64,             // 0x078
    exit_info2: u64,             // 0x080
    exit_int_info: u64,          // 0x088
    np_enable: u64,              // 0x090 — bit 0 enables NPT
    _reserved3: [u8; 0x0A8 - 0x098],
    event_inj: u64,              // 0x0A8 — EVENTINJ (event injection)
    ncr3: u64,                   // 0x0B0 — nested CR3 (NPT root)
    _reserved4: [u8; 0x400 - 0x0B8],
}

/// VMCB segment descriptor.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct VmcbSegment {
    selector: u16,
    attrib: u16,
    limit: u32,
    base: u64,
}

/// VMCB state-save area (offset 0x400-0xFFF).
#[derive(Clone, Copy)]
#[repr(C)]
struct VmcbStateSave {
    es: VmcbSegment,             // 0x400
    cs: VmcbSegment,             // 0x410
    ss: VmcbSegment,             // 0x420
    ds: VmcbSegment,             // 0x430
    fs: VmcbSegment,             // 0x440
    gs: VmcbSegment,             // 0x450
    gdtr: VmcbSegment,           // 0x460
    ldtr: VmcbSegment,           // 0x470
    idtr: VmcbSegment,           // 0x480
    tr: VmcbSegment,             // 0x490
    _reserved1: [u8; 0x4CB - 0x4A0],
    cpl: u8,                     // 0x4CB
    _reserved2: [u8; 0x4D0 - 0x4CC],
    efer: u64,                   // 0x4D0
    _reserved3: [u8; 0x548 - 0x4D8],
    cr4: u64,                    // 0x548
    cr3: u64,                    // 0x550
    cr0: u64,                    // 0x558
    dr7: u64,                    // 0x560
    dr6: u64,                    // 0x568
    rflags: u64,                 // 0x570
    rip: u64,                    // 0x578
    _reserved4: [u8; 0x5D8 - 0x580],
    rsp: u64,                    // 0x5D8
    _reserved5: [u8; 0x5F8 - 0x5E0],
    rax: u64,                    // 0x5F8
    _reserved6: [u8; 0x1000 - 0x600],
}

/// Full VMCB (4KB aligned).
#[repr(C, align(4096))]
struct Vmcb {
    control: VmcbControl,
    state: VmcbStateSave,
}

// ── Per-CPU state ────────────────────────────────────────────────────────

const MAX_CPUS: usize = 64;
static mut HSAVE_AREAS: [u64; MAX_CPUS] = [0; MAX_CPUS];

// ── VM storage ───────────────────────────────────────────────────────────

static NEXT_VM_ID: AtomicU32 = AtomicU32::new(1);
const MAX_VMS: usize = 64;
const MAX_VCPUS_PER_VM: usize = 16;

struct SvmVcpu {
    vmcb_phys: u64,
    /// Non-RAX GPRs (RAX is in the VMCB state-save area).
    guest_gprs: GuestGprs,
    iopm_phys: u64,
    msrpm_phys: u64,
}

struct SvmVm {
    id: u32,
    active: bool,
    vcpus: [Option<SvmVcpu>; MAX_VCPUS_PER_VM],
    npt_root: u64,
    memory_regions: [Option<MemoryRegion>; 32],
    cpuid_table: [Option<CpuidEntry>; 64],
    cpuid_count: usize,
}

static VMS: Spinlock<[Option<SvmVm>; MAX_VMS]> = Spinlock::new([const { None }; MAX_VMS]);

// ── SVM intercept bits ───────────────────────────────────────────────────

const INTERCEPT_CPUID: u64 = 1 << 18;
const INTERCEPT_HLT: u64 = 1 << 24;
const INTERCEPT_IOIO_PROT: u64 = 1 << 27;
const INTERCEPT_MSR_PROT: u64 = 1 << 28;
const INTERCEPT_SHUTDOWN: u64 = 1 << 31;

// ── Global init ──────────────────────────────────────────────────────────

pub fn global_init() {
    // Nothing needed globally.
}

// ── Per-CPU init ─────────────────────────────────────────────────────────

pub fn per_cpu_init() {
    let cpu_id = crate::arch::x86::apic::lapic_id() as usize;
    if cpu_id >= MAX_CPUS {
        return;
    }

    unsafe {
        // Set EFER.SVME (bit 12)
        let efer = rdmsr(MSR_EFER);
        wrmsr(MSR_EFER, efer | (1 << 12));

        // Allocate host save area
        let hsave = match alloc_page_zeroed() {
            Some(p) => p,
            None => {
                crate::serial_println!("SVM: CPU {} — failed to allocate host save area", cpu_id);
                return;
            }
        };

        // Write physical address to VM_HSAVE_PA MSR
        wrmsr(MSR_VM_HSAVE_PA, hsave);
        HSAVE_AREAS[cpu_id] = hsave;

        crate::serial_verbose_println!("SVM: CPU {} — SVME enabled, HSAVE at {:#x}", cpu_id, hsave);
    }
}

// ── VM lifecycle ─────────────────────────────────────────────────────────

pub fn create_vm() -> u32 {
    let id = NEXT_VM_ID.fetch_add(1, Ordering::Relaxed);

    let npt_root = match super::ept::create_npt_root() {
        Some(r) => r,
        None => return 0,
    };

    let vm = SvmVm {
        id,
        active: true,
        vcpus: [const { None }; MAX_VCPUS_PER_VM],
        npt_root,
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
    0
}

pub fn destroy_vm(vm_id: u32) -> bool {
    let mut vms = VMS.lock();
    for slot in vms.iter_mut() {
        if let Some(vm) = slot {
            if vm.id == vm_id {
                for vcpu_opt in vm.vcpus.iter_mut() {
                    if let Some(vcpu) = vcpu_opt.take() {
                        free_page(vcpu.vmcb_phys);
                        if vcpu.iopm_phys != 0 {
                            // IOPM is 12KB = 3 pages
                            free_page(vcpu.iopm_phys);
                            free_page(vcpu.iopm_phys + 0x1000);
                            free_page(vcpu.iopm_phys + 0x2000);
                        }
                        if vcpu.msrpm_phys != 0 {
                            // MSRPM is 8KB = 2 pages
                            free_page(vcpu.msrpm_phys);
                            free_page(vcpu.msrpm_phys + 0x1000);
                        }
                    }
                }
                super::ept::destroy_npt(vm.npt_root);
                *slot = None;
                return true;
            }
        }
    }
    false
}

pub fn set_memory(vm_id: u32, slot: u32, gpa: u64, size: u64, hpa: u64) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(vm) => vm,
        None => return false,
    };

    let slot_idx = slot as usize;
    if slot_idx >= vm.memory_regions.len() {
        return false;
    }

    vm.memory_regions[slot_idx] = Some(MemoryRegion {
        slot,
        guest_phys: gpa,
        size,
        host_phys: hpa,
    });

    super::ept::npt_map_range(vm.npt_root, gpa, hpa, size, true, true);

    // Flush stale nested-page TLB entries via INVLPGA.
    // INVLPGA flushes TLB entries for a given virtual address and ASID.
    // Using address=0 and ASID=0xFFFF_FFFF flushes all entries for all ASIDs.
    unsafe {
        core::arch::asm!(
            "invlpga rax, ecx",
            in("rax") 0u64,
            in("ecx") 0xFFFF_FFFFu32,
            options(nostack, preserves_flags),
        );
    }

    true
}

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
        let vmcb_phys = match alloc_page_zeroed() {
            Some(p) => p,
            None => return false,
        };

        // Allocate I/O permission map (3 contiguous pages = 12KB)
        let iopm_phys = match crate::memory::physical::alloc_contiguous(3) {
            Some(p) => {
                core::ptr::write_bytes(p.as_u64() as *mut u8, 0xFF, 3 * 4096);
                p.as_u64()
            }
            None => {
                free_page(vmcb_phys);
                return false;
            }
        };

        // Allocate MSR permission map (2 contiguous pages = 8KB)
        let msrpm_phys = match crate::memory::physical::alloc_contiguous(2) {
            Some(p) => {
                core::ptr::write_bytes(p.as_u64() as *mut u8, 0xFF, 2 * 4096);
                p.as_u64()
            }
            None => {
                free_page(vmcb_phys);
                free_page(iopm_phys);
                free_page(iopm_phys + 0x1000);
                free_page(iopm_phys + 0x2000);
                return false;
            }
        };

        // Initialize VMCB
        let vmcb = &mut *(vmcb_phys as *mut Vmcb);

        // Control area
        vmcb.control.intercepts = INTERCEPT_CPUID | INTERCEPT_HLT
            | INTERCEPT_IOIO_PROT | INTERCEPT_MSR_PROT | INTERCEPT_SHUTDOWN;
        vmcb.control.iopm_base_pa = iopm_phys;
        vmcb.control.msrpm_base_pa = msrpm_phys;
        vmcb.control.guest_asid = 1; // Must be non-zero
        vmcb.control.np_enable = 1;  // Enable nested paging
        vmcb.control.ncr3 = vm.npt_root;

        // State-save area: real mode defaults
        vmcb.state.cs.selector = 0xF000;
        vmcb.state.cs.base = 0xFFFF_0000;
        vmcb.state.cs.limit = 0xFFFF;
        vmcb.state.cs.attrib = 0x009B;
        vmcb.state.ds = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0093 };
        vmcb.state.es = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0093 };
        vmcb.state.fs = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0093 };
        vmcb.state.gs = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0093 };
        vmcb.state.ss = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0093 };
        vmcb.state.tr = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x008B };
        vmcb.state.ldtr = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0x0082 };
        vmcb.state.gdtr = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0 };
        vmcb.state.idtr = VmcbSegment { selector: 0, base: 0, limit: 0xFFFF, attrib: 0 };

        vmcb.state.cr0 = 0x0000_0030; // ET + NE
        vmcb.state.cr3 = 0;
        vmcb.state.cr4 = 0;
        vmcb.state.efer = (1 << 12); // SVME must be set in guest EFER
        vmcb.state.rip = 0xFFF0;
        vmcb.state.rsp = 0;
        vmcb.state.rflags = 0x2;
        vmcb.state.rax = 0;
        vmcb.state.dr7 = 0x400;
        vmcb.state.dr6 = 0xFFFF_0FF0;

        vm.vcpus[idx] = Some(SvmVcpu {
            vmcb_phys,
            guest_gprs: GuestGprs::default(),
            iopm_phys,
            msrpm_phys,
        });
    }

    true
}

/// Run a vCPU via VMRUN. Returns exit info.
pub fn vcpu_run(vm_id: u32, vcpu_id: u32) -> Option<VmExitInfo> {
    let mut vms = VMS.lock();
    let vm = find_vm_mut(&mut vms, vm_id)?;
    let idx = vcpu_id as usize;
    let vcpu = vm.vcpus[idx].as_mut()?;

    let vmcb_phys = vcpu.vmcb_phys;

    unsafe {
        let vmcb = &mut *(vmcb_phys as *mut Vmcb);

        // Write non-RAX GPRs into registers before VMRUN, read them back after.
        // RAX is in the VMCB state-save area.
        vmcb.state.rax = vcpu.guest_gprs.rax;

        let gprs_ptr = &mut vcpu.guest_gprs as *mut GuestGprs;

        // Drop lock before VMRUN
        drop(vms);

        // VMRUN: RAX = physical address of VMCB
        // We save/restore non-RAX GPRs around VMRUN.
        svm_vmrun(vmcb_phys, gprs_ptr);

        // Re-acquire lock
        let mut vms = VMS.lock();
        let vm = find_vm_mut(&mut vms, vm_id)?;
        let vcpu = vm.vcpus[idx].as_mut()?;

        let vmcb = &*(vmcb_phys as *const Vmcb);

        // Save RAX back from VMCB
        vcpu.guest_gprs.rax = vmcb.state.rax;

        let exit_code = vmcb.control.exit_code;
        let exit_info1 = vmcb.control.exit_info1;
        let exit_info2 = vmcb.control.exit_info2;

        // Handle CPUID internally
        if exit_code == VMEXIT_CPUID {
            handle_cpuid_exit(&vm.cpuid_table[..vm.cpuid_count], vcpu, vmcb_phys);
            return Some(VmExitInfo {
                reason: exit_code as u32,
                ..Default::default()
            });
        }

        let mut info = VmExitInfo {
            reason: exit_code as u32,
            qualification: exit_info1,
            guest_phys_addr: if exit_code == VMEXIT_NPF { exit_info2 } else { 0 },
            ..Default::default()
        };

        match exit_code {
            VMEXIT_IOIO => {
                // SVM exit_info1 for IOIO: bit 0 = IN(1)/OUT(0), bits 6:4 = size encoding,
                // bits 31:16 = port
                info.io_port = (exit_info1 >> 16) as u16;
                info.is_read = (exit_info1 & 1) as u8;
                let sz_bits = (exit_info1 >> 4) & 0x7;
                info.access_size = match sz_bits {
                    1 => 1, 2 => 2, 4 => 4, _ => 1,
                };
                if info.is_read == 0 {
                    info.io_data = vcpu.guest_gprs.rax;
                }
            }
            VMEXIT_NPF => {
                // AMD APM Vol. 2 §15.25.6, EXITINFO1 for #NPF:
                //   Bit 1 = the access was a write (W=1).
                //   Bit 0 = page was present at the time of the fault.
                // So: is_read = !(write bit), i.e. bit 1 == 0 → read access.
                info.is_read = if (exit_info1 >> 1) & 1 != 0 { 0 } else { 1 };
                info.access_size = 4; // default; instruction decode needed for exact size
                if info.is_read == 0 {
                    info.io_data = vcpu.guest_gprs.rax;
                }
            }
            VMEXIT_MSR => {
                info.msr_index = vcpu.guest_gprs.rcx as u32;
                info.is_read = if exit_info1 == 0 { 1 } else { 0 }; // 0=RDMSR, 1=WRMSR
                if info.is_read == 0 {
                    info.io_data = (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
                }
            }
            _ => {}
        }

        Some(info)
    }
}

/// VMRUN with GPR save/restore.
unsafe fn svm_vmrun(vmcb_phys: u64, gprs: *mut GuestGprs) {
    core::arch::asm!(
        // Save callee-saved
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rsi",  // save gprs pointer

        // Load guest GPRs (non-RAX; RAX is in VMCB)
        "mov rbx, [rsi + 0x08]",
        "mov rcx, [rsi + 0x10]",
        "mov rdx, [rsi + 0x18]",
        "mov rbp, [rsi + 0x30]",
        "mov r8,  [rsi + 0x38]",
        "mov r9,  [rsi + 0x40]",
        "mov r10, [rsi + 0x48]",
        "mov r11, [rsi + 0x50]",
        "mov r12, [rsi + 0x58]",
        "mov r13, [rsi + 0x60]",
        "mov r14, [rsi + 0x68]",
        "mov r15, [rsi + 0x70]",
        // Load guest rdi and rsi last
        "mov rdi, [rsi + 0x28]",
        "push rsi",              // save gprs pointer again
        "mov rsi, [rsi + 0x20]", // guest rsi

        // rax = VMCB physical address (already in rax from input)
        "vmrun",

        // After VMRUN: guest state saved in VMCB, host restored
        // Save guest GPRs. RSI was guest RSI, need gprs pointer from stack.
        "xchg rsi, [rsp]",  // rsi = gprs pointer, [rsp] = guest rsi

        "mov [rsi + 0x08], rbx",
        "mov [rsi + 0x10], rcx",
        "mov [rsi + 0x18], rdx",
        "pop rax",                // guest rsi
        "mov [rsi + 0x20], rax",
        "mov [rsi + 0x28], rdi",
        "mov [rsi + 0x30], rbp",
        "mov [rsi + 0x38], r8",
        "mov [rsi + 0x40], r9",
        "mov [rsi + 0x48], r10",
        "mov [rsi + 0x50], r11",
        "mov [rsi + 0x58], r12",
        "mov [rsi + 0x60], r13",
        "mov [rsi + 0x68], r14",
        "mov [rsi + 0x70], r15",

        // Restore host callee-saved
        "pop rsi",  // discard (was the gprs pointer push)
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",

        in("rax") vmcb_phys,
        in("rsi") gprs,
        options(nostack),
    );
}

/// Handle CPUID exit by emulating the instruction.
unsafe fn handle_cpuid_exit(cpuid_table: &[Option<CpuidEntry>], vcpu: &mut SvmVcpu, vmcb_phys: u64) {
    let vmcb = &mut *(vmcb_phys as *mut Vmcb);
    let leaf = vmcb.state.rax as u32;
    let subleaf = vcpu.guest_gprs.rcx as u32;

    let mut found = false;
    for entry_opt in cpuid_table {
        if let Some(entry) = entry_opt {
            if entry.function == leaf && entry.index == subleaf {
                vmcb.state.rax = entry.eax as u64;
                vcpu.guest_gprs.rbx = entry.ebx as u64;
                vcpu.guest_gprs.rcx = entry.ecx as u64;
                vcpu.guest_gprs.rdx = entry.edx as u64;
                found = true;
                break;
            }
        }
    }

    if !found {
        let (eax, ebx, mut ecx, edx) = crate::arch::x86::cpuid::cpuid(leaf, subleaf);
        match leaf {
            // Leaf 1: mask VMX capability and set hypervisor-present.
            1 => {
                ecx &= !(1 << 5);  // clear VMX (ECX bit 5)
                ecx |=  1 << 31;   // set Hypervisor Present (ECX bit 31)
            }
            // Leaf 0x8000_0001: mask SVM capability (ECX bit 2).
            0x8000_0001 => {
                ecx &= !(1 << 2);
            }
            _ => {}
        }
        vmcb.state.rax = eax as u64;
        vcpu.guest_gprs.rbx = ebx as u64;
        vcpu.guest_gprs.rcx = ecx as u64;
        vcpu.guest_gprs.rdx = edx as u64;
    }

    // Advance RIP past CPUID (2 bytes)
    vmcb.state.rip += 2;
}

pub fn get_regs(vm_id: u32, vcpu_id: u32) -> Option<GuestGprs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;
    let mut gprs = vcpu.guest_gprs;
    // RAX is in the VMCB
    unsafe {
        let vmcb = &*(vcpu.vmcb_phys as *const Vmcb);
        gprs.rax = vmcb.state.rax;
    }
    Some(gprs)
}

pub fn set_regs(vm_id: u32, vcpu_id: u32, gprs: &GuestGprs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() { Some(v) => v, None => return false };
    vcpu.guest_gprs = *gprs;
    unsafe {
        let vmcb = &mut *(vcpu.vmcb_phys as *mut Vmcb);
        vmcb.state.rax = gprs.rax;
    }
    true
}

pub fn get_sregs(vm_id: u32, vcpu_id: u32) -> Option<GuestSregs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_id as usize].as_ref()?;

    unsafe {
        let vmcb = &*(vcpu.vmcb_phys as *const Vmcb);
        Some(GuestSregs {
            cs_selector: vmcb.state.cs.selector,
            cs_base: vmcb.state.cs.base,
            cs_limit: vmcb.state.cs.limit,
            cs_ar: vmcb.state.cs.attrib as u32,
            ds_selector: vmcb.state.ds.selector,
            ds_base: vmcb.state.ds.base,
            ds_limit: vmcb.state.ds.limit,
            ds_ar: vmcb.state.ds.attrib as u32,
            es_selector: vmcb.state.es.selector,
            es_base: vmcb.state.es.base,
            es_limit: vmcb.state.es.limit,
            es_ar: vmcb.state.es.attrib as u32,
            fs_selector: vmcb.state.fs.selector,
            fs_base: vmcb.state.fs.base,
            fs_limit: vmcb.state.fs.limit,
            fs_ar: vmcb.state.fs.attrib as u32,
            gs_selector: vmcb.state.gs.selector,
            gs_base: vmcb.state.gs.base,
            gs_limit: vmcb.state.gs.limit,
            gs_ar: vmcb.state.gs.attrib as u32,
            ss_selector: vmcb.state.ss.selector,
            ss_base: vmcb.state.ss.base,
            ss_limit: vmcb.state.ss.limit,
            ss_ar: vmcb.state.ss.attrib as u32,
            tr_selector: vmcb.state.tr.selector,
            tr_base: vmcb.state.tr.base,
            tr_limit: vmcb.state.tr.limit,
            tr_ar: vmcb.state.tr.attrib as u32,
            ldtr_selector: vmcb.state.ldtr.selector,
            ldtr_base: vmcb.state.ldtr.base,
            ldtr_limit: vmcb.state.ldtr.limit,
            ldtr_ar: vmcb.state.ldtr.attrib as u32,
            gdtr_base: vmcb.state.gdtr.base,
            gdtr_limit: vmcb.state.gdtr.limit,
            idtr_base: vmcb.state.idtr.base,
            idtr_limit: vmcb.state.idtr.limit,
            cr0: vmcb.state.cr0,
            cr3: vmcb.state.cr3,
            cr4: vmcb.state.cr4,
            efer: vmcb.state.efer,
            rip: vmcb.state.rip,
            rsp: vmcb.state.rsp,
            rflags: vmcb.state.rflags,
        })
    }
}

pub fn set_sregs(vm_id: u32, vcpu_id: u32, sregs: &GuestSregs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_mut() { Some(v) => v, None => return false };

    unsafe {
        let vmcb = &mut *(vcpu.vmcb_phys as *mut Vmcb);
        vmcb.state.cs = VmcbSegment { selector: sregs.cs_selector, base: sregs.cs_base, limit: sregs.cs_limit, attrib: sregs.cs_ar as u16 };
        vmcb.state.ds = VmcbSegment { selector: sregs.ds_selector, base: sregs.ds_base, limit: sregs.ds_limit, attrib: sregs.ds_ar as u16 };
        vmcb.state.es = VmcbSegment { selector: sregs.es_selector, base: sregs.es_base, limit: sregs.es_limit, attrib: sregs.es_ar as u16 };
        vmcb.state.fs = VmcbSegment { selector: sregs.fs_selector, base: sregs.fs_base, limit: sregs.fs_limit, attrib: sregs.fs_ar as u16 };
        vmcb.state.gs = VmcbSegment { selector: sregs.gs_selector, base: sregs.gs_base, limit: sregs.gs_limit, attrib: sregs.gs_ar as u16 };
        vmcb.state.ss = VmcbSegment { selector: sregs.ss_selector, base: sregs.ss_base, limit: sregs.ss_limit, attrib: sregs.ss_ar as u16 };
        vmcb.state.tr = VmcbSegment { selector: sregs.tr_selector, base: sregs.tr_base, limit: sregs.tr_limit, attrib: sregs.tr_ar as u16 };
        vmcb.state.ldtr = VmcbSegment { selector: sregs.ldtr_selector, base: sregs.ldtr_base, limit: sregs.ldtr_limit, attrib: sregs.ldtr_ar as u16 };
        vmcb.state.gdtr.base = sregs.gdtr_base;
        vmcb.state.gdtr.limit = sregs.gdtr_limit;
        vmcb.state.idtr.base = sregs.idtr_base;
        vmcb.state.idtr.limit = sregs.idtr_limit;
        vmcb.state.cr0 = sregs.cr0;
        vmcb.state.cr3 = sregs.cr3;
        vmcb.state.cr4 = sregs.cr4;
        vmcb.state.efer = sregs.efer;
        vmcb.state.rip = sregs.rip;
        vmcb.state.rsp = sregs.rsp;
        vmcb.state.rflags = sregs.rflags;
    }

    true
}

pub fn inject_irq(vm_id: u32, vcpu_id: u32, vector: u8) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        let vmcb = &mut *(vcpu.vmcb_phys as *mut Vmcb);
        vmcb.control.event_inj = (vector as u64) | (0u64 << 8) | (1u64 << 31);
    }
    true
}

pub fn inject_exception(vm_id: u32, vcpu_id: u32, vector: u8, error_code: u32) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        let vmcb = &mut *(vcpu.vmcb_phys as *mut Vmcb);
        let has_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
        let mut info: u64 = (vector as u64)
            | (3 << 8)
            | (1u64 << 31);
        if has_error {
            info |= (1u64 << 11) | ((error_code as u64) << 32);
        }
        vmcb.control.event_inj = info;
    }
    true
}

pub fn inject_nmi(vm_id: u32, vcpu_id: u32) -> bool {
    let vms = VMS.lock();
    let vm = match find_vm(&vms, vm_id) { Some(v) => v, None => return false };
    let vcpu = match vm.vcpus[vcpu_id as usize].as_ref() { Some(v) => v, None => return false };

    unsafe {
        let vmcb = &mut *(vcpu.vmcb_phys as *mut Vmcb);
        let info: u64 = 2 | (2u64 << 8) | (1u64 << 31);
        vmcb.control.event_inj = info;
    }
    true
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn find_vm(vms: &[Option<SvmVm>; MAX_VMS], vm_id: u32) -> Option<&SvmVm> {
    vms.iter().find_map(|slot| slot.as_ref().filter(|vm| vm.id == vm_id))
}

fn find_vm_mut(vms: &mut [Option<SvmVm>; MAX_VMS], vm_id: u32) -> Option<&mut SvmVm> {
    vms.iter_mut().find_map(|slot| slot.as_mut().filter(|vm| vm.id == vm_id))
}
