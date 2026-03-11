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
    corevm_run_vcpu, corevm_handle_io_exit, corevm_handle_mmio_exit,
    corevm_setup_standard_devices, corevm_setup_ahci,
    corevm_ahci_attach_disk, corevm_ahci_attach_cdrom,
    corevm_load_binary,
    corevm_get_vcpu_regs, corevm_set_vcpu_regs,
    corevm_get_vcpu_sregs, corevm_set_vcpu_sregs,
    corevm_vga_get_framebuffer, corevm_vga_get_text_buffer,
    corevm_last_error, corevm_last_error_len,
    corevm_pit_advance, corevm_debug_port_take_output,
    corevm_fw_cfg_add_file,
};
use libcorevm::backend::{VcpuRegs, VcpuSregs, SegmentReg, DescriptorTable};

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
    const PIT_FREQ: u64 = 1_193_182; // 8254 PIT clock rate in Hz

    loop {
        if control.stop.load(Ordering::Relaxed) {
            break;
        }

        if control.pause.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let mut exit = CExitReason::default();
        let rc = corevm_run_vcpu(handle, 0, &mut exit);
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
                if diag_enabled {
                    diag.log(DiagCategory::IoPort, format!("OUT port=0x{:04X} size={} data=0x{:X}", exit.port, exit.size, exit.data_u32));
                }
                let mut data = exit.data_u32.to_le_bytes();
                corevm_handle_io_exit(handle, exit.port, 1, exit.size, data.as_mut_ptr());
            }
            2 => {
                // MmioRead — dispatch to device
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg);
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
                corevm_handle_mmio_exit(handle, exit.addr, 1, exit.size, data.as_mut_ptr(), 0);
            }
            7 => {
                // Halted — sleep briefly
                if diag_enabled {
                    diag.log(DiagCategory::CpuState, "HLT".to_string());
                }
                thread::sleep(Duration::from_millis(1));
            }
            9 => {
                // Shutdown — clean exit
                if diag_enabled {
                    diag.log(DiagCategory::Error, "VM Shutdown (triple fault)".to_string());
                }
                update_framebuffer(handle, &fb);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Shutdown".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            11 => {
                // Error — fatal
                if diag_enabled {
                    diag.log(DiagCategory::Error, "VM Error exit".to_string());
                }
                update_framebuffer(handle, &fb);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Error".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            other => {
                // Other exits (MsrRead/Write, Cpuid, InterruptWindow, Debug)
                if diag_enabled {
                    diag.log(DiagCategory::Info, format!("Unhandled exit reason={}", other));
                }
            }
        }

        // Advance PIT based on wall-clock elapsed time; IRQ0 injection handled internally
        // Cap at ~1ms worth of ticks to avoid blocking on large time deltas
        let pit_elapsed = last_pit_tick.elapsed();
        if pit_elapsed.as_micros() > 0 {
            let ticks = ((pit_elapsed.as_micros() as u64 * PIT_FREQ / 1_000_000) as u32).min(PIT_FREQ as u32 / 100);
            if ticks > 0 {
                last_pit_tick = Instant::now();
                corevm_pit_advance(handle, ticks);
            }
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
            update_framebuffer(handle, &fb);
            last_fb_update = Instant::now();
        }
    }
}

/// Read VGA state and update the shared framebuffer
fn update_framebuffer(handle: u64, fb: &Arc<Mutex<FrameBufferData>>) {
    let mut fb_ptr: *const u8 = std::ptr::null();
    let mut fb_len: u32 = 0;

    let ret = corevm_vga_get_framebuffer(handle, &mut fb_ptr, &mut fb_len);

    if ret == 0 && !fb_ptr.is_null() && fb_len > 0 {
        // The framebuffer returns raw pixel data; we need width/height/bpp from SVGA.
        // For now, assume 32bpp and derive dimensions from fb_len.
        // TODO: expose width/height/bpp via FFI
        let raw_pixels = unsafe { std::slice::from_raw_parts(fb_ptr, fb_len as usize) };

        if let Ok(mut fb_data) = fb.lock() {
            // Try to determine dimensions from the data size
            // Common resolutions: 640x480, 800x600, 1024x768, 1280x1024
            let (w, h, bpp) = guess_resolution(fb_len as usize);
            if w > 0 && h > 0 {
                fb_data.text_mode = false;
                fb_data.width = w;
                fb_data.height = h;
                display::render_graphics_mode(raw_pixels, w, h, bpp, &mut fb_data.pixels);
                fb_data.dirty = true;
            }
        }
    } else {
        // Try text mode
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

            // Load BIOS at 0xC0000
            corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

            // Load ROM overlay at top of address space (0x100000 - bios_len)
            let rom_base = 0x1_0000_0000u64 - bios.len() as u64;
            corevm_load_binary(handle, rom_base, bios.as_ptr(), bios.len() as u32);

            // VGA BIOS: inject via fw_cfg so SeaBIOS loads it as option ROM
            {
                let name = b"vgaroms/vgabios.bin";
                corevm_fw_cfg_add_file(
                    handle,
                    name.as_ptr(), name.len() as u32,
                    vgabios.as_ptr(), vgabios.len() as u32,
                );
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
