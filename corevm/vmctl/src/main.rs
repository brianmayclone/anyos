//! vmctl — Headless CoreVM CLI controller for Linux/KVM.
//!
//! Usage:
//!   vmctl run -r <mb> -i <iso> -b seabios -t <secs> -s -g
//!   vmctl run -r 512 -i /path/to/linux.iso -b seabios -t 60 -s -g

use std::time::{Duration, Instant};
use std::{env, thread};

use libcorevm::ffi::*;
use libcorevm::setup;
use libcorevm::backend::{VcpuRegs, VcpuSregs};

// ── Argument parsing ──

struct Args {
    ram_mb: u32,
    disk: String,
    iso: String,
    bios: String,
    timeout: u32,
    show_screen: bool,
    show_regs: bool,
    kernel: String,
    initrd: String,
    append: String,
    hpet: bool,
    ide_cdrom: bool,
    send_keys: Vec<(u32, Vec<u8>)>,  // (delay_ms, scancodes)
    send_mouse: Vec<(u32, i16, i16, u8)>, // (delay_ms, dx, dy, buttons)
    enable_hpet: bool,
    vram_mb: u32,     // VRAM in MiB (0 = default 16)
    disk_cache_mb: u32,
    disk_cache_mode: String,
    tap_name: String, // TAP device name (empty = no networking)
    bridge: String,   // Bridge name to attach TAP to (optional)
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
        hpet: false,
        ide_cdrom: false,
        kernel: String::new(),
        initrd: String::new(),
        append: String::new(),
        send_keys: Vec::new(),
        send_mouse: Vec::new(),
        enable_hpet: false,
        vram_mb: 0,
        disk_cache_mb: 32,
        disk_cache_mode: String::from("writeback"),
        tap_name: String::new(),
        bridge: String::new(),
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
            "--hpet" => { args.hpet = true; }
            "--ide-cdrom" => { args.ide_cdrom = true; }
            "--vram" => { i += 1; if i < argv.len() { args.vram_mb = argv[i].parse().unwrap_or(16); } }
            "--disk-cache" => { i += 1; if i < argv.len() { args.disk_cache_mb = argv[i].parse().unwrap_or(32); } }
            "--disk-cache-mode" => { i += 1; if i < argv.len() { args.disk_cache_mode = argv[i].clone(); } }
            "-g" => { args.show_regs = true; }
            "-k" => { i += 1; if i < argv.len() { args.kernel = argv[i].clone(); } }
            "--initrd" => { i += 1; if i < argv.len() { args.initrd = argv[i].clone(); } }
            "--append" => { i += 1; if i < argv.len() { args.append = argv[i].clone(); } }
            "--key" => {
                // --key <delay_ms>:<keyname> e.g. --key 2000:enter --key 5000:esc
                i += 1;
                if i < argv.len() {
                    if let Some((delay_str, key_name)) = argv[i].split_once(':') {
                        let delay: u32 = delay_str.parse().unwrap_or(1000);
                        let scancodes = match key_name {
                            "enter" | "return" => vec![0x1C],
                            "esc" | "escape" => vec![0x01],
                            "space" => vec![0x39],
                            "tab" => vec![0x0F],
                            "up" => vec![0xE0, 0x48],
                            "down" => vec![0xE0, 0x50],
                            "left" => vec![0xE0, 0x4B],
                            "right" => vec![0xE0, 0x4D],
                            "f1" => vec![0x3B],
                            "f2" => vec![0x3C],
                            "f3" => vec![0x3D],
                            "f4" => vec![0x3E],
                            "f5" => vec![0x3F],
                            "f6" => vec![0x40],
                            "f7" => vec![0x41],
                            "f8" => vec![0x42],
                            "f9" => vec![0x43],
                            "f10" => vec![0x44],
                            "f12" => vec![0x58],
                            _ => {
                                // Try as raw scancode hex (e.g. "1c")
                                if let Ok(sc) = u8::from_str_radix(key_name, 16) {
                                    vec![sc]
                                } else {
                                    eprintln!("[vmctl] Unknown key: {}", key_name);
                                    vec![]
                                }
                            }
                        };
                        if !scancodes.is_empty() {
                            args.send_keys.push((delay, scancodes));
                        }
                    }
                }
            }
            "--mouse" => {
                // --mouse <delay_ms>:<dx>,<dy>[,<buttons>]
                // e.g. --mouse 5000:50,0 --mouse 6000:0,50 --mouse 7000:-30,-30,1
                i += 1;
                if i < argv.len() {
                    if let Some((delay_str, rest)) = argv[i].split_once(':') {
                        let delay: u32 = delay_str.parse().unwrap_or(1000);
                        let parts: Vec<&str> = rest.split(',').collect();
                        if parts.len() >= 2 {
                            let dx: i16 = parts[0].parse().unwrap_or(0);
                            let dy: i16 = parts[1].parse().unwrap_or(0);
                            let btn: u8 = if parts.len() >= 3 { parts[2].parse().unwrap_or(0) } else { 0 };
                            args.send_mouse.push((delay, dx, dy, btn));
                        }
                    }
                }
            }
            "--hpet" => { args.enable_hpet = true; }
            "--tap" => { i += 1; if i < argv.len() { args.tap_name = argv[i].clone(); } }
            "--bridge" => { i += 1; if i < argv.len() { args.bridge = argv[i].clone(); } }
            _ => {}
        }
        i += 1;
    }
    args
}

// ── Main ──

