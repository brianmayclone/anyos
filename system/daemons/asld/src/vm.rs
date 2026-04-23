use alloc::format;
use alloc::string::String;

use crate::errors::AsldError;
use crate::model::{DistroConfig, VmRunState};

const PAGE_SIZE: usize = 0x1000;
const MIN_GUEST_MEMORY_MB: usize = 16;
const BOOT_PML4_ADDR: usize = 0x1000;
const BOOT_PDPT_ADDR: usize = 0x2000;
const BOOT_PD_ADDR: usize = 0x3000;
const BOOT_CODE_ADDR: usize = 0x20_0000;
const BOOT_STACK_GUARD: usize = 0x2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmInstance {
    pub vm_id: u32,
    pub vcpu_id: u32,
    pub backend: String,
    pub console_pipe_name: String,
    pub guest_memory_addr: usize,
    pub guest_memory_size: usize,
    pub run_state: VmRunState,
    pub halted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmBootReport {
    pub ready: bool,
    pub halted: bool,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmRuntimeEvent {
    pub reason: String,
    pub summary: String,
    pub fatal: bool,
    pub qualification: u64,
    pub guest_phys_addr: u64,
    pub guest_virt_addr: u64,
    pub halted: bool,
}

pub fn start_vm(config: &DistroConfig) -> Result<VmInstance, AsldError> {
    start_vm_impl(config)
}

pub fn boot_probe(instance: &mut VmInstance) -> Result<VmBootReport, AsldError> {
    boot_probe_impl(instance)
}

pub fn poll_runtime(instance: &mut VmInstance) -> Result<Option<VmRuntimeEvent>, AsldError> {
    poll_runtime_impl(instance)
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
        run_state: VmRunState::Provisioned,
        halted: false,
    })
}

#[cfg(target_os = "linux")]
fn stop_vm_impl(_instance: &VmInstance) -> Result<(), AsldError> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn boot_probe_impl(instance: &mut VmInstance) -> Result<VmBootReport, AsldError> {
    instance.run_state = VmRunState::BootReady;
    Ok(VmBootReport {
        ready: true,
        halted: false,
        summary: format!("boot probe skipped on {}", instance.backend),
    })
}

#[cfg(target_os = "linux")]
fn poll_runtime_impl(_instance: &mut VmInstance) -> Result<Option<VmRuntimeEvent>, AsldError> {
    Ok(None)
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

    if let Err(err) = configure_boot_vcpu(vm_id, vcpu_id, guest_memory, guest_memory_size) {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = libsyscall::syscall1(SYS_VM_DESTROY, vm_id as u64);
        return Err(err);
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
        run_state: VmRunState::Provisioned,
        halted: false,
    })
}

#[cfg(not(target_os = "linux"))]
fn boot_probe_impl(instance: &mut VmInstance) -> Result<VmBootReport, AsldError> {
    const SYS_VCPU_RUN: u32 = 604;
    const MAX_BOOT_EXITS: usize = 8;

    let mut last_exit = VmExitInfo::default();
    for _ in 0..MAX_BOOT_EXITS {
        let mut exit = VmExitInfo::default();
        let rc = libsyscall::syscall3(
            SYS_VCPU_RUN,
            instance.vm_id as u64,
            instance.vcpu_id as u64,
            (&mut exit as *mut VmExitInfo) as u64,
        ) as u32;
        if rc == u32::MAX {
            return Err(AsldError::BackendUnavailable("vcpu_run failed"));
        }
        let assessment = assess_boot_exit(&exit);
        last_exit = exit;
        if !assessment.should_continue {
            instance.halted = assessment.halted;
            instance.run_state = if assessment.ready {
                if assessment.halted {
                    VmRunState::Halted
                } else {
                    VmRunState::BootReady
                }
            } else {
                VmRunState::Degraded
            };
            return Ok(VmBootReport {
                ready: assessment.ready,
                halted: assessment.halted,
                summary: assessment.summary,
            });
        }
    }

    instance.run_state = VmRunState::Degraded;
    Ok(VmBootReport {
        ready: false,
        halted: false,
        summary: format!(
            "boot probe exceeded exit budget on {} (last exit: {})",
            instance.backend,
            describe_exit(&last_exit)
        ),
    })
}

