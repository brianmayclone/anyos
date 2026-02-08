#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(dead_code, static_mut_refs)]

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

#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: u32) -> ! {
    // Phase 1: Early output (serial only — silent boot for end users)
    drivers::serial::init();
    serial_println!("");
    serial_println!("==============================");
    serial_println!("  .anyOS Kernel v0.1");
    serial_println!("==============================");

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

    arch::x86::pit::init(100); // 100 Hz timer
    serial_println!("[OK] PIT configured at 100 Hz");

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
    drivers::ata::init();
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
    if drivers::e1000::init() {
        net::init();
    }

    // Phase 6: Subsystems
    fs::vfs::init();
    fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);

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
    drivers::mouse::init();

    serial_println!("");
    serial_println!(".anyOS initialization complete.");

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

    // Phase 8c: Test user-mode program execution
    serial_println!("");
    serial_println!("--- User-mode self-test ---");
    for prog in &[("hello", "/bin/hello"), ("hello_rust", "/bin/hello_rust")] {
        serial_println!("  [{}]", prog.0);
        match task::loader::load_and_run(prog.1, prog.0) {
            Ok(tid) => {
                serial_println!("  Spawned user program (TID={}), waiting...", tid);
                let exit_code = task::scheduler::waitpid(tid);
                serial_println!("  Exited with code {}", exit_code);
            }
            Err(e) => {
                serial_println!("  Failed to load {}: {}", prog.1, e);
            }
        }
    }
    serial_println!("--- End self-test ---");
    serial_println!("");

    // Phase 9: Start graphical desktop if framebuffer is available
    if let Some(fb) = drivers::framebuffer::info() {
        serial_println!("Starting graphical desktop ({}x{})...", fb.width, fb.height);

        // Initialize Bochs VGA double-buffering
        drivers::bochs_vga::init(fb.addr, fb.width, fb.height, fb.pitch);

        // Initialize global desktop
        ui::desktop::init(fb.width, fb.height, fb.addr, fb.pitch);

        // Set up compositor
        ui::desktop::with_desktop(|desktop| {
            // Enable hardware double-buffering if available
            if drivers::bochs_vga::is_double_buffered() {
                desktop.enable_hw_double_buffer();
                serial_println!("[OK] Hardware double-buffering enabled");
            }

            // Initial full compose + present
            desktop.invalidate();
            desktop.update();
        });

        // Disable interrupts while spawning threads to prevent the compositor
        // from being scheduled before the terminal is loaded.
        unsafe { core::arch::asm!("cli"); }

        // Spawn compositor as a scheduled kernel task (priority 200 = high)
        task::scheduler::spawn(ui::desktop::desktop_task_entry, 200, "compositor");

        // Launch dock FIRST (always-on-top, so it stays above other windows)
        match task::loader::load_and_run("/system/compositor/dock", "dock") {
            Ok(tid) => serial_println!("[OK] Dock launched (TID {})", tid),
            Err(e) => serial_println!("[WARN] Failed to launch dock: {}", e),
        }

        // Launch terminal AFTER compositor so the event loop is running
        match task::loader::load_and_run("/system/terminal", "terminal") {
            Ok(tid) => serial_println!("[OK] Terminal launched (TID {})", tid),
            Err(e) => serial_println!("[WARN] Failed to launch terminal: {}", e),
        }

        serial_println!("Compositor, dock and terminal spawned, entering scheduler...");
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
    crate::task::scheduler::schedule();
}

/// LAPIC timer IRQ 16: scheduling only (no tick counting).
fn irq_lapic_timer(_irq: u8) {
    crate::task::scheduler::schedule();
}

fn irq_keyboard(_irq: u8) {
    let scancode = unsafe { crate::arch::x86::port::inb(0x60) };
    crate::drivers::keyboard::handle_scancode(scancode);
}

fn irq_mouse(_irq: u8) {
    let byte = unsafe { crate::arch::x86::port::inb(0x60) };
    crate::drivers::mouse::handle_byte(byte);
}
