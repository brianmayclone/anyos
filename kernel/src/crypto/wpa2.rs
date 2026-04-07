//! WPA2-specific key derivation helpers shared by the WiFi stack.

use crate::crypto::hmac::hmac_sha1;
use crate::crypto::sha1::SHA1_DIGEST_SIZE;
use alloc::vec::Vec;

/// Pairwise Transient Key derived from PMK + nonces + MACs.
/// Layout: KCK(16) | KEK(16) | TK(16) = 48 bytes from PRF-384.
#[derive(Debug, Clone)]
pub struct Ptk {
    /// Key Confirmation Key (used to compute/verify MIC).
    pub kck: [u8; 16],
    /// Key Encryption Key (used to decrypt GTK in message 3).
    pub kek: [u8; 16],
    /// Temporal Key (used for CCMP data encryption/decryption).
    pub tk: [u8; 16],
}

/// Compare two byte slices lexicographically; returns true if a < b.
fn bytes_lt(a: &[u8], b: &[u8]) -> bool {
    for i in 0..a.len().min(b.len()) {
        if a[i] < b[i] {
            return true;
        }
        if a[i] > b[i] {
            return false;
        }
    }
    a.len() < b.len()
}

/// Derive the PTK from PMK, nonces, and MAC addresses.
pub fn derive_ptk(
    pmk: &[u8; 32],
    anonce: &[u8; 32],
    snonce: &[u8; 32],
    ap_mac: &[u8; 6],
    sta_mac: &[u8; 6],
) -> Ptk {
    let label = b"Pairwise key expansion";

    let mut b_data = [0u8; 6 + 6 + 32 + 32];
    if bytes_lt(ap_mac, sta_mac) {
        b_data[0..6].copy_from_slice(ap_mac);
        b_data[6..12].copy_from_slice(sta_mac);
    } else {
        b_data[0..6].copy_from_slice(sta_mac);
        b_data[6..12].copy_from_slice(ap_mac);
    }
    if bytes_lt(anonce, snonce) {
        b_data[12..44].copy_from_slice(anonce);
        b_data[44..76].copy_from_slice(snonce);
    } else {
        b_data[12..44].copy_from_slice(snonce);
        b_data[44..76].copy_from_slice(anonce);
    }

    let mut prf_out = [0u8; 60];
    for counter in 0u8..3u8 {
        let mut prf_input = Vec::with_capacity(label.len() + 1 + b_data.len() + 1);
        prf_input.extend_from_slice(label);
        prf_input.push(0x00);
        prf_input.extend_from_slice(&b_data);
        prf_input.push(counter);

        let mac = hmac_sha1(pmk, &prf_input);
        let start = counter as usize * SHA1_DIGEST_SIZE;
        prf_out[start..start + SHA1_DIGEST_SIZE].copy_from_slice(&mac);
    }

    let mut kck = [0u8; 16];
    let mut kek = [0u8; 16];
    let mut tk = [0u8; 16];
    kck.copy_from_slice(&prf_out[0..16]);
    kek.copy_from_slice(&prf_out[16..32]);
    tk.copy_from_slice(&prf_out[32..48]);

    Ptk { kck, kek, tk }
}

/// Compute the 16-byte WPA2 EAPOL MIC from an HMAC-SHA1 output.
pub fn compute_mic(kck: &[u8; 16], frame: &[u8]) -> [u8; 16] {
    let full_mac = hmac_sha1(kck, frame);
    let mut mic = [0u8; 16];
    mic.copy_from_slice(&full_mac[..16]);
    mic
}
