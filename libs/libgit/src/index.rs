//! Git index file (.git/index) parsing and writing.
//!
//! The index (staging area) tracks files to be included in the next commit.
//! Format: https://git-scm.com/docs/index-format

use crate::oid::Oid;
use crate::repo::{Error, Repository, Result};
use crate::sha1;
use alloc::string::String;
use alloc::vec::Vec;

/// An entry in the git index.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub ctime_s: u32,
    pub ctime_ns: u32,
    pub mtime_s: u32,
    pub mtime_ns: u32,
    pub dev: u32,
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u32,
    pub oid: Oid,
    pub flags: u16,
    pub name: String,
}

impl IndexEntry {
    /// Create a new index entry from a file path and its blob OID.
    pub fn new(name: &str, oid: Oid, mode: u32, size: u32) -> Self {
        let name_len = name.len().min(0xFFF) as u16;
        IndexEntry {
            ctime_s: 0,
            ctime_ns: 0,
            mtime_s: 0,
            mtime_ns: 0,
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            size,
            oid,
            flags: name_len,
            name: String::from(name),
        }
    }
}

/// The git index.
#[derive(Debug)]
pub struct Index {
    pub entries: Vec<IndexEntry>,
}

impl Index {
    /// Create an empty index.
    pub fn new() -> Self {
        Index {
            entries: Vec::new(),
        }
    }

    /// Read the index from .git/index.
    pub fn read(repo: &Repository) -> Result<Self> {
        let path = repo.gitdir.join("index");
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return Ok(Index::new()), // No index yet
        };

        Self::parse(&data)
    }

    /// Parse index data.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 12 {
            return Err(Error::InvalidIndex);
        }

        // Header: "DIRC" + version (u32) + entry count (u32)
        if &data[0..4] != b"DIRC" {
            return Err(Error::InvalidIndex);
        }
        let _version = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let count = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;

        let mut entries = Vec::with_capacity(count);
        let mut pos = 12;
        let entries_end = data.len().saturating_sub(20);

        for _ in 0..count {
            let entry_start = pos;
            if entry_start + 62 > entries_end {
                break;
            }

            let ctime_s = read_u32(&data[entry_start..]);
            let ctime_ns = read_u32(&data[entry_start + 4..]);
            let mtime_s = read_u32(&data[entry_start + 8..]);
            let mtime_ns = read_u32(&data[entry_start + 12..]);
            let dev = read_u32(&data[entry_start + 16..]);
            let ino = read_u32(&data[entry_start + 20..]);
            let mode = read_u32(&data[entry_start + 24..]);
            let uid = read_u32(&data[entry_start + 28..]);
            let gid = read_u32(&data[entry_start + 32..]);
            let size = read_u32(&data[entry_start + 36..]);

            let mut sha = [0u8; 20];
            sha.copy_from_slice(&data[entry_start + 40..entry_start + 60]);
            let oid = Oid::from_bytes(sha);

            let flags = u16::from_be_bytes([data[entry_start + 60], data[entry_start + 61]]);
            let name_len = (flags & 0xFFF) as usize;

            let name_start = entry_start + 62;
            let name_end = if name_len < 0xFFF {
                let end = name_start + name_len;
                if end > entries_end {
                    return Err(Error::InvalidIndex);
                }
                end
            } else {
                // Name is longer than 0xFFF, find null terminator
                data[name_start..entries_end]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|i| name_start + i)
                    .ok_or(Error::InvalidIndex)?
            };

            let name = core::str::from_utf8(&data[name_start..name_end])
                .unwrap_or("")
                .into();

            entries.push(IndexEntry {
                ctime_s,
                ctime_ns,
                mtime_s,
                mtime_ns,
                dev,
                ino,
                mode,
                uid,
                gid,
                size,
                oid,
                flags,
                name,
            });

            let entry_len = 62 + (name_end - name_start) + 1;
            pos = entry_start + ((entry_len + 7) & !7);
            if pos > entries_end {
                return Err(Error::InvalidIndex);
            }
        }

        Ok(Index { entries })
    }

    /// Write the index to .git/index.
    pub fn write(&self, repo: &Repository) -> Result<()> {
        let data = self.serialize();
        std::fs::write(&repo.gitdir.join("index"), &data).map_err(|_| Error::IoError)
    }

    /// Serialize the index to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Header
        buf.extend_from_slice(b"DIRC");
        buf.extend_from_slice(&2u32.to_be_bytes()); // version 2
        buf.extend_from_slice(&(self.entries.len() as u32).to_be_bytes());

        for entry in &self.entries {
            buf.extend_from_slice(&entry.ctime_s.to_be_bytes());
            buf.extend_from_slice(&entry.ctime_ns.to_be_bytes());
            buf.extend_from_slice(&entry.mtime_s.to_be_bytes());
            buf.extend_from_slice(&entry.mtime_ns.to_be_bytes());
            buf.extend_from_slice(&entry.dev.to_be_bytes());
            buf.extend_from_slice(&entry.ino.to_be_bytes());
            buf.extend_from_slice(&entry.mode.to_be_bytes());
            buf.extend_from_slice(&entry.uid.to_be_bytes());
            buf.extend_from_slice(&entry.gid.to_be_bytes());
            buf.extend_from_slice(&entry.size.to_be_bytes());
            buf.extend_from_slice(entry.oid.as_bytes());

            let name_bytes = entry.name.as_bytes();
            let name_len = name_bytes.len().min(0xFFF) as u16;
            buf.extend_from_slice(&name_len.to_be_bytes());
            buf.extend_from_slice(name_bytes);
            buf.push(0); // null terminator

            // Pad to 8-byte boundary (entry starts at offset 12 in file)
            let entry_size = 62 + name_bytes.len() + 1;
            let padding = (8 - (entry_size % 8)) % 8;
            for _ in 0..padding {
                buf.push(0);
            }
        }

        // Compute and append SHA-1 checksum of everything
        let checksum = sha1::hash(&buf);
        buf.extend_from_slice(&checksum);

        buf
    }

    /// Add or update an entry in the index.
    pub fn add(&mut self, entry: IndexEntry) {
        // Remove existing entry with same name
        self.entries.retain(|e| e.name != entry.name);
        self.entries.push(entry);
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Remove an entry from the index by name.
    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|e| e.name != name);
    }

    /// Find an entry by name.
    pub fn find(&self, name: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

fn read_u32(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}
