//! POSIX ustar tar archive reader/writer.
//!
//! Supports reading and writing tar archives with ustar format headers.
//! Transparently handles `.tar.gz` via the `gzip` module.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// ── Constants ───────────────────────────────────────────────────────────────

const BLOCK_SIZE: usize = 512;
const USTAR_MAGIC: &[u8; 6] = b"ustar\0";

// Header field offsets
const OFF_NAME: usize = 0;
const OFF_MODE: usize = 100;
const OFF_SIZE: usize = 124;
const OFF_CHKSUM: usize = 148;
const OFF_TYPEFLAG: usize = 156;
const OFF_MAGIC: usize = 257;
const OFF_PREFIX: usize = 345;

// ── Tar Entry ───────────────────────────────────────────────────────────────

/// A single entry in a tar archive.
pub struct TarEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    /// Byte offset of the file data in the raw tar data.
    data_offset: usize,
}

// ── Tar Reader ──────────────────────────────────────────────────────────────

/// Reader for tar (and tar.gz) archives.
pub struct TarReader {
    pub entries: Vec<TarEntry>,
    data: Vec<u8>,
}

impl TarReader {
    /// Parse a tar archive from raw bytes.
    /// Automatically detects and decompresses gzip-wrapped archives.
    pub fn parse(data: Vec<u8>) -> Option<TarReader> {
        // Transparent .tar.gz support
        let tar_data = if crate::gzip::is_gzip(&data) {
            crate::gzip::gzip_decompress(&data)?
        } else {
            data
        };

        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + BLOCK_SIZE <= tar_data.len() {
            let header = &tar_data[pos..pos + BLOCK_SIZE];

            // Two consecutive zero blocks mark end of archive
            if header.iter().all(|&b| b == 0) {
                break;
            }

            // Validate checksum
            if !verify_checksum(header) {
                break;
            }

            // Parse entry
            let name = parse_name(header);
            let size = parse_octal(&header[OFF_SIZE..OFF_SIZE + 12]);
            let typeflag = header[OFF_TYPEFLAG];
            let is_dir = typeflag == b'5' || name.ends_with('/');

            let data_offset = pos + BLOCK_SIZE;

            entries.push(TarEntry {
                name,
                size,
                is_dir,
                data_offset,
            });

            // Advance past header + data blocks (data padded to 512-byte boundary)
            let data_blocks = (size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE;
            pos = data_offset + data_blocks * BLOCK_SIZE;
        }

        Some(TarReader { entries, data: tar_data })
    }

    /// Number of entries in the archive.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Extract file data for an entry. Returns None for directories.
    pub fn extract(&self, index: usize) -> Option<Vec<u8>> {
        let entry = self.entries.get(index)?;
        if entry.is_dir || entry.size == 0 {
            return Some(Vec::new());
        }
        let end = entry.data_offset + entry.size as usize;
        if end > self.data.len() {
            return None;
        }
        Some(self.data[entry.data_offset..end].to_vec())
    }
}

// ── Tar Writer ──────────────────────────────────────────────────────────────

/// Writer for creating tar archives.
pub struct TarWriter {
    output: Vec<u8>,
}

impl TarWriter {
    pub fn new() -> TarWriter {
        TarWriter { output: Vec::new() }
    }

    /// Add a file with data.
    pub fn add_file(&mut self, name: &str, data: &[u8]) {
        let mut header = [0u8; BLOCK_SIZE];
        write_name(&mut header, name);
        write_octal(&mut header[OFF_MODE..OFF_MODE + 8], 0o644, 7);
        write_octal(&mut header[OFF_SIZE..OFF_SIZE + 12], data.len() as u64, 11);
        header[OFF_TYPEFLAG] = b'0'; // regular file
        write_ustar_magic(&mut header);
        write_checksum(&mut header);

        self.output.extend_from_slice(&header);
        self.output.extend_from_slice(data);

        // Pad to 512-byte boundary
        let remainder = data.len() % BLOCK_SIZE;
        if remainder != 0 {
            let padding = BLOCK_SIZE - remainder;
            self.output.extend(core::iter::repeat(0u8).take(padding));
        }
    }

    /// Add a directory entry.
    pub fn add_directory(&mut self, name: &str) {
        let mut header = [0u8; BLOCK_SIZE];
        // Ensure directory name ends with '/'
        let dir_name = if name.ends_with('/') {
            String::from(name)
        } else {
            let mut s = String::from(name);
            s.push('/');
            s
        };
        write_name(&mut header, &dir_name);
        write_octal(&mut header[OFF_MODE..OFF_MODE + 8], 0o755, 7);
        write_octal(&mut header[OFF_SIZE..OFF_SIZE + 12], 0, 11);
        header[OFF_TYPEFLAG] = b'5'; // directory
        write_ustar_magic(&mut header);
        write_checksum(&mut header);

        self.output.extend_from_slice(&header);
    }

