//! AMD-V (SVM) support.
//!
//! Implements VMCB management, VMRUN, and exit handling for AMD processors.

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    alloc_page_zeroed, free_page, rdmsr, wrmsr, CpuidEntry, GuestFpuState, GuestGprs, GuestSregs,
    MemoryRegion, VcpuMpState, VmExitInfo,
};
use crate::sync::spinlock::Spinlock;

mod diagnostics;
mod state;
pub use state::{
    get_dirty_log, get_fpu, get_mp_state, get_regs, get_sregs, inject_exception, inject_irq,
    inject_nmi, set_fpu, set_mp_state, set_regs, set_sregs, translate_gva, vcpu_pause, vcpu_resume,
};

// ── MSR addresses ────────────────────────────────────────────────────────

const MSR_EFER: u32 = 0xC000_0080;
const MSR_VM_HSAVE_PA: u32 = 0xC001_0117;
const EFER_SVME: u64 = 1 << 12;

// ── SVM exit codes ───────────────────────────────────────────────────────

pub const VMEXIT_CPUID: u64 = 0x72;
pub const VMEXIT_INVD: u64 = 0x76;
pub const VMEXIT_PAUSE: u64 = 0x77;
pub const VMEXIT_HLT: u64 = 0x78;
pub const VMEXIT_INVLPG: u64 = 0x79;
pub const VMEXIT_INVLPGA: u64 = 0x7A;
pub const VMEXIT_IOIO: u64 = 0x7B;
pub const VMEXIT_MSR: u64 = 0x7C;
pub const VMEXIT_SHUTDOWN: u64 = 0x7F;
pub const VMEXIT_VMRUN: u64 = 0x80;
pub const VMEXIT_VMMCALL: u64 = 0x81;
pub const VMEXIT_WBINVD: u64 = 0x89;
pub const VMEXIT_XSETBV: u64 = 0x8D;
pub const VMEXIT_NPF: u64 = 0x400;
pub const VMEXIT_INVALID: u64 = u64::MAX;

// ── VMCB structures ─────────────────────────────────────────────────────

/// VMCB control area (offset 0x000-0x3FF).
#[derive(Clone, Copy)]
#[repr(C)]
struct VmcbControl {
    intercept_cr_reads: u16,   // 0x000
    intercept_cr_writes: u16,  // 0x002
    intercept_dr_reads: u16,   // 0x004
    intercept_dr_writes: u16,  // 0x006
    intercept_exceptions: u32, // 0x008
    intercepts_low: u32,       // 0x00C — misc intercepts low dword
    intercepts_high: u32,      // 0x010 — misc intercepts high dword
    _reserved1: [u8; 0x040 - 0x014],
    iopm_base_pa: u64,  // 0x040
    msrpm_base_pa: u64, // 0x048
    tsc_offset: u64,    // 0x050
    guest_asid: u32,    // 0x058
    tlb_control: u8,    // 0x05C
    _reserved2: [u8; 0x060 - 0x05D],
    vintr: u64,            // 0x060
    interrupt_shadow: u64, // 0x068
    exit_code: u64,        // 0x070
    exit_info1: u64,       // 0x078
    exit_info2: u64,       // 0x080
    exit_int_info: u64,    // 0x088
    np_enable: u64,        // 0x090 — bit 0 enables NPT
    _reserved3: [u8; 0x0A8 - 0x098],
    event_inj: u64,                // 0x0A8 — EVENTINJ (event injection)
    ncr3: u64,                     // 0x0B0 — nested CR3 (NPT root)
    _reserved4: u64,               // 0x0B8 — virtualization extensions, unused
    clean_bits: u32,               // 0x0C0
    _reserved5: u32,               // 0x0C4
    next_rip: u64,                 // 0x0C8 — valid when host supports NRIPS
    fetched_instruction_len: u8,   // 0x0D0 — decode assists, when supported
    fetched_instruction: [u8; 15], // 0x0D1
    _reserved6: [u8; 0x400 - 0x0E0],
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
    es: VmcbSegment,   // 0x000 (absolute VMCB offset 0x400)
    cs: VmcbSegment,   // 0x010
    ss: VmcbSegment,   // 0x020
    ds: VmcbSegment,   // 0x030
    fs: VmcbSegment,   // 0x040
    gs: VmcbSegment,   // 0x050
    gdtr: VmcbSegment, // 0x060
    ldtr: VmcbSegment, // 0x070
    idtr: VmcbSegment, // 0x080
    tr: VmcbSegment,   // 0x090
    _reserved1: [u8; 0x0CB - 0x0A0],
    cpl: u8, // 0x0CB
    _reserved2: [u8; 0x0D0 - 0x0CC],
    efer: u64, // 0x0D0
    _reserved3: [u8; 0x148 - 0x0D8],
    cr4: u64,    // 0x148
    cr3: u64,    // 0x150
    cr0: u64,    // 0x158
    dr7: u64,    // 0x160
    dr6: u64,    // 0x168
    rflags: u64, // 0x170
    rip: u64,    // 0x178
    _reserved4: [u8; 0x1D8 - 0x180],
    rsp: u64, // 0x1D8
    _reserved5: [u8; 0x1F8 - 0x1E0],
    rax: u64, // 0x1F8
    _reserved6: [u8; 0x0C00 - 0x200],
}

