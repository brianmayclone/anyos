// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! [`CoreFsHandler`] — bindet eine [`OdfDeviceSession`] an das
//! FUSE-Wire-Protokoll. Übersetzt FUSE-Ops in CoreFS-`PersistedState`-
//! Mutationen, mappt zwischen FUSE-`InodeNo` (u64) und CoreFS-`InodeId`
//! plus Pfad.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::domain::inode::{Inode, InodeId, InodeKind};
use corefs_core::domain::metadata::FileMetadata;
use corefs_core::error::CoreFsError;
use corefs_core::platform::Timestamp;
use corefs_core::storage::block_store::BlockRecord;
use corefs_core::storage::ondisk::session::OdfDeviceSession;
use corefs_fuse_adapter::{FuseHandler, HandlerResult};
use corefs_fuse_proto::{Attr, DirEntry, PROTOCOL_VERSION, Reply, Request, StatFs};

const ENOENT: i32 = 2;
const EIO: i32 = 5;
const EEXIST: i32 = 17;
const EINVAL: i32 = 22;
const ENOTDIR: i32 = 20;

fn err_from(e: &CoreFsError) -> i32 {
    match e {
        CoreFsError::NotFound(_) => ENOENT,
        CoreFsError::AlreadyExists(_) => EEXIST,
        CoreFsError::InvalidInput(_) | CoreFsError::InvalidCommand(_) => EINVAL,
        _ => EIO,
    }
}

fn inode_kind_to_u8(k: InodeKind) -> u8 {
    match k {
        InodeKind::File => 1,
        InodeKind::Directory => 2,
        InodeKind::Symlink => 3,
    }
}

fn default_mode_for(kind: InodeKind, metadata_mode: u32) -> u32 {
    if metadata_mode != 0 {
        return metadata_mode;
    }
    match kind {
        InodeKind::Directory => 0o755,
        InodeKind::File => 0o644,
        InodeKind::Symlink => 0o777,
    }
}

/// Vom FUSE-Proto genutzte Inode-Nummer (spiegelt den Typ aus
/// `corefs-fuse-proto::InodeNo`; hier dupliziert, damit dieses Modul
/// no_std-Bewusst bleibt).
type FuseInodeNo = u64;

/// Stateful handler behind the `/dev/fuse` event loop.
pub struct CoreFsHandler {
    session: OdfDeviceSession,
    /// Map FUSE InodeNo → absoluten CoreFS-Pfad (Root = 1 → "/").
    inode_by_no: BTreeMap<FuseInodeNo, String>,
    /// Rückweg: CoreFS-InodeId → FUSE-InodeNo (um beim Lookup alte Nummern
    /// wiederverwenden zu können).
    no_by_inode_id: BTreeMap<InodeId, FuseInodeNo>,
    next_no: FuseInodeNo,
    /// Offene FileHandles → (FUSE-InodeNo, current_offset).
    open_files: BTreeMap<u64, (FuseInodeNo, u64)>,
    next_fh: u64,
    /// Feste Zeitquelle für Inode-Zeitstempel (no_std). Production könnte
    /// einen Clock-Service einhängen; hier nutzen wir EPOCH als platzhalter.
    clock: Timestamp,
}

impl CoreFsHandler {
    /// Konstruiert den Handler aus einer offenen CoreFS-Session. Root
    /// bekommt die FUSE-InodeNo `1` und ist immer `/`.
    pub fn new(session: OdfDeviceSession) -> Self {
        let mut inode_by_no = BTreeMap::new();
        inode_by_no.insert(1u64, "/".to_string());
        Self {
            session,
            inode_by_no,
            no_by_inode_id: BTreeMap::new(),
            next_no: 2,
            open_files: BTreeMap::new(),
            next_fh: 1,
            clock: Timestamp::EPOCH,
        }
    }

