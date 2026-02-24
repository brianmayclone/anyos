// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]

extern crate alloc;

pub mod types;
pub mod exports;
pub mod bmp;
pub mod png;
pub mod deflate;
pub mod jpeg;
pub mod jpeg_tables;
pub mod gif;
pub mod ico;
pub mod lzw;
pub mod video;
pub mod scale;
pub mod iconpack;
pub mod svg_raster;
mod syscall;
mod heap;

/// Dummy entry point (never called — DLL has no entry).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}
