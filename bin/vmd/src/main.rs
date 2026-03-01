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

/// Path to the SeaBIOS ROM image.
const SEABIOS_PATH: &str = "/System/shared/corevm/bios/seabios.bin";

/// Instructions to run per execution batch before checking IPC.
/// Higher = more throughput, lower = more responsive to commands.
const BATCH_SIZE: u64 = 5_000_000;

/// PIT ticks to advance per batch (approximate real PIT rate).
const PIT_TICKS_PER_BATCH: u32 = 4;

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

// ── Command handlers ───────────────────────────────────────────────────

/// Handle `create <name> <ram_mb>` command.
fn cmd_create(name: &str, ram_mb: u32) {
    let d = daemon();

    // Destroy any existing VM.
    if let Some(ref inst) = d.vm {
        update_shm_state(inst, STATE_STOPPED);
        if inst.shm_id != 0 {
            ipc::shm_destroy(inst.shm_id);
        }
    }
    d.vm = None;

    // Create VM.
    let handle = match VmHandle::new(ram_mb) {
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

    let inst = VmInstance {
        handle,
        shm_id,
        shm_ptr,
        running: false,
        name: String::from(name),
    };

    d.vm = Some(inst);

    // Report success with SHM ID.
    send_status(&format!("created 0 {}", shm_id));
    anyos_std::println!("[vmd] VM '{}' created ({} MiB RAM, shm={})", name, ram_mb, shm_id);
}

/// Handle `disk <path>` command — attach a disk image.
fn cmd_disk(path: &str) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        let data = read_file(path);
        if !data.is_empty() {
            inst.handle.ide_attach_disk(&data);
            anyos_std::println!("[vmd] attached disk: {} ({} bytes)", path, data.len());
        } else {
            send_status(&format!("error 0 failed to read disk image: {}", path));
        }
    }
}

/// Handle `iso <path>` command — load ISO into high memory.
fn cmd_iso(path: &str) {
    let d = daemon();
    if let Some(ref inst) = d.vm {
        let data = read_file(path);
        if !data.is_empty() {
            inst.handle.load_binary(0x10_0000, &data);
            anyos_std::println!("[vmd] loaded ISO: {} ({} bytes)", path, data.len());
        }
    }
}

/// Handle `start` command — load BIOS and begin execution.
fn cmd_start() {
    let d = daemon();
    if let Some(ref mut inst) = d.vm {
        if inst.running {
            return;
        }

        // Load SeaBIOS.
        let bios_data = read_file(SEABIOS_PATH);
        if !bios_data.is_empty() {
            let load_addr = if bios_data.len() <= 0x10000 {
                0xF0000u64
            } else {
                (0x10_0000u64).wrapping_sub(bios_data.len() as u64)
            };
            inst.handle.load_binary(load_addr, &bios_data);
            inst.handle.set_rip(0xFFF0);
            anyos_std::println!("[vmd] loaded SeaBIOS ({} bytes at 0x{:X})", bios_data.len(), load_addr);
        } else {
            send_status("error 0 SeaBIOS not found");
            anyos_std::println!("[vmd] ERROR: SeaBIOS not found at {}", SEABIOS_PATH);
            return;
        }

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
            if parts.len() >= 3 {
                let ram_mb = parse_u32(parts[2]);
                cmd_create(parts[1], ram_mb);
            }
        }
        "disk" => {
            if parts.len() >= 2 {
                cmd_disk(parts[1]);
            }
        }
        "iso" => {
            if parts.len() >= 2 {
                cmd_iso(parts[1]);
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
            inst.running = false;
            update_shm_state(inst, STATE_HALTED);
            update_shm_framebuffer(inst);
            send_status("state 0 halted");
            anyos_std::println!("[vmd] VM halted ({} instructions)",
                inst.handle.instruction_count());
            return false;
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
            // Print locally too.
            anyos_std::print!("{}", text);
            // Send to vmmanager.
            send_status(&format!("serial 0 {}", text));
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

        // If no VM is running, sleep briefly to avoid busy-spinning.
        if !vm_active {
            anyos_std::process::sleep(10);
        }
    }
}