    /// Finalize the archive and return raw tar bytes.
    /// Appends two zero blocks as end-of-archive marker.
    pub fn finish(mut self) -> Vec<u8> {
        // End-of-archive: two 512-byte zero blocks
        self.output.extend_from_slice(&[0u8; BLOCK_SIZE * 2]);
        self.output
    }
}

// ── Helper Functions ────────────────────────────────────────────────────────

/// Parse a null-terminated string from a fixed-size field.
fn parse_str(field: &[u8]) -> &str {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).unwrap_or("")
}

/// Parse the full name from header (prefix + name fields).
fn parse_name(header: &[u8]) -> String {
    let prefix = parse_str(&header[OFF_PREFIX..OFF_PREFIX + 155]);
    let name = parse_str(&header[OFF_NAME..OFF_NAME + 100]);
    if prefix.is_empty() {
        String::from(name)
    } else {
        let mut full = String::from(prefix);
        full.push('/');
        full.push_str(name);
        full
    }
}

/// Parse an octal ASCII number from a tar header field.
fn parse_octal(field: &[u8]) -> u64 {
    // Handle GNU binary extension (high bit set in first byte)
    if !field.is_empty() && field[0] & 0x80 != 0 {
        // Binary-encoded size (big-endian, skip first byte)
        let mut val = 0u64;
        for &b in &field[1..] {
            val = (val << 8) | b as u64;
        }
        return val;
    }

    let s = parse_str(field).trim();
    let mut val = 0u64;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'7' {
            val = val * 8 + (b - b'0') as u64;
        }
    }
    val
}

/// Write a name into the header, splitting into prefix+name if > 100 chars.
fn write_name(header: &mut [u8; BLOCK_SIZE], name: &str) {
    let bytes = name.as_bytes();
    if bytes.len() <= 100 {
        header[OFF_NAME..OFF_NAME + bytes.len()].copy_from_slice(bytes);
    } else {
        // Split at last '/' before position 100
        let split = bytes[..100].iter().rposition(|&b| b == b'/').unwrap_or(100);
        let prefix_bytes = &bytes[..split];
        let name_bytes = &bytes[split + 1..]; // skip the '/'
        let plen = prefix_bytes.len().min(155);
        let nlen = name_bytes.len().min(100);
        header[OFF_PREFIX..OFF_PREFIX + plen].copy_from_slice(&prefix_bytes[..plen]);
        header[OFF_NAME..OFF_NAME + nlen].copy_from_slice(&name_bytes[..nlen]);
    }
}

/// Write an octal ASCII number into a field.
fn write_octal(field: &mut [u8], value: u64, width: usize) {
    // Format as octal with leading zeros, null-terminated
    let mut buf = [b'0'; 12];
    let mut v = value;
    let mut i = width;
    while i > 0 {
        i -= 1;
        buf[i] = b'0' + (v & 7) as u8;
        v >>= 3;
    }
    field[..width].copy_from_slice(&buf[..width]);
    if width < field.len() {
        field[width] = 0;
    }
}

/// Write ustar magic and version into header.
fn write_ustar_magic(header: &mut [u8; BLOCK_SIZE]) {
    header[OFF_MAGIC..OFF_MAGIC + 6].copy_from_slice(USTAR_MAGIC);
    header[OFF_MAGIC + 6] = b'0'; // version[0]
    header[OFF_MAGIC + 7] = b'0'; // version[1]
}

/// Compute and write the tar header checksum.
fn write_checksum(header: &mut [u8; BLOCK_SIZE]) {
    // Set checksum field to spaces for calculation
    header[OFF_CHKSUM..OFF_CHKSUM + 8].copy_from_slice(b"        ");

    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    write_octal(&mut header[OFF_CHKSUM..OFF_CHKSUM + 8], sum as u64, 6);
    header[OFF_CHKSUM + 6] = 0;
    header[OFF_CHKSUM + 7] = b' ';
}

/// Verify the checksum of a tar header block.
fn verify_checksum(header: &[u8]) -> bool {
    let stored = parse_octal(&header[OFF_CHKSUM..OFF_CHKSUM + 8]) as u32;

    // Compute checksum treating checksum field as spaces
    let mut sum: u32 = 0;
    for (i, &b) in header.iter().enumerate() {
        if i >= OFF_CHKSUM && i < OFF_CHKSUM + 8 {
            sum += b' ' as u32;
        } else {
            sum += b as u32;
        }
    }

    sum == stored
}
