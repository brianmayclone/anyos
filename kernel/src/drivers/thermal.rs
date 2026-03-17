//! Thermal sensor driver.
//!
//! Aggregates temperature readings from multiple sources:
//!
//! 1. **Intel CPU** via MSR `IA32_THERM_STATUS` (0x19C) and
//!    `MSR_TEMPERATURE_TARGET` (0x1A2).  Temperature = Tjmax − digital_readout.
//!
//! 2. **AMD CPU** via MSR `MSR_AMD_TCTL` (0xC0010021).
//!    Bits [28:21] give Tctl in raw form.  We report Tctl directly (°C × 10).
//!
//! 3. **LM75 / TMP75 / LM77** sensors on the SMBus (addresses 0x48–0x4F,
//!    register 0x00 = temperature word big-endian, bits [15:5] = temp × 0.125 °C).
//!
//! All temperatures are reported in units of 0.1 °C (e.g. 450 = 45.0 °C).

use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;
use crate::drivers::pci::PciDevice;

// ── MSR Addresses ─────────────────────────────────────────────────────────────

/// Intel: per-core digital temperature sensor status.
const IA32_THERM_STATUS: u32 = 0x19C;
/// Intel: package-level digital temperature sensor status.
const IA32_PACKAGE_THERM_STATUS: u32 = 0x1B1;
/// Intel: MSR_TEMPERATURE_TARGET — Tjmax in bits [23:16].
const MSR_TEMPERATURE_TARGET: u32 = 0x1A2;
/// AMD Family 17h+: Temperature Control register.
const MSR_AMD_TCTL: u32 = 0xC001_0021;

// ── Public Types ─────────────────────────────────────────────────────────────

/// CPU vendor identity, detected via CPUID leaf 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

/// Identifies the origin of a particular thermal reading.
#[derive(Debug, Clone, Copy)]
pub enum ThermalSource {
    /// Intel CPU core/package sensor; inner value = core index (0 = package).
    IntelCpu(u32),
    /// AMD CPU Tctl sensor; inner value = core index (0 = package / Tctl).
    AmdCpu(u32),
    /// LM75/TMP75/LM77 sensor on SMBus; inner value = SMBus address (0x48–0x4F).
    Lm75(u8),
    /// Generic SMBus temperature sensor; inner value = SMBus address.
    Smbus(u8),
}

/// A single temperature measurement.
#[derive(Debug, Clone, Copy)]
pub struct ThermalReading {
    /// The source that produced this reading.
    pub source: ThermalSource,
    /// Temperature in units of 0.1 °C (e.g. 450 = 45.0 °C).
    /// Negative values are valid (e.g. below ambient for some sensors).
    pub temp_c_x10: i32,
}

// ── Global State ─────────────────────────────────────────────────────────────

/// Registered thermal sources.  Populated by `init()`, read by `read_all()`.
static THERMAL_SOURCES: Spinlock<Vec<ThermalSource>> = Spinlock::new(Vec::new());

// ── Initialization ────────────────────────────────────────────────────────────

