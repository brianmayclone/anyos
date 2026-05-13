//! confd — central configuration registry daemon.
//!
//! The daemon exposes a Windows-Registry-inspired configuration store over a
//! named pipe while keeping the namespace structured and auditable:
//! - `system/<path>` for machine-wide configuration
//! - `user/<uid>/<path>` for per-user overrides and app settings

#![no_std]
#![no_main]

mod ipc;
mod schema;

use alloc::string::String;
use alloc::vec::Vec;
use libsvc::ServiceLifecycle;

anyos_std::entry!(main);

pub(crate) const DB_DIR: &str = "/System/sysdb";
pub(crate) const DB_PATH: &str = "/System/sysdb/config.db";
pub(crate) const PIPE_NAME: &str = "confd";
const THREAD_ENTRY_SIZE: usize = 80;
const MAX_THREADS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    System = 1,
    User = 2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Directory = 1,
    Value = 2,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ConfValue {
    String(String),
    Int(i64),
    Bool(bool),
    ExternalRef(String),
}

#[derive(Clone)]
pub(crate) struct RegistryEntry {
    pub canonical_path: String,
    pub logical_path: String,
    pub scope: Scope,
    pub owner_uid: u16,
    pub kind: NodeKind,
    pub value: Option<ConfValue>,
    pub version: u64,
    pub updated_at: u64,
    pub writer_uid: u16,
    pub writer_name: String,
}

#[derive(Clone)]
pub(crate) struct Watch {
    pub id: u32,
    pub tid: u32,
    pub reply_pipe_name: String,
    pub scope: Scope,
    pub canonical_prefix: String,
}

#[derive(Clone)]
pub(crate) struct ClientInfo {
    pub tid: u32,
    pub uid: u16,
    pub reply_pipe_name: String,
    pub name: String,
}

pub(crate) struct ConfState {
    pub entries: Vec<RegistryEntry>,
    pub watches: Vec<Watch>,
    pub clients: Vec<ClientInfo>,
    pub next_watch_id: u32,
    pub next_audit_seq: u64,
    pub pending_request: String,
}

impl ConfState {
    fn new(entries: Vec<RegistryEntry>, next_audit_seq: u64) -> Self {
        Self {
            entries,
            watches: Vec::new(),
            clients: Vec::new(),
            next_watch_id: 1,
            next_audit_seq,
            pending_request: String::new(),
        }
    }

    pub fn set_client(&mut self, tid: u32, reply_pipe_name: &str, uid: u16, name: &str) {
        if let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.tid == tid && client.reply_pipe_name == reply_pipe_name)
        {
            client.uid = uid;
            client.name.clear();
            client.name.push_str(name);
            return;
        }

