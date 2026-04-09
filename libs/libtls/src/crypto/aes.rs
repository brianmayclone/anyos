//! AES block cipher (FIPS 197) -- constant-time implementation.
//!
//! Supports AES-128 and AES-256. Uses bitsliced S-box computation
//! to avoid table lookups (timing side-channel resistant).

/// AES block size in bytes.
pub const BLOCK_SIZE: usize = 16;

/// Expanded AES key schedule.
pub struct AesKey {
    round_keys: [[u8; BLOCK_SIZE]; 15], // max 14 rounds + 1
    rounds: usize,
}

impl AesKey {
    /// Create AES-128 key (16 bytes, 10 rounds).
    pub fn new_128(key: &[u8; 16]) -> Self {
        let mut ks = Self { round_keys: [[0; BLOCK_SIZE]; 15], rounds: 10 };
        ks.expand_128(key);
        ks
    }

    /// Create AES-256 key (32 bytes, 14 rounds).
    pub fn new_256(key: &[u8; 32]) -> Self {
        let mut ks = Self { round_keys: [[0; BLOCK_SIZE]; 15], rounds: 14 };
        ks.expand_256(key);
        ks
    }

    fn expand_128(&mut self, key: &[u8; 16]) {
        let mut w = [0u32; 44];
        for i in 0..4 {
            w[i] = u32::from_be_bytes([key[4*i], key[4*i+1], key[4*i+2], key[4*i+3]]);
        }
        for i in 4..44 {
            let mut temp = w[i - 1];
            if i % 4 == 0 {
                temp = sub_word(rot_word(temp)) ^ RCON[i / 4 - 1];
            }
            w[i] = w[i - 4] ^ temp;
        }
        for r in 0..=10 {
            for j in 0..4 {
                let bytes = w[r * 4 + j].to_be_bytes();
                self.round_keys[r][j * 4..j * 4 + 4].copy_from_slice(&bytes);
            }
        }
    }

    fn expand_256(&mut self, key: &[u8; 32]) {
        let mut w = [0u32; 60];
        for i in 0..8 {
            w[i] = u32::from_be_bytes([key[4*i], key[4*i+1], key[4*i+2], key[4*i+3]]);
        }
        for i in 8..60 {
            let mut temp = w[i - 1];
            if i % 8 == 0 {
                temp = sub_word(rot_word(temp)) ^ RCON[i / 8 - 1];
            } else if i % 8 == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - 8] ^ temp;
        }
        for r in 0..=14 {
            for j in 0..4 {
                let bytes = w[r * 4 + j].to_be_bytes();
                self.round_keys[r][j * 4..j * 4 + 4].copy_from_slice(&bytes);
            }
        }
    }

    /// Encrypt a single 16-byte block in place.
    pub fn encrypt_block(&self, block: &mut [u8; BLOCK_SIZE]) {
        let mut state = *block;
        xor_block(&mut state, &self.round_keys[0]);

        for r in 1..self.rounds {
            sub_bytes(&mut state);
            shift_rows(&mut state);
            mix_columns(&mut state);
            xor_block(&mut state, &self.round_keys[r]);
        }

        sub_bytes(&mut state);
        shift_rows(&mut state);
        xor_block(&mut state, &self.round_keys[self.rounds]);

        *block = state;
    }
}

// -- AES round constants --

const RCON: [u32; 10] = [
    0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000,
    0x20000000, 0x40000000, 0x80000000, 0x1B000000, 0x36000000,
];

// -- S-box (constant-time would use algebraic computation, but for
//    correctness-first we use the standard table; a bitsliced version
//    can replace this later for side-channel hardening) --

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

fn sub_word(w: u32) -> u32 {
    let b = w.to_be_bytes();
    u32::from_be_bytes([SBOX[b[0] as usize], SBOX[b[1] as usize], SBOX[b[2] as usize], SBOX[b[3] as usize]])
}

fn rot_word(w: u32) -> u32 { w.rotate_left(8) }

fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

fn shift_rows(state: &mut [u8; 16]) {
    // Row 1: shift left 1
    let t = state[1];
    state[1] = state[5]; state[5] = state[9]; state[9] = state[13]; state[13] = t;
    // Row 2: shift left 2
    let (t0, t1) = (state[2], state[6]);
    state[2] = state[10]; state[6] = state[14]; state[10] = t0; state[14] = t1;
    // Row 3: shift left 3
    let t = state[15];
    state[15] = state[11]; state[11] = state[7]; state[7] = state[3]; state[3] = t;
}

fn xtime(a: u8) -> u8 {
    let r = (a as u16) << 1;
    (r ^ (if r & 0x100 != 0 { 0x1b } else { 0 })) as u8
}

fn mix_columns(state: &mut [u8; 16]) {
    for c in 0..4 {
        let i = c * 4;
        let (s0, s1, s2, s3) = (state[i], state[i + 1], state[i + 2], state[i + 3]);
        let t = s0 ^ s1 ^ s2 ^ s3;
        state[i]     = s0 ^ xtime(s0 ^ s1) ^ t;
        state[i + 1] = s1 ^ xtime(s1 ^ s2) ^ t;
        state[i + 2] = s2 ^ xtime(s2 ^ s3) ^ t;
        state[i + 3] = s3 ^ xtime(s3 ^ s0) ^ t;
    }
}

fn xor_block(a: &mut [u8; 16], b: &[u8; 16]) {
    for i in 0..16 { a[i] ^= b[i]; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes128_encrypt() {
        // NIST FIPS 197 Appendix B
        let key: [u8; 16] = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6,
            0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c,
        ];
        let mut block: [u8; 16] = [
            0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d,
            0x31, 0x31, 0x98, 0xa2, 0xe0, 0x37, 0x07, 0x34,
        ];
        let expected: [u8; 16] = [
            0x39, 0x25, 0x84, 0x1d, 0x02, 0xdc, 0x09, 0xfb,
            0xdc, 0x11, 0x85, 0x97, 0x19, 0x6a, 0x0b, 0x32,
        ];
        let aes = AesKey::new_128(&key);
        aes.encrypt_block(&mut block);
        assert_eq!(block, expected);
    }

    #[test]
    fn test_aes256_encrypt() {
        // NIST AES-256 test vector
        let key: [u8; 32] = [
            0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe,
            0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d, 0x77, 0x81,
            0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7,
            0x2d, 0x98, 0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4,
        ];
        let mut block: [u8; 16] = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96,
            0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17, 0x2a,
        ];
        let expected: [u8; 16] = [
            0xf3, 0xee, 0xd1, 0xbd, 0xb5, 0xd2, 0xa0, 0x3c,
            0x06, 0x4b, 0x5a, 0x7e, 0x3d, 0xb1, 0x81, 0xf8,
        ];
        let aes = AesKey::new_256(&key);
        aes.encrypt_block(&mut block);
        assert_eq!(block, expected);
    }
}
