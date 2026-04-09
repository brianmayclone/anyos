//! ChaCha20 stream cipher (RFC 8439).

/// ChaCha20 block function.
/// Generates 64 bytes of keystream from a 256-bit key, 96-bit nonce, and counter.
pub fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut state = [0u32; 16];

    // Constants "expand 32-byte k"
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;

    // Key
    for i in 0..8 {
        state[4 + i] = u32::from_le_bytes([
            key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3],
        ]);
    }

    // Counter
    state[12] = counter;

    // Nonce
    for i in 0..3 {
        state[13 + i] = u32::from_le_bytes([
            nonce[4 * i], nonce[4 * i + 1], nonce[4 * i + 2], nonce[4 * i + 3],
        ]);
    }

    let initial = state;

    // 20 rounds (10 double-rounds)
    for _ in 0..10 {
        quarter_round(&mut state, 0, 4, 8, 12);
        quarter_round(&mut state, 1, 5, 9, 13);
        quarter_round(&mut state, 2, 6, 10, 14);
        quarter_round(&mut state, 3, 7, 11, 15);
        quarter_round(&mut state, 0, 5, 10, 15);
        quarter_round(&mut state, 1, 6, 11, 12);
        quarter_round(&mut state, 2, 7, 8, 13);
        quarter_round(&mut state, 3, 4, 9, 14);
    }

    // Add initial state
    for i in 0..16 {
        state[i] = state[i].wrapping_add(initial[i]);
    }

    // Serialize as little-endian
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[4 * i..4 * i + 4].copy_from_slice(&state[i].to_le_bytes());
    }
    out
}

#[inline]
fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(7);
}

/// Encrypt/decrypt data in place using ChaCha20.
pub fn chacha20_crypt(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    let mut ctr = counter;
    let mut offset = 0;

    while offset < data.len() {
        let keystream = chacha20_block(key, ctr, nonce);
        ctr = ctr.wrapping_add(1);

        let chunk = (data.len() - offset).min(64);
        for i in 0..chunk {
            data[offset + i] ^= keystream[i];
        }
        offset += chunk;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chacha20_block_rfc8439() {
        // RFC 8439 Section 2.3.2 test vector
        let key: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let nonce: [u8; 12] = [
            0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a,
            0x00, 0x00, 0x00, 0x00,
        ];
        let block = chacha20_block(&key, 1, &nonce);

        // First 16 bytes of expected output
        let expected_start: [u8; 16] = [
            0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15,
            0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20, 0x71, 0xc4,
        ];
        assert_eq!(&block[..16], &expected_start);
    }
}
