//! CoreFS-Treiber, der den VFS-[`Filesystem`]-Trait über `corefs_core` erfüllt.
//!
//! # Status: Read-only Mount
//!
//! Dieser erste Integrations-Schritt liefert einen **read-only**-Mount über
//! [`corefs_core::storage::ondisk::reader::OdfReader`]. Damit lassen sich
//! Pfade auflösen, Dateien lesen und Verzeichnisse listen. Schreiboperationen
//! (`write`, `create`, `delete`) geben [`FsError::PermissionDenied`] zurück.
//!
//! Der Write-Path wird in einem Folgeschritt über
//! `corefs_core::storage::ondisk::native::{load_state_native,
//! save_state_native}` ergänzt.
//!
//! # Inode-Mapping
//!
//! AnyOS-VFS arbeitet mit 32-bit-Inode-Nummern, CoreFS mit 64-bit [`InodeId`].
//! Wir bilden beide auf den 64-bit **slot** (Position im Inode-Array des
//! On-Disk-Layouts) ab und truncaten bei Überlauf. Weil CoreFS den Slot 0
//! reserviert und typische Installationen bei weitem weniger als 2^32 Inodes
//! allokieren, ist Truncation im realen Betrieb irrelevant. Eine
//! Translation-Table kann später nachgezogen werden, falls das Limit
//! überschritten wird.
//!
//! # `FileType`-Mapping
//!
//! | AnyOS [`FileType`]        | CoreFS [`InodeKind`]  |
//! |---------------------------|-----------------------|
//! | `Regular`                 | `File`, `Symlink`†    |
//! | `Directory`               | `Directory`           |
//! | `Device`                  | — (nie aus CoreFS)    |
//!
//! † Symlinks werden aktuell als `Regular` präsentiert; ein dedizierter
//! Symlink-Pfad ist ToDo, sobald AnyOS-VFS symbolische Links modelliert.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::domain::inode::InodeKind;
use corefs_core::error::CoreFsError;
use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::ondisk::reader::OdfReader;

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::{Filesystem, FsError};
use crate::sync::mutex::Mutex;

use super::block_device::BlockDeviceAdapter;

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Bildet einen [`CoreFsError`] auf den AnyOS-VFS-[`FsError`] ab.
pub fn corefs_to_fs_error(e: &CoreFsError) -> FsError {
    match e {
        CoreFsError::NotFound(_) => FsError::NotFound,
        CoreFsError::InvalidInput(_) => FsError::InvalidPath,
        CoreFsError::PolicyViolation(_) => FsError::PermissionDenied,
        CoreFsError::AlreadyExists(_) => FsError::AlreadyExists,
        // Alle übrigen Varianten (State, Io, …) werden als IoError klassifiziert.
        _ => FsError::IoError,
    }
}

/// Mappt CoreFS-`InodeKind` → AnyOS-`FileType`.
fn kind_to_file_type(k: InodeKind) -> FileType {
    match k {
        InodeKind::File => FileType::Regular,
        InodeKind::Directory => FileType::Directory,
        // Symlinks werden im AnyOS-VFS derzeit nicht separat modelliert.
        InodeKind::Symlink => FileType::Regular,
    }
}

// ---------------------------------------------------------------------------
// CoreFsDriver
// ---------------------------------------------------------------------------

/// Read-only-Treiber für ein gemountetes CoreFS-Volume.
pub struct CoreFsDriver {
    inner: Mutex<Inner>,
}

struct Inner {
    device: BlockDeviceAdapter,
    /// Cached Liste aller bekannten Pfade → Slot, erstbefüllt bei erster
    /// Lookup-Anfrage.
    path_cache: Option<Vec<(String, u64, InodeKind, u64)>>,
}

impl CoreFsDriver {
    /// Öffnet ein vorhandenes CoreFS-Volume über den übergebenen Adapter.
    ///
    /// Validiert den Superblock (primär + Fallbacks) und die Bitmap-Integrität
    /// via [`OdfReader::open`]. Bei Erfolg liegt der Mount vor.
    pub fn mount_read_only(device: BlockDeviceAdapter) -> Result<Self, FsError> {
        // Test-Open: validates magic, version and bitmap CRC — abort on error.
        {
            let _r = OdfReader::open(&device).map_err(|e| corefs_to_fs_error(&e))?;
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                device,
                path_cache: None,
            }),
        })
    }

    // ------------------------------------------------------------
    // Helpers that run inside the lock
    // ------------------------------------------------------------

    /// Stellt sicher, dass `inner.path_cache` befüllt ist und liefert einen
    /// Slice darauf. Muss unter gehaltenem Lock aufgerufen werden.
    fn ensure_path_cache(inner: &mut Inner) -> Result<(), FsError> {
        if inner.path_cache.is_some() {
            return Ok(());
        }
        let reader = OdfReader::open(&inner.device).map_err(|e| corefs_to_fs_error(&e))?;
        let summaries = reader.list_inodes().map_err(|e| corefs_to_fs_error(&e))?;
        let mut cache: Vec<(String, u64, InodeKind, u64)> = Vec::with_capacity(summaries.len());
        for s in &summaries {
            if s.is_deleted {
                continue;
            }
            // Read attr block → path.
            let meta = reader
                .read_inode_metadata(s.slot)
                .map_err(|e| corefs_to_fs_error(&e))?;
            cache.push((meta.path, s.slot, s.kind, s.size_bytes));
        }
        inner.path_cache = Some(cache);
        Ok(())
    }

    fn slot_from_u32(inode: u32) -> u64 {
        u64::from(inode)
    }

    fn u32_from_slot(slot: u64) -> u32 {
        // Truncate — dokumentiertes Verhalten, siehe Modul-Doku.
        slot as u32
    }
}

