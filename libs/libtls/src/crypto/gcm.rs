//! AES-GCM AEAD (NIST SP 800-38D).
//!
//! Provides authenticated encryption/decryption using AES in CTR mode
//! with GHASH for authentication.

use crate::crypto::aes::{AesKey, BLOCK_SIZE};

/// AES-GCM context.
pub struct AesGcm {
    key: AesKey,
    h: [u8; 16], // GHASH key: AES_K(0^128)
}

impl AesGcm {
    /// Create AES-128-GCM context.
    pub fn new_128(key: &[u8; 16]) -> Self {
        let aes_key = AesKey::new_128(key);
        let mut h = [0u8; 16];
        aes_key.encrypt_block(&mut h);
        Self { key: aes_key, h }
    }

    /// Create AES-256-GCM context.
    pub fn new_256(key: &[u8; 32]) -> Self {
        let aes_key = AesKey::new_256(key);
        let mut h = [0u8; 16];
        aes_key.encrypt_block(&mut h);
        Self { key: aes_key, h }
    }

    /// Encrypt and authenticate.
    ///
    /// - `nonce`: 12 bytes
    /// - `aad`: additional authenticated data
    /// - `plaintext`: data to encrypt (modified in place to ciphertext)
    /// - `tag`: 16-byte output authentication tag
    pub fn encrypt(&self, nonce: &[u8; 12], aad: &[u8], plaintext: &mut [u8], tag: &mut [u8; 16]) {
        // J0 = nonce || 0x00000001
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;

        // Encrypt plaintext with CTR mode starting at J0 + 1
        let mut counter = j0;
        inc32(&mut counter);
        ctr_crypt(&self.key, &mut counter, plaintext);

        // GHASH
        let ghash = ghash(&self.h, aad, plaintext);

        // Tag = GHASH XOR AES_K(J0)
        let mut enc_j0 = j0;
        self.key.encrypt_block(&mut enc_j0);
        for i in 0..16 {
            tag[i] = ghash[i] ^ enc_j0[i];
        }
    }

    /// Decrypt and verify.
    ///
    /// Returns `true` if the tag is valid, `false` otherwise.
    /// On success, `ciphertext` is replaced with plaintext.
    /// On failure, `ciphertext` is zeroed.
    pub fn decrypt(
        &self,
        nonce: &[u8; 12],
        aad: &[u8],
        ciphertext: &mut [u8],
        tag: &[u8; 16],
    ) -> bool {
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;

        // Compute GHASH over AAD and ciphertext (before decryption)
        let ghash = ghash(&self.h, aad, ciphertext);

        // Expected tag
        let mut enc_j0 = j0;
        self.key.encrypt_block(&mut enc_j0);
        let mut expected_tag = [0u8; 16];
        for i in 0..16 {
            expected_tag[i] = ghash[i] ^ enc_j0[i];
        }

        // Constant-time tag comparison
        let mut diff = 0u8;
        for i in 0..16 {
            diff |= tag[i] ^ expected_tag[i];
        }

        if diff != 0 {
            // Tag mismatch -- zero ciphertext to prevent use of unauthenticated data
            for b in ciphertext.iter_mut() {
                *b = 0;
            }
            return false;
        }

        // Decrypt
        let mut counter = j0;
        inc32(&mut counter);
        ctr_crypt(&self.key, &mut counter, ciphertext);

        true
    }
}

/// Increment the rightmost 32 bits of a 128-bit counter.
fn inc32(counter: &mut [u8; 16]) {
    let mut c = u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]);
    c = c.wrapping_add(1);
    counter[12..16].copy_from_slice(&c.to_be_bytes());
}

/// AES-CTR encryption/decryption.
fn ctr_crypt(key: &AesKey, counter: &mut [u8; 16], data: &mut [u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let mut keystream = *counter;
        key.encrypt_block(&mut keystream);
        inc32(counter);

        let chunk = (data.len() - offset).min(BLOCK_SIZE);
        for i in 0..chunk {
            data[offset + i] ^= keystream[i];
        }
        offset += chunk;
    }
}

/// GHASH: universal hash function for GCM.
///
/// GHASH(H, A, C) where A = AAD, C = ciphertext.
fn ghash(h: &[u8; 16], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut y = [0u8; 16];

    // Process AAD
    ghash_update(&mut y, h, aad);

    // Process ciphertext
    ghash_update(&mut y, h, ciphertext);

    // Final block: len(A) || len(C) in bits, as 64-bit big-endian
    let mut len_block = [0u8; 16];
    let aad_bits = (aad.len() as u64) * 8;
    let ct_bits = (ciphertext.len() as u64) * 8;
    len_block[..8].copy_from_slice(&aad_bits.to_be_bytes());
    len_block[8..].copy_from_slice(&ct_bits.to_be_bytes());
    xor_block(&mut y, &len_block);
    gf128_mul(&mut y, h);

    y
}

