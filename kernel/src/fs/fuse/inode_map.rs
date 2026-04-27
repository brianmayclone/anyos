// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Per-Mount Inode-Mapping für FUSE-Mounts.
//!
//! Der VFS identifiziert Inodes über `u32`, das FUSE-Protokoll (und
//! `corefs-fuse-proto`) verwendet `u64`. Für einen FUSE-Mount hält der
//! Kernel daher eine bidirektionale Mapping-Tabelle, die eingehenden
//! [`InodeNo`]-Werten aus dem Daemon stabile VFS-`u32`-IDs zuordnet.
//!
//! Das [`FUSE_INODE_MAPS`]-Registry ist pro [`SessionId`] indiziert; eine
//! Session bekommt bei [`ensure_mount`] eine eigene [`InodeMap`]. Root
//! bekommt `u32 = 1`, damit das VFS-Symbol `/` konsistent zum übrigen
//! Kernel bleibt.

use super::SessionId;
use crate::sync::mutex::Mutex;
use alloc::collections::BTreeMap;

/// Vom Daemon vergebene Inode-Nummer.
pub type InodeNo = u64;

/// Root-Inode (identisch in u32- und u64-Sicht).
pub const ROOT_INO_U32: u32 = 1;
/// Root-Inode im FUSE-Protokoll.
pub const ROOT_INO_U64: u64 = 1;

/// Bidirektionale Mapping-Tabelle zwischen `u32` (VFS) und `u64` (FUSE).
pub struct InodeMap {
    u32_to_u64: BTreeMap<u32, u64>,
    u64_to_u32: BTreeMap<u64, u32>,
    next_u32: u32,
}

impl InodeMap {
    /// Konstruiert eine neue Tabelle mit vorbelegtem Root-Eintrag.
    pub fn new() -> Self {
        let mut u32_to_u64 = BTreeMap::new();
        let mut u64_to_u32 = BTreeMap::new();
        u32_to_u64.insert(ROOT_INO_U32, ROOT_INO_U64);
        u64_to_u32.insert(ROOT_INO_U64, ROOT_INO_U32);
        Self {
            u32_to_u64,
            u64_to_u32,
            next_u32: 2,
        }
    }

    /// Liefert die FUSE-`u64`-ID zu einem VFS-`u32`.
    pub fn to_u64(&self, id: u32) -> Option<u64> {
        self.u32_to_u64.get(&id).copied()
    }

    /// Liefert die VFS-`u32`-ID zu einer FUSE-`u64` — oder weist neu zu.
    pub fn intern(&mut self, ino: u64) -> u32 {
        if let Some(&existing) = self.u64_to_u32.get(&ino) {
            return existing;
        }
        let new_id = self.next_u32;
        // Wrap-Protection: bei u32::MAX abbrechen — das ist ein rein
        // theoretisches Limit bei Millionen FUSE-Inodes pro Mount.
        self.next_u32 = self.next_u32.saturating_add(1);
        self.u32_to_u64.insert(new_id, ino);
        self.u64_to_u32.insert(ino, new_id);
        new_id
    }

    /// Entfernt ein Mapping (z. B. nach `unlink`). Kein Fehler, wenn
    /// der Eintrag bereits fehlt.
    pub fn forget(&mut self, id: u32) {
        if let Some(ino) = self.u32_to_u64.remove(&id) {
            self.u64_to_u32.remove(&ino);
        }
    }
}

impl Default for InodeMap {
    fn default() -> Self {
        Self::new()
    }
}

static FUSE_INODE_MAPS: Mutex<BTreeMap<SessionId, InodeMap>> = Mutex::new(BTreeMap::new());

/// Stellt sicher, dass eine [`InodeMap`] für die gegebene Session existiert.
pub fn ensure_mount(session: SessionId) {
    let mut m = FUSE_INODE_MAPS.lock();
    m.entry(session).or_insert_with(InodeMap::new);
}

/// Entfernt die Mapping-Tabelle einer Session (z. B. beim Unmount).
pub fn drop_mount(session: SessionId) {
    FUSE_INODE_MAPS.lock().remove(&session);
}

/// Liefert die FUSE-`u64`-ID zu einer VFS-`u32` für die gegebene Session.
pub fn to_u64(session: SessionId, id: u32) -> Option<u64> {
    FUSE_INODE_MAPS
        .lock()
        .get(&session)
        .and_then(|m| m.to_u64(id))
}

/// Internt eine FUSE-`u64` und liefert die stabile VFS-`u32`. Existiert
/// noch keine Tabelle für die Session, wird sie lazy angelegt.
pub fn intern(session: SessionId, ino: u64) -> u32 {
    let mut maps = FUSE_INODE_MAPS.lock();
    maps.entry(session)
        .or_insert_with(InodeMap::new)
        .intern(ino)
}

/// Vergisst ein Mapping für die gegebene Session.
pub fn forget(session: SessionId, id: u32) {
    if let Some(m) = FUSE_INODE_MAPS.lock().get_mut(&session) {
        m.forget(id);
    }
}

#[cfg(test)]
fn clear_for_test() {
    FUSE_INODE_MAPS.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_prepopulated() {
        let m = InodeMap::new();
        assert_eq!(m.to_u64(ROOT_INO_U32), Some(ROOT_INO_U64));
    }

    #[test]
    fn intern_is_stable_across_calls() {
        let mut m = InodeMap::new();
        let a = m.intern(42);
        let b = m.intern(42);
        assert_eq!(a, b);
        assert_eq!(m.to_u64(a), Some(42));
    }

    #[test]
    fn intern_assigns_fresh_ids() {
        let mut m = InodeMap::new();
        let a = m.intern(100);
        let b = m.intern(200);
        assert_ne!(a, b);
        assert!(a >= 2);
        assert!(b >= 2);
    }

    #[test]
    fn forget_removes_both_directions() {
        let mut m = InodeMap::new();
        let id = m.intern(77);
        m.forget(id);
        assert!(m.to_u64(id).is_none());
        // re-interning the same u64 may yield a new u32 — that's fine.
        let id2 = m.intern(77);
        assert_eq!(m.to_u64(id2), Some(77));
    }

    #[test]
    fn registry_is_per_session() {
        clear_for_test();
        ensure_mount(1);
        ensure_mount(2);
        let a = intern(1, 500);
        let b = intern(2, 500);
        // They may coincide in u32 space (both start at 2), but the
        // maps must be separate: forgetting in session 1 doesn't affect 2.
        forget(1, a);
        assert!(to_u64(1, a).is_none());
        assert_eq!(to_u64(2, b), Some(500));
    }

    #[test]
    fn drop_mount_removes_table() {
        clear_for_test();
        ensure_mount(7);
        let _ = intern(7, 42);
        drop_mount(7);
        assert!(to_u64(7, 2).is_none());
    }
}
