//! Monitor detection syscall handlers.
//!
//! Provides userspace access to detected monitors, their EDID data,
//! supported modes, and properties (physical size, color profile, etc.).

use super::helpers::is_valid_user_ptr;

/// SYS_MONITOR_COUNT (327): Returns the number of detected monitors.
pub fn sys_monitor_count() -> u32 {
    crate::drivers::monitor::monitor_count()
}

/// SYS_MONITOR_INFO (328): Write packed MonitorInfoFlat to user buffer.
///
/// `monitor_id`: monitor index (0-based).
/// `buf_ptr`: user pointer to a buffer of at least 128 bytes.
///
/// Returns 0 on success, `u32::MAX` on error.
pub fn sys_monitor_info(monitor_id: u32, buf_ptr: u32) -> u32 {
    if buf_ptr == 0 || !is_valid_user_ptr(buf_ptr as u64, 128) {
        return u32::MAX;
    }
    let info = match crate::drivers::monitor::get_monitor(monitor_id) {
        Some(i) => i,
        None => return u32::MAX,
    };

    // Build packed struct.
    let flat = MonitorInfoFlat {
        id: info.id,
        gpu_output: info.gpu_output,
        manufacturer: info.edid.manufacturer,
        model_name: info.edid.model_name,
        serial_string: info.edid.serial_string,
        serial_number: info.edid.serial_number,
        product_code: info.edid.product_code,
        manufacture_year: info.edid.manufacture_year,
        manufacture_week: info.edid.manufacture_week,
        _pad0: 0,
        width_mm: info.width_mm,
        height_mm: info.height_mm,
        gamma: info.edid.gamma,
        native_width: info.edid.preferred_width,
        native_height: info.edid.preferred_height,
        native_refresh_100: info.edid.preferred_refresh_100,
        color_depth: info.edid.color_depth,
        is_digital: info.edid.is_digital as u8,
        has_edid: info.has_edid as u8,
        _pad1: 0,
        mode_count: info.mode_count as u32,
        red_x: info.edid.chromaticity.red_x,
        red_y: info.edid.chromaticity.red_y,
        green_x: info.edid.chromaticity.green_x,
        green_y: info.edid.chromaticity.green_y,
        blue_x: info.edid.chromaticity.blue_x,
        blue_y: info.edid.chromaticity.blue_y,
        white_x: info.edid.chromaticity.white_x,
        white_y: info.edid.chromaticity.white_y,
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &flat as *const MonitorInfoFlat as *const u8,
            buf_ptr as *mut u8,
            core::mem::size_of::<MonitorInfoFlat>(),
        );
    }
    0
}

/// SYS_MONITOR_EDID (329): Copy raw EDID bytes to user buffer.
///
/// `monitor_id`: monitor index.
/// `buf_ptr`: user pointer.
/// `buf_len`: buffer size in bytes.
///
/// Returns number of bytes written, or 0 on error.
pub fn sys_monitor_edid(monitor_id: u32, buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) {
        return 0;
    }
    let info = match crate::drivers::monitor::get_monitor(monitor_id) {
        Some(i) => i,
        None => return 0,
    };
    let copy_len = (buf_len as usize).min(info.edid_raw_len);
    if copy_len == 0 {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            info.edid_raw.as_ptr(),
            buf_ptr as *mut u8,
            copy_len,
        );
    }
    copy_len as u32
}

/// SYS_MONITOR_MODES (330): Write supported DisplayMode entries to user buffer.
///
/// Each mode is 16 bytes: `[width: u32, height: u32, refresh_hz_100: u32, flags: u32]`.
/// `flags` bit 0 = is_preferred, bit 1 = is_interlaced.
///
/// `monitor_id`: monitor index.
/// `buf_ptr`: user pointer.
/// `buf_len`: buffer size in bytes.
///
/// Returns number of modes written.
pub fn sys_monitor_modes(monitor_id: u32, buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) {
        return 0;
    }
    let info = match crate::drivers::monitor::get_monitor(monitor_id) {
        Some(i) => i,
        None => return 0,
    };
    let mode_size = 16usize; // 4 × u32
    let max_modes = (buf_len as usize) / mode_size;
    let write_count = max_modes.min(info.mode_count);
    unsafe {
        let buf = buf_ptr as *mut u32;
        for i in 0..write_count {
            let m = &info.modes[i];
            *buf.add(i * 4) = m.width;
            *buf.add(i * 4 + 1) = m.height;
            *buf.add(i * 4 + 2) = m.refresh_hz_100;
            *buf.add(i * 4 + 3) = (m.is_preferred as u32) | ((m.is_interlaced as u32) << 1);
        }
    }
    write_count as u32
}

// ──────────────────────────────────────────────
// Packed transfer struct (no pointers, fixed-size)
// ──────────────────────────────────────────────

/// Packed monitor info for userspace transfer via SYS_MONITOR_INFO.
/// Size: 100 bytes.
#[repr(C, packed)]
struct MonitorInfoFlat {
    id: u32,                     // 0
    gpu_output: u32,             // 4
    manufacturer: [u8; 4],       // 8
    model_name: [u8; 14],        // 12
    serial_string: [u8; 14],     // 26
    serial_number: u32,          // 40
    product_code: u16,           // 44
    manufacture_year: u16,       // 46
    manufacture_week: u8,        // 48
    _pad0: u8,                   // 49
    width_mm: u16,               // 50
    height_mm: u16,              // 52
    gamma: u16,                  // 54
    native_width: u32,           // 56
    native_height: u32,          // 60
    native_refresh_100: u32,     // 64
    color_depth: u8,             // 68
    is_digital: u8,              // 69
    has_edid: u8,                // 70
    _pad1: u8,                   // 71
    mode_count: u32,             // 72
    // Chromaticity (CIE 1931 xy, 10-bit fixed-point)
    red_x: u16,                  // 76
    red_y: u16,                  // 78
    green_x: u16,                // 80
    green_y: u16,                // 82
    blue_x: u16,                 // 84
    blue_y: u16,                 // 86
    white_x: u16,                // 88
    white_y: u16,                // 90
    // Total: 92 bytes
}
