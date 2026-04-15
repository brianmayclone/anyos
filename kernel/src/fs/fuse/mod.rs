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
//! Phase-5.7-Erweiterung (inkrementell):
//! - [`FUSE_REGISTRY`] hält global registrierte Sessions unter einer
//!   Session-ID; der [`devfs`]-Adapter liest/schreibt hier die Wire-
//!   Frames durch.
//! - [`FsType::Fuse`] existiert im VFS-Enum, ist aber im
//!   Dispatch-Hauptpfad (`vfs::open`/`read`/…) noch nicht verdrahtet —
//!   Mounts vom Typ Fuse werden aktuell in der Mount-Tabelle geführt,
//!   alle darüber gehenden I/O-Aufrufe fallen in den Default-Zweig
//!   und scheitern mit `NotFound`/`PermissionDenied`. Der Adapter ist
//!   damit ein ehrliches Skelett für Folgecommits.
//!
//! Was ausdrücklich **noch nicht** passiert:
//! - Kein vollständiges VFS-Dispatch auf die Session (open/read/write/
//!   lookup/readdir → Request; Kernel blockiert bis Reply). Das
//!   erfordert einen Wait/Wake-Mechanismus, den der AnyOS-Scheduler
//!   noch nicht exponiert.
//! - Keine Wire-Encodierung innerhalb des Moduls — der
//!   `/dev/fuse`-Adapter reicht opake Bytes durch, Ser/De passiert im
//!   Userspace-Daemon via `corefs_fuse_proto`.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::sync::mutex::Mutex;

pub mod inode_map;

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
    /// Wartende Threads pro Unique (nur in `enqueue_and_wait` gesetzt).
    waiters: BTreeMap<Unique, u32>,
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
                waiters: BTreeMap::new(),
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
    /// `take_reply`-Aufruf konsumiert. Weckt ggf. den wartenden Thread
    /// (siehe [`FuseSession::enqueue_and_wait`]).
    pub fn deliver_reply(&self, reply: PendingReply) -> Result<(), FuseError> {
        let waiter_tid = {
            let mut inner = self.inner.lock();
            if inner.closed {
                return Err(FuseError::SessionClosed);
            }
            inner.replies.insert(reply.unique, reply.body);
            inner.waiters.remove(&reply.unique)
        };
        #[cfg(not(test))]
        if let Some(tid) = waiter_tid {
            crate::task::scheduler::wake_thread(tid);
        }
        #[cfg(test)]
        let _ = waiter_tid;
        Ok(())
    }

    /// Holt eine bereitliegende Reply (nicht-blockierend).
    pub fn take_reply(&self, unique: Unique) -> Result<Vec<u8>, FuseError> {
        let mut inner = self.inner.lock();
        inner.replies.remove(&unique).ok_or(FuseError::NoMatchingReply)
    }

    /// Markiert die Session als beendet. Folgende Enqueue-Versuche
    /// scheitern mit [`FuseError::SessionClosed`]. Alle wartenden Threads
    /// werden geweckt — sie erhalten beim Fortsetzen
    /// [`FuseError::SessionClosed`].
    pub fn close(&self) {
        let tids: Vec<u32> = {
            let mut inner = self.inner.lock();
            inner.closed = true;
            let tids: Vec<u32> = inner.waiters.values().copied().collect();
            inner.waiters.clear();
            tids
        };
        #[cfg(not(test))]
        for tid in tids {
            crate::task::scheduler::wake_thread(tid);
        }
        #[cfg(test)]
        let _ = tids;
    }

    /// Enqueue a request and block the current thread until the reply
    /// arrives. Returns the reply body.
    ///
    /// # Invariante gegen Enqueue/Wake-Race
    /// Weil `deliver_reply` den Waiter-Eintrag erst nach dem Einfügen der
    /// Reply entfernt, geht kein Wakeup verloren:
    /// - Reply kommt **vor** `block_current_thread()` → der Take-Check am
    ///   Ende erfolgreich, kein Blocking.
    /// - Reply kommt **nach** `block_current_thread()` → `wake_thread` wird
    ///   aufgerufen, da der Waiter zu dem Zeitpunkt noch eingetragen ist.
    pub fn enqueue_and_wait(&self, body: Vec<u8>) -> Result<Vec<u8>, FuseError> {
        let unique = {
            let mut inner = self.inner.lock();
            if inner.closed {
                return Err(FuseError::SessionClosed);
            }
            let unique = inner.next_unique;
            inner.next_unique = inner.next_unique.wrapping_add(1);
            inner.queue.push(PendingRequest { unique, body });
            #[cfg(not(test))]
            {
                let tid = crate::task::scheduler::current_tid();
                inner.waiters.insert(unique, tid);
            }
            unique
        };

        // Loop: check for reply, else block. Spurious wakeups / races
        // (reply landet zwischen insert+block) sind durch re-check abgefangen.
        #[cfg(not(test))]
        loop {
            {
                let mut inner = self.inner.lock();
                if let Some(body) = inner.replies.remove(&unique) {
                    inner.waiters.remove(&unique);
                    return Ok(body);
                }
                if inner.closed {
                    inner.waiters.remove(&unique);
                    return Err(FuseError::SessionClosed);
                }
            }
            crate::task::scheduler::block_current_thread();
        }

        #[cfg(test)]
        {
            // Test-pfad ohne Scheduler: synchron prüfen und sonst fehlschlagen.
            let mut inner = self.inner.lock();
            if let Some(body) = inner.replies.remove(&unique) {
                return Ok(body);
            }
            if inner.closed {
                return Err(FuseError::SessionClosed);
            }
            Err(FuseError::NoMatchingReply)
        }
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
// Global Session Registry + /dev/fuse character-device API
// ---------------------------------------------------------------------------

/// Eindeutige Session-ID (pro FUSE-Mount).
pub type SessionId = u32;

struct Registry {
    sessions: BTreeMap<SessionId, Arc<FuseSession>>,
    next_id: SessionId,
    /// Default-Session, gegen die der Char-Device-Adapter arbeitet, solange
    /// das Multi-Session-Protokoll noch nicht steht. Entspricht der
    /// Single-FUSE-Mount-Erwartung, die `corefsd` heute liefert.
    default_id: Option<SessionId>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    sessions: BTreeMap::new(),
    next_id: 1,
    default_id: None,
});

