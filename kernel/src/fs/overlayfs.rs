//! OverlayFS — union mount with writable RAM layer over read-only ISO 9660.
//!
//! When anyOS boots from a CD-ROM (ISO 9660 as root), the OverlayFS provides
//! transparent read-write access:
//!   - **Lower layer**: ISO 9660 (read-only, unchanged)
//!   - **Upper layer**: RamFS (in-memory, writable)
//!
//! Lookup order: whiteouts → upper (RamFS) → lower (ISO 9660).
//! All writes, creates, and mkdirs go to the upper layer.
//! Deleting an ISO file creates a "whiteout" entry so it no longer appears.
//!
//! Inode encoding for open files:
//!   - Bit 31 set (`OVERLAY_RAM_BIT`): data lives in RamFS (lower 31 bits = ram inode)
//!   - Bit 31 clear: data lives in ISO 9660 (inode = LBA as from ISO lookup)

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::fs::file::{DirEntry, FileType};
use crate::fs::iso9660::Iso9660Fs;
use crate::fs::ramfs::RamFs;
use crate::fs::vfs::FsError;

/// Bit flag: when set in an overlay inode, the data lives in RamFS.
pub const OVERLAY_RAM_BIT: u32 = 0x8000_0000;

pub struct OverlayFs {
    /// Writable upper layer (in-memory).
    pub ram: RamFs,
    /// Set of absolute paths that have been deleted (whiteouts).
    /// A whiteout hides the corresponding ISO file from lookups.
    whiteouts: BTreeSet<String>,
}

impl OverlayFs {
    pub fn new() -> Self {
        OverlayFs {
            ram: RamFs::new(),
            whiteouts: BTreeSet::new(),
        }
    }

    /// Check if a path has been whited out (deleted from overlay).
    fn is_whiteout(&self, path: &str) -> bool {
        self.whiteouts.contains(path)
    }

    /// Normalize path for whiteout checks: ensure leading "/" and no trailing "/".
    fn norm(path: &str) -> String {
        let mut p = if path.starts_with('/') {
            String::from(path)
        } else {
            let mut s = String::from("/");
            s.push_str(path);
            s
        };
        while p.len() > 1 && p.ends_with('/') {
            p.pop();
        }
        p
    }

    /// Look up a path. Checks whiteouts, then RamFS, then ISO 9660.
    /// Returns (overlay_inode, file_type, size).
    pub fn lookup(&self, iso: &Iso9660Fs, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let norm = Self::norm(path);

        // Root directory always exists
        if norm == "/" {
            return Ok((OVERLAY_RAM_BIT, FileType::Directory, 0));
        }

        // Whiteout check
        if self.is_whiteout(&norm) {
            return Err(FsError::NotFound);
        }

        // Check upper (RamFS) first
        if let Ok((inode, ft, sz)) = self.ram.lookup(&norm) {
            return Ok((inode | OVERLAY_RAM_BIT, ft, sz));
        }

        // Check lower (ISO 9660)
        let (inode, ft, sz) = iso.lookup(&norm)?;
        Ok((inode, ft, sz))
    }

    /// Read bytes from a file identified by an overlay inode.
    pub fn read_file(
        &self,
        iso: &Iso9660Fs,
        inode: u32,
        offset: u32,
        buf: &mut [u8],
        file_size: u32,
    ) -> Result<usize, FsError> {
        if inode & OVERLAY_RAM_BIT != 0 {
            // Upper layer (RamFS)
            let ram_inode = inode & !OVERLAY_RAM_BIT;
            self.ram.read_file(ram_inode, offset, buf)
        } else {
            // Lower layer (ISO 9660)
            iso.read_file(inode, offset, buf, file_size)
        }
    }

    /// Write bytes to a file. If the file is on the ISO layer, perform copy-on-write
    /// to RamFS first. Returns (new_overlay_inode, new_size).
    pub fn write_file(
        &mut self,
        iso: &Iso9660Fs,
        inode: u32,
        offset: u32,
        buf: &[u8],
        file_size: u32,
        path: &str,
    ) -> Result<(u32, u32), FsError> {
        if inode & OVERLAY_RAM_BIT != 0 {
            // Already in RamFS — write directly
            let ram_inode = inode & !OVERLAY_RAM_BIT;
            let new_size = self.ram.write_file(ram_inode, offset, buf)?;
            Ok((inode, new_size))
        } else {
            // Copy-on-write: read entire file from ISO into RamFS, then write
            let mut iso_data = Vec::new();
            if file_size > 0 {
                iso_data.resize(file_size as usize, 0u8);
                iso.read_file(inode, 0, &mut iso_data, file_size)?;
            }
            let ram_inode = self.ram.store_file(path, &iso_data)?;
            let new_size = self.ram.write_file(ram_inode, offset, buf)?;
            Ok((ram_inode | OVERLAY_RAM_BIT, new_size))
        }
    }

    /// Create a new file. Always in the upper layer.
    /// Returns the overlay inode.
    pub fn create_file(&mut self, path: &str) -> Result<u32, FsError> {
        let norm = Self::norm(path);
        // Remove any whiteout for this path
        self.whiteouts.remove(&norm);

        let (parent_path, filename) = split_overlay_path(&norm)?;
        let parent_inode = self.ram.ensure_dir_path(parent_path)?;
        let ram_inode = self.ram.create_file(parent_inode, filename)?;
        Ok(ram_inode | OVERLAY_RAM_BIT)
    }

    /// Create a directory. Always in the upper layer.
    pub fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        let norm = Self::norm(path);
        self.whiteouts.remove(&norm);

