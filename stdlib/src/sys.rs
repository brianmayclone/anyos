//! System information — time, uptime, sysinfo, dmesg.

use crate::raw::*;

/// Get current time. Writes [year_lo, year_hi, month, day, hour, min, sec, 0] to buf.
pub fn time(buf: &mut [u8; 8]) -> u32 {
    syscall1(SYS_TIME, buf.as_mut_ptr() as u64)
}

/// Get uptime in PIT ticks.
pub fn uptime() -> u32 {
    syscall0(SYS_UPTIME)
}

/// Get the PIT tick rate in Hz (e.g. 100 = 100 ticks/second).
pub fn tick_hz() -> u32 {
    syscall0(SYS_TICK_HZ)
}

/// Get system info. cmd: 0=memory, 1=threads, 2=cpus.
pub fn sysinfo(cmd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_SYSINFO, cmd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Read kernel log (dmesg). Returns bytes written to buf.
pub fn dmesg(buf: &mut [u8]) -> u32 {
    syscall2(SYS_DMESG, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Signal to the kernel that the boot/init phase is complete.
/// The compositor transitions from boot splash to full desktop.
pub fn boot_ready() {
    syscall0(SYS_BOOT_READY);
}

/// Capture the current screen contents into a pixel buffer.
/// Returns (width, height) on success, None on failure.
/// The buffer must be large enough for width*height u32 pixels.
pub fn capture_screen(buf: &mut [u32], info: &mut [u32; 2]) -> bool {
    let ret = syscall3(
        SYS_CAPTURE_SCREEN,
        buf.as_mut_ptr() as u64,
        (buf.len() * 4) as u64,
        info.as_mut_ptr() as u64,
    );
    ret == 0
}

/// Mark the calling thread as critical (won't be killed by kernel RSP recovery).
/// Only system services (compositor) should use this.
pub fn set_critical() {
    syscall0(SYS_SET_CRITICAL);
}

/// List devices. Each 64-byte entry:
///   [0..32]  path (null-terminated)
///   [32..56] driver name (null-terminated)
///   [56]     driver_type (0=Block,1=Char,2=Network,3=Display,4=Input,5=Audio,6=Output,7=Sensor,8=Bus,9=Unknown)
///   [57..64] padding
/// Returns total device count.
pub fn devlist(buf: &mut [u8]) -> u32 {
    syscall2(SYS_DEVLIST, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// List all open pipes. Each 80-byte entry:
///   [0..4]   pipe_id (u32 LE)
///   [4..8]   buffered_bytes (u32 LE)
///   [8..72]  name (64 bytes, null-terminated)
///   [72..80] padding
/// Returns total pipe count.
pub fn pipe_list(buf: &mut [u8]) -> u32 {
    syscall2(SYS_PIPE_LIST, buf.as_mut_ptr() as u64, buf.len() as u64)
}
