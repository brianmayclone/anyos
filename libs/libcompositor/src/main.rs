#![no_std]
#![no_main]

mod syscall;
mod exports;

/// Dummy entry point (DLL has no entry — code is called via export table).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