        self.clients.push(ClientInfo {
            tid,
            uid,
            reply_pipe_name: String::from(reply_pipe_name),
            name: String::from(name),
        });
    }

    pub fn client_name(&self, tid: u32, reply_pipe_name: &str) -> &str {
        self.clients
            .iter()
            .find(|client| client.tid == tid && client.reply_pipe_name == reply_pipe_name)
            .map(|client| client.name.as_str())
            .unwrap_or("unknown")
    }

    pub fn find_entry(&self, canonical_path: &str) -> Option<&RegistryEntry> {
        self.entries
            .iter()
            .find(|entry| entry.canonical_path == canonical_path)
    }

    pub fn find_entry_mut(&mut self, canonical_path: &str) -> Option<&mut RegistryEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.canonical_path == canonical_path)
    }

    pub fn upsert_dir(
        &mut self,
        scope: Scope,
        owner_uid: u16,
        canonical_path: &str,
        logical_path: &str,
        writer_uid: u16,
        writer_name: &str,
        now: u64,
    ) -> (RegistryEntry, bool) {
        if let Some(entry) = self.find_entry_mut(canonical_path) {
            if entry.kind == NodeKind::Directory {
                return (entry.clone(), false);
            }
            entry.kind = NodeKind::Directory;
            entry.value = None;
            entry.writer_uid = writer_uid;
            entry.writer_name.clear();
            entry.writer_name.push_str(writer_name);
            entry.version = entry.version.saturating_add(1);
            entry.updated_at = now;
            return (entry.clone(), true);
        }

        let entry = RegistryEntry {
            canonical_path: String::from(canonical_path),
            logical_path: String::from(logical_path),
            scope,
            owner_uid,
            kind: NodeKind::Directory,
            value: None,
            version: 1,
            updated_at: now,
            writer_uid,
            writer_name: String::from(writer_name),
        };
        self.entries.push(entry.clone());
        (entry, true)
    }

    pub fn upsert_value(
        &mut self,
        scope: Scope,
        owner_uid: u16,
        canonical_path: &str,
        logical_path: &str,
        value: ConfValue,
        writer_uid: u16,
        writer_name: &str,
        now: u64,
    ) -> (RegistryEntry, bool) {
        if let Some(entry) = self.find_entry_mut(canonical_path) {
            if entry.kind == NodeKind::Value && entry.value.as_ref() == Some(&value) {
                return (entry.clone(), false);
            }
            entry.kind = NodeKind::Value;
            entry.value = Some(value);
            entry.writer_uid = writer_uid;
            entry.writer_name.clear();
            entry.writer_name.push_str(writer_name);
            entry.version = entry.version.saturating_add(1);
            entry.updated_at = now;
            return (entry.clone(), true);
        }

        let entry = RegistryEntry {
            canonical_path: String::from(canonical_path),
            logical_path: String::from(logical_path),
            scope,
            owner_uid,
            kind: NodeKind::Value,
            value: Some(value),
            version: 1,
            updated_at: now,
            writer_uid,
            writer_name: String::from(writer_name),
        };
        self.entries.push(entry.clone());
        (entry, true)
    }

    pub fn remove_entry(&mut self, canonical_path: &str) -> Option<RegistryEntry> {
        let pos = self
            .entries
            .iter()
            .position(|entry| entry.canonical_path == canonical_path)?;
        Some(self.entries.remove(pos))
    }

    pub fn list_prefix(&self, canonical_prefix: &str) -> Vec<RegistryEntry> {
        let mut items = Vec::new();
        for entry in &self.entries {
            if path_matches_prefix(&entry.canonical_path, canonical_prefix) {
                items.push(entry.clone());
            }
        }
        items.sort_by(|a, b| a.canonical_path.cmp(&b.canonical_path));
        items
    }

    pub fn list_direct_children(&self, canonical_prefix: &str) -> Vec<RegistryEntry> {
        let mut items = Vec::new();
        for entry in &self.entries {
            if is_direct_child_path(&entry.canonical_path, canonical_prefix) {
                items.push(entry.clone());
            }
        }
        items.sort_by(|a, b| a.canonical_path.cmp(&b.canonical_path));
        items
    }

    pub fn add_watch(
        &mut self,
        tid: u32,
        reply_pipe_name: &str,
        scope: Scope,
        canonical_prefix: &str,
    ) -> u32 {
        const MAX_WATCHES_PER_CLIENT: usize = 32;

        let mut existing_for_client = 0usize;
        for watch in &self.watches {
            if watch.tid == tid && watch.reply_pipe_name == reply_pipe_name {
                existing_for_client += 1;
            }
        }
        if existing_for_client >= MAX_WATCHES_PER_CLIENT {
            return 0;
        }

        let id = self.next_watch_id;
        self.next_watch_id = self.next_watch_id.wrapping_add(1).max(1);
        self.watches.push(Watch {
            id,
            tid,
            reply_pipe_name: String::from(reply_pipe_name),
            scope,
            canonical_prefix: String::from(canonical_prefix),
        });
        id
    }

    pub fn remove_watch(&mut self, tid: u32, reply_pipe_name: &str, watch_id: u32) -> bool {
        let Some(pos) = self.watches.iter().position(|watch| {
            watch.tid == tid && watch.reply_pipe_name == reply_pipe_name && watch.id == watch_id
        }) else {
            return false;
        };
        self.watches.remove(pos);
        true
    }

    pub fn matching_watch_ids(
        &self,
        scope: Scope,
        canonical_path: &str,
    ) -> Vec<(u32, String, u32, Scope)> {
        let mut matches = Vec::new();
        for watch in &self.watches {
            if watch.scope == scope
                && path_matches_prefix(canonical_path, watch.canonical_prefix.as_str())
            {
                matches.push((
                    watch.tid,
                    watch.reply_pipe_name.clone(),
                    watch.id,
                    watch.scope,
                ));
            }
        }
        matches
    }

    pub fn remove_client(&mut self, tid: u32, reply_pipe_name: &str) {
        self.clients
            .retain(|client| client.tid != tid || client.reply_pipe_name != reply_pipe_name);
        self.watches
            .retain(|watch| watch.tid != tid || watch.reply_pipe_name != reply_pipe_name);
    }

    pub fn prune_dead_clients(&mut self) {
        if self.clients.is_empty() && self.watches.is_empty() {
            return;
        }
        let alive = match snapshot_alive_tids() {
            Some(set) => set,
            None => return,
        };
        self.clients.retain(|client| alive.contains(client.tid));
        self.watches.retain(|watch| alive.contains(watch.tid));
    }
}