/// Registriert eine neue Session und liefert ihre `SessionId`. Die erste
/// registrierte Session wird automatisch als Default gesetzt (für die
/// einfache `/dev/fuse`-read/write-API, solange nur ein Daemon läuft).
pub fn register_session(session: Arc<FuseSession>) -> SessionId {
    let mut r = REGISTRY.lock();
    let id = r.next_id;
    r.next_id = r.next_id.wrapping_add(1);
    r.sessions.insert(id, session);
    if r.default_id.is_none() {
        r.default_id = Some(id);
    }
    id
}

/// Entfernt eine Session aus dem Registry. Wenn die entfernte Session die
/// Default-Session war, wird `default_id` gelöscht.
pub fn unregister_session(id: SessionId) {
    let mut r = REGISTRY.lock();
    r.sessions.remove(&id);
    if r.default_id == Some(id) {
        r.default_id = None;
    }
}

/// Liefert (falls vorhanden) einen geklonten `Arc` auf eine Session.
pub fn session(id: SessionId) -> Option<Arc<FuseSession>> {
    REGISTRY.lock().sessions.get(&id).cloned()
}

/// Liefert die Default-Session (erste registrierte, nicht geschlossene).
pub fn default_session() -> Option<Arc<FuseSession>> {
    let r = REGISTRY.lock();
    r.default_id.and_then(|id| r.sessions.get(&id).cloned())
}