/// Detect all available thermal sensors and register them.
///
/// Must be called after PCI scan and SMBus init.  Initialises SMBus if not
/// already done (safe to call multiple times — smbus::init is idempotent once
/// a controller has been found).
pub fn init(pci_devices: &[PciDevice]) {
    // Ensure the SMBus controller is initialised — thermal may be called first.
    crate::drivers::smbus::init(pci_devices);

    let mut sources = THERMAL_SOURCES.lock();
    sources.clear();

    // ── CPU thermal sources ──────────────────────────────────────────────────
    match cpu_vendor() {
        CpuVendor::Intel => {
            // Verify that IA32_THERM_STATUS is accessible (CPUID leaf 6, EAX bit 0).
            let has_dts = {
                let (eax, _, _, _) = cpuid(6, 0);
                eax & 0x01 != 0
            };
            if has_dts {
                // Register the package sensor as index 0.
                sources.push(ThermalSource::IntelCpu(0));
                crate::serial_verbose_println!("  Thermal: Intel CPU package sensor registered");
            } else {
                crate::serial_verbose_println!("  Thermal: Intel CPU DTS not available");
            }
        }
        CpuVendor::Amd => {
            sources.push(ThermalSource::AmdCpu(0));
            crate::serial_verbose_println!("  Thermal: AMD CPU Tctl sensor registered");
        }
        CpuVendor::Unknown => {
            crate::serial_verbose_println!("  Thermal: Unknown CPU, no CPU thermal source");
        }
    }

    // ── SMBus / I²C thermal sensors ──────────────────────────────────────────
    // LM75-family devices occupy addresses 0x48–0x4F (8 possible addresses).
    for addr in 0x48u8..=0x4F {
        if crate::drivers::smbus::quick_write(addr) {
            sources.push(ThermalSource::Lm75(addr));
            crate::serial_verbose_println!("  Thermal: LM75/TMP75 detected at SMBus addr {:#04x}", addr);
        }
    }

    crate::serial_verbose_println!("[OK] Thermal: {} source(s) registered", sources.len());
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Read all registered thermal sources and return a `Vec<ThermalReading>`.
pub fn read_all() -> Vec<ThermalReading> {
    let sources = {
        let guard = THERMAL_SOURCES.lock();
        guard.clone()
    };

    let mut readings = Vec::with_capacity(sources.len());
    for source in &sources {
        if let Some(temp) = read_source(source) {
            readings.push(ThermalReading { source: *source, temp_c_x10: temp });
        }
    }
    readings
}

/// Read the first registered CPU thermal source.
/// Returns temperature in 0.1 °C units, or `None` if no CPU source is available.
///
/// Uses `read_cpu_temp_standalone` which directly invokes the MSR-based
/// reader without holding THERMAL_SOURCES lock, avoiding lock re-entry.
pub fn read_cpu_temp() -> Option<i32> {
    read_cpu_temp_standalone()
}

// Standalone version to avoid borrow issues — reads the first CPU source
// without holding the lock.
pub fn read_cpu_temp_standalone() -> Option<i32> {
    // Try Intel first
    match cpu_vendor() {
        CpuVendor::Intel => read_source(&ThermalSource::IntelCpu(0)),
        CpuVendor::Amd   => read_source(&ThermalSource::AmdCpu(0)),
        CpuVendor::Unknown => None,
    }
}

/// Get the Intel Tjmax from `MSR_TEMPERATURE_TARGET` bits [23:16].
/// Returns `None` if not on an Intel CPU or the MSR is unavailable.
pub fn get_tjmax() -> Option<u32> {
    if cpu_vendor() != CpuVendor::Intel {
        return None;
    }
    let val = read_msr(MSR_TEMPERATURE_TARGET);
    let tjmax = ((val >> 16) & 0xFF) as u32;
    if tjmax == 0 {
        // Fallback: common default for Haswell/Skylake class processors
        Some(100)
    } else {
        Some(tjmax)
    }
}

/// Detect the CPU vendor by examining CPUID leaf 0 vendor string.
pub fn cpu_vendor() -> CpuVendor {
    let [ebx, ecx, edx] = cpuid_vendor();
    // Intel: "GenuineIntel" = EBX="Genu" ECX="ntel" EDX="ineI"
    // AMD:   "AuthenticAMD" = EBX="Auth" ECX="cAMD" EDX="enti"
    if ebx == 0x756E6547 && edx == 0x49656E69 && ecx == 0x6C65746E {
        CpuVendor::Intel
    } else if ebx == 0x68747541 && edx == 0x69746E65 && ecx == 0x444D4163 {
        CpuVendor::Amd
    } else {
        CpuVendor::Unknown
    }
}

// ── Private Helpers ───────────────────────────────────────────────────────────

/// Read a single thermal source and return the temperature in 0.1 °C units.
fn read_source(source: &ThermalSource) -> Option<i32> {
    match source {
        ThermalSource::IntelCpu(_core) => read_intel_cpu(),
        ThermalSource::AmdCpu(_core) => read_amd_cpu(),
        ThermalSource::Lm75(addr) | ThermalSource::Smbus(addr) => read_lm75(*addr),
    }
}

/// Read Intel package temperature from MSR.
/// Returns temperature in 0.1 °C.
fn read_intel_cpu() -> Option<i32> {
    let tjmax = get_tjmax()? as i32;

    // Read package thermal status; fall back to core 0 (IA32_THERM_STATUS).
    // Try the package MSR first (may not be present on older CPUs).
    let pkg_sts = read_msr(IA32_PACKAGE_THERM_STATUS);
    let readout = if pkg_sts & (1 << 31) != 0 {
        // Package sensor valid
        ((pkg_sts >> 16) & 0x7F) as i32
    } else {
        // Fall back to core 0 sensor
        let core_sts = read_msr(IA32_THERM_STATUS);
        if core_sts & (1 << 31) == 0 {
            return None; // sensor reading not valid
        }
        ((core_sts >> 16) & 0x7F) as i32
    };

    // Temperature = Tjmax - digital_readout
    let temp_c = tjmax - readout;
    Some(temp_c * 10) // convert to 0.1 °C units
}

/// Read AMD Tctl from MSR_AMD_TCTL.
/// Returns temperature in 0.1 °C.
fn read_amd_cpu() -> Option<i32> {
    // Guard: only read on AMD hardware (MSR may #GP on other CPUs)
    if cpu_vendor() != CpuVendor::Amd {
        return None;
    }

    // On hypervisors the AMD MSRs are typically not emulated.
    // Check the hypervisor bit (CPUID 1 ECX bit 31) as a safety guard.
    let (_, _, ecx1, _) = cpuid(1, 0);
    if ecx1 & (1 << 31) != 0 {
        // Hypervisor present; skip AMD MSR access
        return None;
    }

    let tctl_raw = read_msr(MSR_AMD_TCTL);
    // Bits [28:21] = Tctl integer degrees (8-bit field, degrees C)
    let tctl = ((tctl_raw >> 21) & 0xFF) as i32;
    Some(tctl * 10)
}

/// Read an LM75/TMP75/LM77 sensor via SMBus.
///
/// Protocol: read word at register 0x00 from the I²C address.
/// The temperature is encoded in the upper 11 bits (big-endian from the device,
/// but the SMBus host returns DAT0=high byte, DAT1=low byte after byte-swap by
/// some implementations; we treat DAT0 as the MSB).
///
/// Temperature word format (11-bit, 0.125 °C LSB):
///   bit 15..5 = integer + fractional temperature
///   bit 4..0  = 0 (don't care)
///
/// We return 0.1 °C units.
fn read_lm75(addr: u8) -> Option<i32> {
    // SMBus Word Data Read returns (DAT0 | DAT1<<8), but LM75 sends MSB first.
    // We need to byte-swap so that the result is big-endian as the device intended.
    let raw = crate::drivers::smbus::read_word(addr, 0x00)?;
    // raw = [DAT1:DAT0] (little-endian word from SMBus host)
    // LM75 sends temp_msb first (high byte), temp_lsb second (low byte)
    // Some ICH controllers swap bytes; treat DAT0 as high byte, DAT1 as low byte.
    let be_word = ((raw & 0xFF) << 8) | ((raw >> 8) & 0xFF);

    // Bits [15:5] = temperature in 1/8 °C units (signed)
    // Shift right 5, treating as signed 11-bit.
    let raw_11: i32 = ((be_word as i32) >> 5) as i32;
    // Extend sign from bit 10
    let raw_signed = if raw_11 & 0x400 != 0 {
        raw_11 | !0x3FF
    } else {
        raw_11
    };

    // raw_signed is in 1/8 °C units; convert to 0.1 °C: multiply by 10 / 8 = 5/4
    // (exact for multiples of 0.125, small rounding otherwise)
    let temp_x10 = (raw_signed * 10) / 8;
    Some(temp_x10)
}

/// Read a Model Specific Register.
///
/// # Safety
/// RDMSR with an invalid MSR causes a General Protection Fault.
/// Callers guard this with CPUID checks before invoking.
#[inline]
pub(crate) fn read_msr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    // Safety: caller is responsible for checking the MSR exists via CPUID.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nostack, preserves_flags),
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Execute CPUID with leaf `eax` and sub-leaf `ecx`.
/// Returns (EAX, EBX, ECX, EDX).
///
/// Note: rbx/ebx cannot be an asm output operand because LLVM reserves it as
/// a base pointer on some targets.  We save and restore it manually.
#[inline]
pub(crate) fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            // Save rbx, execute cpuid, move result out, restore rbx.
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            ebx_out = out(reg) ebx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    (eax, ebx, ecx, edx)
}

/// Return the three CPUID vendor registers [EBX, ECX, EDX] from leaf 0.
#[inline]
pub(crate) fn cpuid_vendor() -> [u32; 3] {
    let (_, ebx, ecx, edx) = cpuid(0, 0);
    [ebx, ecx, edx]
}
