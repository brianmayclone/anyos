//! Single-DES implementation for VNC type-2 authentication.
//!
//! VNC uses DES with the password as the key and the 16-byte challenge split
//! into two 8-byte blocks. Each block is encrypted independently (ECB mode).
//!
//! **VNC key quirk**: the bit order of each password byte is reversed before
//! use as a DES key (LSB becomes MSB). This matches the original VNC reference
//! implementation.
//!
//! This is a pure no_std implementation with no external dependencies.

// ── Permutation and expansion tables ─────────────────────────────────────────

/// Initial permutation (IP) table (1-based bit positions).
#[rustfmt::skip]
const IP: [u8; 64] = [
    58,50,42,34,26,18,10, 2,
    60,52,44,36,28,20,12, 4,
    62,54,46,38,30,22,14, 6,
    64,56,48,40,32,24,16, 8,
    57,49,41,33,25,17, 9, 1,
    59,51,43,35,27,19,11, 3,
    61,53,45,37,29,21,13, 5,
    63,55,47,39,31,23,15, 7,
];

/// Final permutation (IP⁻¹) table.
#[rustfmt::skip]
const FP: [u8; 64] = [
    40, 8,48,16,56,24,64,32,
    39, 7,47,15,55,23,63,31,
    38, 6,46,14,54,22,62,30,
    37, 5,45,13,53,21,61,29,
    36, 4,44,12,52,20,60,28,
    35, 3,43,11,51,19,59,27,
    34, 2,42,10,50,18,58,26,
    33, 1,41, 9,49,17,57,25,
];

/// Permuted-choice 1: 64-bit key → 56-bit key (C28 || D28).
#[rustfmt::skip]
const PC1: [u8; 56] = [
    57,49,41,33,25,17, 9,
     1,58,50,42,34,26,18,
    10, 2,59,51,43,35,27,
    19,11, 3,60,52,44,36,
    63,55,47,39,31,23,15,
     7,62,54,46,38,30,22,
    14, 6,61,53,45,37,29,
    21,13, 5,28,20,12, 4,
];

/// Permuted-choice 2: 56-bit combined CD → 48-bit subkey.
#[rustfmt::skip]
const PC2: [u8; 48] = [
    14,17,11,24, 1, 5,
     3,28,15, 6,21,10,
    23,19,12, 4,26, 8,
    16, 7,27,20,13, 2,
    41,52,31,37,47,55,
    30,40,51,45,33,48,
    44,49,39,56,34,53,
    46,42,50,36,29,32,
];

/// Number of left-rotate positions for each of the 16 rounds.
#[rustfmt::skip]
const SHIFTS: [u8; 16] = [1,1,2,2,2,2,2,2,1,2,2,2,2,2,2,1];

/// Expansion permutation E: 32 bits → 48 bits.
#[rustfmt::skip]
const E: [u8; 48] = [
    32, 1, 2, 3, 4, 5,
     4, 5, 6, 7, 8, 9,
     8, 9,10,11,12,13,
    12,13,14,15,16,17,
    16,17,18,19,20,21,
    20,21,22,23,24,25,
    24,25,26,27,28,29,
    28,29,30,31,32, 1,
];

/// P permutation applied after S-box substitution.
#[rustfmt::skip]
const P: [u8; 32] = [
    16, 7,20,21,
    29,12,28,17,
     1,15,23,26,
     5,18,31,10,
     2, 8,24,14,
    32,27, 3, 9,
    19,13,30, 6,
    22,11, 4,25,
];

