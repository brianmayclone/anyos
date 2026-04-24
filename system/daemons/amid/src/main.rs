//! amid — Anywhere Management Interface daemon.
//!
//! AMI v1 is a shared runtime state service. Other daemons publish their own
//! state into AMI instead of having amid rediscover it by polling syscalls.

#![no_std]
#![no_main]

mod ipc;
mod schema;

use alloc::string::String;
use alloc::vec::Vec;

anyos_std::entry!(main);

pub(crate) const DB_PATH: &str = ":memory:";
pub(crate) const PIPE_NAME: &str = "ami";
const THREAD_ENTRY_SIZE: usize = 80;
const MAX_THREADS: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum AmiValue {
    String(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone)]
pub(crate) struct StateEntry {
    pub key: String,
    pub value: AmiValue,
    pub version: u64,
    pub updated_at: u64,
    pub owner: String,
}

#[derive(Clone)]
pub(crate) struct Watch {
    pub id: u32,
    pub tid: u32,
    pub prefix: String,
}

#[derive(Clone)]
pub(crate) struct ClientIdentity {
    pub tid: u32,
    pub service: String,
}

pub(crate) struct AmiState {
    pub entries: Vec<StateEntry>,
    pub watches: Vec<Watch>,
    pub clients: Vec<ClientIdentity>,
    pub next_watch_id: u32,
    pub pending_request: String,
}

impl AmiState {
    fn new(entries: Vec<StateEntry>) -> Self {
        Self {
            entries,
            watches: Vec::new(),
            clients: Vec::new(),
            next_watch_id: 1,
            pending_request: String::new(),
        }
    }

    pub fn set_service(&mut self, tid: u32, service: &str) {
        if let Some(client) = self.clients.iter_mut().find(|client| client.tid == tid) {
            client.service.clear();
            client.service.push_str(service);
            return;
        }
        self.clients.push(ClientIdentity {
            tid,
            service: String::from(service),
        });
    }

    pub fn service_for(&self, tid: u32) -> Option<&str> {
        self.clients
            .iter()
            .find(|client| client.tid == tid)
            .map(|client| client.service.as_str())
    }

    pub fn find_entry(&self, key: &str) -> Option<&StateEntry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    pub fn find_entry_mut(&mut self, key: &str) -> Option<&mut StateEntry> {
        self.entries.iter_mut().find(|entry| entry.key == key)
    }

    pub fn remove_entry(&mut self, key: &str) -> Option<StateEntry> {
        let pos = self.entries.iter().position(|entry| entry.key == key)?;
        Some(self.entries.remove(pos))
    }

    pub fn upsert_entry(
        &mut self,
        key: &str,
        value: AmiValue,
        owner: &str,
        now: u64,
    ) -> (StateEntry, bool) {
        if let Some(entry) = self.find_entry_mut(key) {
            if entry.value == value {
                return (entry.clone(), false);
            }
            entry.value = value;
            entry.owner.clear();
            entry.owner.push_str(owner);
            entry.version = entry.version.saturating_add(1);
            entry.updated_at = now;
            return (entry.clone(), true);
        }

        let entry = StateEntry {
            key: String::from(key),
            value,
            version: 1,
            updated_at: now,
            owner: String::from(owner),
        };
        self.entries.push(entry.clone());
        (entry, true)
    }

    pub fn add_watch(&mut self, tid: u32, prefix: &str) -> u32 {
        const MAX_WATCHES_PER_TID: usize = 32;

        let mut existing_for_tid = 0usize;
        for watch in &self.watches {
            if watch.tid == tid {
                existing_for_tid += 1;
            }
        }
        if existing_for_tid >= MAX_WATCHES_PER_TID {
            return 0;
        }

        let id = self.next_watch_id;
        self.next_watch_id = self.next_watch_id.wrapping_add(1).max(1);
        self.watches.push(Watch {
            id,
            tid,
            prefix: String::from(prefix),
        });
        id
    }

    pub fn remove_watch(&mut self, tid: u32, watch_id: u32) -> bool {
        if let Some(pos) = self
            .watches
            .iter()
            .position(|watch| watch.tid == tid && watch.id == watch_id)
        {
            self.watches.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn matching_watch_ids(&self, key: &str) -> Vec<(u32, u32)> {
        let mut matches = Vec::new();
        for watch in &self.watches {
            if key.starts_with(watch.prefix.as_str()) {
                matches.push((watch.tid, watch.id));
            }
        }
        matches
    }

    pub fn remove_client(&mut self, tid: u32) {
        self.clients.retain(|client| client.tid != tid);
        self.watches.retain(|watch| watch.tid != tid);
    }

    pub fn prune_dead_clients(&mut self) {
        self.clients.retain(|client| is_tid_alive(client.tid));
        self.watches.retain(|watch| is_tid_alive(watch.tid));
    }

    pub fn list_prefix(&self, prefix: &str) -> Vec<StateEntry> {
        let mut items = Vec::new();
        for entry in &self.entries {
            if entry.key.starts_with(prefix) {
                items.push(entry.clone());
            }
        }
        items.sort_by(|a, b| a.key.cmp(&b.key));
        items
    }
}

fn main() {
    anyos_std::println!("amid: starting Anywhere Management Interface v1");

    if !libdb_client::init() {
        anyos_std::println!("amid: failed to load libdb.so");
        return;
    }

    let db = match libdb_client::Database::open_in_memory() {
        Some(db) => db,
        None => {
            anyos_std::println!("amid: failed to open in-memory database");
            return;
        }
    };

    schema::init_tables(&db);
    let entries = schema::load_entries(&db);
    let mut state = AmiState::new(entries);

    let old_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        anyos_std::ipc::pipe_close(old_pipe);
    }

    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        anyos_std::println!("amid: failed to create '{}' pipe", PIPE_NAME);
        return;
    }

    anyos_std::println!(
        "amid: ready (pipe='{}', db='{}', entries={})",
        PIPE_NAME,
        DB_PATH,
        state.entries.len()
    );

    let mut pipe_buf = [0u8; 4096];
    loop {
        let active = ipc::handle_requests(&db, &mut state, pipe_id, &mut pipe_buf);
        state.prune_dead_clients();
        anyos_std::process::sleep(if active { 20 } else { 100 });
    }
}

fn is_tid_alive(tid: u32) -> bool {
    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX {
        return false;
    }

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let entry_tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if entry_tid == tid {
            let state = buf[off + 5];
            return state <= 2;
        }
    }
    false
}
