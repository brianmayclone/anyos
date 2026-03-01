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
pub mod sched_diag;
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
/// On x86-64: receives the physical address of the [`BootInfo`] struct.
/// On AArch64: receives the DTB physical address from the bootloader.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: u64) -> ! {
    // =========================================================================
    // Phase 1: Early output
    // =========================================================================
    #[cfg(target_arch = "x86_64")]
    {
        drivers::serial::init();
        serial_println!("");
        serial_println!("  .anyOS Kernel (x86_64) v{}", env!("ANYOS_VERSION"));
        drivers::vga_text::init();
    }
    #[cfg(target_arch = "aarch64")]
    {
        drivers::serial::init();
        serial_println!("");
        serial_println!("  .anyOS Kernel (AArch64)");
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
        boot_info
    };
    #[cfg(target_arch = "aarch64")]
    {
        arch::arm64::boot::save_dtb_addr(boot_info_addr);
        serial_println!("DTB at {:#018x}", boot_info_addr);
    }

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
    #[cfg(target_arch = "aarch64")]
    {
        arch::arm64::exceptions::init();
        serial_println!("[OK] Exception vectors installed (VBAR_EL1)");

        arch::arm64::cpu_features::detect();

        arch::arm64::gic::init_distributor();
        let cpu = arch::arm64::smp::current_cpu_id();
        arch::arm64::gic::init_cpu(cpu);
        serial_println!("[OK] GICv3 initialized (CPU {})", cpu);

        arch::arm64::generic_timer::init();

        arch::arm64::smp::init_bsp();
        serial_println!("[OK] BSP initialized (CPU {})", cpu);

        arch::arm64::syscall::init_bsp();
        arch::arm64::power::init();
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
    #[cfg(target_arch = "aarch64")]
    {
        arch::arm64::mmu::init();
        serial_println!("[OK] MMU configured (TCR_EL1 + MAIR_EL1)");

        let (ram_base, ram_size) = arch::arm64::boot::detect_memory();
        memory::physical::init_arm64(ram_base, ram_size);
        serial_println!("[OK] Physical frame allocator initialized");
    }
    memory::heap::init();
    serial_println!("[OK] Heap allocator initialized");

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

        if acpi_info.is_some() {
            arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler);
            arch::x86::irq::register_irq(16, arch::x86::apic::timer_irq_handler);
            arch::x86::ioapic::unmask_irq(0);
            arch::x86::ioapic::unmask_irq(1);
            arch::x86::ioapic::unmask_irq(12);
        } else {
            arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler_with_schedule);
            arch::x86::pic::unmask(0);
            arch::x86::pic::unmask(1);
            arch::x86::pic::unmask(12);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // Timer IRQ (PPI 30 = physical timer) with schedule tick
        arch::arm64::exceptions::register_irq(30, arch::arm64::generic_timer::irq_handler_with_schedule);
        serial_println!("[OK] Timer IRQ registered (PPI 30)");

        // HAL legacy devices (serial port)
        drivers::hal::init();
        drivers::hal::register_legacy_devices();
        drivers::hal::print_devices();
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

        // Phase 8c: Load shared DLIBs
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

            let has_accel = drivers::gpu::with_gpu(|g| g.has_accel()).unwrap_or(false);
            if has_accel {
                GPU_ACCEL.store(true, AtomicOrdering::Relaxed);
            }
            let has_hw_cursor = drivers::gpu::with_gpu(|g| g.has_hw_cursor()).unwrap_or(false);
            if has_hw_cursor {
                GPU_HW_CURSOR.store(true, AtomicOrdering::Relaxed);
                drivers::gpu::enable_splash_cursor(fb.width, fb.height);
            }

            arch::hal::disable_interrupts();
            task::scheduler::spawn(task::cpu_monitor::start, 10, "cpu_monitor");
            task::scheduler::spawn(drivers::usb::poll_thread, 50, "usb_poll");
            #[cfg(feature = "debug_verbose")]
            task::scheduler::spawn(task::stress_test::stress_master, 30, "stress");
            drivers::boot_console::stop_spinner();

            match task::loader::load_and_run("/System/compositor/compositor", "compositor") {
                Ok(tid) => serial_println!("[OK] Userspace compositor spawned (TID={})", tid),
                Err(e) => serial_println!("  WARN: Failed to load compositor: {}", e),
            }
            match task::loader::load_and_run("/System/init", "init") {
                Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
                Err(e) => serial_println!("  WARN: Failed to load /System/init: {}", e),
            }

            serial_println!("Userspace compositor and init spawned, entering scheduler...");
            task::scheduler::run();
        }

        serial_println!("FATAL: No framebuffer available, cannot start compositor.");
    }

    // =========================================================================
    // ARM64: Full init — VirtIO devices, filesystem, userspace
    // =========================================================================
    #[cfg(target_arch = "aarch64")]
    {
        // Phase 5: VirtIO MMIO Device Discovery
        serial_println!("");
        serial_println!("  [Phase 5] ARM64 VirtIO device discovery...");
        let virtio_devices = drivers::arm::probe_all();
        serial_println!("  Found {} VirtIO MMIO device(s)", virtio_devices.len());

        for dev in &virtio_devices {
            match dev.device_id() {
                2 => {
                    drivers::arm::blk::init(dev);
                }
                16 => {
                    drivers::arm::gpu::init(dev);
                }
                18 => {
                    drivers::arm::input::init(dev);
                }
                id => {
                    serial_println!("  VirtIO device ID {} (not handled)", id);
                }
            }
        }

        // Show boot logo on framebuffer (if GPU initialized)
        drivers::boot_console::init();
        // VirtIO GPU requires explicit flush to transfer pixels to host display
        if let Some((_, w, h)) = drivers::arm::gpu::framebuffer_info() {
            drivers::arm::gpu::flush(0, 0, w, h);
        }

        // Phase 7e: Filesystem
        serial_println!("");
        serial_println!("  [Phase 7e] ARM64 filesystem init...");
        drivers::arm::storage::init_filesystem();

        // Phase 8c: Load shared DLIBs (if filesystem is mounted)
        if fs::vfs::has_root_fs() {
            serial_println!("");
            serial_println!("  [Phase 8c] Loading shared libraries...");
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

            // Phase 9: Start userspace
            serial_println!("");
            serial_println!("  [Phase 9] Starting userspace...");

            arch::hal::disable_interrupts();

            match task::loader::load_and_run("/System/compositor/compositor", "compositor") {
                Ok(tid) => serial_println!("[OK] Compositor spawned (TID={})", tid),
                Err(e) => serial_println!("[WARN] Failed to load compositor: {}", e),
            }
            match task::loader::load_and_run("/System/init", "init") {
                Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
                Err(e) => serial_println!("[WARN] Failed to load init: {}", e),
            }
        }

        let fs_ok = if fs::vfs::has_root_fs() { "mounted" } else { "not available" };
        serial_println!("");
        serial_println!("========================================");
        serial_println!("  anyOS ARM64 boot complete");
        serial_println!("  Filesystem: {}", fs_ok);
        serial_println!("  Display: {}x{}",
            drivers::arm::gpu::framebuffer_info().map_or(0, |f| f.1),
            drivers::arm::gpu::framebuffer_info().map_or(0, |f| f.2));
        serial_println!("  Input: {} device(s)", drivers::arm::input::device_count());
        serial_println!("  Entering scheduler...");
        serial_println!("========================================");
        serial_println!("");
        task::scheduler::run();
    }

    // Fallback idle loop
    loop { arch::hal::halt(); }
}