/// Eight 4-bit × 6-bit S-boxes (S1–S8), each 4 rows × 16 cols = 64 entries.
#[rustfmt::skip]
const S: [[u8; 64]; 8] = [
    // S1
    [14, 4,13, 1, 2,15,11, 8, 3,10, 6,12, 5, 9, 0, 7,
      0,15, 7, 4,14, 2,13, 1,10, 6,12,11, 9, 5, 3, 8,
      4, 1,14, 8,13, 6, 2,11,15,12, 9, 7, 3,10, 5, 0,
     15,12, 8, 2, 4, 9, 1, 7, 5,11, 3,14,10, 0, 6,13],
    // S2
    [15, 1, 8,14, 6,11, 3, 4, 9, 7, 2,13,12, 0, 5,10,
      3,13, 4, 7,15, 2, 8,14,12, 0, 1,10, 6, 9,11, 5,
      0,14, 7,11,10, 4,13, 1, 5, 8,12, 6, 9, 3, 2,15,
     13, 8,10, 1, 3,15, 4, 2,11, 6, 7,12, 0, 5,14, 9],
    // S3
    [10, 0, 9,14, 6, 3,15, 5, 1,13,12, 7,11, 4, 2, 8,
     13, 7, 0, 9, 3, 4, 6,10, 2, 8, 5,14,12,11,15, 1,
     13, 6, 4, 9, 8,15, 3, 0,11, 1, 2,12, 5,10,14, 7,
      1,10,13, 0, 6, 9, 8, 7, 4,15,14, 3,11, 5, 2,12],
    // S4
    [ 7,13,14, 3, 0, 6, 9,10, 1, 2, 8, 5,11,12, 4,15,
     13, 8,11, 5, 6,15, 0, 3, 4, 7, 2,12, 1,10,14, 9,
     10, 6, 9, 0,12,11, 7,13,15, 1, 3,14, 5, 2, 8, 4,
      3,15, 0, 6,10, 1,13, 8, 9, 4, 5,11,12, 7, 2,14],
    // S5
    [ 2,12, 4, 1, 7,10,11, 6, 8, 5, 3,15,13, 0,14, 9,
     14,11, 2,12, 4, 7,13, 1, 5, 0,15,10, 3, 9, 8, 6,
      4, 2, 1,11,10,13, 7, 8,15, 9,12, 5, 6, 3, 0,14,
     11, 8,12, 7, 1,14, 2,13, 6,15, 0, 9,10, 4, 5, 3],
    // S6
    [12, 1,10,15, 9, 2, 6, 8, 0,13, 3, 4,14, 7, 5,11,
     10,15, 4, 2, 7,12, 9, 5, 6, 1,13,14, 0,11, 3, 8,
      9,14,15, 5, 2, 8,12, 3, 7, 0, 4,10, 1,13,11, 6,
      4, 3, 2,12, 9, 5,15,10,11,14, 1, 7, 6, 0, 8,13],
    // S7
    [ 4,11, 2,14,15, 0, 8,13, 3,12, 9, 7, 5,10, 6, 1,
     13, 0,11, 7, 4, 9, 1,10,14, 3, 5,12, 2,15, 8, 6,
      1, 4,11,13,12, 3, 7,14,10,15, 6, 8, 0, 5, 9, 2,
      6,11,13, 8, 1, 4,10, 7, 9, 5, 0,15,14, 2, 3,12],
    // S8
    [13, 2, 8, 4, 6,15,11, 1,10, 9, 3,14, 5, 0,12, 7,
      1,15,13, 8,10, 3, 7, 4,12, 5, 6,11, 0,14, 9, 2,
      7,11, 4, 1, 9,12,14, 2, 0, 6,10,13,15, 3, 5, 8,
      2, 1,14, 7, 4,10, 8,13,15,12, 9, 0, 3, 5, 6,11],
];

// ── Bit manipulation helpers ──────────────────────────────────────────────────

/// Extract the `pos`th bit (1-based) of a 64-bit big-endian block.
#[inline]
fn get_bit_64(block: u64, pos: u8) -> u64 {
    (block >> (64 - pos)) & 1
}

/// Set bit `pos` (1-based) in a 64-bit value.
#[inline]
fn set_bit_64(block: u64, pos: u8, val: u64) -> u64 {
    block | (val << (64 - pos))
}

/// Apply an arbitrary permutation table to a 64-bit block.
fn permute_64(block: u64, table: &[u8]) -> u64 {
    let mut out = 0u64;
    for (i, &src) in table.iter().enumerate() {
        let bit = get_bit_64(block, src);
        out = set_bit_64(out, (i + 1) as u8, bit);
    }
    out
}

/// Extract the `pos`th bit (1-based) of a 56-bit half-key stored in u64.
#[inline]
fn get_bit_56(block: u64, pos: u8) -> u64 {
    (block >> (56 - pos)) & 1
}

#[inline]
fn set_bit_56(block: u64, pos: u8, val: u64) -> u64 {
    block | (val << (56 - pos))
}

/// Apply PC2 to a 56-bit CD value, producing a 48-bit subkey.
fn pc2_permute(cd: u64) -> u64 {
    let mut out = 0u64;
    for (i, &src) in PC2.iter().enumerate() {
        let bit = get_bit_56(cd, src);
        out = set_bit_64(out, (i + 1) as u8, bit);
    }
    out
}

/// Rotate the left 28 bits of a 56-bit value left by `n` positions.
fn rotate28(half: u32, n: u8) -> u32 {
    // Only 28 bits are significant.
    let mask = 0x0FFF_FFFFu32;
    ((half << n) | (half >> (28 - n))) & mask
}

// ── Key schedule ──────────────────────────────────────────────────────────────

/// Derive the 16 × 48-bit round subkeys from an 8-byte DES key.
///
/// VNC reverses the bit order of each key byte before scheduling — this
/// reversal is applied here inside `vnc_key_to_des`.
fn key_schedule(key64: u64) -> [u64; 16] {
    // PC1: 64 → 56 bits
    let permuted = permute_64(key64, &PC1);

    // Split into C (bits 1-28) and D (bits 29-56) of the 56-bit value.
    // permute_64 stores output MSB-aligned: bit 1 at u64 position 63, bit 56 at position 8.
    // C = bits 1-28  → u64 positions 63-36 → shift right by 36 to get lower 28 bits.
    // D = bits 29-56 → u64 positions 35-8  → shift right by 8 to get lower 28 bits.
    let mut c = ((permuted >> 36) as u32) & 0x0FFF_FFFF;
    let mut d = ((permuted >> 8) as u32) & 0x0FFF_FFFF;

    let mut subkeys = [0u64; 16];
    for (round, &shift) in SHIFTS.iter().enumerate() {
        c = rotate28(c, shift);
        d = rotate28(d, shift);

        // Reassemble 56-bit CD and apply PC2 to get 48-bit subkey.
        let cd = ((c as u64) << 28) | (d as u64);
        subkeys[round] = pc2_permute(cd);
    }
    subkeys
}

