use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::{VmEntry, FrameBufferData};
use crate::config::{BiosType, MacMode, NetMode};
use crate::diagnostics::{DiagLog, DiagCategory};
use crate::display;
use crate::platform;
use crate::sidebar::VmState;

use libcorevm::ffi::{
    CExitReason,
    corevm_create, corevm_create_vcpu, corevm_destroy,
    corevm_run_vcpu, corevm_handle_io_exit, corevm_handle_mmio_exit, corevm_handle_string_io_exit,
    corevm_setup_standard_devices, corevm_setup_acpi_tables, corevm_setup_acpi_tables_with_hpet, corevm_setup_ahci, corevm_setup_e1000, corevm_setup_hpet, corevm_setup_ac97, corevm_ac97_process,
    corevm_setup_uhci, corevm_uhci_process,
    corevm_complete_string_io,
    corevm_get_vcpu_regs,
    corevm_get_vcpu_sregs,
    corevm_vga_get_framebuffer, corevm_vga_get_text_buffer, corevm_vga_get_mode, corevm_vga_get_fb_offset,
    corevm_pit_advance, corevm_pit_debug, corevm_cmos_advance, corevm_poll_irqs, corevm_pic_debug, corevm_cancel_vcpu, corevm_lapic_timer_advance, corevm_lapic_debug,
    corevm_read_phys, corevm_write_phys, corevm_debug_port_take_output, corevm_check_reset, corevm_set_cpu_count,
    corevm_ahci_flush_caches, corevm_ahci_needs_flush,
    corevm_ahci_poll_irq,
    corevm_setup_net, corevm_net_poll,
};
use libcorevm::setup;
use libcorevm::backend::{VcpuRegs, VcpuSregs};

/// Callback for WHP debug output — routes messages to DiagLog's WHP tab.
#[cfg(target_os = "windows")]
extern "C" fn whp_debug_callback(ctx: *mut std::ffi::c_void, msg: *const u8, len: u32) {
    if ctx.is_null() || msg.is_null() { return; }
    let diag = unsafe { &*(ctx as *const DiagLog) };
    let text = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg, len as usize)) };
    diag.append_whp_text(text);
}

/// Retrieve the last error message from libcorevm.
pub fn get_last_error_public() -> Option<String> {
    setup::get_last_error()
}

/// Shared control flags for the VM thread
pub struct VmControl {
    pub stop: AtomicBool,
    pub pause: AtomicBool,
    pub exited: AtomicBool,
    pub exit_reason: Mutex<String>,
    /// Set when the guest requests a reboot (soft reset).
    pub reboot_requested: AtomicBool,
}

