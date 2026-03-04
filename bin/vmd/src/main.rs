//! vmd — Virtual Machine Daemon for anyOS.
//!
//! Runs VM execution in a dedicated process, decoupled from the UI.
//! The vmmanager GUI spawns this daemon and communicates via IPC:
//!
//! - **Command pipe** (`vmd_cmd`): vmmanager → vmd (text commands)
//! - **Status pipe** (`vmd_status`): vmd → vmmanager (state updates, serial, telemetry)
//! - **Shared memory**: VGA framebuffer (zero-copy display)
//!
//! # SHM Framebuffer Layout
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0      | 4    | width (cols for text, pixels for gfx) |
//! | 4      | 4    | height (rows for text, pixels for gfx) |
//! | 8      | 4    | bpp (0 = text mode, 8/24/32 = graphics) |
//! | 12     | 4    | dirty flag (1 = updated since last read) |
//! | 16     | 4    | vm_state (0=stopped, 1=running, 2=halted, 3=error) |
//! | 20     | 4    | instruction_count low 32 bits |
//! | 24     | 4    | instruction_count high 32 bits |
//! | 28     | 36   | reserved |
//! | 64     | ...  | payload (text: 80*25*2 bytes, gfx: w*h*bpp/8 bytes) |

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::ipc;
use libcorevm_client::{ExitReason, VmHandle};

anyos_std::entry!(main);

/// SHM header size in bytes.
const SHM_HEADER: usize = 64;

/// SHM total size: header + enough for 640x480x32bpp framebuffer + text mode.
/// 4 MiB covers up to 1024x768x32bpp.
const SHM_SIZE: u32 = 4 * 1024 * 1024;

/// Path to the CoreVM BIOS ROM image (custom BIOS, 64 KB, loaded at 0xF0000).
const BIOS_PATH_COREVM: &str = "/Libraries/libcorevm/bios/bios.bin";

/// Path to SeaBIOS ROM image (256 KB, loaded at 0xC0000).
const BIOS_PATH_SEABIOS: &str = "/System/shared/corevm/bios/seabios.bin";

/// Path to VGA BIOS ROM image (option ROM for SeaBIOS).
const VGABIOS_PATH: &str = "/System/shared/corevm/bios/vgabios.bin";

/// Default MAC address for the virtual E1000 NIC.
const DEFAULT_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// E1000 MMIO base address (128 KB region).
const E1000_MMIO_BASE: u64 = 0xD000_0000;

/// Instructions to run per execution batch before checking IPC.
/// Higher = more throughput, lower = more responsive to commands.
const BATCH_SIZE: u64 = 5_000_000;

/// PIT ticks to advance per batch.
/// The real PIT clock is 1,193,182 Hz.  With BATCH_SIZE = 5M instructions
/// and an assumed emulated clock of ~1 GHz, one batch ≈ 5 ms, which
/// corresponds to ~5966 PIT ticks.  We round to 6000.
const PIT_TICKS_PER_BATCH: u32 = 6000;

/// SHM state constants (written to offset 16).
const STATE_STOPPED: u32 = 0;
const STATE_RUNNING: u32 = 1;
const STATE_HALTED: u32 = 2;
const STATE_ERROR: u32 = 3;

/// Runtime state for a single VM instance.
struct VmInstance {
    /// The libcorevm VM handle.
    handle: VmHandle,
    /// Shared memory ID for the VGA framebuffer.
    shm_id: u32,
    /// Mapped shared memory base address.
    shm_ptr: *mut u8,
    /// Whether the VM is currently executing.
    running: bool,
    /// VM name (for logging).
    name: String,
    /// BIOS type from config ("corevm" or "seabios").
    bios_type: String,
    /// Whether JIT acceleration is enabled.
    jit_enabled: bool,
}

/// Global daemon state.
struct DaemonState {
    /// Command pipe ID (vmmanager → vmd).
    cmd_pipe: u32,
    /// Status pipe ID (vmd → vmmanager).
    status_pipe: u32,
    /// Active VM instance (single VM for now).
    vm: Option<VmInstance>,
}

static mut DAEMON: Option<DaemonState> = None;

fn daemon() -> &'static mut DaemonState {
    unsafe { DAEMON.as_mut().unwrap() }
}

// ── IPC helpers ────────────────────────────────────────────────────────

