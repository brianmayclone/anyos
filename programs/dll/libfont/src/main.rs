// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libfont.dll — Font and extended drawing syscall wrappers.
//!
//! Provides font loading/unloading, text measurement, extended text drawing,
//! rounded rectangle fills, and GPU acceleration queries via kernel syscalls.

#![no_std]
#![no_main]

use core::arch::asm;

// ── Syscall numbers ──────────────────────────────────────

const SYS_FONT_LOAD: u32 = 130;
const SYS_FONT_UNLOAD: u32 = 131;
const SYS_FONT_MEASURE: u32 = 132;
const SYS_WIN_DRAW_TEXT_EX: u32 = 133;
const SYS_WIN_FILL_ROUNDED_RECT: u32 = 134;
const SYS_GPU_HAS_ACCEL: u32 = 135;

// ── Syscall helpers (SYSCALL instruction, x86-64) ────────
//
// Convention (matches kernel syscall_fast_entry):
//   RAX = syscall number, RBX = arg1, R10 = arg2
//   Return in RAX.  Clobbers: RCX (← user RIP), R11 (← user RFLAGS).

#[inline(always)]
fn syscall0(num: u32) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
fn syscall2(num: u32, a1: u64, a2: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

// ── Export struct ─────────────────────────────────────────

const NUM_EXPORTS: u32 = 6;

/// Export function table — must be `#[repr(C)]` and placed in `.exports` section.
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset  0: magic [u8; 4]
///   offset  4: version u32
///   offset  8: num_exports u32
///   offset 12: _pad u32
///   offset 16: font_load fn ptr (8 bytes)
///   offset 24: font_unload fn ptr (8 bytes)
///   offset 32: font_measure fn ptr (8 bytes)
///   offset 40: win_draw_text_ex fn ptr (8 bytes)
///   offset 48: win_fill_rounded_rect fn ptr (8 bytes)
///   offset 56: gpu_has_accel fn ptr (8 bytes)
#[repr(C)]
pub struct LibfontExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    pub font_load: extern "C" fn(*const u8, u32) -> u32,
    pub font_unload: extern "C" fn(u32) -> u32,
    pub font_measure: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32) -> u32,
    pub win_draw_text_ex: extern "C" fn(u32, i32, i32, u32, u32, u16, *const u8, u32) -> u32,
    pub win_fill_rounded_rect: extern "C" fn(u32, i32, i32, u32, u32, u32, u32) -> u32,
    pub gpu_has_accel: extern "C" fn() -> u32,
}

#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static LIBFONT_EXPORTS: LibfontExports = LibfontExports {
    magic: *b"DLIB",
    version: 1,
    num_exports: NUM_EXPORTS,
    _pad: 0,
    font_load: font_load_export,
    font_unload: font_unload_export,
    font_measure: font_measure_export,
    win_draw_text_ex: win_draw_text_ex_export,
    win_fill_rounded_rect: win_fill_rounded_rect_export,
    gpu_has_accel: gpu_has_accel_export,
};

// ── Export implementations ───────────────────────────────

/// Load a font file and return a font_id, or 0 on failure.
/// Syscall: SYS_FONT_LOAD(path_ptr, path_len) -> font_id
extern "C" fn font_load_export(path_ptr: *const u8, path_len: u32) -> u32 {
    syscall2(SYS_FONT_LOAD, path_ptr as u64, path_len as u64)
}

/// Unload a previously loaded font.
/// Syscall: SYS_FONT_UNLOAD(font_id) -> 0
extern "C" fn font_unload_export(font_id: u32) -> u32 {
    syscall2(SYS_FONT_UNLOAD, font_id as u64, 0)
}

/// Measure text dimensions for a given font and size.
/// Syscall: SYS_FONT_MEASURE(params_ptr) -> 0
/// params: [font_id:u16, size:u16, text_ptr:u32, text_len:u32, out_w:u32, out_h:u32]
extern "C" fn font_measure_export(
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    text_len: u32,
    out_w: *mut u32,
    out_h: *mut u32,
) -> u32 {
    let params: [u8; 20] = unsafe {
        let mut p = [0u8; 20];
        let fid = font_id as u16;
        core::ptr::copy_nonoverlapping(fid.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(size.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        let tp = text_ptr as u32;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        core::ptr::copy_nonoverlapping(text_len.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 4);
        let ow = out_w as u32;
        core::ptr::copy_nonoverlapping(ow.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
        let oh = out_h as u32;
        core::ptr::copy_nonoverlapping(oh.to_le_bytes().as_ptr(), p.as_mut_ptr().add(16), 4);
        p
    };
    syscall2(SYS_FONT_MEASURE, params.as_ptr() as u64, 0)
}

/// Draw text with a loaded font at a given size and color.
/// Syscall: SYS_WIN_DRAW_TEXT_EX(win_id, params_ptr) -> 0
/// params: [x:i16, y:i16, color:u32, font_id:u16, size:u16, text_ptr:u32]
extern "C" fn win_draw_text_ex_export(
    win_id: u32,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    _text_len: u32,
) -> u32 {
    let params: [u8; 16] = unsafe {
        let mut p = [0u8; 16];
        let px = x as i16;
        let py = y as i16;
        let fid = font_id as u16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        core::ptr::copy_nonoverlapping(fid.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 2);
        core::ptr::copy_nonoverlapping(size.to_le_bytes().as_ptr(), p.as_mut_ptr().add(10), 2);
        let tp = text_ptr as u32;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
        p
    };
    syscall2(SYS_WIN_DRAW_TEXT_EX, win_id as u64, params.as_ptr() as u64)
}

/// Fill a rounded rectangle in a window.
/// Syscall: SYS_WIN_FILL_ROUNDED_RECT(win_id, params_ptr) -> 0
/// params: [x:i16, y:i16, w:u16, h:u16, radius:u16, pad:u16, color:u32]
extern "C" fn win_fill_rounded_rect_export(
    win_id: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    color: u32,
) -> u32 {
    let params: [u8; 16] = unsafe {
        let mut p = [0u8; 16];
        let px = x as i16;
        let py = y as i16;
        let pw = w as u16;
        let ph = h as u16;
        let pr = radius as u16;
        let pad: u16 = 0;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(pw.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 2);
        core::ptr::copy_nonoverlapping(ph.to_le_bytes().as_ptr(), p.as_mut_ptr().add(6), 2);
        core::ptr::copy_nonoverlapping(pr.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 2);
        core::ptr::copy_nonoverlapping(pad.to_le_bytes().as_ptr(), p.as_mut_ptr().add(10), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
        p
    };
    syscall2(SYS_WIN_FILL_ROUNDED_RECT, win_id as u64, params.as_ptr() as u64)
}

/// Query whether GPU acceleration is available.
/// Syscall: SYS_GPU_HAS_ACCEL() -> 0/1
extern "C" fn gpu_has_accel_export() -> u32 {
    syscall0(SYS_GPU_HAS_ACCEL)
}

// ── Entry / panic ────────────────────────────────────────

/// Dummy entry point (never called — DLL has no entry).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
