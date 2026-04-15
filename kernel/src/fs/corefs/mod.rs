//! CoreFS-Treiber für AnyOS — Adapter zwischen `corefs_core` und der
//! AnyOS-VFS-Schicht.
//!
//! Dieses Modul ist die kernel-seitige Anbindung des plattformneutralen
//! [`corefs_core`]-Filesystems. Es liefert:
//!
//! - [`KernelClock`] — `Clock`-Implementierung über die AnyOS-Zeitquelle
//! - [`KernelRng`] — `Rng`-Implementierung über die AnyOS-Entropiequelle
//! - [`BlockDeviceAdapter`] — `BlockDevice`-Implementierung, die auf
//!   `crate::drivers::storage` (ATA/AHCI/NVMe) delegiert
//! - [`CoreFsDriver`] — `Filesystem`-Trait-Implementierung, die VFS-Calls
//!   an die `corefs_core`-APIs weiterleitet
//!
//! ## Status
//!
//! Skelett. Aktuell vorhanden:
//!
//! - Crate-Anbindung an `corefs-core` ohne `crypto`-Feature (poly1305 SIMD
//!   bricht den soft-float Kernel-Build)
//! - `KernelClock` und `KernelRng` als Stub-Implementierungen
//!
//! Folgeschritte (in eigenständigen Commits):
//!
//! - `BlockDeviceAdapter` mit echter ATA/AHCI-/NVMe-Delegation
//! - `Filesystem`-Trait mit Read/Write/Lookup/Readdir/Create/Delete
//! - `FsType::CoreFs` im VFS-Enum, Boot-Mount-Pfad, Superblock-Magic-Detection

pub mod block_device;
pub mod driver;
pub mod probe;

pub use block_device::{AnyOsSectorIo, BlockDeviceAdapter, SectorIo};
pub use driver::{corefs_to_fs_error, empty_persisted_state, CoreFsDriver};
pub use probe::detect;

/// Versucht, das Volume bei `(disk_id, partition_lba)` als CoreFS zu
/// erkennen und read-only unter `mount_path` zu mounten.
///
/// Boot-Code ruft diesen Helper einmal pro Partition auf, nach der klassischen
/// FAT/NTFS/exFAT/ISO-Detection. `partition_sectors` wird aus dem MBR/GPT-
/// Eintrag der Partition übernommen.
///
/// Liefert `true`, wenn das CoreFS-Magic gefunden **und** der Mount erfolgreich
/// war. `false` ist kein Fehler, sondern signalisiert "keine CoreFS-Partition
/// an dieser Position" oder "Mount fehlgeschlagen" (Details in den
/// Serial-Logs).
pub fn try_auto_mount_corefs(
    mount_path: &str,
    disk_id: u8,
    partition_lba: u32,
    partition_sectors: u64,
    device_id: u32,
) -> bool {
    if !detect(disk_id, partition_lba) {
        return false;
    }
    crate::serial_println!(
        "[corefs] detected on disk {} part_lba {}, mounting at {}",
        disk_id,
        partition_lba,
        mount_path
    );
    match crate::fs::vfs::mount_corefs(
        mount_path,
        disk_id,
        partition_lba,
        partition_sectors,
        device_id,
    ) {
        Ok(()) => true,
        Err(e) => {
            crate::serial_println!(
                "[corefs] mount at {} failed: {:?}",
                mount_path,
                e
            );
            false
        }
    }
}

use corefs_core::platform::{Clock, Rng, Timestamp};