/// Send a status message to vmmanager via the status pipe.
fn send_status(msg: &str) {
    let d = daemon();
    if d.status_pipe != 0 {
        ipc::pipe_write(d.status_pipe, msg.as_bytes());
    }
}

/// Read a command from the command pipe (non-blocking).
/// Returns None if no data available.
fn recv_command() -> Option<String> {
    let d = daemon();
    let mut buf = [0u8; 512];
    let n = ipc::pipe_read(d.cmd_pipe, &mut buf);
    if n == 0 || n == u32::MAX {
        return None;
    }
    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    Some(String::from(text))
}

// ── SHM framebuffer helpers ────────────────────────────────────────────

/// Write a u32 to the SHM header at the given byte offset.
unsafe fn shm_write_u32(ptr: *mut u8, offset: usize, val: u32) {
    let dst = ptr.add(offset) as *mut u32;
    dst.write_volatile(val);
}

/// Update the SHM framebuffer from the VM's VGA state.
fn update_shm_framebuffer(inst: &VmInstance) {
    if inst.shm_ptr.is_null() {
        return;
    }

    let icount = inst.handle.instruction_count();

    // Try text mode first.
    if let Some(text_buf) = inst.handle.vga_text_buffer() {
        let cols: u32 = 80;
        let rows: u32 = 25;
        unsafe {
            shm_write_u32(inst.shm_ptr, 0, cols);
            shm_write_u32(inst.shm_ptr, 4, rows);
            shm_write_u32(inst.shm_ptr, 8, 0); // bpp=0 means text mode
            shm_write_u32(inst.shm_ptr, 20, icount as u32);
            shm_write_u32(inst.shm_ptr, 24, (icount >> 32) as u32);

            // Copy text buffer (u16 cells).
            let payload = inst.shm_ptr.add(SHM_HEADER);
            let src = text_buf.as_ptr() as *const u8;
            let byte_len = (text_buf.len() * 2).min(SHM_SIZE as usize - SHM_HEADER);
            core::ptr::copy_nonoverlapping(src, payload, byte_len);

            // Set dirty flag last (acts as release fence).
            shm_write_u32(inst.shm_ptr, 12, 1);
        }
    } else if let Some((fb, w, h, bpp)) = inst.handle.vga_framebuffer() {
        let bytes_per_pixel = ((bpp as usize) + 7) / 8;
        let byte_len = (w as usize * h as usize * bytes_per_pixel)
            .min(SHM_SIZE as usize - SHM_HEADER);
        unsafe {
            shm_write_u32(inst.shm_ptr, 0, w);
            shm_write_u32(inst.shm_ptr, 4, h);
            shm_write_u32(inst.shm_ptr, 8, bpp as u32);
            shm_write_u32(inst.shm_ptr, 20, icount as u32);
            shm_write_u32(inst.shm_ptr, 24, (icount >> 32) as u32);

            let payload = inst.shm_ptr.add(SHM_HEADER);
            core::ptr::copy_nonoverlapping(fb.as_ptr(), payload, byte_len);

            shm_write_u32(inst.shm_ptr, 12, 1);
        }
    } else {
        // No VGA data yet — just update instruction count.
        unsafe {
            shm_write_u32(inst.shm_ptr, 20, icount as u32);
            shm_write_u32(inst.shm_ptr, 24, (icount >> 32) as u32);
            shm_write_u32(inst.shm_ptr, 12, 1);
        }
    }
}

/// Update the SHM state field.
fn update_shm_state(inst: &VmInstance, state: u32) {
    if !inst.shm_ptr.is_null() {
        unsafe { shm_write_u32(inst.shm_ptr, 16, state); }
    }
}

// ── File I/O helper ────────────────────────────────────────────────────

/// Read an entire file into a Vec<u8>. Returns empty Vec on failure.
fn read_file(path: &str) -> Vec<u8> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Vec::new();
    }
    let size = fs::lseek(fd, 0, 2); // seek end
    if size == 0 || size == u32::MAX {
        fs::close(fd);
        return Vec::new();
    }
    fs::lseek(fd, 0, 0); // seek start
    let mut data = alloc::vec![0u8; size as usize];
    let read = fs::read(fd, &mut data);
    fs::close(fd);
    if read == u32::MAX {
        return Vec::new();
    }
    data.truncate(read as usize);
    data
}

// ── VM config reader ──────────────────────────────────────────────────

/// Directory containing per-VM config files (must match vmmanager).
const VMS_DIR: &str = "/System/shared/vmmanager/vms";