fn main() {
    let args = parse_args();

    if args.iso.is_empty() && args.disk.is_empty() && args.kernel.is_empty() {
        eprintln!("Usage: vmctl run -r <mb> -i <iso> [-d <disk>] [-k <kernel> --initrd <initrd> --append <cmdline>] -b seabios -t <secs> -s -g");
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

    if args.vram_mb > 0 {
        setup::set_vram_mb(handle, args.vram_mb);
    }
    corevm_setup_standard_devices(handle);
    if args.hpet {
        corevm_setup_hpet(handle);
        eprintln!("[vmctl] HPET enabled");
        corevm_setup_acpi_tables_with_hpet(handle);
    } else {
        corevm_setup_acpi_tables(handle);
    }
    corevm_setup_ahci(handle, 6);

    // E1000 NIC — default MAC 52:54:00:12:34:56 (QEMU-style locally administered)
    let mac: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    corevm_setup_e1000(handle, mac.as_ptr());

    // AC97 Audio Controller — disabled by default in CLI tool
    // corevm_setup_ac97(handle);
    // eprintln!("[vmctl] AC97 audio controller enabled");

    // TAP network device (optional, requires root/CAP_NET_ADMIN)
    let tap_device = if !args.tap_name.is_empty() {
        match libcorevm::net::tap::TapDevice::new(&args.tap_name) {
            Ok(tap) => {
                if let Err(e) = tap.bring_up() {
                    eprintln!("[vmctl] WARNING: Failed to bring up TAP: {}", e);
                }
                if !args.bridge.is_empty() {
                    if let Err(e) = tap.add_to_bridge(&args.bridge) {
                        eprintln!("[vmctl] WARNING: Failed to add TAP to bridge '{}': {}", args.bridge, e);
                    } else {
                        eprintln!("[vmctl] TAP '{}' added to bridge '{}'", tap.name(), args.bridge);
                    }
                }
                eprintln!("[vmctl] TAP network: {}", tap.name());
                Some(tap)
            }
            Err(e) => {
                eprintln!("[vmctl] WARNING: Failed to create TAP device '{}': {} (networking disabled)", args.tap_name, e);
                None
            }
        }
    } else {
        None
    };

    // VGA LFB is mapped internally by setup_standard_devices via KVM slot 1.
    // Get the SVGA framebuffer pointer for reading display output.
    let mut vga_fb_ptr: *const u8 = core::ptr::null();
    let mut vga_fb_size: u32 = 0;
    corevm_vga_get_framebuffer(handle, &mut vga_fb_ptr as *mut *const u8 as *mut *const u8, &mut vga_fb_size);

    // Attach ISO — --ide-cdrom uses legacy IDE (Windows-compatible)
    if !args.iso.is_empty() {
        if args.ide_cdrom {
            if let Err(e) = setup::attach_cdrom_to_ide(handle, &args.iso) {
                eprintln!("[vmctl] ERROR: {}", e);
                std::process::exit(1);
            }
            eprintln!("[vmctl] ISO (IDE): {}", args.iso);
        } else {
            if let Err(e) = setup::attach_image_to_ahci(handle, &args.iso, 1, true) {
                eprintln!("[vmctl] ERROR: {}", e);
                std::process::exit(1);
            }
            eprintln!("[vmctl] ISO (AHCI): {}", args.iso);
        }
    }

    // Attach disk
    if !args.disk.is_empty() {
        if let Err(e) = setup::attach_image_to_ahci(handle, &args.disk, 0, false) {
            eprintln!("[vmctl] ERROR: {}", e);
            std::process::exit(1);
        }
        eprintln!("[vmctl] Disk: {}", args.disk);
        // Configure disk cache for port 0 (primary disk)
        let cache_mode = match args.disk_cache_mode.as_str() {
            "writethrough" => 1u32,
            "none" => 2,
            _ => 0, // writeback
        };
        setup::configure_disk_cache(handle, 0, args.disk_cache_mb, cache_mode);
        if args.disk_cache_mb > 0 && cache_mode < 2 {
            eprintln!("[vmctl] Disk cache: {} MB ({})", args.disk_cache_mb, args.disk_cache_mode);
        }
    }

    // Load BIOS — use vmmanager asset paths as extra search dirs
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let extra_paths = vec![manifest_dir.join("../vmmanager/assets/bios")];
    if let Err(e) = setup::load_seabios(handle, &extra_paths) {
        eprintln!("[vmctl] ERROR: {}", e);
        std::process::exit(1);
    }

    // Direct kernel boot via fw_cfg (like QEMU -kernel/-initrd/-append)
    if !args.kernel.is_empty() {
        // Load the linuxboot DMA option ROM (SeaBIOS scans for it)
        let linuxboot_paths = [
            "/usr/share/qemu/linuxboot_dma.bin",
            "/usr/share/seabios/linuxboot_dma.bin",
        ];
        for path in &linuxboot_paths {
            if let Ok(rom) = std::fs::read(path) {
                let name = b"genroms/linuxboot_dma.bin";
                corevm_fw_cfg_add_file(handle, name.as_ptr(), name.len() as u32,
                    rom.as_ptr(), rom.len() as u32);
                eprintln!("[vmctl] LinuxBoot ROM: {} ({} bytes)", path, rom.len());
                std::mem::forget(rom);
                break;
            }
        }

        // Load kernel
        let kernel = std::fs::read(&args.kernel)
            .unwrap_or_else(|e| { eprintln!("[vmctl] ERROR: Cannot read kernel: {}", e); std::process::exit(1); });
        eprintln!("[vmctl] Kernel: {} ({} bytes)", args.kernel, kernel.len());

        // Load initrd
        let initrd = if !args.initrd.is_empty() {
            let data = std::fs::read(&args.initrd)
                .unwrap_or_else(|e| { eprintln!("[vmctl] ERROR: Cannot read initrd: {}", e); std::process::exit(1); });
            eprintln!("[vmctl] Initrd: {} ({} bytes)", args.initrd, data.len());
            data
        } else {
            Vec::new()
        };

        // Build command line
        let cmdline = if !args.append.is_empty() {
            eprintln!("[vmctl] Cmdline: {}", args.append);
            args.append.as_bytes().to_vec()
        } else {
            Vec::new()
        };

        // Set up kernel boot via fw_cfg (legacy selectors + etc/linuxboot)
        corevm_fw_cfg_set_kernel(
            handle,
            kernel.as_ptr(), kernel.len() as u32,
            initrd.as_ptr(), initrd.len() as u32,
            cmdline.as_ptr(), cmdline.len() as u32,
        );
        std::mem::forget(kernel);
        std::mem::forget(initrd);

        // Set boot order: linuxboot ROM first (before DVD/CD)
        // This ensures direct kernel boot takes priority over ISO boot.
        {
            let bootorder = b"/rom@genroms/linuxboot_dma.bin\n\0";
            let bname = b"bootorder";
            corevm_fw_cfg_add_file(handle, bname.as_ptr(), bname.len() as u32,
                bootorder.as_ptr(), bootorder.len() as u32);
        }

        // Also add file-based entries for linuxboot_dma.bin compatibility
        if !args.append.is_empty() {
            let mut cmdline_buf = args.append.clone().into_bytes();
            cmdline_buf.push(0);
            let cname = b"etc/cmdline";
            corevm_fw_cfg_add_file(handle, cname.as_ptr(), cname.len() as u32,
                cmdline_buf.as_ptr(), cmdline_buf.len() as u32);
            std::mem::forget(cmdline_buf);
        }
    }

    if let Err(e) = setup::set_initial_cpu_state(handle) {
        eprintln!("[vmctl] ERROR: {}", e);
        std::process::exit(1);
    }

    // Write COM1 base address into BDA so SeaBIOS finds the serial port.
    setup::setup_bda_com1(handle);

    // Install SIGUSR1 handler so cancel_vcpu can interrupt KVM_RUN mid-execution
    libcorevm::backend::kvm::install_sigusr1_handler();

    // Timer thread to cancel vCPU periodically (needed for CMOS/RTC advance,
    // HPET timer polling, and AHCI IRQ polling when guest is in HLT/idle state).
    // 10ms when HPET is enabled (Windows needs timely HPET interrupts ~64 Hz),
    // 100ms otherwise (avoids excessive VM exits for Linux guests).
    let cancel_interval_ms = if args.enable_hpet { 10 } else { 100 };
    let cancel_handle = handle;
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let running2 = running.clone();
    thread::spawn(move || {
        while running2.load(std::sync::atomic::Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(cancel_interval_ms));
            corevm_cancel_vcpu(cancel_handle, 0);
        }
    });

    // Key injection thread: schedule keypresses at specified delays
    if !args.send_keys.is_empty() {
        let key_handle = handle;
        let keys = args.send_keys.clone();
        thread::spawn(move || {
            for (delay_ms, scancodes) in &keys {
                thread::sleep(Duration::from_millis(*delay_ms as u64));

                // Method 1: PS/2 hardware injection (IRQ 1)
                for &sc in scancodes {
                    corevm_ps2_key_press(key_handle, sc);
                    thread::sleep(Duration::from_millis(50));
                    corevm_ps2_key_release(key_handle, sc);
                    thread::sleep(Duration::from_millis(50));
                }

                // Method 2: Also inject directly into BIOS keyboard buffer (BDA)
                // This works even if the guest's INT 09h handler isn't set up properly.
                // BDA keyboard buffer: 0x41E-0x43E (circular, 16 entries of 2 bytes)
                // Head pointer at 0x41A, tail pointer at 0x41C
                // Each entry: low byte = ASCII, high byte = scancode
                let bios_key: u16 = match scancodes.first() {
                    Some(&0x1C) => 0x1C0D, // Enter: ASCII 0x0D, scancode 0x1C
                    Some(&0x01) => 0x011B, // Esc: ASCII 0x1B, scancode 0x01
                    Some(&0x39) => 0x3920, // Space: ASCII 0x20, scancode 0x39
                    _ => 0,
                };
                if bios_key != 0 {
                    // BDA keyboard buffer: segment 0x0040, offsets 0x1A..0x3E
                    // Physical base = 0x400. Head/tail store BDA-relative offsets.
                    const BDA_BASE: u64 = 0x400;
                    let mut buf = [0u8; 4];
                    corevm_read_phys(key_handle, BDA_BASE + 0x1A, buf.as_mut_ptr(), 4);
                    let head = u16::from_le_bytes([buf[0], buf[1]]);
                    let tail = u16::from_le_bytes([buf[2], buf[3]]);
                    // Write key at BDA_BASE + tail offset
                    let key_bytes = bios_key.to_le_bytes();
                    corevm_write_phys(key_handle, BDA_BASE + tail as u64, key_bytes.as_ptr(), 2);
                    // Advance tail (circular: 0x1E to 0x3C, wrap to 0x1E)
                    let new_tail = if tail + 2 >= 0x3E { 0x1E } else { tail + 2 };
                    let tail_bytes = new_tail.to_le_bytes();
                    corevm_write_phys(key_handle, BDA_BASE + 0x1C, tail_bytes.as_ptr(), 2);
                    eprintln!("[vmctl] BDA keyboard inject: key={:#06x} phys={:#06x} tail {:#06x}->{:#06x}",
                        bios_key, BDA_BASE + tail as u64, tail, new_tail);
                }

                eprintln!("[vmctl] Sent key at {}ms: {:?}", delay_ms, scancodes);
            }
        });
    }

    // Mouse injection thread: schedule mouse movements at specified delays
    if !args.send_mouse.is_empty() {
        let mouse_handle = handle;
        let moves = args.send_mouse.clone();
        thread::spawn(move || {
            let start = std::time::Instant::now();
            for (delay_ms, dx, dy, buttons) in &moves {
                // Delays are absolute from VM start, not cumulative.
                let target = Duration::from_millis(*delay_ms as u64);
                let elapsed = start.elapsed();
                if target > elapsed {
                    thread::sleep(target - elapsed);
                }
                corevm_ps2_mouse_move(mouse_handle, *dx, *dy, *buttons);
                eprintln!("[vmctl] Sent mouse at {}ms: dx={} dy={} btn={}", delay_ms, dx, dy, buttons);
            }
        });
    }

    // Stdin forwarding thread: read from host stdin and inject into guest serial port.
    // This enables interactive serial console (e.g., init=/bin/sh).
    {
        let stdin_handle = handle;
        let running3 = running.clone();
        thread::spawn(move || {
            use std::io::Read;
            let stdin = std::io::stdin();
            let mut stdin = stdin.lock();
            let mut buf = [0u8; 64];
            loop {
                if !running3.load(std::sync::atomic::Ordering::Relaxed) { break; }
                match stdin.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        corevm_serial_send_input(stdin_handle, buf.as_ptr(), n as u32);
                        // Wake up the vCPU so it processes the serial interrupt
                        corevm_cancel_vcpu(stdin_handle, 0);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    eprintln!("[vmctl] VM started (timeout={}s)", args.timeout);
    println!("--- SERIAL OUTPUT ---");

    let start = Instant::now();
    let timeout = Duration::from_secs(args.timeout as u64);
    let mut exit_count: u64 = 0;
    let mut serial_bytes: u64 = 0;
    let mut exit_reason = "timeout";
    let mut last_state = Instant::now();
    let mut last_pit_tick = Instant::now();
    let mut last_rtc_tick = Instant::now();
    let mut exit_types = [0u64; 16]; // count by exit.reason
    let mut total_irqs: u64 = 0;
    let mut mmio_addrs: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    let mut io_ports: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    let mut last_net_poll = Instant::now();
    let mut last_audio_poll = Instant::now();
    let mut net_packets: u64 = 0;
    let mut audio_samples: u64 = 0;

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
                *io_ports.entry(exit.port).or_insert(0) += 1;
                if exit.io_count > 1 {
                    corevm_complete_string_io(handle, 0, exit.port, 0, exit.size, exit.io_count);
                } else {
                    let mut data = [0u8; 4];
                    corevm_handle_io_exit(handle, exit.port, 0, exit.size, data.as_mut_ptr());
                }
            }
            1 => { // IoOut
                *io_ports.entry(exit.port).or_insert(0) += 1;
                if exit.io_count > 1 {
                    // String I/O (REP OUTSB): handle all iterations at once
                    corevm_complete_string_io(handle, 0, exit.port, 1, exit.size, exit.io_count);
                } else {
                    let mut data = exit.data_u32.to_le_bytes();
                    corevm_handle_io_exit(handle, exit.port, 1, exit.size, data.as_mut_ptr());
                }
            }
            2 => { // MmioRead
                *mmio_addrs.entry(exit.addr & !0xFFF).or_insert(0) += 1;
                let mut data = [0u8; 8];
                corevm_handle_mmio_exit(handle, exit.addr, 0, exit.size, data.as_mut_ptr(), exit.mmio_dest_reg, exit.mmio_instr_len);
            }
            3 => { // MmioWrite
                *mmio_addrs.entry(exit.addr & !0xFFF).or_insert(0) += 1;
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

        // Advance CMOS RTC periodic timer (32.768 kHz base clock)
        {
            let now = Instant::now();
            let elapsed_us = now.duration_since(last_rtc_tick).as_micros() as u64;
            if elapsed_us >= 100 {
                let rtc_ticks = (elapsed_us * 32768) / 1_000_000;
                if rtc_ticks > 0 {
                    corevm_cmos_advance(handle, rtc_ticks);
                    last_rtc_tick = now;
                }
            }
        }

        // Poll IRQs
        total_irqs += corevm_poll_irqs(handle) as u64;

        // Network polling: exchange packets between TAP and E1000 (~every 1ms)
        if tap_device.is_some() {
            let now = Instant::now();
            if now.duration_since(last_net_poll).as_micros() >= 1000 {
                let tap = tap_device.as_ref().unwrap();
                let n = libcorevm::net::tap::poll_network(tap, handle);
                net_packets += n as u64;
                last_net_poll = now;
            }
        }

        // Audio polling: process AC97 DMA (~every 10ms)
        {
            let now = Instant::now();
            if now.duration_since(last_audio_poll).as_millis() >= 10 {
                corevm_ac97_process(handle);
                // Drain audio samples (discard in vmctl — no audio output in headless mode)
                let mut audio_buf = [0i16; 8192];
                let n = corevm_ac97_take_audio(handle, audio_buf.as_mut_ptr(), audio_buf.len() as u32);
                audio_samples += n as u64;
                last_audio_poll = now;
            }
        }

        // Drain debug port
        {
            let mut dbg = [0u8; 1024];
            let n = corevm_debug_port_take_output(handle, dbg.as_mut_ptr(), dbg.len() as u32);
            if n > 0 {
                if let Ok(s) = std::str::from_utf8(&dbg[..n as usize]) {
                    print!("{}", s);
                    serial_bytes += n as u64;
                }
            }
        }

        // Drain serial output (catches string I/O and any other writes to THR)
        {
            let mut ser = [0u8; 4096];
            let n = corevm_serial_take_output(handle, ser.as_mut_ptr(), ser.len() as u32);
            if n > 0 {
                for &ch in &ser[..n as usize] {
                    if ch >= 0x20 || ch == b'\n' || ch == b'\r' || ch == b'\t' {
                        print!("{}", ch as char);
                        serial_bytes += 1;
                        if ch == b'\n' {
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                        }
                    }
                }
            }
        }

        exit_count += 1;
        if (exit.reason as usize) < exit_types.len() {
            exit_types[exit.reason as usize] += 1;
        }

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
            // Exit type distribution: 0=IoIn 1=IoOut 2=MmioRd 3=MmioWr 7=Hlt 9=Shutdown 13=Cancel
            eprintln!("[vm-exits] IO={}/{} MMIO={}/{} Hlt={} Cancel={} Other={}",
                exit_types[0], exit_types[1], exit_types[2], exit_types[3],
                exit_types[7], exit_types[13],
                exit_types[4]+exit_types[5]+exit_types[6]+exit_types[8]+exit_types[9]+exit_types[10]+exit_types[11]+exit_types[12]);
            // VGA mode and framebuffer info
            {
                let mut w: u32 = 0;
                let mut h: u32 = 0;
                let mut bpp: u8 = 0;
                let mode_rc = corevm_vga_get_mode(handle, &mut w, &mut h, &mut bpp);
                let lfb_addr = corevm_vga_get_lfb_addr(handle);
                let (fb_nonzero, fb_total_nz, fb_offset) = if !vga_fb_ptr.is_null() {
                    let fb = unsafe { core::slice::from_raw_parts(vga_fb_ptr, vga_fb_size as usize) };
                    let head_nz = fb.iter().take(1024 * 4 * 100).filter(|&&b| b != 0).count();
                    let total_nz = fb.iter().filter(|&&b| b != 0).count();
                    // Find first non-zero 4KB page beyond the first 100 lines
                    let mut off = 0usize;
                    for page_start in (0..fb.len()).step_by(4096) {
                        let page_end = (page_start + 4096).min(fb.len());
                        if fb[page_start..page_end].iter().any(|&b| b != 0) {
                            off = page_start;
                            break;
                        }
                    }
                    (head_nz, total_nz, off)
                } else { (0, 0, 0) };
                let vbe_fb_off = corevm_vga_get_fb_offset(handle);
                eprintln!("[vm-gfx] mode={}x{}x{} rc={} lfb={:#x} fb_nz={}/{} off={:#x} vbe_off={:#x} irqs={}",
                    w, h, bpp, mode_rc, lfb_addr, fb_nonzero, fb_total_nz, fb_offset, vbe_fb_off, total_irqs);
            }
            // Check BIOS tick counter at BDA 0x40:0x6C (physical 0x46C)
            {
                let mut tick_buf = [0u8; 4];
                corevm_read_phys(handle, 0x046C, tick_buf.as_mut_ptr(), 4);
                let bios_ticks = u32::from_le_bytes(tick_buf);
                eprintln!("[vm-timer] BIOS_ticks={} (~{:.1}s)", bios_ticks, bios_ticks as f64 / 18.2);
            }
            // LAPIC diagnostics (KVM in-kernel IRQCHIP)
            {
                let mut lapic_buf = [0u8; 1024];
                let rc = corevm_get_lapic(handle, 0, lapic_buf.as_mut_ptr());
                if rc == 0 {
                    // LAPIC registers at 16-byte aligned offsets (only first 4 bytes of each 16-byte slot)
                    let read32 = |off: usize| -> u32 {
                        u32::from_le_bytes([lapic_buf[off], lapic_buf[off+1], lapic_buf[off+2], lapic_buf[off+3]])
                    };
                    let apic_id = read32(0x20);        // APIC ID
                    let lvt_timer = read32(0x320);     // LVT Timer
                    let timer_icr = read32(0x380);     // Timer Initial Count
                    let timer_ccr = read32(0x390);     // Timer Current Count
                    let timer_dcr = read32(0x3E0);     // Timer Divide Config
                    let spurious = read32(0xF0);       // Spurious Interrupt Vector
                    let isr0 = read32(0x100);          // In-Service Register bits 0-31
                    let irr0 = read32(0x200);          // IRQ Request Register bits 0-31
                    let irr1 = read32(0x210);          // IRQ Request Register bits 32-63
                    let irr7 = read32(0x270);          // IRQ Request Register bits 224-255 (timer vec 0xEC=236)
                    let isr7 = read32(0x170);          // In-Service Register bits 224-255
                    eprintln!("[vm-lapic] id={} spur={:#x} lvt_timer={:#010x} icr={} ccr={} dcr={:#x} isr0={:#x} irr0={:#x} irr7={:#x} isr7={:#x}",
                        apic_id >> 24, spurious, lvt_timer, timer_icr, timer_ccr, timer_dcr, isr0, irr0, irr7, isr7);
                }
            }
            // Check BDA keyboard buffer: head @ 0x41A, tail @ 0x41C
            {
                let mut kbd_buf = [0u8; 4];
                corevm_read_phys(handle, 0x041A, kbd_buf.as_mut_ptr(), 4);
                let head = u16::from_le_bytes([kbd_buf[0], kbd_buf[1]]);
                let tail = u16::from_le_bytes([kbd_buf[2], kbd_buf[3]]);
                if head != tail {
                    eprintln!("[vm-kbd] BDA kbd buf head={:#06x} tail={:#06x} (keys pending!)", head, tail);
                }
            }
        }

        // Check timeout
        if args.timeout > 0 && start.elapsed() >= timeout {
            break;
        }
    }

    running.store(false, std::sync::atomic::Ordering::Relaxed);
    let runtime = start.elapsed();

    // MMIO address distribution
    if !mmio_addrs.is_empty() {
        let mut addrs: Vec<_> = mmio_addrs.iter().collect();
        addrs.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("[vmctl] MMIO address distribution (page-aligned):");
        for (addr, count) in addrs.iter().take(10) {
            eprintln!("  {:#010x}: {} accesses", addr, count);
        }
    }

    // IO port distribution
    if !io_ports.is_empty() {
        let mut ports: Vec<_> = io_ports.iter().collect();
        ports.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("[vmctl] IO port distribution:");
        for (port, count) in ports.iter().take(50) {
            eprintln!("  {:#06x}: {} accesses", port, count);
        }
    }

    // LAPIC register dump (Linux/KVM only)
    {
        let mut lapic = [0u8; 1024];
        if corevm_get_lapic(handle, 0, lapic.as_mut_ptr()) == 0 {
            // Key LAPIC registers (offsets from Intel SDM Vol 3A, Table 10-1)
            let read32 = |off: usize| u32::from_le_bytes([lapic[off], lapic[off+1], lapic[off+2], lapic[off+3]]);
            let apic_id = read32(0x020);
            let apic_ver = read32(0x030);
            let tpr = read32(0x080);
            let svr = read32(0x0F0);  // Spurious Vector Register
            let isr0 = read32(0x100);
            let irr0 = read32(0x200);
            let esr = read32(0x280);
            let lvt_timer = read32(0x320);
            let lvt_lint0 = read32(0x350);
            let lvt_lint1 = read32(0x360);
            let timer_init = read32(0x380);
            let timer_cur = read32(0x390);
            let timer_div = read32(0x3E0);
            eprintln!("[lapic] ID={:#x} VER={:#x} TPR={:#x} SVR={:#x}", apic_id, apic_ver, tpr, svr);
            eprintln!("[lapic] ISR0={:#010x} IRR0={:#010x} ESR={:#x}", isr0, irr0, esr);
            eprintln!("[lapic] LVT_TIMER={:#010x} LVT_LINT0={:#010x} LVT_LINT1={:#010x}", lvt_timer, lvt_lint0, lvt_lint1);
            eprintln!("[lapic] TimerInit={} TimerCur={} TimerDiv={:#x}", timer_init, timer_cur, timer_div);
            // Decode LVT Timer: bit 16=masked, bits 18:17=mode (00=one-shot, 01=periodic, 10=TSC-Deadline)
            let masked = (lvt_timer >> 16) & 1;
            let mode = (lvt_timer >> 17) & 3;
            let vector = lvt_timer & 0xFF;
            let mode_str = match mode { 0 => "one-shot", 1 => "periodic", 2 => "TSC-deadline", _ => "?" };
            eprintln!("[lapic] Timer: vector={:#x} mode={} masked={}", vector, mode_str, masked);
            // SVR: bit 8 = APIC software enable
            let apic_enabled = (svr >> 8) & 1;
            eprintln!("[lapic] APIC software enabled={}", apic_enabled);
            // Full IRR and ISR (8 x 32-bit each)
            let mut irr_any = false;
            let mut isr_any = false;
            for i in 0..8 {
                let irr = read32(0x200 + i * 0x10);
                let isr = read32(0x100 + i * 0x10);
                if irr != 0 { eprintln!("[lapic] IRR[{}]={:#010x}", i, irr); irr_any = true; }
                if isr != 0 { eprintln!("[lapic] ISR[{}]={:#010x}", i, isr); isr_any = true; }
            }
            if !irr_any { eprintln!("[lapic] IRR: all zero (no pending interrupts)"); }
            if !isr_any { eprintln!("[lapic] ISR: all zero (no in-service interrupts)"); }
        }
    }
    // IOAPIC redirection table dump
    {
        let mut ioapic = [0u8; 512];
        if corevm_get_irqchip(handle, 2, ioapic.as_mut_ptr()) == 0 {
            // KVM IOAPIC state: base_address(u64), ioregsel(u32), id(u32), irr(u32), pad(u32)
            // Then 24 redirection entries: each 8 bytes (u64)
            // Offset of entries: 8 + 4 + 4 + 4 + 4 = 24
            let read64 = |off: usize| u64::from_le_bytes([
                ioapic[off], ioapic[off+1], ioapic[off+2], ioapic[off+3],
                ioapic[off+4], ioapic[off+5], ioapic[off+6], ioapic[off+7]
            ]);
            let read32 = |off: usize| u32::from_le_bytes([ioapic[off], ioapic[off+1], ioapic[off+2], ioapic[off+3]]);
            let base = read64(0);
            let ioregsel = read32(8);
            let id = read32(12);
            let irr = read32(16);
            eprintln!("[ioapic] base={:#x} ioregsel={:#x} id={:#x} irr={:#010x}", base, ioregsel, id, irr);
            // Dump first 16 redirection entries
            for i in 0..16u32 {
                let entry = read64(24 + i as usize * 8);
                let vector = entry & 0xFF;
                let delivery = (entry >> 8) & 7;
                let dest_mode = (entry >> 11) & 1;
                let polarity = (entry >> 13) & 1;
                let trigger = (entry >> 15) & 1;
                let masked = (entry >> 16) & 1;
                let dest = (entry >> 56) & 0xFF;
                if entry != 0x10000 && entry != 0 { // Skip default masked entries
                    eprintln!("[ioapic] pin {}: vec={:#04x} del={} dest_mode={} pol={} trig={} mask={} dest={}",
                        i, vector, delivery, dest_mode, polarity, trigger, masked, dest);
                }
            }
        }
    }

    // RSDP scan: verify ACPI tables are in guest RAM
    {
        let rsdp_sig = b"RSD PTR ";
        let mut rsdp_found = false;
        // Scan EBDA (first 1KB at segment from BDA[0x40E]) and BIOS ROM area (0xE0000-0xFFFFF)
        let scan_regions: &[(u64, u64)] = &[
            (0x9FC00, 0xA0000),      // EBDA (default location)
            (0xE0000, 0x100000),     // BIOS ROM area
        ];
        for &(start, end) in scan_regions {
            let region_size = (end - start) as usize;
            let mut buf = vec![0u8; region_size];
            corevm_read_phys(handle, start, buf.as_mut_ptr(), region_size as u32);
            // Scan on 16-byte boundaries (RSDP must be 16-byte aligned)
            let mut off = 0;
            while off + 8 <= region_size {
                if &buf[off..off+8] == rsdp_sig {
                    let phys = start + off as u64;
                    eprintln!("[acpi] Found RSDP at phys {:#x}", phys);
                    rsdp_found = true;
                    if off + 36 <= region_size {
                        let revision = buf[off + 15];
                        let rsdt_addr = u32::from_le_bytes([buf[off+16], buf[off+17], buf[off+18], buf[off+19]]);
                        eprintln!("[acpi] RSDP revision={} RSDT={:#x}", revision, rsdt_addr);
                        if revision >= 2 && off + 36 <= region_size {
                            let xsdt_addr = u64::from_le_bytes([
                                buf[off+24], buf[off+25], buf[off+26], buf[off+27],
                                buf[off+28], buf[off+29], buf[off+30], buf[off+31]
                            ]);
                            eprintln!("[acpi] RSDP XSDT={:#x}", xsdt_addr);
                            // Try to read XSDT header
                            if xsdt_addr > 0 && xsdt_addr < args.ram_mb as u64 * 1024 * 1024 {
                                let mut xsdt_hdr = [0u8; 128];
                                corevm_read_phys(handle, xsdt_addr, xsdt_hdr.as_mut_ptr(), 128);
                                let sig = core::str::from_utf8(&xsdt_hdr[0..4]).unwrap_or("????");
                                let len = u32::from_le_bytes([xsdt_hdr[4], xsdt_hdr[5], xsdt_hdr[6], xsdt_hdr[7]]);
                                eprintln!("[acpi] XSDT sig='{}' len={}", sig, len);
                                // Read table pointers from XSDT
                                let num_entries = (len as usize - 36) / 8;
                                for i in 0..num_entries.min(8) {
                                    let off = 36 + i * 8;
                                    let ptr = u64::from_le_bytes([
                                        xsdt_hdr[off], xsdt_hdr[off+1], xsdt_hdr[off+2], xsdt_hdr[off+3],
                                        xsdt_hdr[off+4], xsdt_hdr[off+5], xsdt_hdr[off+6], xsdt_hdr[off+7]
                                    ]);
                                    // Read table signature
                                    if ptr > 0 && ptr < args.ram_mb as u64 * 1024 * 1024 {
                                        let mut tsig = [0u8; 8];
                                        corevm_read_phys(handle, ptr, tsig.as_mut_ptr(), 8);
                                        let s = core::str::from_utf8(&tsig[0..4]).unwrap_or("????");
                                        let tlen = u32::from_le_bytes([tsig[4], tsig[5], tsig[6], tsig[7]]);
                                        eprintln!("[acpi] XSDT[{}] -> {:#x} sig='{}' len={}", i, ptr, s, tlen);
                                    } else {
                                        eprintln!("[acpi] XSDT[{}] -> {:#x} (out of range)", i, ptr);
                                    }
                                }
                            }
                        }
                        if rsdt_addr > 0 && (rsdt_addr as u64) < args.ram_mb as u64 * 1024 * 1024 {
                            let mut rsdt_hdr = [0u8; 44];
                            corevm_read_phys(handle, rsdt_addr as u64, rsdt_hdr.as_mut_ptr(), 44);
                            let sig = core::str::from_utf8(&rsdt_hdr[0..4]).unwrap_or("????");
                            let len = u32::from_le_bytes([rsdt_hdr[4], rsdt_hdr[5], rsdt_hdr[6], rsdt_hdr[7]]);
                            eprintln!("[acpi] RSDT sig='{}' len={}", sig, len);
                        }
                    }
                    break;
                }
                off += 16; // RSDP is 16-byte aligned
            }
            if rsdp_found { break; }
        }
        if !rsdp_found {
            eprintln!("[acpi] WARNING: No RSDP found in EBDA or BIOS ROM area!");
        }
    }

    // VGA text screen dump (always, not just with -s flag)
    {
        eprintln!("[vga-text] First 10 lines of VGA text buffer:");
        for row in 0..10usize {
            let mut vga_row = [0u8; 160];
            corevm_read_phys(handle, 0xB8000 + (row * 160) as u64, vga_row.as_mut_ptr(), 160);
            let line: String = (0..80).map(|col| {
                let ch = vga_row[col * 2];
                if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' }
            }).collect::<String>().trim_end().to_string();
            if !line.is_empty() {
                eprintln!("[vga-text] {:2}: {}", row, line);
            }
        }
    }

    // Search for kernel messages in guest RAM
    {
        let patterns: &[&[u8]] = &[
            b"vesafb:",
            b"Console: ",
            b"Run /init",
            b"Freeing unused",
            b"fb0: ",
            b"bochs-drm",
            b"Xorg",
            b"xinit",
            b"login",
            b"EXT4",
            b"e1000: eth",
            b"e1000 0000:",
            b"Intel(R) PRO/1000",
            b"EEPROM Checksum",
            b"Hardware Err",
        ];
        let scan_end = (args.ram_mb as u64 * 1024 * 1024).min(512 * 1024 * 1024);
        let mut page = [0u8; 4096];
        let mut matches_found = 0u32;
        let mut addr = 0x100000u64;
        while addr < scan_end && matches_found < 20 {
            corevm_read_phys(handle, addr, page.as_mut_ptr(), 4096);
            for pattern in patterns {
                if pattern.len() > 4096 { continue; }
                for i in 0..4096 - pattern.len() {
                    if &page[i..i + pattern.len()] == *pattern {
                        let phys = addr + i as u64;
                        // Read context around the match
                        let ctx_start = if phys > 256 { phys - 256 } else { 0 };
                        let mut ctx = vec![0u8; 1024];
                        corevm_read_phys(handle, ctx_start, ctx.as_mut_ptr(), ctx.len() as u32);
                        // Extract printable ASCII around the match
                        let text: String = ctx.iter().filter_map(|&b| {
                            if b >= 0x20 && b < 0x7F { Some(b as char) }
                            else if b == b'\n' || b == b'\r' { Some(' ') }
                            else { None }
                        }).collect();
                        let pat_str = core::str::from_utf8(pattern).unwrap_or("?");
                        eprintln!("[kernel-scan] '{}' at {:#x}: ...{}...",
                            pat_str, phys, &text[..text.len().min(200)]);
                        matches_found += 1;
                        break;
                    }
                }
            }
            addr += 4096;
        }
        if matches_found == 0 {
            eprintln!("[kernel-scan] No kernel messages found in first {}MB of RAM", scan_end / 1024 / 1024);
        }
    }

    println!("\n--- END SERIAL OUTPUT ---\n");

    println!("--- VM EXIT SUMMARY ---");
    println!("exit_reason: {}", exit_reason);
    println!("runtime_ms: {}", runtime.as_millis());
    println!("exit_count: {}", exit_count);
    println!("serial_bytes: {}", serial_bytes);
    println!("exit_types: IoIn={} IoOut={} MmioRd={} MmioWr={} Hlt={} Cancel={} Shutdown={} Err={}",
        exit_types[0], exit_types[1], exit_types[2], exit_types[3],
        exit_types[7], exit_types[13], exit_types[9], exit_types[11]);
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

        // Dump code at RIP — translate virtual to physical
        let rip = regs.rip;
        let phys_rip = if rip >= 0xFFFFFFFF80000000 {
            rip - 0xFFFFFFFF80000000 // 64-bit kernel text direct mapping
        } else if rip >= 0xC0000000 && rip < 0x100000000 {
            rip - 0xC0000000 // 32-bit kernel PAGE_OFFSET mapping
        } else {
            rip
        };
        let mut code = [0u8; 32];
        corevm_read_phys(handle, phys_rip, code.as_mut_ptr(), 32);
        print!("Code at RIP={:016X} (phys={:08X}): ", rip, phys_rip);
        for b in &code { print!("{:02X} ", b); }
        println!();
    }

    // Save framebuffer as raw image file for inspection.
    // Use VBE offset registers to find where bochs-drm placed its framebuffer.
    if !vga_fb_ptr.is_null() && vga_fb_size > 0 {
        let fb_offset = corevm_vga_get_fb_offset(handle) as usize;
        let fb_path = "/tmp/corevm-framebuffer.raw";
        let avail = (vga_fb_size as usize).saturating_sub(fb_offset);
        let save_size = (1024 * 768 * 4).min(avail);
        if save_size > 0 {
            let fb = unsafe { core::slice::from_raw_parts(vga_fb_ptr.add(fb_offset), save_size) };
            if let Ok(()) = std::fs::write(fb_path, fb) {
                eprintln!("[vmctl] Framebuffer saved to {} (1024x768x32 BGRA, vram_offset=0x{:x})", fb_path, fb_offset);
            }
        }
    }

    corevm_destroy(handle);
}

