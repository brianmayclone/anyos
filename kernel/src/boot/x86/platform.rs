//! CPU, memory, interrupt, and platform bring-up for x86_64 boot.

use crate::arch;
use crate::boot_info::BootInfo;
use crate::drivers;
use crate::graphics;
#[cfg(feature = "kunit")]
use crate::kunit;
use crate::memory;
use crate::net;
use crate::serial_println;
use crate::task;

pub(super) fn init_cpu() {
    arch::x86::gdt::init();
    serial_println!("[OK] GDT initialized");

    arch::x86::idt::init();
    serial_println!("[OK] IDT initialized (256 entries)");

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

pub(super) fn init_memory(boot_info: &BootInfo) {
    arch::x86::pat::init();
    // physical::init records usable RAM in the bootmem bitmap only.
    // The buddy backend writes intrusive link words into free pages,
    // which requires physmap, so the real buddy migration happens
    // below after physmap is established.
    memory::physical::init(boot_info);
    memory::virtual_mem::init(boot_info);
    memory::physmap::init(boot_info);
    // late_init migrates bootmem's free frames into the buddy zones now
    // that physmap can safely address every free page.
    memory::physical::late_init(boot_info);
    memory::heap::init();
    serial_println!("[OK] Heap allocator initialized");
}

pub(super) fn run_unit_tests() {
    #[cfg(feature = "kunit")]
    kunit::runner::run_unit_tests();

    graphics::cc_font::init();
}

pub(super) fn init_platform(boot_info: &BootInfo) -> Option<arch::x86::acpi::AcpiInfo> {
    let rsdp_addr = unsafe { core::ptr::addr_of!((*boot_info).rsdp_addr).read_unaligned() };
    let acpi_info = arch::x86::acpi::init(rsdp_addr);

    if let Some(ref info) = acpi_info {
        arch::x86::apic::init_bsp(info.lapic_address);
        arch::x86::ioapic::init(&info.io_apics, &info.isos);
        arch::x86::ioapic::disable_legacy_pic();
        arch::x86::smp::init_bsp();
        arch::x86::smp::register_halt_ipi();
        arch::x86::smp::register_tlb_shootdown_ipi();
        arch::x86::smp::register_resched_ipi();
        arch::x86::syscall_msr::init_bsp();

        for seg in &info.mcfg_segments {
            drivers::pci_msi::set_ecam_base(seg.base_address, seg.start_bus, seg.end_bus);
        }
    } else {
        serial_println!("  ACPI not found, using legacy PIC");
    }

    #[cfg(feature = "kunit")]
    {
        let ctx = kunit::integration::IntegrationCtx {
            acpi_present: acpi_info.is_some(),
            lapic_address: acpi_info.as_ref().map_or(0, |info| info.lapic_address),
            processor_count: acpi_info.as_ref().map_or(0, |info| info.processors.len()),
            ioapic_count: acpi_info.as_ref().map_or(0, |info| info.io_apics.len()),
            iso_count: acpi_info.as_ref().map_or(0, |info| info.isos.len()),
        };
        kunit::integration::set_context(ctx);
    }

    acpi_info
}

pub(super) fn init_virtualization() {
    arch::x86::virt::init();
    arch::x86::virt::per_cpu_init();
}

pub(super) fn init_early_drivers(boot_info: &BootInfo) {
    drivers::rtc::init();
    drivers::storage::ata::init();
    drivers::storage::atapi::init();
    drivers::framebuffer::init(boot_info);
    drivers::boot_console::init();

    drivers::hal::init();
    drivers::boot_console::tick_spinner();
    drivers::pci::scan_all();
    drivers::pci::print_devices();
    drivers::boot_console::tick_spinner();

    let pci_list = drivers::pci::devices();
    drivers::smbus::init(&pci_list);
    drivers::thermal::init(&pci_list);

    let rsdp_addr = unsafe { core::ptr::addr_of!((*boot_info).rsdp_addr).read_unaligned() };
    arch::x86::acpi_pm::init(rsdp_addr);
    drivers::boot_console::tick_spinner();

    // Try E1000 legacy init (pre-PCI-probe path).
    // net::init() is called later, after PCI probe, so it picks up
    // whichever NIC was registered (E1000, VirtIO Net, etc.).
    let _ = drivers::network::e1000::init();
    drivers::boot_console::tick_spinner();
}

pub(super) fn init_scheduler() {
    task::scheduler::init();
    serial_println!("[OK] Scheduler initialized");
    drivers::boot_console::tick_spinner();
}

pub(super) fn enable_interrupts_and_bind(has_acpi: bool) {
    register_input_irqs(has_acpi);
    arch::hal::enable_interrupts();
    serial_println!("[OK] Interrupts enabled");

    drivers::serial::enable_async();
    serial_println!("[OK] Serial TX now async (IRQ 4)");

    if has_acpi {
        arch::x86::apic::calibrate_timer(1000);
    }

    drivers::hal::probe_and_bind_all();
    drivers::hal::register_legacy_devices();
    drivers::hal::print_devices();

    // Initialize network stack AFTER PCI probe — picks up whatever NIC
    // was registered (E1000, VirtIO Net, RTL8125, IGC, etc.)
    if drivers::network::is_available() {
        net::init();
    }
}

fn register_input_irqs(has_acpi: bool) {
    arch::x86::irq::register_irq(1, drivers::input::keyboard::irq_handler);
    arch::x86::irq::register_irq(12, drivers::input::mouse::irq_handler);

    drain_ps2_controller();

    if has_acpi {
        arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler);
        arch::x86::irq::register_irq(16, arch::x86::apic::timer_irq_handler);
        arch::x86::ioapic::unmask_irq(0);
        arch::x86::ioapic::unmask_irq(1);
        arch::x86::ioapic::unmask_irq(12);
    } else {
        arch::x86::irq::register_irq(0, arch::x86::pit::irq_handler_with_schedule);
        drain_ps2_controller();
        arch::x86::pic::unmask(0);
        arch::x86::pic::unmask(1);
        arch::x86::pic::unmask(12);
    }
}

fn drain_ps2_controller() {
    unsafe {
        for _ in 0..64 {
            if arch::x86::port::inb(0x64) & 0x01 == 0 {
                break;
            }
            let _ = arch::x86::port::inb(0x60);
        }
    }
}

pub(super) fn run_integration_tests() {
    #[cfg(feature = "kunit")]
    kunit::runner::run_integration_tests();
}
