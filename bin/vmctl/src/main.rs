//! vmctl — AI-friendly CoreVM command-line controller for anyOS.
//!
//! Allows headless VM management from the command line. Designed for
//! automation: all output is structured text that can be parsed by scripts
//! or AI agents.
//!
//! # Usage
//!
//! ```text
//! vmctl run [--uuid UUID | --ram N] [--disk path] [--iso path]
//!           [--bios corevm|seabios] [--timeout N] [--serial-log path]
//! vmctl list
//! vmctl info <uuid>
//! vmctl create-disk <path> <size_mb>
//! ```
//!
//! # Subcommands
//!
//! - `run`         — Create a VM, run it, stream serial output, dump state on exit
//! - `list`        — List configured VMs from /System/shared/vmmanager/vms/
//! - `info <uuid>` — Show VM configuration details
//! - `create-disk` — Create a blank disk image
//! - `help`        — Show usage information

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::sys;
use libcorevm_client::{VmExitReason, VmHandle};

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────

/// Path to the CoreVM BIOS ROM image (custom BIOS, 64 KB, loaded at 0xF0000).
const BIOS_PATH_COREVM: &str = "/Libraries/libcorevm/bios/bios.bin";

/// Path to SeaBIOS ROM image (256 KB, loaded at 0xC0000).
const BIOS_PATH_SEABIOS: &str = "/System/shared/corevm/bios/seabios.bin";

/// Directory containing per-VM config files.
const VMS_DIR: &str = "/System/shared/vmmanager/vms";

/// Default MAC address for the virtual E1000 NIC.
const DEFAULT_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// Number of VM exits between serial output drains.
const SERIAL_DRAIN_INTERVAL: u64 = 500;

/// Default timeout in seconds (0 = no timeout, run until VM halts/shuts down).
const DEFAULT_TIMEOUT: u32 = 0;

// ── VM configuration ─────────────────────────────────────────────────────

/// VM configuration (parsed from config file or CLI arguments).
struct VmConfig {
    name: String,
    ram_mb: u32,
    disk_image: String,
    iso_image: String,
    bios_type: String,
    net_enabled: bool,
    mac_address: [u8; 6],
}

impl VmConfig {
    fn new() -> Self {
        VmConfig {
            name: String::from("vmctl-vm"),
            ram_mb: 64,
            disk_image: String::new(),
            iso_image: String::new(),
            bios_type: String::from("corevm"),
            net_enabled: false,
            mac_address: DEFAULT_MAC,
        }
    }
}

// ── File I/O helper ──────────────────────────────────────────────────────

/// Read an entire file into a Vec<u8>. Returns empty Vec on failure.
fn read_file(path: &str) -> Vec<u8> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Vec::new();
    }
    let size = fs::lseek(fd, 0, 2);
    if size == 0 || size == u32::MAX {
        fs::close(fd);
        return Vec::new();
    }
    fs::lseek(fd, 0, 0);
    let mut data = alloc::vec![0u8; size as usize];
    let read = fs::read(fd, &mut data);
    fs::close(fd);
    if read == u32::MAX {
        return Vec::new();
    }
    data.truncate(read as usize);
    data
}

// ── Number parsing (no_std) ──────────────────────────────────────────────

fn parse_u32(s: &str) -> u32 {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        }
    }
    val
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0xFF,
    }
}

/// Parse a MAC address string ("XX:XX:XX:XX:XX:XX") into a 6-byte array.
fn parse_mac(s: &str) -> [u8; 6] {
    let mut mac = DEFAULT_MAC;
    let bytes = s.as_bytes();
    let mut idx = 0;
    let mut pos = 0;
    while idx < 6 && pos + 1 < bytes.len() {
        let hi = hex_digit(bytes[pos]);
        let lo = hex_digit(bytes[pos + 1]);
        if hi > 15 || lo > 15 {
            return DEFAULT_MAC;
        }
        mac[idx] = (hi << 4) | lo;
        idx += 1;
        pos += 2;
        if pos < bytes.len() && (bytes[pos] == b':' || bytes[pos] == b'-') {
            pos += 1;
        }
    }
    mac
}

// ── Config file parser ───────────────────────────────────────────────────

/// Read a VM config file by UUID from /System/shared/vmmanager/vms/<uuid>.conf.
fn read_vm_config(uuid: &str) -> Option<VmConfig> {
    let path = format!("{}/{}.conf", VMS_DIR, uuid);
    let data = read_file(&path);
    if data.is_empty() {
        return None;
    }

    let text = core::str::from_utf8(&data).unwrap_or("");
    let mut config = VmConfig::new();

    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(val) = line.strip_prefix("name=") {
            config.name = String::from(val);
        } else if let Some(val) = line.strip_prefix("ram=") {
            config.ram_mb = parse_u32(val);
            if config.ram_mb == 0 {
                config.ram_mb = 64;
            }
        } else if let Some(val) = line.strip_prefix("disk=") {
            config.disk_image = String::from(val);
        } else if let Some(val) = line.strip_prefix("iso=") {
            config.iso_image = String::from(val);
        } else if let Some(val) = line.strip_prefix("bios=") {
            config.bios_type = String::from(val);
        } else if let Some(val) = line.strip_prefix("net_enabled=") {
            config.net_enabled = val == "1";
        } else if let Some(val) = line.strip_prefix("mac_address=") {
            config.mac_address = parse_mac(val);
        }
    }

    if config.name.is_empty() {
        return None;
    }

    Some(config)
}