/// Parsed VM configuration from a per-VM config file.
struct VmConfigInfo {
    name: String,
    ram_mb: u32,
    ram_prealloc: bool,
    disk_image: String,
    iso_image: String,
    /// BIOS type: "corevm" (default) or "seabios".
    bios_type: String,
    /// Whether JIT acceleration is enabled.
    jit_enabled: bool,
    /// Whether network adapter is enabled.
    net_enabled: bool,
    /// Network mode: "nat" (default) or "bridge".
    net_mode: String,
    /// Host NIC name for bridged mode.
    net_host_nic: String,
    /// MAC address mode: "dynamic" (default) or "static".
    mac_mode: String,
    /// Static MAC address (e.g. "52:54:00:12:34:56").
    mac_address: String,
}

/// Read the VM config file for the given UUID.
///
/// Opens `<VMS_DIR>/<uuid>.conf` and parses key=value fields.
/// Returns `None` if the file cannot be read or is empty.
fn read_vm_config(uuid: &str) -> Option<VmConfigInfo> {
    // Build path: "/System/shared/vmmanager/vms/<uuid>.conf"
    let mut path_buf = [0u8; 128];
    let dir = VMS_DIR.as_bytes();
    let ext = b".conf";
    let uuid_b = uuid.as_bytes();
    let mut p = 0;
    for &b in dir {
        path_buf[p] = b;
        p += 1;
    }
    path_buf[p] = b'/';
    p += 1;
    for &b in uuid_b {
        if p < 127 {
            path_buf[p] = b;
            p += 1;
        }
    }
    for &b in ext {
        if p < 127 {
            path_buf[p] = b;
            p += 1;
        }
    }
    let path = core::str::from_utf8(&path_buf[..p]).unwrap_or("");

    let data = read_file(path);
    if data.is_empty() {
        return None;
    }

    let text = core::str::from_utf8(&data).unwrap_or("");
    let mut name = String::new();
    let mut ram_mb: u32 = 64;
    let mut ram_prealloc = false;
    let mut disk_image = String::new();
    let mut iso_image = String::new();
    let mut bios_type = String::from("corevm");
    let mut jit_enabled = false;
    let mut net_enabled = false;
    let mut net_mode = String::from("nat");
    let mut net_host_nic = String::new();
    let mut mac_mode = String::from("dynamic");
    let mut mac_address = String::new();

    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(val) = line.strip_prefix("name=") {
            name = String::from(val);
        } else if let Some(val) = line.strip_prefix("ram=") {
            ram_mb = parse_u32(val);
            if ram_mb == 0 {
                ram_mb = 64;
            }
        } else if let Some(val) = line.strip_prefix("ram_alloc=") {
            ram_prealloc = val == "prealloc";
        } else if let Some(val) = line.strip_prefix("disk=") {
            disk_image = String::from(val);
        } else if let Some(val) = line.strip_prefix("iso=") {
            iso_image = String::from(val);
        } else if let Some(val) = line.strip_prefix("bios=") {
            bios_type = String::from(val);
        } else if let Some(val) = line.strip_prefix("jit=") {
            jit_enabled = val == "1";
        } else if let Some(val) = line.strip_prefix("net_enabled=") {
            net_enabled = val == "1";
        } else if let Some(val) = line.strip_prefix("net_mode=") {
            net_mode = String::from(val);
        } else if let Some(val) = line.strip_prefix("net_host_nic=") {
            net_host_nic = String::from(val);
        } else if let Some(val) = line.strip_prefix("mac_mode=") {
            mac_mode = String::from(val);
        } else if let Some(val) = line.strip_prefix("mac_address=") {
            mac_address = String::from(val);
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(VmConfigInfo {
        name,
        ram_mb,
        ram_prealloc,
        disk_image,
        iso_image,
        bios_type,
        jit_enabled,
        net_enabled,
        net_mode,
        net_host_nic,
        mac_mode,
        mac_address,
    })
}

// ── Command handlers ───────────────────────────────────────────────────

/// Handle `create <uuid>` command.
///
/// Reads the VM configuration from the shared config file by UUID,
/// creates the VM with the configured RAM, and attaches disk/ISO.
fn cmd_create(uuid: &str) {
    let d = daemon();

    // Destroy any existing VM.
    if let Some(ref inst) = d.vm {
        update_shm_state(inst, STATE_STOPPED);
        if inst.shm_id != 0 {
            ipc::shm_destroy(inst.shm_id);
        }
    }
    d.vm = None;

    // Read VM config from the per-VM file.
    let config = match read_vm_config(uuid) {
        Some(c) => c,
        None => {
            send_status(&format!("error 0 VM config not found for UUID {}", uuid));
            anyos_std::println!("[vmd] ERROR: config not found for UUID {}", uuid);
            return;
        }
    };

    // Create VM with configured RAM.
    let handle = match VmHandle::new(config.ram_mb) {
        Some(h) => h,
        None => {
            send_status("error 0 failed to create VM (out of memory?)");
            return;
        }
    };

    // Set up standard PC devices.
    handle.setup_standard_devices();
    handle.setup_ide();

    // Create shared memory for VGA framebuffer.
    let shm_id = ipc::shm_create(SHM_SIZE);
    let shm_addr = if shm_id != 0 { ipc::shm_map(shm_id) } else { 0 };
    let shm_ptr = if shm_addr != 0 { shm_addr as *mut u8 } else { core::ptr::null_mut() };

    // Zero out SHM header.
    if !shm_ptr.is_null() {
        unsafe {
            core::ptr::write_bytes(shm_ptr, 0, SHM_HEADER);
        }
    }

    // Set up E1000 network adapter if enabled.
    if config.net_enabled {
        handle.setup_pci_bus();
        let mac = if config.mac_mode == "static" && config.mac_address.len() >= 17 {
            parse_mac(&config.mac_address)
        } else {
            DEFAULT_MAC
        };
        handle.setup_e1000(E1000_MMIO_BASE, &mac);
        anyos_std::println!(
            "[vmd] E1000 NIC enabled (mode={}, mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X})",
            config.net_mode, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
    }

    let inst = VmInstance {
        handle,
        shm_id,
        shm_ptr,
        running: false,
        name: config.name.clone(),
        bios_type: config.bios_type.clone(),
        jit_enabled: config.jit_enabled,
    };

    d.vm = Some(inst);

    // Report success with SHM ID BEFORE loading disk/ISO.
    // vmmanager needs the SHM ID promptly; disk/ISO loading can be slow.
    send_status(&format!("created 0 {}", shm_id));
    anyos_std::println!("[vmd] VM '{}' created ({} MiB RAM, shm={})", config.name, config.ram_mb, shm_id);

    // Attach disk image if configured.
    if !config.disk_image.is_empty() {
        let data = read_file(&config.disk_image);
        if !data.is_empty() {
            if let Some(ref inst) = d.vm {
                inst.handle.ide_attach_disk(&data);
            }
            anyos_std::println!("[vmd] attached disk: {} ({} bytes)", config.disk_image, data.len());
        } else {
            send_status(&format!("error 0 failed to read disk image: {}", config.disk_image));
        }
    }

    // Attach ISO as IDE disk if configured (allows BIOS to boot from it).
    if !config.iso_image.is_empty() {
        let data = read_file(&config.iso_image);
        if !data.is_empty() {
            if let Some(ref inst) = d.vm {
                inst.handle.ide_attach_disk(&data);
            }
            anyos_std::println!("[vmd] attached ISO as IDE disk: {} ({} bytes)", config.iso_image, data.len());
        }
    }
}

/// Handle `start` command — load BIOS and begin execution.
///
/// The BIOS type is determined from the VM config. VMD automatically
/// maps each BIOS to the correct physical address:
/// - **CoreVM**: 64 KB loaded at 0xF0000 (reset vector at 0xFFF0)
/// - **SeaBIOS**: 256 KB loaded at 0xC0000 (covers 0xC0000–0xFFFFF,
///   reset vector at 0xFFF0). VGA BIOS is provided via fw_cfg for
///   SeaBIOS option ROM scanning.
fn cmd_start() {
    let d = daemon();
    if let Some(ref mut inst) = d.vm {
        if inst.running {
            return;
        }

        // Select and load BIOS based on configured type.
        let is_seabios = inst.bios_type == "seabios";

        if is_seabios {
            // ── SeaBIOS ROM mapping ──────────────────────────────────
            //
            // SeaBIOS is a 256 KB binary with internal relative jumps
            // spanning the full image. The entire image must be loaded
            // contiguously into writable RAM at 0xC0000-0xFFFFF so that:
            //   - The reset vector at 0xFFFF0 contains the correct entry
            //     point (bios_data[0x3FFF0] for a 256 KB image).
            //   - Relative calls/jumps between the upper and lower halves
            //     of the image resolve to correct addresses.
            //   - SeaBIOS can write to its own data structures in-place.
            //
            // Additionally, a read-only ROM overlay at 0xFFFC0000 (top of
            // 4 GiB) provides access when SeaBIOS switches to 32-bit
            // protected mode and uses its linked addresses.
            //
            // The VGA BIOS is NOT loaded into RAM at 0xC0000 (that would
            // overwrite SeaBIOS code). Instead it is provided exclusively
            // via fw_cfg — SeaBIOS loads option ROMs from fw_cfg during
            // POST and places them at 0xC0000 itself.
            let bios_data = read_file(BIOS_PATH_SEABIOS);
            if bios_data.is_empty() {
                send_status("error 0 SeaBIOS not found");
                anyos_std::println!("[vmd] ERROR: SeaBIOS not found at {}", BIOS_PATH_SEABIOS);
                return;
            }

            // 1. Full 256 KB into writable RAM at 0xC0000-0xFFFFF.
            //    Reset vector at physical 0xFFFF0 = bios_data[size - 16].
            inst.handle.load_binary(0xC0000, &bios_data);
            anyos_std::println!(
                "[vmd] SeaBIOS loaded at 0xC0000 ({} bytes, writable)", bios_data.len()
            );

            // 2. Read-only ROM overlay at top of 4 GiB for 32-bit mode.
            inst.handle.load_rom(0xFFFC_0000, &bios_data);
            anyos_std::println!(
                "[vmd] SeaBIOS ROM overlay at 0xFFFC0000 ({} bytes)", bios_data.len()
            );

            inst.handle.set_rip(0xFFF0);

            // 3. Provide VGA BIOS via fw_cfg only (not in RAM).
            //    SeaBIOS's option ROM scanner loads it from fw_cfg and
            //    places it at 0xC0000 during POST (overwriting the first
            //    part of the BIOS image, which is no longer needed by then).
            let vgabios = read_file(VGABIOS_PATH);
            if !vgabios.is_empty() {
                inst.handle.fw_cfg_add_file("vgaroms/vgabios.bin", &vgabios);
                anyos_std::println!(
                    "[vmd] VGA BIOS via fw_cfg ({} bytes)", vgabios.len()
                );
            }
        } else {
            // ── CoreVM BIOS (64 KB at 0xF0000) ─────────────────────
            let bios_data = read_file(BIOS_PATH_COREVM);
            if bios_data.is_empty() {
                send_status("error 0 BIOS not found");
                anyos_std::println!(
                    "[vmd] ERROR: CoreVM BIOS not found at {}", BIOS_PATH_COREVM
                );
                return;
            }
            inst.handle.load_binary(0xF0000, &bios_data);
            inst.handle.set_rip(0xFFF0);
            anyos_std::println!(
                "[vmd] loaded CoreVM BIOS ({} bytes at 0xF0000)", bios_data.len()
            );
        }

        // Enable JIT acceleration if configured.
        if inst.jit_enabled {
            inst.handle.jit_enable(true);
            anyos_std::println!("[vmd] JIT acceleration enabled");
        }

        // Log MMIO diagnostic info before starting execution.
        let (reg_count, mmio_lo, mmio_hi, _) = inst.handle.mmio_diag();
        anyos_std::println!(
            "[vmd] MMIO diag: {} regions, bounds=[0x{:X}, 0x{:X})",
            reg_count, mmio_lo, mmio_hi
        );

        inst.running = true;
        update_shm_state(inst, STATE_RUNNING);
        send_status("state 0 running");
        anyos_std::println!("[vmd] VM '{}' started", inst.name);
    }
}

/// Handle `stop` command.
fn cmd_stop() {
    let d = daemon();
    if let Some(ref mut inst) = d.vm {
        if !inst.running {
            return;
        }
        inst.handle.request_stop();
        inst.running = false;
        update_shm_state(inst, STATE_STOPPED);
        send_status("state 0 stopped");
        anyos_std::println!("[vmd] VM '{}' stopped", inst.name);
    }
}

/// Handle `key <scancode>` command.
fn cmd_key(scancode: u8) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        if inst.running {
            inst.handle.ps2_key_press(scancode);
            inst.handle.ps2_key_release(scancode);
        }
    }
}