/// Full VMCB (4KB aligned).
#[repr(C, align(4096))]
struct Vmcb {
    control: VmcbControl,
    state: VmcbStateSave,
}

const _: [(); 0x400] = [(); core::mem::size_of::<VmcbControl>()];
const _: [(); 0x0C00] = [(); core::mem::size_of::<VmcbStateSave>()];
const _: [(); 0x1000] = [(); core::mem::size_of::<Vmcb>()];

// ── Per-CPU state ────────────────────────────────────────────────────────

const MAX_CPUS: usize = 64;
static mut HSAVE_AREAS: [u64; MAX_CPUS] = [0; MAX_CPUS];

// ── VM storage ───────────────────────────────────────────────────────────

static NEXT_VM_ID: AtomicU32 = AtomicU32::new(1);
const MAX_VMS: usize = 64;
const MAX_VCPUS_PER_VM: usize = 16;
const DIRTY_LOG_SLOTS: usize = 32;
const DIRTY_LOG_WORDS_PER_SLOT: usize = 64;
const DIRTY_LOG_TOTAL_WORDS: usize = DIRTY_LOG_SLOTS * DIRTY_LOG_WORDS_PER_SLOT;

struct SvmVcpu {
    vmcb_phys: u64,
    /// Non-RAX GPRs (RAX is in the VMCB state-save area).
    guest_gprs: GuestGprs,
    guest_fpu: GuestFpuState,
    iopm_phys: u64,
    msrpm_phys: u64,
    paused: bool,
    mp_state: VcpuMpState,
}

struct SvmVm {
    id: u32,
    active: bool,
    vcpus: [Option<SvmVcpu>; MAX_VCPUS_PER_VM],
    npt_root: u64,
    memory_regions: [Option<MemoryRegion>; 32],
    cpuid_table: [Option<CpuidEntry>; 64],
    cpuid_count: usize,
    dirty_log: [u64; DIRTY_LOG_TOTAL_WORDS],
}

static VMS: Spinlock<[Option<SvmVm>; MAX_VMS]> = Spinlock::new([const { None }; MAX_VMS]);

// ── SVM intercept bits ───────────────────────────────────────────────────

const INTERCEPT_CPUID: u64 = 1 << 18;
const INTERCEPT_HLT: u64 = 1 << 24;
const INTERCEPT_IOIO_PROT: u64 = 1 << 27;
const INTERCEPT_MSR_PROT: u64 = 1 << 28;
const INTERCEPT_SHUTDOWN: u64 = 1 << 31;
const INTERCEPT_VMRUN: u64 = 1 << 32;
const INTERCEPT_VMMCALL: u64 = 1 << 33;
const INTERCEPT_VMLOAD: u64 = 1 << 34;
const INTERCEPT_VMSAVE: u64 = 1 << 35;
const INTERCEPT_STGI: u64 = 1 << 36;
const INTERCEPT_CLGI: u64 = 1 << 37;
const INTERCEPT_SKINIT: u64 = 1 << 38;
const INTERCEPT_WBINVD: u64 = 1 << 41;
const INTERCEPT_XSETBV: u64 = 1 << 45;

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
        wrmsr(MSR_EFER, efer | EFER_SVME);

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
        dirty_log: [0; DIRTY_LOG_TOTAL_WORDS],
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