/// Start a VM. Sets up libcorevm, spawns execution thread.
pub fn start_vm(entry: &mut VmEntry) -> Result<(), String> {
    let config = &entry.config;

    // Install SIGUSR1 handler so the cancel thread can interrupt KVM_RUN
    // without terminating the process (SIGUSR1 default action = terminate).
    #[cfg(target_os = "linux")]
    libcorevm::backend::kvm::install_sigusr1_handler();

    // Reset diagnostics log
    entry.diag_log.clear();
    if entry.config.diagnostics {
        entry.diag_log.log(DiagCategory::Info, format!("Starting VM '{}' RAM={}MB BIOS={:?}", config.name, config.ram_mb, config.bios_type));
    }

    // Create VM
    let handle = corevm_create(config.ram_mb);
    if handle == 0 {
        let msg = setup::get_last_error().unwrap_or_else(|| "Unknown error".into());
        return Err(format!("Failed to create VM: {}", msg));
    }

    // Set CPU count BEFORE creating vCPUs (needed for CPUID topology)
    let num_cpus = config.cpu_cores.max(1).min(32);
    corevm_set_cpu_count(handle, num_cpus);

    // Create vCPUs — libcorevm sets CPUID per-vCPU and APs to INIT_RECEIVED
    for cpu_id in 0..num_cpus {
        if corevm_create_vcpu(handle, cpu_id) != 0 {
            corevm_destroy(handle);
            return Err(format!("Failed to create vCPU {}", cpu_id));
        }
    }
    setup::set_vram_mb(handle, config.vram_mb);

    // Setup devices (includes PCI bus)
    corevm_setup_standard_devices(handle);

    // HPET — required for Windows guests
    if config.guest_os.is_windows() {
        corevm_setup_hpet(handle);
        entry.diag_log.log(DiagCategory::Info, "HPET enabled (Windows guest)".into());
    }

    // VGA LFB is already mapped by setup_standard_devices → setup_vga_lfb_mapping
    // at slot 2 (0xE0000000, 8MB) pointing to the SVGA device's internal framebuffer.
    // No additional mapping needed here.

    // Setup AHCI controller (replaces IDE)
    corevm_setup_ahci(handle, 6);

    // E1000 NIC — only if enabled in config
    if config.net_enabled {
        let mac = setup::resolve_mac(
            &config.uuid,
            config.mac_mode == MacMode::Static,
            &config.mac_address,
        );
        corevm_setup_e1000(handle, mac.as_ptr());
        entry.diag_log.log(DiagCategory::Info, format!(
            "E1000 NIC enabled (mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X})",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ));
        // Load E1000 PXE ROM — SeaBIOS needs this to initialize the NIC
        let extra_bios_paths = platform::bios_search_paths();
        match setup::load_e1000_rom(handle, &extra_bios_paths) {
            Ok(()) => entry.diag_log.log(DiagCategory::Info, "E1000 PXE ROM loaded".into()),
            Err(e) => entry.diag_log.log(DiagCategory::Info, format!("E1000 ROM warning: {}", e)),
        }
        // Set up network backend — mode is just a number, logic is in libcorevm
        let net_mode_id = match config.net_mode {
            NetMode::Disconnected => 0,
            NetMode::UserMode => 1,
            NetMode::Bridge => 2,
        };
        corevm_setup_net(handle, net_mode_id);
        entry.diag_log.log(DiagCategory::Info, format!("Network backend: {:?}", config.net_mode));
    }

    // UHCI USB Controller with tablet — if enabled in config
    if config.usb_tablet {
        corevm_setup_uhci(handle);
        entry.diag_log.log(DiagCategory::Info, "UHCI USB controller + tablet enabled".to_string());
    }

    // AC97 Audio Controller — only if enabled in config
    if config.audio_enabled {
        corevm_setup_ac97(handle);
        entry.diag_log.log(DiagCategory::Info, "AC97 audio controller enabled".to_string());
    }

    // ACPI tables — MUST be generated AFTER all PCI devices are set up,
    // because libcorevm auto-detects which devices are present for _PRT generation.
    let acpi_rc = if config.guest_os.is_windows() {
        corevm_setup_acpi_tables_with_hpet(handle)
    } else {
        corevm_setup_acpi_tables(handle)
    };
    entry.diag_log.log(DiagCategory::Info, format!("ACPI tables setup: rc={} (hpet={})", acpi_rc, config.guest_os.is_windows()));

    // Load BIOS
    let extra_bios_paths = platform::bios_search_paths();
    match config.bios_type {
        BiosType::SeaBios => setup::load_seabios(handle, &extra_bios_paths)?,
        BiosType::CoreVm => setup::load_corevm_bios(handle, &extra_bios_paths)?,
    }

    // Attach ISO — always on AHCI (port 1) for SeaBIOS boot.
    // For Windows guests, also attach on IDE so Windows Setup can use its
    // built-in ATAPI driver (Windows has no AHCI driver during Setup).
    if !config.iso_image.is_empty() {
        setup::attach_image_to_ahci(handle, &config.iso_image, 1, true)?;
        if config.guest_os.is_windows() {
            setup::attach_cdrom_to_ide(handle, &config.iso_image)?;
            entry.diag_log.log(DiagCategory::Info, "ISO attached via AHCI + IDE (Windows compat)".into());
        } else {
            entry.diag_log.log(DiagCategory::Info, "ISO attached via AHCI".into());
        }
    }

    // Attach disk images (port 0, 2, 3, 4, 5 — port 1 is reserved for CDROM)
    let disk_ports = [0u32, 2, 3, 4, 5];
    for (i, disk_path) in config.disk_images.iter().enumerate() {
        if !disk_path.is_empty() {
            if let Some(&port) = disk_ports.get(i) {
                setup::attach_image_to_ahci(handle, disk_path, port, false)?;
                // Configure disk cache
                let cache_mode = match config.disk_cache_mode {
                    crate::config::DiskCacheMode::WriteBack => 0u32,
                    crate::config::DiskCacheMode::WriteThrough => 1,
                    crate::config::DiskCacheMode::None => 2,
                };
                setup::configure_disk_cache(handle, port, config.disk_cache_mb, cache_mode);
            }
        }
    }

    // Set initial CPU state: CS:IP = F000:FFF0 (real-mode reset vector)
    let sregs_rc = setup::set_initial_cpu_state(handle);
    if entry.config.diagnostics {
        if let Err(ref e) = sregs_rc {
            entry.diag_log.log(DiagCategory::Error, format!("set_initial_cpu_state failed: {}", e));
        }
        // Dump actual VP state after setup
        let mut sregs = VcpuSregs::default();
        let mut regs = VcpuRegs::default();
        corevm_get_vcpu_sregs(handle, 0, &mut sregs);
        corevm_get_vcpu_regs(handle, 0, &mut regs);
        entry.diag_log.log(DiagCategory::CpuState, format!(
            "VP state: CS={:04X}:{:016X}(lim={:X} attr={:02X}/{}/{}) RIP={:X} RFLAGS={:X} CR0={:X}",
            sregs.cs.selector, sregs.cs.base, sregs.cs.limit,
            sregs.cs.type_, sregs.cs.s, sregs.cs.present,
            regs.rip, regs.rflags, sregs.cr0
        ));
        entry.diag_log.log(DiagCategory::CpuState, format!(
            "SS={:04X}:{:016X} DS={:04X}:{:016X} TR={:04X}:{:016X}(type={:02X} s={} p={})",
            sregs.ss.selector, sregs.ss.base,
            sregs.ds.selector, sregs.ds.base,
            sregs.tr.selector, sregs.tr.base,
            sregs.tr.type_, sregs.tr.s, sregs.tr.present
        ));
        entry.diag_log.log(DiagCategory::CpuState, format!(
            "GDT base={:X} lim={:X}  IDT base={:X} lim={:X}  CR4={:X} EFER={:X}",
            sregs.gdt.base, sregs.gdt.limit,
            sregs.idt.base, sregs.idt.limit,
            sregs.cr4, sregs.efer
        ));
    }

    // Write COM1 base address into BDA so SeaBIOS finds the serial port.
    setup::setup_bda_com1(handle);

    // Setup shared state
    let control = Arc::new(VmControl {
        stop: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        exit_reason: Mutex::new(String::new()),
        reboot_requested: AtomicBool::new(false),
    });

    // Register WHP debug callback to route output to the diagnostics UI
    #[cfg(target_os = "windows")]
    {
        let diag_for_whp = Box::new(entry.diag_log.clone());
        let ctx = Box::into_raw(diag_for_whp) as *mut std::ffi::c_void;
        libcorevm::ffi::corevm_set_whp_debug_callback(Some(whp_debug_callback), ctx);
    }

    // Install SIGUSR1 handler so cancel_vcpu can interrupt KVM_RUN (Linux/KVM only)
    #[cfg(target_os = "linux")]
    libcorevm::backend::kvm::install_sigusr1_handler();

    let fb = entry.framebuffer.clone();
    let control_clone = control.clone();
    let diag = entry.diag_log.clone();
    let diag_enabled = entry.config.diagnostics;
    let audio_enabled = entry.config.audio_enabled;
    let usb_tablet = entry.config.usb_tablet;
    let net_enabled = entry.config.net_enabled;
    let num_cpus = entry.config.cpu_cores.max(1).min(32);

    // Spawn VM execution thread
    let thread = thread::spawn(move || {
        vm_run_loop(handle, fb, control_clone, diag, diag_enabled, audio_enabled, usb_tablet, net_enabled, num_cpus);
        corevm_destroy(handle);
    });

    entry.vm_handle = Some(handle);
    entry.control = Some(control);
    entry.vm_thread = Some(thread);
    entry.state = VmState::Running;

    Ok(())
}