// ── VM execution ─────────────────────────────────────────────────────────

/// Run a single VM-exit step. Returns (still_running, serial_output).
fn run_vm_step(handle: &VmHandle) -> (bool, Vec<u8>) {
    let exit = handle.run_vcpu(0);

    match exit {
        VmExitReason::IoIn { port, size } => {
            let mut data = [0u8; 4];
            handle.handle_io_exit(port, 0, size, &mut data[..size as usize]);
        }
        VmExitReason::IoOut { port, size, data } => {
            let bytes = data.to_le_bytes();
            let mut buf = [0u8; 4];
            buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
            handle.handle_io_exit(port, 1, size, &mut buf[..size as usize]);
        }
        VmExitReason::MmioRead { addr, size } => {
            let mut data = [0u8; 8];
            handle.handle_mmio_exit(addr, 0, size, &mut data[..size as usize], 0, 0);
        }
        VmExitReason::MmioWrite { addr, size, data } => {
            let bytes = data.to_le_bytes();
            let mut buf = [0u8; 8];
            buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
            handle.handle_mmio_exit(addr, 1, size, &mut buf[..size as usize], 0, 0);
        }
        VmExitReason::Halted => {
            anyos_std::process::sleep_us(500);
        }
        VmExitReason::InterruptWindow => {}
        VmExitReason::Shutdown => {
            anyos_std::println!("[vmctl] VM shutdown (triple fault)");
            return (false, Vec::new());
        }
        VmExitReason::Error => {
            anyos_std::println!("[vmctl] VM execution error");
            return (false, Vec::new());
        }
        _ => {}
    }

    // Drain serial output
    let serial_out = handle.serial_take_output();
    (true, serial_out)
}

/// Print the VGA text buffer as readable text (80x25).
fn dump_vga_text(handle: &VmHandle) {
    if let Some(text_buf) = handle.vga_text_buffer() {
        let cols = 80usize;
        let rows = 25usize;
        let total = cols * rows;

        anyos_std::println!("--- VGA TEXT SCREEN ({}x{}) ---", cols, rows);

        for row in 0..rows {
            let mut line = [b' '; 80];
            let mut last_non_space = 0usize;

            for col in 0..cols {
                let idx = row * cols + col;
                if idx < text_buf.len() && idx < total {
                    let cell = text_buf[idx];
                    let ch = (cell & 0xFF) as u8;
                    if ch >= 0x20 && ch < 0x7F {
                        line[col] = ch;
                        if ch != b' ' {
                            last_non_space = col + 1;
                        }
                    }
                }
            }

            if last_non_space > 0 {
                let s = core::str::from_utf8(&line[..last_non_space]).unwrap_or("");
                anyos_std::println!("{}", s);
            } else {
                anyos_std::println!("");
            }
        }
        anyos_std::println!("--- END SCREEN ---");
    } else {
        anyos_std::println!("[vmctl] No VGA text buffer available");
    }
}

/// Print CPU registers.
fn dump_regs(handle: &VmHandle) {
    let regs = handle.get_vcpu_regs(0);
    let sregs = handle.get_vcpu_sregs(0);

    anyos_std::println!("--- CPU REGISTERS ---");
    anyos_std::println!("RAX={:016X}  RBX={:016X}  RCX={:016X}  RDX={:016X}",
        regs.rax, regs.rbx, regs.rcx, regs.rdx);
    anyos_std::println!("RSI={:016X}  RDI={:016X}  RBP={:016X}  RSP={:016X}",
        regs.rsi, regs.rdi, regs.rbp, regs.rsp);
    anyos_std::println!("R8 ={:016X}  R9 ={:016X}  R10={:016X}  R11={:016X}",
        regs.r8, regs.r9, regs.r10, regs.r11);
    anyos_std::println!("R12={:016X}  R13={:016X}  R14={:016X}  R15={:016X}",
        regs.r12, regs.r13, regs.r14, regs.r15);
    anyos_std::println!("RIP={:016X}  RFLAGS={:016X}", regs.rip, regs.rflags);
    anyos_std::println!("CR0={:016X}  CR2={:016X}  CR3={:016X}  CR4={:016X}  EFER={:016X}",
        sregs.cr0, sregs.cr2, sregs.cr3, sregs.cr4, sregs.efer);
    anyos_std::println!("CS: sel={:04X} base={:016X} limit={:08X}  DS: sel={:04X} base={:016X}",
        sregs.cs.selector, sregs.cs.base, sregs.cs.limit, sregs.ds.selector, sregs.ds.base);
    anyos_std::println!("SS: sel={:04X} base={:016X}  ES: sel={:04X}  FS: sel={:04X}  GS: sel={:04X}",
        sregs.ss.selector, sregs.ss.base, sregs.es.selector, sregs.fs.selector, sregs.gs.selector);
    anyos_std::println!("--- END REGISTERS ---");
}