    /// Liefert die FUSE-InodeNo zu einem Inode — vergibt sie bei Bedarf neu.
    fn intern(&mut self, inode: &Inode) -> FuseInodeNo {
        if let Some(&no) = self.no_by_inode_id.get(&inode.id) {
            // Pfad ggf. updaten (nach rename).
            self.inode_by_no.insert(no, inode.path.clone());
            return no;
        }
        let no = self.next_no;
        self.next_no = self.next_no.saturating_add(1);
        self.inode_by_no.insert(no, inode.path.clone());
        self.no_by_inode_id.insert(inode.id, no);
        no
    }

    fn path_of(&self, ino: FuseInodeNo) -> Option<&String> {
        self.inode_by_no.get(&ino)
    }

    fn inode_for_path<'a>(
        state: &'a corefs_core::storage::persisted_state::PersistedState,
        path: &str,
    ) -> Option<&'a Inode> {
        state.active_inodes.iter().find(|i| i.path == path)
    }

    /// Baut das FUSE-`Attr` aus einem Inode.
    fn attr_for(&self, ino: FuseInodeNo, inode: &Inode) -> Attr {
        let mode_bits = default_mode_for(inode.kind, inode.metadata.mode);
        let kind = inode_kind_to_u8(inode.kind);
        Attr {
            ino,
            size: inode.size as u64,
            blocks: ((inode.size as u64) / 512).max(1),
            mode: mode_bits,
            nlink: 1,
            uid: inode.metadata.uid,
            gid: inode.metadata.gid,
            kind,
            crtime_secs: inode.created_at.as_secs(),
            mtime_secs: inode.modified_at.as_secs(),
            ctime_secs: inode.changed_at.as_secs(),
        }
    }

    // --- Individual op handlers ----------------------------------------

    fn op_getattr(&mut self, ino: FuseInodeNo) -> HandlerResult {
        let path = match self.path_of(ino) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let state = self.session.state();
        if path == "/" {
            // Pseudo-Root, nicht zwingend als Inode gespeichert.
            return HandlerResult::Ok(Reply::Getattr(Attr {
                ino: 1,
                size: 0,
                blocks: 1,
                mode: 0o755,
                nlink: 1,
                uid: 0,
                gid: 0,
                kind: 2,
                crtime_secs: 0,
                mtime_secs: 0,
                ctime_secs: 0,
            }));
        }
        match Self::inode_for_path(state, &path) {
            Some(inode) => {
                let attr = self.attr_for(ino, inode);
                HandlerResult::Ok(Reply::Getattr(attr))
            }
            None => HandlerResult::Err {
                errno: ENOENT,
                message: None,
            },
        }
    }

    fn op_lookup(&mut self, parent: FuseInodeNo, name: String) -> HandlerResult {
        let parent_path = match self.path_of(parent) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let child_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };
        // First resolve to a &Inode then clone what we need to drop the
        // borrow before self.intern() takes &mut self.
        let (inode_id, inode_clone) = {
            let state = self.session.state();
            match Self::inode_for_path(state, &child_path) {
                Some(i) => (i.id, i.clone()),
                None => {
                    return HandlerResult::Err {
                        errno: ENOENT,
                        message: None,
                    };
                }
            }
        };
        let _ = inode_id;
        let no = self.intern(&inode_clone);
        let attr = self.attr_for(no, &inode_clone);
        HandlerResult::Ok(Reply::Lookup(attr))
    }

    fn op_readdir(&mut self, parent: FuseInodeNo) -> HandlerResult {
        let parent_path = match self.path_of(parent) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        // All active inodes whose parent is `parent_path`.
        let mut entries: Vec<(Inode,)> = Vec::new();
        {
            let state = self.session.state();
            for inode in state.active_inodes.iter() {
                if let Some(p) = parent_of(&inode.path) {
                    if p == parent_path {
                        entries.push((inode.clone(),));
                    }
                }
            }
        }
        let mut out: Vec<DirEntry> = Vec::with_capacity(entries.len());
        let mut offset = 1u64;
        for (inode,) in entries {
            let name = name_of(&inode.path).unwrap_or_default().to_string();
            let no = self.intern(&inode);
            out.push(DirEntry {
                ino: no,
                kind: inode_kind_to_u8(inode.kind),
                name,
                next_offset: offset,
            });
            offset += 1;
        }
        HandlerResult::Ok(Reply::Readdir(out))
    }

    fn op_open(&mut self, ino: FuseInodeNo) -> HandlerResult {
        if self.path_of(ino).is_none() && ino != 1 {
            return HandlerResult::Err {
                errno: ENOENT,
                message: None,
            };
        }
        let fh = self.next_fh;
        self.next_fh = self.next_fh.saturating_add(1);
        self.open_files.insert(fh, (ino, 0));
        HandlerResult::Ok(Reply::Open { fh, flags: 0 })
    }

    fn op_release(&mut self, _ino: FuseInodeNo, fh: u64) -> HandlerResult {
        self.open_files.remove(&fh);
        HandlerResult::Ok(Reply::Release)
    }

    fn op_read(&mut self, ino: FuseInodeNo, offset: u64, size: u32) -> HandlerResult {
        let path = match self.path_of(ino) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let state = self.session.state();
        let inode_id = match Self::inode_for_path(state, &path) {
            Some(i) => i.id,
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let data = state
            .block_records
            .iter()
            .find(|r| r.inode == inode_id)
            .map(|r| r.bytes.clone())
            .unwrap_or_default();
        let start = offset as usize;
        let end = start.saturating_add(size as usize).min(data.len());
        let slice = if start >= data.len() {
            Vec::new()
        } else {
            data[start..end].to_vec()
        };
        HandlerResult::Ok(Reply::Read { data: slice })
    }

    fn op_write(&mut self, ino: FuseInodeNo, offset: u64, data: Vec<u8>) -> HandlerResult {
        let path = match self.path_of(ino) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let now = self.clock;
        let write_len = data.len();
        let result = self.session.mutate(|state| {
            let inode_id = state
                .active_inodes
                .iter()
                .find(|i| i.path == path)
                .map(|i| i.id)
                .ok_or_else(|| CoreFsError::NotFound(path.clone()))?;
            // Expand-or-create the corresponding block_record.
            let rec = if let Some(idx) = state
                .block_records
                .iter()
                .position(|r| r.inode == inode_id)
            {
                &mut state.block_records[idx]
            } else {
                state.block_records.push(BlockRecord {
                    inode: inode_id,
                    bytes: Vec::new(),
                    checksum: 0,
                    device_block: 0,
                    allocated_blocks: 0,
                });
                state.block_records.last_mut().unwrap()
            };
            let start = offset as usize;
            let end = start + data.len();
            if rec.bytes.len() < end {
                rec.bytes.resize(end, 0);
            }
            rec.bytes[start..end].copy_from_slice(&data);
            // Update inode size/mtime.
            if let Some(inode) = state.active_inodes.iter_mut().find(|i| i.id == inode_id) {
                if end > inode.size {
                    inode.size = end;
                }
                inode.touch_modified_at(now);
            }
            Ok(())
        });
        match result {
            Ok(_) => HandlerResult::Ok(Reply::Write {
                written: write_len as u32,
            }),
            Err(e) => HandlerResult::Err {
                errno: err_from(&e),
                message: Some(e.to_string()),
            },
        }
    }

    fn op_create(&mut self, parent: FuseInodeNo, name: String) -> HandlerResult {
        self.create_entry(parent, name, InodeKind::File, true)
    }

    fn op_mkdir(&mut self, parent: FuseInodeNo, name: String) -> HandlerResult {
        self.create_entry(parent, name, InodeKind::Directory, false)
    }

    fn create_entry(
        &mut self,
        parent: FuseInodeNo,
        name: String,
        kind: InodeKind,
        assign_fh: bool,
    ) -> HandlerResult {
        let parent_path = match self.path_of(parent) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        if name.is_empty() || name.contains('/') {
            return HandlerResult::Err {
                errno: EINVAL,
                message: None,
            };
        }
        let child_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };
        let now = self.clock;

        let result = self.session.mutate(|state| {
            if state.active_inodes.iter().any(|i| i.path == child_path) {
                return Err(CoreFsError::AlreadyExists(child_path.clone()));
            }
            // Assign a fresh InodeId.
            let next_id = state
                .active_inodes
                .iter()
                .map(|i| i.id.0)
                .chain(state.deleted_inodes.iter().map(|i| i.id.0))
                .max()
                .unwrap_or(0)
                + 1;
            let mut metadata = FileMetadata::default();
            metadata.mode = match kind {
                InodeKind::Directory => 0o755,
                InodeKind::File => 0o644,
                InodeKind::Symlink => 0o777,
            };
            let inode = Inode::new_at(
                InodeId(next_id),
                kind,
                child_path.clone(),
                metadata,
                now,
            );
            state.active_inodes.push(inode);
            Ok(InodeId(next_id))
        });
        match result {
            Ok((id, _)) => {
                // Look up the freshly-created inode so we can build Attr.
                let inode_clone = {
                    let state = self.session.state();
                    state
                        .active_inodes
                        .iter()
                        .find(|i| i.id == id)
                        .cloned()
                };
                let Some(inode) = inode_clone else {
                    return HandlerResult::Err {
                        errno: EIO,
                        message: None,
                    };
                };
                let no = self.intern(&inode);
                let attr = self.attr_for(no, &inode);
                if assign_fh {
                    let fh = self.next_fh;
                    self.next_fh = self.next_fh.saturating_add(1);
                    self.open_files.insert(fh, (no, 0));
                    HandlerResult::Ok(Reply::Create { attr, fh, flags: 0 })
                } else {
                    HandlerResult::Ok(Reply::Mkdir(attr))
                }
            }
            Err(e) => HandlerResult::Err {
                errno: err_from(&e),
                message: Some(e.to_string()),
            },
        }
    }

    fn op_unlink(&mut self, parent: FuseInodeNo, name: String) -> HandlerResult {
        let parent_path = match self.path_of(parent) {
            Some(p) => p.clone(),
            None => {
                return HandlerResult::Err {
                    errno: ENOENT,
                    message: None,
                };
            }
        };
        let child_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };
        let result = self.session.mutate(|state| {
            let idx = state
                .active_inodes
                .iter()
                .position(|i| i.path == child_path)
                .ok_or_else(|| CoreFsError::NotFound(child_path.clone()))?;
            let removed = state.active_inodes.remove(idx);
            // Remove backing block records.
            state.block_records.retain(|r| r.inode != removed.id);
            state.deleted_inodes.push(removed);
            Ok(())
        });
        match result {
            Ok(_) => {
                // Forget the InodeNo mapping for this path.
                let stale: Vec<FuseInodeNo> = self
                    .inode_by_no
                    .iter()
                    .filter(|(_, p)| **p == child_path)
                    .map(|(n, _)| *n)
                    .collect();
                for n in stale {
                    self.inode_by_no.remove(&n);
                }
                self.no_by_inode_id.retain(|_, v| {
                    self.inode_by_no.contains_key(v)
                });
                HandlerResult::Ok(Reply::Unlink)
            }
            Err(e) => HandlerResult::Err {
                errno: err_from(&e),
                message: Some(e.to_string()),
            },
        }
    }

    fn op_statfs(&self) -> HandlerResult {
        let state = self.session.state();
        let block_size = state.volume.block_size as u32;
        let used_blocks: u64 = state
            .block_records
            .iter()
            .map(|r| r.allocated_blocks)
            .sum();
        // VolumeDescriptor does not track total_blocks directly in this
        // snapshot — fake it with a generous constant derived from the
        // inode count so statfs doesn't report zero free blocks.
        let total_blocks = used_blocks.saturating_add(state.active_inodes.len() as u64 * 16).max(1024);
        let free = total_blocks.saturating_sub(used_blocks);
        HandlerResult::Ok(Reply::Statfs(StatFs {
            bsize: block_size.max(512),
            blocks: total_blocks,
            bfree: free,
            bavail: free,
            files: state.active_inodes.len() as u64,
            ffree: 0,
            namelen: 255,
        }))
    }
}

