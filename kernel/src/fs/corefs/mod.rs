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
pub use driver::{corefs_to_fs_error, CoreFsDriver};
pub use probe::detect;

use corefs_core::platform::{Clock, Rng, Timestamp};

/// AnyOS-Zeitquelle für `corefs-core`.
///
/// Der konkrete `now()`-Wert ist aktuell ein Platzhalter — sobald die
/// AnyOS-Wand-Uhr-Schnittstelle (z. B. RTC-Driver) verfügbar ist, wird sie
/// hier eingehängt.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelClock;

impl Clock for KernelClock {
    fn now(&self) -> Timestamp {
        // TODO: an die echte AnyOS-RTC anbinden, sobald `crate::time` vorliegt.
        // Bis dahin liefern wir Epoch — Aufrufer, die einen monotonen Counter
        // brauchen, müssen das selbst sicherstellen.
        Timestamp::EPOCH
    }
}

/// AnyOS-Zufallsquelle für `corefs-core`.
///
/// Aktuell ein xorshift64-Stub mit Compile-Time-Seed. Sobald der
/// AnyOS-Entropiepool steht, wird dieser Stub durch die echte Quelle ersetzt.
#[derive(Debug, Clone)]
pub struct KernelRng {
    state: u64,
}

impl KernelRng {
    /// Konstruiert einen neuen `KernelRng` mit explizitem Seed.
    ///
    /// Der Aufrufer ist dafür verantwortlich, einen ausreichend zufälligen
    /// Seed zu liefern (z. B. aus RDRAND, einer TSC-basierten Mischung
    /// oder Boot-Time-Entropie).
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }
}

impl Default for KernelRng {
    fn default() -> Self {
        // Platzhalter-Seed; Boot-Code soll später `from_seed(rdrand())` nutzen.
        Self::from_seed(0xCAFEBABE_DEADBEEF)
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
    fn kernel_clock_returns_epoch_for_now() {
        let c = KernelClock;
        assert_eq!(c.now(), Timestamp::EPOCH);
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