/// Send a string as PS/2 keyboard scancodes (ASCII only).
fn type_string(handle: &VmHandle, text: &str) {
    for &b in text.as_bytes() {
        let (scancode, shift) = ascii_to_scancode(b);
        if scancode == 0 {
            continue;
        }
        if shift {
            handle.ps2_key_press(0x2A); // Left Shift press
        }
        handle.ps2_key_press(scancode);
        handle.ps2_key_release(scancode);
        if shift {
            handle.ps2_key_release(0x2A); // Left Shift release
        }
    }
}

/// Map ASCII character to PS/2 scancode set 1. Returns (scancode, needs_shift).
fn ascii_to_scancode(ch: u8) -> (u8, bool) {
    match ch {
        b'a'..=b'z' => {
            let sc = match ch {
                b'a' => 0x1E, b'b' => 0x30, b'c' => 0x2E, b'd' => 0x20,
                b'e' => 0x12, b'f' => 0x21, b'g' => 0x22, b'h' => 0x23,
                b'i' => 0x17, b'j' => 0x24, b'k' => 0x25, b'l' => 0x26,
                b'm' => 0x32, b'n' => 0x31, b'o' => 0x18, b'p' => 0x19,
                b'q' => 0x10, b'r' => 0x13, b's' => 0x1F, b't' => 0x14,
                b'u' => 0x16, b'v' => 0x2F, b'w' => 0x11, b'x' => 0x2D,
                b'y' => 0x15, b'z' => 0x2C, _ => 0,
            };
            (sc, false)
        }
        b'A'..=b'Z' => {
            let (sc, _) = ascii_to_scancode(ch.to_ascii_lowercase());
            (sc, true)
        }
        b'0' => (0x0B, false),
        b'1' => (0x02, false), b'2' => (0x03, false), b'3' => (0x04, false),
        b'4' => (0x05, false), b'5' => (0x06, false), b'6' => (0x07, false),
        b'7' => (0x08, false), b'8' => (0x09, false), b'9' => (0x0A, false),
        b' ' => (0x39, false),
        b'\n' => (0x1C, false),  // Enter
        b'\r' => (0x1C, false),
        b'\t' => (0x0F, false),  // Tab
        b'-' => (0x0C, false),
        b'=' => (0x0D, false),
        b'[' => (0x1A, false),
        b']' => (0x1B, false),
        b';' => (0x27, false),
        b'\'' => (0x28, false),
        b'`' => (0x29, false),
        b'\\' => (0x2B, false),
        b',' => (0x33, false),
        b'.' => (0x34, false),
        b'/' => (0x35, false),
        // Shifted symbols
        b'!' => (0x02, true),  b'@' => (0x03, true),  b'#' => (0x04, true),
        b'$' => (0x05, true),  b'%' => (0x06, true),  b'^' => (0x07, true),
        b'&' => (0x08, true),  b'*' => (0x09, true),  b'(' => (0x0A, true),
        b')' => (0x0B, true),  b'_' => (0x0C, true),  b'+' => (0x0D, true),
        b'{' => (0x1A, true),  b'}' => (0x1B, true),  b':' => (0x27, true),
        b'"' => (0x28, true),  b'~' => (0x29, true),  b'|' => (0x2B, true),
        b'<' => (0x33, true),  b'>' => (0x34, true),  b'?' => (0x35, true),
        _ => (0, false),
    }
}

// ── Subcommand: run ──────────────────────────────────────────────────────

