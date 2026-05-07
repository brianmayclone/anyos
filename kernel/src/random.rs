//! Zentrale Kernel-Entropiequelle.
//!
//! Prioritätskette:
//! 1. VirtIO RNG (hardware entropy vom Hypervisor)
//! 2. RDRAND (Intel/AMD hardware RNG)
//! 3. TSC + Thread-ID + Code-Adresse (PRNG-Fallback, schwächste Quelle)
//!
//! Alle Kernel-Subsysteme (CoreFS, Netzwerk-Stack, …) sollen ausschließlich
//! diese API nutzen statt eigene Entropie-Logik zu implementieren.

use core::sync::atomic::{AtomicU64, Ordering};

// ── Globaler PRNG-State (Fallback) ───────────────────────────────────────────

/// Globaler SplitMix64-State für den PRNG-Fallback.
/// Wird beim ersten Aufruf von `fill_random` mit Hardware-Entropie initialisiert.
static PRNG_STATE: AtomicU64 = AtomicU64::new(0);

// ── Interne Hilfsfunktionen ──────────────────────────────────────────────────

/// Versucht, einen u64 via RDRAND zu lesen (bis zu 10 Retries, per Intel SDM).
#[cfg(target_arch = "x86_64")]
fn try_rdrand() -> Option<u64> {
    let feats = crate::arch::x86::cpuid::features();
    if !feats.rdrand {
        return None;
    }
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
fn try_rdrand() -> Option<u64> {
    None
}

/// SplitMix64-Schritt.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Erzeugt einen Hardware-Seed (RDRAND × 4 gemischt mit TSC, Fallback: TSC +
/// Thread-ID + Code-Adresse).
fn hardware_seed() -> u64 {
    #[cfg(target_arch = "x86_64")]
    let tsc = crate::arch::x86::pit::rdtsc();
    #[cfg(not(target_arch = "x86_64"))]
    let tsc: u64 = 0;

    let mut seed = tsc;
    let mut got_rdrand = false;

    for _ in 0..4 {
        if let Some(x) = try_rdrand() {
            seed ^= x;
            seed = splitmix64(&mut seed);
            got_rdrand = true;
        }
    }

    if !got_rdrand {
        let tid = crate::task::scheduler::current_tid() as u64;
        let addr = (hardware_seed as fn() -> u64) as usize as u64;
        seed ^= tid.rotate_left(17) ^ addr.rotate_left(31);
        seed = splitmix64(&mut seed);
    }

    if seed == 0 {
        1
    } else {
        seed
    }
}

/// Stellt sicher, dass `PRNG_STATE` initialisiert ist, und liefert den
/// nächsten atomisch aktualisierten Zustand.
fn prng_next() -> u64 {
    loop {
        let old = PRNG_STATE.load(Ordering::Relaxed);
        let next = if old == 0 { hardware_seed() } else { old };
        let mut state = next;
        let out = splitmix64(&mut state);
        // CAS: wenn ein anderer Kern dazwischen gewechselt hat, retry.
        if PRNG_STATE
            .compare_exchange_weak(old, state, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return out;
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn fill_virtio_random(buf: &mut [u8]) -> usize {
    if crate::drivers::virtio::rng::is_available() {
        crate::drivers::virtio::rng::fill_random(buf)
    } else {
        0
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn fill_virtio_random(_buf: &mut [u8]) -> usize {
    0
}

// ── Öffentliche API ──────────────────────────────────────────────────────────

/// Füllt `buf` mit zufälligen Bytes.
///
/// Nutzt VirtIO RNG wenn vorhanden, danach RDRAND (Remainder), danach PRNG.
pub fn fill_random(buf: &mut [u8]) {
    if buf.is_empty() {
        return;
    }

    let mut filled = 0;

    // 1. VirtIO RNG (beste Qualität, nur im echten Betrieb verfügbar)
    filled = fill_virtio_random(buf);

    // 2. RDRAND für den Rest (oder den ganzen Puffer, falls kein VirtIO)
    if filled < buf.len() {
        let rest = &mut buf[filled..];
        for chunk in rest.chunks_mut(8) {
            match try_rdrand() {
                Some(x) => {
                    let bytes = x.to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                    filled += chunk.len();
                }
                None => break,
            }
        }
    }

    // 3. PRNG-Fallback für den noch nicht gefüllten Rest
    if filled < buf.len() {
        let rest = &mut buf[filled..];
        for chunk in rest.chunks_mut(8) {
            let bytes = prng_next().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// Liefert einen zufälligen u64-Wert.
pub fn next_u64() -> u64 {
    // Versuche zuerst RDRAND (direkt, kein Umweg über fill_random)
    if let Some(x) = try_rdrand() {
        return x;
    }
    // VirtIO RNG für genau 8 Bytes
    let mut buf = [0u8; 8];
    if fill_virtio_random(&mut buf) == 8 {
        return u64::from_le_bytes(buf);
    }
    prng_next()
}
