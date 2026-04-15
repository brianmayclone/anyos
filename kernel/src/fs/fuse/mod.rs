// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! AnyOS FUSE-Subsystem (Skelett).
//!
//! # Status: Phase 5.6 — Skeleton
//!
//! Dieses Modul legt das Grundgerüst für ein in-Kernel-FUSE-Subsystem, das
//! es einem Userspace-Daemon (z. B. `corefsd`) erlaubt, einen VFS-Mount zu
//! bedienen. Die konkrete Bindung an den VFS-Dispatch (neue `FsType::Fuse`-
//! Variante, Character-Device `/dev/fuse`, Reply-Queue-Scheduling) ist
//! noch nicht verdrahtet — sie ist als **Phase 5.7** dokumentiert.
//!
//! Was dieses Modul heute liefert:
//! - Eine serialisierende [`FuseSession`]-Struktur mit Request-Queue,
//!   Pending-Reply-Map und monoton steigender `unique`-ID. Die Session
//!   ist unter [`crate::sync::mutex::Mutex`] zugriffssicher.
//! - Platzhalter-Operationen [`FuseSession::enqueue_request`] und
//!   [`FuseSession::deliver_reply`] — beide deterministisch, ohne
//!   echte IPC.
//!
//! Was ausdrücklich **nicht** passiert:
//! - Kein `/dev/fuse`-Gerät registriert.
//! - Keine `FsType::Fuse`-VFS-Variante.
//! - Keine Umleitung von VFS-Calls auf die Session.
//! - Keine Wire-Encodierung (die Wire-Kodierung lebt in
//!   `corefs_fuse_proto::{RequestFrame, ReplyFrame}` und wird vom
//!   zukünftigen `/dev/fuse`-Treiber bedient, nicht von diesem Modul).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::sync::mutex::Mutex;

/// Eindeutige Request-ID — gespiegelt vom Daemon in der passenden Reply.
/// Kompatibel zu `corefs_fuse_proto::Unique`.
pub type Unique = u64;

/// Ein einzelner, noch nicht beantworteter Request in der Queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// Eindeutige ID, die der Kernel bei [`FuseSession::enqueue_request`]
    /// vergibt und in der Reply wieder erwartet.
    pub unique: Unique,
    /// Opaque Request-Payload (später: serialisierter `RequestFrame`).
    pub body: Vec<u8>,
}

/// Eine Reply vom Daemon, die noch nicht an den wartenden VFS-Call
/// zurückgegeben wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingReply {
    /// ID des beantworteten Requests.
    pub unique: Unique,
    /// Opaque Reply-Payload (später: serialisierter `ReplyFrame`).
    pub body: Vec<u8>,
}

/// Fehler beim Session-Handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuseError {
    /// Es liegt keine Reply mit der angefragten `unique`-ID vor.
    NoMatchingReply,
    /// Session wurde bereits beendet.
    SessionClosed,
}

/// Pro-Mount FUSE-Session. Hält Queue + Pending-Replies + Unique-Counter.
///
/// **TODO (Phase 5.7):** Interaktion mit einem Wait-/Wake-Mechanismus, der
/// blockierende VFS-Calls schlafen legt, bis die passende Reply eintrifft.
/// Im aktuellen Skelett ist die API polling-basiert.
pub struct FuseSession {
    inner: Mutex<SessionInner>,
}

struct SessionInner {
    /// Queue ausgehender Requests (FIFO; Daemon zieht vom Kopf).
    queue: Vec<PendingRequest>,
    /// Replies, indiziert per `unique`. VFS-Caller polt hier.
    replies: BTreeMap<Unique, Vec<u8>>,
    /// Monoton steigend — wird bei jedem `enqueue_request` inkrementiert.
    next_unique: Unique,
    /// Gesetzt nach einem `Destroy`-Request bzw. explizitem Close.
    closed: bool,
}

impl FuseSession {
    /// Konstruiert eine leere Session.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SessionInner {
                queue: Vec::new(),
                replies: BTreeMap::new(),
                next_unique: 1,
                closed: false,
            }),
        }
    }

    /// Legt einen neuen Request in die Queue und vergibt eine frische
    /// `unique`-ID, die in der späteren Reply erwartet wird.
    pub fn enqueue_request(&self, body: Vec<u8>) -> Result<Unique, FuseError> {
        let mut inner = self.inner.lock();
        if inner.closed {
            return Err(FuseError::SessionClosed);
        }
        let unique = inner.next_unique;
        inner.next_unique = inner.next_unique.wrapping_add(1);
        inner.queue.push(PendingRequest { unique, body });
        Ok(unique)
    }

    /// Zieht den ältesten Request aus der Queue. Der Daemon ruft das auf.
    pub fn pop_request(&self) -> Option<PendingRequest> {
        let mut inner = self.inner.lock();
        if inner.queue.is_empty() {
            return None;
        }
        Some(inner.queue.remove(0))
    }

    /// Der Daemon liefert eine Reply zurück. Wird von der passenden
    /// `take_reply`-Aufruf konsumiert.
    pub fn deliver_reply(&self, reply: PendingReply) -> Result<(), FuseError> {
        let mut inner = self.inner.lock();
        if inner.closed {
            return Err(FuseError::SessionClosed);
        }
        inner.replies.insert(reply.unique, reply.body);
        Ok(())
    }

    /// Holt eine bereitliegende Reply (nicht-blockierend).
    pub fn take_reply(&self, unique: Unique) -> Result<Vec<u8>, FuseError> {
        let mut inner = self.inner.lock();
        inner.replies.remove(&unique).ok_or(FuseError::NoMatchingReply)
    }

    /// Markiert die Session als beendet. Folgende Enqueue-Versuche
    /// scheitern mit [`FuseError::SessionClosed`].
    pub fn close(&self) {
        let mut inner = self.inner.lock();
        inner.closed = true;
    }

    /// `true`, wenn noch Requests in der Queue warten.
    pub fn has_pending(&self) -> bool {
        let inner = self.inner.lock();
        !inner.queue.is_empty()
    }
}

impl Default for FuseSession {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn enqueue_assigns_monotonic_uniques() {
        let s = FuseSession::new();
        let u1 = s.enqueue_request(vec![1]).unwrap();
        let u2 = s.enqueue_request(vec![2]).unwrap();
        assert!(u2 > u1);
    }

    #[test]
    fn pop_returns_fifo_order() {
        let s = FuseSession::new();
        s.enqueue_request(vec![1]).unwrap();
        s.enqueue_request(vec![2]).unwrap();
        let a = s.pop_request().unwrap();
        let b = s.pop_request().unwrap();
        assert_eq!(a.body, vec![1]);
        assert_eq!(b.body, vec![2]);
        assert!(s.pop_request().is_none());
    }

    #[test]
    fn reply_is_matched_by_unique() {
        let s = FuseSession::new();
        let u = s.enqueue_request(vec![42]).unwrap();
        s.deliver_reply(PendingReply { unique: u, body: vec![7] }).unwrap();
        let body = s.take_reply(u).unwrap();
        assert_eq!(body, vec![7]);
    }

    #[test]
    fn take_reply_missing_returns_error() {
        let s = FuseSession::new();
        assert!(matches!(s.take_reply(999), Err(FuseError::NoMatchingReply)));
    }

    #[test]
    fn closed_session_rejects_new_requests() {
        let s = FuseSession::new();
        s.close();
        assert!(matches!(s.enqueue_request(vec![0]), Err(FuseError::SessionClosed)));
    }
}