/// Register a userspace memory slot without assuming the backing host pages are
/// physically contiguous. The syscall layer maps each translated page
/// separately after registering the full slot for dirty logging and diagnostics.
pub fn register_memory_region(vm_id: u32, slot: u32, gpa: u64, size: u64, hpa: u64) -> bool {
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
    true
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

    flush_npt_tlb();

    true
}

/// Map a single 4KB page in the NPT for a VM (used by sys_vm_set_memory for pages
/// beyond the first, which are mapped without slot re-registration).
pub fn map_page_in_npt(vm_id: u32, gpa: u64, hpa: u64) -> bool {
    let mut vms = VMS.lock();
    if let Some(vm) = find_vm_mut(&mut vms, vm_id) {
        super::ept::npt_map_range(vm.npt_root, gpa, hpa, 0x1000, true, true);
        flush_npt_tlb();
        true
    } else {
        false
    }
}

fn flush_npt_tlb() {
    // Flush stale nested-page TLB entries via INVLPGA. Using address=0 and
    // ASID=0xFFFF_FFFF flushes all entries for all ASIDs.
    unsafe {
        core::arch::asm!(
            "invlpga rax, ecx",
            in("rax") 0u64,
            in("ecx") 0xFFFF_FFFFu32,
            options(nostack, preserves_flags),
        );
    }
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

    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    if vm.vcpus[idx].is_some() {
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
        let vmcb = &mut *(super::phys_to_virt(vmcb_phys) as *mut Vmcb);

        // Control area
        let intercepts = INTERCEPT_CPUID
            | INTERCEPT_HLT
            | INTERCEPT_IOIO_PROT
            | INTERCEPT_MSR_PROT
            | INTERCEPT_SHUTDOWN
            | INTERCEPT_VMRUN
            | INTERCEPT_VMMCALL
            | INTERCEPT_VMLOAD
            | INTERCEPT_VMSAVE
            | INTERCEPT_STGI
            | INTERCEPT_CLGI
            | INTERCEPT_SKINIT
            | INTERCEPT_WBINVD
            | INTERCEPT_XSETBV;
        vmcb.control.intercepts_low = intercepts as u32;
        vmcb.control.intercepts_high = (intercepts >> 32) as u32;
        vmcb.control.iopm_base_pa = iopm_phys;
        vmcb.control.msrpm_base_pa = msrpm_phys;
        vmcb.control.guest_asid = 1; // Must be non-zero
        vmcb.control.np_enable = 1; // Enable nested paging
        vmcb.control.ncr3 = vm.npt_root;

        // State-save area: real mode defaults
        vmcb.state.cs.selector = 0xF000;
        vmcb.state.cs.base = 0xFFFF_0000;
        vmcb.state.cs.limit = 0xFFFF;
        vmcb.state.cs.attrib = 0x009B;
        vmcb.state.ds = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0093,
        };
        vmcb.state.es = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0093,
        };
        vmcb.state.fs = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0093,
        };
        vmcb.state.gs = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0093,
        };
        vmcb.state.ss = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0093,
        };
        vmcb.state.tr = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x008B,
        };
        vmcb.state.ldtr = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0x0082,
        };
        vmcb.state.gdtr = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0,
        };
        vmcb.state.idtr = VmcbSegment {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attrib: 0,
        };

        vmcb.state.cr0 = 0x0000_0030; // ET + NE (real mode)
        vmcb.state.cr3 = 0;
        vmcb.state.cr4 = 0;
        // SVM's VMRUN consistency checks require EFER.SVME in the VMCB EFER
        // field. AVM keeps that bit invisible through get_sregs/RDMSR.
        vmcb.state.efer = avm_efer_to_svm(0);
        vmcb.state.rip = 0xFFF0;
        vmcb.state.rsp = 0;
        vmcb.state.rflags = 0x2;
        vmcb.state.rax = 0;
        vmcb.state.dr7 = 0x400;
        vmcb.state.dr6 = 0xFFFF_0FF0;

        vm.vcpus[idx] = Some(SvmVcpu {
            vmcb_phys,
            guest_gprs: GuestGprs::default(),
            guest_fpu: GuestFpuState::default(),
            iopm_phys,
            msrpm_phys,
            paused: false,
            mp_state: VcpuMpState::Runnable,
        });
    }

    true
}