impl FuseHandler for CoreFsHandler {
    fn handle(&mut self, request: Request) -> HandlerResult {
        match request {
            Request::Init { .. } => HandlerResult::Ok(Reply::Init {
                version: PROTOCOL_VERSION,
            }),
            Request::Destroy => HandlerResult::Ok(Reply::Destroy),
            Request::Getattr { ino } => self.op_getattr(ino),
            Request::Lookup { parent, name } => self.op_lookup(parent, name),
            Request::Readdir { ino, .. } => self.op_readdir(ino),
            Request::Open { ino, .. } => self.op_open(ino),
            Request::Release { ino, fh } => self.op_release(ino, fh),
            Request::Read { ino, offset, size, .. } => self.op_read(ino, offset, size),
            Request::Write { ino, offset, data, .. } => self.op_write(ino, offset, data),
            Request::Create { parent, name, .. } => self.op_create(parent, name),
            Request::Mkdir { parent, name, .. } => self.op_mkdir(parent, name),
            Request::Unlink { parent, name } => self.op_unlink(parent, name),
            Request::Statfs => self.op_statfs(),
        }
    }
}

fn parent_of(path: &str) -> Option<&str> {
    if path == "/" || path.is_empty() {
        return None;
    }
    match path.rfind('/') {
        Some(0) => Some("/"),
        Some(pos) => Some(&path[..pos]),
        None => None,
    }
}