/// Handle `mouse <dx> <dy> <buttons>` command.
fn cmd_mouse(dx: i16, dy: i16, buttons: u8) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        if inst.running {
            inst.handle.ps2_mouse_move(dx, dy, buttons);
        }
    }
}

// ── Command dispatch ───────────────────────────────────────────────────

/// Parse and execute a single command line.
fn dispatch_command(line: &str) {
    let parts: Vec<&str> = line.trim().splitn(4, ' ').collect();
    if parts.is_empty() {
        return;
    }

    match parts[0] {
        "create" => {
            if parts.len() >= 2 {
                cmd_create(parts[1]); // parts[1] = UUID (no spaces)
            }
        }
        "start" => cmd_start(),
        "stop" => cmd_stop(),
        "destroy" => {
            let d = daemon();
            if let Some(ref inst) = d.vm {
                update_shm_state(inst, STATE_STOPPED);
                if inst.shm_id != 0 {
                    ipc::shm_destroy(inst.shm_id);
                }
            }
            d.vm = None;
            send_status("state 0 destroyed");
        }
        "key" => {
            if parts.len() >= 2 {
                let sc = parse_u32(parts[1]) as u8;
                cmd_key(sc);
            }
        }
        "mouse" => {
            if parts.len() >= 4 {
                let dx = parse_i16(parts[1]);
                let dy = parse_i16(parts[2]);
                let btn = parse_u32(parts[3]) as u8;
                cmd_mouse(dx, dy, btn);
            }
        }
        "quit" => {
            let d = daemon();
            if let Some(ref inst) = d.vm {
                update_shm_state(inst, STATE_STOPPED);
                if inst.shm_id != 0 {
                    ipc::shm_destroy(inst.shm_id);
                }
            }
            d.vm = None;
            anyos_std::println!("[vmd] shutting down");
            anyos_std::process::exit(0);
        }
        _ => {
            anyos_std::println!("[vmd] unknown command: {}", parts[0]);
        }
    }
}

