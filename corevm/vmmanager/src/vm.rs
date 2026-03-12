use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::{VmEntry, FrameBufferData};
use crate::config::BiosType;
use crate::diagnostics::{DiagLog, DiagCategory};
use crate::display;
use crate::platform;
use crate::sidebar::VmState;

use libcorevm::ffi::{
    CExitReason,
    corevm_create, corevm_create_vcpu, corevm_destroy,
    corevm_run_vcpu, corevm_handle_io_exit, corevm_handle_mmio_exit, corevm_handle_string_io_exit,
    corevm_setup_standard_devices, corevm_setup_acpi_tables, corevm_setup_ahci,
    corevm_ahci_attach_disk, corevm_ahci_attach_cdrom,
    corevm_load_binary,
    corevm_get_vcpu_regs, corevm_set_vcpu_regs,
    corevm_get_vcpu_sregs, corevm_set_vcpu_sregs,
    corevm_vga_get_framebuffer, corevm_vga_get_text_buffer, corevm_vga_get_mode, corevm_vga_get_lfb_addr,
    corevm_last_error, corevm_last_error_len,
    corevm_pit_advance, corevm_pit_debug, corevm_poll_irqs, corevm_pic_debug, corevm_cancel_vcpu, corevm_lapic_timer_advance, corevm_lapic_debug,
    corevm_read_phys, corevm_debug_port_take_output,
    corevm_fw_cfg_add_file, corevm_set_memory_region,
};
use libcorevm::backend::{VcpuRegs, VcpuSregs, SegmentReg, DescriptorTable};

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
    let len = corevm_last_error_len() as usize;
    if len == 0 { return None; }
    let ptr = corevm_last_error();
    if ptr.is_null() { return None; }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn get_last_error() -> Option<String> {
    get_last_error_public()
}

/// Shared control flags for the VM thread
pub struct VmControl {
    pub stop: AtomicBool,
    pub pause: AtomicBool,
    pub exited: AtomicBool,
    pub exit_reason: Mutex<String>,
}

