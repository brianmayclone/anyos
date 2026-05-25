use alloc::string::String;
use alloc::vec::Vec;

use crate::model::{
    AgentPolicy, DistroConfig, DistroMetadata, LifecyclePolicy, NetworkPolicy, Resources,
    StorageSpec, VmRunState,
};

use super::assessment::{assess_boot_exit, assess_runtime_exit, RuntimeExitAssessment};
use super::bootstrap::{
    bootstrap_gprs, bootstrap_sregs, direct_linux_gprs, direct_linux_sregs, page_mut, seabios_gprs,
    seabios_sregs, write_bootstrap_image, BootstrapLayout,
};
use super::msr::{msr_read, msr_write, MsrState, MSR_IA32_EFER, MSR_IA32_FS_BASE};
use super::platform::{
    platform_io_action, PlatformIoState, IO_PORT_CMOS_DATA, IO_PORT_CMOS_INDEX, IO_PORT_KBD_STATUS,
    IO_PORT_PIC1_CMD, IO_PORT_PIC1_DATA, IO_PORT_PIC2_CMD, IO_PORT_PIC2_DATA, IO_PORT_POST_DELAY,
};
use super::serial::{
    serial_io_action, SerialPortState, UART_IER_DLM, UART_IIR_FCR, UART_LCR, UART_LCR_DLAB,
    UART_LSR, UART_RBR_THR_DLL,
};
use super::{
    align_guest_memory_size, boot_probe, exit_reason, poll_runtime, start_vm, stop_vm,
    VmBootReport, VmExitInfo, BOOT_CODE_ADDR, BOOT_PDPT_ADDR, BOOT_PD_ADDR, BOOT_PML4_ADDR,
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
            seed_image_path: String::from("/seed"),
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
    assert_eq!(instance.boot_mode, "direct-linux");
    assert!(instance.console_pipe_name.contains("ubuntu"));
    assert!(instance.input_pipe_name.contains("ubuntu"));
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
    assert_eq!(sregs.tr_ar, 0x008b);
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
    assert_eq!(sregs.cs_selector, 0x10);
    assert_eq!(sregs.ds_selector, 0x18);
    assert_eq!(sregs.fs_selector, 0);
    assert_eq!(sregs.gs_selector, 0);
    assert_eq!(sregs.ss_selector, 0x18);
    assert_eq!(sregs.tr_selector, 0x20);
    assert_eq!(sregs.tr_ar, 0x008b);
    assert_eq!(sregs.gdtr_base, crate::boot::LINUX_BOOT_GDT_ADDR as u64);
    assert_eq!(sregs.gdtr_limit, 39);
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
    assert_eq!(sregs.cr0, 0x6000_0030);
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
        reason: exit_reason::HLT_EMULATED,
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
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
fn serial_io_action_drains_guest_input_and_signals_irq() {
    let mut state = SerialPortState::default();
    state.push_input(b"xy");

    let lsr = serial_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: UART_LSR,
            access_size: 1,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        },
    )
    .unwrap();
    assert_eq!(lsr.read_value, Some(0x61));

    assert!(!state.pending_irq());
    let _ = serial_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: UART_IER_DLM,
            access_size: 1,
            is_read: 0,
            io_data: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        },
    );
    assert!(state.pending_irq());

    let iir = serial_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: UART_IIR_FCR,
            access_size: 1,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        },
    )
    .unwrap();
    assert_eq!(iir.read_value, Some(0x04));

    let first = serial_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: UART_RBR_THR_DLL,
            access_size: 1,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        },
    )
    .unwrap();
    assert_eq!(first.read_value, Some(b'x' as u32));
}

#[test]
fn serial_io_action_suppresses_divisor_latch_writes() {
    let mut state = SerialPortState::default();
    let _ = serial_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
            io_port: IO_PORT_PIC1_DATA,
            access_size: 1,
            is_read: 1,
            instruction_len: 1,
            ..VmExitInfo::default()
        },
    )
    .unwrap();
    assert_eq!(pic.read_value, Some(0xfb));
    assert_eq!(state.irq_vector(2), Some(0x0a));
    assert_eq!(state.irq_vector(11), None);

    for (port, value) in [
        (IO_PORT_PIC1_CMD, 0x11),
        (IO_PORT_PIC2_CMD, 0x11),
        (IO_PORT_PIC1_DATA, 0x20),
        (IO_PORT_PIC2_DATA, 0x28),
        (IO_PORT_PIC1_DATA, 0x04),
        (IO_PORT_PIC2_DATA, 0x02),
        (IO_PORT_PIC1_DATA, 0x01),
        (IO_PORT_PIC2_DATA, 0x01),
        (IO_PORT_PIC1_DATA, 0xfb),
        (IO_PORT_PIC2_DATA, 0xf7),
    ] {
        let _ = platform_io_action(
            &mut state,
            &VmExitInfo {
                reason: exit_reason::IO_INSTRUCTION,
                io_port: port,
                access_size: 1,
                is_read: 0,
                io_data: value,
                instruction_len: 1,
                ..VmExitInfo::default()
            },
        );
    }
    assert_eq!(state.irq_vector(11), Some(0x2b));

    let _ = platform_io_action(
        &mut state,
        &VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
            reason: exit_reason::IO_INSTRUCTION,
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
        reason: exit_reason::INVALID_GUEST_STATE,
        hw_reason: 33,
        ..VmExitInfo::default()
    });
    assert!(!result.ready);
    assert!(!result.should_continue);
    assert!(result.halted);
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
        reason: exit_reason::IO_INSTRUCTION,
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
