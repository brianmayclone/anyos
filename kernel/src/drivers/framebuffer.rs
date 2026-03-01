//! VESA VBE framebuffer information storage.
//!
//! Captures framebuffer address, dimensions, pitch, and bit depth from the
//! boot info structure. Used by the compositor and GPU drivers for display setup.
//!
//! GPU drivers that change the active framebuffer (e.g. VirtIO GPU switching to
//! guest RAM) call [`update()`] — registered hooks are notified automatically.

use crate::boot_info::BootInfo;

/// Framebuffer information from VESA VBE mode
pub struct FramebufferInfo {
    pub addr: u64,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
}

static mut FB_INFO: Option<FramebufferInfo> = None;

/// Change hook: called synchronously when [`update()`] stores new FB parameters.
/// Signature: `fn(addr, pitch, width, height)`.
static mut ON_CHANGE: Option<fn(u64, u32, u32, u32)> = None;

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
            addr: addr as u64,
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

/// Update framebuffer parameters at runtime (e.g. GPU driver changed resolution
/// or switched to a different backing buffer). Notifies registered hooks.
pub fn update(addr: u64, pitch: u32, width: u32, height: u32, bpp: u8) {
    unsafe {
        FB_INFO = Some(FramebufferInfo {
            addr,
            pitch,
            width,
            height,
            bpp,
        });

        if let Some(hook) = ON_CHANGE {
            hook(addr, pitch, width, height);
        }
    }
}

/// Register a hook that is called when framebuffer parameters change.
/// Only one hook is supported (boot_console). Called during early boot,
/// before interrupts — no synchronization needed.
pub fn register_change_hook(hook: fn(u64, u32, u32, u32)) {
    unsafe { ON_CHANGE = Some(hook); }
}

/// Get framebuffer info (returns None if not initialized or no VESA mode)
pub fn info() -> Option<&'static FramebufferInfo> {
    unsafe { FB_INFO.as_ref() }
}

/// Check if framebuffer is available
pub fn is_available() -> bool {
    unsafe { FB_INFO.is_some() }
}
