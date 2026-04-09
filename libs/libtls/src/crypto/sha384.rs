//! SHA-384 -- truncated SHA-512 with different initial values (FIPS 180-4).

use crate::crypto::sha512::Sha512;

pub const DIGEST_SIZE: usize = 48;
pub const BLOCK_SIZE: usize = 128; // Same as SHA-512

const H0_384: [u64; 8] = [
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507,
    0x9159015a3070dd17, 0x152fecd8f70e5939,
    0x67332667ffc00b31, 0x8eb44a8768581511,
    0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];

/// Incremental SHA-384 context.
pub struct Sha384 {
    inner: Sha512,
}

impl Sha384 {
    pub fn new() -> Self {
        Self { inner: Sha512::new_with_iv(H0_384) }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; DIGEST_SIZE] {
        let full = self.inner.finalize_internal(DIGEST_SIZE);
        let mut out = [0u8; DIGEST_SIZE];
        out.copy_from_slice(&full[..DIGEST_SIZE]);
        out
    }
}

impl Clone for Sha384 {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// One-shot SHA-384.
pub fn sha384(data: &[u8]) -> [u8; DIGEST_SIZE] {
    let mut ctx = Sha384::new();
    ctx.update(data);
    ctx.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abc() {
        let expected: [u8; 48] = [
            0xcb, 0x00, 0x75, 0x3f, 0x45, 0xa3, 0x5e, 0x8b,
            0xb5, 0xa0, 0x3d, 0x69, 0x9a, 0xc6, 0x50, 0x07,
            0x27, 0x2c, 0x32, 0xab, 0x0e, 0xde, 0xd1, 0x63,
            0x1a, 0x8b, 0x60, 0x5a, 0x43, 0xff, 0x5b, 0xed,
            0x80, 0x86, 0x07, 0x2b, 0xa1, 0xe7, 0xcc, 0x23,
            0x58, 0xba, 0xec, 0xa1, 0x34, 0xc8, 0x25, 0xa7,
        ];
        assert_eq!(sha384(b"abc"), expected);
    }

    #[test]
    fn test_empty() {
        let expected: [u8; 48] = [
            0x38, 0xb0, 0x60, 0xa7, 0x51, 0xac, 0x96, 0x38,
            0x4c, 0xd9, 0x32, 0x7e, 0xb1, 0xb1, 0xe3, 0x6a,
            0x21, 0xfd, 0xb7, 0x11, 0x14, 0xbe, 0x07, 0x43,
            0x4c, 0x0c, 0xc7, 0xbf, 0x63, 0xf6, 0xe1, 0xda,
            0x27, 0x4e, 0xde, 0xbf, 0xe7, 0x6f, 0x65, 0xfb,
            0xd5, 0x1a, 0xd2, 0xf1, 0x48, 0x98, 0xb9, 0x5b,
        ];
        assert_eq!(sha384(b""), expected);
    }
}
