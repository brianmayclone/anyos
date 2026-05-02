//! ZLIB wrapper format (RFC 1950) around raw DEFLATE streams.

use alloc::vec::Vec;

use crate::deflate;
use crate::inflate;

/// Compress data into zlib format.
pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    out.extend_from_slice(&[0x78, 0x9c]);
    out.extend_from_slice(&deflate::deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Decompress zlib-wrapped DEFLATE data.
pub fn zlib_decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 6 {
        return None;
    }
    let header = u16::from_be_bytes([data[0], data[1]]);
    if header % 31 != 0 || data[0] & 0x0f != 8 {
        return None;
    }
    let compressed = &data[2..data.len() - 4];
    let output = inflate::inflate(compressed)?;
    let expected = u32::from_be_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);
    if adler32(&output) != expected {
        return None;
    }
    Some(output)
}

pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + *byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}
