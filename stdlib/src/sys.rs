//! System information — time, uptime, sysinfo, dmesg.

use crate::raw::*;

/// Get current time. Writes [year_lo, year_hi, month, day, hour, min, sec, 0] to buf.
pub fn time(buf: &mut [u8; 8]) -> u32 {
    syscall1(SYS_TIME, buf.as_mut_ptr() as u32)
}

/// Get uptime in PIT ticks (100 Hz).
pub fn uptime() -> u32 {
    syscall0(SYS_UPTIME)
}

/// Get system info. cmd: 0=memory, 1=threads, 2=cpus.
pub fn sysinfo(cmd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_SYSINFO, cmd, buf.as_mut_ptr() as u32, buf.len() as u32)
}

/// Read kernel log (dmesg). Returns bytes written to buf.
pub fn dmesg(buf: &mut [u8]) -> u32 {
    syscall2(SYS_DMESG, buf.as_mut_ptr() as u32, buf.len() as u32)
}
