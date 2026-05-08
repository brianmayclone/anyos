//! Gzip compression/decompression (RFC 1952).
//!
//! Gzip is a thin wrapper around DEFLATE with a 10-byte header and 8-byte trailer.
//! Reuses the existing `deflate` and `inflate` modules for the actual compression.

use crate::crc32;
use crate::deflate;
use crate::inflate;
use alloc::vec::Vec;

// ── Gzip constants ──────────────────────────────────────────────────────────

const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
const METHOD_DEFLATE: u8 = 0x08;

// Flag bits in header byte 3
const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;
const RESERVED_FLAGS: u8 = 0xE0;

// ── Compress ────────────────────────────────────────────────────────────────

/// Compress data into gzip format (RFC 1952).
pub fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let crc = crc32::crc32(data);
    let isize = data.len() as u32;
    let compressed = deflate::deflate(data);

    let mut out = Vec::with_capacity(10 + compressed.len() + 8);

    // Header (10 bytes)
    out.push(GZIP_MAGIC[0]); // ID1
    out.push(GZIP_MAGIC[1]); // ID2
    out.push(METHOD_DEFLATE); // CM
    out.push(0); // FLG (no extras)
    out.extend_from_slice(&[0; 4]); // MTIME (unknown)
    out.push(0); // XFL
    out.push(0xFF); // OS = unknown

    // Compressed data (raw DEFLATE stream)
    out.extend_from_slice(&compressed);

    // Trailer (8 bytes)
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&isize.to_le_bytes());

    out
}

// ── Decompress ──────────────────────────────────────────────────────────────

/// Decompress gzip data (RFC 1952). Returns None on error.
pub fn gzip_decompress(data: &[u8]) -> Option<Vec<u8>> {
    gzip_decompress_with_limit(data, usize::MAX)
}

/// Decompress gzip data and fail once output would exceed `max_output`.
pub fn gzip_decompress_with_limit(data: &[u8], max_output: usize) -> Option<Vec<u8>> {
    if data.len() < 18 {
        return None; // minimum: 10 header + 0 data + 8 trailer
    }

    // Validate magic and method
    if data[0] != GZIP_MAGIC[0] || data[1] != GZIP_MAGIC[1] {
        return None;
    }
    if data[2] != METHOD_DEFLATE {
        return None;
    }

    let flags = data[3];
    if flags & RESERVED_FLAGS != 0 {
        return None;
    }
    let mut pos = 10usize; // skip fixed header

    // Skip optional FEXTRA field
    if flags & FEXTRA != 0 {
        if pos + 2 > data.len() {
            return None;
        }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos = pos.checked_add(2)?.checked_add(xlen)?;
        if pos > data.len() {
            return None;
        }
    }

    // Skip optional FNAME (null-terminated string)
    if flags & FNAME != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return None;
        }
        pos += 1; // skip null terminator
    }

    // Skip optional FCOMMENT (null-terminated string)
    if flags & FCOMMENT != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return None;
        }
        pos += 1;
    }

    // Skip optional FHCRC (2-byte CRC16 of header)
    if flags & FHCRC != 0 {
        if pos + 2 > data.len() {
            return None;
        }
        pos += 2;
    }

    if pos >= data.len() {
        return None;
    }

    // Trailer is the last 8 bytes
    if data.len() < pos + 8 {
        return None;
    }
    let trailer_start = data.len() - 8;

    let expected_crc = u32::from_le_bytes([
        data[trailer_start],
        data[trailer_start + 1],
        data[trailer_start + 2],
        data[trailer_start + 3],
    ]);
    let expected_isize = u32::from_le_bytes([
        data[trailer_start + 4],
        data[trailer_start + 5],
        data[trailer_start + 6],
        data[trailer_start + 7],
    ]);
    let expected_size = expected_isize as usize;
    if expected_size > max_output {
        return None;
    }

    // Decompress the DEFLATE stream (between header and trailer)
    let compressed = &data[pos..trailer_start];
    let decompressed = inflate::inflate_with_limit(compressed, expected_size)?;

    // Verify CRC-32
    let actual_crc = crc32::crc32(&decompressed);
    if actual_crc != expected_crc {
        return None;
    }

    // Verify ISIZE (original size mod 2^32)
    let actual_isize = decompressed.len() as u32;
    if actual_isize != expected_isize {
        return None;
    }

    Some(decompressed)
}

/// Check if data starts with gzip magic bytes.
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1]
}

#[cfg(test)]
mod tests {
    use super::{gzip_compress, gzip_decompress, gzip_decompress_with_limit};

    #[test]
    fn decompress_respects_output_limit() {
        let compressed = gzip_compress(b"hello");

        assert_eq!(
            gzip_decompress_with_limit(&compressed, 5).as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(gzip_decompress_with_limit(&compressed, 4), None);
    }

    #[test]
    fn rejects_reserved_header_flags() {
        let mut compressed = gzip_compress(b"hello");
        compressed[3] = 0x20;

        assert_eq!(gzip_decompress(&compressed), None);
    }

    #[test]
    fn rejects_output_beyond_trailer_size() {
        let mut compressed = gzip_compress(b"hello");
        let trailer = compressed.len() - 8;
        compressed[trailer..trailer + 4].copy_from_slice(&0u32.to_le_bytes());
        compressed[trailer + 4..trailer + 8].copy_from_slice(&0u32.to_le_bytes());

        assert_eq!(gzip_decompress(&compressed), None);
    }
}
