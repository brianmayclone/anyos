#![no_std]
#![no_main]

pub mod color;
pub mod rect;
pub mod surface;
pub mod renderer;
pub mod exports;

/// Dummy entry point (never called — DLL has no entry).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
