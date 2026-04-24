use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{DistroConfig, VmRunState};

const PAGE_SIZE: usize = 0x1000;
const MIN_GUEST_MEMORY_MB: usize = 16;
const BOOT_PML4_ADDR: usize = 0x1000;
const BOOT_PDPT_ADDR: usize = 0x2000;
const BOOT_PD_ADDR: usize = 0x3000;
const BOOT_CODE_ADDR: usize = 0x20_0000;
const BOOT_STACK_GUARD: usize = 0x2000;
const COM1_BASE: u16 = 0x3f8;
const UART_RBR_THR_DLL: u16 = COM1_BASE;
const UART_IER_DLM: u16 = COM1_BASE + 1;
const UART_IIR_FCR: u16 = COM1_BASE + 2;
const UART_LCR: u16 = COM1_BASE + 3;
const UART_MCR: u16 = COM1_BASE + 4;
const UART_LSR: u16 = COM1_BASE + 5;
const UART_MSR: u16 = COM1_BASE + 6;
const UART_SCR: u16 = COM1_BASE + 7;
const UART_LCR_DLAB: u8 = 0x80;
const IO_PORT_POST_DELAY: u16 = 0x80;
const IO_PORT_PIC1_CMD: u16 = 0x20;
const IO_PORT_PIC1_DATA: u16 = 0x21;
const IO_PORT_PIC2_CMD: u16 = 0xa0;
const IO_PORT_PIC2_DATA: u16 = 0xa1;
const IO_PORT_PIT_CH0: u16 = 0x40;
const IO_PORT_PIT_CH1: u16 = 0x41;
const IO_PORT_PIT_CH2: u16 = 0x42;
const IO_PORT_PIT_CMD: u16 = 0x43;
const IO_PORT_CMOS_INDEX: u16 = 0x70;
const IO_PORT_CMOS_DATA: u16 = 0x71;
const IO_PORT_KBD_DATA: u16 = 0x60;
const IO_PORT_KBD_STATUS: u16 = 0x64;
const MSR_IA32_TSC: u32 = 0x10;
const MSR_IA32_APIC_BASE: u32 = 0x1b;
const MSR_IA32_SYSENTER_CS: u32 = 0x174;
const MSR_IA32_SYSENTER_ESP: u32 = 0x175;
const MSR_IA32_SYSENTER_EIP: u32 = 0x176;
const MSR_IA32_PAT: u32 = 0x277;
const MSR_IA32_MTRR_DEF_TYPE: u32 = 0x2ff;
const MSR_IA32_EFER: u32 = 0xc000_0080;
const MSR_IA32_STAR: u32 = 0xc000_0081;
const MSR_IA32_LSTAR: u32 = 0xc000_0082;
const MSR_IA32_CSTAR: u32 = 0xc000_0083;
const MSR_IA32_FMASK: u32 = 0xc000_0084;
const MSR_IA32_FS_BASE: u32 = 0xc000_0100;
const MSR_IA32_GS_BASE: u32 = 0xc000_0101;
const MSR_IA32_KERNEL_GS_BASE: u32 = 0xc000_0102;
const MSR_IA32_TSC_AUX: u32 = 0xc000_0103;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmInstance {
    pub distro_name: String,
    pub vm_id: u32,
    pub vcpu_id: u32,
    pub vm_handle: u64,
    pub vcpu_handle: u64,
    pub backend: String,
    pub console_pipe_name: String,
    pub guest_memory_addr: usize,
    pub guest_memory_size: usize,
    pub run_state: VmRunState,
    pub halted: bool,
    serial: SerialPortState,
    platform_io: PlatformIoState,
    msrs: MsrState,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SerialPortState {
    lcr: u8,
    ier: u8,
    mcr: u8,
    scratch: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SerialIoAction {
    output: Vec<u8>,
    read_value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlatformIoState {
    pic1_cmd: u8,
    pic1_data: u8,
    pic2_cmd: u8,
    pic2_data: u8,
    pit_cmd: u8,
    pit_data: [u8; 3],
    cmos_index: u8,
}

impl Default for PlatformIoState {
    fn default() -> Self {
        Self {
            pic1_cmd: 0,
            pic1_data: 0xff,
            pic2_cmd: 0,
            pic2_data: 0xff,
            pit_cmd: 0,
            pit_data: [0; 3],
            cmos_index: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PlatformIoAction {
    read_value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MsrState {
    apic_base: u64,
    sysenter_cs: u64,
    sysenter_esp: u64,
    sysenter_eip: u64,
    pat: u64,
    mtrr_def_type: u64,
    efer: u64,
    star: u64,
    lstar: u64,
    cstar: u64,
    fmask: u64,
    fs_base: u64,
    gs_base: u64,
    kernel_gs_base: u64,
    tsc_aux: u64,
    xcr0: u64,
}

impl Default for MsrState {
    fn default() -> Self {
        Self {
            apic_base: 0xfee0_0800,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
            pat: 0x0007_0406_0007_0406,
            mtrr_def_type: 0,
            efer: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            fmask: 0,
            fs_base: 0,
            gs_base: 0,
            kernel_gs_base: 0,
            tsc_aux: 0,
            xcr0: 1,
        }
    }
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
        distro_name: config.name.clone(),
        vm_id: 1,
        vcpu_id: 0,
        vm_handle: 1,
        vcpu_handle: 1,
        backend: String::from("host-stub"),
        console_pipe_name: format!("asl-console-{}", config.name),
        guest_memory_addr: 0,
        guest_memory_size: align_guest_memory_size(config.resources.memory_mb),
        run_state: VmRunState::Provisioned,
        halted: false,
        serial: SerialPortState::default(),
        platform_io: PlatformIoState::default(),
        msrs: MsrState::default(),
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
    let avm = libavm::Avm::new();
    avm.require_api_version()
        .map_err(|_| AsldError::BackendUnavailable("avm api unavailable"))?;
    let info = avm
        .backend_info()
        .map_err(|_| AsldError::BackendUnavailable("avm backend info failed"))?;
    if info.backend_kind == 0 {
        return Err(AsldError::BackendUnavailable(
            "hardware virtualization not available",
        ));
    }

    let vm = avm
        .create_vm()
        .map_err(|_| AsldError::BackendUnavailable("avm create_vm failed"))?;
    let vm_id = (vm.raw_handle() & 0xFFFF_FFFF) as u32;

    let guest_memory_size = align_guest_memory_size(config.resources.memory_mb);
    let guest_memory = anyos_std::process::mmap(guest_memory_size);
    if guest_memory.is_null() {
        let _ = vm.destroy();
        return Err(AsldError::BackendUnavailable(
            "guest memory allocation failed",
        ));
    }
    unsafe {
        *guest_memory = 0xf4;
    }

    let memory_region = libavm::AvmUserspaceMemoryRegion {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: guest_memory_size as u64,
        userspace_addr: guest_memory as u64,
    };
    if vm.set_user_memory_region(&memory_region).is_err() {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(AsldError::BackendUnavailable(
            "avm set_user_memory_region failed",
        ));
    }

    let vcpu_id = 0u32;
    let vcpu = match vm.create_vcpu(vcpu_id) {
        Ok(vcpu) => vcpu,
        Err(_) => {
            let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
            let _ = vm.destroy();
            return Err(AsldError::BackendUnavailable("avm create_vcpu failed"));
        }
    };

    let boot_result = if crate::boot::is_smoke_test(config) {
        configure_boot_vcpu(&vcpu, guest_memory, guest_memory_size)
    } else if crate::boot::is_seabios(config) {
        configure_seabios_vcpu(&vcpu, guest_memory, guest_memory_size)
    } else {
        configure_direct_linux_vcpu(config, &vcpu, guest_memory, guest_memory_size)
    };
    if let Err(err) = boot_result {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(err);
    }

    let console_pipe_name = format!("asl-console-{}", config.name);
    if let Err(err) = ensure_pipe(&console_pipe_name) {
        let _ = anyos_std::process::munmap(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(err);
    }

    Ok(VmInstance {
        distro_name: config.name.clone(),
        vm_id,
        vcpu_id,
        vm_handle: vm.raw_handle(),
        vcpu_handle: vcpu.raw_handle(),
        backend: match info.backend_kind {
            1 => String::from("avm-vmx"),
            2 => String::from("avm-svm"),
            _ => String::from("avm-unknown"),
        },
        console_pipe_name,
        guest_memory_addr: guest_memory as usize,
        guest_memory_size,
        run_state: VmRunState::Provisioned,
        halted: false,
        serial: SerialPortState::default(),
        platform_io: PlatformIoState::default(),
        msrs: MsrState::default(),
    })
}

#[cfg(not(target_os = "linux"))]
fn boot_probe_impl(instance: &mut VmInstance) -> Result<VmBootReport, AsldError> {
    const MAX_BOOT_EXITS: usize = 512;

    let mut last_exit = VmExitInfo::default();
    let vcpu = libavm::AvmVcpu::from_raw_handle(instance.vcpu_handle);
    for _ in 0..MAX_BOOT_EXITS {
        let mut exit = VmExitInfo::default();
        if vcpu.run(&mut exit).is_err() {
            return Err(AsldError::BackendUnavailable("avm vcpu run failed"));
        }
        if handle_emulated_exit(instance, &vcpu, &exit)? {
            continue;
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
    const MAX_RUNTIME_EXITS: usize = 4;

    if instance.halted {
        return Ok(None);
    }

    let vcpu = libavm::AvmVcpu::from_raw_handle(instance.vcpu_handle);
    for _ in 0..MAX_RUNTIME_EXITS {
        let mut exit = VmExitInfo::default();
        if vcpu.run(&mut exit).is_err() {
            instance.run_state = VmRunState::Degraded;
            return Err(AsldError::BackendUnavailable("avm vcpu run failed"));
        }
        if handle_emulated_exit(instance, &vcpu, &exit)? {
            continue;
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
fn configure_direct_linux_vcpu(
    config: &DistroConfig,
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let layout = crate::boot::prepare_direct_linux_boot(config, guest_memory, guest_memory_size)?;
    let regs = direct_linux_gprs(&layout);
    let sregs = direct_linux_sregs(&layout);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }
    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn configure_seabios_vcpu(
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let layout = crate::boot::prepare_seabios_boot(guest_memory, guest_memory_size)?;
    let regs = seabios_gprs(&layout);
    let sregs = seabios_sregs(&layout);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }
    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn configure_boot_vcpu(
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let bootstrap = BootstrapLayout::new(guest_memory_size)?;
    write_bootstrap_image(guest_memory, guest_memory_size, &bootstrap)?;
    let regs = bootstrap_gprs();
    let sregs = bootstrap_sregs(&bootstrap);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }

    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn stop_vm_impl(instance: &VmInstance) -> Result<(), AsldError> {
    if instance.guest_memory_addr != 0 && instance.guest_memory_size != 0 {
        let _ = anyos_std::process::munmap(
            instance.guest_memory_addr as *mut u8,
            instance.guest_memory_size,
        );
    }
    let vm = libavm::AvmVm::from_raw_handle(instance.vm_handle);
    if vm.destroy().is_err() {
        return Err(AsldError::BackendUnavailable("avm destroy_vm failed"));
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
        return Err(AsldError::BackendUnavailable(
            "console pipe provisioning failed",
        ));
    }
    Ok(())
}

type VmExitInfo = libavm::AvmExitInfo;

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
            return Err(AsldError::InvalidState(
                "guest memory too small for bootstrap",
            ));
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

type GuestGprs = libavm::AvmRegs;
type GuestSregs = libavm::AvmSregs;

fn bootstrap_gprs() -> GuestGprs {
    GuestGprs::default()
}

fn direct_linux_gprs(layout: &crate::boot::DirectLinuxLayout) -> GuestGprs {
    let mut regs = GuestGprs::default();
    regs.rsi = layout.boot_params_addr as u64;
    regs.rsp = 0x90000;
    regs
}

fn seabios_gprs(_layout: &crate::boot::SeaBiosLayout) -> GuestGprs {
    GuestGprs::default()
}

fn seabios_sregs(layout: &crate::boot::SeaBiosLayout) -> GuestSregs {
    const REAL_CODE_AR: u32 = 0x009B;
    const REAL_DATA_AR: u32 = 0x0093;
    const REAL_TSS_AR: u32 = 0x008B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_RESET: u64 = 0x6000_0010;

    GuestSregs {
        cs_selector: 0xf000,
        cs_base: 0xf0000,
        cs_limit: 0xffff,
        cs_ar: REAL_CODE_AR,
        ds_selector: 0,
        ds_base: 0,
        ds_limit: 0xffff,
        ds_ar: REAL_DATA_AR,
        es_selector: 0,
        es_base: 0,
        es_limit: 0xffff,
        es_ar: REAL_DATA_AR,
        fs_selector: 0,
        fs_base: 0,
        fs_limit: 0xffff,
        fs_ar: REAL_DATA_AR,
        gs_selector: 0,
        gs_base: 0,
        gs_limit: 0xffff,
        gs_ar: REAL_DATA_AR,
        ss_selector: 0,
        ss_base: 0,
        ss_limit: 0xffff,
        ss_ar: REAL_DATA_AR,
        tr_selector: 0,
        tr_base: 0,
        tr_limit: 0xffff,
        tr_ar: REAL_TSS_AR,
        ldtr_selector: 0,
        ldtr_base: 0,
        ldtr_limit: 0,
        ldtr_ar: NULL_SEGMENT_AR,
        gdtr_base: 0,
        gdtr_limit: 0xffff,
        idtr_base: 0,
        idtr_limit: 0x03ff,
        cr0: CR0_RESET,
        cr3: 0,
        cr4: 0,
        efer: 0,
        rip: (layout.reset_vector - 0xf0000) as u64,
        rsp: 0,
        rflags: 0x2,
    }
}

fn direct_linux_sregs(layout: &crate::boot::DirectLinuxLayout) -> GuestSregs {
    const CODE_SEGMENT_AR: u32 = 0xC09B;
    const DATA_SEGMENT_AR: u32 = 0xC093;
    const TSS_SEGMENT_AR: u32 = 0x808B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_PE: u64 = 1 << 0;
    const CR0_ET: u64 = 1 << 4;
    const CR0_NE: u64 = 1 << 5;
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
        cr0: CR0_PE | CR0_ET | CR0_NE,
        cr3: 0,
        cr4: 0,
        efer: 0,
        rip: layout.kernel_entry_addr as u64,
        rsp: 0x90000,
        rflags: 0x2,
    }
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

#[cfg(not(target_os = "linux"))]
fn handle_emulated_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if handle_serial_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_platform_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_msr_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_xsetbv_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    Ok(false)
}

#[cfg(not(target_os = "linux"))]
fn handle_serial_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    let Some(action) = serial_io_action(&mut instance.serial, exit) else {
        return Ok(false);
    };

    if !action.output.is_empty() {
        let _ = crate::broker::write_console_bytes(&instance.distro_name, &action.output);
    }

    if let Some(value) = action.read_value {
        write_io_read_value(vcpu, exit.access_size, value)?;
    }
    advance_guest_rip(vcpu, exit.instruction_len)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_platform_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    let Some(action) = platform_io_action(&mut instance.platform_io, exit) else {
        return Ok(false);
    };

    if let Some(value) = action.read_value {
        write_io_read_value(vcpu, exit.access_size, value)?;
    }
    advance_guest_rip(vcpu, exit.instruction_len)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_msr_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    match exit.reason {
        exit_reason::RDMSR => {
            let value = msr_read(&instance.msrs, exit.msr_index);
            write_msr_read_value(vcpu, value)?;
            advance_guest_rip(vcpu, exit.instruction_len)?;
            Ok(true)
        }
        exit_reason::WRMSR => {
            msr_write(&mut instance.msrs, exit.msr_index, exit.io_data);
            sync_guest_msr_side_effects(vcpu, exit.msr_index, &instance.msrs)?;
            advance_guest_rip(vcpu, exit.instruction_len)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(not(target_os = "linux"))]
fn handle_xsetbv_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if exit.reason != exit_reason::XSETBV {
        return Ok(false);
    }
    if exit.msr_index == 0 {
        instance.msrs.xcr0 = exit.io_data;
    }
    advance_guest_rip(vcpu, exit.instruction_len)?;
    Ok(true)
}

fn serial_io_action(state: &mut SerialPortState, exit: &VmExitInfo) -> Option<SerialIoAction> {
    if exit.reason != exit_reason::IO_INSTRUCTION || !is_com1_port(exit.io_port) {
        return None;
    }

    if exit.is_read != 0 {
        return Some(SerialIoAction {
            output: Vec::new(),
            read_value: Some(serial_read(state, exit.io_port)),
        });
    }

    let value = (exit.io_data & 0xff) as u8;
    let mut output = Vec::new();
    match exit.io_port {
        UART_RBR_THR_DLL => {
            if state.lcr & UART_LCR_DLAB == 0 {
                output.push(value);
            }
        }
        UART_IER_DLM => {
            if state.lcr & UART_LCR_DLAB == 0 {
                state.ier = value;
            }
        }
        UART_LCR => state.lcr = value,
        UART_MCR => state.mcr = value,
        UART_SCR => state.scratch = value,
        UART_IIR_FCR | UART_LSR | UART_MSR => {}
        _ => {}
    }
    Some(SerialIoAction {
        output,
        read_value: None,
    })
}

fn serial_read(state: &SerialPortState, port: u16) -> u32 {
    match port {
        UART_RBR_THR_DLL => 0,
        UART_IER_DLM => {
            if state.lcr & UART_LCR_DLAB == 0 {
                state.ier as u32
            } else {
                0
            }
        }
        UART_IIR_FCR => 0x01,
        UART_LCR => state.lcr as u32,
        UART_MCR => state.mcr as u32,
        UART_LSR => 0x60,
        UART_MSR => 0xb0,
        UART_SCR => state.scratch as u32,
        _ => 0,
    }
}

fn is_com1_port(port: u16) -> bool {
    (COM1_BASE..=UART_SCR).contains(&port)
}

fn platform_io_action(state: &mut PlatformIoState, exit: &VmExitInfo) -> Option<PlatformIoAction> {
    if exit.reason != exit_reason::IO_INSTRUCTION || !is_platform_io_port(exit.io_port) {
        return None;
    }

    if exit.is_read != 0 {
        return Some(PlatformIoAction {
            read_value: Some(platform_io_read(state, exit.io_port)),
        });
    }

    let value = (exit.io_data & 0xff) as u8;
    match exit.io_port {
        IO_PORT_PIC1_CMD => state.pic1_cmd = value,
        IO_PORT_PIC1_DATA => state.pic1_data = value,
        IO_PORT_PIC2_CMD => state.pic2_cmd = value,
        IO_PORT_PIC2_DATA => state.pic2_data = value,
        IO_PORT_PIT_CH0 => state.pit_data[0] = value,
        IO_PORT_PIT_CH1 => state.pit_data[1] = value,
        IO_PORT_PIT_CH2 => state.pit_data[2] = value,
        IO_PORT_PIT_CMD => state.pit_cmd = value,
        IO_PORT_CMOS_INDEX => state.cmos_index = value & 0x7f,
        IO_PORT_POST_DELAY | IO_PORT_CMOS_DATA | IO_PORT_KBD_DATA | IO_PORT_KBD_STATUS => {}
        _ => {}
    }
    Some(PlatformIoAction { read_value: None })
}

fn platform_io_read(state: &PlatformIoState, port: u16) -> u32 {
    match port {
        IO_PORT_PIC1_CMD => state.pic1_cmd as u32,
        IO_PORT_PIC1_DATA => state.pic1_data as u32,
        IO_PORT_PIC2_CMD => state.pic2_cmd as u32,
        IO_PORT_PIC2_DATA => state.pic2_data as u32,
        IO_PORT_PIT_CH0 => state.pit_data[0] as u32,
        IO_PORT_PIT_CH1 => state.pit_data[1] as u32,
        IO_PORT_PIT_CH2 => state.pit_data[2] as u32,
        IO_PORT_PIT_CMD => state.pit_cmd as u32,
        IO_PORT_CMOS_INDEX => state.cmos_index as u32,
        IO_PORT_CMOS_DATA => cmos_read(state.cmos_index),
        IO_PORT_KBD_DATA => 0,
        IO_PORT_KBD_STATUS => 0x10,
        IO_PORT_POST_DELAY => 0,
        _ => 0,
    }
}

fn cmos_read(index: u8) -> u32 {
    match index {
        0x0a => 0x26,
        0x0b => 0x02,
        0x0c => 0,
        0x0d => 0x80,
        0x15 => 0,
        0x16 => 0,
        0x17 => 0,
        0x18 => 0,
        _ => 0,
    }
}

fn is_platform_io_port(port: u16) -> bool {
    matches!(
        port,
        IO_PORT_POST_DELAY
            | IO_PORT_PIC1_CMD
            | IO_PORT_PIC1_DATA
            | IO_PORT_PIC2_CMD
            | IO_PORT_PIC2_DATA
            | IO_PORT_PIT_CH0
            | IO_PORT_PIT_CH1
            | IO_PORT_PIT_CH2
            | IO_PORT_PIT_CMD
            | IO_PORT_CMOS_INDEX
            | IO_PORT_CMOS_DATA
            | IO_PORT_KBD_DATA
            | IO_PORT_KBD_STATUS
    )
}

fn msr_read(state: &MsrState, msr: u32) -> u64 {
    match msr {
        MSR_IA32_TSC => 0,
        MSR_IA32_APIC_BASE => state.apic_base,
        MSR_IA32_SYSENTER_CS => state.sysenter_cs,
        MSR_IA32_SYSENTER_ESP => state.sysenter_esp,
        MSR_IA32_SYSENTER_EIP => state.sysenter_eip,
        MSR_IA32_PAT => state.pat,
        MSR_IA32_MTRR_DEF_TYPE => state.mtrr_def_type,
        MSR_IA32_EFER => state.efer,
        MSR_IA32_STAR => state.star,
        MSR_IA32_LSTAR => state.lstar,
        MSR_IA32_CSTAR => state.cstar,
        MSR_IA32_FMASK => state.fmask,
        MSR_IA32_FS_BASE => state.fs_base,
        MSR_IA32_GS_BASE => state.gs_base,
        MSR_IA32_KERNEL_GS_BASE => state.kernel_gs_base,
        MSR_IA32_TSC_AUX => state.tsc_aux,
        _ => 0,
    }
}

fn msr_write(state: &mut MsrState, msr: u32, value: u64) {
    match msr {
        MSR_IA32_TSC => {}
        MSR_IA32_APIC_BASE => state.apic_base = value,
        MSR_IA32_SYSENTER_CS => state.sysenter_cs = value,
        MSR_IA32_SYSENTER_ESP => state.sysenter_esp = value,
        MSR_IA32_SYSENTER_EIP => state.sysenter_eip = value,
        MSR_IA32_PAT => state.pat = value,
        MSR_IA32_MTRR_DEF_TYPE => state.mtrr_def_type = value,
        MSR_IA32_EFER => state.efer = value,
        MSR_IA32_STAR => state.star = value,
        MSR_IA32_LSTAR => state.lstar = value,
        MSR_IA32_CSTAR => state.cstar = value,
        MSR_IA32_FMASK => state.fmask = value,
        MSR_IA32_FS_BASE => state.fs_base = value,
        MSR_IA32_GS_BASE => state.gs_base = value,
        MSR_IA32_KERNEL_GS_BASE => state.kernel_gs_base = value,
        MSR_IA32_TSC_AUX => state.tsc_aux = value,
        _ => {}
    }
}

#[cfg(not(target_os = "linux"))]
fn write_io_read_value(
    vcpu: &libavm::AvmVcpu,
    access_size: u8,
    value: u32,
) -> Result<(), AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    let mask = match access_size {
        1 => 0xffu64,
        2 => 0xffffu64,
        4 => 0xffff_ffffu64,
        _ => 0xffu64,
    };
    regs.rax = (regs.rax & !mask) | ((value as u64) & mask);
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))
}

#[cfg(not(target_os = "linux"))]
fn write_msr_read_value(vcpu: &libavm::AvmVcpu, value: u64) -> Result<(), AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    regs.rax = (regs.rax & !0xffff_ffffu64) | (value & 0xffff_ffffu64);
    regs.rdx = (regs.rdx & !0xffff_ffffu64) | ((value >> 32) & 0xffff_ffffu64);
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))
}

#[cfg(not(target_os = "linux"))]
fn sync_guest_msr_side_effects(
    vcpu: &libavm::AvmVcpu,
    msr: u32,
    state: &MsrState,
) -> Result<(), AsldError> {
    if !matches!(msr, MSR_IA32_EFER | MSR_IA32_FS_BASE | MSR_IA32_GS_BASE) {
        return Ok(());
    }
    let mut sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;
    match msr {
        MSR_IA32_EFER => sregs.efer = state.efer,
        MSR_IA32_FS_BASE => sregs.fs_base = state.fs_base,
        MSR_IA32_GS_BASE => sregs.gs_base = state.gs_base,
        _ => {}
    }
    vcpu.set_sregs(&sregs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_sregs failed"))
}

#[cfg(not(target_os = "linux"))]
fn advance_guest_rip(vcpu: &libavm::AvmVcpu, instruction_len: u32) -> Result<(), AsldError> {
    let mut sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;
    sregs.rip = sregs.rip.wrapping_add(if instruction_len == 0 {
        1
    } else {
        instruction_len as u64
    });
    vcpu.set_sregs(&sregs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_sregs failed"))
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
                summary: format!(
                    "guest failed to enter stable boot state ({})",
                    describe_exit(exit)
                ),
            }
        }
        _ => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "unexpected guest exit during boot ({})",
                describe_exit(exit)
            ),
        },
    }
}

fn assess_runtime_exit(exit: &VmExitInfo) -> RuntimeExitAssessment {
    match exit.reason {
        exit_reason::CPUID_EMULATED | exit_reason::CPUID | exit_reason::PAUSE => {
            RuntimeExitAssessment::Continue
        }
        exit_reason::HLT | exit_reason::HLT_EMULATED => RuntimeExitAssessment::Record(
            build_runtime_event(exit, "guest halted after runtime dispatch", false, true),
        ),
        exit_reason::IO_INSTRUCTION => RuntimeExitAssessment::Record(build_runtime_event(
            exit,
            "guest triggered unsupported I/O instruction",
            true,
            false,
        )),
        exit_reason::EPT_VIOLATION | exit_reason::EPT_MISCONFIG => RuntimeExitAssessment::Record(
            build_runtime_event(exit, "guest hit memory translation failure", true, false),
        ),
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
        bootstrap_sregs, direct_linux_gprs, direct_linux_sregs, msr_read, msr_write, page_mut,
        platform_io_action, poll_runtime, seabios_gprs, seabios_sregs, serial_io_action, start_vm,
        stop_vm, write_bootstrap_image, BootstrapLayout, MsrState, PlatformIoState,
        RuntimeExitAssessment, SerialPortState, VmBootReport, VmExitInfo, VmRunState,
        BOOT_CODE_ADDR, BOOT_PDPT_ADDR, BOOT_PD_ADDR, BOOT_PML4_ADDR, IO_PORT_CMOS_DATA,
        IO_PORT_CMOS_INDEX, IO_PORT_KBD_STATUS, IO_PORT_PIC1_DATA, IO_PORT_POST_DELAY,
        MSR_IA32_EFER, MSR_IA32_FS_BASE, UART_LCR, UART_LCR_DLAB, UART_LSR, UART_RBR_THR_DLL,
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
    fn direct_linux_sregs_target_32bit_protected_entry() {
        let layout = crate::boot::DirectLinuxLayout {
            boot_params_addr: crate::boot::LINUX_BOOT_PARAMS_ADDR,
            kernel_load_addr: crate::boot::LINUX_KERNEL_LOAD_ADDR,
            kernel_entry_addr: crate::boot::LINUX_KERNEL_LOAD_ADDR,
            kernel_size: 4096,
            cmdline_addr: crate::boot::LINUX_CMDLINE_ADDR,
            cmdline_size: 64,
            initrd_addr: 0,
            initrd_size: 0,
        };
        let regs = direct_linux_gprs(&layout);
        let sregs = direct_linux_sregs(&layout);
        assert_eq!(regs.rsi, crate::boot::LINUX_BOOT_PARAMS_ADDR as u64);
        assert_eq!(sregs.rip, crate::boot::LINUX_KERNEL_LOAD_ADDR as u64);
        assert_eq!(sregs.cs_selector, 0x08);
        assert_eq!(sregs.ss_selector, 0x10);
        assert_eq!(sregs.efer, 0);
        assert_eq!(sregs.cr4, 0);
        assert_eq!(sregs.cr0 & 1, 1);
        assert_eq!(sregs.cr0 & (1 << 31), 0);
    }

    #[test]
    fn seabios_sregs_target_real_mode_reset_vector() {
        let layout = crate::boot::SeaBiosLayout {
            firmware_addr: 0xe0000,
            firmware_size: 128 * 1024,
            reset_vector: crate::boot::SEABIOS_RESET_VECTOR,
        };
        let regs = seabios_gprs(&layout);
        let sregs = seabios_sregs(&layout);
        assert_eq!(regs, Default::default());
        assert_eq!(sregs.cs_selector, 0xf000);
        assert_eq!(sregs.cs_base, 0xf0000);
        assert_eq!(sregs.rip, 0xfff0);
        assert_eq!(sregs.cr0, 0x6000_0010);
        assert_eq!(sregs.efer, 0);
    }

    #[test]
    fn bootstrap_image_writes_page_tables_and_hlt_stub() {
        let guest_memory_size = align_guest_memory_size(16);
        let layout = BootstrapLayout::new(guest_memory_size).unwrap();
        let mut memory = vec![0u8; guest_memory_size];
        write_bootstrap_image(memory.as_mut_ptr(), guest_memory_size, &layout).unwrap();

        let pml4 = page_mut(&mut memory, BOOT_PML4_ADDR).unwrap();
        assert_eq!(
            u64::from_le_bytes(pml4[0..8].try_into().unwrap()),
            (BOOT_PDPT_ADDR as u64) | 0x3
        );

        let pdpt = page_mut(&mut memory, BOOT_PDPT_ADDR).unwrap();
        assert_eq!(
            u64::from_le_bytes(pdpt[0..8].try_into().unwrap()),
            (BOOT_PD_ADDR as u64) | 0x3
        );

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
    fn serial_io_action_captures_com1_output_and_status_reads() {
        let mut state = SerialPortState::default();
        let action = serial_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: UART_RBR_THR_DLL,
                access_size: 1,
                is_read: 0,
                io_data: b'A' as u64,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert_eq!(action.output, alloc::vec![b'A']);
        assert_eq!(action.read_value, None);

        let status = serial_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: UART_LSR,
                access_size: 1,
                is_read: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert_eq!(status.read_value, Some(0x60));
    }

    #[test]
    fn serial_io_action_suppresses_divisor_latch_writes() {
        let mut state = SerialPortState::default();
        let _ = serial_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: UART_LCR,
                access_size: 1,
                is_read: 0,
                io_data: UART_LCR_DLAB as u64,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        );
        let action = serial_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: UART_RBR_THR_DLL,
                access_size: 1,
                is_read: 0,
                io_data: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert!(action.output.is_empty());
    }

    #[test]
    fn platform_io_action_handles_early_pc_ports() {
        let mut state = PlatformIoState::default();
        assert!(platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_POST_DELAY,
                access_size: 1,
                is_read: 0,
                io_data: 0xaa,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .is_some());

        let _ = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_PIC1_DATA,
                access_size: 1,
                is_read: 0,
                io_data: 0xfb,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        );
        let pic = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_PIC1_DATA,
                access_size: 1,
                is_read: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert_eq!(pic.read_value, Some(0xfb));

        let _ = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_CMOS_INDEX,
                access_size: 1,
                is_read: 0,
                io_data: 0x0d,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        );
        let cmos = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_CMOS_DATA,
                access_size: 1,
                is_read: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert_eq!(cmos.read_value, Some(0x80));

        let keyboard = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: super::exit_reason::IO_INSTRUCTION,
                io_port: IO_PORT_KBD_STATUS,
                access_size: 1,
                is_read: 1,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        )
        .unwrap();
        assert_eq!(keyboard.read_value, Some(0x10));
    }

    #[test]
    fn msr_state_tracks_linux_boot_msrs() {
        let mut state = MsrState::default();
        msr_write(&mut state, MSR_IA32_EFER, 0x500);
        msr_write(&mut state, MSR_IA32_FS_BASE, 0x1234_5000);

        assert_eq!(msr_read(&state, MSR_IA32_EFER), 0x500);
        assert_eq!(msr_read(&state, MSR_IA32_FS_BASE), 0x1234_5000);
        assert_eq!(msr_read(&state, 0xdead_beef), 0);
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