/// Run a VM until timeout or shutdown, streaming serial output to stdout.
fn cmd_run(config: VmConfig, timeout_secs: u32, show_screen: bool, show_regs: bool) {
    anyos_std::println!("[vmctl] Initializing libcorevm...");
    if !libcorevm_client::init() {
        anyos_std::println!("[vmctl] ERROR: Failed to load libcorevm.so");
        anyos_std::process::exit(1);
    }

    if VmHandle::has_hw_support() {
        anyos_std::println!("[vmctl] Hardware virtualization available");
    } else {
        anyos_std::println!("[vmctl] Using software emulation");
    }

    // Create VM
    anyos_std::println!("[vmctl] Creating VM '{}' ({} MiB RAM, bios={})...",
        config.name, config.ram_mb, config.bios_type);

    let handle = match VmHandle::new(config.ram_mb) {
        Ok(h) => h,
        Err(e) => {
            anyos_std::println!("[vmctl] ERROR: Failed to create VM: {}", e);
            anyos_std::process::exit(1);
        }
    };

    handle.create_vcpu(0);
    handle.setup_standard_devices();
    handle.setup_ahci(2);

    // Set up network if enabled
    if config.net_enabled {
        handle.setup_e1000(&config.mac_address);
        anyos_std::println!("[vmctl] E1000 NIC enabled");
    }

    // Attach disk image
    if !config.disk_image.is_empty() {
        let fd = fs::open(&config.disk_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_disk(0, fd as i32, size);
                anyos_std::println!("[vmctl] Disk: {} ({} bytes)", config.disk_image, size);
            } else {
                fs::close(fd);
                anyos_std::println!("[vmctl] WARNING: Disk image empty: {}", config.disk_image);
            }
        } else {
            anyos_std::println!("[vmctl] ERROR: Cannot open disk: {}", config.disk_image);
        }
    }

    // Attach ISO image
    if !config.iso_image.is_empty() {
        let fd = fs::open(&config.iso_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_cdrom(1, fd as i32, size);
                anyos_std::println!("[vmctl] ISO: {} ({} bytes)", config.iso_image, size);
            } else {
                fs::close(fd);
            }
        } else {
            anyos_std::println!("[vmctl] WARNING: Cannot open ISO: {}", config.iso_image);
        }
    }

    // Load BIOS
    let is_seabios = config.bios_type == "seabios";
    if is_seabios {
        let bios_data = read_file(BIOS_PATH_SEABIOS);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: SeaBIOS not found at {}", BIOS_PATH_SEABIOS);
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xC0000, &bios_data);
        handle.load_binary(0xFFFC_0000, &bios_data);
        anyos_std::println!("[vmctl] SeaBIOS loaded ({} bytes)", bios_data.len());
    } else {
        let bios_data = read_file(BIOS_PATH_COREVM);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: CoreVM BIOS not found at {}", BIOS_PATH_COREVM);
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xF0000, &bios_data);
        anyos_std::println!("[vmctl] CoreVM BIOS loaded ({} bytes)", bios_data.len());
    }

    // Set initial CPU state: Real Mode, CS:IP = F000:FFF0
    let mut sregs = handle.get_vcpu_sregs(0);
    sregs.cs.base = 0xF0000;
    sregs.cs.selector = 0xF000;
    sregs.cs.limit = 0xFFFF;
    sregs.cs.type_ = 0x0B;
    sregs.cs.present = 1;
    sregs.cs.s = 1;
    sregs.ds.base = 0;
    sregs.ds.selector = 0;
    sregs.ds.limit = 0xFFFF;
    sregs.ds.type_ = 0x03;
    sregs.ds.present = 1;
    sregs.ds.s = 1;
    sregs.es = sregs.ds;
    sregs.ss = sregs.ds;
    sregs.fs = sregs.ds;
    sregs.gs = sregs.ds;
    handle.set_vcpu_sregs(0, &sregs);

    let mut regs = handle.get_vcpu_regs(0);
    regs.rip = 0xFFF0;
    regs.rflags = 0x2;
    handle.set_vcpu_regs(0, &regs);

    // Run the VM
    let start_ms = sys::uptime_ms();
    let timeout_ms = if timeout_secs > 0 { timeout_secs * 1000 } else { 0u32 };
    let mut exit_count: u64 = 0;
    let mut total_serial = Vec::new();
    let mut exit_reason = "running";

    anyos_std::println!("[vmctl] VM started (timeout={}s)", timeout_secs);
    anyos_std::println!("--- SERIAL OUTPUT ---");

    loop {
        let (running, serial_out) = run_vm_step(&handle);

        if !serial_out.is_empty() {
            // Print serial output live
            if let Ok(text) = core::str::from_utf8(&serial_out) {
                anyos_std::print!("{}", text);
            }
            total_serial.extend_from_slice(&serial_out);
        }

        if !running {
            exit_reason = "shutdown";
            break;
        }

        exit_count += 1;

        // Check timeout
        if timeout_ms > 0 {
            let elapsed = sys::uptime_ms().wrapping_sub(start_ms);
            if elapsed >= timeout_ms {
                exit_reason = "timeout";
                break;
            }
        }
    }

    let elapsed_ms = sys::uptime_ms().wrapping_sub(start_ms);
    anyos_std::println!("");
    anyos_std::println!("--- END SERIAL OUTPUT ---");
    anyos_std::println!("");

    // Dump final state
    anyos_std::println!("--- VM EXIT SUMMARY ---");
    anyos_std::println!("exit_reason: {}", exit_reason);
    anyos_std::println!("runtime_ms: {}", elapsed_ms);
    anyos_std::println!("exit_count: {}", exit_count);
    anyos_std::println!("serial_bytes: {}", total_serial.len());
    anyos_std::println!("--- END SUMMARY ---");
    anyos_std::println!("");

    if show_screen {
        dump_vga_text(&handle);
        anyos_std::println!("");
    }

    if show_regs {
        dump_regs(&handle);
        anyos_std::println!("");
    }
}