// ── VM execution ───────────────────────────────────────────────────────

/// Run one execution batch for the active VM.
/// Returns true if the VM is still running after this batch.
fn run_vm_batch() -> bool {
    let d = daemon();
    let inst = match d.vm.as_mut() {
        Some(i) if i.running => i,
        _ => return false,
    };

    // Advance PIT and deliver timer interrupts.
    for _ in 0..PIT_TICKS_PER_BATCH {
        if inst.handle.pit_tick() {
            inst.handle.pic_raise_irq(0);
        }
    }

    // Execute instructions.
    let exit = inst.handle.run(BATCH_SIZE);

    match exit {
        ExitReason::Halted => {
            // HLT pauses until the next interrupt — deliver a PIT tick
            // and resume. This is critical: SeaBIOS uses HLT during POST
            // to wait for timer events. Sleep briefly to avoid busy-spinning.
            anyos_std::process::sleep(1);
            if inst.handle.pit_tick() {
                inst.handle.pic_raise_irq(0);
            }
            // Drain serial and debug port output (SeaBIOS debug messages).
            let serial_out = inst.handle.serial_take_output_vec();
            if !serial_out.is_empty() {
                if let Ok(text) = core::str::from_utf8(&serial_out) {
                    anyos_std::print!("{}", text);
                }
            }
            let debug_out = inst.handle.debug_take_output_vec();
            if !debug_out.is_empty() {
                if let Ok(text) = core::str::from_utf8(&debug_out) {
                    anyos_std::print!("{}", text);
                }
            }
            // Update framebuffer on HLT (SeaBIOS may have written to VGA).
            update_shm_framebuffer(inst);
            // Continue running — HLT is not a terminal state.
            return true;
        }
        ExitReason::Exception => {
            inst.running = false;
            update_shm_state(inst, STATE_ERROR);
            update_shm_framebuffer(inst);
            let rip = inst.handle.last_error_rip();
            if let Some(ref msg) = inst.handle.last_error() {
                send_status(&format!("error 0 Exception at RIP=0x{:X}: {}", rip, msg));
                anyos_std::println!("[vmd] exception at RIP=0x{:X}: {}", rip, msg);
            } else {
                send_status("error 0 unrecoverable exception");
                anyos_std::println!("[vmd] unrecoverable exception at RIP=0x{:X}", rip);
            }
            return false;
        }
        ExitReason::InstructionLimit => {
            // Normal: ran full batch, continue.
        }
        ExitReason::StopRequested => {
            inst.running = false;
            update_shm_state(inst, STATE_STOPPED);
            update_shm_framebuffer(inst);
            send_status("state 0 stopped");
            return false;
        }
        ExitReason::Breakpoint => {
            // Continue running after breakpoint.
        }
    }

    // Drain serial output and forward to vmmanager.
    let serial_out = inst.handle.serial_take_output_vec();
    if !serial_out.is_empty() {
        if let Ok(text) = core::str::from_utf8(&serial_out) {
            anyos_std::print!("{}", text);
            send_status(&format!("serial 0 {}", text));
        }
    }

    // Drain debug port output (SeaBIOS writes to port 0x402).
    let debug_out = inst.handle.debug_take_output_vec();
    if !debug_out.is_empty() {
        if let Ok(text) = core::str::from_utf8(&debug_out) {
            anyos_std::print!("{}", text);
        }
    }

    // Update shared memory framebuffer.
    update_shm_framebuffer(inst);

    true
}

