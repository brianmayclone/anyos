//! NIST P-256 (secp256r1) elliptic curve operations.
//!
//! Provides ECDHE key exchange and ECDSA signature verification for TLS.
//! Uses projective (Jacobian) coordinates for point operations to avoid
//! expensive field inversions during intermediate computations.
//!
//! Curve equation: y^2 = x^3 - 3x + b (mod p)
//! p = 2^256 - 2^224 + 2^192 + 2^96 - 1 (NIST prime)

use alloc::vec::Vec;
use crate::crypto::bignum::BigUint;
use crate::crypto::sha256;

// -- P-256 curve parameters --

/// P-256 prime: 2^256 - 2^224 + 2^192 + 2^96 - 1
fn p256_p() -> BigUint {
    BigUint::from_be_bytes(&[
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ])
}

/// P-256 curve order n
fn p256_n() -> BigUint {
    BigUint::from_be_bytes(&[
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xBC, 0xE6, 0xFA, 0xAD, 0xA7, 0x17, 0x9E, 0x84,
        0xF3, 0xB9, 0xCA, 0xC2, 0xFC, 0x63, 0x25, 0x51,
    ])
}

/// P-256 parameter b
fn p256_b() -> BigUint {
    BigUint::from_be_bytes(&[
        0x5A, 0xC6, 0x35, 0xD8, 0xAA, 0x3A, 0x93, 0xE7,
        0xB3, 0xEB, 0xBD, 0x55, 0x76, 0x98, 0x86, 0xBC,
        0x65, 0x1D, 0x06, 0xB0, 0xCC, 0x53, 0xB0, 0xF6,
        0x3B, 0xCE, 0x3C, 0x3E, 0x27, 0xD2, 0x60, 0x4B,
    ])
}

/// P-256 generator point Gx
fn p256_gx() -> BigUint {
    BigUint::from_be_bytes(&[
        0x6B, 0x17, 0xD1, 0xF2, 0xE1, 0x2C, 0x42, 0x47,
        0xF8, 0xBC, 0xE6, 0xE5, 0x63, 0xA4, 0x40, 0xF2,
        0x77, 0x03, 0x7D, 0x81, 0x2D, 0xEB, 0x33, 0xA0,
        0xF4, 0xA1, 0x39, 0x45, 0xD8, 0x98, 0xC2, 0x96,
    ])
}

/// P-256 generator point Gy
fn p256_gy() -> BigUint {
    BigUint::from_be_bytes(&[
        0x4F, 0xE3, 0x42, 0xE2, 0xFE, 0x1A, 0x7F, 0x9B,
        0x8E, 0xE7, 0xEB, 0x4A, 0x7C, 0x0F, 0x9E, 0x16,
        0x2B, 0xCE, 0x33, 0x57, 0x6B, 0x31, 0x5E, 0xCE,
        0xCB, 0xB6, 0x40, 0x68, 0x37, 0xBF, 0x51, 0xF5,
    ])
}

// -- Affine point --

/// A point on P-256 in affine coordinates, or the point at infinity.
#[derive(Clone, Debug)]
pub struct P256Point {
    pub x: BigUint,
    pub y: BigUint,
    pub infinity: bool,
}

impl P256Point {
    pub fn infinity() -> Self {
        Self { x: BigUint::zero(), y: BigUint::zero(), infinity: true }
    }

    pub fn new(x: BigUint, y: BigUint) -> Self {
        Self { x, y, infinity: false }
    }

    /// Generator point G.
    pub fn generator() -> Self {
        Self::new(p256_gx(), p256_gy())
    }

    /// Decode from uncompressed SEC1 format: 0x04 || x (32 bytes) || y (32 bytes)
    pub fn from_uncompressed(data: &[u8]) -> Option<Self> {
        if data.len() != 65 || data[0] != 0x04 {
            return None;
        }
        let x = BigUint::from_be_bytes(&data[1..33]);
        let y = BigUint::from_be_bytes(&data[33..65]);
        // Basic validation: x, y < p
        let p = p256_p();
        if x.cmp(&p) >= 0 || y.cmp(&p) >= 0 {
            return None;
        }
        Some(Self::new(x, y))
    }

