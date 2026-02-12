//! Kernel panic and allocation error handlers.
//!
//! Displays panic information on serial, framebuffer, and VGA text outputs,
//! then halts the CPU.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Enter panic mode: halt other CPUs, switch to blocking serial,
    // force-release serial lock to prevent deadlock.
    crate::drivers::serial::enter_panic_mode();

    crate::serial_println!("=== KERNEL PANIC ===");
    crate::serial_println!("{}", info);

    // Show Red Screen of Death on the framebuffer
    crate::drivers::rsod::show_panic(&format_args!("{}", info));

    loop {
        unsafe { core::arch::asm!("cli; hlt"); }
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation failed: {:?}", layout);
}
