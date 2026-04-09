//! Pure Rust SHA-1 implementation (FIPS 180-4).
//!
//! Adapted from kernel/src/crypto/sha1.rs. Provided for TLS 1.2
//! compatibility (PRF uses HMAC-SHA1 in some configurations).

pub const DIGEST_SIZE: usize = 20;
pub const BLOCK_SIZE: usize = 64;

const H0: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
const SHA1_K: [u32; 4] = [0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6];

fn compress(state: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }

    let [mut a, mut b, mut c, mut d, mut e] = *state;

    for i in 0..80 {
        let (f, k) = match i {
            0..=19 => ((b & c) | ((!b) & d), SHA1_K[0]),
            20..=39 => (b ^ c ^ d, SHA1_K[1]),
            40..=59 => ((b & c) | (b & d) | (c & d), SHA1_K[2]),
            _ => (b ^ c ^ d, SHA1_K[3]),
        };
        let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e)
            .wrapping_add(k).wrapping_add(w[i]);
        e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
}

/// One-shot SHA-1.
pub fn sha1(data: &[u8]) -> [u8; DIGEST_SIZE] {
    let mut state = H0;
    let len_bits = (data.len() as u64).wrapping_mul(8);

    let full_blocks = data.len() / BLOCK_SIZE;
    for i in 0..full_blocks {
        let mut block = [0u8; BLOCK_SIZE];
        block.copy_from_slice(&data[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE]);
        compress(&mut state, &block);
    }

    let remainder = &data[full_blocks * BLOCK_SIZE..];
    let mut pad = [0u8; BLOCK_SIZE];
    pad[..remainder.len()].copy_from_slice(remainder);
    pad[remainder.len()] = 0x80;

    if remainder.len() + 1 > 55 {
        compress(&mut state, &pad);
        pad = [0u8; BLOCK_SIZE];
    }

    pad[56..64].copy_from_slice(&len_bits.to_be_bytes());
    compress(&mut state, &pad);

    let mut digest = [0u8; DIGEST_SIZE];
    for (i, &word) in state.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abc() {
        let expected: [u8; 20] = [
            0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a,
            0xba, 0x3e, 0x25, 0x71, 0x78, 0x50, 0xc2, 0x6c,
            0x9c, 0xd0, 0xd8, 0x9d,
        ];
        assert_eq!(sha1(b"abc"), expected);
    }
}
