//! ChaCha20-Poly1305 AEAD (RFC 8439).

use crate::crypto::chacha20;
use crate::crypto::poly1305;

/// Encrypt and authenticate with ChaCha20-Poly1305.
///
/// - `key`: 32 bytes
/// - `nonce`: 12 bytes
/// - `aad`: additional authenticated data
/// - `plaintext`: modified in place to ciphertext
/// - `tag`: 16-byte output
pub fn encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &mut [u8],
    tag: &mut [u8; 16],
) {
    // Generate Poly1305 key from first ChaCha20 block
    let poly_key_block = chacha20::chacha20_block(key, 0, nonce);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_key_block[..32]);

    // Encrypt with ChaCha20 starting at counter 1
    chacha20::chacha20_crypt(key, 1, nonce, plaintext);

    // Compute Poly1305 tag over construction:
    // AAD || pad(AAD) || ciphertext || pad(CT) || len(AAD) || len(CT)
    let mac_data = build_mac_data(aad, plaintext);
    *tag = poly1305::poly1305(&poly_key, &mac_data);
}

/// Decrypt and verify with ChaCha20-Poly1305.
///
/// Returns `true` if the tag is valid. On failure, ciphertext is zeroed.
pub fn decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &mut [u8],
    tag: &[u8; 16],
) -> bool {
    let poly_key_block = chacha20::chacha20_block(key, 0, nonce);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&poly_key_block[..32]);

    // Verify tag over ciphertext (before decryption)
    let mac_data = build_mac_data(aad, ciphertext);
    let computed = poly1305::poly1305(&poly_key, &mac_data);

    // Constant-time comparison
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= tag[i] ^ computed[i];
    }

    if diff != 0 {
        for b in ciphertext.iter_mut() {
            *b = 0;
        }
        return false;
    }

    // Decrypt
    chacha20::chacha20_crypt(key, 1, nonce, ciphertext);
    true
}

/// Build the Poly1305 MAC construction data per RFC 8439 Section 2.8.
fn build_mac_data(aad: &[u8], ct: &[u8]) -> alloc::vec::Vec<u8> {
    let aad_padded = pad16(aad.len());
    let ct_padded = pad16(ct.len());
    let total = aad.len() + aad_padded + ct.len() + ct_padded + 16;

    let mut data = alloc::vec::Vec::with_capacity(total);
    data.extend_from_slice(aad);
    data.resize(data.len() + aad_padded, 0);
    data.extend_from_slice(ct);
    data.resize(data.len() + ct_padded, 0);
    data.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    data.extend_from_slice(&(ct.len() as u64).to_le_bytes());
    data
}

fn pad16(len: usize) -> usize {
    if len % 16 == 0 {
        0
    } else {
        16 - (len % 16)
    }
}
