//! System information — time, uptime, sysinfo, dmesg.

use crate::raw::*;

/// Get current time. Writes [year_lo, year_hi, month, day, hour, min, sec, 0] to buf.
pub fn time(buf: &mut [u8; 8]) -> u32 {
    syscall1(SYS_TIME, buf.as_mut_ptr() as u64)
}

/// Set system date/time via RTC.
/// buf: [year_lo, year_hi, month, day, hour, min, sec, 0]
/// Returns 0 on success, u32::MAX on error.
pub fn set_time(buf: &[u8; 8]) -> u32 {
    syscall1(SYS_SET_TIME, buf.as_ptr() as u64)
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
/// The buffer must be large enough for width*height u32 pixels.
/// info layout: [width, height, pitch_bytes].
pub fn capture_screen(buf: &mut [u32], info: &mut [u32; 3]) -> bool {
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

/// Enable or disable verbose serial output (driver/subsystem messages).
/// When disabled (default), only core kernel messages appear on the serial console.
/// When enabled, all driver and subsystem messages are also shown.
pub fn set_serial_verbose(enable: bool) -> u32 {
    syscall1(SYS_SET_SERIAL_VERBOSE, if enable { 1 } else { 0 })
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

/// Get the system hostname. Copies hostname into `buf`.
/// Returns the number of bytes written, or `u32::MAX` on error.
pub fn get_hostname(buf: &mut [u8]) -> u32 {
    syscall2(SYS_GET_HOSTNAME, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Set the system hostname.
/// Returns 0 on success, `u32::MAX` on error. Max 63 bytes.
pub fn set_hostname(name: &str) -> u32 {
    syscall2(SYS_SET_HOSTNAME, name.as_ptr() as u64, name.len() as u64)
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

// =========================================================================
// Text-mode console I/O (nogui mode only)
// =========================================================================

/// Write a string to the kernel framebuffer text console.
/// Only functional when the kernel was booted with `nogui`.
/// Returns number of bytes written.
pub fn con_write(s: &str) -> u32 {
    syscall2(SYS_CON_WRITE, s.as_ptr() as u64, s.len() as u64)
}

/// Read a line from the keyboard (with echo) into `buf`.
/// Blocks until Enter is pressed.
/// Returns number of bytes read (not including null terminator).
pub fn con_read_line(buf: &mut [u8]) -> usize {
    let len = buf.len().min(4095) as u32;
    syscall2(SYS_CON_READ, buf.as_mut_ptr() as u64, len as u64) as usize
}

/// Read a password line (no echo, shows `*` instead) into `buf`.
/// Blocks until Enter is pressed.
/// Returns number of bytes read.
pub fn con_read_password(buf: &mut [u8]) -> usize {
    let len = (buf.len().min(4095) as u32) | 0x8000_0000;
    syscall2(SYS_CON_READ, buf.as_mut_ptr() as u64, len as u64) as usize
}

/// Non-blocking keyboard poll for text console use.
/// Returns the Unicode codepoint of the next key press, or 0 if none pending.
/// Special values: 0x03 = Ctrl+C, 0x04 = Ctrl+D.
/// Bit 29 set = Ctrl modifier held.
pub fn con_poll_key() -> u32 {
    syscall0(SYS_CON_POLL_KEY)
}

/// Return the current text console size as `(columns, rows)`.
/// Values are derived from the framebuffer resolution and the kernel font size (8×16).
/// Returns (0, 0) if not in nogui mode or console not yet initialised.
pub fn con_get_size() -> (u32, u32) {
    let packed = syscall0(SYS_CON_GET_SIZE);
    (packed >> 16, packed & 0xFFFF)
}

/// Resize the text console to exactly `cols` columns and `rows` rows.
/// Recomputes cell dimensions from the framebuffer, clears the screen, homes cursor.
/// Returns new `(cols, rows)` on success, `(0, 0)` on failure.
pub fn con_resize(cols: u32, rows: u32) -> (u32, u32) {
    let packed = syscall1(SYS_CON_RESIZE, ((cols << 16) | rows) as u64);
    (packed >> 16, packed & 0xFFFF)
}

/// Set console mode flags. Returns the previous flags value.
///
/// Flags bitmask:
/// - `CON_MODE_HIDE_CURSOR`   (0x01): hide the blinking cursor block
/// - `CON_MODE_NO_AUTOSCROLL` (0x02): disable automatic scrolling on newline at bottom
///
/// Example — enter fullscreen TUI mode:
/// ```no_run
/// con_set_mode(CON_MODE_HIDE_CURSOR | CON_MODE_NO_AUTOSCROLL);
/// ```
/// Example — restore on exit:
/// ```no_run
/// con_set_mode(0);
/// ```
pub fn con_set_mode(flags: u32) -> u32 {
    syscall1(SYS_CON_SET_MODE, flags as u64)
}

/// Cursor-hide flag for [`con_set_mode`].
pub const CON_MODE_HIDE_CURSOR: u32   = 0x01;
/// Disable-auto-scroll flag for [`con_set_mode`].
pub const CON_MODE_NO_AUTOSCROLL: u32 = 0x02;

/// Key code prefix returned by [`con_poll_key`] for arrow keys.
/// The low byte encodes: A=Up, B=Down, C=Right, D=Left (VT100 CSI convention).
pub const KEY_ARROW_PREFIX: u32  = 0x10_0000;
/// Key code prefix for navigation keys (Home/End/PageUp/PageDown/Delete).
pub const KEY_NAV_PREFIX: u32    = 0x20_0000;
/// Key code prefix for function keys F1–F12.
/// Low byte: 1=F1 … 12=F12.
pub const KEY_FN_PREFIX: u32     = 0x30_0000;

pub const KEY_UP: u32     = KEY_ARROW_PREFIX | 0x41;
pub const KEY_DOWN: u32   = KEY_ARROW_PREFIX | 0x42;
pub const KEY_RIGHT: u32  = KEY_ARROW_PREFIX | 0x43;
pub const KEY_LEFT: u32   = KEY_ARROW_PREFIX | 0x44;

/// Shift modifier bit for arrow keys (bit 28).
pub const KEY_SHIFT_BIT: u32   = 0x1400_0000;
pub const KEY_SHIFT_UP: u32    = KEY_SHIFT_BIT | 0x41;
pub const KEY_SHIFT_DOWN: u32  = KEY_SHIFT_BIT | 0x42;
pub const KEY_SHIFT_RIGHT: u32 = KEY_SHIFT_BIT | 0x43;
pub const KEY_SHIFT_LEFT: u32  = KEY_SHIFT_BIT | 0x44;

pub const KEY_HOME: u32     = KEY_NAV_PREFIX | 0x48;
pub const KEY_END: u32      = KEY_NAV_PREFIX | 0x4B;
pub const KEY_PGUP: u32     = KEY_NAV_PREFIX | 0x49;
pub const KEY_PGDOWN: u32   = KEY_NAV_PREFIX | 0x51;
pub const KEY_DELETE: u32   = KEY_NAV_PREFIX | 0x53;

pub const KEY_F1: u32  = KEY_FN_PREFIX | 0x01;
pub const KEY_F2: u32  = KEY_FN_PREFIX | 0x02;
pub const KEY_F3: u32  = KEY_FN_PREFIX | 0x03;
pub const KEY_F4: u32  = KEY_FN_PREFIX | 0x04;
pub const KEY_F5: u32  = KEY_FN_PREFIX | 0x05;
pub const KEY_F6: u32  = KEY_FN_PREFIX | 0x06;
pub const KEY_F7: u32  = KEY_FN_PREFIX | 0x07;
pub const KEY_F8: u32  = KEY_FN_PREFIX | 0x08;
pub const KEY_F9: u32  = KEY_FN_PREFIX | 0x09;
pub const KEY_F10: u32 = KEY_FN_PREFIX | 0x0A;

// =========================================================================
// Thermal sensors
// =========================================================================

/// Raw entry returned by [`thermal_read_raw`].
///
/// Layout matches the kernel ABI (8 bytes per entry):
/// - `src_type`: 0=IntelCpu, 1=AmdCpu, 2=Lm75, 3=Smbus
/// - `src_id`:   core index or SMBus address
/// - `temp_x10`: temperature in 0.1 °C units (signed)
#[derive(Debug, Clone, Copy)]
pub struct ThermalEntry {
    pub src_type: u8,
    pub src_id:   u8,
    pub temp_x10: i32,
}

/// Read all registered thermal sensors.
///
/// Returns a `Vec` of up to `max` entries.  Pass a large value (e.g. 32)
/// to retrieve every available sensor.
pub fn thermal_read(max: u32) -> alloc::vec::Vec<ThermalEntry> {
    use alloc::vec;
    if max == 0 { return vec![]; }
    let entry_size = 8usize;
    let mut buf = vec![0u8; max as usize * entry_size];
    let count = syscall2(SYS_THERMAL_READ, buf.as_mut_ptr() as u64, max as u64) as usize;
    let count = count.min(max as usize);
    let mut out = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let off = i * entry_size;
        let src_type = buf[off];
        let src_id   = buf[off + 1];
        let temp_x10 = i32::from_le_bytes([buf[off+4], buf[off+5], buf[off+6], buf[off+7]]);
        out.push(ThermalEntry { src_type, src_id, temp_x10 });
    }
    out
}

/// Read the primary CPU temperature in 0.1 °C units.
///
/// Returns `None` if no CPU thermal sensor is available on this hardware.
pub fn thermal_cpu() -> Option<i32> {
    let v = syscall0(SYS_THERMAL_CPU);
    if v == u32::MAX { None } else { Some(v as i32) }
}

// =========================================================================
// ACPI power management
// =========================================================================

/// Request an ACPI sleep / power state.
///
/// | `state` | Meaning           |
/// |---------|-------------------|
/// | 0       | S0 — no-op        |
/// | 3       | S3 — suspend RAM  |
/// | 4       | S4 — hibernate    |
/// | 5       | S5 — power off    |
///
/// Returns `Ok(())` on success, `Err(())` for an unrecognised state.
pub fn acpi_sleep(state: u32) -> Result<(), ()> {
    if syscall1(SYS_ACPI_SLEEP, state as u64) == u32::MAX { Err(()) } else { Ok(()) }
}

/// Get the current CPU P-state frequency ratio byte.
///
/// On Intel this is `IA32_PERF_CTL[15:8]`; on AMD `PERF_CTL[7:0]`.
/// Returns `None` if the kernel call fails.
pub fn acpi_perf_get() -> Option<u8> {
    let v = syscall2(SYS_ACPI_PERF, 0, 0);
    if v == u32::MAX { None } else { Some(v as u8) }
}

/// Set the CPU P-state frequency ratio.
///
/// Returns `Ok(())` on success, `Err(())` on failure.
pub fn acpi_perf_set(ratio: u8) -> Result<(), ()> {
    if syscall2(SYS_ACPI_PERF, 1, ratio as u64) == u32::MAX { Err(()) } else { Ok(()) }
}

// =========================================================================
// I²C / SMBus
// =========================================================================

/// Read a byte from I²C device at 7-bit address `addr`, register `reg`.
///
/// Returns `None` if the device does not ACK or the address/register is out of range.
pub fn i2c_read_byte(addr: u8, reg: u8) -> Option<u8> {
    let v = syscall2(SYS_I2C_READ, addr as u64, reg as u64);
    if v == u32::MAX { None } else { Some(v as u8) }
}

/// Write `value` to register `reg` of I²C device at 7-bit address `addr`.
///
/// Returns `Ok(())` on success, `Err(())` on error.
pub fn i2c_write_byte(addr: u8, reg: u8, value: u8) -> Result<(), ()> {
    let v = syscall3(SYS_I2C_WRITE, addr as u64, reg as u64, value as u64);
    if v == u32::MAX { Err(()) } else { Ok(()) }
}

/// Probe whether an I²C device is present at 7-bit address `addr`.
///
/// Returns `true` if the device ACKed the quick-write probe.
pub fn i2c_detect(addr: u8) -> bool {
    syscall1(SYS_I2C_DETECT, addr as u64) == 1
}
