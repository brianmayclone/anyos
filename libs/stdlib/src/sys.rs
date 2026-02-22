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

/// Get uptime in milliseconds (TSC-based, sub-ms precision).
/// Wraps at ~49 days — use wrapping_sub for deltas.
pub fn uptime_ms() -> u32 {
    syscall0(SYS_UPTIME_MS)
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

/// Fill a buffer with random bytes from the kernel RNG.
/// Returns number of bytes written.
pub fn random(buf: &mut [u8]) -> u32 {
    if buf.is_empty() { return 0; }
    let len = buf.len().min(256);
    syscall2(SYS_RANDOM, buf.as_mut_ptr() as u64, len as u64)
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

/// Retrieve crash report for a terminated thread.
/// Returns bytes written to buf, or 0 if no crash report exists for that TID.
/// Buffer must be large enough for the kernel's CrashReport struct.
pub fn get_crash_info(tid: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_GET_CRASH_INFO, tid as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

// =========================================================================
// Disk / Partition management
// =========================================================================

/// List all block devices. Each 32-byte entry:
///   [0]      id (u8)
///   [1]      disk_id (u8)
///   [2]      partition (0xFF = whole disk, else partition index)
///   [3..7]   padding
///   [8..16]  start_lba (u64 LE)
///   [16..24] size_sectors (u64 LE)
///   [24..32] padding
/// Returns number of devices.
pub fn disk_list(buf: &mut [u8]) -> u32 {
    syscall2(SYS_DISK_LIST, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// List partitions for a specific disk. Each 32-byte entry:
///   [0]      index (u8)
///   [1]      part_type (MBR type byte: 0x07=NTFS/exFAT, 0x0B=FAT32, etc.)
///   [2]      bootable (0 or 1)
///   [3]      scheme (0=None, 1=MBR, 2=GPT)
///   [4..8]   padding
///   [8..16]  start_lba (u64 LE)
///   [16..24] size_sectors (u64 LE)
///   [24..32] padding
/// Returns number of partitions found, or u32::MAX on error.
pub fn disk_partitions(disk_id: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_DISK_PARTITIONS, disk_id as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Read raw sectors from a block device.
/// Returns 0 on success, u32::MAX on error.
pub fn disk_read(device_id: u32, lba: u64, count: u32, buf: &mut [u8]) -> u32 {
    syscall5(SYS_DISK_READ, device_id as u64, lba as u32 as u64, count as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Write raw sectors to a block device.
/// Returns 0 on success, u32::MAX on error.
pub fn disk_write(device_id: u32, lba: u64, count: u32, buf: &[u8]) -> u32 {
    syscall5(SYS_DISK_WRITE, device_id as u64, lba as u32 as u64, count as u64, buf.as_ptr() as u64, buf.len() as u64)
}

/// Create/update an MBR partition entry.
/// `entry` is a 16-byte buffer:
///   [0]      partition index (0-3)
///   [1]      type byte (0x07, 0x0B, 0x0C, etc.)
///   [2]      bootable (0 or 0x80)
///   [3]      padding
///   [4..8]   start_lba (u32 LE)
///   [8..12]  size_sectors (u32 LE)
///   [12..16] padding
/// Returns 0 on success, u32::MAX on error.
pub fn partition_create(disk_id: u32, entry: &[u8; 16]) -> u32 {
    syscall3(SYS_PARTITION_CREATE, disk_id as u64, entry.as_ptr() as u64, 16)
}

/// Delete an MBR partition entry (zero it out).
/// Returns 0 on success, u32::MAX on error.
pub fn partition_delete(disk_id: u32, index: u32) -> u32 {
    syscall2(SYS_PARTITION_DELETE, disk_id as u64, index as u64)
}

/// Re-scan partition table and re-register block devices.
/// Returns number of partitions found.
pub fn partition_rescan(disk_id: u32) -> u32 {
    syscall1(SYS_PARTITION_RESCAN, disk_id as u64)
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
