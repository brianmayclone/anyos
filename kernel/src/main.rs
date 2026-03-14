#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(dead_code, static_mut_refs)]
//! Kernel entry point and initialization sequence.
//!
//! Initializes all subsystems in 10 phases, from serial output to the desktop environment.

extern crate alloc;

mod arch;
mod boot_info;
mod crypto;
mod drivers;
mod fs;
mod graphics;
mod ipc;
#[cfg(feature = "kunit")]
mod kunit;
mod memory;
mod net;
mod panic;
pub mod sched_diag;
mod sync;
mod syscall;
mod task;

#[cfg(target_arch = "x86_64")]
use boot_info::BootInfo;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering as AtomicOrdering};

/// Boot mode: 0 = Legacy BIOS, 1 = UEFI.
static BOOT_MODE: AtomicU8 = AtomicU8::new(0);

/// GPU 2D acceleration available (queried by SYS_GPU_HAS_ACCEL).
pub static GPU_ACCEL: AtomicBool = AtomicBool::new(false);

/// GPU hardware cursor available (queried by SYS_GPU_HAS_HW_CURSOR).
pub static GPU_HW_CURSOR: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "nogui" parameter.
/// Skips compositor and init; starts textmode_console instead.
pub static NOGUI: AtomicBool = AtomicBool::new(false);

/// Get the boot mode (0 = BIOS, 1 = UEFI).
pub fn boot_mode() -> u8 {
    BOOT_MODE.load(AtomicOrdering::Relaxed)
}