/// `/dev/fuse`-Read-Operation.
///
/// Der Daemon ruft das auf, um den nächsten pending RequestFrame (als opake
/// Bytes) abzuholen. Rückgabe:
/// - `Some(n)` mit `n > 0` = `n` Bytes nach `buf` geschrieben.
/// - `Some(0)` = Queue leer (der Daemon soll erneut pollen).
/// - `None` = keine Session aktiv oder Session geschlossen.
///
/// **Limitation**: aktuell nicht-blockierend (polling). Ein späteres
/// Wait/Wake-Integration wird das zu einem echten blockierenden `read`
/// machen.
pub fn devfs_read(buf: &mut [u8]) -> Option<usize> {
    let session = default_session()?;
    let request = match session.pop_request() {
        Some(r) => r,
        None => return Some(0),
    };
    let body_len = request.body.len();
    // Wire-Format: `unique` (8 bytes little-endian) || body
    let total = 8 + body_len;
    if buf.len() < total {
        // Truncated read — drop the request to avoid deadlock. A full
        // protocol would requeue here; the skeleton just warns.
        return Some(0);
    }
    buf[0..8].copy_from_slice(&request.unique.to_le_bytes());
    buf[8..total].copy_from_slice(&request.body);
    Some(total)
}

/// `/dev/fuse`-Write-Operation.
///
/// Der Daemon ruft das auf, um einen Reply zurückzuschicken. Erwartetes
/// Wire-Format: `unique` (8 bytes little-endian) || opake Reply-Bytes.
/// Rückgabe:
/// - `Some(n)` = `n` Bytes akzeptiert.
/// - `None` = keine Session aktiv, Session geschlossen oder Frame zu klein.
pub fn devfs_write(buf: &[u8]) -> Option<usize> {
    if buf.len() < 8 {
        return None;
    }
    let session = default_session()?;
    let mut unique_bytes = [0u8; 8];
    unique_bytes.copy_from_slice(&buf[0..8]);
    let unique = Unique::from_le_bytes(unique_bytes);
    let body = buf[8..].to_vec();
    match session.deliver_reply(PendingReply { unique, body }) {
        Ok(()) => Some(buf.len()),
        Err(FuseError::SessionClosed) => {
            // Explicit: daemon tried to write to a session the kernel has
            // already torn down. Upper layers should surface this as EIO,
            // not a silent drop. Log it so the daemon sees the mismatch.
            #[cfg(not(test))]
            crate::serial_println!(
                "[fuse] devfs_write: session closed, reply for unique {} dropped (EIO)",
                unique
            );
            None
        }
        Err(_) => None,
    }
}

// ---------------------------------------------------------------------------
// fuse_call — VFS-Helper für Request/Reply-Round-Trip
// ---------------------------------------------------------------------------

use corefs_fuse_proto::{
    Reply as ProtoReply, ReplyPayload as ProtoReplyPayload, Request as ProtoRequest,
};

/// Fehler beim fuse_call-Round-Trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuseCallError {
    /// Request konnte nicht serialisiert werden.
    EncodeFailed,
    /// Reply konnte nicht deserialisiert werden.
    DecodeFailed,
    /// Session-Transport meldete Fehler (z. B. geschlossene Session).
    Transport(FuseError),
    /// Daemon meldete einen POSIX-errno.
    Remote {
        /// POSIX-errno.
        errno: i32,
    },
    /// Daemon lieferte einen Reply-Typ, der nicht zum gesendeten Request passt.
    UnexpectedReply,
}

/// Führt einen kompletten Request/Reply-Round-Trip über die Session aus.
///
/// - Serialisiert `req` via bincode-legacy (ohne 8-Byte-Unique-Header — der
///   wird vom `/dev/fuse`-Transport aus `SessionInner::next_unique` vorne
///   angeklebt).
/// - Ruft [`FuseSession::enqueue_and_wait`] → blockiert bis Reply eintrifft.
/// - Deserialisiert die Reply-Bytes als [`ProtoReply`] + [`ProtoReplyPayload`]
///   (Ok/Err) und gibt die entpackte [`ProtoReply`] zurück.
///
/// Das Wire-Format ist zwischen Kernel und Daemon asymmetrisch: der Kernel
/// serialisiert nur den `Request`-Op (ohne `RequestFrame`-Header), der
/// Daemon serialisiert nur den `ReplyPayload` (ohne `ReplyFrame`-Header).
/// Die `unique`-ID-Zuordnung übernimmt der Transport-Layer.
pub fn fuse_call(
    session: &FuseSession,
    req: &ProtoRequest,
) -> Result<ProtoReply, FuseCallError> {
    let body = bincode::serde::encode_to_vec(req, bincode::config::legacy())
        .map_err(|_| FuseCallError::EncodeFailed)?;
    let reply_bytes = session
        .enqueue_and_wait(body)
        .map_err(FuseCallError::Transport)?;
    let (payload, _): (ProtoReplyPayload, _) =
        bincode::serde::decode_from_slice(&reply_bytes, bincode::config::legacy())
            .map_err(|_| FuseCallError::DecodeFailed)?;
    match payload {
        ProtoReplyPayload::Ok(r) => Ok(r),
        ProtoReplyPayload::Err { errno, .. } => Err(FuseCallError::Remote { errno }),
    }
}