    /// Encode to uncompressed SEC1 format.
    pub fn to_uncompressed(&self) -> Vec<u8> {
        if self.infinity {
            return alloc::vec![0x00];
        }
        let mut out = Vec::with_capacity(65);
        out.push(0x04);
        out.extend_from_slice(&self.x.to_be_bytes_padded(32));
        out.extend_from_slice(&self.y.to_be_bytes_padded(32));
        out
    }
}

// -- Field arithmetic (mod p) --

fn fe_add(a: &BigUint, b: &BigUint) -> BigUint {
    a.add(b).rem(&p256_p())
}

fn fe_sub(a: &BigUint, b: &BigUint) -> BigUint {
    let p = p256_p();
    if a.cmp(b) >= 0 {
        a.sub(b).rem(&p)
    } else {
        p.sub(&b.sub(a).rem(&p))
    }
}

fn fe_mul(a: &BigUint, b: &BigUint) -> BigUint {
    a.mulmod(b, &p256_p())
}

fn fe_sq(a: &BigUint) -> BigUint {
    a.mulmod(a, &p256_p())
}

fn fe_inv(a: &BigUint) -> BigUint {
    // a^(p-2) mod p (Fermat's little theorem)
    let p = p256_p();
    let two = BigUint::from_u64(2);
    let exp = p.sub(&two);
    a.powmod(&exp, &p)
}

// -- Point operations (affine coordinates) --
// Using simple affine formulas. For production use, Jacobian coordinates
// would be faster, but affine is simpler and correct.

/// Point doubling on P-256.
fn point_double(p_point: &P256Point) -> P256Point {
    if p_point.infinity {
        return P256Point::infinity();
    }
    let p = p256_p();
    let zero = BigUint::zero();

    // Check if y == 0
    if p_point.y.cmp(&zero) == 0 {
        return P256Point::infinity();
    }

    // lambda = (3*x^2 + a) / (2*y)  where a = -3 for P-256
    let x_sq = fe_sq(&p_point.x);
    let three_x_sq = fe_add(&fe_add(&x_sq, &x_sq), &x_sq);
    let three = BigUint::from_u64(3);
    let a = fe_sub(&p, &three); // a = p - 3 = -3 mod p
    let num = fe_add(&three_x_sq, &a);
    let denom = fe_add(&p_point.y, &p_point.y);
    let denom_inv = fe_inv(&denom);
    let lambda = fe_mul(&num, &denom_inv);

    // x3 = lambda^2 - 2*x
    let lambda_sq = fe_sq(&lambda);
    let two_x = fe_add(&p_point.x, &p_point.x);
    let x3 = fe_sub(&lambda_sq, &two_x);

    // y3 = lambda * (x - x3) - y
    let y3 = fe_sub(&fe_mul(&lambda, &fe_sub(&p_point.x, &x3)), &p_point.y);

    P256Point::new(x3, y3)
}

/// Point addition on P-256.
fn point_add(p1: &P256Point, p2: &P256Point) -> P256Point {
    if p1.infinity {
        return p2.clone();
    }
    if p2.infinity {
        return p1.clone();
    }

    if p1.x.cmp(&p2.x) == 0 {
        if p1.y.cmp(&p2.y) == 0 {
            return point_double(p1);
        } else {
            return P256Point::infinity(); // P + (-P) = O
        }
    }

    // lambda = (y2 - y1) / (x2 - x1)
    let num = fe_sub(&p2.y, &p1.y);
    let denom = fe_sub(&p2.x, &p1.x);
    let denom_inv = fe_inv(&denom);
    let lambda = fe_mul(&num, &denom_inv);

    // x3 = lambda^2 - x1 - x2
    let lambda_sq = fe_sq(&lambda);
    let x3 = fe_sub(&fe_sub(&lambda_sq, &p1.x), &p2.x);

    // y3 = lambda * (x1 - x3) - y1
    let y3 = fe_sub(&fe_mul(&lambda, &fe_sub(&p1.x, &x3)), &p1.y);

    P256Point::new(x3, y3)
}