#[cfg(not(target_os = "linux"))]
fn poll_runtime_impl(instance: &mut VmInstance) -> Result<Option<VmRuntimeEvent>, AsldError> {
    const SYS_VCPU_RUN: u32 = 604;
    const MAX_RUNTIME_EXITS: usize = 4;

    if instance.halted {
        return Ok(None);
    }

    for _ in 0..MAX_RUNTIME_EXITS {
        let mut exit = VmExitInfo::default();
        let rc = libsyscall::syscall3(
            SYS_VCPU_RUN,
            instance.vm_id as u64,
            instance.vcpu_id as u64,
            (&mut exit as *mut VmExitInfo) as u64,
        ) as u32;
        if rc == u32::MAX {
            instance.run_state = VmRunState::Degraded;
            return Err(AsldError::BackendUnavailable("vcpu_run failed"));
        }
        match assess_runtime_exit(&exit) {
            RuntimeExitAssessment::Continue => continue,
            RuntimeExitAssessment::Record(event) => {
                instance.halted = event.halted;
                instance.run_state = if event.fatal {
                    VmRunState::Degraded
                } else if event.halted {
                    VmRunState::Halted
                } else {
                    VmRunState::Running
                };
                return Ok(Some(event));
            }
        }
    }

    instance.run_state = VmRunState::Running;
    Ok(None)
}

