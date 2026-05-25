use alloc::format;
use alloc::string::String;

use crate::errors::AsldError;
use crate::model::{DistroConfig, VmRunState};

mod apic;
mod aslnet;
mod assessment;
mod bootstrap;
mod e1000;
mod exit_reason;
mod ide;
mod memory;
mod mmio;
mod msr;
mod pci;
mod platform;
mod runtime_io;
mod serial;
#[cfg(test)]
mod tests;
#[cfg(not(target_os = "linux"))]
mod vcpu;
mod vga;
mod virtio;
use apic::ApicState;
#[cfg(not(target_os = "linux"))]
use apic::{apic_mmio_action, is_apic_mmio};
use aslnet::AslNetDevice;
#[cfg(not(target_os = "linux"))]
use assessment::{assess_boot_exit, assess_runtime_exit, describe_exit, RuntimeExitAssessment};
#[cfg(not(target_os = "linux"))]
use bootstrap::{configure_boot_vcpu, configure_direct_linux_vcpu, configure_seabios_vcpu};
#[cfg(not(target_os = "linux"))]
use e1000::is_e1000_mmio_region;
use e1000::E1000Device;
use ide::IdeController;
#[cfg(not(target_os = "linux"))]
use memory::{
    address_register_value, io_string_info, read_guest_bytes, read_guest_physical,
    update_address_register, write_guest_bytes, write_guest_physical, IoStringInfo,
};
#[cfg(not(target_os = "linux"))]
use mmio::{complete_mmio_read, prepare_mmio_exit};
use msr::MsrState;
#[cfg(not(target_os = "linux"))]
use msr::{msr_read, msr_write};
use pci::PciBus;
#[cfg(not(target_os = "linux"))]
use platform::platform_io_action;
use platform::PlatformIoState;
use runtime_io::{align_guest_memory_size, ensure_pipe};
#[cfg(not(target_os = "linux"))]
use serial::serial_io_action;
use serial::SerialPortState;
#[cfg(not(target_os = "linux"))]
use vcpu::{
    advance_guest_rip, sync_guest_msr_side_effects, write_io_read_value, write_msr_read_value,
};