// ── Subcommand: list ─────────────────────────────────────────────────────

fn cmd_list() {
    let mut buf = [0u8; 64 * 128]; // max 128 entries
    let count = fs::readdir(VMS_DIR, &mut buf);

    if count == 0 || count == u32::MAX {
        anyos_std::println!("[vmctl] No VMs found in {}", VMS_DIR);
        return;
    }

    anyos_std::println!("--- VM LIST ---");
    anyos_std::println!("{:<36}  {:<20}  {:>6}  {}", "UUID", "NAME", "RAM", "DISK");
    anyos_std::println!("--------------------------------------------------------------------------------------");

    let entry_size = 64usize;
    for i in 0..count as usize {
        let base = i * entry_size;
        if base + entry_size > buf.len() {
            break;
        }

        // Entry format: [type:u8, name_len:u8, flags:u8, pad:u8, size:u32, name:56bytes]
        let name_len = buf[base + 1] as usize;
        if name_len == 0 || name_len > 56 {
            continue;
        }
        let name_bytes = &buf[base + 8..base + 8 + name_len];
        let filename = core::str::from_utf8(name_bytes).unwrap_or("");

        // Only process .conf files
        if !filename.ends_with(".conf") {
            continue;
        }

        // Extract UUID (filename without .conf)
        let uuid = &filename[..filename.len() - 5];

        // Read config to get name and RAM
        if let Some(config) = read_vm_config(uuid) {
            anyos_std::println!("{:<36}  {:<20}  {:>4}MB  {}",
                uuid, config.name, config.ram_mb,
                if config.disk_image.is_empty() { "(none)" } else { &config.disk_image });
        } else {
            anyos_std::println!("{:<36}  (invalid config)", uuid);
        }
    }

    anyos_std::println!("--- END LIST ---");
}

// ── Subcommand: info ─────────────────────────────────────────────────────

fn cmd_info(uuid: &str) {
    match read_vm_config(uuid) {
        Some(config) => {
            anyos_std::println!("--- VM INFO ---");
            anyos_std::println!("uuid: {}", uuid);
            anyos_std::println!("name: {}", config.name);
            anyos_std::println!("ram_mb: {}", config.ram_mb);
            anyos_std::println!("bios: {}", config.bios_type);
            anyos_std::println!("disk: {}", if config.disk_image.is_empty() { "(none)" } else { &config.disk_image });
            anyos_std::println!("iso: {}", if config.iso_image.is_empty() { "(none)" } else { &config.iso_image });
            anyos_std::println!("net_enabled: {}", config.net_enabled);
            anyos_std::println!("mac: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                config.mac_address[0], config.mac_address[1], config.mac_address[2],
                config.mac_address[3], config.mac_address[4], config.mac_address[5]);
            anyos_std::println!("--- END INFO ---");
        }
        None => {
            anyos_std::println!("[vmctl] ERROR: VM config not found for UUID '{}'", uuid);
            anyos_std::process::exit(1);
        }
    }
}

// ── Subcommand: create-disk ──────────────────────────────────────────────

fn cmd_create_disk(path: &str, size_mb: u32) {
    if size_mb == 0 {
        anyos_std::println!("[vmctl] ERROR: Disk size must be > 0 MB");
        anyos_std::process::exit(1);
    }

    let fd = fs::open(path, 1); // O_CREAT | O_WRONLY
    if fd == u32::MAX {
        anyos_std::println!("[vmctl] ERROR: Cannot create file: {}", path);
        anyos_std::process::exit(1);
    }

    // Write zeros in 64 KB chunks
    let chunk_size = 65536u32;
    let total_bytes = size_mb as u64 * 1024 * 1024;
    let mut written: u64 = 0;
    let zeros = alloc::vec![0u8; chunk_size as usize];

    while written < total_bytes {
        let remaining = total_bytes - written;
        let to_write = if remaining > chunk_size as u64 { chunk_size } else { remaining as u32 };
        let n = fs::write(fd, &zeros[..to_write as usize]);
        if n == u32::MAX {
            fs::close(fd);
            anyos_std::println!("[vmctl] ERROR: Write failed at offset {}", written);
            anyos_std::process::exit(1);
        }
        written += n as u64;
    }

    fs::close(fd);
    anyos_std::println!("[vmctl] Created disk: {} ({} MB, {} bytes)", path, size_mb, total_bytes);
}

// ── Subcommand: serial ───────────────────────────────────────────────────

