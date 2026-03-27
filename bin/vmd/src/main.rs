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
//! | 20     | 4    | reserved (was instruction_count low) |
//! | 24     | 4    | reserved (was instruction_count high) |
//! | 28     | 36   | reserved |
//! | 64     | ...  | payload (text: 80*25*2 bytes, gfx: w*h*bpp/8 bytes) |

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::process::Thread;
use anyos_std::sys;
use core::sync::atomic::{AtomicBool, Ordering};
use libcorevm_client::{VmExitReason, VmHandle};

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

/// Number of VM exits between framebuffer updates.
const FB_UPDATE_INTERVAL: u64 = 200;

/// SHM state constants (written to offset 16).
const STATE_STOPPED: u32 = 0;
const STATE_RUNNING: u32 = 1;
const _STATE_HALTED: u32 = 2;
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
    /// Boot order from config ("disk", "cd", or "floppy").
    boot_order: String,
    /// Exit counter for periodic tasks.
    exit_count: u64,
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

// ── Management thread shared state ──────────────────────────────────────

/// Command pipe ID, set before spawning the management thread.
static mut MGMT_CMD_PIPE: u32 = 0;

/// Command buffer shared between management thread (writer) and main loop (reader).
static mut CMD_BUF: [u8; 512] = [0; 512];
static mut CMD_BUF_LEN: u32 = 0;

/// Set by management thread when a new command is ready.
static CMD_PENDING: AtomicBool = AtomicBool::new(false);

/// Set by main thread to signal the management thread to exit.
static MGMT_STOP: AtomicBool = AtomicBool::new(false);

/// Management thread entry point.
///
/// Continuously reads from the command pipe and forwards commands to the
/// main loop via the atomic CMD_PENDING slot.  This runs in its own thread
/// so IPC stays responsive even when the VM is executing.
fn mgmt_thread_entry() {
    let pipe_id = unsafe { MGMT_CMD_PIPE };
    let mut buf = [0u8; 512];

    loop {
        if MGMT_STOP.load(Ordering::Acquire) {
            break;
        }

        let n = ipc::pipe_read(pipe_id, &mut buf);
        if n > 0 && n != u32::MAX {
            anyos_std::println!("[vmd-mgmt] received {} bytes: {}", n,
                core::str::from_utf8(&buf[..n as usize]).unwrap_or("(invalid utf8)"));
            // Wait for main thread to consume the previous command.
            while CMD_PENDING.load(Ordering::Acquire) {
                if MGMT_STOP.load(Ordering::Relaxed) {
                    return;
                }
                anyos_std::process::yield_cpu();
            }

            let len = (n as usize).min(512);
            unsafe {
                CMD_BUF[..len].copy_from_slice(&buf[..len]);
                CMD_BUF_LEN = len as u32;
            }
            CMD_PENDING.store(true, Ordering::Release);
        } else {
            // No data available — sleep briefly before retrying.
            anyos_std::process::sleep(5);
        }
    }
}

