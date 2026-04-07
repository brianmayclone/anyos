// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! RamFS — in-memory filesystem with inode-based storage.
//!
//! Used as the writable upper layer for OverlayFS when booting from CD-ROM.
//! All data lives in kernel heap memory and is lost on reboot.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;

/// A single node (file or directory) in the RamFS.
struct RamNode {
    file_type: FileType,
    /// File content bytes (empty for directories).
    data: Vec<u8>,
    /// Directory children: (name, inode_index).
    children: Vec<(String, u32)>,
}

/// In-memory filesystem. Inode 0 is always the root directory.
pub struct RamFs {
    nodes: Vec<RamNode>,
}

impl RamFs {
    /// Create a new RamFS with an empty root directory.
    pub fn new() -> Self {
        let root = RamNode {
            file_type: FileType::Directory,
            data: Vec::new(),
            children: Vec::new(),
        };
        RamFs { nodes: vec![root] }
    }

    /// Allocate a new inode and return its index.
    fn alloc_node(&mut self, file_type: FileType) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(RamNode {
            file_type,
            data: Vec::new(),
            children: Vec::new(),
        });
        idx
    }

    /// Resolve a path to (inode, file_type, size). Path must start with "/".
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((0, FileType::Directory, 0));
        }

        let mut current = 0u32; // root inode
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for (idx, component) in components.iter().enumerate() {
            let node = self.nodes.get(current as usize).ok_or(FsError::NotFound)?;
            if node.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            let child = node.children.iter()
                .find(|(name, _)| name == component)
                .ok_or(FsError::NotFound)?;
            current = child.1;

            if idx == components.len() - 1 {
                let n = &self.nodes[current as usize];
                let size = n.data.len() as u32;
                return Ok((current, n.file_type, size));
            }
        }

        Err(FsError::NotFound)
    }

    /// Look up a single entry inside a directory inode.
    pub fn lookup_in_dir(&self, dir_inode: u32, name: &str) -> Result<(u32, FileType, u32), FsError> {
        let node = self.nodes.get(dir_inode as usize).ok_or(FsError::NotFound)?;
        if node.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let child = node.children.iter()
            .find(|(n, _)| n == name)
            .ok_or(FsError::NotFound)?;
        let child_node = &self.nodes[child.1 as usize];
        Ok((child.1, child_node.file_type, child_node.data.len() as u32))
    }

    /// Read bytes from a file at the given offset.
    pub fn read_file(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let node = self.nodes.get(inode as usize).ok_or(FsError::NotFound)?;
        if node.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        let data = &node.data;
        if offset as usize >= data.len() {
            return Ok(0); // EOF
        }
        let start = offset as usize;
        let available = data.len() - start;
        let to_copy = buf.len().min(available);
        buf[..to_copy].copy_from_slice(&data[start..start + to_copy]);
        Ok(to_copy)
    }

    /// Write bytes to a file at the given offset, extending the file if needed.
    /// Returns new size.
    pub fn write_file(&mut self, inode: u32, offset: u32, buf: &[u8]) -> Result<u32, FsError> {
        let node = self.nodes.get_mut(inode as usize).ok_or(FsError::NotFound)?;
        if node.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        let start = offset as usize;
        let end = start + buf.len();
        // Extend file if writing beyond current size
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        node.data[start..end].copy_from_slice(buf);
        Ok(node.data.len() as u32)
    }

    /// Create a file in a directory. Returns the new file's inode.
    pub fn create_file(&mut self, parent_inode: u32, name: &str) -> Result<u32, FsError> {
        // Check parent is a directory and name doesn't already exist
        {
            let parent = self.nodes.get(parent_inode as usize).ok_or(FsError::NotFound)?;
            if parent.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            if parent.children.iter().any(|(n, _)| n == name) {
                return Err(FsError::AlreadyExists);
            }
        }
        let new_inode = self.alloc_node(FileType::Regular);
        let parent = &mut self.nodes[parent_inode as usize];
        parent.children.push((String::from(name), new_inode));
        Ok(new_inode)
    }

    /// Create a subdirectory. Returns the new directory's inode.
    pub fn create_dir(&mut self, parent_inode: u32, name: &str) -> Result<u32, FsError> {
        {
            let parent = self.nodes.get(parent_inode as usize).ok_or(FsError::NotFound)?;
            if parent.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            if parent.children.iter().any(|(n, _)| n == name) {
                return Err(FsError::AlreadyExists);
            }
        }
        let new_inode = self.alloc_node(FileType::Directory);
        let parent = &mut self.nodes[parent_inode as usize];
        parent.children.push((String::from(name), new_inode));
        Ok(new_inode)
    }

    /// Ensure a full directory path exists, creating intermediate dirs as needed.
    /// Returns the inode of the deepest directory.
    pub fn ensure_dir_path(&mut self, path: &str) -> Result<u32, FsError> {
        let path = path.trim_start_matches('/').trim_end_matches('/');
        if path.is_empty() {
            return Ok(0); // root
        }
        let mut current = 0u32;
        for component in path.split('/') {
            if component.is_empty() { continue; }
            let node = &self.nodes[current as usize];
            if let Some(child) = node.children.iter().find(|(n, _)| n == component) {
                current = child.1;
            } else {
                let new_inode = self.alloc_node(FileType::Directory);
                self.nodes[current as usize].children.push((String::from(component), new_inode));
                current = new_inode;
            }
        }
        Ok(current)
    }

    /// Store a complete file (create path + write data). Used for copy-on-write.
    pub fn store_file(&mut self, path: &str, data: &[u8]) -> Result<u32, FsError> {
        let (parent_path, filename) = split_path(path)?;
        let parent_inode = self.ensure_dir_path(parent_path)?;

        // If file already exists in this dir, overwrite its data
        {
            let parent = &self.nodes[parent_inode as usize];
            if let Some(existing) = parent.children.iter().find(|(n, _)| n == filename) {
                let inode = existing.1;
                self.nodes[inode as usize].data = Vec::from(data);
                return Ok(inode);
            }
        }

        let new_inode = self.alloc_node(FileType::Regular);
        self.nodes[new_inode as usize].data = Vec::from(data);
        self.nodes[parent_inode as usize].children.push((String::from(filename), new_inode));
        Ok(new_inode)
    }

    /// List directory entries.
    pub fn read_dir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError> {
        let node = self.nodes.get(inode as usize).ok_or(FsError::NotFound)?;
        if node.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = Vec::new();
        for (name, child_idx) in &node.children {
            let child = &self.nodes[*child_idx as usize];
            entries.push(DirEntry {
                name: name.clone(),
                file_type: child.file_type,
                size: child.data.len() as u32,
                is_symlink: false,
                uid: 0,
                gid: 0,
                mode: 0xFFF,
            });
        }
        Ok(entries)
    }

    /// Delete a file or directory by name from a parent directory.
    pub fn delete(&mut self, parent_inode: u32, name: &str) -> Result<(), FsError> {
        let parent = self.nodes.get_mut(parent_inode as usize).ok_or(FsError::NotFound)?;
        if parent.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let pos = parent.children.iter().position(|(n, _)| n == name)
            .ok_or(FsError::NotFound)?;
        // Note: we don't reclaim the inode slot (it stays allocated but unreachable).
        // For a RAM overlay that lives only until reboot, this is acceptable.
        parent.children.remove(pos);
        Ok(())
    }

    /// Truncate a file to zero length.
    pub fn truncate_file(&mut self, inode: u32) -> Result<(), FsError> {
        let node = self.nodes.get_mut(inode as usize).ok_or(FsError::NotFound)?;
        if node.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        node.data.clear();
        Ok(())
    }

    /// Get file size.
    pub fn file_size(&self, inode: u32) -> Result<u32, FsError> {
        let node = self.nodes.get(inode as usize).ok_or(FsError::NotFound)?;
        Ok(node.data.len() as u32)
    }

    /// Check if a path exists.
    pub fn exists(&self, path: &str) -> bool {
        self.lookup(path).is_ok()
    }
}

/// Split "/System/hello.txt" into ("System", "hello.txt").
fn split_path(path: &str) -> Result<(&str, &str), FsError> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(pos) => Ok((&path[..pos], &path[pos + 1..])),
        None => Ok(("", path)),
    }
}
