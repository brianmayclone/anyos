//! Password hashing and verification helpers.

use crate::crypto::md5::md5_hex;
use crate::crypto::pbkdf2::pbkdf2_sha256;
use crate::crypto::random::fill_random;
use alloc::format;
use alloc::string::String;

const PASSWORD_SCHEME: &str = "pbkdf2-sha256";
const PASSWORD_ITERS: u32 = 100_000;
const PASSWORD_SALT_LEN: usize = 16;
const PASSWORD_HASH_LEN: usize = 32;

/// Hash a password using PBKDF2-HMAC-SHA256 with a random salt.
pub fn hash_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }

    let mut salt = [0u8; PASSWORD_SALT_LEN];
    fill_random(&mut salt);
    let digest = pbkdf2_sha256(password.as_bytes(), &salt, PASSWORD_ITERS);

    format!(
        "{}${}${}${}",
        PASSWORD_SCHEME,
        PASSWORD_ITERS,
        encode_hex(&salt),
        encode_hex(&digest),
    )
}

/// Verify a password against a stored password record.
pub fn verify_password(password: &str, stored: &str) -> bool {
    if stored.is_empty() {
        return password.is_empty();
    }

    if let Some((iterations, salt, expected)) = parse_pbkdf2_sha256(stored) {
        let actual = pbkdf2_sha256(password.as_bytes(), &salt, iterations);
        return constant_time_eq(&actual, &expected);
    }

    // Backward compatibility for legacy MD5 password records.
    if stored.len() == 32 && stored.as_bytes().iter().all(|b| b.is_ascii_hexdigit()) {
        let actual = md5_hex(password.as_bytes());
        return constant_time_eq(actual.as_slice(), stored.as_bytes());
    }

    false
}

/// Returns true when the stored hash should be rewritten in the current format.
pub fn needs_rehash(stored: &str) -> bool {
    if stored.is_empty() {
        return false;
    }
    match parse_pbkdf2_sha256(stored) {
        Some((iterations, _, _)) => iterations < PASSWORD_ITERS,
        None => true,
    }
}

fn parse_pbkdf2_sha256(
    stored: &str,
) -> Option<(u32, [u8; PASSWORD_SALT_LEN], [u8; PASSWORD_HASH_LEN])> {
    let mut parts = stored.split('$');
    let scheme = parts.next()?;
    let iter_str = parts.next()?;
    let salt_hex = parts.next()?;
    let hash_hex = parts.next()?;
    if parts.next().is_some() || scheme != PASSWORD_SCHEME {
        return None;
    }

    let iterations = iter_str.parse().ok()?;
    let salt = decode_hex_16(salt_hex)?;
    let hash = decode_hex_32(hash_hex)?;
    Some((iterations, salt, hash))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex_n<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    let bytes = hex.as_bytes();
    for i in 0..N {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn decode_hex_16(hex: &str) -> Option<[u8; PASSWORD_SALT_LEN]> {
    decode_hex_n::<PASSWORD_SALT_LEN>(hex)
}

fn decode_hex_32(hex: &str) -> Option<[u8; PASSWORD_HASH_LEN]> {
    decode_hex_n::<PASSWORD_HASH_LEN>(hex)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