/// Read a command forwarded by the management thread (non-blocking).
fn recv_command_from_mgmt() -> Option<String> {
    if !CMD_PENDING.load(Ordering::Acquire) {
        return None;
    }

    let text = unsafe {
        let len = CMD_BUF_LEN as usize;
        let s = core::str::from_utf8(&CMD_BUF[..len]).unwrap_or("");
        String::from(s)
    };

    CMD_PENDING.store(false, Ordering::Release);

    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

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

    // Query actual VGA mode for correct dimensions.
    let (mode_w, mode_h, mode_bpp) = inst.handle.vga_get_mode();

    if mode_bpp == 0 {
        // Text mode: mode_w = cols, mode_h = rows.
        if let Some(text_buf) = inst.handle.vga_text_buffer() {
            let cols = if mode_w > 0 { mode_w } else { 80 };
            let rows = if mode_h > 0 { mode_h } else { 25 };
            unsafe {
                shm_write_u32(inst.shm_ptr, 0, cols);
                shm_write_u32(inst.shm_ptr, 4, rows);
                shm_write_u32(inst.shm_ptr, 8, 0); // bpp=0 means text mode

                let payload = inst.shm_ptr.add(SHM_HEADER);
                let src = text_buf.as_ptr() as *const u8;
                let byte_len = (text_buf.len() * 2).min(SHM_SIZE as usize - SHM_HEADER);
                core::ptr::copy_nonoverlapping(src, payload, byte_len);

                // Set dirty flag last (acts as release fence).
                shm_write_u32(inst.shm_ptr, 12, 1);
            }
        }
    } else {
        // Graphics mode.
        if let Some((fb, _, _, _)) = inst.handle.vga_framebuffer() {
            let w = mode_w;
            let h = mode_h;
            let bpp = mode_bpp;
            let bytes_per_pixel = ((bpp as usize) + 7) / 8;
            let byte_len = (w as usize * h as usize * bytes_per_pixel)
                .min(SHM_SIZE as usize - SHM_HEADER)
                .min(fb.len());
            unsafe {
                shm_write_u32(inst.shm_ptr, 0, w);
                shm_write_u32(inst.shm_ptr, 4, h);
                shm_write_u32(inst.shm_ptr, 8, bpp);

                let payload = inst.shm_ptr.add(SHM_HEADER);
                core::ptr::copy_nonoverlapping(fb.as_ptr(), payload, byte_len);

                shm_write_u32(inst.shm_ptr, 12, 1);
            }
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
    disk_image: String,
    /// Additional disk images (disk2=, disk3=, ...).
    extra_disks: Vec<String>,
    iso_image: String,
    /// BIOS type: "corevm" (default) or "seabios".
    bios_type: String,
    /// Boot order: "disk" (default), "cd", or "floppy".
    boot_order: String,
    /// Whether network adapter is enabled.
    net_enabled: bool,
    /// Network mode: "nat" (default) or "bridge".
    net_mode: String,
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
    let mut disk_image = String::new();
    let mut extra_disks: Vec<String> = Vec::new();
    let mut iso_image = String::new();
    let mut bios_type = String::from("corevm");
    let mut boot_order = String::from("disk");
    let mut net_enabled = false;
    let mut net_mode = String::from("nat");
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
        } else if let Some(val) = line.strip_prefix("disk=") {
            disk_image = String::from(val);
        } else if line.starts_with("disk") && line.contains('=') {
            // Parse disk2=, disk3=, ... for additional disks
            if let Some(eq_pos) = line.find('=') {
                let key = &line[..eq_pos];
                let val = &line[eq_pos + 1..];
                if key.len() > 4 && key[4..].chars().all(|c| c.is_ascii_digit()) && !val.is_empty() {
                    extra_disks.push(String::from(val));
                }
            }
        } else if let Some(val) = line.strip_prefix("iso=") {
            iso_image = String::from(val);
        } else if let Some(val) = line.strip_prefix("bios=") {
            bios_type = String::from(val);
        } else if let Some(val) = line.strip_prefix("boot=") {
            boot_order = String::from(val);
        } else if let Some(val) = line.strip_prefix("net_enabled=") {
            net_enabled = val == "1";
        } else if let Some(val) = line.strip_prefix("net_mode=") {
            net_mode = String::from(val);
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
        disk_image,
        extra_disks,
        iso_image,
        bios_type,
        boot_order,
        net_enabled,
        net_mode,
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
        Ok(h) => h,
        Err(e) => {
            send_status(&format!("error 0 failed to create VM: {}", e));
            return;
        }
    };

    // Create vCPU 0.
    if handle.create_vcpu(0) != 0 {
        send_status("error 0 failed to create vCPU");
        anyos_std::println!("[vmd] ERROR: failed to create vCPU 0");
        return;
    }

    // Set up standard PC devices (PIC, PIT, PS/2, CMOS, serial, VGA).
    handle.setup_standard_devices();

    // Set CMOS boot order based on VM configuration.
    // Values: 0=none, 1=floppy, 2=HDD, 3=CD-ROM, 4=network.
    match config.boot_order.as_str() {
        "cd" => {
            handle.cmos_set_boot_order(3, 2); // CD first, HDD second
            anyos_std::println!("[vmd] boot order: CD first, HDD second");
        }
        "floppy" => {
            handle.cmos_set_boot_order(1, 2); // floppy first, HDD second
            anyos_std::println!("[vmd] boot order: floppy first, HDD second");
        }
        _ => {
            handle.cmos_set_boot_order(2, 3); // HDD first, CD second
            anyos_std::println!("[vmd] boot order: HDD first, CD second");
        }
    }

    // Set up AHCI controller: 1 disk + 1 cdrom + extra disks
    let ahci_ports = 2 + config.extra_disks.len().min(6) as u8;
    handle.setup_ahci(ahci_ports);

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
        let mac = if config.mac_mode == "static" && config.mac_address.len() >= 17 {
            parse_mac(&config.mac_address)
        } else {
            DEFAULT_MAC
        };
        handle.setup_e1000(&mac);
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
        boot_order: config.boot_order.clone(),
        exit_count: 0,
    };

    d.vm = Some(inst);

    // Report success with SHM ID BEFORE loading disk/ISO.
    // vmmanager needs the SHM ID promptly; disk/ISO loading can be slow.
    send_status(&format!("created 0 {}", shm_id));
    anyos_std::println!(
        "[vmd] VM '{}' created ({} MiB RAM, shm={})",
        config.name,
        config.ram_mb,
        shm_id
    );

    // Attach disk image as AHCI port 0 (HDD).
    if !config.disk_image.is_empty() {
        let fd = fs::open(&config.disk_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64; // seek to end
            fs::lseek(fd, 0, 0);                    // seek back to start
            if size > 0 {
                if let Some(ref inst) = d.vm {
                    inst.handle.ahci_attach_disk(0, fd as i32, size);
                }
                anyos_std::println!("[vmd] attached disk on AHCI port 0 (fd={}): {} ({} bytes)", fd, config.disk_image, size);
            } else {
                fs::close(fd);
                send_status(&format!("error 0 disk image empty: {}", config.disk_image));
            }
        } else {
            send_status(&format!("error 0 failed to open disk image: {}", config.disk_image));
        }
    }

    // Attach ISO image as AHCI port 1 (CD-ROM).
    if !config.iso_image.is_empty() {
        let fd = fs::open(&config.iso_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                if let Some(ref inst) = d.vm {
                    inst.handle.ahci_attach_cdrom(1, fd as i32, size);
                }
                anyos_std::println!("[vmd] attached ISO on AHCI port 1 (fd={}): {} ({} bytes)", fd, config.iso_image, size);
            } else {
                fs::close(fd);
            }
        }
    }

    // Attach extra disk images as AHCI port 2, 3, ... (HDD).
    for (i, disk_path) in config.extra_disks.iter().enumerate() {
        if disk_path.is_empty() { continue; }
        let port = (2 + i) as u32;
        let fd = fs::open(disk_path, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                if let Some(ref inst) = d.vm {
                    inst.handle.ahci_attach_disk(port, fd as i32, size);
                }
                anyos_std::println!("[vmd] attached disk on AHCI port {} (fd={}): {} ({} bytes)", port, fd, disk_path, size);
            } else {
                fs::close(fd);
            }
        }
    }
}