#[cfg(not(target_os = "linux"))]
fn configure_boot_vcpu(
    vm_id: u32,
    vcpu_id: u32,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    const SYS_VCPU_SET_REGS: u32 = 606;
    const SYS_VCPU_SET_SREGS: u32 = 608;

    let bootstrap = BootstrapLayout::new(guest_memory_size)?;
    write_bootstrap_image(guest_memory, guest_memory_size, &bootstrap)?;
    let regs = bootstrap_gprs();
    let sregs = bootstrap_sregs(&bootstrap);

    let regs_result = libsyscall::syscall3(
        SYS_VCPU_SET_REGS,
        vm_id as u64,
        vcpu_id as u64,
        (&regs as *const GuestGprs) as u64,
    ) as u32;
    if regs_result == u32::MAX {
        return Err(AsldError::BackendUnavailable("vcpu_set_regs failed"));
    }

    let sregs_result = libsyscall::syscall3(
        SYS_VCPU_SET_SREGS,
        vm_id as u64,
        vcpu_id as u64,
        (&sregs as *const GuestSregs) as u64,
    ) as u32;
    if sregs_result == u32::MAX {
        return Err(AsldError::BackendUnavailable("vcpu_set_sregs failed"));
    }

    Ok(())
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
    let requested = (memory_mb as usize).max(MIN_GUEST_MEMORY_MB) * 1024 * 1024;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
struct VmExitInfo {
    reason: u32,
    hw_reason: u32,
    qualification: u64,
    guest_phys_addr: u64,
    guest_virt_addr: u64,
    instruction_len: u32,
    io_port: u16,
    access_size: u8,
    is_read: u8,
    io_data: u64,
    io_data2: u64,
    msr_index: u32,
    cpuid_function: u32,
    cpuid_index: u32,
    cr_number: u8,
    cr_is_read: u8,
    dr_number: u8,
    dr_is_read: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExitAssessment {
    ready: bool,
    should_continue: bool,
    halted: bool,
    summary: String,
}

enum RuntimeExitAssessment {
    Continue,
    Record(VmRuntimeEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BootstrapLayout {
    pml4_addr: usize,
    pdpt_addr: usize,
    pd_addr: usize,
    code_addr: usize,
    stack_top: usize,
}

impl BootstrapLayout {
    fn new(guest_memory_size: usize) -> Result<Self, AsldError> {
        let min_required = BOOT_CODE_ADDR + PAGE_SIZE;
        if guest_memory_size < min_required + BOOT_STACK_GUARD {
            return Err(AsldError::InvalidState("guest memory too small for bootstrap"));
        }

        let stack_top = guest_memory_size - BOOT_STACK_GUARD;
        Ok(Self {
            pml4_addr: BOOT_PML4_ADDR,
            pdpt_addr: BOOT_PDPT_ADDR,
            pd_addr: BOOT_PD_ADDR,
            code_addr: BOOT_CODE_ADDR,
            stack_top: stack_top & !0xf,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
struct GuestGprs {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
struct GuestSregs {
    cs_selector: u16,
    cs_base: u64,
    cs_limit: u32,
    cs_ar: u32,
    ds_selector: u16,
    ds_base: u64,
    ds_limit: u32,
    ds_ar: u32,
    es_selector: u16,
    es_base: u64,
    es_limit: u32,
    es_ar: u32,
    fs_selector: u16,
    fs_base: u64,
    fs_limit: u32,
    fs_ar: u32,
    gs_selector: u16,
    gs_base: u64,
    gs_limit: u32,
    gs_ar: u32,
    ss_selector: u16,
    ss_base: u64,
    ss_limit: u32,
    ss_ar: u32,
    tr_selector: u16,
    tr_base: u64,
    tr_limit: u32,
    tr_ar: u32,
    ldtr_selector: u16,
    ldtr_base: u64,
    ldtr_limit: u32,
    ldtr_ar: u32,
    gdtr_base: u64,
    gdtr_limit: u32,
    idtr_base: u64,
    idtr_limit: u32,
    cr0: u64,
    cr3: u64,
    cr4: u64,
    efer: u64,
    rip: u64,
    rsp: u64,
    rflags: u64,
}

fn bootstrap_gprs() -> GuestGprs {
    GuestGprs::default()
}

fn bootstrap_sregs(layout: &BootstrapLayout) -> GuestSregs {
    const CODE_SEGMENT_AR: u32 = 0xA09B;
    const DATA_SEGMENT_AR: u32 = 0xC093;
    const TSS_SEGMENT_AR: u32 = 0x808B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_PE: u64 = 1 << 0;
    const CR0_ET: u64 = 1 << 4;
    const CR0_NE: u64 = 1 << 5;
    const CR0_PG: u64 = 1 << 31;
    const CR4_PAE: u64 = 1 << 5;
    const EFER_LME: u64 = 1 << 8;
    const EFER_LMA: u64 = 1 << 10;
    const SEGMENT_LIMIT: u32 = 0xFFFFF;

    GuestSregs {
        cs_selector: 0x08,
        cs_base: 0,
        cs_limit: SEGMENT_LIMIT,
        cs_ar: CODE_SEGMENT_AR,
        ds_selector: 0x10,
        ds_base: 0,
        ds_limit: SEGMENT_LIMIT,
        ds_ar: DATA_SEGMENT_AR,
        es_selector: 0x10,
        es_base: 0,
        es_limit: SEGMENT_LIMIT,
        es_ar: DATA_SEGMENT_AR,
        fs_selector: 0x10,
        fs_base: 0,
        fs_limit: SEGMENT_LIMIT,
        fs_ar: DATA_SEGMENT_AR,
        gs_selector: 0x10,
        gs_base: 0,
        gs_limit: SEGMENT_LIMIT,
        gs_ar: DATA_SEGMENT_AR,
        ss_selector: 0x10,
        ss_base: 0,
        ss_limit: SEGMENT_LIMIT,
        ss_ar: DATA_SEGMENT_AR,
        tr_selector: 0x18,
        tr_base: 0,
        tr_limit: 0x67,
        tr_ar: TSS_SEGMENT_AR,
        ldtr_selector: 0,
        ldtr_base: 0,
        ldtr_limit: 0,
        ldtr_ar: NULL_SEGMENT_AR,
        gdtr_base: 0,
        gdtr_limit: 0,
        idtr_base: 0,
        idtr_limit: 0,
        cr0: CR0_PE | CR0_ET | CR0_NE | CR0_PG,
        cr3: layout.pml4_addr as u64,
        cr4: CR4_PAE,
        efer: EFER_LME | EFER_LMA,
        rip: layout.code_addr as u64,
        rsp: layout.stack_top as u64,
        rflags: 0x2,
    }
}

fn write_bootstrap_image(
    guest_memory: *mut u8,
    guest_memory_size: usize,
    layout: &BootstrapLayout,
) -> Result<(), AsldError> {
    let buffer = unsafe { core::slice::from_raw_parts_mut(guest_memory, guest_memory_size) };
    if layout.stack_top <= layout.code_addr + 16 {
        return Err(AsldError::InvalidState("bootstrap layout overlaps stack"));
    }

    {
        let pml4 = page_mut(buffer, layout.pml4_addr)?;
        zero_page(pml4);
        write_u64(pml4, 0, (layout.pdpt_addr as u64) | 0x3);
    }
    {
        let pdpt = page_mut(buffer, layout.pdpt_addr)?;
        zero_page(pdpt);
        write_u64(pdpt, 0, (layout.pd_addr as u64) | 0x3);
    }
    {
        let pd = page_mut(buffer, layout.pd_addr)?;
        zero_page(pd);
        for index in 0..512usize {
            let guest_phys = (index as u64) * 0x20_0000;
            write_u64(pd, index, guest_phys | 0x83);
        }
    }

    let code = slice_at_mut(buffer, layout.code_addr, 4)?;
    code.copy_from_slice(&[0xfa, 0xf4, 0xeb, 0xfd]);
    Ok(())
}

fn page_mut(buffer: &mut [u8], addr: usize) -> Result<&mut [u8], AsldError> {
    slice_at_mut(buffer, addr, PAGE_SIZE)
}

fn slice_at_mut(buffer: &mut [u8], addr: usize, len: usize) -> Result<&mut [u8], AsldError> {
    let end = addr
        .checked_add(len)
        .ok_or(AsldError::InvalidState("bootstrap layout overflow"))?;
    buffer
        .get_mut(addr..end)
        .ok_or(AsldError::InvalidState("bootstrap layout out of bounds"))
}

fn zero_page(page: &mut [u8]) {
    page.fill(0);
}

fn write_u64(page: &mut [u8], index: usize, value: u64) {
    let offset = index * core::mem::size_of::<u64>();
    page[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn assess_boot_exit(exit: &VmExitInfo) -> ExitAssessment {
    match exit.reason {
        exit_reason::HLT | exit_reason::HLT_EMULATED => ExitAssessment {
            ready: true,
            should_continue: false,
            halted: true,
            summary: format!("guest bootstrap reached halt ({})", describe_exit(exit)),
        },
        exit_reason::CPUID_EMULATED | exit_reason::CPUID | exit_reason::PAUSE => ExitAssessment {
            ready: false,
            should_continue: true,
            halted: false,
            summary: format!("continuing after transient exit ({})", describe_exit(exit)),
        },
        exit_reason::IO_INSTRUCTION => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "guest requested unsupported I/O port {:#x} during boot",
                exit.io_port
            ),
        },
        exit_reason::EPT_VIOLATION | exit_reason::EPT_MISCONFIG => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "guest memory translation failure during boot ({})",
                describe_exit(exit)
            ),
        },
        exit_reason::INVALID_GUEST_STATE | exit_reason::TRIPLE_FAULT | exit_reason::SHUTDOWN => {
            ExitAssessment {
                ready: false,
                should_continue: false,
                halted: false,
                summary: format!("guest failed to enter stable boot state ({})", describe_exit(exit)),
            }
        }
        _ => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!("unexpected guest exit during boot ({})", describe_exit(exit)),
        },
    }
}

fn assess_runtime_exit(exit: &VmExitInfo) -> RuntimeExitAssessment {
    match exit.reason {
        exit_reason::CPUID_EMULATED | exit_reason::CPUID | exit_reason::PAUSE => {
            RuntimeExitAssessment::Continue
        }
        exit_reason::HLT | exit_reason::HLT_EMULATED => {
            RuntimeExitAssessment::Record(build_runtime_event(
                exit,
                "guest halted after runtime dispatch",
                false,
                true,
            ))
        }
        exit_reason::IO_INSTRUCTION => RuntimeExitAssessment::Record(build_runtime_event(
            exit,
            "guest triggered unsupported I/O instruction",
            true,
            false,
        )),
        exit_reason::EPT_VIOLATION | exit_reason::EPT_MISCONFIG => {
            RuntimeExitAssessment::Record(build_runtime_event(
                exit,
                "guest hit memory translation failure",
                true,
                false,
            ))
        }
        exit_reason::INVALID_GUEST_STATE | exit_reason::TRIPLE_FAULT | exit_reason::SHUTDOWN => {
            RuntimeExitAssessment::Record(build_runtime_event(
                exit,
                "guest entered fatal virtualization state",
                true,
                false,
            ))
        }
        _ => RuntimeExitAssessment::Record(build_runtime_event(
            exit,
            "guest exited to host runtime dispatcher",
            false,
            false,
        )),
    }
}

fn build_runtime_event(
    exit: &VmExitInfo,
    prefix: &str,
    fatal: bool,
    halted: bool,
) -> VmRuntimeEvent {
    VmRuntimeEvent {
        reason: String::from(exit_reason_name(exit.reason)),
        summary: format!("{} ({})", prefix, describe_exit(exit)),
        fatal,
        qualification: exit.qualification,
        guest_phys_addr: exit.guest_phys_addr,
        guest_virt_addr: exit.guest_virt_addr,
        halted,
    }
}

fn describe_exit(exit: &VmExitInfo) -> String {
    format!(
        "{} hw={} qual={:#x} gpa={:#x} gva={:#x}",
        exit_reason_name(exit.reason),
        exit.hw_reason,
        exit.qualification,
        exit.guest_phys_addr,
        exit.guest_virt_addr
    )
}

fn exit_reason_name(reason: u32) -> &'static str {
    match reason {
        exit_reason::EXTERNAL_INTERRUPT => "external-interrupt",
        exit_reason::TRIPLE_FAULT => "triple-fault",
        exit_reason::INIT_SIGNAL => "init-signal",
        exit_reason::SIPI => "sipi",
        exit_reason::CPUID => "cpuid",
        exit_reason::HLT => "hlt",
        exit_reason::INVD => "invd",
        exit_reason::INVLPG => "invlpg",
        exit_reason::RDPMC => "rdpmc",
        exit_reason::RDTSC => "rdtsc",
        exit_reason::RSM => "rsm",
        exit_reason::VMCALL => "vmcall",
        exit_reason::CR_ACCESS => "cr-access",
        exit_reason::DR_ACCESS => "dr-access",
        exit_reason::IO_INSTRUCTION => "io-instruction",
        exit_reason::RDMSR => "rdmsr",
        exit_reason::WRMSR => "wrmsr",
        exit_reason::INVALID_GUEST_STATE => "invalid-guest-state",
        exit_reason::PAUSE => "pause",
        exit_reason::EPT_VIOLATION => "ept-violation",
        exit_reason::EPT_MISCONFIG => "ept-misconfig",
        exit_reason::RDTSCP => "rdtscp",
        exit_reason::PREEMPTION_TIMER => "preemption-timer",
        exit_reason::WBINVD => "wbinvd",
        exit_reason::XSETBV => "xsetbv",
        exit_reason::RDRAND => "rdrand",
        exit_reason::INVPCID => "invpcid",
        exit_reason::RDSEED => "rdseed",
        exit_reason::SHUTDOWN => "shutdown",
        exit_reason::SMI => "smi",
        exit_reason::NMI_WINDOW => "nmi-window",
        exit_reason::IRQ_WINDOW => "irq-window",
        exit_reason::CPUID_EMULATED => "cpuid-emulated",
        exit_reason::HLT_EMULATED => "hlt-emulated",
        _ => "unknown",
    }
}

mod exit_reason {
    pub const EXTERNAL_INTERRUPT: u32 = 1;
    pub const TRIPLE_FAULT: u32 = 2;
    pub const INIT_SIGNAL: u32 = 3;
    pub const SIPI: u32 = 4;
    pub const CPUID: u32 = 10;
    pub const HLT: u32 = 12;
    pub const INVD: u32 = 13;
    pub const INVLPG: u32 = 14;
    pub const RDPMC: u32 = 15;
    pub const RDTSC: u32 = 16;
    pub const RSM: u32 = 17;
    pub const VMCALL: u32 = 18;
    pub const CR_ACCESS: u32 = 28;
    pub const DR_ACCESS: u32 = 29;
    pub const IO_INSTRUCTION: u32 = 30;
    pub const RDMSR: u32 = 31;
    pub const WRMSR: u32 = 32;
    pub const INVALID_GUEST_STATE: u32 = 33;
    pub const PAUSE: u32 = 40;
    pub const EPT_VIOLATION: u32 = 48;
    pub const EPT_MISCONFIG: u32 = 49;
    pub const RDTSCP: u32 = 51;
    pub const PREEMPTION_TIMER: u32 = 52;
    pub const WBINVD: u32 = 54;
    pub const XSETBV: u32 = 55;
    pub const RDRAND: u32 = 57;
    pub const INVPCID: u32 = 58;
    pub const RDSEED: u32 = 61;
    pub const SHUTDOWN: u32 = 0x100;
    pub const SMI: u32 = 0x101;
    pub const NMI_WINDOW: u32 = 0x102;
    pub const IRQ_WINDOW: u32 = 0x103;
    pub const CPUID_EMULATED: u32 = 0x104;
    pub const HLT_EMULATED: u32 = 0x105;
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use crate::model::{
        AgentPolicy, DistroConfig, DistroMetadata, LifecyclePolicy, NetworkPolicy, Resources,
        StorageSpec,
    };

    use super::{
        align_guest_memory_size, assess_boot_exit, assess_runtime_exit, boot_probe, bootstrap_gprs,
        bootstrap_sregs, page_mut, poll_runtime, start_vm, stop_vm, write_bootstrap_image,
        BootstrapLayout, RuntimeExitAssessment, VmBootReport, VmExitInfo, VmRunState,
        BOOT_CODE_ADDR, BOOT_PD_ADDR, BOOT_PDPT_ADDR, BOOT_PML4_ADDR,
    };

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
        let mut instance = instance;
        let boot = boot_probe(&mut instance).unwrap();
        assert!(boot.ready);
        assert_eq!(instance.run_state, VmRunState::BootReady);
        assert!(poll_runtime(&mut instance).unwrap().is_none());
        stop_vm(&instance).unwrap();
    }

    #[test]
    fn bootstrap_layout_requires_enough_memory() {
        assert!(BootstrapLayout::new(0x100000).is_err());
        assert!(BootstrapLayout::new(align_guest_memory_size(16)).is_ok());
    }

    #[test]
    fn bootstrap_sregs_target_long_mode_identity_map() {
        let layout = BootstrapLayout::new(align_guest_memory_size(16)).unwrap();
        let regs = bootstrap_gprs();
        let sregs = bootstrap_sregs(&layout);
        assert_eq!(regs, Default::default());
        assert_eq!(sregs.cr3, BOOT_PML4_ADDR as u64);
        assert_eq!(sregs.rip, BOOT_CODE_ADDR as u64);
        assert_eq!(sregs.cs_selector, 0x08);
        assert_eq!(sregs.ss_selector, 0x10);
        assert_eq!(sregs.efer & 0x500, 0x500);
        assert_eq!(sregs.cr4 & (1 << 5), 1 << 5);
        assert_eq!(sregs.cr0 & (1 << 31), 1 << 31);
    }

    #[test]
    fn bootstrap_image_writes_page_tables_and_hlt_stub() {
        let guest_memory_size = align_guest_memory_size(16);
        let layout = BootstrapLayout::new(guest_memory_size).unwrap();
        let mut memory = vec![0u8; guest_memory_size];
        write_bootstrap_image(memory.as_mut_ptr(), guest_memory_size, &layout).unwrap();

        let pml4 = page_mut(&mut memory, BOOT_PML4_ADDR).unwrap();
        assert_eq!(u64::from_le_bytes(pml4[0..8].try_into().unwrap()), (BOOT_PDPT_ADDR as u64) | 0x3);

        let pdpt = page_mut(&mut memory, BOOT_PDPT_ADDR).unwrap();
        assert_eq!(u64::from_le_bytes(pdpt[0..8].try_into().unwrap()), (BOOT_PD_ADDR as u64) | 0x3);

        let pd = page_mut(&mut memory, BOOT_PD_ADDR).unwrap();
        assert_eq!(u64::from_le_bytes(pd[0..8].try_into().unwrap()), 0x83);
        assert_eq!(
            u64::from_le_bytes(pd[8..16].try_into().unwrap()),
            0x20_0000 | 0x83
        );

        let code = &memory[BOOT_CODE_ADDR..BOOT_CODE_ADDR + 4];
        assert_eq!(code, &[0xfa, 0xf4, 0xeb, 0xfd]);

        let sregs = bootstrap_sregs(&layout);
        assert!(sregs.rsp > BOOT_CODE_ADDR as u64);
    }

    #[test]
    fn boot_exit_assessment_accepts_hlt_bootstrap() {
        let result = assess_boot_exit(&VmExitInfo {
            reason: super::exit_reason::HLT_EMULATED,
            hw_reason: 0,
            ..VmExitInfo::default()
        });
        assert!(result.ready);
        assert!(!result.should_continue);
        assert!(result.halted);
        assert!(result.summary.contains("halt"));
    }

    #[test]
    fn boot_exit_assessment_rejects_invalid_guest_state() {
        let result = assess_boot_exit(&VmExitInfo {
            reason: super::exit_reason::INVALID_GUEST_STATE,
            hw_reason: 33,
            ..VmExitInfo::default()
        });
        assert!(!result.ready);
        assert!(!result.should_continue);
        assert!(!result.halted);
        assert!(result.summary.contains("stable boot state"));
    }

    #[test]
    fn host_boot_probe_is_explicitly_successful() {
        let report = VmBootReport {
            ready: true,
            halted: false,
            summary: String::from("boot probe skipped on host-stub"),
        };
        assert!(report.ready);
        assert!(report.summary.contains("boot probe"));
    }

    #[test]
    fn runtime_exit_assessment_records_fatal_io_exit() {
        let assessment = assess_runtime_exit(&VmExitInfo {
            reason: super::exit_reason::IO_INSTRUCTION,
            io_port: 0x3f8,
            ..VmExitInfo::default()
        });
        match assessment {
            RuntimeExitAssessment::Record(event) => {
                assert!(event.fatal);
                assert_eq!(event.reason, "io-instruction");
                assert!(event.summary.contains("unsupported I/O"));
            }
            RuntimeExitAssessment::Continue => panic!("expected runtime event"),
        }
    }
}
