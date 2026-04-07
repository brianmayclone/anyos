//! HMAC helpers for the kernel crypto subsystem.

use crate::crypto::sha1::{sha1, SHA1_BLOCK_SIZE, SHA1_DIGEST_SIZE};
use crate::crypto::sha256::{sha256, SHA256_BLOCK_SIZE, SHA256_DIGEST_SIZE};
use alloc::vec::Vec;

/// Compute HMAC-SHA1(key, data).
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; SHA1_DIGEST_SIZE] {
    let key_buf;
    let key_normalized: &[u8] = if key.len() > SHA1_BLOCK_SIZE {
        key_buf = sha1(key);
        &key_buf
    } else {
        key
    };

    let mut k_ipad = [0x36u8; SHA1_BLOCK_SIZE];
    let mut k_opad = [0x5Cu8; SHA1_BLOCK_SIZE];
    for i in 0..key_normalized.len() {
        k_ipad[i] ^= key_normalized[i];
        k_opad[i] ^= key_normalized[i];
    }

    let mut inner_input = Vec::with_capacity(SHA1_BLOCK_SIZE + data.len());
    inner_input.extend_from_slice(&k_ipad);
    inner_input.extend_from_slice(data);
    let inner = sha1(&inner_input);

    let mut outer_input = [0u8; SHA1_BLOCK_SIZE + SHA1_DIGEST_SIZE];
    outer_input[..SHA1_BLOCK_SIZE].copy_from_slice(&k_opad);
    outer_input[SHA1_BLOCK_SIZE..].copy_from_slice(&inner);
    sha1(&outer_input)
}

/// Compute HMAC-SHA256(key, data).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; SHA256_DIGEST_SIZE] {
    let key_buf;
    let key_normalized: &[u8] = if key.len() > SHA256_BLOCK_SIZE {
        key_buf = sha256(key);
        &key_buf
    } else {
        key
    };

    let mut k_ipad = [0x36u8; SHA256_BLOCK_SIZE];
    let mut k_opad = [0x5Cu8; SHA256_BLOCK_SIZE];
    for i in 0..key_normalized.len() {
        k_ipad[i] ^= key_normalized[i];
        k_opad[i] ^= key_normalized[i];
    }

    let mut inner_input = Vec::with_capacity(SHA256_BLOCK_SIZE + data.len());
    inner_input.extend_from_slice(&k_ipad);
    inner_input.extend_from_slice(data);
    let inner = sha256(&inner_input);

    let mut outer_input = [0u8; SHA256_BLOCK_SIZE + SHA256_DIGEST_SIZE];
    outer_input[..SHA256_BLOCK_SIZE].copy_from_slice(&k_opad);
    outer_input[SHA256_BLOCK_SIZE..].copy_from_slice(&inner);
    sha256(&outer_input)
}