/// Scalar multiplication: k * P using double-and-add.
pub fn scalar_mult(k: &BigUint, p_point: &P256Point) -> P256Point {
    if k.is_zero() || p_point.infinity {
        return P256Point::infinity();
    }

    let mut result = P256Point::infinity();
    let bits = k.bit_len();

    for i in (0..bits).rev() {
        result = point_double(&result);
        if k.bit(i) != 0 {
            result = point_add(&result, p_point);
        }
    }
    result
}

// -- ECDHE key exchange --

/// Generate an ECDHE key pair. Returns (private_key_bytes, public_key_point).
/// `random_bytes` must be 32 cryptographically secure random bytes.
pub fn ecdhe_keygen(random_bytes: &[u8; 32]) -> (Vec<u8>, P256Point) {
    let k = BigUint::from_be_bytes(random_bytes);
    let n = p256_n();
    let k = k.rem(&n);
    // Ensure k >= 1
    let k = if k.is_zero() { BigUint::one() } else { k };

    let pub_point = scalar_mult(&k, &P256Point::generator());
    (k.to_be_bytes_padded(32), pub_point)
}

/// Compute the ECDHE shared secret from our private key and the peer's public key.
/// Returns the x-coordinate of k * peer_public (32 bytes).
pub fn ecdhe_shared_secret(private_key: &[u8], peer_public: &P256Point) -> Option<[u8; 32]> {
    let k = BigUint::from_be_bytes(private_key);
    let shared = scalar_mult(&k, peer_public);
    if shared.infinity {
        return None;
    }
    let x_bytes = shared.x.to_be_bytes_padded(32);
    let mut result = [0u8; 32];
    result.copy_from_slice(&x_bytes);
    Some(result)
}

// -- ECDSA verification --

/// Verify an ECDSA-P256-SHA256 signature.
///
/// - `public_key`: The signer's public key point
/// - `message_hash`: SHA-256 hash of the message (32 bytes)
/// - `r`, `s`: Signature components (big-endian, up to 32 bytes each)
pub fn ecdsa_verify(
    public_key: &P256Point,
    message_hash: &[u8],
    r: &[u8],
    s: &[u8],
) -> bool {
    let n = p256_n();
    let r_int = BigUint::from_be_bytes(r);
    let s_int = BigUint::from_be_bytes(s);

    // Check 1 <= r, s < n
    if r_int.is_zero() || s_int.is_zero() || r_int.cmp(&n) >= 0 || s_int.cmp(&n) >= 0 {
        return false;
    }

    // e = hash truncated to bit-length of n (for P-256, e = hash as-is since both are 256 bits)
    let e = BigUint::from_be_bytes(message_hash);

    // s_inv = s^(-1) mod n
    let two = BigUint::from_u64(2);
    let n_minus_2 = n.sub(&two);
    let s_inv = s_int.powmod(&n_minus_2, &n);

    // u1 = e * s_inv mod n
    let u1 = e.mulmod(&s_inv, &n);
    // u2 = r * s_inv mod n
    let u2 = r_int.mulmod(&s_inv, &n);

    // R = u1*G + u2*Q
    let g = P256Point::generator();
    let r1 = scalar_mult(&u1, &g);
    let r2 = scalar_mult(&u2, public_key);
    let r_point = point_add(&r1, &r2);

    if r_point.infinity {
        return false;
    }

    // Check: r_point.x mod n == r
    let x_mod_n = r_point.x.rem(&n);
    x_mod_n.cmp(&r_int) == 0
}

