use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::{VmEntry, FrameBufferData};
use crate::config::BiosType;
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
};
use libcorevm::backend::{VcpuRegs, VcpuSregs, SegmentReg};

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

    // Create VM
    let handle = corevm_create(config.ram_mb);
    if handle == 0 {
        return Err("Failed to create VM".into());
    }

    // Create vCPU
    if corevm_create_vcpu(handle, 0) != 0 {
        corevm_destroy(handle);
        return Err("Failed to create vCPU".into());
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
    set_initial_cpu_state(handle);

    // Setup shared state
    let control = Arc::new(VmControl {
        stop: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        exit_reason: Mutex::new(String::new()),
    });

    let fb = entry.framebuffer.clone();
    let control_clone = control.clone();

    // Spawn VM execution thread
    let thread = thread::spawn(move || {
        vm_run_loop(handle, fb, control_clone);
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
    use std::os::unix::io::IntoRawFd;
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
fn set_initial_cpu_state(handle: u64) {
    let mut sregs = VcpuSregs::default();
    corevm_get_vcpu_sregs(handle, 0, &mut sregs);

    // CS: base=0xF0000, selector=0xF000, limit=0xFFFF
    sregs.cs = SegmentReg {
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
    };

    // CR0: PE=0 (real mode)
    sregs.cr0 = 0x10; // ET bit set (FPU extension type)

    corevm_set_vcpu_sregs(handle, 0, &sregs);

    let mut regs = VcpuRegs::default();
    corevm_get_vcpu_regs(handle, 0, &mut regs);
    regs.rip = 0xFFF0;
    regs.rflags = 0x02; // reserved bit
    corevm_set_vcpu_regs(handle, 0, &regs);
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
) {
    let mut last_fb_update = Instant::now();
    let fb_interval = Duration::from_millis(16); // ~60fps

    loop {
        if control.stop.load(Ordering::Relaxed) {
            break;
        }

        if control.pause.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let mut exit = CExitReason::default();
        corevm_run_vcpu(handle, 0, &mut exit);

        match exit.reason {
            0 => {
                // IoIn — dispatch to device, device fills data
                let mut data = [0u8; 4];
                corevm_handle_io_exit(handle, exit.port, 0, exit.size, data.as_mut_ptr());
            }
            1 => {
                // IoOut — dispatch to device
                let mut data = exit.data_u32.to_le_bytes();
                corevm_handle_io_exit(handle, exit.port, 1, exit.size, data.as_mut_ptr());
            }
            2 => {
                // MmioRead — dispatch to device
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr());
            }
            3 => {
                // MmioWrite — dispatch to device
                let mut data = exit.data_u64.to_le_bytes();
                corevm_handle_mmio_exit(handle, exit.addr, 1, exit.size, data.as_mut_ptr());
            }
            7 => {
                // Halted — sleep briefly
                thread::sleep(Duration::from_millis(1));
            }
            9 => {
                // Shutdown — clean exit
                update_framebuffer(handle, &fb);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Shutdown".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            11 => {
                // Error — fatal
                update_framebuffer(handle, &fb);
                if let Ok(mut r) = control.exit_reason.lock() {
                    *r = "Error".into();
                }
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            _ => {
                // Other exits (MsrRead/Write, Cpuid, InterruptWindow, Debug)
                // For now, continue
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
            let _vgabios = std::fs::read(&vgabios_path)
                .map_err(|e| format!("Failed to read VGA BIOS: {}", e))?;

            // Load BIOS at 0xC0000
            corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

            // Load ROM overlay at top of address space (0x100000 - bios_len)
            let rom_base = 0x1_0000_0000u64 - bios.len() as u64;
            corevm_load_binary(handle, rom_base, bios.as_ptr(), bios.len() as u32);

            // VGA BIOS: load at 0xC0000 area (typically separate from main BIOS)
            // SeaBIOS expects VGA BIOS at 0xC0000 via fw_cfg, but without fw_cfg
            // we load it directly at the option ROM area
            // Note: fw_cfg is not available in the new API, so VGA BIOS
            // initialization depends on SVGA device's built-in ROM support.

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
