//! PBKDF2 helpers built on top of the kernel HMAC primitives.

use crate::crypto::hmac::{hmac_sha1, hmac_sha256};
use crate::crypto::sha1::SHA1_DIGEST_SIZE;
use crate::crypto::sha256::SHA256_DIGEST_SIZE;
use alloc::vec::Vec;

/// Derive a 32-byte WPA2 PMK with PBKDF2-HMAC-SHA1.
pub fn pbkdf2_sha1(passphrase: &[u8], salt: &[u8]) -> [u8; 32] {
    let mut dk = [0u8; 32];

    for block_idx in 1u32..=2u32 {
        let mut salt_block = Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&block_idx.to_be_bytes());

        let mut u = hmac_sha1(passphrase, &salt_block);
        let mut xor_acc = u;

        for _ in 1..4096 {
            u = hmac_sha1(passphrase, &u);
            for j in 0..SHA1_DIGEST_SIZE {
                xor_acc[j] ^= u[j];
            }
        }

        let start = (block_idx as usize - 1) * SHA1_DIGEST_SIZE;
        let end = (start + SHA1_DIGEST_SIZE).min(32);
        dk[start..end].copy_from_slice(&xor_acc[..end - start]);
    }

    dk
}

/// Derive a 32-byte key with PBKDF2-HMAC-SHA256.
pub fn pbkdf2_sha256(passphrase: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut dk = [0u8; 32];

    for block_idx in 1u32..=1u32 {
        let mut salt_block = Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&block_idx.to_be_bytes());

        let mut u = hmac_sha256(passphrase, &salt_block);
        let mut xor_acc = u;

        for _ in 1..iterations {
            u = hmac_sha256(passphrase, &u);
            for j in 0..SHA256_DIGEST_SIZE {
                xor_acc[j] ^= u[j];
            }
        }

        dk.copy_from_slice(&xor_acc);
    }

    dk
}
