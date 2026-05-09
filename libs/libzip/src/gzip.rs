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

pub const GZIP_STATUS_OK: u32 = 0;
pub const GZIP_ERR_TOO_SHORT: u32 = 1;
pub const GZIP_ERR_BAD_MAGIC: u32 = 2;
pub const GZIP_ERR_BAD_METHOD: u32 = 3;
pub const GZIP_ERR_BAD_FLAGS: u32 = 4;
pub const GZIP_ERR_BAD_HEADER: u32 = 5;
pub const GZIP_ERR_TOO_LARGE: u32 = 6;
pub const GZIP_ERR_INFLATE: u32 = 7;
pub const GZIP_ERR_BAD_CRC: u32 = 8;
pub const GZIP_ERR_BAD_SIZE: u32 = 9;
pub const GZIP_ERR_READ_FILE: u32 = 10;
pub const GZIP_ERR_WRITE_FILE: u32 = 11;

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
    gzip_decompress_with_limit(data, usize::MAX).ok()
}

/// Decompress gzip data and fail once output would exceed `max_output`.
pub fn gzip_decompress_with_limit(data: &[u8], max_output: usize) -> Result<Vec<u8>, u32> {
    if data.len() < 18 {
        return Err(GZIP_ERR_TOO_SHORT); // minimum: 10 header + 0 data + 8 trailer
    }

    // Validate magic and method
    if data[0] != GZIP_MAGIC[0] || data[1] != GZIP_MAGIC[1] {
        return Err(GZIP_ERR_BAD_MAGIC);
    }
    if data[2] != METHOD_DEFLATE {
        return Err(GZIP_ERR_BAD_METHOD);
    }

    let flags = data[3];
    if flags & RESERVED_FLAGS != 0 {
        return Err(GZIP_ERR_BAD_FLAGS);
    }
    let mut pos = 10usize; // skip fixed header

    // Skip optional FEXTRA field
    if flags & FEXTRA != 0 {
        if pos + 2 > data.len() {
            return Err(GZIP_ERR_BAD_HEADER);
        }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos = pos
            .checked_add(2)
            .and_then(|p| p.checked_add(xlen))
            .ok_or(GZIP_ERR_BAD_HEADER)?;
        if pos > data.len() {
            return Err(GZIP_ERR_BAD_HEADER);
        }
    }

    // Skip optional FNAME (null-terminated string)
    if flags & FNAME != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return Err(GZIP_ERR_BAD_HEADER);
        }
        pos += 1; // skip null terminator
    }

    // Skip optional FCOMMENT (null-terminated string)
    if flags & FCOMMENT != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return Err(GZIP_ERR_BAD_HEADER);
        }
        pos += 1;
    }

    // Skip optional FHCRC (2-byte CRC16 of header)
    if flags & FHCRC != 0 {
        if pos + 2 > data.len() {
            return Err(GZIP_ERR_BAD_HEADER);
        }
        pos += 2;
    }

    if pos >= data.len() {
        return Err(GZIP_ERR_BAD_HEADER);
    }

    // Trailer is the last 8 bytes
    if data.len() < pos + 8 {
        return Err(GZIP_ERR_BAD_HEADER);
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
        return Err(GZIP_ERR_TOO_LARGE);
    }

    // Decompress the DEFLATE stream (between header and trailer)
    let compressed = &data[pos..trailer_start];
    let decompressed =
        inflate::inflate_with_output_capacity(compressed, expected_size, expected_size)
            .ok_or(GZIP_ERR_INFLATE)?;

    // Verify CRC-32
    let actual_crc = crc32::crc32(&decompressed);
    if actual_crc != expected_crc {
        return Err(GZIP_ERR_BAD_CRC);
    }

    // Verify ISIZE (original size mod 2^32)
    let actual_isize = decompressed.len() as u32;
    if actual_isize != expected_isize {
        return Err(GZIP_ERR_BAD_SIZE);
    }

    Ok(decompressed)
}

/// Check if data starts with gzip magic bytes.
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == GZIP_MAGIC[0] && data[1] == GZIP_MAGIC[1]
}

#[cfg(test)]
mod tests {
    use super::{gzip_compress, gzip_decompress, gzip_decompress_with_limit, GZIP_ERR_TOO_LARGE};

    #[test]
    fn decompress_respects_output_limit() {
        let compressed = gzip_compress(b"hello");

        assert_eq!(
            gzip_decompress_with_limit(&compressed, 5).ok().as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(
            gzip_decompress_with_limit(&compressed, 4),
            Err(GZIP_ERR_TOO_LARGE)
        );
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