// ── Number parsing (no_std) ────────────────────────────────────────────

/// Parse a decimal u32 from a string.
fn parse_u32(s: &str) -> u32 {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        }
    }
    val
}

/// Parse a MAC address string ("XX:XX:XX:XX:XX:XX") into a 6-byte array.
///
/// Returns `DEFAULT_MAC` if the string cannot be parsed.
fn parse_mac(s: &str) -> [u8; 6] {
    let mut mac = DEFAULT_MAC;
    let bytes = s.as_bytes();
    let mut idx = 0;
    let mut pos = 0;
    while idx < 6 && pos < bytes.len() {
        let hi = hex_digit(bytes[pos]);
        if hi > 15 {
            return DEFAULT_MAC;
        }
        pos += 1;
        if pos >= bytes.len() {
            return DEFAULT_MAC;
        }
        let lo = hex_digit(bytes[pos]);
        if lo > 15 {
            return DEFAULT_MAC;
        }
        mac[idx] = (hi << 4) | lo;
        idx += 1;
        pos += 1;
        // Skip separator (: or -)
        if pos < bytes.len() && (bytes[pos] == b':' || bytes[pos] == b'-') {
            pos += 1;
        }
    }
    mac
}

/// Convert a hex ASCII digit to its numeric value (0–15).
/// Returns 0xFF for invalid digits.
fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0xFF,
    }
}