        let (parent_path, dirname) = split_overlay_path(&norm)?;
        let parent_inode = self.ram.ensure_dir_path(parent_path)?;
        self.ram.create_dir(parent_inode, dirname)?;
        Ok(())
    }

    /// Delete a file or directory. If it exists in RamFS, remove it there.
    /// If it exists in ISO, add a whiteout.
    pub fn delete(&mut self, iso: &Iso9660Fs, path: &str) -> Result<(), FsError> {
        let norm = Self::norm(path);

        // Try to delete from RamFS
        if let Ok((_, _, _)) = self.ram.lookup(&norm) {
            let (parent_path, filename) = split_overlay_path(&norm)?;
            if let Ok((parent_inode, _, _)) = self.ram.lookup(&format!("/{}", parent_path)) {
                let _ = self.ram.delete(parent_inode, filename);
            }
        }

        // If it exists in ISO, add whiteout so it stays hidden
        if iso.lookup(&norm).is_ok() {
            self.whiteouts.insert(norm);
        }

        Ok(())
    }

    /// Rename a file/directory within the overlay.
    pub fn rename(&mut self, iso: &Iso9660Fs, old_path: &str, new_path: &str) -> Result<(), FsError> {
        let old_norm = Self::norm(old_path);
        let new_norm = Self::norm(new_path);

        // Look up the source
        let (inode, _ft, sz) = self.lookup(iso, &old_norm)?;

        if inode & OVERLAY_RAM_BIT == 0 {
            // Source is on ISO — copy to RAM first
            let mut data = Vec::new();
            if sz > 0 {
                data.resize(sz as usize, 0u8);
                iso.read_file(inode, 0, &mut data, sz)?;
            }
            self.ram.store_file(&new_norm, &data)?;
        } else {
            // Source is in RAM — read data, store at new path
            let ram_inode = inode & !OVERLAY_RAM_BIT;
            let size = self.ram.file_size(ram_inode)?;
            let mut data = Vec::new();
            if size > 0 {
                data.resize(size as usize, 0u8);
                self.ram.read_file(ram_inode, 0, &mut data)?;
            }
            self.ram.store_file(&new_norm, &data)?;
            // Delete old from RAM
            let (parent_path, filename) = split_overlay_path(&old_norm)?;
            if let Ok((parent_inode, _, _)) = self.ram.lookup(&format!("/{}", parent_path)) {
                let _ = self.ram.delete(parent_inode, filename);
            }
        }

        // Whiteout the old path (hides ISO original)
        if iso.lookup(&old_norm).is_ok() {
            self.whiteouts.insert(old_norm);
        }
        // Remove whiteout for new path if it existed
        self.whiteouts.remove(&new_norm);

        Ok(())
    }

    /// Read merged directory listing (upper + lower, minus whiteouts).
    pub fn read_dir(&self, iso: &Iso9660Fs, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let norm = Self::norm(path);
        let dir_prefix = if norm == "/" {
            String::from("/")
        } else {
            let mut s = norm.clone();
            s.push('/');
            s
        };

        let mut entries = Vec::new();
        let mut seen_names: Vec<String> = Vec::new();

        // Upper layer entries first (they take priority)
        if let Ok((ram_inode, FileType::Directory, _)) = self.ram.lookup(&norm) {
            let ram_inode_raw = ram_inode & !OVERLAY_RAM_BIT;
            if let Ok(ram_entries) = self.ram.read_dir(ram_inode_raw) {
                for e in ram_entries {
                    // Check whiteout for the child path
                    let child_path = if norm == "/" {
                        let mut s = String::from("/");
                        s.push_str(&e.name);
                        s
                    } else {
                        let mut s = norm.clone();
                        s.push('/');
                        s.push_str(&e.name);
                        s
                    };
                    if !self.is_whiteout(&child_path) {
                        seen_names.push(e.name.clone());
                        entries.push(e);
                    }
                }
            }
        }

        // Lower layer (ISO) — add entries not already seen and not whiteout'd
        if let Ok((iso_inode, FileType::Directory, iso_size)) = iso.lookup(&norm) {
            if let Ok(iso_entries) = iso.read_dir(iso_inode, iso_size) {
                for e in iso_entries {
                    // Skip if already in upper layer
                    if seen_names.iter().any(|n| *n == e.name) {
                        continue;
                    }
                    // Skip if whiteout'd
                    let child_path = if norm == "/" {
                        let mut s = String::from("/");
                        s.push_str(&e.name);
                        s
                    } else {
                        let mut s = norm.clone();
                        s.push('/');
                        s.push_str(&e.name);
                        s
                    };
                    if !self.is_whiteout(&child_path) {
                        entries.push(e);
                    }
                }
            }
        }

        Ok(entries)
    }

    /// Stat a path through the overlay.
    pub fn stat(&self, iso: &Iso9660Fs, path: &str) -> Result<(FileType, u32), FsError> {
        let (_, ft, sz) = self.lookup(iso, path)?;
        Ok((ft, sz))
    }

    /// Truncate a file. If on ISO, copy-on-write an empty file.
    pub fn truncate(&mut self, iso: &Iso9660Fs, path: &str) -> Result<(), FsError> {
        let norm = Self::norm(path);
        let (inode, _ft, _sz) = self.lookup(iso, &norm)?;

        if inode & OVERLAY_RAM_BIT != 0 {
            let ram_inode = inode & !OVERLAY_RAM_BIT;
            self.ram.truncate_file(ram_inode)
        } else {
            // File on ISO — store empty file in RAM (effectively truncating)
            self.ram.store_file(&norm, &[])?;
            Ok(())
        }
    }
}

/// Split "/System/hello.txt" into ("System", "hello.txt").
fn split_overlay_path(path: &str) -> Result<(&str, &str), FsError> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(pos) => Ok((&path[..pos], &path[pos + 1..])),
        None => Ok(("", path)),
    }
}