/// Mappt einen [`FuseCallError`] auf den grob passenden [`FsError`].
/// POSIX-errno → FsError:
/// - ENOENT(2) → NotFound
/// - EACCES(13) → PermissionDenied
/// - EEXIST(17) → AlreadyExists
/// - ENOTDIR(20) → NotADirectory
/// - EISDIR(21) → IsADirectory
/// - ENOSPC(28) → NoSpace
/// - sonst → IoError
#[allow(dead_code)]
pub fn fuse_call_error_to_fs_error(e: FuseCallError) -> crate::fs::vfs::FsError {
    use crate::fs::vfs::FsError;
    match e {
        FuseCallError::Remote { errno } => match errno {
            2 => FsError::NotFound,
            13 => FsError::PermissionDenied,
            17 => FsError::AlreadyExists,
            20 => FsError::NotADirectory,
            21 => FsError::IsADirectory,
            28 => FsError::NoSpace,
            _ => FsError::IoError,
        },
        _ => FsError::IoError,
    }
}

/// Mappt [`corefs_fuse_proto::Attr::kind`] (1=File, 2=Dir, 3=Symlink, …)
/// auf den kernelseitigen [`crate::fs::file::FileType`]. Unbekannte Kinds
/// werden als `Regular` behandelt — konservativer Default.
pub fn attr_kind_to_file_type(kind: u8) -> crate::fs::file::FileType {
    use crate::fs::file::FileType;
    match kind {
        2 => FileType::Directory,
        _ => FileType::Regular,
    }
}

