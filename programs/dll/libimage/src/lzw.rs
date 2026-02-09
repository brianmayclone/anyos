// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! LZW decompressor for GIF images.
//!
//! Implements variable-width code LZW decompression with string table stored
//! in a caller-provided scratch buffer. No heap allocation.

use crate::types::*;

/// Maximum LZW code size (12 bits = 4096 entries).
const MAX_TABLE_SIZE: usize = 4096;

/// Each table entry: (prefix index: u16, suffix byte: u8, length: u16) = 5 bytes.
/// We pack into 8 bytes per entry for alignment.
const ENTRY_SIZE: usize = 8;

/// Required scratch size for the LZW string table.
pub const LZW_SCRATCH_SIZE: usize = MAX_TABLE_SIZE * ENTRY_SIZE;

/// Read a table entry's prefix index from scratch buffer.
#[inline]
fn entry_prefix(scratch: &[u8], idx: usize) -> u16 {
    let off = idx * ENTRY_SIZE;
    u16::from_le_bytes([scratch[off], scratch[off + 1]])
}

/// Read a table entry's suffix byte from scratch buffer.
#[inline]
fn entry_suffix(scratch: &[u8], idx: usize) -> u8 {
    scratch[idx * ENTRY_SIZE + 2]
}

/// Read a table entry's string length from scratch buffer.
#[inline]
fn entry_length(scratch: &[u8], idx: usize) -> u16 {
    let off = idx * ENTRY_SIZE + 4;
    u16::from_le_bytes([scratch[off], scratch[off + 1]])
}

/// Write a table entry into the scratch buffer.
#[inline]
fn set_entry(scratch: &mut [u8], idx: usize, prefix: u16, suffix: u8, length: u16) {
    let off = idx * ENTRY_SIZE;
    let p = prefix.to_le_bytes();
    let l = length.to_le_bytes();
    scratch[off] = p[0];
    scratch[off + 1] = p[1];
    scratch[off + 2] = suffix;
    scratch[off + 3] = 0; // padding
    scratch[off + 4] = l[0];
    scratch[off + 5] = l[1];
    scratch[off + 6] = 0;
    scratch[off + 7] = 0;
}

/// Output the string for a table entry (follows prefix chain).
/// Returns the first byte of the string (needed for table construction).
fn output_string(scratch: &[u8], idx: usize, output: &mut [u8], out_pos: usize) -> (u8, usize) {
    let len = entry_length(scratch, idx) as usize;
    if out_pos + len > output.len() {
        return (0, 0);
    }

    // Walk chain backwards, fill from the end
    let mut pos = out_pos + len - 1;
    let mut cur = idx;
    loop {
        output[pos] = entry_suffix(scratch, cur);
        let prefix = entry_prefix(scratch, cur) as usize;
        if prefix == 0xFFFF {
            break;
        }
        if pos == 0 {
            break;
        }
        pos -= 1;
        cur = prefix;
    }

    let first = output[out_pos];
    (first, len)
}

/// Decompress LZW-encoded GIF sub-block data.
///
/// - `data`: concatenated sub-block payload (without length bytes)
/// - `output`: destination for decompressed palette indices
/// - `min_code_size`: LZW minimum code size from the GIF stream
/// - `scratch`: working buffer, must be at least `LZW_SCRATCH_SIZE` bytes
///
/// Returns the number of bytes written to `output`, or a negative error code.
pub fn decompress(data: &[u8], output: &mut [u8], min_code_size: u8, scratch: &mut [u8]) -> i32 {
    if min_code_size < 2 || min_code_size > 11 {
        return ERR_INVALID_DATA;
    }
    if scratch.len() < LZW_SCRATCH_SIZE {
        return ERR_SCRATCH_TOO_SMALL;
    }

    let clear_code = 1u16 << min_code_size;
    let eoi_code = clear_code + 1;

    // Bit reader state
    let mut bit_pos: usize = 0;
    let total_bits = data.len() * 8;

    // Read `n_bits` from the data stream (LSB first, as per GIF spec).
    let read_code = |bit_pos: &mut usize, n_bits: u8| -> Option<u16> {
        let n = n_bits as usize;
        if *bit_pos + n > total_bits {
            return None;
        }
        let byte_off = *bit_pos / 8;
        let bit_off = *bit_pos % 8;

        // Read up to 3 bytes to cover the code
        let mut raw: u32 = data[byte_off] as u32;
        if byte_off + 1 < data.len() {
            raw |= (data[byte_off + 1] as u32) << 8;
        }
        if byte_off + 2 < data.len() {
            raw |= (data[byte_off + 2] as u32) << 16;
        }

        let code = ((raw >> bit_off) & ((1u32 << n) - 1)) as u16;
        *bit_pos += n;
        Some(code)
    };

    let mut out_pos: usize = 0;
    let mut code_size: u8 = min_code_size + 1;
    let mut table_size: usize;
    let mut prev_code: i32;

    // Initialize table
    let init_table = |scratch: &mut [u8]| -> usize {
        for i in 0..clear_code as usize {
            set_entry(scratch, i, 0xFFFF, i as u8, 1);
        }
        // clear_code and eoi_code entries (not used for output but occupy slots)
        set_entry(scratch, clear_code as usize, 0xFFFF, 0, 0);
        set_entry(scratch, eoi_code as usize, 0xFFFF, 0, 0);
        (eoi_code + 1) as usize
    };

    table_size = init_table(scratch);
    prev_code = -1;

    loop {
        let code = match read_code(&mut bit_pos, code_size) {
            Some(c) => c,
            None => break,
        };

        if code == eoi_code {
            break;
        }

        if code == clear_code {
            table_size = init_table(scratch);
            code_size = min_code_size + 1;
            prev_code = -1;
            continue;
        }

        let code_idx = code as usize;

        if code_idx < table_size {
            // Code is in the table
            let (first, len) = output_string(scratch, code_idx, output, out_pos);
            if len == 0 && entry_length(scratch, code_idx) > 0 {
                return ERR_BUFFER_TOO_SMALL;
            }
            out_pos += len;

            // Add new entry: prev_code string + first byte of current string
            if prev_code >= 0 && table_size < MAX_TABLE_SIZE {
                let prev_len = entry_length(scratch, prev_code as usize);
                set_entry(scratch, table_size, prev_code as u16, first, prev_len + 1);
                table_size += 1;
            }
        } else if code_idx == table_size {
            // Special case: code not yet in table
            if prev_code < 0 {
                return ERR_INVALID_DATA;
            }
            let prev_idx = prev_code as usize;
            // The new string is prev_code's string + first byte of prev_code's string
            let (first, _) = output_string(scratch, prev_idx, output, out_pos);

            let prev_len = entry_length(scratch, prev_idx);
            if table_size < MAX_TABLE_SIZE {
                set_entry(scratch, table_size, prev_code as u16, first, prev_len + 1);
                table_size += 1;
            }

            // Output the new entry
            let (_, len) = output_string(scratch, code_idx, output, out_pos);
            if len == 0 {
                return ERR_BUFFER_TOO_SMALL;
            }
            out_pos += len;
        } else {
            return ERR_INVALID_DATA;
        }

        prev_code = code as i32;

        // Increase code size when table reaches the next power of 2
        if table_size >= (1usize << code_size) && code_size < 12 {
            code_size += 1;
        }
    }

    out_pos as i32
}
