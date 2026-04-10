//! Git repository operations.
//!
//! Handles the .git directory structure, object storage (loose objects),
//! and high-level operations like init, read/write objects.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use std::path::{Path, PathBuf};
use std::io::{Read, Write, Seek, SeekFrom};
use crate::oid::Oid;
use crate::object::{Object, ObjectType};
use crate::inflate;
use crate::deflate;

/// A git repository handle.
pub struct Repository {
    /// Path to the working directory.
    pub workdir: PathBuf,
    /// Path to the .git directory.
    pub gitdir: PathBuf,
}

/// Errors from repository operations.
#[derive(Debug)]
pub enum Error {
    NotFound,
    InvalidObject,
    IoError,
    InvalidRef,
    InvalidIndex,
    MergeConflict,
    Other(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotFound => f.write_str("not found"),
            Error::InvalidObject => f.write_str("invalid object"),
            Error::IoError => f.write_str("I/O error"),
            Error::InvalidRef => f.write_str("invalid ref"),
            Error::InvalidIndex => f.write_str("invalid index"),
            Error::MergeConflict => f.write_str("merge conflict"),
            Error::Other(s) => write!(f, "{}", s),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

impl Repository {
    /// Initialize a new git repository at the given path.
    pub fn init(path: &str) -> Result<Self> {
        let workdir = PathBuf::from(path);
        let gitdir = workdir.join(".git");

        // Create .git directory structure
        mkdir_p(&gitdir)?;
        mkdir_p(&gitdir.join("objects"))?;
        mkdir_p(&gitdir.join("objects/info"))?;
        mkdir_p(&gitdir.join("objects/pack"))?;
        mkdir_p(&gitdir.join("refs"))?;
        mkdir_p(&gitdir.join("refs/heads"))?;
        mkdir_p(&gitdir.join("refs/tags"))?;

        // Write HEAD
        write_file(
            &gitdir.join("HEAD"),
            b"ref: refs/heads/main\n",
        )?;

        // Write config
        write_file(
            &gitdir.join("config"),
            b"[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n",
        )?;

        // Write description
        write_file(
            &gitdir.join("description"),
            b"Unnamed repository\n",
        )?;

        Ok(Repository { workdir, gitdir })
    }

    /// Open an existing repository by looking for .git in the given path
    /// or any parent directory.
    pub fn open(path: &str) -> Result<Self> {
        let mut current = PathBuf::from(path);
        loop {
            let gitdir = current.join(".git");
            if gitdir.is_dir() {
                return Ok(Repository {
                    workdir: current,
                    gitdir,
                });
            }
            if !current.pop() {
                return Err(Error::NotFound);
            }
        }
    }

    /// Read a loose object by its OID.
    pub fn read_object(&self, oid: &Oid) -> Result<Object> {
        let hex = oid.to_hex();
        let path = self.gitdir
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..]);

        let compressed = read_file(&path)?;
        // Skip zlib header (2 bytes: CMF + FLG) if present
        let deflate_data = if compressed.len() >= 2 && compressed[0] == 0x78 {
            &compressed[2..]
        } else {
            &compressed
        };
        let raw = inflate::inflate(deflate_data).ok_or(Error::InvalidObject)?;
        Object::deserialize(&raw).ok_or(Error::InvalidObject)
    }

    /// Write an object to the loose object store. Returns its OID.
    pub fn write_object(&self, obj: &Object) -> Result<Oid> {
        let oid = obj.id();
        let hex = oid.to_hex();

        let dir = self.gitdir.join("objects").join(&hex[..2]);
        mkdir_p(&dir)?;

        let path = dir.join(&hex[2..]);
        // Don't overwrite if already exists
        if path.exists() {
            return Ok(oid);
        }

        let raw = obj.serialize();
        let compressed = deflate::deflate(&raw);

        // Write with zlib header (0x78, 0x01)
        let mut zlib_data = Vec::with_capacity(2 + compressed.len());
        zlib_data.push(0x78);
        zlib_data.push(0x01);
        zlib_data.extend_from_slice(&compressed);

        write_file(&path, &zlib_data)?;
        Ok(oid)
    }

    /// Hash an object without writing it to the store.
    pub fn hash_object(&self, obj: &Object) -> Oid {
        obj.id()
    }

    /// Get HEAD reference (resolves symbolic refs).
    pub fn head(&self) -> Result<Oid> {
        crate::refs::resolve_ref(self, "HEAD")
    }

    /// Get the current branch name (None if detached HEAD).
    pub fn current_branch(&self) -> Result<Option<String>> {
        let head_content = read_file_string(&self.gitdir.join("HEAD"))?;
        let trimmed = head_content.trim();
        if let Some(refname) = trimmed.strip_prefix("ref: ") {
            if let Some(branch) = refname.strip_prefix("refs/heads/") {
                Ok(Some(String::from(branch)))
            } else {
                Ok(Some(String::from(refname)))
            }
        } else {
            Ok(None) // Detached HEAD
        }
    }

    /// Get the path to a file relative to the working directory.
    pub fn workdir_path(&self, relative: &str) -> PathBuf {
        self.workdir.join(relative)
    }