/// Process data through GHASH in 16-byte blocks (with zero-padding).
fn ghash_update(y: &mut [u8; 16], h: &[u8; 16], data: &[u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let mut block = [0u8; 16];
        let chunk = (data.len() - offset).min(16);
        block[..chunk].copy_from_slice(&data[offset..offset + chunk]);
        xor_block(y, &block);
        gf128_mul(y, h);
        offset += 16;
    }
}

/// GF(2^128) multiplication (used by GHASH).
///
/// Multiply `a` by `b` in GF(2^128) with the GCM polynomial
/// x^128 + x^7 + x^2 + x + 1 (0xE1 reduction).
fn gf128_mul(a: &mut [u8; 16], b: &[u8; 16]) {
    let mut z = [0u8; 16];
    let mut v = *b;

    for i in 0..128 {
        // If bit i of a is set
        if a[i / 8] & (0x80 >> (i % 8)) != 0 {
            xor_block(&mut z, &v);
        }
        // v = v >> 1 in GF(2^128)
        let carry = v[15] & 1;
        for j in (1..16).rev() {
            v[j] = (v[j] >> 1) | (v[j - 1] << 7);
        }
        v[0] >>= 1;
        if carry != 0 {
            v[0] ^= 0xE1; // Reduction polynomial
        }
    }

    *a = z;
}

fn xor_block(a: &mut [u8; 16], b: &[u8; 16]) {
    for i in 0..16 {
        a[i] ^= b[i];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes128_gcm_nist() {
        // NIST GCM test vector (Test Case 3 from SP 800-38D)
        let key: [u8; 16] = [
            0xfe, 0xff, 0xe9, 0x92, 0x86, 0x65, 0x73, 0x1c, 0x6d, 0x6a, 0x8f, 0x94, 0x67, 0x30,
            0x83, 0x08,
        ];
        let nonce: [u8; 12] = [
            0xca, 0xfe, 0xba, 0xbe, 0xfa, 0xce, 0xdb, 0xad, 0xde, 0xca, 0xf8, 0x88,
        ];
        let mut plaintext: [u8; 64] = [
            0xd9, 0x31, 0x32, 0x25, 0xf8, 0x84, 0x06, 0xe5, 0xa5, 0x59, 0x09, 0xc5, 0xaf, 0xf5,
            0x26, 0x9a, 0x86, 0xa7, 0xa9, 0x53, 0x15, 0x34, 0xf7, 0xda, 0x2e, 0x4c, 0x30, 0x3d,
            0x8a, 0x31, 0x8a, 0x72, 0x1c, 0x3c, 0x0c, 0x95, 0x95, 0x68, 0x09, 0x53, 0x2f, 0xcf,
            0x0e, 0x24, 0x49, 0xa6, 0xb5, 0x25, 0xb1, 0x6a, 0xed, 0xf5, 0xaa, 0x0d, 0xe6, 0x57,
            0xba, 0x63, 0x7b, 0x39, 0x1a, 0xaf, 0xd2, 0x55,
        ];
        let expected_ct: [u8; 64] = [
            0x42, 0x83, 0x1e, 0xc2, 0x21, 0x77, 0x74, 0x24, 0x4b, 0x72, 0x21, 0xb7, 0x84, 0xd0,
            0xd4, 0x9c, 0xe3, 0xaa, 0x21, 0x2f, 0x2c, 0x02, 0xa4, 0xe0, 0x35, 0xc1, 0x7e, 0x23,
            0x29, 0xac, 0xa1, 0x2e, 0x21, 0xd5, 0x14, 0xb2, 0x54, 0x66, 0x93, 0x1c, 0x7d, 0x8f,
            0x6a, 0x5a, 0xac, 0x84, 0xaa, 0x05, 0x1b, 0xa3, 0x0b, 0x39, 0x6a, 0x0a, 0xac, 0x97,
            0x3d, 0x58, 0xe0, 0x91, 0x47, 0x3f, 0x59, 0x85,
        ];
        let expected_tag: [u8; 16] = [
            0x4d, 0x5c, 0x2a, 0xf3, 0x27, 0xcd, 0x64, 0xa6, 0x2c, 0xf3, 0x5a, 0xbd, 0x2b, 0xa6,
            0xfa, 0xb4,
        ];

        let gcm = AesGcm::new_128(&key);
        let mut tag = [0u8; 16];
        gcm.encrypt(&nonce, &[], &mut plaintext, &mut tag);
        assert_eq!(plaintext, expected_ct);
        assert_eq!(tag, expected_tag);

        // Decrypt and verify
        assert!(gcm.decrypt(&nonce, &[], &mut plaintext, &tag));
    }
}