/// Stop a running VM
pub fn stop_vm(entry: &mut VmEntry) {
    if let Some(ref control) = entry.control {
        control.stop.store(true, Ordering::Relaxed);
    }
    if let Some(thread) = entry.vm_thread.take() {
        let _ = thread.join();
    }
    entry.vm_handle = None;
    entry.control = None;
    entry.state = VmState::Stopped;
}

/// Pause a running VM
pub fn pause_vm(entry: &mut VmEntry) {
    if let Some(ref control) = entry.control {
        control.pause.store(true, Ordering::Relaxed);
    }
    entry.state = VmState::Paused;
}

/// Resume a paused VM
pub fn resume_vm(entry: &mut VmEntry) {
    if let Some(ref control) = entry.control {
        control.pause.store(false, Ordering::Relaxed);
    }
    entry.state = VmState::Running;
}

/// The main VM execution loop (runs in dedicated thread)
fn vm_run_loop(
    handle: u64,
    fb: Arc<Mutex<FrameBufferData>>,
    control: Arc<VmControl>,
    diag: DiagLog,
    diag_enabled: bool,
    audio_enabled: bool,
    usb_tablet: bool,
    net_enabled: bool,
    num_cpus: u32,
) {
    let mut last_fb_update = Instant::now();
    let mut last_pit_tick = Instant::now();
    let fb_interval = Duration::from_millis(16); // ~60fps
    let mut consecutive_errors: u32 = 0;
    let mut fb_debug_count: u32 = 0;
    let mut cache_flush_counter: u32 = 0;

    // Re-install SIGUSR1 handler in the VM thread to ensure it wasn't
    // overridden by eframe/winit signal handlers in the main thread.
    #[cfg(target_os = "linux")]
    libcorevm::backend::kvm::install_sigusr1_handler();

    // AP threads for vCPU 1+.
    let mut ap_threads: Vec<thread::JoinHandle<()>> = Vec::new();
    for cpu_id in 1..num_cpus {
        let ap_control = control.clone();
        let ap_handle = handle;
        ap_threads.push(thread::spawn(move || {
            #[cfg(target_os = "linux")]
            libcorevm::backend::kvm::install_sigusr1_handler();

            loop {
                if ap_control.stop.load(Ordering::Relaxed)
                    || ap_control.exited.load(Ordering::Relaxed) {
                    break;
                }
                if ap_control.pause.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }

                let mut exit = CExitReason::default();
                let rc = corevm_run_vcpu(ap_handle, cpu_id, &mut exit);
                if rc != 0 {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }

                match exit.reason {
                    0 => {
                        let mut data = [0u8; 4];
                        corevm_handle_io_exit(ap_handle, exit.port, 0, exit.size, data.as_mut_ptr());
                    }
                    1 => {
                        let mut data = exit.data_u32.to_le_bytes();
                        corevm_handle_io_exit(ap_handle, exit.port, 1, exit.size, data.as_mut_ptr());
                    }
                    2 => {
                        let mut data = [0u8; 8];
                        corevm_handle_mmio_exit(ap_handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg, exit.mmio_instr_len);
                        corevm_ahci_poll_irq(ap_handle);
                    }
                    3 => {
                        let mut data = exit.data_u64.to_le_bytes();
                        corevm_handle_mmio_exit(ap_handle, exit.addr, 1, exit.size, data.as_mut_ptr(), 0, 0);
                        corevm_ahci_poll_irq(ap_handle);
                    }
                    7 | 13 => { /* HLT/Cancel — re-enter immediately */ }
                    _ => {}
                }
            }
        }));
    }

    // Timer thread: periodically cancel run_vcpu so the main loop can
    // advance PIT, inject IRQs, and handle other events.
    let cancel_control = control.clone();
    let cancel_handle = handle;
    let cancel_num_cpus = num_cpus;
    thread::spawn(move || {
        while !cancel_control.stop.load(Ordering::Relaxed)
            && !cancel_control.exited.load(Ordering::Relaxed)
        {
            thread::sleep(Duration::from_millis(10));
            for cpu_id in 0..cancel_num_cpus {
                corevm_cancel_vcpu(cancel_handle, cpu_id);
            }
        }
    });

    loop {
        if control.stop.load(Ordering::Relaxed) {
            break;
        }

        if control.pause.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let mut exit = CExitReason::default();
        let run_start = Instant::now();
        let rc = corevm_run_vcpu(handle, 0, &mut exit);
        let run_elapsed = run_start.elapsed();
        if run_elapsed.as_secs() >= 2 {
            diag.log(DiagCategory::Error, format!(
                "run_vcpu took {}ms! reason={} port=0x{:04X} rc={}",
                run_elapsed.as_millis(), exit.reason, exit.port, rc
            ));
        }
        if rc != 0 {
            consecutive_errors += 1;
            let err_msg = setup::get_last_error().unwrap_or_else(|| "unknown".into());
            if diag_enabled {
                diag.log(DiagCategory::Error, format!("run_vcpu error: {}", err_msg));
            }
            if consecutive_errors >= 10 {
                diag.log(DiagCategory::Error, format!("Too many consecutive errors ({}), stopping VM", consecutive_errors));
                *control.exit_reason.lock().unwrap() = format!("Fatal: {}", err_msg);
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        consecutive_errors = 0;

        match exit.reason {
            0 => {
                // IoIn — dispatch to device, device fills data
                if exit.io_count > 1 {
                    // String I/O (REP INSB): handle all iterations at once
                    corevm_complete_string_io(handle, 0, exit.port, 0, exit.size, exit.io_count);
                } else {
                    let mut data = [0u8; 4];
                    corevm_handle_io_exit(handle, exit.port, 0, exit.size, data.as_mut_ptr());
                    if diag_enabled {
                        let val = match exit.size { 1 => data[0] as u32, 2 => u16::from_le_bytes([data[0], data[1]]) as u32, _ => u32::from_le_bytes(data) };
                        diag.log(DiagCategory::IoPort, format!("IN  port=0x{:04X} size={} -> 0x{:X}", exit.port, exit.size, val));
                    }
                    // Process UHCI frames on every UHCI I/O read (SeaBIOS polls USBSTS)
                    if usb_tablet {
                        corevm_uhci_process(handle);
                    }
                }
            }
            1 => {
                // IoOut — dispatch to device
                if exit.io_count > 1 {
                    // String I/O (REP OUTSB): handle all iterations at once
                    corevm_complete_string_io(handle, 0, exit.port, 1, exit.size, exit.io_count);
                } else {
                    // Port 0xCF9: System Reset Control Register
                    if exit.port == 0x0CF9 && exit.size == 1 {
                        let val = exit.data_u32 & 0xFF;
                        if val & 0x04 != 0 {
                            // Bit 2 = System Reset — treat as reboot request
                            if diag_enabled {
                                diag.log(DiagCategory::Info, format!("System reset via port 0xCF9 (val=0x{:02X})", val));
                            }
                            if let Ok(mut r) = control.exit_reason.lock() {
                                *r = "Reboot".into();
                            }
                            control.exited.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    // Capture serial port COM1 (0x3F8) data register output
                    if exit.port == 0x3F8 && exit.size == 1 {
                        let ch = (exit.data_u32 & 0xFF) as u8;
                        if ch >= 0x20 || ch == b'\n' || ch == b'\r' || ch == b'\t' {
                            eprint!("{}", ch as char);
                        }
                    }
                    if diag_enabled {
                        diag.log(DiagCategory::IoPort, format!("OUT port=0x{:04X} size={} data=0x{:X}", exit.port, exit.size, exit.data_u32));
                    }
                    let mut data = exit.data_u32.to_le_bytes();
                    corevm_handle_io_exit(handle, exit.port, 1, exit.size, data.as_mut_ptr());
                    // Process UHCI frames on every UHCI I/O write (SeaBIOS configures UHCI)
                    if usb_tablet {
                        corevm_uhci_process(handle);
                    }
                }
            }
            2 => {
                // MmioRead — dispatch to device
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg, exit.mmio_instr_len);
                if diag_enabled {
                    diag.log(DiagCategory::Mmio, format!("MMIO RD addr=0x{:08X} size={}", exit.addr, exit.size));
                }
                // Immediately update AHCI IRQ after MMIO access (level-triggered)
                corevm_ahci_poll_irq(handle);
            }
            3 => {
                // MmioWrite — dispatch to device
                if diag_enabled {
                    diag.log(DiagCategory::Mmio, format!("MMIO WR addr=0x{:08X} size={} data=0x{:X}", exit.addr, exit.size, exit.data_u64));
                }
                let mut data = exit.data_u64.to_le_bytes();
                corevm_handle_mmio_exit(handle, exit.addr, 1, exit.size, data.as_mut_ptr(), 0, 0);
                // Immediately update AHCI IRQ after MMIO write (command completion)
                corevm_ahci_poll_irq(handle);
            }
            7 => {
                thread::sleep(Duration::from_millis(1));
            }
            9 => {
                // Shutdown — clean exit
                if diag_enabled {
                    diag.log(DiagCategory::Error, "VM Shutdown (triple fault)".to_string());
                }
                update_framebuffer(handle, &fb, &diag, &mut fb_debug_count);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Shutdown".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            11 => {
                // Error — fatal (triple fault / emulation failure)
                let mut regs = VcpuRegs::default();
                let mut sregs = VcpuSregs::default();
                corevm_get_vcpu_regs(handle, 0, &mut regs);
                corevm_get_vcpu_sregs(handle, 0, &mut sregs);
                diag.log(DiagCategory::Error, format!(
                    "VM Error exit — RIP=0x{:X} RSP=0x{:X} RFLAGS=0x{:X} CR0=0x{:X} CR3=0x{:X} CR4=0x{:X} CS.sel=0x{:X} CS.base=0x{:X}",
                    regs.rip, regs.rsp, regs.rflags, sregs.cr0, sregs.cr3, sregs.cr4,
                    sregs.cs.selector, sregs.cs.base
                ));
                update_framebuffer(handle, &fb, &diag, &mut fb_debug_count);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Error".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            12 => {
                // StringIo — bulk REP INSB/OUTSB
                corevm_handle_string_io_exit(
                    handle, exit.port, exit.string_io_is_write,
                    exit.string_io_count, exit.string_io_gpa,
                    exit.string_io_step, exit.string_io_instr_len,
                    exit.string_io_addr_size, exit.size,
                );
                if diag_enabled {
                    let dir = if exit.string_io_is_write != 0 { "OUT" } else { "IN" };
                    diag.log(DiagCategory::IoPort, format!("STRING {} port=0x{:04X} count={} gpa=0x{:X}",
                        dir, exit.port, exit.string_io_count, exit.string_io_gpa));
                }
            }
            8 => {
                // InterruptWindow — guest is now ready to accept interrupts.
                // poll_irqs below will inject the pending interrupt.
            }
            13 => {
                // Cancelled — timer kicked us out of KVM_RUN
            }
            other => {
                // Other exits (MsrRead/Write, Cpuid, Debug)
                if diag_enabled {
                    diag.log(DiagCategory::Info, format!("Unhandled exit reason={}", other));
                }
            }
        }

        let handler_elapsed = run_start.elapsed();
        if handler_elapsed.as_secs() >= 2 {
            diag.log(DiagCategory::Error, format!(
                "exit handler took {}ms! reason={}",
                handler_elapsed.as_millis() - run_elapsed.as_millis(), exit.reason
            ));
        }

        // Advance PIT based on wall-clock elapsed time; IRQ0 injection handled internally
        // Tick every >=100µs to keep channel 2 output responsive for port 0x61 delay loops
        {
            let now = Instant::now();
            let elapsed_us = now.duration_since(last_pit_tick).as_micros() as u64;
            if elapsed_us >= 100 {
                let pit_ticks = ((elapsed_us * 1193) / 1000) as u32;
                last_pit_tick = now;
                let fires = corevm_pit_advance(handle, pit_ticks);
                if fires > 0 {
                    let pic_st = corevm_pic_debug(handle);
                    let mut regs = VcpuRegs::default();
                    corevm_get_vcpu_regs(handle, 0, &mut regs);
                    let if_flag = regs.rflags & 0x200 != 0;
                    diag.log(DiagCategory::Interrupt, format!(
                        "PIT fired {} ticks={} PIC IRR={:#04x} IMR={:#04x} ISR={:#04x} icw={} IF={} exit={} RIP={:#x}",
                        fires, pit_ticks,
                        pic_st & 0xFF, (pic_st >> 8) & 0xFF, (pic_st >> 16) & 0xFF,
                        (pic_st >> 24) & 1, if_flag, exit.reason, regs.rip
                    ));
                }
            }
        }

        // Advance CMOS RTC periodic timer based on fresh wall-clock time.
        // 32.768 kHz base clock → ticks = elapsed_us * 32768 / 1_000_000
        {
            let now = Instant::now();
            let elapsed_us = now.duration_since(last_pit_tick).as_micros() as u64;
            if elapsed_us > 0 {
                let rtc_ticks = (elapsed_us * 32768) / 1_000_000;
                if rtc_ticks > 0 {
                    corevm_cmos_advance(handle, rtc_ticks);
                }
            }
        }

        // Periodic stderr state dump (time-based, every 2 seconds)
        static mut LAST_STATE_DUMP: Option<Instant> = None;
        static mut EXIT_COUNTS: [u64; 16] = [0; 16];
        unsafe {
            if (exit.reason as usize) < 16 { EXIT_COUNTS[exit.reason as usize] += 1; }
            let now = Instant::now();
            let should_dump = match LAST_STATE_DUMP {
                None => { LAST_STATE_DUMP = Some(now); true }
                Some(last) => {
                    if now.duration_since(last).as_secs() >= 2 {
                        LAST_STATE_DUMP = Some(now);
                        true
                    } else {
                        false
                    }
                }
            };
            if should_dump {
                let mut regs = VcpuRegs::default();
                let mut sregs = VcpuSregs::default();
                corevm_get_vcpu_regs(handle, 0, &mut regs);
                corevm_get_vcpu_sregs(handle, 0, &mut sregs);
                // Read VGA text buffer (first 2 lines)
                let mut vga_buf = [0u8; 160];
                corevm_read_phys(handle, 0xB8000, vga_buf.as_mut_ptr(), 160);
                let vga_line: String = (0..80).map(|i| {
                    let ch = vga_buf[i * 2];
                    if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' }
                }).collect::<String>().trim_end().to_string();
                // Read a few bytes from VBE framebuffer at 0xE0000000 to check if graphics mode active
                let mut fb_sample = [0u8; 16];
                corevm_read_phys(handle, 0xE000_0000, fb_sample.as_mut_ptr(), 16);
                let fb_nonzero = fb_sample.iter().any(|&b| b != 0);
                diag.log(DiagCategory::CpuState, format!("[vm-state] exit={} RIP={:#x} CS={:#x} CR0={:#x} RFLAGS={:#x} IF={} PE={} PG={} FB={} VGA=[{}] exits=[io:{}/{} mmio:{}/{} hlt:{} cancel:{} shut:{} err:{}]",
                    exit.reason, regs.rip, sregs.cs.selector, sregs.cr0, regs.rflags,
                    if regs.rflags & 0x200 != 0 { 1 } else { 0 },
                    if sregs.cr0 & 1 != 0 { 1 } else { 0 },
                    if sregs.cr0 & (1 << 31) != 0 { 1 } else { 0 },
                    if fb_nonzero { "data" } else { "empty" },
                    vga_line,
                    EXIT_COUNTS[0], EXIT_COUNTS[1],  // IoIn, IoOut
                    EXIT_COUNTS[2], EXIT_COUNTS[3],  // MmioRead, MmioWrite
                    EXIT_COUNTS[7],                   // Halted
                    EXIT_COUNTS[13],                  // Cancelled (SIGUSR1/immediate_exit)
                    EXIT_COUNTS[9],                   // Shutdown
                    EXIT_COUNTS[11],                  // Error
                ));
            }
        }

        // Log PIT state every 5000 iterations
        static mut PIT_LOG_CTR: u64 = 0;
        unsafe { PIT_LOG_CTR += 1; }
        if unsafe { PIT_LOG_CTR } % 200 == 0 {
            // Read BDA timer tick count at physical 0x46C (DWORD)
            let mut tick_buf = [0u8; 4];
            corevm_read_phys(handle, 0x46C, tick_buf.as_mut_ptr(), 4);
            let bda_ticks = u32::from_le_bytes(tick_buf);
            // Read IVT entry for INT 8 (4 bytes at physical 0x20)
            let mut ivt_buf = [0u8; 4];
            corevm_read_phys(handle, 0x20, ivt_buf.as_mut_ptr(), 4);
            let ivt8 = u32::from_le_bytes(ivt_buf);
            let mut lapic_init = 0u32;
            let mut lapic_cur = 0u32;
            let mut lapic_lvt = 0u32;
            let lapic_st = corevm_lapic_debug(handle, &mut lapic_init, &mut lapic_cur, &mut lapic_lvt);
            // Read VGA text buffer first 160 bytes (2 lines of 80 chars, char+attr pairs)
            let mut vga_buf = [0u8; 160];
            corevm_read_phys(handle, 0xB8000, vga_buf.as_mut_ptr(), 160);
            let vga_line: String = (0..80).map(|i| {
                let ch = vga_buf[i * 2];
                if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' }
            }).collect::<String>().trim_end().to_string();
            diag.log(DiagCategory::Interrupt, format!(
                "BDA ticks=0x{:08X} IVT[8]=0x{:08X} LAPIC armed={} pend={} div={} init={:#x} cur={:#x} lvt={:#x} VGA=[{}]",
                bda_ticks, ivt8,
                lapic_st & 1, (lapic_st >> 1) & 1, (lapic_st >> 2) & 0xFF,
                lapic_init, lapic_cur, lapic_lvt,
                vga_line
            ));
        }

        // LAPIC timer is handled by WHP internally in XApic mode.
        // No need to call corevm_lapic_timer_advance.

        // Poll device IRQs (PS/2 keyboard IRQ 1, mouse IRQ 12, etc.)
        let poll_inj = corevm_poll_irqs(handle);
        if poll_inj > 0 {
            diag.log(DiagCategory::Interrupt, format!("poll_irqs ret={:#010x}", poll_inj));
        }

        // Periodic disk cache flush (every ~200 iterations or when cache is full)
        cache_flush_counter += 1;
        if cache_flush_counter >= 200 || corevm_ahci_needs_flush(handle) != 0 {
            corevm_ahci_flush_caches(handle);
            cache_flush_counter = 0;
        }

        // Check if guest requested a system reset (PS/2 0xFE or port 0xCF9)
        if corevm_check_reset(handle) != 0 {
            if diag_enabled {
                diag.log(DiagCategory::Info, "System reset requested by guest — rebooting VM".into());
            }
            // Flush caches before reboot
            corevm_ahci_flush_caches(handle);
            control.reboot_requested.store(true, Ordering::Relaxed);
            control.exited.store(true, Ordering::Relaxed);
            break;
        }

        // Process AC97 audio DMA (reads audio data from guest buffers)
        if audio_enabled {
            corevm_ac97_process(handle);
        }

        // Process UHCI USB frames — always process when UHCI is active,
        // not just when USB tablet is enabled. SeaBIOS probes the UHCI
        // during boot and hangs if frames are not processed.
        if usb_tablet {
            corevm_uhci_process(handle);
        }

        // Poll network backend: TX from E1000 → backend, RX from backend → E1000
        if net_enabled {
            corevm_net_poll(handle);
        }

        // Drain debug port output on every iteration
        {
            let mut dbg_buf = [0u8; 1024];
            let n = corevm_debug_port_take_output(handle, dbg_buf.as_mut_ptr(), dbg_buf.len() as u32);
            if n > 0 {
                if let Ok(s) = std::str::from_utf8(&dbg_buf[..n as usize]) {
                    diag.append_debug_text(s);
                }
            }
        }

        // Update framebuffer at ~60fps
        if last_fb_update.elapsed() >= fb_interval {
            update_framebuffer(handle, &fb, &diag, &mut fb_debug_count);
            last_fb_update = Instant::now();
        }
    }

    // Wait for AP threads to finish
    for t in ap_threads {
        let _ = t.join();
    }

    // Final cache flush — ensure all dirty blocks are written to disk
    // before the VM is destroyed. Covers normal shutdown, reboot, and crash.
    corevm_ahci_flush_caches(handle);
}

/// Read VGA state and update the shared framebuffer
fn update_framebuffer(handle: u64, fb: &Arc<Mutex<FrameBufferData>>, diag: &DiagLog, fb_debug_count: &mut u32) {
    // Query VGA mode to get exact dimensions
    let mut vga_w: u32 = 0;
    let mut vga_h: u32 = 0;
    let mut vga_bpp: u8 = 0;
    let mode_ret = corevm_vga_get_mode(handle, &mut vga_w, &mut vga_h, &mut vga_bpp);

    if *fb_debug_count < 5 {
        *fb_debug_count += 1;
        diag.log(DiagCategory::Info, format!(
            "update_fb #{}: mode_ret={} {}x{}x{}",
            fb_debug_count, mode_ret, vga_w, vga_h, vga_bpp
        ));
    }

    if mode_ret == 1 {
        // Text mode — read text buffer
        let mut text_ptr: *const u16 = std::ptr::null();
        let mut text_len: u32 = 0;
        let ret = corevm_vga_get_text_buffer(handle, &mut text_ptr, &mut text_len);
        if ret == 0 && !text_ptr.is_null() && text_len > 0 {
            let text_cells = unsafe { std::slice::from_raw_parts(text_ptr, text_len as usize) };
            if let Ok(mut fb_data) = fb.lock() {
                fb_data.text_mode = true;
                fb_data.text_buffer = text_cells.to_vec();
                let buf = fb_data.text_buffer.clone();
                let (tw, th) = display::render_text_mode(&buf, &mut fb_data.pixels);
                fb_data.width = tw;
                fb_data.height = th;
                fb_data.dirty = true;
            }
        }
    } else if mode_ret == 0 && vga_w > 0 && vga_h > 0 && vga_bpp > 0 {
        // Graphics mode — read from the SVGA device's internal framebuffer.
        // With KVM, setup_vga_lfb_mapping maps 0xE0000000 directly to the
        // SVGA framebuffer buffer, so guest writes update it in-place.
        let bytes_per_pixel = (vga_bpp as usize + 7) / 8;
        let fb_size = vga_w as usize * vga_h as usize * bytes_per_pixel;

        // bochs-drm places its framebuffer at an offset within VRAM using
        // VBE_DISPI_INDEX_X/Y_OFFSET registers. Query the byte offset.
        let vram_offset = corevm_vga_get_fb_offset(handle);

        // Read LFB via corevm_read_phys (reads from KVM memory region = SVGA buffer)
        let mut raw_pixels = vec![0u8; fb_size];
        let read_addr = 0xE000_0000u64 + vram_offset;
        let phys_ret = corevm_read_phys(handle, read_addr, raw_pixels.as_mut_ptr(), fb_size as u32);
        let have_pixels = if phys_ret == 0 {
            // Check if LFB has actual data (guest may have switched to text mode).
            // Sample from multiple locations across the framebuffer — checking
            // only the first 64 bytes fails when the top-left corner is black
            // (e.g. a terminal with black background).
            let stride = vga_w as usize * bytes_per_pixel;
            let sample_rows = [0, vga_h as usize / 4, vga_h as usize / 2, vga_h as usize * 3 / 4];
            sample_rows.iter().any(|&row| {
                let off = row * stride;
                if off + 64 <= raw_pixels.len() {
                    raw_pixels[off..off + 64].iter().any(|&b| b != 0)
                } else {
                    false
                }
            })
        } else {
            // Fallback: read internal SVGA framebuffer directly
            let mut fb_ptr: *const u8 = std::ptr::null();
            let mut fb_len: u32 = 0;
            corevm_vga_get_framebuffer(handle, &mut fb_ptr, &mut fb_len);
            if !fb_ptr.is_null() && fb_len > 0 {
                let off = vram_offset as usize;
                let avail = (fb_len as usize).saturating_sub(off);
                let len = avail.min(fb_size);
                if len > 0 {
                    let raw = unsafe { std::slice::from_raw_parts(fb_ptr.add(off), len) };
                    raw_pixels[..len].copy_from_slice(raw);
                    // Sample from multiple rows, not just the first 64 bytes
                    let stride = vga_w as usize * bytes_per_pixel;
                    let sample_rows = [0, vga_h as usize / 4, vga_h as usize / 2, vga_h as usize * 3 / 4];
                    sample_rows.iter().any(|&row| {
                        let roff = row * stride;
                        if roff + 64 <= len {
                            raw_pixels[roff..roff + 64].iter().any(|&b| b != 0)
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            } else {
                false
            }
        };

        // Always render graphics mode when VGA reports it — even if the
        // framebuffer appears black (e.g. Windows loading screen with dark background).
        if let Ok(mut fb_data) = fb.lock() {
            fb_data.text_mode = false;
            fb_data.width = vga_w;
            fb_data.height = vga_h;
            display::render_graphics_mode(&raw_pixels, vga_w, vga_h, vga_bpp, &mut fb_data.pixels);
            fb_data.dirty = true;
        }
    }
}

/// Fallback: render text mode when graphics mode reports empty LFB.
/// This handles the case where the guest switches to text mode via VGA
/// register programming without disabling VBE.
fn render_text_fallback(handle: u64, fb: &Arc<Mutex<FrameBufferData>>) {
    let mut text_ptr: *const u16 = std::ptr::null();
    let mut text_len: u32 = 0;
    let ret = corevm_vga_get_text_buffer(handle, &mut text_ptr, &mut text_len);
    if ret == 0 && !text_ptr.is_null() && text_len > 0 {
        let text_cells = unsafe { std::slice::from_raw_parts(text_ptr, text_len as usize) };
        if let Ok(mut fb_data) = fb.lock() {
            fb_data.text_mode = true;
            fb_data.text_buffer = text_cells.to_vec();
            let buf = fb_data.text_buffer.clone();
            let (tw, th) = display::render_text_mode(&buf, &mut fb_data.pixels);
            fb_data.width = tw;
            fb_data.height = th;
            fb_data.dirty = true;
        }
    }
}

/// Guess framebuffer resolution from byte length.
/// Returns (width, height, bpp).
fn guess_resolution(len: usize) -> (u32, u32, u8) {
    // Try common resolutions at 32bpp first, then 24bpp, then 16bpp
    let common = [
        (1280, 1024), (1024, 768), (800, 600), (640, 480),
        (1920, 1080), (1600, 1200), (1280, 800), (1280, 720),
    ];
    for &(w, h) in &common {
        if len == (w * h * 4) as usize { return (w, h, 32); }
    }
    for &(w, h) in &common {
        if len == (w * h * 3) as usize { return (w, h, 24); }
    }
    for &(w, h) in &common {
        if len == (w * h * 2) as usize { return (w, h, 16); }
    }
    // Fallback: assume 32bpp, try to find a reasonable width
    let pixels = len / 4;
    if pixels > 0 {
        // Try 640-wide
        if pixels % 640 == 0 {
            return (640, (pixels / 640) as u32, 32);
        }
        if pixels % 800 == 0 {
            return (800, (pixels / 800) as u32, 32);
        }
    }
    (0, 0, 0)
}

