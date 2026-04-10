//! Git pack file format (v2) — parsing and writing.
//!
//! Pack files contain multiple git objects in a compressed format.
//! Format: https://git-scm.com/docs/pack-format
//!
//! Object types in pack:
//! - OBJ_COMMIT (1), OBJ_TREE (2), OBJ_BLOB (3), OBJ_TAG (4)
//! - OBJ_OFS_DELTA (6), OBJ_REF_DELTA (7)

use alloc::vec::Vec;
use alloc::string::String;
use crate::oid::Oid;
use crate::object::{Object, ObjectType};
use crate::sha1;
use crate::inflate;
use crate::deflate;

use core::sync::atomic::{AtomicBool, Ordering};

/// Enable verbose pack parsing output.
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Set verbose mode for pack operations.
pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Pack object types.
const OBJ_COMMIT: u8 = 1;
const OBJ_TREE: u8 = 2;
const OBJ_BLOB: u8 = 3;
const OBJ_TAG: u8 = 4;
const OBJ_OFS_DELTA: u8 = 6;
const OBJ_REF_DELTA: u8 = 7;

/// A single entry from a pack file.
#[derive(Debug, Clone)]
pub struct PackEntry {
    pub obj_type: ObjectType,
    pub data: Vec<u8>,
    pub oid: Oid,
}

/// Parsed pack file.
#[derive(Debug)]
pub struct PackFile {
    pub entries: Vec<PackEntry>,
}