    /// List all branches.
    pub fn branches(&self) -> Result<Vec<String>> {
        let heads_dir = self.gitdir.join("refs/heads");
        let mut branches = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&heads_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if let Some(name) = entry.file_name().to_str() {
                        branches.push(String::from(name));
                    }
                }
            }
        }
        Ok(branches)
    }

    /// Read an object, trying loose objects first, then pack files.
    pub fn read_object_any(&self, oid: &Oid) -> Result<Object> {
        // Try loose object first
        if let Ok(obj) = self.read_object(oid) {
            return Ok(obj);
        }

        // Try pack files
        let pack_dir = self.gitdir.join("objects/pack");
        if let Ok(entries) = std::fs::read_dir(&pack_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let name = match entry.file_name().to_str() {
                        Some(n) => String::from(n),
                        None => continue,
                    };
                    if name.ends_with(".pack") {
                        let pack_path = pack_dir.join(&*name);
                        if let Ok(pack_data) = read_file(&pack_path) {
                            if let Some(pack) = crate::pack::parse_pack(&pack_data) {
                                if let Some(entry) = pack.entries.iter().find(|e| e.oid == *oid) {
                                    return Ok(Object {
                                        obj_type: entry.obj_type,
                                        data: entry.data.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(Error::NotFound)
    }

    /// Store a pack file and its objects.
    pub fn store_pack(&self, pack_data: &[u8]) -> Result<u32> {
        if crate::pack::verbose() {
            anyos_std::println!("[store_pack] input size={}", pack_data.len());
            if pack_data.len() >= 12 {
                anyos_std::println!("[store_pack] header: {:?}", &pack_data[..12]);
            }
            if pack_data.len() < 12 {
                anyos_std::println!("[store_pack] ERROR: pack data too small ({} bytes)", pack_data.len());
                // Dump first bytes for debugging
                let show = core::cmp::min(pack_data.len(), 64);
                for i in 0..show {
                    anyos_std::print!("{:02x} ", pack_data[i]);
                    if (i + 1) % 16 == 0 { anyos_std::println!(); }
                }
                anyos_std::println!();
            }
        }
        let pack = crate::pack::parse_pack(pack_data)
            .ok_or(Error::InvalidObject)?;

        let mut count = 0u32;
        for entry in &pack.entries {
            let obj = Object {
                obj_type: entry.obj_type,
                data: entry.data.clone(),
            };
            self.write_object(&obj)?;
            count += 1;
        }

        Ok(count)
    }

    /// Build a recursive tree from the index (handles subdirectories).
    pub fn write_tree_recursive(&self, index: &crate::index::Index) -> Result<Oid> {
        use crate::tree::TreeEntry;
        use alloc::collections::BTreeMap;

        // Group entries by their top-level directory
        let mut trees: BTreeMap<String, Vec<(String, &crate::index::IndexEntry)>> = BTreeMap::new();
        let mut direct_entries: Vec<TreeEntry> = Vec::new();

        for entry in &index.entries {
            if let Some(slash) = entry.name.find('/') {
                let dir = &entry.name[..slash];
                let rest = &entry.name[slash + 1..];
                trees.entry(String::from(dir))
                    .or_insert_with(Vec::new)
                    .push((String::from(rest), entry));
            } else {
                direct_entries.push(TreeEntry {
                    mode: if entry.mode == 0o100755 { 100755 } else { 100644 },
                    name: entry.name.clone(),
                    oid: entry.oid,
                });
            }
        }

        // Recursively build subtrees
        for (dir, sub_entries) in &trees {
            let mut sub_index = crate::index::Index::new();
            for (name, entry) in sub_entries {
                sub_index.add(crate::index::IndexEntry::new(
                    name,
                    entry.oid,
                    entry.mode,
                    entry.size,
                ));
            }
            let sub_tree_oid = self.write_tree_recursive(&sub_index)?;
            direct_entries.push(TreeEntry {
                mode: 40000,
                name: dir.clone(),
                oid: sub_tree_oid,
            });
        }

        let tree_data = crate::tree::build_tree(&mut direct_entries);
        let obj = Object::tree(tree_data);
        self.write_object(&obj)
    }

    /// Collect all objects reachable from a commit (for push).
    pub fn collect_objects(&self, oid: &Oid, exclude: &[Oid]) -> Result<Vec<Object>> {
        let mut objects = Vec::new();
        let mut visited = Vec::new();
        self.collect_objects_recursive(oid, exclude, &mut objects, &mut visited)?;
        Ok(objects)
    }

    fn collect_objects_recursive(
        &self,
        oid: &Oid,
        exclude: &[Oid],
        objects: &mut Vec<Object>,
        visited: &mut Vec<Oid>,
    ) -> Result<()> {
        if visited.contains(oid) || exclude.contains(oid) {
            return Ok(());
        }
        visited.push(*oid);

        let obj = match self.read_object(oid) {
            Ok(o) => o,
            Err(_) => return Ok(()), // Object might be on remote already
        };

        match obj.obj_type {
            ObjectType::Commit => {
                if let Some(commit) = crate::object::Commit::parse(&obj.data) {
                    objects.push(obj);
                    self.collect_objects_recursive(&commit.tree, exclude, objects, visited)?;
                    for parent in &commit.parents {
                        self.collect_objects_recursive(parent, exclude, objects, visited)?;
                    }
                }
            }
            ObjectType::Tree => {
                objects.push(obj.clone());
                let entries = crate::tree::parse_tree(&obj.data);
                for entry in &entries {
                    self.collect_objects_recursive(&entry.oid, exclude, objects, visited)?;
                }
            }
            ObjectType::Blob | ObjectType::Tag => {
                objects.push(obj);
            }
        }

        Ok(())
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn mkdir_p(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|_| Error::IoError)
}

fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    std::fs::write(path, data).map_err(|_| Error::IoError)
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|_| Error::NotFound)
}

fn read_file_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|_| Error::NotFound)
}