/// Handle `start` command — load BIOS, set initial CPU state, begin execution.
fn cmd_start() {
    let d = daemon();
    if let Some(ref mut inst) = d.vm {
        if inst.running {
            return;
        }

        // Select and load BIOS based on configured type.
        let is_seabios = inst.bios_type == "seabios";

        if is_seabios {
            let bios_data = read_file(BIOS_PATH_SEABIOS);
            if bios_data.is_empty() {
                send_status("error 0 SeaBIOS not found");
                anyos_std::println!("[vmd] ERROR: SeaBIOS not found at {}", BIOS_PATH_SEABIOS);
                return;
            }

            // Load SeaBIOS at top of 4 GiB (reset vector area).
            // For a 256 KB BIOS: load at 0xFFFC0000 (4G - 256K).
            inst.handle.load_binary(0xFFFC_0000, &bios_data);
            anyos_std::println!(
                "[vmd] SeaBIOS loaded at 0xFFFC0000 ({} bytes)", bios_data.len()
            );

            // Also shadow into low memory for real-mode BIOS calls.
            // SeaBIOS needs the last 64 KB at 0xF0000 for real-mode compatibility.
            let bios_len = bios_data.len();
            if bios_len >= 0x10000 {
                let offset = bios_len - 0x10000;
                inst.handle.load_binary(0xF0000, &bios_data[offset..]);
            }

            // Set up ACPI tables before BIOS starts.
            inst.handle.setup_acpi_tables();

            // Load VGA BIOS as option ROM at 0xC0000.
            let vgabios = read_file(VGABIOS_PATH);
            if !vgabios.is_empty() {
                inst.handle.load_binary(0xC0000, &vgabios);
                anyos_std::println!("[vmd] VGA BIOS loaded at 0xC0000 ({} bytes)", vgabios.len());
            }
        } else {
            // CoreVM BIOS (64 KB at 0xF0000).
            let bios_data = read_file(BIOS_PATH_COREVM);
            if bios_data.is_empty() {
                send_status("error 0 BIOS not found");
                anyos_std::println!(
                    "[vmd] ERROR: CoreVM BIOS not found at {}", BIOS_PATH_COREVM
                );
                return;
            }
            inst.handle.load_binary(0xF0000, &bios_data);
            anyos_std::println!(
                "[vmd] loaded CoreVM BIOS ({} bytes at 0xF0000)", bios_data.len()
            );
        }

        // Set initial CPU state: Real Mode.
        let mut sregs = inst.handle.get_vcpu_sregs(0);
        if is_seabios {
            // SeaBIOS reset vector: CS.base=0xFFFF0000, IP=0xFFF0
            // → first fetch at physical 0xFFFFFFF0 (top of 4 GiB).
            sregs.cs.base = 0xFFFF_0000;
            sregs.cs.selector = 0xF000;
        } else {
            // CoreVM BIOS: CS.base=0xF0000, IP=0xFFF0
            // → first fetch at physical 0xFFFF0 (within 64 KB BIOS area).
            sregs.cs.base = 0xF0000;
            sregs.cs.selector = 0xF000;
        }
        sregs.cs.limit = 0xFFFF;
        sregs.cs.type_ = 0x0B; // Execute/Read, Accessed
        sregs.cs.present = 1;
        sregs.cs.s = 1;
        sregs.ds.base = 0;
        sregs.ds.selector = 0;
        sregs.ds.limit = 0xFFFF;
        sregs.ds.type_ = 0x03; // Read/Write, Accessed
        sregs.ds.present = 1;
        sregs.ds.s = 1;
        sregs.es = sregs.ds;
        sregs.ss = sregs.ds;
        sregs.fs = sregs.ds;
        sregs.gs = sregs.ds;
        inst.handle.set_vcpu_sregs(0, &sregs);

        let mut regs = inst.handle.get_vcpu_regs(0);
        regs.rip = 0xFFF0;
        regs.rflags = 0x2;
        inst.handle.set_vcpu_regs(0, &regs);

        inst.running = true;
        inst.exit_count = 0;
        update_shm_state(inst, STATE_RUNNING);
        send_status("state 0 running");
        anyos_std::println!("[vmd] VM '{}' started (uptime_ms={})",
            inst.name, sys::uptime_ms());
    }
}