/// Interactive serial console mode: run VM and forward stdin/stdout via serial.
fn cmd_serial(config: VmConfig) {
    anyos_std::println!("[vmctl] Starting interactive serial console...");
    anyos_std::println!("[vmctl] (Serial I/O connected to stdin/stdout)");
    anyos_std::println!("");

    if !libcorevm_client::init() {
        anyos_std::println!("[vmctl] ERROR: Failed to load libcorevm.so");
        anyos_std::process::exit(1);
    }

    let handle = match VmHandle::new(config.ram_mb) {
        Ok(h) => h,
        Err(e) => {
            anyos_std::println!("[vmctl] ERROR: Failed to create VM: {}", e);
            anyos_std::process::exit(1);
        }
    };

    handle.create_vcpu(0);
    handle.setup_standard_devices();
    handle.setup_ahci(2);

    // Attach storage
    if !config.disk_image.is_empty() {
        let fd = fs::open(&config.disk_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_disk(0, fd as i32, size);
            } else {
                fs::close(fd);
            }
        }
    }
    if !config.iso_image.is_empty() {
        let fd = fs::open(&config.iso_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_cdrom(1, fd as i32, size);
            } else {
                fs::close(fd);
            }
        }
    }

    // Load BIOS
    let is_seabios = config.bios_type == "seabios";
    if is_seabios {
        let bios_data = read_file(BIOS_PATH_SEABIOS);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: SeaBIOS not found");
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xC0000, &bios_data);
        handle.load_binary(0xFFFC_0000, &bios_data);
    } else {
        let bios_data = read_file(BIOS_PATH_COREVM);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: CoreVM BIOS not found");
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xF0000, &bios_data);
    }

    // Set initial CPU state
    let mut sregs = handle.get_vcpu_sregs(0);
    sregs.cs.base = 0xF0000;
    sregs.cs.selector = 0xF000;
    sregs.cs.limit = 0xFFFF;
    sregs.cs.type_ = 0x0B;
    sregs.cs.present = 1;
    sregs.cs.s = 1;
    sregs.ds.base = 0;
    sregs.ds.selector = 0;
    sregs.ds.limit = 0xFFFF;
    sregs.ds.type_ = 0x03;
    sregs.ds.present = 1;
    sregs.ds.s = 1;
    sregs.es = sregs.ds;
    sregs.ss = sregs.ds;
    sregs.fs = sregs.ds;
    sregs.gs = sregs.ds;
    handle.set_vcpu_sregs(0, &sregs);

    let mut regs = handle.get_vcpu_regs(0);
    regs.rip = 0xFFF0;
    regs.rflags = 0x2;
    handle.set_vcpu_regs(0, &regs);

    // Run with serial I/O
    let mut exit_count: u64 = 0;
    loop {
        let (running, serial_out) = run_vm_step(&handle);
        if !serial_out.is_empty() {
            if let Ok(text) = core::str::from_utf8(&serial_out) {
                anyos_std::print!("{}", text);
            }
        }
        if !running {
            break;
        }

        exit_count += 1;

        // Periodically check for stdin input and forward to serial
        if exit_count % SERIAL_DRAIN_INTERVAL == 0 {
            let mut in_buf = [0u8; 256];
            let n = fs::read_nonblock(0, &mut in_buf);
            if n > 0 {
                handle.serial_send_input(&in_buf[..n as usize]);
            }
        }
    }

    anyos_std::println!("\n[vmctl] VM exited");
}

// ── Help ─────────────────────────────────────────────────────────────────

fn print_help() {
    anyos_std::println!("vmctl — AI-friendly CoreVM CLI controller");
    anyos_std::println!("");
    anyos_std::println!("USAGE:");
    anyos_std::println!("  vmctl <command> [options]");
    anyos_std::println!("");
    anyos_std::println!("COMMANDS:");
    anyos_std::println!("  run           Create and run a VM, stream serial output");
    anyos_std::println!("  serial        Interactive serial console (stdin/stdout)");
    anyos_std::println!("  list          List configured VMs");
    anyos_std::println!("  info <uuid>   Show VM configuration");
    anyos_std::println!("  create-disk   Create a blank disk image");
    anyos_std::println!("  help          Show this help");
    anyos_std::println!("");
    anyos_std::println!("RUN OPTIONS:");
    anyos_std::println!("  -u <uuid>     Load config from existing VM by UUID");
    anyos_std::println!("  -r <mb>       RAM size in MiB (default: 64)");
    anyos_std::println!("  -d <path>     Disk image path");
    anyos_std::println!("  -i <path>     ISO/CD-ROM image path");
    anyos_std::println!("  -b <type>     BIOS type: corevm (default) or seabios");
    anyos_std::println!("  -t <secs>     Timeout in seconds (0 = no timeout)");
    anyos_std::println!("  -s            Show VGA text screen on exit");
    anyos_std::println!("  -g            Show CPU registers on exit");
    anyos_std::println!("  -n            Enable network (E1000 NIC)");
    anyos_std::println!("  -k <text>     Type text via PS/2 keyboard after boot");
    anyos_std::println!("  -w <ms>       Wait N ms before typing -k text");
    anyos_std::println!("");
    anyos_std::println!("EXAMPLES:");
    anyos_std::println!("  vmctl run -r 128 -d /data/disk.img -t 30 -s -g");
    anyos_std::println!("  vmctl run -u 01234567890abcdef -t 60 -s");
    anyos_std::println!("  vmctl run -r 64 -i /data/boot.iso -b seabios -t 10 -s");
    anyos_std::println!("  vmctl serial -r 64 -d /data/disk.img");
    anyos_std::println!("  vmctl list");
    anyos_std::println!("  vmctl info 01234567890abcdef");
    anyos_std::println!("  vmctl create-disk /data/blank.img 256");
    anyos_std::println!("");
    anyos_std::println!("AI USAGE:");
    anyos_std::println!("  All output is structured text with delimiters:");
    anyos_std::println!("  --- SERIAL OUTPUT --- / --- END SERIAL OUTPUT ---");
    anyos_std::println!("  --- VGA TEXT SCREEN --- / --- END SCREEN ---");
    anyos_std::println!("  --- CPU REGISTERS --- / --- END REGISTERS ---");
    anyos_std::println!("  --- VM EXIT SUMMARY --- / --- END SUMMARY ---");
}

