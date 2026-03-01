//! ARM64 CPU feature detection via ID registers.

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the CPU supports Atomics (LSE) — FEAT_LSE.
pub static HAS_LSE: AtomicBool = AtomicBool::new(false);

/// Whether the CPU supports CRC32 instructions.
pub static HAS_CRC32: AtomicBool = AtomicBool::new(false);

/// Whether the CPU supports SHA-256 instructions.
pub static HAS_SHA256: AtomicBool = AtomicBool::new(false);

/// Whether the CPU supports AES instructions.
pub static HAS_AES: AtomicBool = AtomicBool::new(false);

/// Whether the CPU supports RNG (FEAT_RNG, RNDR instruction).
pub static HAS_RNG: AtomicBool = AtomicBool::new(false);

/// Detect CPU features from ID registers.
pub fn detect() {
    let midr: u64;
    let isar0: u64;
    let isar1: u64;
    let pfr0: u64;

    unsafe {
        core::arch::asm!("mrs {}, midr_el1", out(reg) midr, options(nomem, nostack));
        core::arch::asm!("mrs {}, id_aa64isar0_el1", out(reg) isar0, options(nomem, nostack));
        core::arch::asm!("mrs {}, id_aa64isar1_el1", out(reg) isar1, options(nomem, nostack));
        core::arch::asm!("mrs {}, id_aa64pfr0_el1", out(reg) pfr0, options(nomem, nostack));
    }

    let implementer = (midr >> 24) & 0xFF;
    let variant = (midr >> 20) & 0xF;
    let part = (midr >> 4) & 0xFFF;
    let revision = midr & 0xF;

    crate::serial_println!(
        "CPU: impl={:#04x} part={:#05x} variant={} revision={}",
        implementer, part, variant, revision,
    );

    // ID_AA64ISAR0_EL1 fields
    let atomic = (isar0 >> 20) & 0xF;
    HAS_LSE.store(atomic >= 2, Ordering::Relaxed);

    let crc32 = (isar0 >> 16) & 0xF;
    HAS_CRC32.store(crc32 >= 1, Ordering::Relaxed);

    let sha2 = (isar0 >> 12) & 0xF;
    HAS_SHA256.store(sha2 >= 1, Ordering::Relaxed);

    let aes = (isar0 >> 4) & 0xF;
    HAS_AES.store(aes >= 1, Ordering::Relaxed);

    let rndr = (isar0 >> 60) & 0xF;
    HAS_RNG.store(rndr >= 1, Ordering::Relaxed);

    crate::serial_println!(
        "  Features: LSE={} CRC32={} SHA256={} AES={} RNG={}",
        HAS_LSE.load(Ordering::Relaxed),
        HAS_CRC32.load(Ordering::Relaxed),
        HAS_SHA256.load(Ordering::Relaxed),
        HAS_AES.load(Ordering::Relaxed),
        HAS_RNG.load(Ordering::Relaxed),
    );
}
