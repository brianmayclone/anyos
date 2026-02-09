#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(dead_code, static_mut_refs)]
//! Kernel entry point and initialization sequence.
//!
//! Initializes all subsystems in 10 phases, from serial output to the desktop environment.

extern crate alloc;

mod apps;
mod arch;
mod boot_info;
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
mod ui;

use boot_info::BootInfo;

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
    arch::x86::syscall_msr::init_bsp();

    arch::x86::pit::init();
    serial_println!("[OK] PIT configured at {} Hz", arch::x86::pit::TICK_HZ);

    // Phase 3: Memory
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
    let acpi_info = arch::x86::acpi::init();
    if let Some(ref info) = acpi_info {
        arch::x86::apic::init_bsp(info.lapic_address);
        arch::x86::ioapic::init(&info.io_apics, &info.isos);
        arch::x86::ioapic::disable_legacy_pic();
        arch::x86::smp::init_bsp();
    } else {
        serial_println!("  ACPI not found, using legacy PIC");
    }

    // Phase 5: Drivers
    drivers::rtc::init();
    drivers::storage::ata::init();
    drivers::framebuffer::init(boot_info);
    drivers::boot_console::init(); // Show boot splash (color logo)

    // Phase 5b: HAL + PCI device enumeration
    drivers::hal::init();
    drivers::pci::scan_all();
    drivers::pci::print_devices();
    drivers::hal::probe_and_bind_all();
    drivers::hal::register_legacy_devices();
    drivers::hal::print_devices();

    // Phase 5c: E1000 NIC + Network Stack
    if drivers::network::e1000::init() {
        net::init();
    }

    // Phase 6: Subsystems
    fs::vfs::init();
    fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);

    // Initialize TTF font manager (loads /system/fonts/system.ttf from disk)
    graphics::font_manager::init();

    task::scheduler::init();

    // Phase 7: Register IRQ handlers and enable interrupts
    arch::x86::irq::register_irq(1, irq_keyboard);
    arch::x86::irq::register_irq(12, irq_mouse);

    if acpi_info.is_some() {
        // APIC mode:
        // PIT IRQ 0 → timekeeping only (needed for calibration + uptime)
        // LAPIC timer IRQ 16 → scheduling only
        // Separating them prevents double-counting ticks.
        arch::x86::irq::register_irq(0, irq_pit_tick);
        arch::x86::irq::register_irq(16, irq_lapic_timer);
        arch::x86::ioapic::unmask_irq(0);  // PIT (for timekeeping + calibration)
        arch::x86::ioapic::unmask_irq(1);  // Keyboard
        arch::x86::ioapic::unmask_irq(12); // Mouse
    } else {
        // Legacy PIC mode: PIT IRQ 0 does both timekeeping AND scheduling
        arch::x86::irq::register_irq(0, irq_pit_tick_and_schedule);
        arch::x86::pic::unmask(0);  // Timer (IRQ0)
        arch::x86::pic::unmask(1);  // Keyboard (IRQ1)
        arch::x86::pic::unmask(12); // Mouse (IRQ12)
    }
    unsafe { core::arch::asm!("sti"); }
    serial_println!("[OK] Interrupts enabled (timer + keyboard + mouse)");

    // Phase 7b: Calibrate LAPIC timer (needs PIT IRQ running, so after sti)
    if acpi_info.is_some() {
        arch::x86::apic::calibrate_timer(100);
    }

    // Phase 8: Initialize mouse
    drivers::input::mouse::init();

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

    // Phase 8c: Load shared DLLs from filesystem
    match task::dll::load_dll("/system/lib/uisys.dll", 0x0400_0000) {
        Ok(pages) => serial_println!("[OK] uisys.dll: {} pages", pages),
        Err(e) => serial_println!("[WARN] uisys.dll not loaded: {}", e),
    }
    match task::dll::load_dll("/system/lib/libimage.dll", 0x0410_0000) {
        Ok(pages) => serial_println!("[OK] libimage.dll: {} pages", pages),
        Err(e) => serial_println!("[WARN] libimage.dll not loaded: {}", e),
    }
    match task::dll::load_dll("/system/lib/libfont.dll", 0x0420_0000) {
        Ok(pages) => serial_println!("[OK] libfont.dll: {} pages", pages),
        Err(e) => serial_println!("[WARN] libfont.dll not loaded: {}", e),
    }

    // Phase 9: Start graphical desktop if framebuffer is available
    // NOTE: Init process is spawned AFTER desktop init (Phase 9d below)
    // so that wallpaper loading and other UI syscalls can reach the compositor.
    if let Some(fb) = drivers::framebuffer::info() {
        // GPU driver may already be registered via HAL PCI probe (Phase 5b).
        // If not, initialize Bochs VGA as fallback using boot framebuffer info.
        if !drivers::gpu::is_available() {
            serial_println!("  No GPU driver via PCI, using Bochs VGA fallback...");
            drivers::gpu::bochs_vga::init(fb.addr, fb.width, fb.height, fb.pitch);
        }

        // Query GPU for mode info, else fall back to boot framebuffer
        let (width, height, pitch, fb_addr) = drivers::gpu::with_gpu(|g| g.get_mode())
            .unwrap_or((fb.width, fb.height, fb.pitch, fb.addr));

        // Initialize global desktop
        ui::desktop::init(width, height, fb_addr, pitch);

        // Configure compositor with GPU capabilities
        ui::desktop::with_desktop(|desktop| {
            let has_dblbuf = drivers::gpu::with_gpu(|g| g.has_double_buffer()).unwrap_or(false);
            let has_hw_cursor = drivers::gpu::with_gpu(|g| g.has_hw_cursor()).unwrap_or(false);
            let has_accel = drivers::gpu::with_gpu(|g| g.has_accel()).unwrap_or(false);

            if let Some(name) = drivers::gpu::with_gpu(|g| {
                let mut n = alloc::string::String::new();
                n.push_str(g.name());
                n
            }) {
                serial_println!("[OK] GPU driver: {}", name);
            }

            if has_dblbuf {
                desktop.enable_hw_double_buffer();
                serial_println!("[OK] Hardware double-buffering enabled");
            }

            if has_hw_cursor {
                desktop.compositor.enable_hw_cursor();
                // Enable boot-splash cursor: IRQ handler updates HW cursor directly
                drivers::gpu::enable_splash_cursor(width, height);
                serial_println!("[OK] Hardware cursor enabled (splash mode)");
            }

            if has_accel {
                desktop.compositor.set_gpu_accel(true);
                graphics::font_manager::set_gpu_accel(true);
                serial_println!("[OK] GPU 2D acceleration enabled");
            }

            // Skip initial compose — boot splash logo stays on framebuffer.
            // The compositor will compose+present after init signals boot ready.
        });

        // Disable interrupts while spawning threads to prevent the compositor
        // from being scheduled before init is loaded.
        unsafe { core::arch::asm!("cli"); }

        // Spawn compositor as a scheduled kernel task (priority 200 = high)
        // It enters boot-splash mode first: only tracks HW cursor, no compositing.
        task::scheduler::spawn(ui::desktop::desktop_task_entry, 200, "compositor");

        // Spawn CPU monitor kernel thread (writes CPU load to sys:cpu_load pipe)
        task::scheduler::spawn(task::cpu_monitor::start, 10, "cpu_monitor");

        // NOTE: Dock is launched by the compositor task AFTER boot splash completes.
        // This ensures the desktop (with wallpaper) appears before the dock.

        // Phase 9d: Run init process (benchmark + wallpaper + boot_ready signal).
        // Runs concurrently — compositor stays in splash mode until init calls boot_ready.
        match task::loader::load_and_run("/system/init", "init") {
            Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
            Err(e) => {
                serial_println!("  WARN: Failed to load /system/init: {}", e);
                serial_println!("  System may not be fully configured.");
                // No init → signal boot ready immediately so desktop still appears
                ui::desktop::signal_boot_ready();
            }
        }

        serial_println!("Compositor and init spawned, entering scheduler...");
        // scheduler::run() re-enables interrupts (sti) and enters the idle loop
        task::scheduler::run();
    }

    // Fallback: text mode shell
    serial_println!("No framebuffer available, starting text terminal.");
    let mut text_terminal = apps::text_terminal::TextTerminal::new();
    text_terminal.run();
}

// IRQ handler functions for dynamic IRQ dispatch

/// PIT IRQ 0 (APIC mode): timekeeping only. LAPIC timer handles scheduling.
fn irq_pit_tick(_irq: u8) {
    crate::arch::x86::pit::tick();
}

/// PIT IRQ 0 (legacy PIC mode): timekeeping AND scheduling (no LAPIC timer).
fn irq_pit_tick_and_schedule(_irq: u8) {
    crate::arch::x86::pit::tick();
    crate::task::scheduler::schedule_tick();
}

/// LAPIC timer IRQ 16: scheduling only (no tick counting).
fn irq_lapic_timer(_irq: u8) {
    crate::task::scheduler::schedule_tick();
}

/// Keyboard IRQ handler: reads scancode from PS/2 port 0x60.
fn irq_keyboard(_irq: u8) {
    let scancode = unsafe { crate::arch::x86::port::inb(0x60) };
    crate::drivers::input::keyboard::handle_scancode(scancode);
}

/// Mouse IRQ handler: reads byte from PS/2 port 0x60.
fn irq_mouse(_irq: u8) {
    let byte = unsafe { crate::arch::x86::port::inb(0x60) };
    crate::drivers::input::mouse::handle_byte(byte);
}