const MAX_STRING_IO_BYTES: usize = 512;
const BOOT_PML4_ADDR: usize = 0x1000;
const BOOT_PDPT_ADDR: usize = 0x2000;
const BOOT_PD_ADDR: usize = 0x3000;
const BOOT_CODE_ADDR: usize = 0x20_0000;
const BOOT_STACK_GUARD: usize = 0x2000;
const SERIAL_IRQ: u8 = 4;
const E1000_IRQ: u8 = 11;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmInstance {
    pub distro_name: String,
    pub boot_mode: String,
    pub vm_id: u32,
    pub vcpu_id: u32,
    pub vm_handle: u64,
    pub vcpu_handle: u64,
    pub backend: String,
    pub console_pipe_name: String,
    pub input_pipe_name: String,
    pub guest_memory_addr: usize,
    pub guest_memory_size: usize,
    pub total_vm_exits: u64,
    pub last_vm_exit_summary: String,
    pub run_state: VmRunState,
    pub halted: bool,
    serial: SerialPortState,
    platform_io: PlatformIoState,
    pci: PciBus,
    ide: IdeController,
    net: AslNetDevice,
    e1000: E1000Device,
    display: vga::GuestFramebuffer,
    virtio_gpu: virtio::VirtioGpuDevice,
    apic: ApicState,
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
    let console_pipe_name = format!("asl-console-{}", config.name);
    let input_pipe_name = format!("asl-input-{}", config.name);
    ensure_pipe(&console_pipe_name)?;
    ensure_pipe(&input_pipe_name)?;
    Ok(VmInstance {
        distro_name: config.name.clone(),
        boot_mode: crate::boot::build_boot_plan(config).mode,
        vm_id: 1,
        vcpu_id: 0,
        vm_handle: 1,
        vcpu_handle: 1,
        backend: String::from("host-stub"),
        console_pipe_name,
        input_pipe_name,
        guest_memory_addr: 0,
        guest_memory_size: align_guest_memory_size(config.resources.memory_mb),
        total_vm_exits: 0,
        last_vm_exit_summary: String::new(),
        run_state: VmRunState::Provisioned,
        halted: false,
        serial: SerialPortState::default(),
        platform_io: PlatformIoState::default(),
        pci: PciBus::default(),
        ide: IdeController::disabled(),
        net: AslNetDevice::default(),
        e1000: E1000Device::default(),
        display: vga::GuestFramebuffer::default(),
        virtio_gpu: virtio::VirtioGpuDevice::default(),
        apic: ApicState::default(),
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
    let guest_memory = anyos_std::process::mmap_large(guest_memory_size);
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
        let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(AsldError::BackendUnavailable(
            "avm set_user_memory_region failed",
        ));
    }

    let vcpu_id = 0u32;
    let vcpu = match vm.create_vcpu(vcpu_id) {
        Ok(vcpu) => vcpu,
        Err(_) => {
            let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
            let _ = vm.destroy();
            return Err(AsldError::BackendUnavailable("avm create_vcpu failed"));
        }
    };

    let boot_plan = crate::boot::build_boot_plan(config);
    let boot_result = if crate::boot::is_smoke_test(config) {
        configure_boot_vcpu(&vcpu, guest_memory, guest_memory_size)
    } else if crate::boot::is_seabios(config) {
        configure_seabios_vcpu(&vcpu, guest_memory, guest_memory_size)
    } else {
        configure_direct_linux_vcpu(config, &vcpu, guest_memory, guest_memory_size)
    };
    if let Err(err) = boot_result {
        let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(err);
    }

    let console_pipe_name = format!("asl-console-{}", config.name);
    let input_pipe_name = format!("asl-input-{}", config.name);
    if let Err(err) = ensure_pipe(&console_pipe_name) {
        let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(err);
    }
    if let Err(err) = ensure_pipe(&input_pipe_name) {
        let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
        let _ = vm.destroy();
        return Err(err);
    }

    let ide = if crate::boot::is_seabios(config) {
        match IdeController::open_asl_disks(
            &config.storage.base_image_path,
            &config.storage.seed_image_path,
        ) {
            Ok(ide) => ide,
            Err(err) => {
                let _ = anyos_std::process::munmap_large(guest_memory, guest_memory_size);
                let _ = vm.destroy();
                return Err(err);
            }
        }
    } else {
        IdeController::disabled()
    };

    Ok(VmInstance {
        distro_name: config.name.clone(),
        boot_mode: boot_plan.mode,
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
        input_pipe_name,
        guest_memory_addr: guest_memory as usize,
        guest_memory_size,
        total_vm_exits: 0,
        last_vm_exit_summary: String::new(),
        run_state: VmRunState::Provisioned,
        halted: false,
        serial: SerialPortState::default(),
        platform_io: PlatformIoState::default(),
        pci: PciBus::default(),
        ide,
        net: AslNetDevice::default(),
        e1000: E1000Device::default(),
        display: vga::GuestFramebuffer::default(),
        virtio_gpu: virtio::VirtioGpuDevice::default(),
        apic: ApicState::default(),
        msrs: MsrState::default(),
    })
}

#[cfg(not(target_os = "linux"))]
fn boot_probe_impl(instance: &mut VmInstance) -> Result<VmBootReport, AsldError> {
    const MAX_BOOT_EXITS: usize = 512;

    let mut last_exit = VmExitInfo::default();
    let vcpu = libavm::AvmVcpu::from_raw_handle(instance.vcpu_handle);
    for _ in 0..MAX_BOOT_EXITS {
        drain_serial_input(instance);
        poll_e1000_rx(instance, &vcpu)?;
        if inject_pending_local_apic_irq(instance, &vcpu)? {
            continue;
        }
        if inject_pending_serial_irq(instance, &vcpu)? {
            continue;
        }
        if inject_pending_platform_irq(instance, &vcpu)? {
            continue;
        }
        let mut exit = VmExitInfo::default();
        if vcpu.run(&mut exit).is_err() {
            instance.halted = true;
            instance.run_state = VmRunState::Degraded;
            return Err(AsldError::BackendUnavailable("avm vcpu run failed"));
        }
        record_vm_exit_sample(instance, &exit);
        if handle_emulated_exit(instance, &vcpu, &exit)? {
            continue;
        }
        if matches!(exit.reason, exit_reason::HLT | exit_reason::HLT_EMULATED)
            && inject_pending_device_irqs(instance, &vcpu)?
        {
            continue;
        }
        let mut assessment = assess_boot_exit(&exit);
        if assessment.ready
            && assessment.halted
            && instance.boot_mode != crate::boot::SMOKE_TEST_KERNEL_PROFILE
        {
            if instance.serial.output_bytes() == 0 {
                assessment.ready = false;
                assessment.summary =
                    format!("guest halted before serial output ({})", describe_exit(&exit));
            } else {
                let _ = vcpu.resume();
                assessment.halted = false;
                assessment.summary = format!(
                    "guest reached idle halt; runtime remains active ({})",
                    describe_exit(&exit)
                );
            }
        }
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
        drain_serial_input(instance);
        poll_e1000_rx(instance, &vcpu)?;
        if inject_pending_local_apic_irq(instance, &vcpu)? {
            continue;
        }
        if inject_pending_serial_irq(instance, &vcpu)? {
            continue;
        }
        if inject_pending_platform_irq(instance, &vcpu)? {
            continue;
        }
        let mut exit = VmExitInfo::default();
        if vcpu.run(&mut exit).is_err() {
            instance.halted = true;
            instance.run_state = VmRunState::Degraded;
            return Err(AsldError::BackendUnavailable("avm vcpu run failed"));
        }
        record_vm_exit_sample(instance, &exit);
        if handle_emulated_exit(instance, &vcpu, &exit)? {
            continue;
        }
        if matches!(exit.reason, exit_reason::HLT | exit_reason::HLT_EMULATED)
            && inject_pending_device_irqs(instance, &vcpu)?
        {
            continue;
        }
        if is_runtime_idle_hlt(instance, &exit) {
            let _ = vcpu.resume();
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
fn record_vm_exit_sample(instance: &mut VmInstance, exit: &VmExitInfo) {
    instance.total_vm_exits = instance.total_vm_exits.saturating_add(1);
    instance.last_vm_exit_summary = describe_exit(exit);
}

#[cfg(not(target_os = "linux"))]
fn is_runtime_idle_hlt(instance: &VmInstance, exit: &VmExitInfo) -> bool {
    instance.boot_mode != crate::boot::SMOKE_TEST_KERNEL_PROFILE
        && matches!(exit.reason, exit_reason::HLT | exit_reason::HLT_EMULATED)
}

#[cfg(not(target_os = "linux"))]
fn stop_vm_impl(instance: &VmInstance) -> Result<(), AsldError> {
    if instance.guest_memory_addr != 0 && instance.guest_memory_size != 0 {
        let _ = anyos_std::process::munmap_large(
            instance.guest_memory_addr as *mut u8,
            instance.guest_memory_size,
        );
    }
    instance.ide.close();
    let vm = libavm::AvmVm::from_raw_handle(instance.vm_handle);
    if vm.destroy().is_err() {
        return Err(AsldError::BackendUnavailable("avm destroy_vm failed"));
    }
    Ok(())
}

type VmExitInfo = libavm::AvmExitInfo;

#[cfg(not(target_os = "linux"))]
fn handle_emulated_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if handle_serial_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_ide_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_asl_net_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_pci_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_platform_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if vga::handle_vga_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if virtio::handle_virtio_gpu_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_e1000_mmio_exit(instance, vcpu, exit)? {
        return Ok(true);
    }
    if handle_apic_mmio_exit(instance, vcpu, exit)? {
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
    let _ = inject_pending_device_irqs(instance, vcpu)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_ide_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if handle_ide_string_io_exit(instance, vcpu, exit)? {
        return Ok(true);
    }

    let Some(action) = instance.ide.io_action(exit) else {
        return Ok(false);
    };

    if let Some(value) = action.read_value {
        write_io_read_value(vcpu, exit.access_size, value)?;
    }
    advance_guest_rip(vcpu, exit.instruction_len)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_ide_string_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    let Some(string_io) = io_string_info(exit) else {
        return Ok(false);
    };
    if exit.access_size != 2 {
        return Ok(false);
    }
    if exit.is_read == 0 {
        return handle_ide_string_write_exit(instance, vcpu, exit, string_io);
    }

    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    let sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;

    let requested_units = if string_io.rep {
        address_register_value(regs.rcx, string_io.address_size) as usize
    } else {
        1
    };
    if requested_units == 0 {
        advance_guest_rip(vcpu, exit.instruction_len)?;
        return Ok(true);
    }

    let max_units = MAX_STRING_IO_BYTES / exit.access_size as usize;
    let units = requested_units.min(max_units);
    let byte_count = units * exit.access_size as usize;
    let mut buffer = [0u8; MAX_STRING_IO_BYTES];
    let Some(copied) = instance
        .ide
        .data_string_read_into(exit, &mut buffer[..byte_count])
    else {
        return Ok(false);
    };
    let copied_units = copied / exit.access_size as usize;

    let mut index = address_register_value(regs.rdi, string_io.address_size);
    let step = exit.access_size as u64;
    for chunk in buffer[..copied].chunks_exact(exit.access_size as usize) {
        let linear = sregs.es_base.wrapping_add(index);
        write_guest_bytes(instance, vcpu, linear, chunk)?;
        index = if (sregs.rflags & (1 << 10)) != 0 {
            index.wrapping_sub(step)
        } else {
            index.wrapping_add(step)
        };
    }

    regs.rdi = update_address_register(regs.rdi, string_io.address_size, index);
    if string_io.rep {
        let remaining = requested_units.saturating_sub(copied_units) as u64;
        regs.rcx = update_address_register(regs.rcx, string_io.address_size, remaining);
    }
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))?;

    if !string_io.rep || copied_units >= requested_units {
        advance_guest_rip(vcpu, exit.instruction_len)?;
    }
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_ide_string_write_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
    string_io: IoStringInfo,
) -> Result<bool, AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    let sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;

    let requested_units = if string_io.rep {
        address_register_value(regs.rcx, string_io.address_size) as usize
    } else {
        1
    };
    if requested_units == 0 {
        advance_guest_rip(vcpu, exit.instruction_len)?;
        return Ok(true);
    }

    let max_units = MAX_STRING_IO_BYTES / exit.access_size as usize;
    let units = requested_units.min(max_units);
    let byte_count = units * exit.access_size as usize;
    let mut buffer = [0u8; MAX_STRING_IO_BYTES];
    let mut index = address_register_value(regs.rsi, string_io.address_size);
    let step = exit.access_size as u64;

    for chunk in buffer[..byte_count].chunks_exact_mut(exit.access_size as usize) {
        let linear = sregs.ds_base.wrapping_add(index);
        read_guest_bytes(instance, vcpu, linear, chunk)?;
        index = if (sregs.rflags & (1 << 10)) != 0 {
            index.wrapping_sub(step)
        } else {
            index.wrapping_add(step)
        };
    }

    let Some(copied) = instance
        .ide
        .data_string_write_from(exit, &buffer[..byte_count])
    else {
        return Ok(false);
    };
    let copied_units = copied / exit.access_size as usize;

    regs.rsi = update_address_register(regs.rsi, string_io.address_size, index);
    if string_io.rep {
        let remaining = requested_units.saturating_sub(copied_units) as u64;
        regs.rcx = update_address_register(regs.rcx, string_io.address_size, remaining);
    }
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))?;

    if !string_io.rep || copied_units >= requested_units {
        advance_guest_rip(vcpu, exit.instruction_len)?;
    }
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_asl_net_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    let Some(action) = instance.net.io_action(exit) else {
        return Ok(false);
    };

    if let Some(frame) = action.tx_frame {
        crate::broker::network_tx_frame(&instance.distro_name, &frame)?;
    }
    if action.rx_poll {
        if let Some(frame) = crate::broker::network_rx_frame(&instance.distro_name, 1518)? {
            instance.net.load_rx_frame(frame);
        }
    }
    if let Some(value) = action.read_value {
        write_io_read_value(vcpu, exit.access_size, value)?;
    }
    advance_guest_rip(vcpu, exit.instruction_len)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_pci_io_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    let Some(action) = instance.pci.io_action(exit) else {
        return Ok(false);
    };

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
fn handle_apic_mmio_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if exit.reason != exit_reason::EPT_VIOLATION || !is_apic_mmio(exit.guest_phys_addr) {
        return Ok(false);
    };
    let prepared = prepare_mmio_exit(instance, vcpu, exit)?;
    let Some(action) = apic_mmio_action(&mut instance.apic, &prepared.exit) else {
        return Ok(false);
    };

    if let Some(value) = action.read_value {
        complete_mmio_read(vcpu, &prepared, value)?;
    }
    advance_guest_rip(vcpu, prepared.instruction_len())?;
    let _ = inject_pending_device_irqs(instance, vcpu)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn handle_e1000_mmio_exit(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if exit.reason != exit_reason::EPT_VIOLATION || !is_e1000_mmio_region(exit.guest_phys_addr) {
        return Ok(false);
    }
    let prepared = prepare_mmio_exit(instance, vcpu, exit)?;
    let memory_addr = instance.guest_memory_addr;
    let memory_size = instance.guest_memory_size;
    let Some(action) = instance.e1000.mmio_action(
        &prepared.exit,
        |gpa, dest| read_guest_physical(memory_addr, memory_size, gpa, dest),
        |gpa, bytes| write_guest_physical(memory_addr, memory_size, gpa, bytes),
    ) else {
        return Ok(false);
    };

    for frame in action.tx_frames {
        crate::broker::network_tx_frame(&instance.distro_name, &frame)?;
    }
    if action.rx_poll {
        if let Some(frame) = crate::broker::network_rx_frame(&instance.distro_name, 1518)? {
            let memory_addr = instance.guest_memory_addr;
            let memory_size = instance.guest_memory_size;
            let _ = instance.e1000.inject_rx_frame(
                &frame,
                |gpa, dest| read_guest_physical(memory_addr, memory_size, gpa, dest),
                |gpa, bytes| write_guest_physical(memory_addr, memory_size, gpa, bytes),
            );
        }
    }
    if action.interrupt {
        let _ = inject_device_irq(instance, vcpu, E1000_IRQ)?;
    }
    if let Some(value) = action.read_value {
        complete_mmio_read(vcpu, &prepared, value)?;
    }
    advance_guest_rip(vcpu, prepared.instruction_len())?;
    let _ = inject_pending_device_irqs(instance, vcpu)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn poll_e1000_rx(instance: &mut VmInstance, vcpu: &libavm::AvmVcpu) -> Result<(), AsldError> {
    if !instance.e1000.wants_rx_poll() {
        return Ok(());
    }
    if let Some(frame) = crate::broker::network_rx_frame(&instance.distro_name, 1518)? {
        let memory_addr = instance.guest_memory_addr;
        let memory_size = instance.guest_memory_size;
        let injected = instance.e1000.inject_rx_frame(
            &frame,
            |gpa, dest| read_guest_physical(memory_addr, memory_size, gpa, dest),
            |gpa, bytes| write_guest_physical(memory_addr, memory_size, gpa, bytes),
        );
        if injected {
            let _ = inject_pending_device_irqs(instance, vcpu)?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn inject_pending_device_irqs(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
) -> Result<bool, AsldError> {
    if inject_pending_local_apic_irq(instance, vcpu)? {
        return Ok(true);
    }
    if inject_pending_serial_irq(instance, vcpu)? {
        return Ok(true);
    }
    if inject_pending_platform_irq(instance, vcpu)? {
        return Ok(true);
    }
    if instance.e1000.interrupt_pending() {
        return inject_device_irq(instance, vcpu, E1000_IRQ);
    }
    Ok(false)
}

#[cfg(not(target_os = "linux"))]
fn inject_pending_serial_irq(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
) -> Result<bool, AsldError> {
    if !instance.serial.pending_irq() {
        return Ok(false);
    }
    inject_device_irq(instance, vcpu, SERIAL_IRQ)
}

#[cfg(not(target_os = "linux"))]
fn inject_pending_local_apic_irq(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
) -> Result<bool, AsldError> {
    let Some(vector) = instance.apic.pending_local_irq() else {
        return Ok(false);
    };
    vcpu.inject_irq(vector)
        .map_err(|_| AsldError::BackendUnavailable("avm inject_irq failed"))?;
    instance.apic.ack_local_irq(vector);
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn inject_pending_platform_irq(
    instance: &mut VmInstance,
    vcpu: &libavm::AvmVcpu,
) -> Result<bool, AsldError> {
    let Some(irq) = instance.platform_io.pending_irq() else {
        return Ok(false);
    };
    if inject_device_irq(instance, vcpu, irq)? {
        instance.platform_io.ack_irq(irq);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(not(target_os = "linux"))]
fn inject_device_irq(
    instance: &VmInstance,
    vcpu: &libavm::AvmVcpu,
    irq: u8,
) -> Result<bool, AsldError> {
    let Some(vector) = instance
        .apic
        .irq_vector(irq)
        .or_else(|| instance.platform_io.irq_vector(irq))
    else {
        return Ok(false);
    };
    vcpu.inject_irq(vector)
        .map_err(|_| AsldError::BackendUnavailable("avm inject_irq failed"))?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn drain_serial_input(instance: &mut VmInstance) {
    let pipe = anyos_std::ipc::pipe_open(&instance.input_pipe_name);
    if pipe == 0 || pipe == u32::MAX {
        return;
    }
    let mut buf = [0u8; 128];
    loop {
        let n = anyos_std::ipc::pipe_read(pipe, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        instance.serial.push_input(&buf[..n as usize]);
        if n < buf.len() as u32 {
            break;
        }
    }
    let _ = anyos_std::ipc::pipe_close(pipe);
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
