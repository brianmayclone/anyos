//! ZIP archive format (PKZIP / APPNOTE 6.3.x).
//!
//! Supports reading and writing ZIP archives with Stored and Deflate methods.

use alloc::string::String;
use alloc::vec::Vec;
use crate::crc32;
use crate::inflate;
use crate::deflate;

// ─── Constants ──────────────────────────────────────────────────────────────

const LOCAL_FILE_HEADER_SIG: u32 = 0x04034B50;
const CENTRAL_DIR_SIG: u32 = 0x02014B50;
const END_CENTRAL_DIR_SIG: u32 = 0x06054B50;

const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

// ─── Utility ────────────────────────────────────────────────────────────────

fn read_u16(data: &[u8], offset: usize) -> u16 {
    if offset + 2 > data.len() { return 0; }
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() { return 0; }
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

fn write_u16(buf: &mut Vec<u8>, val: u16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

// ─── ZIP Entry ──────────────────────────────────────────────────────────────

/// A single file entry in a ZIP archive.
pub struct ZipEntry {
    pub name: String,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub crc32: u32,
    pub method: u16,
    pub local_header_offset: u32,
    // Offset to actual compressed data within archive
    pub data_offset: u32,
}

// ─── ZIP Reader ─────────────────────────────────────────────────────────────

/// A parsed ZIP archive (read-only).
pub struct ZipReader {
    pub data: Vec<u8>,
    pub entries: Vec<ZipEntry>,
}

impl ZipReader {
    /// Parse a ZIP archive from raw bytes.
    pub fn parse(data: Vec<u8>) -> Option<ZipReader> {
        let len = data.len();
        if len < 22 {
            return None;
        }

        // Find End of Central Directory record (search backwards)
        let mut eocd_offset = None;
        let search_start = if len > 65557 { len - 65557 } else { 0 };
        let mut i = len - 22;
        loop {
            if read_u32(&data, i) == END_CENTRAL_DIR_SIG {
                eocd_offset = Some(i);
                break;
            }
            if i == search_start {
                break;
            }
            i -= 1;
        }

        let eocd = eocd_offset?;
        let entry_count = read_u16(&data, eocd + 10) as usize;
        let central_dir_offset = read_u32(&data, eocd + 16) as usize;

        // Parse central directory entries
        let mut entries = Vec::with_capacity(entry_count);
        let mut pos = central_dir_offset;

        for _ in 0..entry_count {
            if pos + 46 > len || read_u32(&data, pos) != CENTRAL_DIR_SIG {
                break;
            }

            let method = read_u16(&data, pos + 10);
            let crc = read_u32(&data, pos + 16);
            let compressed_size = read_u32(&data, pos + 20);
            let uncompressed_size = read_u32(&data, pos + 24);
            let name_len = read_u16(&data, pos + 28) as usize;
            let extra_len = read_u16(&data, pos + 30) as usize;
            let comment_len = read_u16(&data, pos + 32) as usize;
            let local_header_offset = read_u32(&data, pos + 42);

            let name_start = pos + 46;
            let name_end = (name_start + name_len).min(len);
            let name = core::str::from_utf8(&data[name_start..name_end])
                .unwrap_or("")
                .into();

            // Calculate actual data offset from local header
            let lh = local_header_offset as usize;
            let data_offset = if lh + 30 <= len {
                let lh_name_len = read_u16(&data, lh + 26) as u32;
                let lh_extra_len = read_u16(&data, lh + 28) as u32;
                local_header_offset + 30 + lh_name_len + lh_extra_len
            } else {
                0
            };

            entries.push(ZipEntry {
                name,
                compressed_size,
                uncompressed_size,
                crc32: crc,
                method,
                local_header_offset,
                data_offset,
            });

            pos += 46 + name_len + extra_len + comment_len;
        }

        Some(ZipReader { data, entries })
    }

    /// Extract an entry by index. Returns decompressed data or None.
    pub fn extract(&self, index: usize) -> Option<Vec<u8>> {
        let entry = self.entries.get(index)?;
        let start = entry.data_offset as usize;
        let end = start + entry.compressed_size as usize;

        if end > self.data.len() {
            return None;
        }

        let compressed = &self.data[start..end];

        let decompressed = match entry.method {
            METHOD_STORED => compressed.to_vec(),
            METHOD_DEFLATE => inflate::inflate(compressed)?,
            _ => return None, // Unsupported method
        };

        // Verify CRC
        if entry.uncompressed_size > 0 {
            let actual_crc = crc32::crc32(&decompressed);
            if actual_crc != entry.crc32 {
                return None; // CRC mismatch
            }
        }

        Some(decompressed)
    }

    /// Get entry count.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// ─── ZIP Writer ─────────────────────────────────────────────────────────────

struct WriterEntry {
    name: String,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    method: u16,
    local_header_offset: u32,
    compressed_data: Vec<u8>,
}

/// Builds a new ZIP archive in memory.
pub struct ZipWriter {
    entries: Vec<WriterEntry>,
}

impl ZipWriter {
    pub fn new() -> Self {
        ZipWriter { entries: Vec::new() }
    }

    /// Add a file entry with optional DEFLATE compression.
    /// `compress` = true uses DEFLATE, false uses Stored.
    pub fn add(&mut self, name: &str, data: &[u8], compress: bool) {
        let crc = crc32::crc32(data);
        let uncompressed_size = data.len() as u32;

        let (method, compressed_data) = if compress && !data.is_empty() {
            let compressed = deflate::deflate(data);
            // Only use compressed if it's actually smaller
            if compressed.len() < data.len() {
                (METHOD_DEFLATE, compressed)
            } else {
                (METHOD_STORED, data.to_vec())
            }
        } else {
            (METHOD_STORED, data.to_vec())
        };

        let compressed_size = compressed_data.len() as u32;

        self.entries.push(WriterEntry {
            name: String::from(name),
            crc32: crc,
            compressed_size,
            uncompressed_size,
            method,
            local_header_offset: 0, // filled in during finalize
            compressed_data,
        });
    }

    /// Add a directory entry (name should end with '/').
    pub fn add_directory(&mut self, name: &str) {
        self.entries.push(WriterEntry {
            name: String::from(name),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            method: METHOD_STORED,
            local_header_offset: 0,
            compressed_data: Vec::new(),
        });
    }

    /// Finalize and produce the ZIP file bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let mut output = Vec::new();

        // Write local file headers + data
        for entry in &mut self.entries {
            entry.local_header_offset = output.len() as u32;
            write_local_header(&mut output, entry);
            output.extend_from_slice(&entry.compressed_data);
        }

        // Write central directory
        let central_dir_offset = output.len() as u32;
        for entry in &self.entries {
            write_central_dir_entry(&mut output, entry);
        }
        let central_dir_size = output.len() as u32 - central_dir_offset;

        // Write end of central directory
        write_u32(&mut output, END_CENTRAL_DIR_SIG);
        write_u16(&mut output, 0); // disk number
        write_u16(&mut output, 0); // disk with central dir
        write_u16(&mut output, self.entries.len() as u16); // entries on this disk
        write_u16(&mut output, self.entries.len() as u16); // total entries
        write_u32(&mut output, central_dir_size);
        write_u32(&mut output, central_dir_offset);
        write_u16(&mut output, 0); // comment length

        output
    }
}

fn write_local_header(buf: &mut Vec<u8>, entry: &WriterEntry) {
    write_u32(buf, LOCAL_FILE_HEADER_SIG);
    write_u16(buf, 20); // version needed (2.0)
    write_u16(buf, 0);  // flags
    write_u16(buf, entry.method);
    write_u16(buf, 0);  // mod time
    write_u16(buf, 0);  // mod date
    write_u32(buf, entry.crc32);
    write_u32(buf, entry.compressed_size);
    write_u32(buf, entry.uncompressed_size);
    write_u16(buf, entry.name.len() as u16);
    write_u16(buf, 0);  // extra field length
    buf.extend_from_slice(entry.name.as_bytes());
}

fn write_central_dir_entry(buf: &mut Vec<u8>, entry: &WriterEntry) {
    write_u32(buf, CENTRAL_DIR_SIG);
    write_u16(buf, 20); // version made by
    write_u16(buf, 20); // version needed
    write_u16(buf, 0);  // flags
    write_u16(buf, entry.method);
    write_u16(buf, 0);  // mod time
    write_u16(buf, 0);  // mod date
    write_u32(buf, entry.crc32);
    write_u32(buf, entry.compressed_size);
    write_u32(buf, entry.uncompressed_size);
    write_u16(buf, entry.name.len() as u16);
    write_u16(buf, 0);  // extra field length
    write_u16(buf, 0);  // comment length
    write_u16(buf, 0);  // disk number start
    write_u16(buf, 0);  // internal file attributes
    write_u32(buf, 0);  // external file attributes
    write_u32(buf, entry.local_header_offset);
    buf.extend_from_slice(entry.name.as_bytes());
}
