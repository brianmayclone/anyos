use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};
use std::ffi::CString;

use crate::app::{VmEntry, FrameBufferData};
use crate::config::BiosType;
use crate::display;
use crate::platform;
use crate::sidebar::VmState;

use std::sync::atomic::AtomicU64;

/// Shared control flags and metrics for the VM thread
pub struct VmControl {
    pub stop: AtomicBool,
    pub pause: AtomicBool,
    pub exited: AtomicBool,
    pub exit_reason: AtomicU64,
    pub total_instructions: AtomicU64,
    pub mips: AtomicU64, // stored as f64 bits
}

/// Start a VM. Sets up libcorevm, spawns execution thread.
pub fn start_vm(entry: &mut VmEntry) -> Result<(), String> {
    let config = &entry.config;

    // Create VM
    let handle = libcorevm::corevm_create_ex(config.ram_mb, config.cpu_cores);
    if handle == 0 {
        return Err("Failed to create VM".into());
    }

    // Setup devices
    libcorevm::corevm_setup_standard_devices(handle);
    libcorevm::corevm_setup_pci_bus(handle);
    libcorevm::corevm_setup_ide(handle);

    // Load BIOS
    load_bios(handle, &config.bios_type)?;

    // Attach ISO if configured
    if !config.iso_image.is_empty() {
        let iso_data = std::fs::read(&config.iso_image)
            .map_err(|e| format!("Failed to read ISO: {}", e))?;
        libcorevm::corevm_ide_attach_slave(handle, iso_data.as_ptr(), iso_data.len() as u32);
        // VM needs the data for its lifetime
        std::mem::forget(iso_data);
    }

    // Attach disk if configured
    if !config.disk_image.is_empty() {
        let disk_data = std::fs::read(&config.disk_image)
            .map_err(|e| format!("Failed to read disk: {}", e))?;
        libcorevm::corevm_ide_attach_disk(handle, disk_data.as_ptr(), disk_data.len() as u32);
        std::mem::forget(disk_data);
    }

    // Enable JIT if configured
    if config.jit_enabled {
        libcorevm::corevm_jit_enable(handle, 1);
    }

    // Setup shared state
    let control = Arc::new(VmControl {
        stop: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        exit_reason: AtomicU64::new(0),
        total_instructions: AtomicU64::new(0),
        mips: AtomicU64::new(0),
    });

    let fb = entry.framebuffer.clone();
    let control_clone = control.clone();

    // Spawn VM execution thread
    let thread = thread::spawn(move || {
        vm_run_loop(handle, fb, control_clone);
        libcorevm::corevm_destroy(handle);
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
    if let Some(handle) = entry.vm_handle {
        libcorevm::corevm_request_stop(handle);
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

/// Run a batch of instructions, polling IDE IRQs between sub-batches.
/// The IDE poll quantum must be large enough that the wall-clock overhead
/// between corevm_run calls does not dominate — the PIT timer inside
/// corevm_run only measures time *within* each call.
fn run_batch_with_irq_poll(handle: u64, batch: u64) -> u32 {
    const IDE_IRQ_POLL_QUANTUM: u64 = 500_000;
    let mut remaining = batch.max(1);
    let mut exit_code = 2;

    while remaining > 0 {
        let chunk = remaining.min(IDE_IRQ_POLL_QUANTUM);
        exit_code = libcorevm::corevm_run(handle, chunk);
        if libcorevm::corevm_ide_irq_raised(handle) != 0 {
            libcorevm::corevm_pic_raise_irq(handle, 14);
            libcorevm::corevm_ide_clear_irq(handle);
        }
        if exit_code != 2 {
            break;
        }
        remaining -= chunk;
    }

    exit_code
}

/// The main VM execution loop (runs in dedicated thread)
fn vm_run_loop(
    handle: u64,
    fb: Arc<Mutex<FrameBufferData>>,
    control: Arc<VmControl>,
) {
    let batch_size: u64 = 500_000;
    let mut last_fb_update = Instant::now();
    let fb_interval = Duration::from_millis(16); // ~60fps
    let mut last_metrics = Instant::now();
    let mut instructions_since_last: u64 = 0;
    let mut total: u64 = 0;

    loop {
        if control.stop.load(Ordering::Relaxed) {
            break;
        }

        if control.pause.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        // Run a batch with frequent IDE IRQ polling (every 1024 instructions).
        // Do NOT call corevm_pit_tick — corevm_run handles PIT internally
        // via wall-clock time. Calling pit_tick sets pit_is_externally_clocked
        // which disables the automatic PIT.
        let exit_reason = run_batch_with_irq_poll(handle, batch_size);
        instructions_since_last += batch_size;
        total += batch_size;

        // Update framebuffer at ~60fps
        if last_fb_update.elapsed() >= fb_interval {
            update_framebuffer(handle, &fb);
            last_fb_update = Instant::now();
        }

        // Update metrics every 500ms
        let metrics_elapsed = last_metrics.elapsed();
        if metrics_elapsed >= Duration::from_millis(500) {
            let secs = metrics_elapsed.as_secs_f64();
            let mips_val = (instructions_since_last as f64) / secs / 1_000_000.0;
            control.mips.store(mips_val.to_bits(), Ordering::Relaxed);
            control.total_instructions.store(total, Ordering::Relaxed);
            instructions_since_last = 0;
            last_metrics = Instant::now();
        }

        match exit_reason {
            2 => {
                // InstructionLimit — normal, batch completed, continue
            }
            0 => {
                // HLT — sleep briefly, interrupts might wake it
                thread::sleep(Duration::from_millis(1));
            }
            1 => {
                // Exception — fatal, stop VM
                update_framebuffer(handle, &fb);
                control.exit_reason.store(exit_reason as u64, Ordering::Relaxed);
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            4 => {
                // StopRequested — clean shutdown
                update_framebuffer(handle, &fb);
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
            _ => {
                // Breakpoint, PS2Reset, etc.
                update_framebuffer(handle, &fb);
                control.exit_reason.store(exit_reason as u64, Ordering::Relaxed);
                control.exited.store(true, Ordering::Relaxed);
                break;
            }
        }
    }
}

/// Read VGA state and update the shared framebuffer
fn update_framebuffer(handle: u64, fb: &Arc<Mutex<FrameBufferData>>) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    let mut bpp: u8 = 0;

    let fb_ptr = libcorevm::corevm_vga_get_framebuffer(handle, &mut w, &mut h, &mut bpp);

    if !fb_ptr.is_null() && w > 0 && h > 0 {
        let fb_size = (w as usize) * (h as usize) * (bpp as usize) / 8;
        let raw_pixels = unsafe { std::slice::from_raw_parts(fb_ptr, fb_size) };

        if let Ok(mut fb_data) = fb.lock() {
            fb_data.text_mode = false;
            fb_data.width = w;
            fb_data.height = h;
            display::render_graphics_mode(raw_pixels, w, h, bpp, &mut fb_data.pixels);
            fb_data.dirty = true;
        }
    } else {
        // Try text mode
        let mut count: u32 = 0;
        let text_ptr = libcorevm::corevm_vga_get_text_buffer(handle, &mut count);

        if !text_ptr.is_null() && count > 0 {
            let text_cells = unsafe { std::slice::from_raw_parts(text_ptr, count as usize) };

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
            libcorevm::corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

            // Load ROM overlay at top of address space
            libcorevm::corevm_load_rom(handle, 0xFFFC_0000, bios.as_ptr(), bios.len() as u32);

            // Add VGA BIOS via fw_cfg
            let name = CString::new("vgaroms/vgabios.bin").unwrap();
            libcorevm::corevm_fw_cfg_add_file(
                handle,
                name.as_ptr() as *const u8,
                vgabios.as_ptr(),
                vgabios.len() as u32,
            );

            Ok(())
        }
        BiosType::CoreVm => {
            let bios_path = platform::find_bios("corevm-bios.bin")
                .ok_or("CoreVM BIOS not found")?;
            let bios = std::fs::read(&bios_path)
                .map_err(|e| format!("Failed to read BIOS: {}", e))?;

            libcorevm::corevm_load_rom(handle, 0xF0000, bios.as_ptr(), bios.len() as u32);

            Ok(())
        }
    }
}