// ── Main ─────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"urdibtkw");

    let command = args.first_or("help");

    match command {
        "run" | "serial" => {
            let is_serial = command == "serial";

            // Build config from either UUID or CLI flags
            let mut config = if let Some(uuid) = args.opt(b'u') {
                match read_vm_config(uuid) {
                    Some(c) => c,
                    None => {
                        anyos_std::println!("[vmctl] ERROR: VM config not found for UUID '{}'", uuid);
                        anyos_std::process::exit(1);
                    }
                }
            } else {
                VmConfig::new()
            };

            // CLI flags override config file
            if let Some(ram) = args.opt(b'r') {
                let r = parse_u32(ram);
                if r > 0 {
                    config.ram_mb = r;
                }
            }
            if let Some(disk) = args.opt(b'd') {
                config.disk_image = String::from(disk);
            }
            if let Some(iso) = args.opt(b'i') {
                config.iso_image = String::from(iso);
            }
            if let Some(bios) = args.opt(b'b') {
                config.bios_type = String::from(bios);
            }
            if args.has(b'n') {
                config.net_enabled = true;
            }

            if is_serial {
                cmd_serial(config);
            } else {
                let timeout = args.opt_u32(b't', DEFAULT_TIMEOUT);
                let show_screen = args.has(b's');
                let show_regs = args.has(b'g');
                // -k and -w are for typing after boot
                let type_text = args.opt(b'k');
                let type_delay_ms = args.opt_u32(b'w', 3000);

                if type_text.is_some() {
                    // Run VM with keyboard input after delay
                    run_with_typing(config, timeout, show_screen, show_regs,
                                    type_text.unwrap(), type_delay_ms);
                } else {
                    cmd_run(config, timeout, show_screen, show_regs);
                }
            }
        }
        "list" => cmd_list(),
        "info" => {
            if let Some(uuid) = args.pos(1) {
                cmd_info(uuid);
            } else {
                anyos_std::println!("[vmctl] ERROR: Missing UUID. Usage: vmctl info <uuid>");
            }
        }
        "create-disk" => {
            let path = args.pos(1);
            let size = args.pos(2);
            if let (Some(p), Some(s)) = (path, size) {
                cmd_create_disk(p, parse_u32(s));
            } else {
                anyos_std::println!("[vmctl] ERROR: Usage: vmctl create-disk <path> <size_mb>");
            }
        }
        "help" | "--help" | "-h" => print_help(),
        _ => {
            anyos_std::println!("[vmctl] Unknown command: {}", command);
            anyos_std::println!("Run 'vmctl help' for usage information.");
        }
    }
}