/// Handle `stop` command.
fn cmd_stop() {
    let d = daemon();
    if let Some(ref mut inst) = d.vm {
        if !inst.running {
            return;
        }
        inst.running = false;
        update_shm_state(inst, STATE_STOPPED);
        send_status("state 0 stopped");
        anyos_std::println!("[vmd] VM '{}' stopped", inst.name);
    }
}

/// Handle `key <scancode>` command (atomic press+release).
fn cmd_key(scancode: u8) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        if inst.running {
            inst.handle.ps2_key_press(scancode);
            inst.handle.ps2_key_release(scancode);
        }
    }
}

/// Handle `keydown <scancode>` command (key press only).
fn cmd_keydown(scancode: u8) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        if inst.running {
            inst.handle.ps2_key_press(scancode);
        }
    }
}

/// Handle `keyup <scancode>` command (key release only).
fn cmd_keyup(scancode: u8) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        if inst.running {
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
        "keydown" => {
            if parts.len() >= 2 {
                let sc = parse_u32(parts[1]) as u8;
                cmd_keydown(sc);
            }
        }
        "keyup" => {
            if parts.len() >= 2 {
                let sc = parse_u32(parts[1]) as u8;
                cmd_keyup(sc);
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

// ── VM execution (VM-exit run loop) ────────────────────────────────────

/// Run the VM exit loop: enter guest, handle exits, re-enter.
/// Returns true if the VM is still running.
fn run_vm_step() -> bool {
    let d = daemon();
    let inst = match d.vm.as_mut() {
        Some(i) if i.running => i,
        _ => return false,
    };

    let exit = inst.handle.run_vcpu(0);

    // Drain coalesced MMIO writes batched by KVM during this exit.
    // Must happen before dispatching the exit so preceding writes are visible to reads.
    inst.handle.drain_coalesced_mmio();

    match exit {
        VmExitReason::IoIn { port, size } => {
            let mut data = [0u8; 4];
            inst.handle.handle_io_exit(port, 0, size, &mut data[..size as usize]);
        }
        VmExitReason::IoOut { port, size, data } => {
            let bytes = data.to_le_bytes();
            let mut buf = [0u8; 4];
            buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
            inst.handle.handle_io_exit(port, 1, size, &mut buf[..size as usize]);
        }
        VmExitReason::MmioRead { addr, size } => {
            let mut data = [0u8; 8];
            inst.handle.handle_mmio_exit(addr, 0, size, &mut data[..size as usize], 0, 0);
        }
        VmExitReason::MmioWrite { addr, size, data } => {
            let bytes = data.to_le_bytes();
            let mut buf = [0u8; 8];
            buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
            inst.handle.handle_mmio_exit(addr, 1, size, &mut buf[..size as usize], 0, 0);
        }
        VmExitReason::StringIo { port, size, is_out, count } => {
            let direction = if is_out { 1u8 } else { 0u8 };
            let total = (count as usize) * (size as usize);
            let mut buf = alloc::vec![0u8; total.min(4096)];
            inst.handle.handle_string_io_exit(port, direction, size, &mut buf, count);
        }
        VmExitReason::Halted => {
            // HLT pauses until the next interrupt. Sleep briefly so the
            // host loop does not busy-spin.
            anyos_std::process::sleep_us(500);
        }
        VmExitReason::InterruptWindow => {
            // Guest ready for interrupt injection, continue.
        }
        VmExitReason::Shutdown => {
            // Triple fault.
            inst.running = false;
            update_shm_state(inst, STATE_ERROR);
            update_shm_framebuffer(inst);
            send_status("error 0 VM shutdown (triple fault)");
            anyos_std::println!("[vmd] VM shutdown (triple fault)");
            return false;
        }
        VmExitReason::Error => {
            inst.running = false;
            update_shm_state(inst, STATE_ERROR);
            update_shm_framebuffer(inst);
            send_status("error 0 VM execution error");
            anyos_std::println!("[vmd] VM execution error");
            return false;
        }
        VmExitReason::MsrRead { .. } | VmExitReason::MsrWrite { .. } |
        VmExitReason::CpuidExit { .. } | VmExitReason::Debug => {
            // These are handled internally or can be ignored.
        }
    }

    inst.exit_count += 1;

    // Advance PIT timer (~1193 ticks per ms, estimate ~1ms per VM-exit batch).
    inst.handle.pit_advance(1193);

    // Poll device IRQs and route through PIC/IOAPIC.
    let pending_vector = inst.handle.poll_irqs();

    // If there's a pending interrupt, inject it into the vCPU.
    if pending_vector != 0 && pending_vector != u32::MAX {
        inst.handle.inject_interrupt(0, pending_vector as u8);
    }

    // Advance LAPIC timer (~1M TSC ticks per ms).
    inst.handle.lapic_timer_advance(1_000_000);

    // Periodic tasks: drain serial, update framebuffer.
    if inst.exit_count % FB_UPDATE_INTERVAL == 0 {
        // Drain serial output.
        let serial_out = inst.handle.serial_take_output();
        if !serial_out.is_empty() {
            if let Ok(text) = core::str::from_utf8(&serial_out) {
                anyos_std::print!("{}", text);
                send_status(&format!("serial 0 {}", text));
            }
        }

        // Update shared memory framebuffer.
        update_shm_framebuffer(inst);
    }

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

/// Convert a hex ASCII digit to its numeric value (0-15).
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
    anyos_std::println!("[vmd] starting... (VM-exit run loop)");

    // Initialize libcorevm.
    anyos_std::println!("[vmd] loading libcorevm.so...");
    if !libcorevm_client::init() {
        anyos_std::println!("[vmd] ERROR: failed to load libcorevm.so");
        anyos_std::process::exit(1);
    }
    anyos_std::println!("[vmd] libcorevm.so loaded OK");

    // Check for hardware virtualization support.
    if VmHandle::has_hw_support() {
        anyos_std::println!("[vmd] hardware virtualization available");
    } else {
        anyos_std::println!("[vmd] no hardware virtualization, using software emulation");
    }

    // Open IPC pipes (created by vmmanager before spawning us).
    anyos_std::println!("[vmd] opening IPC pipes...");
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

    // Spawn management thread for IPC command reception.
    unsafe { MGMT_CMD_PIPE = cmd_pipe; }
    let _mgmt = Thread::spawn(mgmt_thread_entry, "vmd-mgmt");
    anyos_std::println!("[vmd] management thread spawned");

    // Signal readiness.
    send_status("ready");
    anyos_std::println!("[vmd] sent 'ready' to vmmanager");

    // Main loop: dispatch commands from mgmt thread, run VM, repeat.
    loop {
        // Process all pending commands forwarded by the management thread.
        loop {
            match recv_command_from_mgmt() {
                Some(cmd) => {
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

        // Run VM exit loop step if active.
        let vm_active = run_vm_step();

        if !vm_active {
            anyos_std::process::sleep(10);
        }
    }
}
