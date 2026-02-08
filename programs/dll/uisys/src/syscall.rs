//! Minimal syscall wrappers for DLL use (int 0x80).

use core::arch::asm;

const SYS_WIN_FILL_RECT: u32 = 54;
const SYS_WIN_DRAW_TEXT: u32 = 55;
const SYS_WIN_DRAW_TEXT_MONO: u32 = 58;

#[inline(always)]
fn syscall2(num: u32, a1: u32, a2: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!("int 0x80",
            inlateout("eax") num => ret,
            in("ebx") a1, in("ecx") a2,
            lateout("edx") _,
        );
    }
    ret
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
    syscall2(SYS_WIN_FILL_RECT, win, params.as_ptr() as u32);
}

/// Draw proportional text (Cape Coral font) in a window.
/// params: [x:i16, y:i16, color:u32, text_ptr:u32] = 12 bytes
#[inline(always)]
pub fn win_draw_text(win: u32, x: i32, y: i32, color: u32, text: *const u8) {
    let params: [u8; 12] = unsafe {
        let mut p = [0u8; 12];
        let px = x as i16;
        let py = y as i16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        let tp = text as u32;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 4);
        p
    };
    syscall2(SYS_WIN_DRAW_TEXT, win, params.as_ptr() as u32);
}

/// Draw monospace text (8x16 bitmap font) in a window.
#[inline(always)]
pub fn win_draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: *const u8) {
    let params: [u8; 12] = unsafe {
        let mut p = [0u8; 12];
        let px = x as i16;
        let py = y as i16;
        core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        let tp = text as u32;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 4);
        p
    };
    syscall2(SYS_WIN_DRAW_TEXT_MONO, win, params.as_ptr() as u32);
}
