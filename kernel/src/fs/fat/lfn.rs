//! VFAT Long Filename (LFN) support functions.
//!
//! Handles encoding/decoding of VFAT long filename directory entries,
//! which store filenames as UTF-16LE across multiple 32-byte entries.

use alloc::string::String;
use alloc::vec::Vec;
use super::bpb::ATTR_LONG_NAME;

/// Compute the VFAT checksum of an 8.3 name (11 bytes).
pub(crate) fn lfn_checksum(name83: &[u8]) -> u8 {
    let mut sum: u8 = 0;
    for i in 0..11 {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(name83[i]);
    }
    sum
}

/// Extract 13 UTF-16LE characters from a 32-byte LFN directory entry.
pub(crate) fn lfn_extract_chars(entry: &[u8]) -> [u16; 13] {
    let mut chars = [0u16; 13];
    // Chars 1-5: bytes 1..10
    for j in 0..5 {
        chars[j] = u16::from_le_bytes([entry[1 + j * 2], entry[2 + j * 2]]);
    }
    // Chars 6-11: bytes 14..25
    for j in 0..6 {
        chars[5 + j] = u16::from_le_bytes([entry[14 + j * 2], entry[15 + j * 2]]);
    }
    // Chars 12-13: bytes 28..31
    for j in 0..2 {
        chars[11 + j] = u16::from_le_bytes([entry[28 + j * 2], entry[29 + j * 2]]);
    }
    chars
}

/// Convert a UTF-16LE LFN buffer to a String (ASCII only).
pub(crate) fn lfn_to_string(buf: &[u16], max_len: usize) -> String {
    let mut s = String::new();
    for i in 0..max_len {
        let c = buf[i];
        if c == 0x0000 || c == 0xFFFF {
            break;
        }
        if c < 128 {
            s.push(c as u8 as char);
        } else {
            s.push('?');
        }
    }
    s
}

/// Check if an accumulated LFN buffer matches a name (case-insensitive).
pub(crate) fn lfn_name_matches(lfn_buf: &[u16], lfn_max: usize, name: &str) -> bool {
    let mut lfn_len = 0;
    for i in 0..lfn_max {
        if lfn_buf[i] == 0x0000 || lfn_buf[i] == 0xFFFF {
            break;
        }
        lfn_len += 1;
    }
    if lfn_len != name.len() {
        return false;
    }
    for (i, b) in name.bytes().enumerate() {
        if i >= lfn_max {
            return false;
        }
        let lfn_char = lfn_buf[i];
        if lfn_char >= 128 {
            return false;
        }
        if (lfn_char as u8).to_ascii_lowercase() != b.to_ascii_lowercase() {
            return false;
        }
    }
    true
}

/// Check if a filename requires LFN entries (doesn't fit 8.3 format).
pub(crate) fn needs_lfn(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return true;
    }
    if name.starts_with('.') && name != "." && name != ".." {
        return true;
    }
    let dot_count = name.bytes().filter(|&b| b == b'.').count();
    if dot_count > 1 {
        return true;
    }
    let (base, ext) = if let Some(dot_pos) = name.find('.') {
        (&name[..dot_pos], &name[dot_pos + 1..])
    } else {
        (name, "")
    };
    if base.len() > 8 || ext.len() > 3 {
        return true;
    }
    for b in name.bytes() {
        match b {
            b' ' | b'+' | b',' | b';' | b'=' | b'[' | b']' => return true,
            _ => {}
        }
    }
    false
}

/// Store 13 UTF-16LE characters into a 32-byte LFN entry buffer.
pub(crate) fn lfn_store_chars(entry: &mut [u8], chars: &[u16; 13]) {
    for j in 0..5 {
        let bytes = chars[j].to_le_bytes();
        entry[1 + j * 2] = bytes[0];
        entry[2 + j * 2] = bytes[1];
    }
    for j in 0..6 {
        let bytes = chars[5 + j].to_le_bytes();
        entry[14 + j * 2] = bytes[0];
        entry[15 + j * 2] = bytes[1];
    }
    for j in 0..2 {
        let bytes = chars[11 + j].to_le_bytes();
        entry[28 + j * 2] = bytes[0];
        entry[29 + j * 2] = bytes[1];
    }
}

/// Build LFN directory entries for a name. Returns entries in disk order
/// (last LFN entry first, sequence 1 last, ready to write before the 8.3 entry).
pub(crate) fn make_lfn_entries(name: &str, name83: &[u8; 11]) -> Vec<[u8; 32]> {
    let chksum = lfn_checksum(name83);
    let utf16: Vec<u16> = name.bytes().map(|b| b as u16).collect();
    let num_entries = (utf16.len() + 12) / 13;

    let mut entries = Vec::with_capacity(num_entries);

    for seq in 1..=num_entries {
        let mut entry = [0u8; 32];
        let is_last = seq == num_entries;

        entry[0] = seq as u8 | if is_last { 0x40 } else { 0 };
        entry[11] = ATTR_LONG_NAME;
        entry[12] = 0;
        entry[13] = chksum;
        entry[26] = 0;
        entry[27] = 0;

        let start = (seq - 1) * 13;
        let mut chars = [0xFFFFu16; 13];
        for j in 0..13 {
            let idx = start + j;
            if idx < utf16.len() {
                chars[j] = utf16[idx];
            } else if idx == utf16.len() {
                chars[j] = 0x0000;
            }
        }
        lfn_store_chars(&mut entry, &chars);
        entries.push(entry);
    }

    entries.reverse();
    entries
}