/// Parse a pack file (v2 format).
///
/// Pack format:
/// - 4 bytes: "PACK"
/// - 4 bytes: version (2)
/// - 4 bytes: number of objects (big-endian)
/// - N object entries
/// - 20 bytes: SHA-1 checksum of everything before
pub fn parse_pack(data: &[u8]) -> Option<PackFile> {
    if data.len() < 12 {
        return None;
    }

    // Header
    if &data[0..4] != b"PACK" {
        if verbose() {
            anyos_std::println!("[pack] ERROR: no PACK header, got {:?}", &data[0..4.min(data.len())]);
        }
        return None;
    }
    let version = read_u32_be(&data[4..8]);
    if version != 2 && version != 3 {
        if verbose() {
            anyos_std::println!("[pack] ERROR: unsupported version {}", version);
        }
        return None;
    }
    let num_objects = read_u32_be(&data[8..12]) as usize;
    if verbose() {
        anyos_std::println!("[pack] version={} objects={} total_size={}", version, num_objects, data.len());
    }

    let mut entries = Vec::with_capacity(num_objects);
    let mut pos = 12;

    // We need to store all resolved objects for delta resolution
    let mut resolved: Vec<(Oid, Vec<u8>, u8)> = Vec::with_capacity(num_objects);
    let mut offsets: Vec<usize> = Vec::with_capacity(num_objects);

    for _ in 0..num_objects {
        if pos >= data.len() {
            break;
        }

        // Read object header (variable-length encoding)
        let (obj_type_raw, uncompressed_size, header_len) = read_pack_entry_header(&data[pos..]);
        let entry_start = pos;
        pos += header_len;

        if verbose() {
            anyos_std::println!("[pack] obj {}/{}: offset={} type={} size={} hdr_len={}",
                entries.len() + 1, num_objects, entry_start, obj_type_raw, uncompressed_size, header_len);
        }

        match obj_type_raw {
            OBJ_COMMIT | OBJ_TREE | OBJ_BLOB | OBJ_TAG => {
                // Non-delta object: inflate the data
                let (inflated, consumed) = match inflate::inflate_counted(&data[pos..]) {
                    Some(r) => r,
                    None => {
                        if verbose() {
                            anyos_std::println!("[pack]   inflate FAILED at pos={} remaining={}", pos, data.len() - pos);
                        }
                        break;
                    }
                };
                pos += consumed;

                let obj_type = match obj_type_raw {
                    OBJ_COMMIT => ObjectType::Commit,
                    OBJ_TREE => ObjectType::Tree,
                    OBJ_BLOB => ObjectType::Blob,
                    OBJ_TAG => ObjectType::Tag,
                    _ => unreachable!(),
                };

                let oid = Oid::from_bytes(sha1::hash_object(obj_type.as_str(), &inflated));
                if verbose() {
                    anyos_std::println!("[pack]   OK: {} {} bytes compressed={} oid={}",
                        obj_type.as_str(), inflated.len(), consumed, oid.short());
                }
                resolved.push((oid, inflated.clone(), obj_type_raw));
                offsets.push(entry_start);

                entries.push(PackEntry {
                    obj_type,
                    data: inflated,
                    oid,
                });
            }
            OBJ_REF_DELTA => {
                // REF_DELTA: 20-byte base object SHA-1 + delta data
                if pos + 20 > data.len() {
                    if verbose() { anyos_std::println!("[pack]   REF_DELTA: not enough data for base sha"); }
                    break;
                }
                let mut base_sha = [0u8; 20];
                base_sha.copy_from_slice(&data[pos..pos + 20]);
                let base_oid = Oid::from_bytes(base_sha);
                pos += 20;

                if verbose() {
                    anyos_std::println!("[pack]   REF_DELTA base={}", base_oid.short());
                }

                let (delta_data, consumed) = match inflate::inflate_counted(&data[pos..]) {
                    Some(r) => r,
                    None => {
                        if verbose() { anyos_std::println!("[pack]   REF_DELTA inflate FAILED"); }
                        break;
                    }
                };
                pos += consumed;

                // Find base object and apply delta
                if let Some((_, base_data, base_type)) = resolved.iter().find(|(oid, _, _)| *oid == base_oid) {
                    let result = apply_delta(base_data, &delta_data);
                    let obj_type = pack_type_to_object_type(*base_type);

                    let oid = Oid::from_bytes(sha1::hash_object(
                        obj_type.as_str(),
                        &result,
                    ));
                    resolved.push((oid, result.clone(), *base_type));
                    offsets.push(entry_start);

                    entries.push(PackEntry {
                        obj_type,
                        data: result,
                        oid,
                    });
                }
            }
            OBJ_OFS_DELTA => {
                // OFS_DELTA: negative offset to base object + delta data
                let (offset, offset_bytes) = read_ofs_delta_offset(&data[pos..]);
                pos += offset_bytes;

                let (delta_data, consumed) = match inflate::inflate_counted(&data[pos..]) {
                    Some(r) => r,
                    None => break,
                };
                pos += consumed;

                // Base object is at absolute pack offset (entry_start - offset)
                // We need to find which resolved object was at that offset.
                // Use the offsets we recorded.
                let base_abs = entry_start.saturating_sub(offset);
                // Find the resolved object whose pack offset matches
                if let Some((_, base_data, base_type)) = offsets.iter()
                    .zip(resolved.iter())
                    .find(|(off, _)| **off == base_abs)
                    .map(|(_, res)| res)
                {
                    let result = apply_delta(base_data, &delta_data);
                    let obj_type = pack_type_to_object_type(*base_type);

                    let oid = Oid::from_bytes(sha1::hash_object(
                        obj_type.as_str(),
                        &result,
                    ));
                    resolved.push((oid, result.clone(), *base_type));
                    offsets.push(entry_start);

                    entries.push(PackEntry {
                        obj_type,
                        data: result,
                        oid,
                    });
                }
            }
            _ => {
                // Unknown type, skip
                break;
            }
        }
    }

    Some(PackFile { entries })
}

/// Build a pack file from a list of objects.
pub fn build_pack(objects: &[Object]) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header
    buf.extend_from_slice(b"PACK");
    buf.extend_from_slice(&2u32.to_be_bytes()); // version 2
    buf.extend_from_slice(&(objects.len() as u32).to_be_bytes());

    // Objects
    for obj in objects {
        let type_num = match obj.obj_type {
            ObjectType::Commit => OBJ_COMMIT,
            ObjectType::Tree => OBJ_TREE,
            ObjectType::Blob => OBJ_BLOB,
            ObjectType::Tag => OBJ_TAG,
        };

        // Write entry header
        write_pack_entry_header(&mut buf, type_num, obj.data.len());

        // Compress data
        let compressed = deflate::deflate(&obj.data);
        buf.extend_from_slice(&compressed);
    }

    // Checksum (SHA-1 of everything)
    let checksum = sha1::hash(&buf);
    buf.extend_from_slice(&checksum);

    buf
}

// ── Pack entry header encoding ──────────────────────────────────────────────

fn read_pack_entry_header(data: &[u8]) -> (u8, usize, usize) {
    if data.is_empty() {
        return (0, 0, 0);
    }

    let mut c = data[0];
    let obj_type = (c >> 4) & 0x07;
    let mut size = (c & 0x0F) as usize;
    let mut shift = 4;
    let mut pos = 1;

    while c & 0x80 != 0 && pos < data.len() {
        c = data[pos];
        size |= ((c & 0x7F) as usize) << shift;
        shift += 7;
        pos += 1;
    }

    (obj_type, size, pos)
}

