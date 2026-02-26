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
mod memory;
mod net;
mod panic;
mod sync;
mod syscall;
mod task;

use boot_info::BootInfo;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering as AtomicOrdering};

/// Boot mode: 0 = Legacy BIOS, 1 = UEFI.
static BOOT_MODE: AtomicU8 = AtomicU8::new(0);

/// GPU 2D acceleration available (queried by SYS_GPU_HAS_ACCEL).
pub static GPU_ACCEL: AtomicBool = AtomicBool::new(false);

/// GPU hardware cursor available (queried by SYS_GPU_HAS_HW_CURSOR).
pub static GPU_HW_CURSOR: AtomicBool = AtomicBool::new(false);

/// Get the boot mode (0 = BIOS, 1 = UEFI).
pub fn boot_mode() -> u8 {
    BOOT_MODE.load(AtomicOrdering::Relaxed)
}

/// Kernel entry point called from assembly after boot.
///
/// Receives the physical address of the [`BootInfo`] struct from the stage 2
/// bootloader and drives initialization through 10 sequential phases.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: u64) -> ! {
    // Phase 1: Early output (serial only — silent boot for end users)
    drivers::serial::init();
    serial_println!("");
    serial_println!("  .anyOS Kernel v0.1");

    drivers::vga_text::init();

    // Validate boot info
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
    serial_println!(
        "Kernel loaded at {:#010x} - {:#010x}",
        kstart, kend
    );

    // Phase 2: CPU setup
    arch::x86::gdt::init();
    serial_println!("[OK] GDT initialized");

    arch::x86::idt::init();
    serial_println!("[OK] IDT initialized (256 entries + syscall int 0x80)");

    arch::x86::tss::init();

    arch::x86::pic::init();
    serial_println!("[OK] PIC remapped (IRQ 0-15 -> INT 32-47)");

    arch::x86::cpuid::detect();
    arch::x86::cpuid::enable_smep();
    // TODO: enable_xsave() causes SIGSEGV in VMware — disabled until investigated
    // arch::x86::cpuid::enable_xsave();

    arch::x86::pit::init();
    serial_println!("[OK] PIT configured at {} Hz", arch::x86::pit::TICK_HZ);

    // Calibrate TSC early using PIT channel 2 polled readback (no IRQs needed).
    // Must be done before LAPIC timer calibration which depends on TSC.
    arch::x86::pit::calibrate_tsc();

    // CPU power management: P-states, C-states, frequency detection
    arch::x86::power::init();

    // Phase 3: Memory
    arch::x86::pat::init(); // Program PAT before mapping framebuffer with WC
    memory::physical::init(boot_info);
    memory::virtual_mem::init(boot_info);
    memory::heap::init();

    // Phase 4: Test heap allocation
    {
        use alloc::vec::Vec;
        let mut v = Vec::new();
        v.push(42u32);
        v.push(43);
        v.push(44);
        serial_println!("Heap test: vec = {:?}", v);
    }

    // Initialize Cape Coral anti-aliased font (requires heap)
    graphics::cc_font::init();

    // Phase 4b: ACPI + APIC (requires heap for Vec, so after memory init)
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

    // Phase 5: Drivers
    drivers::rtc::init();
    drivers::storage::ata::init();
    drivers::storage::atapi::init();
    drivers::framebuffer::init(boot_info);
    drivers::boot_console::init(); // Show boot splash (color logo)

    // Phase 5b: HAL + PCI device enumeration
    // Note: probe_and_bind_all() is deferred to after TSC calibration
    // because USB controller init uses delay_ms() which needs working timers.
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

    // Phase 6: Scheduler (before interrupts, does not need filesystem)
    task::scheduler::init();
    drivers::boot_console::tick_spinner();

    // Phase 7: Register IRQ handlers and enable interrupts
    arch::x86::irq::register_irq(1, drivers::input::keyboard::irq_handler);
    arch::x86::irq::register_irq(12, drivers::input::mouse::irq_handler);

    if acpi_info.is_some() {
        // APIC mode:
        // PIT IRQ 0 → timekeeping only (needed for calibration + uptime)
        // LAPIC timer IRQ 16 → scheduling only
        // Separating them prevents double-counting ticks.
        arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler);
        arch::x86::irq::register_irq(16, arch::x86::apic::timer_irq_handler);
        arch::x86::ioapic::unmask_irq(0);  // PIT (for timekeeping + calibration)
        arch::x86::ioapic::unmask_irq(1);  // Keyboard
        arch::x86::ioapic::unmask_irq(12); // Mouse
    } else {
        // Legacy PIC mode: PIT IRQ 0 does both timekeeping AND scheduling
        arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler_with_schedule);
        arch::x86::pic::unmask(0);  // Timer (IRQ0)
        arch::x86::pic::unmask(1);  // Keyboard (IRQ1)
        arch::x86::pic::unmask(12); // Mouse (IRQ12)
    }
    unsafe { core::arch::asm!("sti"); }
    serial_println!("[OK] Interrupts enabled (timer + keyboard + mouse)");

    // Switch serial output from blocking to async (IRQ 4 driven TX buffer)
    drivers::serial::enable_async();
    serial_println!("[OK] Serial TX now async (IRQ 4)");

    // Phase 7b: Calibrate LAPIC timer using TSC (already calibrated in Phase 2).
    // Uses TSC-based 10ms measurement — no PIT IRQ dependency.
    if acpi_info.is_some() {
        arch::x86::apic::calibrate_timer(1000);
    }

    // Phase 7d: HAL driver binding (deferred from Phase 5b).
    // USB controller init uses delay_ms() which requires working timers
    // (TSC calibrated or PIT IRQs running — both available now).
    drivers::hal::probe_and_bind_all();
    drivers::hal::register_legacy_devices();
    drivers::hal::print_devices();

    // Phase 7e: Filesystem (after HAL probe — AHCI storage driver must be initialized first)

    // Register the boot disk as a block device and scan for MBR/GPT partitions.
    // If a partition table is found, derive the root filesystem LBA from it.
    // Otherwise, fall back to the default hardcoded LBA 8192.
    {
        use drivers::storage::blockdev;
        use fs::partition::PartitionType;

        // Register whole boot disk (disk 0). Size 0 = unknown (whole disk).
        blockdev::register_device(blockdev::BlockDevice {
            id: 0,
            disk_id: 0,
            partition: None,
            start_lba: 0,
            size_sectors: 0, // whole disk, size unknown
        });
        blockdev::scan_and_register_partitions(0);

        // Find the first data partition to use as root filesystem
        let devices = blockdev::list_devices();
        let mut found_root_lba = false;
        for dev in &devices {
            if dev.disk_id == 0 && dev.partition.is_some() {
                // Use the first non-ESP partition as root
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

    // Auto-detect CD-ROM with ISO 9660 filesystem
    if drivers::storage::atapi::is_present() && drivers::storage::atapi::capacity_lba() > 0 {
        if fs::vfs::has_root_fs() {
            // Hard disk + CD-ROM: mount CD at /mnt/cdrom0
            fs::vfs::mount("/mnt/cdrom0", fs::vfs::FsType::Iso9660, 0);
        } else {
            // CD-ROM only (Live CD boot): mount ISO 9660 as root filesystem
            serial_println!("  No disk filesystem detected, using ISO 9660 as root filesystem");
            fs::vfs::mount("/", fs::vfs::FsType::Iso9660, 0);
        }
    }

    // Phase 7f: External driver loading (requires VFS — scans /System/Drivers/)
    // Only loads drivers whose match rules correspond to unbound PCI devices.
    drivers::kdrv::probe_external_drivers();

    // Phase 7g: User database (requires VFS)
    task::users::init();

    // Phase 8: Initialize mouse
    serial_println!("  Phase 8: Initializing mouse...");
    drivers::input::mouse::init();

    // Phase 8a: Try VMware backdoor (vmmouse) for absolute mouse in VMs
    // Must be after PS/2 init — if backdoor is present, IRQ12 will use it instead of PS/2.
    serial_println!("  Phase 8a: Detecting vmmouse...");
    drivers::input::vmmouse::init();

    serial_println!("  Phase 8a done.");

    // Emit boot complete event
    ipc::event_bus::system_emit(ipc::event_bus::EventData::new(
        ipc::event_bus::EVT_BOOT_COMPLETE, 0, 0, 0, 0,
    ));

    // Phase 8b: Start Application Processors (SMP)
    if let Some(ref info) = acpi_info {
        if info.processors.len() > 1 {
            arch::x86::smp::start_aps(&info.processors);
        }
    }

    // Phase 8c: Load shared DLIBs from filesystem
    // Note: libfont is now a .so (loaded on demand via SYS_DLL_LOAD), not a boot-time DLIB.
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

    // Phase 9: Start userspace compositor + init process
    if let Some(fb) = drivers::framebuffer::info() {
        // GPU driver may already be registered via HAL PCI probe (Phase 5b).
        // If not, initialize Bochs VGA as fallback using boot framebuffer info.
        if !drivers::gpu::is_available() {
            serial_println!("  No GPU driver via PCI, using Bochs VGA fallback...");
            drivers::gpu::bochs_vga::init(fb.addr, fb.width, fb.height, fb.pitch);
        }

        // Log GPU info
        if let Some(name) = drivers::gpu::with_gpu(|g| {
            let mut n = alloc::string::String::new();
            n.push_str(g.name());
            n
        }) {
            serial_println!("[OK] GPU driver: {}", name);
        }

        let has_accel = drivers::gpu::with_gpu(|g| g.has_accel()).unwrap_or(false);
        if has_accel {
            GPU_ACCEL.store(true, AtomicOrdering::Relaxed);
            serial_println!("[OK] GPU 2D acceleration enabled");
        }

        let has_hw_cursor = drivers::gpu::with_gpu(|g| g.has_hw_cursor()).unwrap_or(false);
        if has_hw_cursor {
            GPU_HW_CURSOR.store(true, AtomicOrdering::Relaxed);
            drivers::gpu::enable_splash_cursor(fb.width, fb.height);
            serial_println!("[OK] Hardware cursor enabled (splash mode)");
        }

        // Disable interrupts while spawning threads
        unsafe { core::arch::asm!("cli"); }

        // Spawn CPU monitor kernel thread
        task::scheduler::spawn(task::cpu_monitor::start, 10, "cpu_monitor");

        // Spawn USB hot-plug poll thread (low priority)
        task::scheduler::spawn(drivers::usb::poll_thread, 50, "usb_poll");

        // Spawn kernel thread stress test when debug_verbose is enabled
        #[cfg(feature = "debug_verbose")]
        task::scheduler::spawn(task::stress_test::stress_master, 30, "stress");

        // Stop boot spinner — compositor will take over the framebuffer
        drivers::boot_console::stop_spinner();

        // Launch userspace compositor (highest user priority)
        match task::loader::load_and_run("/System/compositor/compositor", "compositor") {
            Ok(tid) => serial_println!("[OK] Userspace compositor spawned (TID={})", tid),
            Err(e) => serial_println!("  WARN: Failed to load compositor: {}", e),
        }

        // Launch init process (runs benchmarks, loads wallpaper, starts programs)
        match task::loader::load_and_run("/System/init", "init") {
            Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
            Err(e) => {
                serial_println!("  WARN: Failed to load /System/init: {}", e);
            }
        }

        serial_println!("Userspace compositor and init spawned, entering scheduler...");
        task::scheduler::run();
    }

    // No framebuffer — cannot start compositor, halt.
    serial_println!("FATAL: No framebuffer available, cannot start compositor.");
    loop { unsafe { core::arch::asm!("hlt"); } }
}

