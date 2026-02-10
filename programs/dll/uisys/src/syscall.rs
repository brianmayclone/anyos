//! Minimal syscall wrappers for DLL use (SYSCALL instruction, x86-64).
//!
//! Convention (matches kernel syscall_fast_entry):
//!   RAX = syscall number, RBX = arg1, R10 = arg2, RDX = arg3
//!   Return in RAX.  Clobbers: RCX (← user RIP), R11 (← user RFLAGS).

use core::arch::asm;

const SYS_WIN_FILL_RECT: u32 = 54;
const SYS_WIN_DRAW_TEXT: u32 = 55;
const SYS_WIN_DRAW_TEXT_MONO: u32 = 58;
const SYS_FONT_MEASURE: u32 = 132;
const SYS_WIN_DRAW_TEXT_EX: u32 = 133;
const SYS_WIN_FILL_ROUNDED_RECT: u32 = 134;
const SYS_GPU_HAS_ACCEL: u32 = 135;
const SYS_FONT_RENDER_BUF: u32 = 160;

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
fn syscall1(num: u32, a1: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
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

/// Fill a rectangle in a window.
/// params: [x:i16, y:i16, w:u16, h:u16, color:u32] = 12 bytes
#[inline(always)]
pub fn win_fill_rect(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let params: [u8; 12] = unsafe {
        let mut p = [0u8; 12];
        let px = x as i16;
        let py = y as i16;
        let pw = w as u16;
        let ph = h as u16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(pw.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 2);
        core::ptr::copy_nonoverlapping(ph.to_le_bytes().as_ptr(), p.as_mut_ptr().add(6), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 4);
        p
    };
    syscall2(SYS_WIN_FILL_RECT, win as u64, params.as_ptr() as u64);
}

/// Draw proportional text (Cape Coral font) in a window.
/// params: [x:i16, y:i16, color:u32, text_ptr:u64] = 16 bytes
#[inline(always)]
pub fn win_draw_text(win: u32, x: i32, y: i32, color: u32, text: *const u8) {
    let params: [u8; 16] = unsafe {
        let mut p = [0u8; 16];
        let px = x as i16;
        let py = y as i16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        let tp = text as u64;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 8);
        p
    };
    syscall2(SYS_WIN_DRAW_TEXT, win as u64, params.as_ptr() as u64);
}

/// Draw monospace text (8x16 bitmap font) in a window.
#[inline(always)]
pub fn win_draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: *const u8) {
    let params: [u8; 16] = unsafe {
        let mut p = [0u8; 16];
        let px = x as i16;
        let py = y as i16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        let tp = text as u64;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 8);
        p
    };
    syscall2(SYS_WIN_DRAW_TEXT_MONO, win as u64, params.as_ptr() as u64);
}

/// Fill a rounded rectangle via kernel AA syscall.
/// params: [x:i16, y:i16, w:u16, h:u16, radius:u16, _pad:u16, color:u32] = 16 bytes
#[inline(always)]
pub fn win_fill_rounded_rect(win_id: u32, params_ptr: u32) -> u32 {
    syscall2(SYS_WIN_FILL_ROUNDED_RECT, win_id as u64, params_ptr as u64)
}

/// Draw text with explicit font selection.
/// params: [x:i16, y:i16, color:u32, font_id:u16, size:u16, text_ptr:u32] = 16 bytes
#[inline(always)]
pub fn win_draw_text_ex(win_id: u32, params_ptr: u32) -> u32 {
    syscall2(SYS_WIN_DRAW_TEXT_EX, win_id as u64, params_ptr as u64)
}

/// Measure text extent with a specific font.
/// params: [font_id:u16, size:u16, text_ptr:u32, text_len:u32, out_w_ptr:u32, out_h_ptr:u32] = 20 bytes
#[inline(always)]
pub fn font_measure(params_ptr: u32) -> u32 {
    syscall1(SYS_FONT_MEASURE, params_ptr as u64)
}

/// Render TTF text to a user pixel buffer.
/// params: 36-byte packed struct (see SYS_FONT_RENDER_BUF definition).
#[inline(always)]
pub fn font_render_buf(params_ptr: u32) -> u32 {
    syscall1(SYS_FONT_RENDER_BUF, params_ptr as u64)
}

/// Query GPU acceleration availability.
#[inline(always)]
pub fn gpu_has_accel() -> u32 {
    syscall0(SYS_GPU_HAS_ACCEL)
}