fn write_pack_entry_header(buf: &mut Vec<u8>, obj_type: u8, mut size: usize) {
    let mut c = (obj_type << 4) | (size & 0x0F) as u8;
    size >>= 4;
    if size > 0 {
        c |= 0x80;
    }
    buf.push(c);

    while size > 0 {
        let mut c = (size & 0x7F) as u8;
        size >>= 7;
        if size > 0 {
            c |= 0x80;
        }
        buf.push(c);
    }
}

fn read_ofs_delta_offset(data: &[u8]) -> (usize, usize) {
    if data.is_empty() {
        return (0, 0);
    }

    let mut c = data[0];
    let mut offset = (c & 0x7F) as usize;
    let mut pos = 1;

    while c & 0x80 != 0 && pos < data.len() {
        c = data[pos];
        offset = ((offset + 1) << 7) | (c & 0x7F) as usize;
        pos += 1;
    }

    (offset, pos)
}

// ── Delta application ───────────────────────────────────────────────────────

/// Apply a git delta to a base object.
///
/// Delta format:
/// - Source length (variable-length int)
/// - Target length (variable-length int)
/// - Instructions:
///   - Copy: bit 7 set, followed by offset/size bytes
///   - Insert: bit 7 clear, value = number of bytes to insert
fn apply_delta(base: &[u8], delta: &[u8]) -> Vec<u8> {
    let mut pos = 0;

    // Read source length
    let (_src_len, bytes) = read_delta_size(&delta[pos..]);
    pos += bytes;

    // Read target length
    let (tgt_len, bytes) = read_delta_size(&delta[pos..]);
    pos += bytes;

    let mut result = Vec::with_capacity(tgt_len);

    while pos < delta.len() {
        let cmd = delta[pos];
        pos += 1;

        if cmd & 0x80 != 0 {
            // Copy instruction
            let mut offset = 0usize;
            let mut size = 0usize;

            if cmd & 0x01 != 0 { offset = delta.get(pos).copied().unwrap_or(0) as usize; pos += 1; }
            if cmd & 0x02 != 0 { offset |= (delta.get(pos).copied().unwrap_or(0) as usize) << 8; pos += 1; }
            if cmd & 0x04 != 0 { offset |= (delta.get(pos).copied().unwrap_or(0) as usize) << 16; pos += 1; }
            if cmd & 0x08 != 0 { offset |= (delta.get(pos).copied().unwrap_or(0) as usize) << 24; pos += 1; }

            if cmd & 0x10 != 0 { size = delta.get(pos).copied().unwrap_or(0) as usize; pos += 1; }
            if cmd & 0x20 != 0 { size |= (delta.get(pos).copied().unwrap_or(0) as usize) << 8; pos += 1; }
            if cmd & 0x40 != 0 { size |= (delta.get(pos).copied().unwrap_or(0) as usize) << 16; pos += 1; }

            if size == 0 {
                size = 0x10000;
            }

            if offset + size <= base.len() {
                result.extend_from_slice(&base[offset..offset + size]);
            }
        } else if cmd > 0 {
            // Insert instruction
            let n = cmd as usize;
            if pos + n <= delta.len() {
                result.extend_from_slice(&delta[pos..pos + n]);
                pos += n;
            }
        }
        // cmd == 0 is reserved
    }

    result
}

fn read_delta_size(data: &[u8]) -> (usize, usize) {
    let mut size = 0usize;
    let mut shift = 0;
    let mut pos = 0;

    loop {
        if pos >= data.len() {
            break;
        }
        let c = data[pos];
        size |= ((c & 0x7F) as usize) << shift;
        shift += 7;
        pos += 1;
        if c & 0x80 == 0 {
            break;
        }
    }

    (size, pos)
}

// ── Delta generation ────────────────────────────────────────────────────────