// ── Feistel function ──────────────────────────────────────────────────────────

/// Apply the DES Feistel function f(R, K) where R is 32 bits, K is 48 bits.
fn feistel(r: u32, k: u64) -> u32 {
    // Expand R from 32 → 48 bits.
    let r64 = (r as u64) << 32;
    let mut expanded = 0u64;
    for (i, &src) in E.iter().enumerate() {
        // E table is 1-based over 32-bit R; we stored R in the top 32 bits.
        let bit = (r64 >> (64 - src)) & 1;
        expanded = set_bit_64(expanded, (i + 1) as u8, bit);
    }

    // XOR with subkey.
    let xored = expanded ^ k;

    // S-box substitution: 48 bits → 32 bits (8 groups of 6 bits each).
    let mut sout = 0u32;
    for i in 0..8 {
        // Extract 6-bit group from positions i*6+1 .. i*6+6 (1-based).
        let shift = 64 - (i * 6 + 6);
        let group = ((xored >> shift) & 0x3F) as usize;
        // Row = bits 1 and 6 of group; col = bits 2-5.
        let row = ((group >> 4) & 2) | (group & 1);
        let col = (group >> 1) & 0xF;
        let s_val = S[i][row * 16 + col] as u32;
        sout = (sout << 4) | s_val;
    }

    // P permutation: 32 → 32 bits.
    let sout64 = (sout as u64) << 32;
    let mut pout = 0u32;
    for (i, &src) in P.iter().enumerate() {
        let bit = ((sout64 >> (64 - src)) & 1) as u32;
        pout |= bit << (31 - i);
    }
    pout
}

// ── DES encrypt / decrypt core ────────────────────────────────────────────────

/// Encrypt or decrypt an 8-byte block using the given 16 subkeys.
///
/// Pass subkeys in forward order for encryption, reversed for decryption.
fn des_block(block: u64, subkeys: &[u64; 16]) -> u64 {
    // Initial permutation.
    let permuted = permute_64(block, &IP);

    let mut l = (permuted >> 32) as u32;
    let mut r = permuted as u32;

    // 16 Feistel rounds.
    for &k in subkeys.iter() {
        let f = feistel(r, k);
        let new_r = l ^ f;
        l = r;
        r = new_r;
    }

    // Pre-output: swap L and R.
    let preout = ((r as u64) << 32) | (l as u64);

    // Final permutation (IP⁻¹).
    permute_64(preout, &FP)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Reverse the bit order of a single byte (VNC key quirk).
#[inline]
fn reverse_byte(b: u8) -> u8 {
    let mut v = b;
    v = ((v & 0xF0) >> 4) | ((v & 0x0F) << 4);
    v = ((v & 0xCC) >> 2) | ((v & 0x33) << 2);
    v = ((v & 0xAA) >> 1) | ((v & 0x55) << 1);
    v
}

/// Convert an 8-byte VNC password into the DES key integer.
///
/// VNC reverses the bit order of each password byte before using it as a key.
/// Bytes beyond the password length are zero-padded.
fn vnc_key_to_des(password: &[u8; 8]) -> u64 {
    let mut key = 0u64;
    for (i, &b) in password.iter().enumerate() {
        key |= (reverse_byte(b) as u64) << (56 - i * 8);
    }
    key
}

/// Encrypt the 16-byte VNC challenge with the given 8-byte password.
///
/// The challenge is split into two 8-byte blocks; each is DES-encrypted
/// with the password-derived key. The ciphertext is written back into
/// `challenge` in place.
///
/// # VNC Auth Flow
/// 1. Server sends 16 random bytes as `challenge`.
/// 2. Client encrypts them with this function (using the shared password).
/// 3. Server does the same and compares — matching means auth success.
pub fn vnc_encrypt_challenge(password: &[u8; 8], challenge: &mut [u8; 16]) {
    let key = vnc_key_to_des(password);
    let subkeys = key_schedule(key);

    for block_idx in 0..2 {
        let offset = block_idx * 8;
        let mut block_u64 = 0u64;
        for i in 0..8 {
            block_u64 |= (challenge[offset + i] as u64) << (56 - i * 8);
        }
        let encrypted = des_block(block_u64, &subkeys);
        for i in 0..8 {
            challenge[offset + i] = ((encrypted >> (56 - i * 8)) & 0xFF) as u8;
        }
    }
}

/// Verify a VNC auth response from a client.
///
/// Returns `true` if the client's 16-byte response matches what we would
/// get by encrypting `challenge` with `password`.
pub fn vnc_verify_response(password: &[u8; 8], challenge: &[u8; 16], response: &[u8; 16]) -> bool {
    let mut expected = *challenge;
    vnc_encrypt_challenge(password, &mut expected);
    // Constant-time comparison to avoid timing side-channels.
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= expected[i] ^ response[i];
    }
    diff == 0
}
