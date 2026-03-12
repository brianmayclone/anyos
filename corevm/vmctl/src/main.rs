//! vmctl — Headless CoreVM CLI controller for Linux/KVM.
//!
//! Usage:
//!   vmctl run -r <mb> -i <iso> -b seabios -t <secs> -s -g
//!   vmctl run -r 512 -i /path/to/linux.iso -b seabios -t 60 -s -g

use std::time::{Duration, Instant};
use std::{env, thread};

use libcorevm::ffi::*;
use libcorevm::backend::{VcpuRegs, VcpuSregs, SegmentReg, DescriptorTable};

// ── BIOS search paths ──

fn find_bios(name: &str) -> Option<String> {
    let exe_dir = env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let mut paths = Vec::new();
    if let Some(d) = &exe_dir {
        paths.push(d.join("bios"));
        paths.push(d.to_path_buf());
    }
    paths.push(manifest_dir.join("../vmmanager/assets/bios"));
    if let Some(root) = manifest_dir.parent().and_then(|p| p.parent()) {
        paths.push(root.join("libs/libcorevm/bios"));
        paths.push(root.join("build"));
    }
    paths.push(std::path::PathBuf::from("/usr/share/seabios"));
    paths.push(std::path::PathBuf::from("/usr/share/qemu"));

    for dir in &paths {
        let p = dir.join(name);
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

// ── Argument parsing ──

struct Args {
    ram_mb: u32,
    disk: String,
    iso: String,
    bios: String,
    timeout: u32,
    show_screen: bool,
    show_regs: bool,
}

fn parse_args() -> Args {
    let argv: Vec<String> = env::args().collect();
    let mut args = Args {
        ram_mb: 256,
        disk: String::new(),
        iso: String::new(),
        bios: "seabios".into(),
        timeout: 30,
        show_screen: false,
        show_regs: false,
    };

    let mut i = 1;
    // Skip "run" subcommand if present
    if i < argv.len() && argv[i] == "run" { i += 1; }

    while i < argv.len() {
        match argv[i].as_str() {
            "-r" => { i += 1; if i < argv.len() { args.ram_mb = argv[i].parse().unwrap_or(256); } }
            "-d" => { i += 1; if i < argv.len() { args.disk = argv[i].clone(); } }
            "-i" => { i += 1; if i < argv.len() { args.iso = argv[i].clone(); } }
            "-b" => { i += 1; if i < argv.len() { args.bios = argv[i].clone(); } }
            "-t" => { i += 1; if i < argv.len() { args.timeout = argv[i].parse().unwrap_or(30); } }
            "-s" => { args.show_screen = true; }
            "-g" => { args.show_regs = true; }
            _ => {}
        }
        i += 1;
    }
    args
}

// ── BIOS loading (matches vmmanager logic) ──

fn load_seabios(handle: u64) -> Result<(), String> {
    let bios_path = find_bios("bios.bin")
        .ok_or("SeaBIOS bios.bin not found")?;
    let vgabios_path = find_bios("vgabios.bin")
        .ok_or("VGA BIOS vgabios.bin not found")?;

    let bios = std::fs::read(&bios_path)
        .map_err(|e| format!("Failed to read BIOS {}: {}", bios_path, e))?;
    let vgabios = std::fs::read(&vgabios_path)
        .map_err(|e| format!("Failed to read VGA BIOS {}: {}", vgabios_path, e))?;

    eprintln!("[vmctl] SeaBIOS: {} ({} bytes)", bios_path, bios.len());
    eprintln!("[vmctl] VGA BIOS: {} ({} bytes)", vgabios_path, vgabios.len());

    // Load at 0xC0000 (256KB SeaBIOS covers 0xC0000-0xFFFFF)
    corevm_load_binary(handle, 0xC0000, bios.as_ptr(), bios.len() as u32);

    // ROM overlay at top of 4GB address space
    let rom_base = 0x1_0000_0000u64 - bios.len() as u64;
    let rom_alloc = (bios.len() + 0xFFF) & !0xFFF;
    let layout = std::alloc::Layout::from_size_align(rom_alloc, 4096)
        .map_err(|e| format!("ROM layout error: {}", e))?;
    let rom_ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if rom_ptr.is_null() { return Err("Failed to alloc ROM".into()); }
    unsafe { std::ptr::copy_nonoverlapping(bios.as_ptr(), rom_ptr, bios.len()); }
    let ret = corevm_set_memory_region(handle, 1, rom_base, rom_alloc as u64, rom_ptr);
    if ret != 0 { return Err(format!("Failed to map ROM at {:#x}", rom_base)); }

    // VGA BIOS via fw_cfg
    let name = b"vgaroms/vgabios.bin";
    corevm_fw_cfg_add_file(handle, name.as_ptr(), name.len() as u32,
        vgabios.as_ptr(), vgabios.len() as u32);

    Ok(())
}

fn set_initial_cpu_state(handle: u64) {
    let data_seg = SegmentReg {
        base: 0, limit: 0xFFFF, selector: 0,
        type_: 0x03, present: 1, dpl: 0, db: 0, s: 1, l: 0, g: 0, avl: 0,
    };
    let sregs = VcpuSregs {
        cs: SegmentReg {
            base: 0xF0000, limit: 0xFFFF, selector: 0xF000,
            type_: 0x0B, present: 1, dpl: 0, db: 0, s: 1, l: 0, g: 0, avl: 0,
        },
        ds: data_seg, es: data_seg, fs: data_seg, gs: data_seg, ss: data_seg,
        tr: SegmentReg {
            base: 0, limit: 0xFFFF, selector: 0,
            type_: 0x0B, present: 1, dpl: 0, db: 0, s: 0, l: 0, g: 0, avl: 0,
        },
        ldt: SegmentReg {
            base: 0, limit: 0xFFFF, selector: 0,
            type_: 0x02, present: 1, dpl: 0, db: 0, s: 0, l: 0, g: 0, avl: 0,
        },
        gdt: DescriptorTable { base: 0, limit: 0xFFFF },
        idt: DescriptorTable { base: 0, limit: 0xFFFF },
        cr0: 0x10, cr2: 0, cr3: 0, cr4: 0, efer: 0,
    };
    corevm_set_vcpu_sregs(handle, 0, &sregs);

    let mut regs = VcpuRegs::default();
    regs.rip = 0xFFF0;
    regs.rflags = 0x02;
    corevm_set_vcpu_regs(handle, 0, &regs);
}

// ── Main ──

fn main() {
    let args = parse_args();

    if args.iso.is_empty() && args.disk.is_empty() {
        eprintln!("Usage: vmctl run -r <mb> -i <iso> [-d <disk>] -b seabios -t <secs> -s -g");
        std::process::exit(1);
    }

    eprintln!("[vmctl] Creating VM (RAM={}MB, bios={})...", args.ram_mb, args.bios);

    let handle = corevm_create(args.ram_mb);
    if handle == 0 {
        eprintln!("[vmctl] ERROR: Failed to create VM");
        std::process::exit(1);
    }

    let rc = corevm_create_vcpu(handle, 0);
    if rc != 0 {
        eprintln!("[vmctl] ERROR: Failed to create vCPU (rc={})", rc);
        std::process::exit(1);
    }

    corevm_setup_standard_devices(handle);
    corevm_setup_acpi_tables(handle);
    corevm_setup_ahci(handle, 6);

    // Map VGA framebuffer (same as vmmanager)
    let vga_fb_size: usize = 0x80_0000;
    let vga_layout = std::alloc::Layout::from_size_align(vga_fb_size, 4096).unwrap();
    let vga_fb_ptr = unsafe { std::alloc::alloc_zeroed(vga_layout) };
    if !vga_fb_ptr.is_null() {
        corevm_set_memory_region(handle, 10, 0xE000_0000, vga_fb_size as u64, vga_fb_ptr);
        corevm_set_memory_region(handle, 11, 0xFD00_0000, vga_fb_size as u64, vga_fb_ptr);
    }

    // Attach ISO
    if !args.iso.is_empty() {
        let path = std::ffi::CString::new(args.iso.as_str()).unwrap();
        let fd = unsafe { libc_open(path.as_ptr(), 0) }; // O_RDONLY
        if fd < 0 {
            eprintln!("[vmctl] ERROR: Cannot open ISO: {}", args.iso);
            std::process::exit(1);
        }
        let size = unsafe { libc_lseek(fd, 0, 2) } as u64;
        unsafe { libc_lseek(fd, 0, 0); }
        corevm_ahci_attach_cdrom(handle, 1, fd, size);
        eprintln!("[vmctl] ISO: {} ({} bytes)", args.iso, size);
    }

    // Attach disk
    if !args.disk.is_empty() {
        let path = std::ffi::CString::new(args.disk.as_str()).unwrap();
        let fd = unsafe { libc_open(path.as_ptr(), 2) }; // O_RDWR
        if fd < 0 {
            eprintln!("[vmctl] ERROR: Cannot open disk: {}", args.disk);
            std::process::exit(1);
        }
        let size = unsafe { libc_lseek(fd, 0, 2) } as u64;
        unsafe { libc_lseek(fd, 0, 0); }
        corevm_ahci_attach_disk(handle, 0, fd, size);
        eprintln!("[vmctl] Disk: {} ({} bytes)", args.disk, size);
    }

    // Load BIOS
    if let Err(e) = load_seabios(handle) {
        eprintln!("[vmctl] ERROR: {}", e);
        std::process::exit(1);
    }

    set_initial_cpu_state(handle);

    // Timer thread to cancel vCPU periodically (needed for PIT/IRQ delivery)
    let cancel_handle = handle;
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let running2 = running.clone();
    thread::spawn(move || {
        while running2.load(std::sync::atomic::Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
            corevm_cancel_vcpu(cancel_handle, 0);
        }
    });

    eprintln!("[vmctl] VM started (timeout={}s)", args.timeout);
    println!("--- SERIAL OUTPUT ---");

    let start = Instant::now();
    let timeout = Duration::from_secs(args.timeout as u64);
    let mut exit_count: u64 = 0;
    let mut serial_bytes: u64 = 0;
    let mut exit_reason = "timeout";
    let mut last_state = Instant::now();
    let mut last_pit_tick = Instant::now();

    loop {
        let mut exit = CExitReason::default();
        let rc = corevm_run_vcpu(handle, 0, &mut exit);
        if rc != 0 {
            thread::sleep(Duration::from_millis(1));
            if start.elapsed() >= timeout && args.timeout > 0 { break; }
            continue;
        }

        match exit.reason {
            0 => { // IoIn
                let mut data = [0u8; 4];
                corevm_handle_io_exit(handle, exit.port, 0, exit.size, data.as_mut_ptr());
            }
            1 => { // IoOut
                // Capture COM1 serial output
                if exit.port == 0x3F8 && exit.size == 1 {
                    let ch = (exit.data_u32 & 0xFF) as u8;
                    if ch >= 0x20 || ch == b'\n' || ch == b'\r' || ch == b'\t' {
                        print!("{}", ch as char);
                        serial_bytes += 1;
                    }
                }
                let mut data = exit.data_u32.to_le_bytes();
                corevm_handle_io_exit(handle, exit.port, 1, exit.size, data.as_mut_ptr());
            }
            2 => { // MmioRead
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg, exit.mmio_instr_len);
            }
            3 => { // MmioWrite
                let mut data = exit.data_u64.to_le_bytes();
                corevm_handle_mmio_exit(handle, exit.addr, 1, exit.size, data.as_mut_ptr(), 0, 0);
            }
            7 => { // Halted
                thread::sleep(Duration::from_millis(1));
            }
            9 => { // Shutdown
                exit_reason = "shutdown";
                eprintln!("[vmctl] VM shutdown (triple fault)");
                break;
            }
            11 => { // Error
                exit_reason = "error";
                eprintln!("[vmctl] VM error");
                break;
            }
            12 => { // StringIo
                corevm_handle_string_io_exit(
                    handle, exit.port, exit.string_io_is_write,
                    exit.string_io_count, exit.string_io_gpa,
                    exit.string_io_step, exit.string_io_instr_len,
                    exit.string_io_addr_size, exit.size,
                );
            }
            13 => { // Cancelled — timer kicked us out of KVM_RUN
            }
            _ => {}
        }

        // Time-based PIT ticking: advance channels proportional to elapsed real time.
        // PIT runs at 1.193182 MHz. Tick every ~100µs (~119 ticks) to keep channel 2
        // output responsive for port 0x61 delay loops.
        {
            let now = Instant::now();
            let elapsed_us = now.duration_since(last_pit_tick).as_micros() as u64;
            if elapsed_us >= 100 {
                let pit_ticks = ((elapsed_us * 1193) / 1000) as u32;
                corevm_pit_advance(handle, pit_ticks);
                last_pit_tick = now;
            }
        }

        // Poll IRQs
        corevm_poll_irqs(handle);

        // Drain debug port
        {
            let mut dbg = [0u8; 1024];
            let n = corevm_debug_port_take_output(handle, dbg.as_mut_ptr(), dbg.len() as u32);
            if n > 0 {
                if let Ok(s) = std::str::from_utf8(&dbg[..n as usize]) {
                    eprint!("{}", s);
                }
            }
        }

        exit_count += 1;

        // Periodic state dump (every 2s)
        if last_state.elapsed() >= Duration::from_secs(2) {
            last_state = Instant::now();
            let mut regs = VcpuRegs::default();
            let mut sregs = VcpuSregs::default();
            corevm_get_vcpu_regs(handle, 0, &mut regs);
            corevm_get_vcpu_sregs(handle, 0, &mut sregs);
            let mut vga_buf = [0u8; 160];
            corevm_read_phys(handle, 0xB8000, vga_buf.as_mut_ptr(), 160);
            let vga_line: String = (0..80).map(|i| {
                let ch = vga_buf[i * 2];
                if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' }
            }).collect::<String>().trim_end().to_string();
            eprintln!("[vm-state] #{} exit={} RIP={:#x} CS={:#x} CR0={:#x} IF={} PE={} PG={} VGA=[{}]",
                exit_count, exit.reason, regs.rip, sregs.cs.selector, sregs.cr0,
                if regs.rflags & 0x200 != 0 { 1 } else { 0 },
                if sregs.cr0 & 1 != 0 { 1 } else { 0 },
                if sregs.cr0 & (1 << 31) != 0 { 1 } else { 0 },
                vga_line);
        }

        // Check timeout
        if args.timeout > 0 && start.elapsed() >= timeout {
            break;
        }
    }

    running.store(false, std::sync::atomic::Ordering::Relaxed);
    let runtime = start.elapsed();

    println!("\n--- END SERIAL OUTPUT ---\n");

    println!("--- VM EXIT SUMMARY ---");
    println!("exit_reason: {}", exit_reason);
    println!("runtime_ms: {}", runtime.as_millis());
    println!("exit_count: {}", exit_count);
    println!("serial_bytes: {}", serial_bytes);
    println!("--- END SUMMARY ---\n");

    if args.show_screen {
        println!("--- VGA TEXT SCREEN (80x25) ---");
        for row in 0..25usize {
            let mut vga_row = [0u8; 160];
            corevm_read_phys(handle, 0xB8000 + (row * 160) as u64, vga_row.as_mut_ptr(), 160);
            let line: String = (0..80).map(|col| {
                let ch = vga_row[col * 2];
                if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' }
            }).collect::<String>().trim_end().to_string();
            println!("{}", line);
        }
        println!("--- END SCREEN ---\n");
    }

    if args.show_regs {
        let mut regs = VcpuRegs::default();
        let mut sregs = VcpuSregs::default();
        corevm_get_vcpu_regs(handle, 0, &mut regs);
        corevm_get_vcpu_sregs(handle, 0, &mut sregs);

        println!("--- CPU REGISTERS ---");
        println!("RAX={:016X}  RBX={:016X}  RCX={:016X}  RDX={:016X}", regs.rax, regs.rbx, regs.rcx, regs.rdx);
        println!("RSI={:016X}  RDI={:016X}  RBP={:016X}  RSP={:016X}", regs.rsi, regs.rdi, regs.rbp, regs.rsp);
        println!("R8 ={:016X}  R9 ={:016X}  R10={:016X}  R11={:016X}", regs.r8, regs.r9, regs.r10, regs.r11);
        println!("R12={:016X}  R13={:016X}  R14={:016X}  R15={:016X}", regs.r12, regs.r13, regs.r14, regs.r15);
        println!("RIP={:016X}  RFLAGS={:016X}", regs.rip, regs.rflags);
        println!("CR0={:016X}  CR2={:016X}  CR3={:016X}  CR4={:016X}  EFER={:016X}",
            sregs.cr0, sregs.cr2, sregs.cr3, sregs.cr4, sregs.efer);
        println!("CS: sel={:04X} base={:016X} limit={:08X}  DS: sel={:04X} base={:016X}",
            sregs.cs.selector, sregs.cs.base, sregs.cs.limit, sregs.ds.selector, sregs.ds.base);
        println!("SS: sel={:04X} base={:016X}  ES: sel={:04X}  FS: sel={:04X}  GS: sel={:04X}",
            sregs.ss.selector, sregs.ss.base, sregs.es.selector, sregs.fs.selector, sregs.gs.selector);
        println!("--- END REGISTERS ---");

        // Dump code at RIP — translate virtual to physical via kernel text mapping
        let rip = regs.rip;
        let phys_rip = if rip >= 0xFFFFFFFF80000000 {
            rip - 0xFFFFFFFF80000000 // kernel text direct mapping
        } else {
            rip
        };
        let mut code = [0u8; 32];
        corevm_read_phys(handle, phys_rip, code.as_mut_ptr(), 32);
        print!("Code at RIP={:016X} (phys={:08X}): ", rip, phys_rip);
        for b in &code { print!("{:02X} ", b); }
        println!();
    }

    corevm_destroy(handle);
}

// Minimal libc bindings for file operations
extern "C" {
    fn open(path: *const i8, flags: i32) -> i32;
    fn lseek(fd: i32, offset: i64, whence: i32) -> i64;
}

unsafe fn libc_open(path: *const i8, flags: i32) -> i32 {
    unsafe { open(path, flags) }
}

unsafe fn libc_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    unsafe { lseek(fd, offset, whence) }
}
