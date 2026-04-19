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
        let Some(pos) = self
            .watches
            .iter()
            .position(|watch| {
                watch.tid == tid
                    && watch.reply_pipe_name == reply_pipe_name
                    && watch.id == watch_id
            })
        else {
            return false;
        };
        self.watches.remove(pos);
        true
    }

    pub fn matching_watch_ids(
        &self,
        scope: Scope,
        canonical_path: &str,
    ) -> Vec<(u32, &str, u32, Scope)> {
        let mut matches = Vec::new();
        for watch in &self.watches {
            if watch.scope == scope
                && path_matches_prefix(canonical_path, watch.canonical_prefix.as_str())
            {
                matches.push((watch.tid, watch.reply_pipe_name.as_str(), watch.id, watch.scope));
            }
        }
        matches
    }
}

fn main() {
    anyos_std::println!("[confd] starting central configuration registry");
    let mut lifecycle = connect_lifecycle();
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.notify_starting();
    }

    if !libdb_client::init() {
        anyos_std::println!("[confd] failed to load libdb.so");
        notify_failed(&mut lifecycle, "libdb_init_failed");
        return;
    }

    anyos_std::fs::mkdir(DB_DIR);
    let db_preexisting = ensure_db_file();
    log_db_file_state("before-open");

    let db = match libdb_client::Database::open(DB_PATH) {
        Some(db) => db,
        None => {
            anyos_std::println!("[confd] failed to open database at {}", DB_PATH);
            notify_failed(&mut lifecycle, "database_open_failed");
            return;
        }
    };

    schema::init_tables(&db);
    log_db_file_state("after-init");
    let entries = schema::load_entries(&db);
    let next_audit_seq = schema::load_next_audit_seq(&db);
    let registry_rows = schema::count_rows(&db, "registry");
    let audit_rows = schema::count_rows(&db, "audit");
    let schema_rows = schema::count_rows(&db, "schemas");
    if registry_rows.is_some() && audit_rows.is_some() && schema_rows.is_some() {
        anyos_std::println!(
            "[confd] db integrity check OK (registry={}, audit={}, schemas={})",
            registry_rows.unwrap_or(0),
            audit_rows.unwrap_or(0),
            schema_rows.unwrap_or(0),
        );
    } else {
        anyos_std::println!(
            "[confd] db integrity check WARNING (registry={:?}, audit={:?}, schemas={:?})",
            registry_rows,
            audit_rows,
            schema_rows,
        );
    }
    let mut state = ConfState::new(entries, next_audit_seq);

    let old_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        anyos_std::ipc::pipe_close(old_pipe);
    }

    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        anyos_std::println!("[confd] failed to create '{}' pipe", PIPE_NAME);
        notify_failed(&mut lifecycle, "pipe_create_failed");
        return;
    }

    anyos_std::println!(
        "[confd] ready (pipe='{}', db='{}', existed={}, entries={}, registry_rows={:?}, audit_rows={:?}, schema_rows={:?}, next_audit_seq={})",
        PIPE_NAME,
        DB_PATH,
        db_preexisting,
        state.entries.len(),
        registry_rows,
        audit_rows,
        schema_rows,
        state.next_audit_seq,
    );
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.set_detail("pipe", PIPE_NAME);
        let _ = svc.set_detail("db", DB_PATH);
        let _ = svc.set_detail("entries", &alloc::format!("{}", state.entries.len()));
        let _ = svc.notify_ready();
    }

    let mut pipe_buf = [0u8; 4096];
    loop {
        let active = ipc::handle_requests(&db, &mut state, pipe_id, &mut pipe_buf);
        anyos_std::process::sleep(if active { 20 } else { 100 });
    }
}

fn ensure_db_file() -> bool {
    let probe = anyos_std::fs::open(DB_PATH, 0);
    if probe != u32::MAX {
        anyos_std::fs::close(probe);
        anyos_std::println!("[confd] database file found at {}", DB_PATH);
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
    anyos_std::println!("[confd] created new database file at {}", DB_PATH);
    false
}

fn log_db_file_state(stage: &str) {
    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(DB_PATH, &mut stat_buf) != 0 {
        anyos_std::println!("[confd] db {} stat FAILED path={}", stage, DB_PATH);
        return;
    }

    anyos_std::println!(
        "[confd] db {} type={} size={} uid={} gid={} mode=0x{:x} mtime={}",
        stage,
        stat_buf[0],
        stat_buf[1],
        stat_buf[3],
        stat_buf[4],
        stat_buf[5],
        stat_buf[6],
    );
}

fn connect_lifecycle() -> Option<ServiceLifecycle> {
    for _ in 0..100 {
        match ServiceLifecycle::connect("confd") {
            Ok(svc) => return Some(svc),
            Err(_) => anyos_std::process::sleep(20),
        }
    }
    anyos_std::println!("[confd] WARNING - AMID not reachable for service lifecycle");
    None
}

fn notify_failed(lifecycle: &mut Option<ServiceLifecycle>, reason: &str) {
    if let Some(svc) = lifecycle.as_mut() {
        let _ = svc.notify_failed(reason);
    }
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
