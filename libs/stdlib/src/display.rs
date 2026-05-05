//! Low-level multi-monitor display API.
//!
//! Wraps the kernel `SYS_DISPLAY_*` syscalls (700–704). Higher-level
//! ergonomic accessors (`Screen::list`, `Screen::primary`) live in
//! `libanyui_client` so GUI apps don't have to depend on this module
//! directly.
//!
//! All functions here are compositor-only on the kernel side
//! (`is_compositor()` check) — calling them from a non-compositor
//! process returns an error.

use crate::raw::{
    syscall0, syscall1_u64, syscall2, syscall2_u64, syscall3, SYS_DISPLAY_FLUSH, SYS_DISPLAY_LIST,
    SYS_DISPLAY_MAP_FB, SYS_DISPLAY_POLL_EVENT, SYS_DISPLAY_SET_LAYOUT, SYS_REGISTER_DISPLAY_OWNER,
};
use crate::Vec;

/// Per-output info as returned by `SYS_DISPLAY_LIST`. Wire-compatible
/// with the kernel `DisplayInfoFfi`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct DisplayInfo {
    pub id: u32,
    pub connected: u32,
    pub current_w: u32,
    pub current_h: u32,
    pub preferred_w: u32,
    pub preferred_h: u32,
    pub refresh_mhz: u32,
    pub bpp: u32,
    pub physical_mm: u32,
    pub edid_hash: u64,
    pub manufacturer: u32,
    pub flags: u32,
    pub mirror_of: u32,
    pub _reserved: [u32; 2],
}

impl DisplayInfo {
    pub fn is_connected(&self) -> bool {
        self.connected != 0
    }

    pub fn is_primary(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn is_mirror(&self) -> bool {
        self.flags & 2 != 0
    }

    pub fn physical_mm_pair(&self) -> (u16, u16) {
        (
            (self.physical_mm & 0xFFFF) as u16,
            (self.physical_mm >> 16) as u16,
        )
    }

    /// 3-letter PNPID from EDID, e.g. `"DEL"` for Dell. May be empty if
    /// EDID was unavailable.
    pub fn manufacturer_str(&self) -> [u8; 3] {
        let bytes = self.manufacturer.to_le_bytes();
        [bytes[0], bytes[1], bytes[2]]
    }
}

/// Wire-compatible with the kernel `LayoutEntryFfi`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LayoutEntry {
    pub id: u32,
    pub virtual_x: i32,
    pub virtual_y: i32,
    pub mode_w: u32,
    pub mode_h: u32,
    pub mode_refresh_mhz: u32,
    pub scale: u32,
    pub flags: u32,
    pub mirror_of: u32,
}

impl LayoutEntry {
    /// `mirror_of` value meaning "not a mirror".
    pub const NO_MIRROR: u32 = u32::MAX;

    pub fn primary(id: u32, x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            id,
            virtual_x: x,
            virtual_y: y,
            mode_w: w,
            mode_h: h,
            mode_refresh_mhz: 60_000,
            scale: 100,
            flags: 1,
            mirror_of: Self::NO_MIRROR,
        }
    }

    pub fn secondary(id: u32, x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            id,
            virtual_x: x,
            virtual_y: y,
            mode_w: w,
            mode_h: h,
            mode_refresh_mhz: 60_000,
            scale: 100,
            flags: 0,
            mirror_of: Self::NO_MIRROR,
        }
    }
}

/// Per-output framebuffer mapping returned by `SYS_DISPLAY_MAP_FB`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FbMapInfo {
    pub fb_addr: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

/// One drained display event from `SYS_DISPLAY_POLL_EVENT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayEvent {
    None,
    HotplugChanged,
    PreferredModeChanged { output: u32 },
    LayoutApplied,
}

/// Enumerate all advertised display outputs. Returns up to `max` entries.
/// On error returns an empty Vec.
pub fn list(max: usize) -> Vec<DisplayInfo> {
    let max = max.min(16);
    let mut buf: Vec<DisplayInfo> = Vec::with_capacity(max);
    for _ in 0..max {
        buf.push(DisplayInfo::default());
    }
    let count =
        unsafe { crate::raw::syscall2(SYS_DISPLAY_LIST, buf.as_mut_ptr() as u64, max as u64) };
    if count == u32::MAX {
        return Vec::new();
    }
    buf.truncate(count as usize);
    buf
}

/// Atomically apply a complete layout. Returns 0 on success, a non-zero
/// `LayoutError::code()` on validation failure (current layout is
/// preserved), or `u32::MAX` on hard error.
pub fn set_layout(entries: &[LayoutEntry]) -> u32 {
    if entries.is_empty() || entries.len() > 32 {
        return u32::MAX;
    }
    syscall2(
        SYS_DISPLAY_SET_LAYOUT,
        entries.as_ptr() as u64,
        entries.len() as u64,
    )
}

/// Map output `output_id`'s framebuffer into the calling process's
/// address space at a per-output base (kernel uses 0x2000_0000 + id*64 MiB).
pub fn map_fb(output_id: u32) -> Option<FbMapInfo> {
    let mut info = FbMapInfo::default();
    let r = syscall2(
        SYS_DISPLAY_MAP_FB,
        output_id as u64,
        &mut info as *mut FbMapInfo as u64,
    );
    if r == 0 {
        Some(info)
    } else {
        None
    }
}

/// Transfer + flush a rect on `output_id`. Coordinates clamped at the
/// kernel; out-of-range rects are silently no-op.
pub fn flush(output_id: u32, x: u32, y: u32, w: u32, h: u32) -> u32 {
    let xy = ((x & 0xFFFF) << 16) | (y & 0xFFFF);
    let wh = ((w & 0xFFFF) << 16) | (h & 0xFFFF);
    syscall3(SYS_DISPLAY_FLUSH, output_id as u64, xy as u64, wh as u64)
}

/// Register the calling process as the display-layout owner.
/// First-caller-wins; the compositor is expected to spawn a single
/// trusted displayd before any other process can grab this slot.
/// Returns 0 on success, `u32::MAX` if already taken.
pub fn register_owner() -> u32 {
    syscall0(SYS_REGISTER_DISPLAY_OWNER)
}

/// Drain one display event. Returns `DisplayEvent::None` if no event is
/// pending.
pub fn poll_event() -> DisplayEvent {
    let raw = syscall0(SYS_DISPLAY_POLL_EVENT);
    let _ = syscall1_u64;
    let _ = syscall2_u64;
    match raw & 0xFF {
        0 => DisplayEvent::None,
        1 => DisplayEvent::HotplugChanged,
        2 => DisplayEvent::PreferredModeChanged {
            output: (raw >> 8) & 0xFF,
        },
        3 => DisplayEvent::LayoutApplied,
        _ => DisplayEvent::None,
    }
}
