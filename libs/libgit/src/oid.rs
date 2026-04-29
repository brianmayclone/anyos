//! Git Object ID (SHA-1 hash) type.

use alloc::string::String;
use core::fmt;

/// A 20-byte SHA-1 object identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid {
    bytes: [u8; 20],
}

impl Oid {
    /// The zero/null OID.
    pub const ZERO: Oid = Oid { bytes: [0; 20] };

    /// Create from raw bytes.
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Oid { bytes }
    }

    /// Parse from a 40-character hex string.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 40 {
            return None;
        }
        let mut bytes = [0u8; 20];
        for i in 0..20 {
            bytes[i] = hex_byte(hex.as_bytes()[i * 2], hex.as_bytes()[i * 2 + 1])?;
        }
        Some(Oid { bytes })
    }

    /// Parse from a hex string prefix (at least 4 chars, for abbreviations).
    pub fn from_hex_prefix(hex: &str) -> Option<Self> {
        if hex.len() < 4 || hex.len() > 40 {
            return None;
        }
        let mut bytes = [0u8; 20];
        let full_bytes = hex.len() / 2;
        for i in 0..full_bytes {
            bytes[i] = hex_byte(hex.as_bytes()[i * 2], hex.as_bytes()[i * 2 + 1])?;
        }
        if hex.len() % 2 == 1 {
            let hi = hex_digit(hex.as_bytes()[hex.len() - 1])?;
            bytes[full_bytes] = hi << 4;
        }
        Some(Oid { bytes })
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.bytes
    }

    /// Convert to 40-character hex string.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(40);
        for b in &self.bytes {
            s.push(HEX_CHARS[(b >> 4) as usize] as char);
            s.push(HEX_CHARS[(b & 0xf) as usize] as char);
        }
        s
    }

    /// Get the short hex representation (first 7 chars).
    pub fn short(&self) -> String {
        self.to_hex()[..7].into()
    }

    /// Check if this is the zero OID.
    pub fn is_zero(&self) -> bool {
        self.bytes == [0; 20]
    }

    /// Get the first two hex characters (used for object storage path).
    pub fn dir_prefix(&self) -> (u8, u8) {
        (
            HEX_CHARS[(self.bytes[0] >> 4) as usize],
            HEX_CHARS[(self.bytes[0] & 0xf) as usize],
        )
    }
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_digit(hi)? << 4) | hex_digit(lo)?)
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.to_hex())
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}