/// Generate a delta between base and target.
/// Uses a simple approach: if target is very different, just emit insert instructions.
pub fn generate_delta(base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut delta = Vec::new();

    // Source length
    write_delta_size(&mut delta, base.len());
    // Target length
    write_delta_size(&mut delta, target.len());

    // Simple strategy: try to find matching blocks, otherwise insert
    let mut tgt_pos = 0;

    while tgt_pos < target.len() {
        // Try to find a match in base (simple sliding window)
        let best = find_best_match(base, &target[tgt_pos..]);

        if let Some((offset, length)) = best {
            // Flush any pending insert before copy
            // Emit copy instruction
            emit_copy(&mut delta, offset, length);
            tgt_pos += length;
        } else {
            // No match, emit insert (up to 127 bytes at a time)
            let insert_len = core::cmp::min(127, target.len() - tgt_pos);
            delta.push(insert_len as u8);
            delta.extend_from_slice(&target[tgt_pos..tgt_pos + insert_len]);
            tgt_pos += insert_len;
        }
    }

    delta
}

fn find_best_match(base: &[u8], target: &[u8]) -> Option<(usize, usize)> {
    if target.len() < 4 || base.len() < 4 {
        return None;
    }

    let mut best_offset = 0;
    let mut best_length = 0;
    let min_match = 8; // Minimum match length to be worth a copy instruction

    // Simple O(n*m) search — for production, use a hash index
    for i in 0..base.len() {
        let mut len = 0;
        while len < target.len() && i + len < base.len() && base[i + len] == target[len] {
            len += 1;
        }
        if len > best_length && len >= min_match {
            best_offset = i;
            best_length = len;
        }
    }

    if best_length >= min_match {
        Some((best_offset, best_length))
    } else {
        None
    }
}

fn emit_copy(delta: &mut Vec<u8>, mut offset: usize, mut size: usize) {
    let mut cmd = 0x80u8;
    let mut extra = Vec::new();

    if offset & 0xFF != 0 || (offset >> 8) == 0 { cmd |= 0x01; extra.push((offset & 0xFF) as u8); }
    offset >>= 8;
    if offset & 0xFF != 0 { cmd |= 0x02; extra.push((offset & 0xFF) as u8); }
    offset >>= 8;
    if offset & 0xFF != 0 { cmd |= 0x04; extra.push((offset & 0xFF) as u8); }
    offset >>= 8;
    if offset & 0xFF != 0 { cmd |= 0x08; extra.push((offset & 0xFF) as u8); }

    if size & 0xFF != 0 || size < 0x10000 { cmd |= 0x10; extra.push((size & 0xFF) as u8); }
    size >>= 8;
    if size & 0xFF != 0 { cmd |= 0x20; extra.push((size & 0xFF) as u8); }
    size >>= 8;
    if size & 0xFF != 0 { cmd |= 0x40; extra.push((size & 0xFF) as u8); }

    delta.push(cmd);
    delta.extend_from_slice(&extra);
}

fn write_delta_size(buf: &mut Vec<u8>, mut size: usize) {
    loop {
        let mut c = (size & 0x7F) as u8;
        size >>= 7;
        if size > 0 {
            c |= 0x80;
        }
        buf.push(c);
        if size == 0 {
            break;
        }
    }
}

// ── Side-band demuxing ──────────────────────────────────────────────────────

/// Demux side-band data from git smart HTTP responses.
/// Channel 1 = pack data, Channel 2 = progress, Channel 3 = error
pub fn demux_sideband(data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut pack_data = Vec::new();
    let mut progress = Vec::new();
    let mut errors = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        // Read pkt-line length (4 hex digits)
        if pos + 4 > data.len() {
            break;
        }
        let len_hex = core::str::from_utf8(&data[pos..pos + 4]).unwrap_or("0000");
        let pkt_len = usize::from_str_radix(len_hex, 16).unwrap_or(0);
        pos += 4;

        if pkt_len == 0 {
            // Flush packet
            continue;
        }
        if pkt_len < 5 {
            pos += pkt_len - 4;
            continue;
        }

        let payload_len = pkt_len - 5; // -4 for length, -1 for channel byte
        if pos >= data.len() {
            break;
        }

        let channel = data[pos];
        pos += 1;

        let end = core::cmp::min(pos + payload_len, data.len());
        let payload = &data[pos..end];

        match channel {
            1 => pack_data.extend_from_slice(payload),
            2 => progress.extend_from_slice(payload),
            3 => errors.extend_from_slice(payload),
            _ => {}
        }

        pos = end;
    }

    (pack_data, progress, errors)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn pack_type_to_object_type(t: u8) -> ObjectType {
    match t {
        OBJ_COMMIT => ObjectType::Commit,
        OBJ_TREE => ObjectType::Tree,
        OBJ_BLOB => ObjectType::Blob,
        OBJ_TAG => ObjectType::Tag,
        _ => ObjectType::Blob,
    }
}

fn read_u32_be(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}