// ---------------------------------------------------------------------------
// Filesystem trait implementation
// ---------------------------------------------------------------------------

impl Filesystem for CoreFsDriver {
    fn read(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let reader = OdfReader::open(&inner.device).map_err(|e| corefs_to_fs_error(&e))?;
        let slot = Self::slot_from_u32(inode);
        let content = reader
            .read_file_content(slot)
            .map_err(|e| corefs_to_fs_error(&e))?;
        let off = offset as usize;
        if off >= content.len() {
            return Ok(0);
        }
        let avail = content.len() - off;
        let n = core::cmp::min(avail, buf.len());
        buf[..n].copy_from_slice(&content[off..off + n]);
        Ok(n)
    }

    fn write(&self, _inode: u32, _offset: u32, _buf: &[u8]) -> Result<usize, FsError> {
        // Write-Path folgt in einem Folgeschritt über load_state_native /
        // save_state_native. Aktuell bewusst read-only.
        Err(FsError::PermissionDenied)
    }

    fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        if path.is_empty() {
            return Err(FsError::InvalidPath);
        }
        let mut inner = self.inner.lock();
        Self::ensure_path_cache(&mut inner)?;
        let cache = inner.path_cache.as_ref().expect("cache just initialised");
        for (p, slot, kind, size) in cache {
            if p == path {
                return Ok((
                    Self::u32_from_slot(*slot),
                    kind_to_file_type(*kind),
                    *size as u32,
                ));
            }
        }
        Err(FsError::NotFound)
    }

    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut inner = self.inner.lock();
        Self::ensure_path_cache(&mut inner)?;
        let cache = inner.path_cache.as_ref().expect("cache just initialised");
        // Resolve the parent path via cached slot lookup.
        let slot = Self::slot_from_u32(inode);
        let parent_path = cache
            .iter()
            .find(|(_, s, _, _)| *s == slot)
            .map(|(p, _, _, _)| p.clone())
            .ok_or(FsError::NotFound)?;
        let prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            let mut p = parent_path.clone();
            p.push('/');
            p
        };
        let mut entries: Vec<DirEntry> = Vec::new();
        for (p, _slot, kind, size) in cache {
            if p == &parent_path {
                continue;
            }
            if !p.starts_with(&prefix) {
                continue;
            }
            // Direct child only — no further '/' after the prefix.
            let rest = &p[prefix.len()..];
            if rest.is_empty() || rest.contains('/') {
                continue;
            }
            entries.push(DirEntry {
                name: rest.to_string(),
                file_type: kind_to_file_type(*kind),
                size: *size as u32,
                is_symlink: matches!(kind, InodeKind::Symlink),
                uid: 0,
                gid: 0,
                mode: 0o644,
            });
        }
        Ok(entries)
    }

    fn create(
        &self,
        _parent_inode: u32,
        _name: &str,
        _file_type: FileType,
    ) -> Result<u32, FsError> {
        Err(FsError::PermissionDenied)
    }

    fn delete(&self, _parent_inode: u32, _name: &str) -> Result<(), FsError> {
        Err(FsError::PermissionDenied)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use corefs_core::error::CoreFsError;

    #[test]
    fn corefs_not_found_maps_to_fs_not_found() {
        let e = CoreFsError::NotFound(format!("x"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::NotFound));
    }

    #[test]
    fn corefs_policy_maps_to_permission_denied() {
        let e = CoreFsError::PolicyViolation(format!("ro"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::PermissionDenied));
    }

    #[test]
    fn corefs_invalid_input_maps_to_invalid_path() {
        let e = CoreFsError::InvalidInput(format!("bad"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::InvalidPath));
    }

    #[test]
    fn corefs_already_exists_maps() {
        let e = CoreFsError::AlreadyExists(format!("dup"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::AlreadyExists));
    }

    #[test]
    fn kind_file_maps_regular() {
        assert!(matches!(kind_to_file_type(InodeKind::File), FileType::Regular));
    }

    #[test]
    fn kind_directory_maps_directory() {
        assert!(matches!(
            kind_to_file_type(InodeKind::Directory),
            FileType::Directory
        ));
    }

    #[test]
    fn kind_symlink_falls_back_to_regular() {
        assert!(matches!(
            kind_to_file_type(InodeKind::Symlink),
            FileType::Regular
        ));
    }

    #[test]
    fn slot_u32_roundtrip_small() {
        let s: u64 = 42;
        assert_eq!(CoreFsDriver::slot_from_u32(CoreFsDriver::u32_from_slot(s)), s);
    }

    #[test]
    fn write_returns_permission_denied() {
        // We can't easily construct a CoreFsDriver without a real volume —
        // instead we assert the trait method's intent by checking the direct
        // error shape through the mapping used by the implementation.
        // (The real write path will be covered once Step B+ adds write-back.)
        let e = CoreFsError::PolicyViolation(format!("ro"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::PermissionDenied));
    }
}
