//! Kernel panic and allocation error handlers.
//!
//! Displays panic information on serial, framebuffer, and VGA text outputs,
//! then halts the CPU.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("cli"); }

    crate::serial_println!("=== KERNEL PANIC ===");
    crate::serial_println!("{}", info);

    // Switch framebuffer to error display
    crate::drivers::boot_console::enter_error_mode();
    {
        use core::fmt::Write;
        let _ = write!(crate::drivers::boot_console::ErrorWriter, "KERNEL PANIC\n\n{}\n", info);
    }

    crate::drivers::vga_text::set_color(
        crate::drivers::vga_text::Color::White,
        crate::drivers::vga_text::Color::Red,
    );
    crate::vga_println!("KERNEL PANIC: {}", info);

    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation failed: {:?}", layout);
}