/// Run a VM, type text after a delay, then continue until timeout/shutdown.
fn run_with_typing(config: VmConfig, timeout_secs: u32, show_screen: bool,
                   show_regs: bool, text: &str, delay_ms: u32) {
    anyos_std::println!("[vmctl] Initializing libcorevm...");
    if !libcorevm_client::init() {
        anyos_std::println!("[vmctl] ERROR: Failed to load libcorevm.so");
        anyos_std::process::exit(1);
    }

    if VmHandle::has_hw_support() {
        anyos_std::println!("[vmctl] Hardware virtualization available");
    } else {
        anyos_std::println!("[vmctl] Using software emulation");
    }

    anyos_std::println!("[vmctl] Creating VM '{}' ({} MiB RAM, bios={})...",
        config.name, config.ram_mb, config.bios_type);

    let handle = match VmHandle::new(config.ram_mb) {
        Ok(h) => h,
        Err(e) => {
            anyos_std::println!("[vmctl] ERROR: Failed to create VM: {}", e);
            anyos_std::process::exit(1);
        }
    };

    handle.create_vcpu(0);
    handle.setup_standard_devices();
    handle.setup_ahci(2);

    if config.net_enabled {
        handle.setup_e1000(&config.mac_address);
    }

    // Attach storage
    if !config.disk_image.is_empty() {
        let fd = fs::open(&config.disk_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_disk(0, fd as i32, size);
                anyos_std::println!("[vmctl] Disk: {} ({} bytes)", config.disk_image, size);
            } else {
                fs::close(fd);
            }
        }
    }
    if !config.iso_image.is_empty() {
        let fd = fs::open(&config.iso_image, 0);
        if fd != u32::MAX {
            let size = fs::lseek(fd, 0, 2) as u64;
            fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_cdrom(1, fd as i32, size);
                anyos_std::println!("[vmctl] ISO: {} ({} bytes)", config.iso_image, size);
            } else {
                fs::close(fd);
            }
        }
    }

    // Load BIOS
    let is_seabios = config.bios_type == "seabios";
    if is_seabios {
        let bios_data = read_file(BIOS_PATH_SEABIOS);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: SeaBIOS not found");
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xC0000, &bios_data);
        handle.load_binary(0xFFFC_0000, &bios_data);
    } else {
        let bios_data = read_file(BIOS_PATH_COREVM);
        if bios_data.is_empty() {
            anyos_std::println!("[vmctl] ERROR: CoreVM BIOS not found");
            anyos_std::process::exit(1);
        }
        handle.load_binary(0xF0000, &bios_data);
    }

    // Set initial CPU state
    let mut sregs = handle.get_vcpu_sregs(0);
    sregs.cs.base = 0xF0000;
    sregs.cs.selector = 0xF000;
    sregs.cs.limit = 0xFFFF;
    sregs.cs.type_ = 0x0B;
    sregs.cs.present = 1;
    sregs.cs.s = 1;
    sregs.ds.base = 0;
    sregs.ds.selector = 0;
    sregs.ds.limit = 0xFFFF;
    sregs.ds.type_ = 0x03;
    sregs.ds.present = 1;
    sregs.ds.s = 1;
    sregs.es = sregs.ds;
    sregs.ss = sregs.ds;
    sregs.fs = sregs.ds;
    sregs.gs = sregs.ds;
    handle.set_vcpu_sregs(0, &sregs);

    let mut regs = handle.get_vcpu_regs(0);
    regs.rip = 0xFFF0;
    regs.rflags = 0x2;
    handle.set_vcpu_regs(0, &regs);

    // Run VM
    let start_ms = sys::uptime_ms();
    let timeout_ms = if timeout_secs > 0 { timeout_secs * 1000 } else { 0u32 };
    let mut exit_count: u64 = 0;
    let mut total_serial = Vec::new();
    let mut exit_reason = "running";
    let mut typed = false;

    anyos_std::println!("[vmctl] VM started (timeout={}s, type after {}ms: \"{}\")",
        timeout_secs, delay_ms, text);
    anyos_std::println!("--- SERIAL OUTPUT ---");

    loop {
        let (running, serial_out) = run_vm_step(&handle);

        if !serial_out.is_empty() {
            if let Ok(t) = core::str::from_utf8(&serial_out) {
                anyos_std::print!("{}", t);
            }
            total_serial.extend_from_slice(&serial_out);
        }

        if !running {
            exit_reason = "shutdown";
            break;
        }

        exit_count += 1;
        let elapsed = sys::uptime_ms().wrapping_sub(start_ms);

        // Type text after delay
        if !typed && elapsed >= delay_ms {
            anyos_std::println!("\n[vmctl] Typing: \"{}\"", text);
            type_string(&handle, text);
            typed = true;
        }

        // Check timeout
        if timeout_ms > 0 && elapsed >= timeout_ms {
            exit_reason = "timeout";
            break;
        }
    }

    let elapsed_ms = sys::uptime_ms().wrapping_sub(start_ms);
    anyos_std::println!("");
    anyos_std::println!("--- END SERIAL OUTPUT ---");
    anyos_std::println!("");

    anyos_std::println!("--- VM EXIT SUMMARY ---");
    anyos_std::println!("exit_reason: {}", exit_reason);
    anyos_std::println!("runtime_ms: {}", elapsed_ms);
    anyos_std::println!("exit_count: {}", exit_count);
    anyos_std::println!("serial_bytes: {}", total_serial.len());
    anyos_std::println!("typed: {}", typed);
    anyos_std::println!("--- END SUMMARY ---");
    anyos_std::println!("");

    if show_screen {
        dump_vga_text(&handle);
        anyos_std::println!("");
    }

    if show_regs {
        dump_regs(&handle);
        anyos_std::println!("");
    }
}