/// Kernel entry point called from assembly after boot.
///
/// Receives the physical address of the [`BootInfo`] struct.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: u64) -> ! {
    /// Initialize serial port early so we can print debug info during boot.
    drivers::serial::init();

    // =========================================================================
    // Phase 1: Early output
    // =========================================================================
    serial_println!("");
    serial_println!("  .anyOS Kernel (x86_64) v{}", env!("ANYOS_VERSION"));

    #[cfg(target_arch = "x86_64")]
    {
        drivers::vga_text::init();
    }


    // =========================================================================
    // Phase 1b: Validate boot info (x86) / save DTB (ARM64)
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    let boot_info = {
        let boot_info = unsafe { &*(boot_info_addr as *const BootInfo) };
        let magic = unsafe { core::ptr::addr_of!((*boot_info).magic).read_unaligned() };
        if magic != boot_info::BOOT_INFO_MAGIC {
            serial_println!("WARNING: BootInfo magic mismatch (got {:#010x})", magic);
        } else {
            serial_println!("BootInfo validated (magic OK)");
        }

        let bmode = unsafe { core::ptr::addr_of!((*boot_info).boot_mode).read_unaligned() };
        BOOT_MODE.store(bmode, AtomicOrdering::Relaxed);
        serial_println!("Boot mode: {}", if bmode == 1 { "UEFI" } else { "BIOS" });

        let kstart = unsafe { core::ptr::addr_of!((*boot_info).kernel_phys_start).read_unaligned() };
        let kend = unsafe { core::ptr::addr_of!((*boot_info).kernel_phys_end).read_unaligned() };
        serial_println!("Kernel loaded at {:#010x} - {:#010x}", kstart, kend);

        // Parse boot_params for early options (e.g. "verbose")
        {
            let params = unsafe { core::ptr::addr_of!((*boot_info).boot_params).read_unaligned() };
            // Find null terminator
            let len = params.iter().position(|&b| b == 0).unwrap_or(params.len());
            if len > 0 {
                if let Ok(s) = core::str::from_utf8(&params[..len]) {
                    serial_println!("Boot params: \"{}\"", s);
                    for token in s.split_ascii_whitespace() {
                        if token == "verbose" {
                            crate::drivers::serial::set_verbose(true);
                            serial_println!("Verbose logging enabled via boot params");
                        } else if token == "nogui" {
                            NOGUI.store(true, AtomicOrdering::Relaxed);
                            serial_println!("No-GUI mode enabled via boot params (textmode_console)");
                        } else if let Some(res) = token.strip_prefix("res=") {
                            // Parse "res=WxH"
                            if let Some((w_str, h_str)) = res.split_once('x') {
                                if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.parse::<u32>()) {
                                    if w >= 640 && h >= 480 {
                                        crate::drivers::gpu::set_preferred_resolution(w, h);
                                        serial_println!("Preferred resolution: {}x{}", w, h);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        boot_info
    };

    // =========================================================================
    // Phase 2: CPU setup
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        arch::x86::gdt::init();
        serial_println!("[OK] GDT initialized");

        arch::x86::idt::init();
        serial_println!("[OK] IDT initialized (256 entries + syscall int 0x80)");

        arch::x86::tss::init();

        arch::x86::pic::init();
        serial_println!("[OK] PIC remapped (IRQ 0-15 -> INT 32-47)");

        arch::x86::cpuid::detect();
        arch::x86::cpuid::enable_smep();

        arch::x86::pit::init();
        serial_println!("[OK] PIT configured at {} Hz", arch::x86::pit::TICK_HZ);

        arch::x86::pit::calibrate_tsc();

        arch::x86::power::init();
    }

    // =========================================================================
    // Phase 3: Memory
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        arch::x86::pat::init(); // Program PAT before mapping framebuffer with WC
        memory::physical::init(boot_info);
        memory::virtual_mem::init(boot_info);
    }
    memory::heap::init();
    serial_println!("[OK] Heap allocator initialized");

    // =========================================================================
    // Phase 4: KUnit unit tests — pure algorithm / data-structure tests.
    // No hardware, scheduler, or driver state required.
    // =========================================================================
    #[cfg(feature = "kunit")]
    kunit::runner::run_unit_tests();

    // Initialize Cape Coral anti-aliased font (requires heap)
    #[cfg(target_arch = "x86_64")]
    graphics::cc_font::init();

    // =========================================================================
    // Phase 4b: ACPI + APIC (x86 only)
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    let acpi_info = {
        let rsdp_addr = unsafe { core::ptr::addr_of!((*boot_info).rsdp_addr).read_unaligned() };
        let acpi_info = arch::x86::acpi::init(rsdp_addr);
        if let Some(ref info) = acpi_info {
            arch::x86::apic::init_bsp(info.lapic_address);
            arch::x86::ioapic::init(&info.io_apics, &info.isos);
            arch::x86::ioapic::disable_legacy_pic();
            arch::x86::smp::init_bsp();
            arch::x86::smp::register_halt_ipi();
            arch::x86::smp::register_tlb_shootdown_ipi();
            arch::x86::syscall_msr::init_bsp();
        } else {
            serial_println!("  ACPI not found, using legacy PIC");
        }
        // KUnit: store ACPI results for integration tests.
        #[cfg(feature = "kunit")]
        {
            let ctx = kunit::integration::IntegrationCtx {
                acpi_present:     acpi_info.is_some(),
                lapic_address:    acpi_info.as_ref().map_or(0, |i| i.lapic_address),
                processor_count:  acpi_info.as_ref().map_or(0, |i| i.processors.len()),
                ioapic_count:     acpi_info.as_ref().map_or(0, |i| i.io_apics.len()),
                iso_count:        acpi_info.as_ref().map_or(0, |i| i.isos.len()),
            };
            kunit::integration::set_context(ctx);
        }

        acpi_info
    };

    // =========================================================================
    // Phase 5: Drivers (x86 only for now)
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        drivers::rtc::init();
        drivers::storage::ata::init();
        drivers::storage::atapi::init();
        drivers::framebuffer::init(boot_info);
        drivers::boot_console::init(); // Show boot splash (color logo)

        // Phase 5b: HAL + PCI device enumeration
        drivers::hal::init();
        drivers::boot_console::tick_spinner();
        drivers::pci::scan_all();
        drivers::pci::print_devices();
        drivers::boot_console::tick_spinner();

        // Phase 5c: E1000 NIC + Network Stack
        if drivers::network::e1000::init() {
            net::init();
        }
        drivers::boot_console::tick_spinner();
    }

    // =========================================================================
    // Phase 6: Scheduler (before interrupts, does not need filesystem)
    // =========================================================================
    task::scheduler::init();
    serial_println!("[OK] Scheduler initialized");
    #[cfg(target_arch = "x86_64")]
    drivers::boot_console::tick_spinner();

    // =========================================================================
    // Phase 7: Register IRQ handlers and enable interrupts
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        arch::x86::irq::register_irq(1, drivers::input::keyboard::irq_handler);
        arch::x86::irq::register_irq(12, drivers::input::mouse::irq_handler);

        // Drain stale data from the 8042 PS/2 controller before unmasking
        // keyboard/mouse IRQs.  The IOAPIC uses edge-triggered delivery for
        // ISA IRQs: if the 8042 output buffer is full (IRQ line already HIGH)
        // when we unmask, the IOAPIC never sees a rising edge and the
        // interrupt is lost forever — keyboard appears completely dead.
        // This can happen when a user presses keys during the boot sequence
        // (phases 1-6) while all IOAPIC entries are still masked.
        unsafe {
            for _ in 0..64 {
                if arch::x86::port::inb(0x64) & 0x01 == 0 { break; }
                let _ = arch::x86::port::inb(0x60);
            }
        }

        if acpi_info.is_some() {
            arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler);
            arch::x86::irq::register_irq(16, arch::x86::apic::timer_irq_handler);
            arch::x86::ioapic::unmask_irq(0);
            arch::x86::ioapic::unmask_irq(1);
            arch::x86::ioapic::unmask_irq(12);
        } else {
            arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler_with_schedule);
            // Drain 8042 output buffer (same edge-trigger issue applies to PIC)
            unsafe {
                for _ in 0..64 {
                    if arch::x86::port::inb(0x64) & 0x01 == 0 { break; }
                    let _ = arch::x86::port::inb(0x60);
                }
            }
            arch::x86::pic::unmask(0);
            arch::x86::pic::unmask(1);
            arch::x86::pic::unmask(12);
        }
    }
    arch::hal::enable_interrupts();
    serial_println!("[OK] Interrupts enabled");

    #[cfg(target_arch = "x86_64")]
    {
        // Switch serial output from blocking to async (IRQ 4 driven TX buffer)
        drivers::serial::enable_async();
        serial_println!("[OK] Serial TX now async (IRQ 4)");

        // Calibrate LAPIC timer using TSC
        if acpi_info.is_some() {
            arch::x86::apic::calibrate_timer(1000);
        }

        // HAL driver binding
        drivers::hal::probe_and_bind_all();
        drivers::hal::register_legacy_devices();
        drivers::hal::print_devices();
    }

    // =========================================================================
    // Phase 7i: KUnit integration tests — hardware state after full init.
    // Runs after: ACPI, APIC, IRQ routing, PIT, scheduler, interrupts enabled.
    // =========================================================================
    #[cfg(feature = "kunit")]
    kunit::runner::run_integration_tests();

    // =========================================================================
    // Phase 7e-9: Filesystem, Drivers, SMP, Userspace (x86 full path)
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        // Phase 7e: Filesystem
        {
            use drivers::storage::blockdev;
            use fs::partition::PartitionType;

            blockdev::register_device(blockdev::BlockDevice {
                id: 0, disk_id: 0, partition: None, start_lba: 0, size_sectors: 0,
            });
            blockdev::scan_and_register_partitions(0);

            let devices = blockdev::list_devices();
            let mut found_root_lba = false;
            for dev in &devices {
                if dev.disk_id == 0 && dev.partition.is_some() {
                    serial_println!("  Partition hd0p{}: start_lba={}", dev.partition.unwrap() + 1, dev.start_lba);
                    if !found_root_lba {
                        fs::vfs::set_root_partition_lba(dev.start_lba as u32);
                        found_root_lba = true;
                    }
                }
            }
            if !found_root_lba {
                serial_println!("  No partition table found, using default LBA 8192");
            }
        }

        fs::vfs::init();
        fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);
        fs::vfs::mount_devfs();

        if drivers::storage::atapi::is_present() && drivers::storage::atapi::capacity_lba() > 0 {
            if fs::vfs::has_root_fs() {
                fs::vfs::mount("/mnt/cdrom0", fs::vfs::FsType::Iso9660, 0);
            } else {
                serial_println!("  No disk filesystem detected, using ISO 9660 as root filesystem");
                fs::vfs::mount("/", fs::vfs::FsType::Iso9660, 0);
            }
        }

        drivers::kdrv::probe_external_drivers();
        task::users::init();
        net::load_config_files();

        // Phase 8: Input devices
        drivers::input::mouse::init();
        drivers::input::vmmouse::init();

        ipc::event_bus::system_emit(ipc::event_bus::EventData::new(
            ipc::event_bus::EVT_BOOT_COMPLETE, 0, 0, 0, 0,
        ));

        // Phase 8b: Start Application Processors (SMP)
        if let Some(ref info) = acpi_info {
            if info.processors.len() > 1 {
                arch::x86::smp::start_aps(&info.processors);
            }
        }

        let nogui = NOGUI.load(AtomicOrdering::Relaxed);

        // Phase 8c: Load shared DLIBs (only needed for GUI mode)
        if !nogui {
            const DLLS: [(&str, u64); 4] = [
                ("/Libraries/uisys.dlib", 0x0400_0000u64),
                ("/Libraries/libimage.dlib", 0x0410_0000u64),
                ("/Libraries/librender.dlib", 0x0430_0000u64),
                ("/Libraries/libcompositor.dlib", 0x0438_0000u64),
            ];
            for (path, base) in DLLS {
                let name = path.rsplit('/').next().unwrap_or(path);
                match task::dll::load_dll(path, base) {
                    Ok(pages) => serial_println!("[OK] {}: {} pages", name, pages),
                    Err(e) => serial_println!("[WARN] {} not loaded: {}", name, e),
                }
            }
        }

        // Phase 9: Start userspace
        if let Some(fb) = drivers::framebuffer::info() {
            if !drivers::gpu::is_available() {
                drivers::gpu::bochs_vga::init(fb.addr as u32, fb.width, fb.height, fb.pitch);
            }
            if let Some(name) = drivers::gpu::with_gpu(|g| {
                let mut n = alloc::string::String::new();
                n.push_str(g.name());
                n
            }) {
                serial_println!("[OK] GPU driver: {}", name);
            }
            // Apply preferred resolution from boot params if set
            if let Some((w, h)) = drivers::gpu::preferred_resolution() {
                if drivers::gpu::with_gpu(|g| g.set_mode(w, h, 32)).is_some() {
                    serial_println!("[OK] Applied boot resolution: {}x{}", w, h);
                }
            }

            let has_accel = drivers::gpu::with_gpu(|g| g.has_accel()).unwrap_or(false);
            if has_accel {
                GPU_ACCEL.store(true, AtomicOrdering::Relaxed);
            }

            arch::hal::disable_interrupts();
            task::scheduler::spawn(task::cpu_monitor::start, 10, "cpu_monitor");
            task::scheduler::spawn(drivers::usb::poll_thread, 50, "usb_poll");

            drivers::boot_console::stop_spinner();

            if nogui {
                // --- No-GUI / Textmode path ---
                // Initialise the framebuffer text console.
                drivers::textcon::init();

                // Clear the framebuffer (remove boot splash) and flush.
                #[cfg(target_arch = "x86_64")]
                if let Some((w, h, _, _)) = drivers::gpu::with_gpu(|g| g.get_mode()) {
                    drivers::gpu::with_gpu(|g| {
                        g.transfer_rect(0, 0, w, h);
                        g.flush_display(0, 0, w, h);
                    });
                }
                serial_println!("[OK] NOGUI mode: skipping compositor/init");
                match task::loader::load_and_run("/System/bin/textmode_console", "textmode_console") {
                    Ok(tid) => serial_println!("[OK] textmode_console spawned (TID={})", tid),
                    Err(e) => serial_println!("  WARN: Failed to load textmode_console: {}", e),
                }
            } else {
                // --- Normal GUI path ---
                let has_hw_cursor = drivers::gpu::with_gpu(|g| g.has_hw_cursor()).unwrap_or(false);
                if has_hw_cursor {
                    GPU_HW_CURSOR.store(true, AtomicOrdering::Relaxed);
                    drivers::gpu::enable_splash_cursor(fb.width, fb.height);
                }

                // Flush the current framebuffer content (boot splash with spinner cleared)
                // to the display before the compositor starts.  This has two benefits:
                //  1. The display always shows *something* during the brief gap between
                //     stop_spinner() and the compositor's first compose+flush.
                //  2. It warm-starts the VirtIO GPU command path: QEMU processes a
                //     transfer+flush here so its virtio-gpu iothread is awake and ready
                //     by the time the compositor fires its own commands.  Without this,
                //     the compositor's very first GPU kick can arrive while QEMU is still
                //     busy with block/net I/O from earlier boot phases, causing a timeout
                //     and virtqueue de-synchronisation that results in a black screen.
                #[cfg(target_arch = "x86_64")]
                if let Some((w, h, _, _)) = drivers::gpu::with_gpu(|g| g.get_mode()) {
                    drivers::gpu::with_gpu(|g| {
                        g.transfer_rect(0, 0, w, h);
                        g.flush_display(0, 0, w, h);
                    });
                    serial_println!("[OK] Pre-compositor GPU flush ({}x{})", w, h);
                }

                match task::loader::load_and_run("/System/compositor/compositor", "compositor") {
                    Ok(tid) => serial_println!("[OK] Userspace compositor spawned (TID={})", tid),
                    Err(e) => serial_println!("  WARN: Failed to load compositor: {}", e),
                }
                match task::loader::load_and_run("/System/init", "init") {
                    Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
                    Err(e) => serial_println!("  WARN: Failed to load /System/init: {}", e),
                }
                serial_println!("Userspace compositor and init spawned, entering scheduler...");
            }

            task::scheduler::run();
        }

        serial_println!("FATAL: No framebuffer available, cannot start userspace.");
    }

    // Fallback idle loop
    #[allow(unreachable_code)]
    loop { arch::hal::halt(); }
}

