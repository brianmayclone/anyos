//! VESA VBE framebuffer information storage.
//!
//! Captures framebuffer address, dimensions, pitch, and bit depth from the
//! boot info structure. Used by the compositor and GPU drivers for display setup.

use crate::boot_info::BootInfo;

/// Framebuffer information from VESA VBE mode
pub struct FramebufferInfo {
    pub addr: u32,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
}

static mut FB_INFO: Option<FramebufferInfo> = None;

/// Initialize framebuffer from boot info
pub fn init(boot_info: &BootInfo) {
    let addr = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_addr).read_unaligned() };
    let pitch = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_pitch).read_unaligned() };
    let width = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_width).read_unaligned() };
    let height = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_height).read_unaligned() };
    let bpp = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_bpp).read_unaligned() };

    if addr == 0 || width == 0 || height == 0 {
        crate::serial_println!("  Framebuffer: not available (no VESA mode set)");
        return;
    }

    unsafe {
        FB_INFO = Some(FramebufferInfo {
            addr,
            pitch,
            width,
            height,
            bpp,
        });
    }

    crate::serial_println!(
        "[OK] Framebuffer: {}x{}x{} at {:#010x}, pitch={}",
        width, height, bpp, addr, pitch
    );
}

/// Get framebuffer info (returns None if not initialized or no VESA mode)
pub fn info() -> Option<&'static FramebufferInfo> {
    unsafe { FB_INFO.as_ref() }
}

/// Check if framebuffer is available
pub fn is_available() -> bool {
    unsafe { FB_INFO.is_some() }
}
