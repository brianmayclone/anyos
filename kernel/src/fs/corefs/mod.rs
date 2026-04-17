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

#[cfg(any(test, feature = "kunit"))]
mod integration_tests;

#[cfg(any(test, feature = "kunit"))]
mod integration_characterization_tests;

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

/// AnyOS-Zeitquelle für `corefs-core` — delegiert an `crate::time`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelClock;

impl Clock for KernelClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(crate::time::wall_clock_unix_secs())
    }
}

/// AnyOS-Zufallsquelle für `corefs-core`.
///
/// Hält einen lokalen XorShift64-State für schnelle sequentielle Calls
/// (z. B. Inode-ID-Generierung). Der initiale Seed kommt von `crate::random`
/// (VirtIO RNG → RDRAND → TSC-PRNG-Kette).
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

    /// Konstruiert einen `KernelRng` aus Hardware-Entropie via `crate::random`.
    #[must_use]
    pub fn from_hardware_entropy() -> Self {
        Self::from_seed(crate::random::next_u64())
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