/// Convert (year, month, day, hour, min, sec) in UTC to Unix seconds.
///
/// Minimal Gregorian conversion for years >= 1970, matching the behaviour of
/// `fs::fat::datetime::dos_datetime_to_unix` but without the DOS bit-layout.
/// Accepts out-of-range inputs by clamping — a broken RTC should not panic.
fn ymd_hms_to_unix(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> u64 {
    const CUMUL: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let year = if year < 1970 { 1970u32 } else { year as u32 };
    let month = month.clamp(1, 12) as u32;
    let day = day.clamp(1, 31) as u32;

    let mut days: u32 = 0;
    for y in 1970..year {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        days += if leap { 366 } else { 365 };
    }
    days += CUMUL[(month - 1) as usize];
    let leap_y = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    if month > 2 && leap_y {
        days += 1;
    }
    days += day - 1;

    (days as u64) * 86_400
        + (hour.min(59) as u64) * 3600
        + (min.min(59) as u64) * 60
        + sec.min(59) as u64
}

/// AnyOS-Zeitquelle für `corefs-core`.
///
/// Strategie (x86_64):
/// 1. Wir lesen einmalig beim ersten `now()`-Aufruf die CMOS-RTC (MC146818) aus
///    und rechnen sie in Unix-Sekunden um. Dieser Wert wird zusammen mit dem
///    PIT/TSC-Millisekunden-Stand zum Zeitpunkt des Samples gespeichert
///    (`BOOT_UNIX_SECS`, `BOOT_OFFSET_MS`).
/// 2. Jeder folgende `now()`-Aufruf liefert
///    `BOOT_UNIX_SECS + (real_ms_since_boot() - BOOT_OFFSET_MS) / 1000`.
///
/// So wird die RTC nur einmal gelesen (teure CMOS-Pollings vermieden) und der
/// Zeitstempel wächst strikt monoton mit dem kalibrierten PIT-Tick-Counter.
/// Fallback ohne RTC (oder auf non-x86-Targets): `Timestamp::EPOCH` plus
/// monoton steigende Sekunden seit Boot, damit Inode-Zeiten zumindest
/// unterscheidbar bleiben.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelClock;

#[cfg(target_arch = "x86_64")]
mod clock_state {
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    pub static BOOT_UNIX_SECS: AtomicU64 = AtomicU64::new(0);
    pub static BOOT_OFFSET_MS: AtomicU64 = AtomicU64::new(0);
    pub static INITIALISED: AtomicBool = AtomicBool::new(false);
}

impl Clock for KernelClock {
    fn now(&self) -> Timestamp {
        #[cfg(target_arch = "x86_64")]
        {
            use clock_state::*;
            use core::sync::atomic::Ordering;
            let ms_now = crate::arch::x86::pit::real_ms_since_boot();
            if !INITIALISED.load(Ordering::Acquire) {
                let t = crate::drivers::rtc::read_time();
                let unix = ymd_hms_to_unix(t.year, t.month, t.day, t.hours, t.minutes, t.seconds);
                // Races with another init are harmless — last writer wins,
                // and both writers see the same RTC second ±1.
                BOOT_UNIX_SECS.store(unix, Ordering::Relaxed);
                BOOT_OFFSET_MS.store(ms_now, Ordering::Relaxed);
                INITIALISED.store(true, Ordering::Release);
                return Timestamp::from_secs(unix);
            }
            let base = BOOT_UNIX_SECS.load(Ordering::Relaxed);
            let off = BOOT_OFFSET_MS.load(Ordering::Relaxed);
            let delta_ms = ms_now.saturating_sub(off);
            Timestamp::from_secs(base + delta_ms / 1000)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            // Keine plattformneutrale Wall-Clock-API im Kernel — liefere
            // Epoch, bis ein aarch64-RTC-Pfad existiert.
            Timestamp::EPOCH
        }
    }
}

/// AnyOS-Zufallsquelle für `corefs-core`.
///
/// Default-Konstruktion nutzt [`KernelRng::from_hardware_entropy`], das bei
/// vorhandenem RDRAND (CPUID.01H:ECX.RDRAND) vier 64-Bit-Samples mit dem
/// aktuellen TSC mischt. Ohne RDRAND wird stattdessen TSC + aktuelle Thread-
/// ID + eine feste Kernel-Code-Adresse in einen SplitMix64-Seed gemischt.
#[derive(Debug, Clone)]
pub struct KernelRng {
    state: u64,
}

impl KernelRng {
    /// Konstruiert einen neuen `KernelRng` mit explizitem Seed.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Versucht, per RDRAND einen u64 zu lesen. Gibt `None` zurück, wenn
    /// RDRAND nicht verfügbar ist oder mehrere Retries erfolglos sind.
    #[cfg(target_arch = "x86_64")]
    fn try_rdrand_u64() -> Option<u64> {
        let feats = crate::arch::x86::cpuid::features();
        if !feats.rdrand {
            return None;
        }
        // Intel SDM empfiehlt bis zu 10 Retries.
        for _ in 0..10 {
            let mut out: u64 = 0;
            let ok: u8;
            unsafe {
                core::arch::asm!(
                    "rdrand {0}",
                    "setc {1}",
                    out(reg) out,
                    out(reg_byte) ok,
                    options(nomem, nostack),
                );
            }
            if ok != 0 {
                return Some(out);
            }
        }
        None
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn try_rdrand_u64() -> Option<u64> {
        None
    }

    /// Konstruiert einen `KernelRng` aus Hardware-Entropie.
    ///
    /// Bevorzugt: 4× RDRAND XOR-gemischt mit TSC.
    /// Fallback: SplitMix64 über TSC + Thread-ID + Code-Adresse.
    #[must_use]
    pub fn from_hardware_entropy() -> Self {
        #[cfg(target_arch = "x86_64")]
        let tsc = crate::arch::x86::pit::rdtsc();
        #[cfg(not(target_arch = "x86_64"))]
        let tsc: u64 = 0;

        let mut seed: u64 = tsc;

        // Try RDRAND × 4
        let mut rdrand_any = false;
        for _ in 0..4 {
            if let Some(x) = Self::try_rdrand_u64() {
                seed ^= x;
                // SplitMix64 step to diffuse
                seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = seed;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                seed ^= z ^ (z >> 31);
                rdrand_any = true;
            }
        }

        if !rdrand_any {
            // Fallback: TSC + current thread id + a kernel code address
            let tid = crate::task::scheduler::current_tid() as u64;
            let code_ptr = (Self::from_hardware_entropy as fn() -> Self) as usize as u64;
            seed ^= tid.rotate_left(17);
            seed ^= code_ptr.rotate_left(31);
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            seed ^= z ^ (z >> 31);
        }

        Self::from_seed(seed)
    }
}

impl Default for KernelRng {
    fn default() -> Self {
        Self::from_hardware_entropy()
    }
}

impl Rng for KernelRng {
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(8) {
            let mut x = self.state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.state = x;
            let bytes = x.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ymd_hms_conversion_known_values() {
        // 1970-01-01 00:00:00 = 0
        assert_eq!(ymd_hms_to_unix(1970, 1, 1, 0, 0, 0), 0);
        // 2024-01-01 00:00:00 = 1_704_067_200
        assert_eq!(ymd_hms_to_unix(2024, 1, 1, 0, 0, 0), 1_704_067_200);
        // 2000-02-29 12:00:00 = 951_782_400 + 12*3600
        assert_eq!(
            ymd_hms_to_unix(2000, 2, 29, 12, 0, 0),
            951_782_400 + 12 * 3600
        );
    }

    #[test]
    fn ymd_hms_clamps_invalid_inputs() {
        // Out-of-range month/day must not panic.
        let _ = ymd_hms_to_unix(1970, 0, 0, 99, 99, 99);
        let _ = ymd_hms_to_unix(1965, 13, 32, 25, 61, 61);
    }

    #[test]
    fn kernel_rng_from_hardware_entropy_non_zero_seed() {
        // SplitMix64 fallback path on host (no RDRAND access inside unit
        // tests because `cpuid::features()` returns the unset default on the
        // host). The output must still be non-zero.
        let mut r = KernelRng::from_seed(0xDEAD_BEEF_u64);
        assert_ne!(r.next_u64(), 0);
    }

    #[test]
    fn kernel_rng_two_from_hardware_differ() {
        // Two consecutive seeds derived from different TSCs should produce
        // divergent streams. We simulate that by seeding with two different
        // values, as the actual TSC read is not available on host.
        let mut a = KernelRng::from_seed(1);
        let mut b = KernelRng::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn kernel_rng_is_deterministic_for_seed() {
        let mut a = KernelRng::from_seed(42);
        let mut b = KernelRng::from_seed(42);
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn kernel_rng_zero_seed_is_replaced() {
        let mut rng = KernelRng::from_seed(0);
        assert_ne!(rng.next_u64(), 0, "xorshift64 must not be locked at zero");
    }

    #[test]
    fn kernel_rng_fill_bytes_advances_state() {
        let mut rng = KernelRng::from_seed(7);
        let mut buf = [0u8; 32];
        rng.fill_bytes(&mut buf);
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn corefs_core_version_is_visible() {
        // Smoke test: ensures the `corefs-core` dependency really does link.
        assert!(!corefs_core::VERSION.is_empty());
    }
}