fn main() {
    let start_ms = anyos_std::sys::uptime_ms();
    // Non-blocking: only a single attempt. Real retry happens in the main loop.
    let mut lifecycle = ServiceLifecycle::connect("confd").ok();
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.notify_starting();
    }

    if !libdb_client::init() {
        anyos_std::println!("[confd] failed to load libdb.so");
        notify_failed(&mut lifecycle, "libdb_init_failed");
        return;
    }

    anyos_std::fs::mkdir(DB_DIR);
    ensure_db_file();

    let db = match libdb_client::Database::open(DB_PATH) {
        Some(db) => db,
        None => {
            anyos_std::println!("[confd] failed to open database at {}", DB_PATH);
            notify_failed(&mut lifecycle, "database_open_failed");
            return;
        }
    };

    let db_open_ms = anyos_std::sys::uptime_ms();
    schema::init_tables(&db);
    let schema_ms = anyos_std::sys::uptime_ms();
    let entries = schema::load_entries(&db);
    let entries_ms = anyos_std::sys::uptime_ms();
    let next_audit_seq = schema::load_next_audit_seq(&db);
    let audit_ms = anyos_std::sys::uptime_ms();
    let mut state = ConfState::new(entries, next_audit_seq);

    let old_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 && old_pipe != u32::MAX {
        anyos_std::ipc::pipe_close(old_pipe);
    }

    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        anyos_std::println!("[confd] failed to create '{}' pipe", PIPE_NAME);
        notify_failed(&mut lifecycle, "pipe_create_failed");
        return;
    }
    let pipe_ms = anyos_std::sys::uptime_ms();

    anyos_std::println!(
        "[confd] startup: db={}ms schema={}ms entries={}ms audit={}ms pipe={}ms total={}ms entries={}",
        db_open_ms.saturating_sub(start_ms),
        schema_ms.saturating_sub(db_open_ms),
        entries_ms.saturating_sub(schema_ms),
        audit_ms.saturating_sub(entries_ms),
        pipe_ms.saturating_sub(audit_ms),
        pipe_ms.saturating_sub(start_ms),
        state.entries.len()
    );

    let entries_detail = alloc::format!("{}", state.entries.len());
    let mut ready_notified = false;
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.set_detail("pipe", PIPE_NAME);
        let _ = svc.set_detail("db", DB_PATH);
        let _ = svc.set_detail("entries", &entries_detail);
        if svc.notify_ready().is_ok() {
            ready_notified = true;
        }
    }

    let mut pipe_buf = [0u8; 4096];
    let mut retry_counter: u32 = 0;
    let mut prune_counter: u32 = 0;
    loop {
        let active = ipc::handle_requests(&db, &mut state, pipe_id, &mut pipe_buf);
        prune_counter = prune_counter.saturating_add(1);
        // Idle-Tick = 100ms, also prune alle ~5s. Bei aktivem Verkehr
        // (20ms) etwas haeufiger, ist aber nicht zeitkritisch.
        if prune_counter >= 50 {
            prune_counter = 0;
            state.prune_dead_clients();
        }

        if !ready_notified {
            retry_counter = retry_counter.saturating_add(1);
            // Keep boot readiness snappy if AMI was not reachable during the
            // first startup race. Idle iterations sleep for 100ms, active ones
            // for 20ms, so this retries at roughly 100ms cadence.
            let threshold = if active { 5 } else { 1 };
            if retry_counter >= threshold {
                retry_counter = 0;
                if lifecycle.is_none() {
                    if let Ok(svc) = ServiceLifecycle::connect("confd") {
                        lifecycle = Some(svc);
                        if let Some(svc) = lifecycle.as_mut() {
                            let _ = svc.notify_starting();
                        }
                    }
                }
                if let Some(svc) = lifecycle.as_mut() {
                    let _ = svc.set_detail("pipe", PIPE_NAME);
                    let _ = svc.set_detail("db", DB_PATH);
                    let _ = svc.set_detail("entries", &entries_detail);
                    if svc.notify_ready().is_ok() {
                        ready_notified = true;
                    }
                }
            }
        }

        anyos_std::process::sleep(if active { 20 } else { 100 });
    }
}

fn ensure_db_file() -> bool {
    let probe = anyos_std::fs::open(DB_PATH, 0);
    if probe != u32::MAX {
        anyos_std::fs::close(probe);
        return true;
    }

    let fd = anyos_std::fs::open(
        DB_PATH,
        anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC,
    );
    if fd == u32::MAX {
        anyos_std::println!("[confd] failed to create database file at {}", DB_PATH);
        return false;
    }
    anyos_std::fs::close(fd);
    false
}

fn notify_failed(lifecycle: &mut Option<ServiceLifecycle>, reason: &str) {
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.notify_failed(reason);
    }
}

pub(crate) struct AliveTids {
    tids: Vec<u32>,
}

impl AliveTids {
    pub fn contains(&self, tid: u32) -> bool {
        self.tids.iter().any(|t| *t == tid)
    }
}

fn snapshot_alive_tids() -> Option<AliveTids> {
    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX {
        return None;
    }
    let mut tids = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let state = buf[off + 5];
        if state > 2 {
            continue;
        }
        let entry_tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        tids.push(entry_tid);
    }
    Some(AliveTids { tids })
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }

    if let Some(rest) = path.strip_prefix(prefix) {
        return rest.starts_with('/');
    }

    false
}

fn is_direct_child_path(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return !path.is_empty() && !path.contains('/');
    }
    if path == prefix {
        return false;
    }
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    !rest.is_empty() && !rest.contains('/')
}
