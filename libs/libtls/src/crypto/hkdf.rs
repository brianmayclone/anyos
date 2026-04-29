//! HKDF (RFC 5869) -- HMAC-based Key Derivation Function.
//!
//! Used by TLS 1.3 for all key derivation (RFC 8446 Section 7.1).
//! Supports both SHA-256 and SHA-384 hash functions.

use crate::crypto::hmac::{hmac_sha256, hmac_sha384, HmacSha256, HmacSha384};
use crate::crypto::sha256;
use crate::crypto::sha384;

/// HKDF-Extract with SHA-256: PRK = HMAC-SHA256(salt, IKM).
pub fn hkdf_extract_sha256(salt: &[u8], ikm: &[u8]) -> [u8; sha256::DIGEST_SIZE] {
    let salt = if salt.is_empty() {
        &[0u8; sha256::DIGEST_SIZE][..]
    } else {
        salt
    };
    hmac_sha256(salt, ikm)
}

/// HKDF-Expand with SHA-256: OKM = T(1) || T(2) || ... truncated to `length`.
pub fn hkdf_expand_sha256(prk: &[u8], info: &[u8], output: &mut [u8]) {
    let hash_len = sha256::DIGEST_SIZE;
    let n = (output.len() + hash_len - 1) / hash_len;
    let mut t = [0u8; sha256::DIGEST_SIZE];
    let mut offset = 0;

    for i in 1..=n {
        let mut ctx = HmacSha256::new(prk);
        if i > 1 {
            ctx.update(&t);
        }
        ctx.update(info);
        ctx.update(&[i as u8]);
        t = ctx.finalize();

        let copy_len = (output.len() - offset).min(hash_len);
        output[offset..offset + copy_len].copy_from_slice(&t[..copy_len]);
        offset += copy_len;
    }
}

/// HKDF-Extract with SHA-384.
pub fn hkdf_extract_sha384(salt: &[u8], ikm: &[u8]) -> [u8; sha384::DIGEST_SIZE] {
    let salt = if salt.is_empty() {
        &[0u8; sha384::DIGEST_SIZE][..]
    } else {
        salt
    };
    hmac_sha384(salt, ikm)
}

/// HKDF-Expand with SHA-384.
pub fn hkdf_expand_sha384(prk: &[u8], info: &[u8], output: &mut [u8]) {
    let hash_len = sha384::DIGEST_SIZE;
    let n = (output.len() + hash_len - 1) / hash_len;
    let mut t = [0u8; sha384::DIGEST_SIZE];
    let mut offset = 0;

    for i in 1..=n {
        let mut ctx = HmacSha384::new(prk);
        if i > 1 {
            ctx.update(&t);
        }
        ctx.update(info);
        ctx.update(&[i as u8]);
        t = ctx.finalize();

        let copy_len = (output.len() - offset).min(hash_len);
        output[offset..offset + copy_len].copy_from_slice(&t[..copy_len]);
        offset += copy_len;
    }
}

/// TLS 1.3 HKDF-Expand-Label (RFC 8446 Section 7.1).
///
/// Derives keys using the label format:
///   HkdfLabel = struct {
///     uint16 length;
///     opaque label<7..255> = "tls13 " + Label;
///     opaque context<0..255> = Hash.value;
///   };
pub fn tls13_hkdf_expand_label_sha256(
    secret: &[u8],
    label: &[u8],
    context: &[u8],
    output: &mut [u8],
) {
    let mut info = [0u8; 512];
    let len = build_hkdf_label(&mut info, output.len(), label, context);
    hkdf_expand_sha256(secret, &info[..len], output);
}

/// TLS 1.3 HKDF-Expand-Label with SHA-384.
pub fn tls13_hkdf_expand_label_sha384(
    secret: &[u8],
    label: &[u8],
    context: &[u8],
    output: &mut [u8],
) {
    let mut info = [0u8; 512];
    let len = build_hkdf_label(&mut info, output.len(), label, context);
    hkdf_expand_sha384(secret, &info[..len], output);
}

/// Build the HkdfLabel struct for TLS 1.3.
fn build_hkdf_label(buf: &mut [u8], length: usize, label: &[u8], context: &[u8]) -> usize {
    let full_label_len = 6 + label.len(); // "tls13 " prefix
    let mut pos = 0;

    // uint16 length
    buf[pos] = (length >> 8) as u8;
    buf[pos + 1] = length as u8;
    pos += 2;

    // opaque label<7..255>
    buf[pos] = full_label_len as u8;
    pos += 1;
    buf[pos..pos + 6].copy_from_slice(b"tls13 ");
    pos += 6;
    buf[pos..pos + label.len()].copy_from_slice(label);
    pos += label.len();

    // opaque context<0..255>
    buf[pos] = context.len() as u8;
    pos += 1;
    buf[pos..pos + context.len()].copy_from_slice(context);
    pos += context.len();

    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hkdf_sha256_rfc5869_case1() {
        // RFC 5869 Test Case 1
        let ikm = [0x0bu8; 22];
        let salt: [u8; 13] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let info: [u8; 10] = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];

        let prk = hkdf_extract_sha256(&salt, &ikm);
        let expected_prk: [u8; 32] = [
            0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf, 0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b,
            0xba, 0x63, 0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31, 0x22, 0xec, 0x84, 0x4a,
            0xd7, 0xc2, 0xb3, 0xe5,
        ];
        assert_eq!(prk, expected_prk);

        let mut okm = [0u8; 42];
        hkdf_expand_sha256(&prk, &info, &mut okm);
        let expected_okm: [u8; 42] = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        assert_eq!(okm, expected_okm);
    }
}
