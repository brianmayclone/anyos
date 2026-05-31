//! Platform / thermal / ACPI / I²C syscall handlers.
//!
//! Implements the following syscalls (numbers 320–326):
//!
//! | Num | Name              | Description                                  |
//! |-----|-------------------|----------------------------------------------|
//! | 320 | SYS_THERMAL_READ  | Read all thermal sensors into user buffer    |
//! | 321 | SYS_THERMAL_CPU   | Read CPU temperature (0.1 °C), u32::MAX=n/a |
//! | 322 | SYS_ACPI_SLEEP    | Request ACPI sleep state (0=S0, 3=S3, 5=S5) |
//! | 323 | SYS_ACPI_PERF     | Get/set CPU P-state frequency ratio          |
//! | 324 | SYS_I2C_READ      | Read byte from I²C device register           |
//! | 325 | SYS_I2C_WRITE     | Write byte to I²C device register            |
//! | 326 | SYS_I2C_DETECT    | Detect I²C device presence at address        |

#[cfg(target_arch = "x86_64")]
use super::helpers::copy_to_user_bytes;
use super::helpers::is_valid_user_ptr;
#[cfg(target_arch = "x86_64")]
use crate::drivers::i2c::I2cDevice;

// ── SYS_THERMAL_READ (320) ────────────────────────────────────────────────────

/// Write thermal readings into a user-space buffer.
///
/// Each entry is 8 bytes:
/// - byte 0: source type (0=IntelCpu, 1=AmdCpu, 2=Lm75, 3=Smbus)
/// - byte 1: source id  (core index or SMBus address)
/// - bytes 2–3: padding (zero)
/// - bytes 4–7: temperature in 0.1 °C units (i32 LE, signed)
///
/// Returns the number of entries written.
pub fn sys_thermal_read(buf_ptr: u64, max_count: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if buf_ptr == 0 || max_count == 0 {
            return 0;
        }
        let entry_size = 8usize;
        let total_bytes = (max_count as usize) * entry_size;
        // Validate user pointer
        if !is_valid_user_ptr(buf_ptr as u64, total_bytes as u64) {
            return 0;
        }

        let readings = crate::drivers::thermal::read_all();
        let count = readings.len().min(max_count as usize);
        let total = count * entry_size;
        if total == 0 {
            return 0;
        }

        // Build the entries in a kernel-owned buffer, then copy out once via the
        // mapping-validated helper (zero-initialized -> padding bytes are 0).
        let mut out = alloc::vec![0u8; total];

        for (i, r) in readings[..count].iter().enumerate() {
            let off = i * entry_size;
            use crate::drivers::thermal::ThermalSource;
            let (src_type, src_id) = match r.source {
                ThermalSource::IntelCpu(id) => (0u8, id as u8),
                ThermalSource::AmdCpu(id) => (1u8, id as u8),
                ThermalSource::Lm75(addr) => (2u8, addr),
                ThermalSource::Smbus(addr) => (3u8, addr),
            };
            out[off] = src_type;
            out[off + 1] = src_id;
            // out[off + 2] and out[off + 3] remain 0 (padding).
            let temp_bytes = r.temp_c_x10.to_le_bytes();
            out[off + 4..off + 8].copy_from_slice(&temp_bytes);
        }

        if !copy_to_user_bytes(buf_ptr, &out, total) {
            return 0;
        }

        count as u32
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (buf_ptr, max_count);
        0
    }
}

// ── SYS_THERMAL_CPU (321) ─────────────────────────────────────────────────────

/// Read CPU temperature in 0.1 °C units.
/// Returns `u32::MAX` if no CPU thermal sensor is available.
pub fn sys_thermal_cpu() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        match crate::drivers::thermal::read_cpu_temp_standalone() {
            Some(t) => t as u32,
            None => u32::MAX,
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        u32::MAX
    }
}

// ── SYS_ACPI_SLEEP (322) ─────────────────────────────────────────────────────

/// Request an ACPI sleep state.
///
/// `state`:
/// - 0 = S0 (no-op, already running)
/// - 3 = S3 (suspend to RAM)
/// - 4 = S4 (suspend to disk / hibernate)
/// - 5 = S5 (power off)
///
/// Returns 0 on success, `u32::MAX` if the state is unrecognized.
pub fn sys_acpi_sleep(state: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        use crate::arch::x86::acpi_pm::SleepState;
        let s = match state {
            0 => SleepState::S0,
            3 => SleepState::S3,
            4 => SleepState::S4,
            5 => SleepState::S5,
            _ => return u32::MAX,
        };
        crate::arch::x86::acpi_pm::request_sleep_state(s);
        0
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = state;
        u32::MAX
    }
}

// ── SYS_ACPI_PERF (323) ──────────────────────────────────────────────────────

/// Get or set the CPU P-state frequency ratio.
///
/// `cmd`:
/// - 0 = get current ratio (returns the ratio byte as u32)
/// - 1 = set ratio to `arg` (returns 0)
///
/// Returns `u32::MAX` for unknown commands.
pub fn sys_acpi_perf(cmd: u32, arg: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        match cmd {
            0 => crate::arch::x86::acpi_pm::get_perf_level() as u32,
            1 => {
                crate::arch::x86::acpi_pm::set_perf_level(arg as u8);
                0
            }
            _ => u32::MAX,
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (cmd, arg);
        u32::MAX
    }
}

// ── SYS_I2C_READ (324) ───────────────────────────────────────────────────────

/// Read a byte from I²C device at 7-bit address `addr`, register `reg`.
/// Returns the byte value (0–255) or `u32::MAX` on error / absent device.
pub fn sys_i2c_read(addr: u32, reg: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if addr > 0x7F || reg > 0xFF {
            return u32::MAX;
        }
        let dev = I2cDevice { addr: addr as u8 };
        match crate::drivers::i2c::i2c_read_byte_data(&dev, reg as u8) {
            Some(val) => val as u32,
            None => u32::MAX,
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (addr, reg);
        u32::MAX
    }
}

// ── SYS_I2C_WRITE (325) ──────────────────────────────────────────────────────

/// Write `value` to register `reg` of I²C device at 7-bit address `addr`.
/// Returns 0 on success, `u32::MAX` on error.
pub fn sys_i2c_write(addr: u32, reg: u32, value: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if addr > 0x7F || reg > 0xFF || value > 0xFF {
            return u32::MAX;
        }
        let dev = I2cDevice { addr: addr as u8 };
        if crate::drivers::i2c::i2c_write_byte_data(&dev, reg as u8, value as u8) {
            0
        } else {
            u32::MAX
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (addr, reg, value);
        u32::MAX
    }
}

// ── SYS_I2C_DETECT (326) ─────────────────────────────────────────────────────

/// Probe whether an I²C device is present at 7-bit address `addr`.
/// Returns 1 if the device ACKed, 0 if absent or no SMBus controller.
pub fn sys_i2c_detect(addr: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if addr > 0x7F {
            return 0;
        }
        if crate::drivers::i2c::i2c_detect(addr as u8) {
            1
        } else {
            0
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = addr;
        0
    }
}
