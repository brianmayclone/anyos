//! HMAC (RFC 2104) over SHA-256 and SHA-384.
//!
//! Provides both one-shot and incremental interfaces. The incremental
//! interface is needed for HKDF-Expand which calls HMAC repeatedly.

use crate::crypto::sha256::{self, Sha256};
use crate::crypto::sha384::{self, Sha384};

/// HMAC-SHA256 (one-shot).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; sha256::DIGEST_SIZE] {
    let mut ctx = HmacSha256::new(key);
    ctx.update(data);
    ctx.finalize()
}

/// HMAC-SHA384 (one-shot).
pub fn hmac_sha384(key: &[u8], data: &[u8]) -> [u8; sha384::DIGEST_SIZE] {
    let mut ctx = HmacSha384::new(key);
    ctx.update(data);
    ctx.finalize()
}

/// Incremental HMAC-SHA256 context.
pub struct HmacSha256 {
    inner: Sha256,
    opad: [u8; sha256::BLOCK_SIZE],
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let mut key_block = [0u8; sha256::BLOCK_SIZE];
        if key.len() > sha256::BLOCK_SIZE {
            key_block[..sha256::DIGEST_SIZE].copy_from_slice(&sha256::sha256(key));
        } else {
            key_block[..key.len()].copy_from_slice(key);
        }

        let mut ipad = [0x36u8; sha256::BLOCK_SIZE];
        let mut opad = [0x5Cu8; sha256::BLOCK_SIZE];
        for i in 0..sha256::BLOCK_SIZE {
            ipad[i] ^= key_block[i];
            opad[i] ^= key_block[i];
        }

        let mut inner = Sha256::new();
        inner.update(&ipad);

        Self { inner, opad }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; sha256::DIGEST_SIZE] {
        let inner_hash = self.inner.finalize();
        let mut outer = Sha256::new();
        outer.update(&self.opad);
        outer.update(&inner_hash);
        outer.finalize()
    }
}

/// Incremental HMAC-SHA384 context.
pub struct HmacSha384 {
    inner: Sha384,
    opad: [u8; sha384::BLOCK_SIZE],
}

impl HmacSha384 {
    pub fn new(key: &[u8]) -> Self {
        let mut key_block = [0u8; sha384::BLOCK_SIZE];
        if key.len() > sha384::BLOCK_SIZE {
            key_block[..sha384::DIGEST_SIZE].copy_from_slice(&sha384::sha384(key));
        } else {
            key_block[..key.len()].copy_from_slice(key);
        }

        let mut ipad = [0x36u8; sha384::BLOCK_SIZE];
        let mut opad = [0x5Cu8; sha384::BLOCK_SIZE];
        for i in 0..sha384::BLOCK_SIZE {
            ipad[i] ^= key_block[i];
            opad[i] ^= key_block[i];
        }

        let mut inner = Sha384::new();
        inner.update(&ipad);

        Self { inner, opad }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; sha384::DIGEST_SIZE] {
        let inner_hash = self.inner.finalize();
        let mut outer = Sha384::new();
        outer.update(&self.opad);
        outer.update(&inner_hash);
        outer.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sha256_rfc4231_case1() {
        // RFC 4231 Test Case 1
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let expected: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(hmac_sha256(&key, data), expected);
    }

    #[test]
    fn test_hmac_sha256_rfc4231_case2() {
        // RFC 4231 Test Case 2 ("Jefe" / "what do ya want for nothing?")
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(
            hmac_sha256(b"Jefe", b"what do ya want for nothing?"),
            expected
        );
    }

    #[test]
    fn test_hmac_sha384_rfc4231_case1() {
        // RFC 4231 Test Case 1
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let expected: [u8; 48] = [
            0xaf, 0xd0, 0x39, 0x44, 0xd8, 0x48, 0x95, 0x62, 0x6b, 0x08, 0x25, 0xf4, 0xab, 0x46,
            0x90, 0x7f, 0x15, 0xf9, 0xda, 0xdb, 0xe4, 0x10, 0x1e, 0xc6, 0x82, 0xaa, 0x03, 0x4c,
            0x7c, 0xeb, 0xc5, 0x9c, 0xfa, 0xea, 0x9e, 0xa9, 0x07, 0x6e, 0xde, 0x7f, 0x4a, 0xf1,
            0x52, 0xe8, 0xb2, 0xfa, 0x9c, 0xb6,
        ];
        assert_eq!(hmac_sha384(&key, data), expected);
    }
}