fn name_of(path: &str) -> Option<&str> {
    if path == "/" || path.is_empty() {
        return None;
    }
    path.rsplit('/').next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use corefs_core::config::CoreFsConfig;
    use corefs_core::storage::block_device::{BlockDevice, MemoryDevice};
    use corefs_core::storage::ondisk::session::OdfSessionOptions;

    fn mk_session() -> OdfDeviceSession {
        let opts = OdfSessionOptions {
            capacity_bytes: 4 * 1024 * 1024,
            ..OdfSessionOptions::with_defaults()
        };
        let dev: Box<dyn BlockDevice> =
            Box::new(MemoryDevice::new(4 * 1024 * 1024, 4096).unwrap());
        OdfDeviceSession::format_new_at(dev, &opts, Timestamp::EPOCH).unwrap()
    }

    fn mk_handler() -> CoreFsHandler {
        CoreFsHandler::new(mk_session())
    }

    #[test]
    fn init_returns_protocol_version() {
        let mut h = mk_handler();
        match h.handle(Request::Init {
            version: PROTOCOL_VERSION,
        }) {
            HandlerResult::Ok(Reply::Init { version }) => assert_eq!(version, PROTOCOL_VERSION),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn getattr_root_is_directory() {
        let mut h = mk_handler();
        match h.handle(Request::Getattr { ino: 1 }) {
            HandlerResult::Ok(Reply::Getattr(a)) => {
                assert_eq!(a.ino, 1);
                assert_eq!(a.kind, 2);
            }
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn readdir_root_is_empty_initially() {
        let mut h = mk_handler();
        match h.handle(Request::Readdir { ino: 1, offset: 0 }) {
            HandlerResult::Ok(Reply::Readdir(v)) => assert!(v.is_empty()),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn create_then_getattr_roundtrip() {
        let mut h = mk_handler();
        let attr = match h.handle(Request::Create {
            parent: 1,
            name: "hello.txt".to_string(),
            mode: 0o644,
            flags: 0,
        }) {
            HandlerResult::Ok(Reply::Create { attr, .. }) => attr,
            other => panic!("unexpected {:?}", other),
        };
        assert_eq!(attr.kind, 1);
        match h.handle(Request::Getattr { ino: attr.ino }) {
            HandlerResult::Ok(Reply::Getattr(a2)) => assert_eq!(a2.ino, attr.ino),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn lookup_after_create_succeeds() {
        let mut h = mk_handler();
        let _ = h.handle(Request::Create {
            parent: 1,
            name: "a".to_string(),
            mode: 0o644,
            flags: 0,
        });
        match h.handle(Request::Lookup {
            parent: 1,
            name: "a".to_string(),
        }) {
            HandlerResult::Ok(Reply::Lookup(attr)) => assert_eq!(attr.kind, 1),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn write_then_read_roundtrip() {
        let mut h = mk_handler();
        let created = h.handle(Request::Create {
            parent: 1,
            name: "w.txt".to_string(),
            mode: 0o644,
            flags: 0,
        });
        let (ino, fh) = match created {
            HandlerResult::Ok(Reply::Create { attr, fh, .. }) => (attr.ino, fh),
            other => panic!("unexpected {:?}", other),
        };
        // Write
        match h.handle(Request::Write {
            ino,
            fh,
            offset: 0,
            data: b"hello".to_vec(),
        }) {
            HandlerResult::Ok(Reply::Write { written }) => assert_eq!(written, 5),
            other => panic!("unexpected {:?}", other),
        }
        // Read
        match h.handle(Request::Read {
            ino,
            fh,
            offset: 0,
            size: 16,
        }) {
            HandlerResult::Ok(Reply::Read { data }) => assert_eq!(&data, b"hello"),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn unlink_removes_entry() {
        let mut h = mk_handler();
        let _ = h.handle(Request::Create {
            parent: 1,
            name: "u".to_string(),
            mode: 0o644,
            flags: 0,
        });
        match h.handle(Request::Unlink {
            parent: 1,
            name: "u".to_string(),
        }) {
            HandlerResult::Ok(Reply::Unlink) => {}
            other => panic!("unexpected {:?}", other),
        }
        // Follow-up lookup must fail.
        match h.handle(Request::Lookup {
            parent: 1,
            name: "u".to_string(),
        }) {
            HandlerResult::Err { errno, .. } => assert_eq!(errno, ENOENT),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn mkdir_then_readdir_lists_it() {
        let mut h = mk_handler();
        match h.handle(Request::Mkdir {
            parent: 1,
            name: "d".to_string(),
            mode: 0o755,
        }) {
            HandlerResult::Ok(Reply::Mkdir(a)) => assert_eq!(a.kind, 2),
            other => panic!("unexpected {:?}", other),
        }
        match h.handle(Request::Readdir { ino: 1, offset: 0 }) {
            HandlerResult::Ok(Reply::Readdir(v)) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].name, "d");
                assert_eq!(v[0].kind, 2);
            }
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn open_release_cycle() {
        let mut h = mk_handler();
        let _ = h.handle(Request::Create {
            parent: 1,
            name: "x".to_string(),
            mode: 0o644,
            flags: 0,
        });
        let attr = match h.handle(Request::Lookup {
            parent: 1,
            name: "x".to_string(),
        }) {
            HandlerResult::Ok(Reply::Lookup(a)) => a,
            other => panic!("unexpected {:?}", other),
        };
        let fh = match h.handle(Request::Open { ino: attr.ino, flags: 0 }) {
            HandlerResult::Ok(Reply::Open { fh, .. }) => fh,
            other => panic!("unexpected {:?}", other),
        };
        match h.handle(Request::Release { ino: attr.ino, fh }) {
            HandlerResult::Ok(Reply::Release) => {}
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn statfs_reports_volume() {
        let h = mk_handler();
        let res = CoreFsHandler::op_statfs(&h);
        match res {
            HandlerResult::Ok(Reply::Statfs(s)) => {
                assert!(s.bsize >= 512);
            }
            other => panic!("unexpected {:?}", other),
        }
    }
}
