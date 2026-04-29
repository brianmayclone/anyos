//! Git tree object parsing and building.
//!
//! Tree entries are stored as: "mode name\0<20-byte-sha1>" concatenated.

use crate::oid::Oid;
use alloc::string::String;
use alloc::vec::Vec;

/// A single entry in a tree object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: u32,
    pub name: String,
    pub oid: Oid,
}

impl TreeEntry {
    /// Check if this entry is a directory (subtree).
    pub fn is_tree(&self) -> bool {
        self.mode == 0o40000 || self.mode == 40000
    }

    /// Check if this entry is a regular file.
    pub fn is_blob(&self) -> bool {
        self.mode == 0o100644 || self.mode == 0o100755 || self.mode == 100644 || self.mode == 100755
    }

    /// Check if this entry is executable.
    pub fn is_executable(&self) -> bool {
        self.mode == 0o100755 || self.mode == 100755
    }

    /// Check if this entry is a symlink.
    pub fn is_symlink(&self) -> bool {
        self.mode == 0o120000 || self.mode == 120000
    }

    /// Check if this entry is a gitlink (submodule commit).
    pub fn is_gitlink(&self) -> bool {
        self.mode == 0o160000 || self.mode == 160000
    }

    /// Get the mode as a string for serialization.
    pub fn mode_str(&self) -> String {
        match self.mode {
            0o40000 | 40000 => String::from("40000"),
            0o100644 | 100644 => String::from("100644"),
            0o100755 | 100755 => String::from("100755"),
            0o120000 | 120000 => String::from("120000"),
            0o160000 | 160000 => String::from("160000"),
            _ => alloc::format!("{:o}", self.mode),
        }
    }
}

/// Parse a tree object's data into entries.
pub fn parse_tree(data: &[u8]) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        // Find the space between mode and name
        let space = match data[pos..].iter().position(|&b| b == b' ') {
            Some(i) => pos + i,
            None => break,
        };

        // Parse mode
        let mode_str = core::str::from_utf8(&data[pos..space]).unwrap_or("0");
        let mode = parse_octal(mode_str);

        // Find the null byte after name
        let null = match data[space + 1..].iter().position(|&b| b == 0) {
            Some(i) => space + 1 + i,
            None => break,
        };

        // Name is between space+1 and null
        let name = core::str::from_utf8(&data[space + 1..null]).unwrap_or("");

        // 20 bytes of SHA-1 follow the null
        if null + 1 + 20 > data.len() {
            break;
        }
        let mut sha = [0u8; 20];
        sha.copy_from_slice(&data[null + 1..null + 21]);

        entries.push(TreeEntry {
            mode,
            name: String::from(name),
            oid: Oid::from_bytes(sha),
        });

        pos = null + 21;
    }

    entries
}

/// Build a tree object's data from entries.
/// Entries are sorted by name (directories have "/" appended for sorting).
pub fn build_tree(entries: &mut [TreeEntry]) -> Vec<u8> {
    // Git sorts tree entries: directories sort as if they had "/" appended
    entries.sort_by(|a, b| {
        let a_name = if a.is_tree() {
            alloc::format!("{}/", a.name)
        } else {
            a.name.clone()
        };
        let b_name = if b.is_tree() {
            alloc::format!("{}/", b.name)
        } else {
            b.name.clone()
        };
        a_name.cmp(&b_name)
    });

    let mut buf = Vec::new();
    for entry in entries {
        // Mode (no leading zeros for trees: "40000", but "100644" for files)
        let mode_str = entry.mode_str();
        buf.extend_from_slice(mode_str.as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(entry.name.as_bytes());
        buf.push(0);
        buf.extend_from_slice(entry.oid.as_bytes());
    }
    buf
}

fn parse_octal(s: &str) -> u32 {
    let mut n = 0u32;
    for c in s.bytes() {
        if c >= b'0' && c <= b'7' {
            n = n * 8 + (c - b'0') as u32;
        }
    }
    n
}