/// Start a VM. Sets up libcorevm, spawns execution thread.
pub fn start_vm(entry: &mut VmEntry) -> Result<(), String> {
    let config = &entry.config;

    // Reset diagnostics log
    entry.diag_log.clear();
    if entry.config.diagnostics {
        entry.diag_log.log(DiagCategory::Info, format!("Starting VM '{}' RAM={}MB BIOS={:?}", config.name, config.ram_mb, config.bios_type));
    }

    // Create VM
    let handle = corevm_create(config.ram_mb);
    if handle == 0 {
        let msg = get_last_error().unwrap_or_else(|| "Unknown error".into());
        return Err(format!("Failed to create VM: {}", msg));
    }

    // Create vCPU
    let vcpu_rc = corevm_create_vcpu(handle, 0);
    if vcpu_rc != 0 {
        let e = get_last_error().unwrap_or_else(|| "unknown".into());
        corevm_destroy(handle);
        return Err(format!("Failed to create vCPU: {}", e));
    }

    // Setup devices (includes PCI bus)
    corevm_setup_standard_devices(handle);
    let acpi_rc = corevm_setup_acpi_tables(handle);
    entry.diag_log.log(DiagCategory::Info, format!("ACPI tables setup: rc={}", acpi_rc));

    // Map VGA linear framebuffer RAM at both the Bochs VBE default address
    // (0xE0000000, where SeaVGABIOS places the LFB) and the PCI BAR0 address
    // (0xFD000000). 8MB matches the VRAM size reported by VBE_DISPI_INDEX_VIDEO_MEMORY_64K.
    let vga_fb_size: usize = 0x80_0000; // 8MB
    let vga_fb_layout = std::alloc::Layout::from_size_align(vga_fb_size, 4096).unwrap();
    let vga_fb_ptr = unsafe { std::alloc::alloc_zeroed(vga_fb_layout) };
    if !vga_fb_ptr.is_null() {
        // Primary: Bochs VBE default LFB at 0xE0000000
        let ret1 = corevm_set_memory_region(handle, 10, 0xE000_0000, vga_fb_size as u64, vga_fb_ptr);
        // Secondary: PCI BAR0 at 0xFD000000 (same physical memory)
        let ret2 = corevm_set_memory_region(handle, 11, 0xFD00_0000, vga_fb_size as u64, vga_fb_ptr);
        if entry.config.diagnostics {
            entry.diag_log.log(DiagCategory::Info, format!(
                "VGA LFB mapped: 0xE0000000 ret={} 0xFD000000 ret={} (8MB)", ret1, ret2
            ));
        }
    }

    // Setup AHCI controller (replaces IDE)
    corevm_setup_ahci(handle, 6);

    // Load BIOS
    load_bios(handle, &config.bios_type)?;

    // Attach ISO if configured (as AHCI CDROM on port 1)
    if !config.iso_image.is_empty() {
        attach_image_to_ahci(handle, &config.iso_image, 1, true)?;
    }

    // Attach disk if configured (as AHCI disk on port 0)
    if !config.disk_image.is_empty() {
        attach_image_to_ahci(handle, &config.disk_image, 0, false)?;
    }

    // Set initial CPU state: CS:IP = F000:FFF0 (real-mode reset vector)
    let sregs_rc = set_initial_cpu_state(handle);
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

    // Setup shared state
    let control = Arc::new(VmControl {
        stop: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        exit_reason: Mutex::new(String::new()),
    });

    // Register WHP debug callback to route output to the diagnostics UI
    #[cfg(target_os = "windows")]
    {
        let diag_for_whp = Box::new(entry.diag_log.clone());
        let ctx = Box::into_raw(diag_for_whp) as *mut std::ffi::c_void;
        libcorevm::ffi::corevm_set_whp_debug_callback(Some(whp_debug_callback), ctx);
    }

    let fb = entry.framebuffer.clone();
    let control_clone = control.clone();
    let diag = entry.diag_log.clone();
    let diag_enabled = entry.config.diagnostics;

    // Spawn VM execution thread
    let thread = thread::spawn(move || {
        vm_run_loop(handle, fb, control_clone, diag, diag_enabled);
        corevm_destroy(handle);
    });

    entry.vm_handle = Some(handle);
    entry.control = Some(control);
    entry.vm_thread = Some(thread);
    entry.state = VmState::Running;

    Ok(())
}

/// Attach a disk or ISO image to an AHCI port via file descriptor.
#[cfg(unix)]
fn attach_image_to_ahci(handle: u64, path: &str, port: u32, is_cdrom: bool) -> Result<(), String> {
    use std::os::unix::io::{IntoRawFd, FromRawFd};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(!is_cdrom)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    let fd = file.into_raw_fd();

    let ret = if is_cdrom {
        corevm_ahci_attach_cdrom(handle, port, fd, size)
    } else {
        corevm_ahci_attach_disk(handle, port, fd, size)
    };

    if ret != 0 {
        // Close fd on failure by reclaiming ownership
        unsafe { drop(std::fs::File::from_raw_fd(fd)); }
        return Err(format!("Failed to attach {} to AHCI port {}", path, port));
    }
    // fd ownership transferred to AHCI, do NOT close it
    Ok(())
}

#[cfg(windows)]
fn attach_image_to_ahci(handle: u64, path: &str, port: u32, is_cdrom: bool) -> Result<(), String> {
    use std::os::windows::io::IntoRawHandle;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(!is_cdrom)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;
    let size = file.metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path, e))?.len();
    // On Windows, pass the raw handle as an i32 (narrowing cast)
    let handle_raw = file.into_raw_handle();
    let fd = handle_raw as isize as i32;

    let ret = if is_cdrom {
        corevm_ahci_attach_cdrom(handle, port, fd, size)
    } else {
        corevm_ahci_attach_disk(handle, port, fd, size)
    };

    if ret != 0 {
        return Err(format!("Failed to attach {} to AHCI port {}", path, port));
    }
    Ok(())
}

