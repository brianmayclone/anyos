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
libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

/// Dummy entry point (never called — DLL has no entry).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    syscall::write(1, b"PANIC [libimage]: ");
    if let Some(loc) = info.location() {
        syscall::write(1, loc.file().as_bytes());
        syscall::write(1, b":");
        let mut buf = [0u8; 10];
        let s = fmt_u32(loc.line(), &mut buf);
        syscall::write(1, s);
    }
    syscall::write(1, b"\n");
    syscall::exit(1);
}

/// Format a u32 as decimal into a buffer, returning the used slice.
fn fmt_u32(mut val: u32, buf: &mut [u8; 10]) -> &[u8] {
    if val == 0 {
        buf[9] = b'0';
        return &buf[9..10];
    }
    let mut i = 10;
    while val > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    &buf[i..10]
}
