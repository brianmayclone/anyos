//! X25519 Diffie-Hellman key exchange (RFC 7748).
//!
//! Implements scalar multiplication on Curve25519 using the Montgomery
//! ladder algorithm. All field arithmetic is done modulo p = 2^255 - 19.

/// X25519 key size in bytes.
pub const KEY_SIZE: usize = 32;

/// The X25519 base point (generator) u-coordinate = 9.
pub const BASE_POINT: [u8; 32] = [
    9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Field element: 5 limbs of 51 bits each (total 255 bits).
type Fe = [u64; 5];

const MASK51: u64 = (1u64 << 51) - 1;

/// Compute X25519(scalar, point).
///
/// `scalar` is clamped per RFC 7748 Section 5.
pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    // Clamp scalar
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let u = fe_decode(point);
    let result = scalarmult(&k, &u);
    fe_encode(&result)
}

/// Generate a key pair. `private_key` should be 32 random bytes.
/// Returns the public key.
pub fn x25519_base(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(private_key, &BASE_POINT)
}

// -- Montgomery ladder scalar multiplication --

fn scalarmult(k: &[u8; 32], u: &Fe) -> Fe {
    let mut x_1 = *u;
    let mut x_2 = fe_one();
    let mut z_2 = fe_zero();
    let mut x_3 = *u;
    let mut z_3 = fe_one();
    let mut swap: u64 = 0;

    for t in (0..255).rev() {
        let k_t = ((k[t >> 3] >> (t & 7)) & 1) as u64;
        swap ^= k_t;
        fe_cswap(&mut x_2, &mut x_3, swap);
        fe_cswap(&mut z_2, &mut z_3, swap);
        swap = k_t;

        let a = fe_add(&x_2, &z_2);
        let aa = fe_sq(&a);
        let b = fe_sub(&x_2, &z_2);
        let bb = fe_sq(&b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x_3, &z_3);
        let d = fe_sub(&x_3, &z_3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        x_3 = fe_sq(&fe_add(&da, &cb));
        z_3 = fe_mul(&x_1, &fe_sq(&fe_sub(&da, &cb)));
        x_2 = fe_mul(&aa, &bb);
        z_2 = fe_mul(&e, &fe_add(&bb, &fe_mul_121666(&e)));
    }

    fe_cswap(&mut x_2, &mut x_3, swap);
    fe_cswap(&mut z_2, &mut z_3, swap);

    fe_mul(&x_2, &fe_invert(&z_2))
}

// -- Field arithmetic (mod 2^255 - 19) --

fn fe_zero() -> Fe { [0; 5] }
fn fe_one() -> Fe { [1, 0, 0, 0, 0] }

fn fe_decode(s: &[u8; 32]) -> Fe {
    let mut h = [0u64; 5];
    h[0] = load8(&s[0..]) & MASK51;
    h[1] = (load8(&s[6..]) >> 3) & MASK51;
    h[2] = (load8(&s[12..]) >> 6) & MASK51;
    h[3] = (load8(&s[19..]) >> 1) & MASK51;
    h[4] = (load8(&s[24..]) >> 12) & MASK51;
    h
}

fn fe_encode(h: &Fe) -> [u8; 32] {
    let mut t = *h;

    // Carry propagation
    let mut c: u64;
    c = t[0] >> 51; t[0] &= MASK51; t[1] += c;
    c = t[1] >> 51; t[1] &= MASK51; t[2] += c;
    c = t[2] >> 51; t[2] &= MASK51; t[3] += c;
    c = t[3] >> 51; t[3] &= MASK51; t[4] += c;
    c = t[4] >> 51; t[4] &= MASK51; t[0] += c * 19;
    c = t[0] >> 51; t[0] &= MASK51; t[1] += c;

    // Canonical reduction: compute q = floor((h + 19) / 2^255)
    // If h >= p, then h + 19 >= 2^255, so q = 1, and we subtract p.
    // Otherwise q = 0.
    let mut q = (t[0] + 19) >> 51;
    q = (t[1] + q) >> 51;
    q = (t[2] + q) >> 51;
    q = (t[3] + q) >> 51;
    q = (t[4] + q) >> 51; // q is 0 or 1

    // If q == 1, subtract p: h -= p means h[0] += 19 (since p = 2^255 - 19)
    t[0] += 19 * q;
    c = t[0] >> 51; t[0] &= MASK51; t[1] += c;
    c = t[1] >> 51; t[1] &= MASK51; t[2] += c;
    c = t[2] >> 51; t[2] &= MASK51; t[3] += c;
    c = t[3] >> 51; t[3] &= MASK51; t[4] += c;
    t[4] &= MASK51; // Clear bit 255

    // Serialize as little-endian bytes
    let mut s = [0u8; 32];
    s[0] = t[0] as u8;
    s[1] = (t[0] >> 8) as u8;
    s[2] = (t[0] >> 16) as u8;
    s[3] = (t[0] >> 24) as u8;
    s[4] = (t[0] >> 32) as u8;
    s[5] = (t[0] >> 40) as u8;
    s[6] = ((t[0] >> 48) | (t[1] << 3)) as u8;
    s[7] = (t[1] >> 5) as u8;
    s[8] = (t[1] >> 13) as u8;
    s[9] = (t[1] >> 21) as u8;
    s[10] = (t[1] >> 29) as u8;
    s[11] = (t[1] >> 37) as u8;
    s[12] = ((t[1] >> 45) | (t[2] << 6)) as u8;
    s[13] = (t[2] >> 2) as u8;
    s[14] = (t[2] >> 10) as u8;
    s[15] = (t[2] >> 18) as u8;
    s[16] = (t[2] >> 26) as u8;
    s[17] = (t[2] >> 34) as u8;
    s[18] = (t[2] >> 42) as u8;
    s[19] = ((t[2] >> 50) | (t[3] << 1)) as u8;
    s[20] = (t[3] >> 7) as u8;
    s[21] = (t[3] >> 15) as u8;
    s[22] = (t[3] >> 23) as u8;
    s[23] = (t[3] >> 31) as u8;
    s[24] = (t[3] >> 39) as u8;
    s[25] = ((t[3] >> 47) | (t[4] << 4)) as u8;
    s[26] = (t[4] >> 4) as u8;
    s[27] = (t[4] >> 12) as u8;
    s[28] = (t[4] >> 20) as u8;
    s[29] = (t[4] >> 28) as u8;
    s[30] = (t[4] >> 36) as u8;
    s[31] = (t[4] >> 44) as u8;
    s
}

fn fe_add(a: &Fe, b: &Fe) -> Fe {
    [a[0]+b[0], a[1]+b[1], a[2]+b[2], a[3]+b[3], a[4]+b[4]]
}

fn fe_sub(a: &Fe, b: &Fe) -> Fe {
    // Add 2p to avoid underflow.
    // 2p = (2^52 - 38, 2^52 - 2, 2^52 - 2, 2^52 - 2, 2^52 - 2)
    // 2*MASK51 = 2^52 - 2, so limb 0 needs -36 extra.
    [
        a[0] + 2*MASK51 - 36 - b[0],
        a[1] + 2*MASK51 - b[1],
        a[2] + 2*MASK51 - b[2],
        a[3] + 2*MASK51 - b[3],
        a[4] + 2*MASK51 - b[4],
    ]
}

fn fe_mul(a: &Fe, b: &Fe) -> Fe {
    let (a0, a1, a2, a3, a4) = (a[0] as u128, a[1] as u128, a[2] as u128, a[3] as u128, a[4] as u128);
    let (b0, b1, b2, b3, b4) = (b[0] as u128, b[1] as u128, b[2] as u128, b[3] as u128, b[4] as u128);

    let b1_19 = b1 * 19;
    let b2_19 = b2 * 19;
    let b3_19 = b3 * 19;
    let b4_19 = b4 * 19;

    let t0 = a0*b0 + a1*b4_19 + a2*b3_19 + a3*b2_19 + a4*b1_19;
    let t1 = a0*b1 + a1*b0 + a2*b4_19 + a3*b3_19 + a4*b2_19;
    let t2 = a0*b2 + a1*b1 + a2*b0 + a3*b4_19 + a4*b3_19;
    let t3 = a0*b3 + a1*b2 + a2*b1 + a3*b0 + a4*b4_19;
    let t4 = a0*b4 + a1*b3 + a2*b2 + a3*b1 + a4*b0;

    let mut r = [0u64; 5];
    let mut c: u128;
    r[0] = (t0 & (MASK51 as u128)) as u64;
    c = t0 >> 51;
    let t1 = t1 + c;
    r[1] = (t1 & (MASK51 as u128)) as u64;
    c = t1 >> 51;
    let t2 = t2 + c;
    r[2] = (t2 & (MASK51 as u128)) as u64;
    c = t2 >> 51;
    let t3 = t3 + c;
    r[3] = (t3 & (MASK51 as u128)) as u64;
    c = t3 >> 51;
    let t4 = t4 + c;
    r[4] = (t4 & (MASK51 as u128)) as u64;
    c = t4 >> 51;
    r[0] += (c as u64) * 19;
    c = (r[0] >> 51) as u128;
    r[0] &= MASK51;
    r[1] += c as u64;
    r
}

fn fe_sq(a: &Fe) -> Fe { fe_mul(a, a) }

fn fe_mul_121666(a: &Fe) -> Fe {
    let mut r = [0u64; 5];
    let mut c: u128;
    let t0 = (a[0] as u128) * 121666;
    let t1 = (a[1] as u128) * 121666;
    let t2 = (a[2] as u128) * 121666;
    let t3 = (a[3] as u128) * 121666;
    let t4 = (a[4] as u128) * 121666;

    r[0] = (t0 & (MASK51 as u128)) as u64;
    c = t0 >> 51;
    r[1] = ((t1 + c) & (MASK51 as u128)) as u64;
    c = (t1 + c) >> 51;
    r[2] = ((t2 + c) & (MASK51 as u128)) as u64;
    c = (t2 + c) >> 51;
    r[3] = ((t3 + c) & (MASK51 as u128)) as u64;
    c = (t3 + c) >> 51;
    r[4] = ((t4 + c) & (MASK51 as u128)) as u64;
    c = (t4 + c) >> 51;
    r[0] += (c as u64) * 19;
    r
}

fn fe_invert(z: &Fe) -> Fe {
    // z^(p-2) = z^(2^255 - 21) using a chain of squarings and multiplications
    let mut t0 = fe_sq(z);            // z^2
    let mut t1 = fe_sq(&t0);          // z^4
    t1 = fe_sq(&t1);                  // z^8
    t1 = fe_mul(&t1, z);              // z^9
    t0 = fe_mul(&t0, &t1);           // z^11
    let mut t2 = fe_sq(&t0);         // z^22
    t1 = fe_mul(&t1, &t2);           // z^(2^5 - 1)
    t2 = fe_sq(&t1);
    for _ in 0..4 { t2 = fe_sq(&t2); }  // z^(2^10 - 2^5)
    t1 = fe_mul(&t2, &t1);           // z^(2^10 - 1)
    t2 = fe_sq(&t1);
    for _ in 0..9 { t2 = fe_sq(&t2); }  // z^(2^20 - 2^10)
    t2 = fe_mul(&t2, &t1);           // z^(2^20 - 1)
    let mut t3 = fe_sq(&t2);
    for _ in 0..19 { t3 = fe_sq(&t3); } // z^(2^40 - 2^20)
    t2 = fe_mul(&t3, &t2);           // z^(2^40 - 1)
    t2 = fe_sq(&t2);
    for _ in 0..9 { t2 = fe_sq(&t2); }  // z^(2^50 - 2^10)
    t1 = fe_mul(&t2, &t1);           // z^(2^50 - 1)
    t2 = fe_sq(&t1);
    for _ in 0..49 { t2 = fe_sq(&t2); } // z^(2^100 - 2^50)
    t2 = fe_mul(&t2, &t1);           // z^(2^100 - 1)
    t3 = fe_sq(&t2);
    for _ in 0..99 { t3 = fe_sq(&t3); } // z^(2^200 - 2^100)
    t2 = fe_mul(&t3, &t2);           // z^(2^200 - 1)
    t2 = fe_sq(&t2);
    for _ in 0..49 { t2 = fe_sq(&t2); } // z^(2^250 - 2^50)
    t1 = fe_mul(&t2, &t1);           // z^(2^250 - 1)
    t1 = fe_sq(&t1);                 // z^(2^251 - 2)
    t1 = fe_sq(&t1);                 // z^(2^252 - 4)
    t1 = fe_sq(&t1);                 // z^(2^253 - 8)
    t1 = fe_sq(&t1);                 // z^(2^254 - 16)
    t1 = fe_sq(&t1);                 // z^(2^255 - 32)
    fe_mul(&t1, &t0)                 // z^(2^255 - 21)
}

/// Constant-time conditional swap.
fn fe_cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
    let mask = 0u64.wrapping_sub(swap); // 0 or 0xFFFFFFFFFFFFFFFF
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

fn load8(s: &[u8]) -> u64 {
    let mut r = 0u64;
    for i in 0..8.min(s.len()) {
        r |= (s[i] as u64) << (8 * i);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fe_roundtrip() {
        let bytes: [u8; 32] = [
            9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let fe = fe_decode(&bytes);
        let result = fe_encode(&fe);
        assert_eq!(result, bytes);
    }

    #[test]
    fn test_fe_mul_one() {
        let one = fe_one();
        let nine_bytes: [u8; 32] = [
            9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let nine = fe_decode(&nine_bytes);
        let result = fe_mul(&nine, &one);
        assert_eq!(fe_encode(&result), nine_bytes);
    }

    #[test]
    fn test_fe_sub_self() {
        let nine_bytes: [u8; 32] = [
            9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let nine = fe_decode(&nine_bytes);
        let zero = fe_sub(&nine, &nine);
        assert_eq!(fe_encode(&zero), [0u8; 32]);
    }

    #[test]
    fn test_fe_add_sub() {
        let a_bytes = [100u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let b_bytes = [42u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let a = fe_decode(&a_bytes);
        let b = fe_decode(&b_bytes);
        let diff = fe_sub(&a, &b);
        assert_eq!(fe_encode(&diff), [58u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn test_fe_sq_vs_mul() {
        let bytes = [42u8, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                      0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let x = fe_decode(&bytes);
        assert_eq!(fe_encode(&fe_sq(&x)), fe_encode(&fe_mul(&x, &x)));
    }

    #[test]
    fn test_fe_invert() {
        let nine_bytes: [u8; 32] = [
            9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let nine = fe_decode(&nine_bytes);
        let inv = fe_invert(&nine);
        let product = fe_mul(&nine, &inv);
        let one_bytes: [u8; 32] = [
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(fe_encode(&product), one_bytes);
    }

    #[test]
    fn test_mul_after_sub() {
        // Test that fe_mul works correctly when inputs have large limbs (from fe_sub)
        let a = fe_decode(&{let mut b = [0u8; 32]; b[0] = 100; b});
        let c = fe_decode(&{let mut b = [0u8; 32]; b[0] = 42; b});
        let d = fe_decode(&{let mut b = [0u8; 32]; b[0] = 7; b});
        let diff1 = fe_sub(&a, &c); // 58
        let diff2 = fe_sub(&a, &d); // 93
        let product = fe_mul(&diff1, &diff2); // 58 * 93 = 5394
        let expected = {let mut b = [0u8; 32]; b[0] = 0x12; b[1] = 0x15; b}; // 5394 = 0x1512
        assert_eq!(fe_encode(&product), expected);
    }

    #[test]
    fn test_ladder_one_step() {
        // RFC 7748 Section 6.1 iterated test: x25519(k=9, u=9)
        let nine: [u8; 32] = {
            let mut b = [0u8; 32]; b[0] = 9; b
        };
        let result = x25519(&nine, &nine);
        let expected = [
            0x42, 0x2c, 0x8e, 0x7a, 0x62, 0x27, 0xd7, 0xbc,
            0xa1, 0x35, 0x0b, 0x3e, 0x2b, 0xb7, 0x27, 0x9f,
            0x78, 0x97, 0xb8, 0x7b, 0xb6, 0x85, 0x4b, 0x78,
            0x3c, 0x60, 0xe8, 0x03, 0x11, 0xae, 0x30, 0x79,
        ];
        assert_eq!(result, expected, "x25519(9, 9) failed");
    }

    #[test]
    fn test_x25519_rfc7748_vector1() {
        // RFC 7748 Section 6.1
        let scalar: [u8; 32] = [
            0xa5, 0x46, 0xe3, 0x6b, 0xf0, 0x52, 0x7c, 0x9d,
            0x3b, 0x16, 0x15, 0x4b, 0x82, 0x46, 0x5e, 0xdd,
            0x62, 0x14, 0x4c, 0x0a, 0xc1, 0xfc, 0x5a, 0x18,
            0x50, 0x6a, 0x22, 0x44, 0xba, 0x44, 0x9a, 0xc4,
        ];
        let u_coord: [u8; 32] = [
            0xe6, 0xdb, 0x68, 0x67, 0x58, 0x30, 0x30, 0xdb,
            0x35, 0x94, 0xc1, 0xa4, 0x24, 0xb1, 0x5f, 0x7c,
            0x72, 0x66, 0x24, 0xec, 0x26, 0xb3, 0x35, 0x3b,
            0x10, 0xa9, 0x03, 0xa6, 0xd0, 0xab, 0x1c, 0x4c,
        ];
        let expected: [u8; 32] = [
            0xc3, 0xda, 0x55, 0x37, 0x9d, 0xe9, 0xc6, 0x90,
            0x8e, 0x94, 0xea, 0x4d, 0xf2, 0x8d, 0x08, 0x4f,
            0x32, 0xec, 0xcf, 0x03, 0x49, 0x1c, 0x71, 0xf7,
            0x54, 0xb4, 0x07, 0x55, 0x77, 0xa2, 0x85, 0x52,
        ];
        assert_eq!(x25519(&scalar, &u_coord), expected);
    }

    #[test]
    fn test_x25519_base_point() {
        // Verify that x25519(scalar, 9) produces a known result
        let scalar: [u8; 32] = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d,
            0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66, 0x45,
            0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a,
            0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let expected: [u8; 32] = [
            0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54,
            0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e, 0xf7, 0x5a,
            0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4,
            0xeb, 0xa4, 0xa9, 0x8e, 0xaa, 0x9b, 0x4e, 0x6a,
        ];
        assert_eq!(x25519_base(&scalar), expected);
    }
}