/// Set initial CPU state to real-mode reset vector (F000:FFF0).
fn set_initial_cpu_state(handle: u64) -> Result<(), String> {
    let data_seg = SegmentReg {
        base: 0,
        limit: 0xFFFF,
        selector: 0,
        type_: 0x03, // read/write, accessed
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 0,
        g: 0,
        avl: 0,
    };

    let sregs = VcpuSregs {
        cs: SegmentReg {
            base: 0xF0000,
            limit: 0xFFFF,
            selector: 0xF000,
            type_: 0x0B, // execute/read, accessed
            present: 1,
            dpl: 0,
            db: 0,
            s: 1,
            l: 0,
            g: 0,
            avl: 0,
        },
        ds: data_seg,
        es: data_seg,
        fs: data_seg,
        gs: data_seg,
        ss: data_seg,
        tr: SegmentReg {
            base: 0,
            limit: 0xFFFF,
            selector: 0,
            type_: 0x0B, // 16-bit busy TSS
            present: 1,
            dpl: 0,
            db: 0,
            s: 0, // system segment
            l: 0,
            g: 0,
            avl: 0,
        },
        ldt: SegmentReg {
            base: 0,
            limit: 0xFFFF,
            selector: 0,
            type_: 0x02, // LDT
            present: 1,
            dpl: 0,
            db: 0,
            s: 0, // system segment
            l: 0,
            g: 0,
            avl: 0,
        },
        gdt: DescriptorTable { base: 0, limit: 0xFFFF },
        idt: DescriptorTable { base: 0, limit: 0xFFFF },
        cr0: 0x10, // ET bit set (FPU extension type), PE=0 (real mode)
        cr2: 0,
        cr3: 0,
        cr4: 0,
        efer: 0,
    };

    let rc1 = corevm_set_vcpu_sregs(handle, 0, &sregs);
    if rc1 != 0 {
        let e = get_last_error().unwrap_or_else(|| "unknown".into());
        return Err(format!("set_vcpu_sregs failed (rc={}): {}", rc1, e));
    }

    let mut regs = VcpuRegs::default();
    regs.rip = 0xFFF0;
    regs.rflags = 0x02; // reserved bit
    let rc2 = corevm_set_vcpu_regs(handle, 0, &regs);
    if rc2 != 0 {
        let e = get_last_error().unwrap_or_else(|| "unknown".into());
        return Err(format!("set_vcpu_regs failed (rc={}): {}", rc2, e));
    }
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
) {
    let mut last_fb_update = Instant::now();
    let mut last_pit_tick = Instant::now();
    let fb_interval = Duration::from_millis(16); // ~60fps
    let mut consecutive_errors: u32 = 0;
    let mut fb_debug_count: u32 = 0;
    const PIT_FREQ: u64 = 1_193_182; // 8254 PIT clock rate in Hz

    // Timer thread: periodically cancel run_vcpu so the main loop can
    // advance PIT, inject IRQs, and handle other events.
    let cancel_control = control.clone();
    let cancel_handle = handle;
    thread::spawn(move || {
        while !cancel_control.stop.load(Ordering::Relaxed)
            && !cancel_control.exited.load(Ordering::Relaxed)
        {
            thread::sleep(Duration::from_millis(10));
            corevm_cancel_vcpu(cancel_handle, 0);
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
            let err_msg = get_last_error().unwrap_or_else(|| "unknown".into());
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
                let mut data = [0u8; 4];
                corevm_handle_io_exit(handle, exit.port, 0, exit.size, data.as_mut_ptr());
                if diag_enabled {
                    let val = match exit.size { 1 => data[0] as u32, 2 => u16::from_le_bytes([data[0], data[1]]) as u32, _ => u32::from_le_bytes(data) };
                    diag.log(DiagCategory::IoPort, format!("IN  port=0x{:04X} size={} -> 0x{:X}", exit.port, exit.size, val));
                }
            }
            1 => {
                // IoOut — dispatch to device
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
            }
            2 => {
                // MmioRead — dispatch to device
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg, exit.mmio_instr_len);
                if diag_enabled {
                    diag.log(DiagCategory::Mmio, format!("MMIO RD addr=0x{:08X} size={}", exit.addr, exit.size));
                }
            }
            3 => {
                // MmioWrite — dispatch to device
                if diag_enabled {
                    diag.log(DiagCategory::Mmio, format!("MMIO WR addr=0x{:08X} size={} data=0x{:X}", exit.addr, exit.size, exit.data_u64));
                }
                let mut data = exit.data_u64.to_le_bytes();
                corevm_handle_mmio_exit(handle, exit.addr, 1, exit.size, data.as_mut_ptr(), 0, 0);
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
        // Cap at ~1ms worth of ticks to avoid blocking on large time deltas
        let pit_elapsed = last_pit_tick.elapsed();
        if pit_elapsed.as_micros() > 0 {
            let ticks = ((pit_elapsed.as_micros() as u64 * PIT_FREQ / 1_000_000) as u32).min(PIT_FREQ as u32 / 100);
            if ticks > 0 {
                last_pit_tick = Instant::now();
                let fires = corevm_pit_advance(handle, ticks);
                if fires > 0 {
                    let pic_st = corevm_pic_debug(handle);
                    let mut regs = VcpuRegs::default();
                    corevm_get_vcpu_regs(handle, 0, &mut regs);
                    let if_flag = regs.rflags & 0x200 != 0;
                    diag.log(DiagCategory::Interrupt, format!(
                        "PIT fired {} ticks={} PIC IRR={:#04x} IMR={:#04x} ISR={:#04x} icw={} IF={} exit={} RIP={:#x}",
                        fires, ticks,
                        pic_st & 0xFF, (pic_st >> 8) & 0xFF, (pic_st >> 16) & 0xFF,
                        (pic_st >> 24) & 1, if_flag, exit.reason, regs.rip
                    ));
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
                eprintln!("[vm-state] exit={} RIP={:#x} CS={:#x} CR0={:#x} RFLAGS={:#x} IF={} PE={} PG={} FB={} VGA=[{}] exits=[io:{}/{} mmio:{}/{} hlt:{} cancel:{} shut:{} err:{}]",
                    exit.reason, regs.rip, sregs.cs.selector, sregs.cr0, regs.rflags,
                    if regs.rflags & 0x200 != 0 { 1 } else { 0 },
                    if sregs.cr0 & 1 != 0 { 1 } else { 0 },
                    if sregs.cr0 & (1 << 31) != 0 { 1 } else { 0 },
                    if fb_nonzero { "data" } else { "empty" },
                    vga_line,
                    EXIT_COUNTS[0], EXIT_COUNTS[1],  // IoIn, IoOut
                    EXIT_COUNTS[2], EXIT_COUNTS[3],  // MmioRead, MmioWrite
                    EXIT_COUNTS[7],                   // Halted
                    EXIT_COUNTS[8],                   // InterruptWindow / Cancel
                    EXIT_COUNTS[9],                   // Shutdown
                    EXIT_COUNTS[11],                  // Error
                );
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
        // Graphics mode — try guest physical memory at BAR0 first (WHP path:
        // guest writes directly to RAM at 0xFD000000, bypassing MMIO).
        // Fall back to internal svga.framebuffer (software emulation path).
        let bytes_per_pixel = (vga_bpp as usize + 7) / 8;
        let fb_size = vga_w as usize * vga_h as usize * bytes_per_pixel;
        let mut raw_pixels = vec![0u8; fb_size];
        // Try Bochs VBE default LFB address (0xE0000000) first, then PCI BAR0
        let mut lfb_addr = corevm_vga_get_lfb_addr(handle);
        if lfb_addr == 0 { lfb_addr = 0xE000_0000; }
        // SeaVGABIOS uses 0xE0000000 regardless of BAR0, so try that first
        let mut phys_ret = corevm_read_phys(handle, 0xE000_0000, raw_pixels.as_mut_ptr(), fb_size as u32);
        if phys_ret != 0 {
            phys_ret = corevm_read_phys(handle, lfb_addr, raw_pixels.as_mut_ptr(), fb_size as u32);
        }
        if phys_ret == 0 {
            if let Ok(mut fb_data) = fb.lock() {
                fb_data.text_mode = false;
                fb_data.width = vga_w;
                fb_data.height = vga_h;
                display::render_graphics_mode(&raw_pixels, vga_w, vga_h, vga_bpp, &mut fb_data.pixels);
                fb_data.dirty = true;
            }
        } else {
            // Software emulation fallback: internal svga.framebuffer
            let mut fb_ptr: *const u8 = std::ptr::null();
            let mut fb_len: u32 = 0;
            let ret = corevm_vga_get_framebuffer(handle, &mut fb_ptr, &mut fb_len);
            if ret == 0 && !fb_ptr.is_null() && fb_len > 0 {
                let raw = unsafe { std::slice::from_raw_parts(fb_ptr, fb_len as usize) };
                if let Ok(mut fb_data) = fb.lock() {
                    fb_data.text_mode = false;
                    fb_data.width = vga_w;
                    fb_data.height = vga_h;
                    display::render_graphics_mode(raw, vga_w, vga_h, vga_bpp, &mut fb_data.pixels);
                    fb_data.dirty = true;
                }
            }
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

/// Load BIOS files into the VM
fn load_bios(handle: u64, bios_type: &BiosType) -> Result<(), String> {
    match bios_type {
        BiosType::SeaBios => {
            let bios_path = platform::find_bios("bios.bin")
                .ok_or("SeaBIOS bios.bin not found")?;
            let vgabios_path = platform::find_bios("vgabios.bin")
                .ok_or("VGA BIOS vgabios.bin not found")?;

            let bios = std::fs::read(&bios_path)
                .map_err(|e| format!("Failed to read BIOS: {}", e))?;
            let vgabios = std::fs::read(&vgabios_path)
                .map_err(|e| format!("Failed to read VGA BIOS: {}", e))?;

            // Load full BIOS at 0xC0000 (256KB SeaBIOS covers 0xC0000-0xFFFFF).
            // SeaBIOS needs the reset vector at 0xFFFF0 and its code/data here.
            // During POST, SeaBIOS relocates its init code to high RAM and then
            // loads VGA option ROM from fw_cfg into 0xC0000 (overwriting the
            // no-longer-needed lower portion of the BIOS image).
            corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

            // ROM overlay at top of 4GB address space.
            // This is OUTSIDE the RAM region, so we must create a separate memory
            // mapping. Allocate a page-aligned buffer, copy BIOS into it, and register
            // it as memory slot 1 at the high address.
            let rom_base = 0x1_0000_0000u64 - bios.len() as u64;
            let rom_size = bios.len();
            // Round up to page boundary (4KB)
            let rom_alloc = (rom_size + 0xFFF) & !0xFFF;
            let layout = std::alloc::Layout::from_size_align(rom_alloc, 4096)
                .map_err(|e| format!("ROM layout error: {}", e))?;
            let rom_ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if rom_ptr.is_null() {
                return Err("Failed to allocate ROM memory".into());
            }
            // Copy BIOS data into the ROM buffer
            unsafe {
                std::ptr::copy_nonoverlapping(bios.as_ptr(), rom_ptr, rom_size);
            }
            // Register as memory slot 1 with WHP
            let ret = corevm_set_memory_region(handle, 1, rom_base, rom_alloc as u64, rom_ptr);
            if ret != 0 {
                return Err(format!("Failed to map ROM at 0x{:X}", rom_base));
            }
            // Note: rom_ptr is intentionally leaked — it must remain valid for VM lifetime

            // VGA BIOS: inject via fw_cfg so SeaBIOS loads it as option ROM.
            // SeaBIOS reads fw_cfg file directory during POST, finds the
            // "vgaroms/vgabios.bin" entry, and loads it at 0xC0000 as option ROM.
            {
                let name = b"vgaroms/vgabios.bin";
                let fw_rc = corevm_fw_cfg_add_file(
                    handle,
                    name.as_ptr(), name.len() as u32,
                    vgabios.as_ptr(), vgabios.len() as u32,
                );
                if fw_rc != 0 {
                    return Err(format!(
                        "Failed to add VGA BIOS to fw_cfg (rc={}, vgabios_size={})",
                        fw_rc, vgabios.len()
                    ));
                }
            }

            Ok(())
        }
        BiosType::CoreVm => {
            let bios_path = platform::find_bios("corevm-bios.bin")
                .ok_or("CoreVM BIOS not found")?;
            let bios = std::fs::read(&bios_path)
                .map_err(|e| format!("Failed to read BIOS: {}", e))?;

            corevm_load_binary(handle, 0xF0000, bios.as_ptr(), bios.len() as u32);

            Ok(())
        }
    }
}