/// Run a vCPU via VMRUN. Returns exit info.
pub fn vcpu_run(vm_id: u32, vcpu_id: u32) -> Option<VmExitInfo> {
    let mut vms = VMS.lock();
    let vm = find_vm_mut(&mut vms, vm_id)?;
    let idx = vcpu_index(vcpu_id)?;
    let vcpu = vm.vcpus[idx].as_mut()?;

    // Refuse to enter a paused or halted vCPU.
    if vcpu.paused || vcpu.mp_state == VcpuMpState::Halted {
        return None;
    }

    let vmcb_phys = vcpu.vmcb_phys;

    unsafe {
        let vmcb = &mut *(super::phys_to_virt(vmcb_phys) as *mut Vmcb);

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

        let vmcb = &mut *(super::phys_to_virt(vmcb_phys) as *mut Vmcb);

        // Save RAX back from VMCB
        vcpu.guest_gprs.rax = vmcb.state.rax;
        vcpu.guest_gprs.rsp = vmcb.state.rsp;

        let exit_code = vmcb.control.exit_code;
        let exit_info1 = vmcb.control.exit_info1;
        let exit_info2 = vmcb.control.exit_info2;
        let instr_len = svm_exit_instruction_len(exit_code, vmcb);

        if exit_code == VMEXIT_INVALID {
            vcpu.mp_state = VcpuMpState::Halted;
            crate::serial_println!(
                "[svm] vmentry failed: invalid guest state rip={:#x} cr0={:#x} cr3={:#x} cr4={:#x} efer={:#x} cpl={} cs={:#x}/{:#x} ds={:#x}/{:#x} ss={:#x}/{:#x} tr={:#x}/{:#x} ldtr={:#x}/{:#x} gdtr={:#x}:{:#x} idtr={:#x}:{:#x}",
                vmcb.state.rip,
                vmcb.state.cr0,
                vmcb.state.cr3,
                vmcb.state.cr4,
                vmcb.state.efer,
                vmcb.state.cpl,
                vmcb.state.cs.selector,
                vmcb.state.cs.attrib,
                vmcb.state.ds.selector,
                vmcb.state.ds.attrib,
                vmcb.state.ss.selector,
                vmcb.state.ss.attrib,
                vmcb.state.tr.selector,
                vmcb.state.tr.attrib,
                vmcb.state.ldtr.selector,
                vmcb.state.ldtr.attrib,
                vmcb.state.gdtr.base,
                vmcb.state.gdtr.limit,
                vmcb.state.idtr.base,
                vmcb.state.idtr.limit,
            );
            return Some(VmExitInfo {
                reason: super::exit_reason::INVALID_GUEST_STATE,
                hw_reason: u32::MAX,
                qualification: exit_info1,
                guest_phys_addr: exit_info2,
                instruction_len: instr_len,
                ..Default::default()
            });
        }

        if exit_code == VMEXIT_SHUTDOWN {
            diagnostics::log_shutdown(vm.npt_root, vcpu, vmcb);
        } else {
            crate::serial_println!(
                "[svm] vmexit: code={:#x} info1={:#x} info2={:#x} rip={:#x} cs.base={:#x}",
                exit_code,
                exit_info1,
                exit_info2,
                vmcb.state.rip,
                vmcb.state.cs.base
            );
        }

        // Handle CPUID internally — fill registers, advance RIP, return synthetic reason.
        if exit_code == VMEXIT_CPUID {
            handle_cpuid_exit(&vm.cpuid_table[..vm.cpuid_count], vcpu, vmcb_phys);
            return Some(VmExitInfo {
                reason: super::exit_reason::CPUID_EMULATED,
                hw_reason: exit_code as u32,
                ..Default::default()
            });
        }

        // Map SVM exit code → portable exit_reason::* value.
        let reason = match exit_code {
            VMEXIT_CPUID => super::exit_reason::CPUID,
            VMEXIT_HLT => super::exit_reason::HLT,
            VMEXIT_IOIO => super::exit_reason::IO_INSTRUCTION,
            VMEXIT_MSR => {
                // exit_info1: 0=RDMSR, 1=WRMSR
                if exit_info1 == 0 {
                    super::exit_reason::RDMSR
                } else {
                    super::exit_reason::WRMSR
                }
            }
            VMEXIT_SHUTDOWN => super::exit_reason::SHUTDOWN,
            VMEXIT_INVALID => super::exit_reason::INVALID_GUEST_STATE,
            VMEXIT_NPF => super::exit_reason::EPT_VIOLATION,
            VMEXIT_VMRUN | VMEXIT_VMMCALL => super::exit_reason::VMCALL,
            VMEXIT_INVD => super::exit_reason::INVD,
            VMEXIT_PAUSE => super::exit_reason::PAUSE,
            VMEXIT_INVLPG | VMEXIT_INVLPGA => super::exit_reason::INVLPG,
            VMEXIT_WBINVD => super::exit_reason::WBINVD,
            VMEXIT_XSETBV => super::exit_reason::XSETBV,
            0x7E => super::exit_reason::SMI,
            _ => exit_code as u32,
        };

        let guest_phys = if exit_code == VMEXIT_NPF {
            exit_info2
        } else {
            0
        };

        let mut info = VmExitInfo {
            reason,
            hw_reason: exit_code as u32,
            qualification: exit_info1,
            guest_phys_addr: guest_phys,
            instruction_len: instr_len,
            ..Default::default()
        };

        match exit_code {
            VMEXIT_HLT => {
                vmcb.state.rip = vmcb.state.rip.wrapping_add(instr_len.max(1) as u64);
                vcpu.mp_state = VcpuMpState::Halted;
                info.reason = super::exit_reason::HLT_EMULATED;
            }
            VMEXIT_IOIO => {
                // AMD APM Vol. 2 §15.10.2, EXITINFO1 for IOIO:
                //   bit 0   = IN(1) / OUT(0)
                //   bits 6:4 = data size (SZ8=1, SZ16=2, SZ32=4)
                //   bits 31:16 = port number
                info.io_port = (exit_info1 >> 16) as u16;
                info.is_read = (exit_info1 & 1) as u8;
                let sz_bits = (exit_info1 >> 4) & 0x7;
                info.access_size = match sz_bits {
                    1 => 1,
                    2 => 2,
                    4 => 4,
                    _ => 1,
                };
                if info.is_read == 0 {
                    info.io_data = vcpu.guest_gprs.rax;
                }
            }
            VMEXIT_NPF => {
                // AMD APM Vol. 2 §15.25.6: EXITINFO1 bit 1 = Write (W=1).
                let is_write = (exit_info1 >> 1) & 1;
                info.is_read = if is_write != 0 { 0 } else { 1 };
                info.access_size = 4;
                if is_write != 0 {
                    info.io_data = vcpu.guest_gprs.rax;
                    mark_dirty_page(&mut vm.dirty_log, &vm.memory_regions, exit_info2);
                }
            }
            VMEXIT_MSR => {
                info.msr_index = vcpu.guest_gprs.rcx as u32;
                info.is_read = if exit_info1 == 0 { 1 } else { 0 };
                if exit_info1 != 0 {
                    // WRMSR: value = EDX:EAX
                    info.io_data =
                        (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
                }
            }
            VMEXIT_VMRUN | VMEXIT_VMMCALL => {
                info.io_data = vcpu.guest_gprs.rax; // hypercall number
                info.io_data2 = vcpu.guest_gprs.rbx; // first arg
            }
            VMEXIT_XSETBV => {
                info.msr_index = vcpu.guest_gprs.rcx as u32;
                info.io_data = (vcpu.guest_gprs.rdx << 32) | (vcpu.guest_gprs.rax & 0xFFFF_FFFF);
            }
            _ => {}
        }

        Some(info)
    }
}

fn avm_segment_attr_to_svm(ar: u32) -> u16 {
    // AVM exposes VMX/KVM-style access-right bytes; SVM stores AVL/L/DB/G in
    // bits 8..11 instead of 12..15.
    let low = ar & 0x00ff;
    let vmx_high = (ar >> 12) & 0x0f;
    if vmx_high != 0 || (ar & 0x1_0000) != 0 {
        (low | (vmx_high << 8)) as u16
    } else {
        (ar & 0x0fff) as u16
    }
}

fn svm_segment_attr_to_avm(attr: u16) -> u32 {
    // Return the public AVM encoding so userspace sees identical sregs on VMX
    // and SVM backends.
    let attr = attr as u32;
    (attr & 0x00ff) | ((attr & 0x0f00) << 4)
}

fn avm_efer_to_svm(efer: u64) -> u64 {
    efer | EFER_SVME
}

fn svm_efer_to_avm(efer: u64) -> u64 {
    efer & !EFER_SVME
}

fn svm_exit_instruction_len(exit_code: u64, vmcb: &Vmcb) -> u32 {
    let next_rip = vmcb.control.next_rip;
    if next_rip > vmcb.state.rip {
        let len = next_rip - vmcb.state.rip;
        if len <= 15 {
            return len as u32;
        }
    }

    let fetched_len = vmcb.control.fetched_instruction_len as u32;
    if (1..=15).contains(&fetched_len) {
        return fetched_len;
    }

    match exit_code {
        VMEXIT_CPUID | VMEXIT_MSR => 2,
        VMEXIT_HLT => 1,
        VMEXIT_XSETBV | VMEXIT_VMRUN | VMEXIT_VMMCALL => 3,
        _ => 0,
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
        "push rdi",
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",

        // Load guest GPRs (non-RAX; RAX is in VMCB)
        "mov rbx, [rsi + 0x08]",
        "mov rcx, [rsi + 0x10]",
        "mov rdx, [rsi + 0x18]",
        "mov rbp, [rsi + 0x30]",
        "mov r8,  [rsi + 0x40]",
        "mov r9,  [rsi + 0x48]",
        "mov r10, [rsi + 0x50]",
        "mov r11, [rsi + 0x58]",
        "mov r12, [rsi + 0x60]",
        "mov r13, [rsi + 0x68]",
        "mov r14, [rsi + 0x70]",
        "mov r15, [rsi + 0x78]",
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
        "mov [rsi + 0x40], r8",
        "mov [rsi + 0x48], r9",
        "mov [rsi + 0x50], r10",
        "mov [rsi + 0x58], r11",
        "mov [rsi + 0x60], r12",
        "mov [rsi + 0x68], r13",
        "mov [rsi + 0x70], r14",
        "mov [rsi + 0x78], r15",

        // Restore host registers.
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "pop rdi",
        "pop rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",

        inout("rax") vmcb_phys => _,
        in("rsi") gprs,
    );
}

/// Handle CPUID exit by emulating the instruction.
unsafe fn handle_cpuid_exit(
    cpuid_table: &[Option<CpuidEntry>],
    vcpu: &mut SvmVcpu,
    vmcb_phys: u64,
) {
    let vmcb = &mut *(super::phys_to_virt(vmcb_phys) as *mut Vmcb);
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
                ecx &= !(1 << 5); // clear VMX (ECX bit 5)
                ecx |= 1 << 31; // set Hypervisor Present (ECX bit 31)
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

// ── Internal dirty-tracking helper ───────────────────────────────────────

fn mark_dirty_page(
    dirty_log: &mut [u64; DIRTY_LOG_TOTAL_WORDS],
    regions: &[Option<MemoryRegion>; 32],
    gpa: u64,
) {
    for (slot_idx, region_opt) in regions.iter().enumerate() {
        if let Some(r) = region_opt {
            if gpa >= r.guest_phys && gpa < r.guest_phys + r.size {
                let page_offset = ((gpa - r.guest_phys) >> 12) as usize;
                let word = page_offset / 64;
                let bit = page_offset % 64;
                if word < DIRTY_LOG_WORDS_PER_SLOT {
                    let idx = slot_idx * DIRTY_LOG_WORDS_PER_SLOT + word;
                    dirty_log[idx] |= 1u64 << bit;
                }
                return;
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn vcpu_index(vcpu_id: u32) -> Option<usize> {
    let idx = vcpu_id as usize;
    if idx < MAX_VCPUS_PER_VM {
        Some(idx)
    } else {
        None
    }
}

fn find_vm(vms: &[Option<SvmVm>; MAX_VMS], vm_id: u32) -> Option<&SvmVm> {
    vms.iter()
        .find_map(|slot| slot.as_ref().filter(|vm| vm.id == vm_id))
}

fn find_vm_mut(vms: &mut [Option<SvmVm>; MAX_VMS], vm_id: u32) -> Option<&mut SvmVm> {
    vms.iter_mut()
        .find_map(|slot| slot.as_mut().filter(|vm| vm.id == vm_id))
}