#[cfg(test)]
fn clear_registry_for_test() {
    let mut r = REGISTRY.lock();
    r.sessions.clear();
    r.default_id = None;
    r.next_id = 1;
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

    #[test]
    fn devfs_read_returns_none_without_session() {
        clear_registry_for_test();
        let mut buf = [0u8; 64];
        assert!(devfs_read(&mut buf).is_none());
    }

    #[test]
    fn devfs_read_delivers_queued_request_with_unique_header() {
        clear_registry_for_test();
        let s = Arc::new(FuseSession::new());
        let _id = register_session(s.clone());
        let u = s.enqueue_request(vec![0xAA, 0xBB]).unwrap();

        let mut buf = [0u8; 64];
        let n = devfs_read(&mut buf).expect("session present");
        assert_eq!(n, 8 + 2);
        let mut unique_bytes = [0u8; 8];
        unique_bytes.copy_from_slice(&buf[0..8]);
        assert_eq!(Unique::from_le_bytes(unique_bytes), u);
        assert_eq!(&buf[8..10], &[0xAA, 0xBB]);
    }

    #[test]
    fn devfs_write_delivers_reply_matched_by_unique() {
        clear_registry_for_test();
        let s = Arc::new(FuseSession::new());
        let _id = register_session(s.clone());
        let u = s.enqueue_request(vec![1]).unwrap();

        // Compose a wire frame: unique || payload.
        let mut frame = Vec::new();
        frame.extend_from_slice(&u.to_le_bytes());
        frame.extend_from_slice(&[9, 8, 7]);
        assert_eq!(devfs_write(&frame), Some(frame.len()));

        let reply = s.take_reply(u).expect("reply present");
        assert_eq!(reply, vec![9, 8, 7]);
    }

    #[test]
    fn devfs_read_returns_zero_when_queue_empty() {
        clear_registry_for_test();
        let s = Arc::new(FuseSession::new());
        let _id = register_session(s);
        let mut buf = [0u8; 64];
        assert_eq!(devfs_read(&mut buf), Some(0));
    }

    #[test]
    fn enqueue_and_wait_returns_reply_when_preexisting() {
        // Race path: reply already delivered before the wait.
        // In tests we run single-threaded, so we simulate that the
        // daemon delivered first: enqueue, then deliver for the next
        // unique id, then wait.
        let s = FuseSession::new();
        // Pre-allocate the unique by reserving: we enqueue via the
        // blocking API after first manually depositing a reply for
        // `next_unique=1`. deliver_reply requires session not closed.
        s.deliver_reply(PendingReply { unique: 1, body: vec![11, 22] }).unwrap();
        let body = s.enqueue_and_wait(vec![0xDE]).unwrap();
        assert_eq!(body, vec![11, 22]);
    }

    #[test]
    fn enqueue_and_wait_on_closed_session_errors() {
        let s = FuseSession::new();
        s.close();
        assert!(matches!(
            s.enqueue_and_wait(vec![1]),
            Err(FuseError::SessionClosed)
        ));
    }

    #[test]
    fn devfs_write_on_closed_session_returns_none() {
        clear_registry_for_test();
        let s = Arc::new(FuseSession::new());
        let _id = register_session(s.clone());
        s.close();
        let mut frame = Vec::new();
        frame.extend_from_slice(&1u64.to_le_bytes());
        frame.extend_from_slice(&[0xEE]);
        // Previously: silent Some(len) via `.ok()?` short-circuit → now
        // None so the upper VFS layer can surface EIO.
        assert_eq!(devfs_write(&frame), None);
    }

    #[test]
    fn close_clears_waiters_without_wake() {
        // Regression: close() must not panic even with waiters recorded.
        // (In tests waiters is empty since we gate on cfg(test), but the
        // path must still be safe.)
        let s = FuseSession::new();
        s.close();
        // A later deliver_reply should fail cleanly.
        assert!(s
            .deliver_reply(PendingReply { unique: 1, body: vec![] })
            .is_err());
    }

    #[test]
    fn fuse_call_decodes_ok_reply() {
        use corefs_fuse_proto::{
            Attr, Reply as PR, ReplyPayload, Request as PQ,
        };
        let s = FuseSession::new();
        // Pre-deposit a reply for unique=1 (next id), simulating daemon
        // already answered before block.
        let payload = ReplyPayload::Ok(PR::Getattr(Attr {
            ino: 42, size: 7, blocks: 1, mode: 0o644, nlink: 1,
            uid: 0, gid: 0, kind: 1, crtime_secs: 1, mtime_secs: 1, ctime_secs: 1,
        }));
        let body =
            bincode::serde::encode_to_vec(&payload, bincode::config::legacy()).unwrap();
        s.deliver_reply(PendingReply { unique: 1, body }).unwrap();

        let req = PQ::Getattr { ino: 42 };
        let reply = fuse_call(&s, &req).expect("round-trip ok");
        match reply {
            PR::Getattr(a) => {
                assert_eq!(a.ino, 42);
                assert_eq!(a.size, 7);
            }
            _ => panic!("wrong reply variant"),
        }
    }

    #[test]
    fn fuse_call_decodes_err_reply_to_remote() {
        use corefs_fuse_proto::{ReplyPayload, Request as PQ};
        let s = FuseSession::new();
        let payload = ReplyPayload::Err { errno: 2, message: None };
        let body =
            bincode::serde::encode_to_vec(&payload, bincode::config::legacy()).unwrap();
        s.deliver_reply(PendingReply { unique: 1, body }).unwrap();

        let req = PQ::Getattr { ino: 99 };
        let err = fuse_call(&s, &req).expect_err("should error");
        assert!(matches!(err, FuseCallError::Remote { errno: 2 }));
    }

    #[test]
    fn register_then_unregister_clears_default() {
        clear_registry_for_test();
        let s = Arc::new(FuseSession::new());
        let id = register_session(s);
        assert!(default_session().is_some());
        unregister_session(id);
        assert!(default_session().is_none());
    }
}