/// Parse a decimal i16 from a string (supports negative).
fn parse_i16(s: &str) -> i16 {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let (neg, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
    let mut val: i32 = 0;
    for &b in &bytes[start..] {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as i32;
        }
    }
    if neg { -val as i16 } else { val as i16 }
}

// ── Entry point ────────────────────────────────────────────────────────

fn main() {
    anyos_std::println!("[vmd] starting...");

    // Initialize libcorevm.
    if !libcorevm_client::init() {
        anyos_std::println!("[vmd] ERROR: failed to load libcorevm.so");
        anyos_std::process::exit(1);
    }

    // Open IPC pipes (created by vmmanager before spawning us).
    let cmd_pipe = ipc::pipe_open("vmd_cmd");
    let status_pipe = ipc::pipe_open("vmd_status");

    if cmd_pipe == 0 {
        anyos_std::println!("[vmd] ERROR: cannot open vmd_cmd pipe");
        anyos_std::process::exit(1);
    }
    if status_pipe == 0 {
        anyos_std::println!("[vmd] ERROR: cannot open vmd_status pipe");
        anyos_std::process::exit(1);
    }

    anyos_std::println!("[vmd] IPC pipes connected (cmd={}, status={})", cmd_pipe, status_pipe);

    unsafe {
        DAEMON = Some(DaemonState {
            cmd_pipe,
            status_pipe,
            vm: None,
        });
    }

    // Signal readiness.
    send_status("ready");

    // Main loop: poll commands, run VM, repeat.
    loop {
        // Process all pending commands.
        loop {
            match recv_command() {
                Some(cmd) => {
                    // Commands may contain multiple lines (unlikely but handle it).
                    for line in cmd.split('\n') {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            dispatch_command(trimmed);
                        }
                    }
                }
                None => break,
            }
        }

        // Run VM execution batch if active.
        let vm_active = run_vm_batch();

        // Sleep briefly to avoid 100% CPU usage.
        if !vm_active {
            anyos_std::process::sleep(10);
        } else {
            anyos_std::process::sleep(1);
        }
    }
}