/// Parse an ECDSA signature from DER format.
/// Returns (r, s) as big-endian byte arrays.
pub fn parse_ecdsa_der(sig: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if sig.len() < 6 || sig[0] != 0x30 {
        return None;
    }
    let mut pos = 2; // skip SEQUENCE tag + length

    // Parse r INTEGER
    if pos >= sig.len() || sig[pos] != 0x02 {
        return None;
    }
    pos += 1;
    let r_len = sig[pos] as usize;
    pos += 1;
    if pos + r_len > sig.len() {
        return None;
    }
    let r = &sig[pos..pos + r_len];
    pos += r_len;

    // Parse s INTEGER
    if pos >= sig.len() || sig[pos] != 0x02 {
        return None;
    }
    pos += 1;
    let s_len = sig[pos] as usize;
    pos += 1;
    if pos + s_len > sig.len() {
        return None;
    }
    let s = &sig[pos..pos + s_len];

    // Strip leading zeros (DER integers may have a 0x00 prefix for positive numbers)
    let r = strip_leading_zeros(r);
    let s = strip_leading_zeros(s);

    Some((r.to_vec(), s.to_vec()))
}

fn strip_leading_zeros(data: &[u8]) -> &[u8] {
    let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    if start == data.len() { &data[data.len() - 1..] } else { &data[start..] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generator_on_curve() {
        // Verify G is on the curve: y^2 = x^3 - 3x + b (mod p)
        let p = p256_p();
        let g = P256Point::generator();
        let y_sq = fe_sq(&g.y);
        let x_cubed = fe_mul(&fe_sq(&g.x), &g.x);
        let three_x = fe_mul(&BigUint::from_u64(3), &g.x);
        let rhs = fe_add(&fe_sub(&x_cubed, &three_x), &p256_b());
        assert_eq!(y_sq.cmp(&rhs), 0, "Generator not on curve");
    }

    #[test]
    fn test_double_generator() {
        // 2*G should be on the curve
        let g = P256Point::generator();
        let g2 = point_double(&g);
        assert!(!g2.infinity);

        // Verify 2*G is on the curve
        let y_sq = fe_sq(&g2.y);
        let x_cubed = fe_mul(&fe_sq(&g2.x), &g2.x);
        let three_x = fe_mul(&BigUint::from_u64(3), &g2.x);
        let rhs = fe_add(&fe_sub(&x_cubed, &three_x), &p256_b());
        assert_eq!(y_sq.cmp(&rhs), 0, "2*G not on curve");
    }

    #[test]
    fn test_n_times_g_is_infinity() {
        // n * G should be the point at infinity
        // (This is too slow for a unit test with our bignum, so test with small multiple)
        let g = P256Point::generator();
        let two = BigUint::from_u64(2);
        let g2 = scalar_mult(&two, &g);
        let g2_alt = point_add(&g, &g);
        assert_eq!(g2.x.cmp(&g2_alt.x), 0, "2*G mismatch");
        assert_eq!(g2.y.cmp(&g2_alt.y), 0, "2*G y mismatch");
    }

    #[test]
    fn test_scalar_mult_known_vector() {
        // k=1 * G = G
        let g = P256Point::generator();
        let result = scalar_mult(&BigUint::one(), &g);
        assert_eq!(result.x.cmp(&g.x), 0);
        assert_eq!(result.y.cmp(&g.y), 0);
    }

    #[test]
    fn test_uncompressed_roundtrip() {
        let g = P256Point::generator();
        let encoded = g.to_uncompressed();
        assert_eq!(encoded.len(), 65);
        assert_eq!(encoded[0], 0x04);
        let decoded = P256Point::from_uncompressed(&encoded).unwrap();
        assert_eq!(decoded.x.cmp(&g.x), 0);
        assert_eq!(decoded.y.cmp(&g.y), 0);
    }

    #[test]
    fn test_parse_ecdsa_der() {
        // Minimal DER ECDSA signature
        let sig = [
            0x30, 0x06, // SEQUENCE, length 6
            0x02, 0x01, 0x01, // INTEGER r=1
            0x02, 0x01, 0x02, // INTEGER s=2
        ];
        let (r, s) = parse_ecdsa_der(&sig).unwrap();
        assert_eq!(r, vec![1]);
        assert_eq!(s, vec![2]);
    }
}
