// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Background mail sync worker.
//!
//! Runs in a separate thread via `Thread::spawn_with_stack()`.
//! Communicates with the UI thread through a shared `SyncState` structure
//! using a spinlock for Vec/String fields and atomic flags for status polling.
//! The UI thread polls every 500ms via `anyui::set_timer()`.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::mail::message::MessageSummary;
use crate::protocol::imap::{ImapClient, ImapFolder};
use crate::protocol::pop3::Pop3Client;
use crate::storage::account::{Account, Security};
use crate::storage::maildir;

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Messages delivered to the UI per batch.
const DELIVERY_BATCH: usize = 50;

/// Default number of messages to fetch per folder (lazy loading).
pub const LAZY_BATCH_SIZE: u32 = 200;

// ═══════════════════════════════════════════════════════════════════════════
// Sync Phase
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum SyncPhase {
    Idle = 0,
    Connecting = 1,
    SyncingFolders = 2,
    FetchingHeaders = 3,
    FetchingBodies = 4,
    Done = 5,
    Error = 6,
}

// ═══════════════════════════════════════════════════════════════════════════
// Result Types
// ═══════════════════════════════════════════════════════════════════════════

/// Folder list delivered from worker to UI.
pub struct FolderSyncResult {
    pub account_idx: usize,
    pub folders: Vec<ImapFolder>,
}

/// A batch of newly fetched messages.
pub struct MessageBatch {
    pub account_id: String,
    pub folder_name: String,
    pub messages: Vec<MessageSummary>,
    /// True if this is the last batch for this folder.
    pub is_final: bool,
    /// Total message count on server for this folder (for lazy loading).
    pub total_on_server: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared State
// ═══════════════════════════════════════════════════════════════════════════

pub struct SyncState {
    // ── Atomic flags (lock-free polling) ─────────────────────────
    pub phase: AtomicU32,
    pub cancel_requested: AtomicBool,
    pub folders_ready: AtomicBool,
    pub messages_ready: AtomicBool,
    pub status_ready: AtomicBool,
    pub worker_active: AtomicBool,
    lock: AtomicBool,

    // ── Input (UI -> Worker, set before thread spawn) ────────────
    pub accounts: Vec<Account>,
    pub base_dir: String,
    pub target_account_idx: Option<usize>,
    pub target_folder: Option<String>,
    pub uid_offset: u32,
    pub fetch_limit: u32,

    // ── Output (Worker -> UI, read by poll timer) ────────────────
    pub folder_results: Vec<FolderSyncResult>,
    pub message_batches: Vec<MessageBatch>,
    pub status_text: String,
    pub error_text: String,
    pub progress_current: u32,
    pub progress_total: u32,
}

impl SyncState {
    pub const fn new() -> Self {
        Self {
            phase: AtomicU32::new(0),
            cancel_requested: AtomicBool::new(false),
            folders_ready: AtomicBool::new(false),
            messages_ready: AtomicBool::new(false),
            status_ready: AtomicBool::new(false),
            worker_active: AtomicBool::new(false),
            lock: AtomicBool::new(false),
            accounts: Vec::new(),
            base_dir: String::new(),
            target_account_idx: None,
            target_folder: None,
            uid_offset: 0,
            fetch_limit: LAZY_BATCH_SIZE,
            folder_results: Vec::new(),
            message_batches: Vec::new(),
            status_text: String::new(),
            error_text: String::new(),
            progress_current: 0,
            progress_total: 0,
        }
    }

    pub fn acquire(&self) {
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    pub fn release(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Reset all output fields (call under lock).
    pub fn reset_output(&mut self) {
        self.folder_results.clear();
        self.message_batches.clear();
        self.status_text.clear();
        self.error_text.clear();
        self.progress_current = 0;
        self.progress_total = 0;
        self.folders_ready.store(false, Ordering::Relaxed);
        self.messages_ready.store(false, Ordering::Relaxed);
        self.status_ready.store(false, Ordering::Relaxed);
        self.cancel_requested.store(false, Ordering::Relaxed);
    }
}

static mut SYNC_STATE: SyncState = SyncState::new();

pub fn sync_state() -> &'static mut SyncState {
    unsafe { &mut SYNC_STATE }
}

// ═══════════════════════════════════════════════════════════════════════════
// Worker Thread Entry
// ═══════════════════════════════════════════════════════════════════════════

/// Entry point for the background sync thread.
/// Reads all input from SYNC_STATE, writes results back.
pub fn sync_worker_entry() {
    let ss = sync_state();
    ss.phase
        .store(SyncPhase::Connecting as u32, Ordering::Release);

    // Clone inputs from shared state (under lock)
    ss.acquire();
    let accounts = ss.accounts.clone();
    let base_dir = ss.base_dir.clone();
    let target_acct = ss.target_account_idx;
    let target_folder = ss.target_folder.clone();
    let uid_offset = ss.uid_offset;
    let fetch_limit = ss.fetch_limit;
    ss.release();

    // Determine which accounts to sync
    let (start, end) = match target_acct {
        Some(idx) => (idx, idx + 1),
        None => (0, accounts.len()),
    };

    for acct_idx in start..end {
        if ss.cancel_requested.load(Ordering::Relaxed) {
            break;
        }
        if acct_idx >= accounts.len() {
            break;
        }
        let account = &accounts[acct_idx];

        if account.is_imap() {
            sync_imap_async(
                account,
                acct_idx,
                &base_dir,
                &target_folder,
                uid_offset,
                fetch_limit,
            );
        } else {
            sync_pop3_async(account, acct_idx, &base_dir, uid_offset, fetch_limit);
        }
    }

    // Save contacts (on worker thread — file I/O is safe)
    crate::app().address_book.save();

    ss.phase.store(SyncPhase::Done as u32, Ordering::Release);
    ss.worker_active.store(false, Ordering::Release);
    // Thread has no return address on the stack (mmap zeroes it), so returning
    // would jump to RIP=0x0.  Kill this thread cleanly instead.
    anyos_std::process::kill(anyos_std::process::getpid());
}

// ═══════════════════════════════════════════════════════════════════════════
// IMAP Async Sync
// ═══════════════════════════════════════════════════════════════════════════

fn sync_imap_async(
    account: &Account,
    acct_idx: usize,
    base_dir: &str,
    target_folder: &Option<String>,
    uid_offset: u32,
    fetch_limit: u32,
) {
    let ss = sync_state();

    // ── Phase 1: Connect and Authenticate ────────────────────────
    set_worker_status(&alloc::format!(
        "Connecting to {}...",
        account.incoming_host
    ));

    let mut client = ImapClient::new();
    let use_tls = account.incoming_use_tls();
    if let Err(e) = client.connect(&account.incoming_host, account.incoming_port, use_tls) {
        set_worker_error(&alloc::format!("IMAP connect failed: {:?}", e));
        return;
    }

    // STARTTLS if needed
    if account.incoming_security == Security::StartTls {
        if let Err(e) = client.starttls() {
            set_worker_error(&alloc::format!("IMAP STARTTLS failed: {:?}", e));
            client.logout();
            return;
        }
    }

    // Login
    if let Err(e) = client.login(&account.incoming_user, &account.incoming_pass) {
        set_worker_error(&alloc::format!("IMAP login failed: {:?}", e));
        client.logout();
        return;
    }

    // ── Phase 2: Sync Folder Hierarchy ───────────────────────────
    ss.phase
        .store(SyncPhase::SyncingFolders as u32, Ordering::Release);
    set_worker_status(&alloc::format!("Syncing folders for {}...", account.email));

    if let Ok(folders) = client.list_folders() {
        // Ensure local directories for each folder
        maildir::ensure_dirs(base_dir, &account.id);
        for f in &folders {
            let folder_dir = alloc::format!(
                "{}/accounts/{}/{}",
                base_dir,
                account.id,
                sanitize_folder(&f.name)
            );
            anyos_std::fs::mkdir(&folder_dir);
            let msg_dir = alloc::format!("{}/messages", folder_dir);
            anyos_std::fs::mkdir(&msg_dir);
        }

        ss.acquire();
        ss.folder_results.push(FolderSyncResult {
            account_idx: acct_idx,
            folders,
        });
        ss.release();
        ss.folders_ready.store(true, Ordering::Release);
    }

    if ss.cancel_requested.load(Ordering::Relaxed) {
        client.logout();
        return;
    }

    // ── Phase 3: Fetch Messages ──────────────────────────────────
    let folders_to_sync: Vec<&str> = match target_folder {
        Some(ref f) => alloc::vec![f.as_str()],
        None => alloc::vec!["INBOX", "Sent", "Drafts", "Archive"],
    };

    ss.phase
        .store(SyncPhase::FetchingHeaders as u32, Ordering::Release);

    for folder_name in &folders_to_sync {
        if ss.cancel_requested.load(Ordering::Relaxed) {
            break;
        }

        let folder_status = match client.select(folder_name) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Load existing local index
        let idx_path = maildir::index_path(base_dir, &account.id, folder_name);
        let existing = maildir::load_index(&idx_path);
        let max_uid = if uid_offset > 0 {
            uid_offset
        } else {
            existing.iter().map(|m| m.uid).max().unwrap_or(0)
        };

        // Search for UIDs
        let search_query = if max_uid > 0 {
            alloc::format!("UID {}:*", max_uid + 1)
        } else {
            String::from("ALL")
        };

        let all_uids = match client.search(&search_query) {
            Ok(u) => u,
            Err(_) => continue,
        };

        // Filter out UIDs we already have
        let new_uids: Vec<u32> = all_uids
            .iter()
            .filter(|&&uid| !existing.iter().any(|m| m.uid == uid))
            .copied()
            .collect();

        if new_uids.is_empty() {
            // No new messages — deliver a final empty batch with server total
            deliver_message_batch(
                &account.id,
                folder_name,
                &mut Vec::new(),
                true,
                folder_status.exists,
            );
            continue;
        }

        // Lazy loading: only fetch the most recent `fetch_limit` UIDs
        let uids_to_fetch = if fetch_limit > 0 && new_uids.len() > fetch_limit as usize {
            let start = new_uids.len() - fetch_limit as usize;
            &new_uids[start..]
        } else {
            &new_uids[..]
        };

        let total = uids_to_fetch.len();
        set_worker_status(&alloc::format!(
            "Fetching {} messages from {}...",
            total,
            folder_name
        ));
        ss.phase
            .store(SyncPhase::FetchingBodies as u32, Ordering::Release);

        let mut new_messages = existing.clone();
        let mut batch = Vec::new();

        for (i, &uid) in uids_to_fetch.iter().enumerate() {
            if ss.cancel_requested.load(Ordering::Relaxed) {
                break;
            }

            // Update progress
            ss.acquire();
            ss.progress_current = i as u32 + 1;
            ss.progress_total = total as u32;
            ss.release();

            // Fetch headers
            let uid_str = alloc::format!("{}", uid);
            match client.fetch_headers(&uid_str) {
                Ok(summaries) => {
                    for summary in summaries {
                        // Fetch body and save to disk
                        if let Ok(body_data) = client.fetch_body(uid) {
                            let msg_path =
                                maildir::message_path(base_dir, &account.id, folder_name, uid);
                            maildir::save_message(&msg_path, &body_data);

                            // Parse for preview
                            let full = crate::mail::mime::parse_message(&body_data);
                            let mut s = summary;
                            if s.preview.is_empty() && !full.text_body.is_empty() {
                                let preview: String = full.text_body.chars().take(100).collect();
                                s.preview = preview;
                            }
                            s.category = maildir::classify_message(&s, folder_name);

                            // Learn contacts
                            crate::app().address_book.learn_from_addresses(&s.to);

                            batch.push(s.clone());
                            new_messages.push(s);
                        }

                        // Deliver batch every DELIVERY_BATCH messages
                        if batch.len() >= DELIVERY_BATCH {
                            deliver_message_batch(
                                &account.id,
                                folder_name,
                                &mut batch,
                                false,
                                folder_status.exists,
                            );
                        }
                    }
                }
                Err(_) => continue,
            }
        }

        // Deliver remaining messages
        deliver_message_batch(
            &account.id,
            folder_name,
            &mut batch,
            true,
            folder_status.exists,
        );

        // Save updated index
        maildir::save_index(&idx_path, &new_messages);
    }

    client.logout();
}

// ═══════════════════════════════════════════════════════════════════════════
// POP3 Async Sync
// ═══════════════════════════════════════════════════════════════════════════

fn sync_pop3_async(
    account: &Account,
    _acct_idx: usize,
    base_dir: &str,
    _uid_offset: u32,
    fetch_limit: u32,
) {
    let ss = sync_state();

    // ── Connect ──────────────────────────────────────────────────
    set_worker_status(&alloc::format!(
        "Connecting to {}...",
        account.incoming_host
    ));

    let mut client = Pop3Client::new();
    let use_tls = account.incoming_use_tls();
    match client.connect(&account.incoming_host, account.incoming_port, use_tls) {
        Ok(_) => {}
        Err(e) => {
            set_worker_error(&alloc::format!("POP3 connect failed: {:?}", e));
            return;
        }
    }

    // STARTTLS
    if account.incoming_security == Security::StartTls {
        if let Err(e) = client.starttls() {
            set_worker_error(&alloc::format!("POP3 STARTTLS failed: {:?}", e));
            client.quit();
            return;
        }
    }

    // Login
    if let Err(e) = client.login(&account.incoming_user, &account.incoming_pass) {
        set_worker_error(&alloc::format!("POP3 login failed: {:?}", e));
        client.quit();
        return;
    }

    ss.phase
        .store(SyncPhase::FetchingHeaders as u32, Ordering::Release);

    // Get UIDL list
    let uidls = match client.uidl() {
        Ok(u) => u,
        Err(_) => {
            client.quit();
            return;
        }
    };

    // Load existing INBOX index
    let idx_path = maildir::index_path(base_dir, &account.id, "INBOX");
    let mut existing = maildir::load_index(&idx_path);
    let existing_ids: Vec<String> = existing.iter().map(|m| m.message_id.clone()).collect();

    // Filter new messages
    let new_uidls: Vec<&(u32, String)> = uidls
        .iter()
        .filter(|(_, uidl)| !existing_ids.iter().any(|id| id == uidl))
        .collect();

    // Lazy loading: only fetch most recent N
    let uidls_to_fetch = if fetch_limit > 0 && new_uidls.len() > fetch_limit as usize {
        let start = new_uidls.len() - fetch_limit as usize;
        &new_uidls[start..]
    } else {
        &new_uidls[..]
    };

    let total_on_server = uidls.len() as u32;
    let total = uidls_to_fetch.len();

    if total == 0 {
        deliver_message_batch(&account.id, "INBOX", &mut Vec::new(), true, total_on_server);
        client.quit();
        return;
    }

    set_worker_status(&alloc::format!("Downloading {} messages...", total));
    ss.phase
        .store(SyncPhase::FetchingBodies as u32, Ordering::Release);

    let mut batch = Vec::new();

    for (i, (msg_num, uidl)) in uidls_to_fetch.iter().enumerate() {
        if ss.cancel_requested.load(Ordering::Relaxed) {
            break;
        }

        // Update progress
        ss.acquire();
        ss.progress_current = i as u32 + 1;
        ss.progress_total = total as u32;
        ss.release();

        match client.retr(*msg_num) {
            Ok(data) => {
                // Parse headers for summary
                let mut summary = crate::mail::mime::parse_headers_only(&data);
                summary.uid = *msg_num;
                summary.message_id = uidl.clone();
                summary.size = data.len() as u64;

                // Parse full for preview
                let full = crate::mail::mime::parse_message(&data);
                if summary.preview.is_empty() && !full.text_body.is_empty() {
                    let preview: String = full.text_body.chars().take(100).collect();
                    summary.preview = preview;
                }
                summary.category = maildir::classify_message(&summary, "INBOX");

                // Save .eml
                let msg_path = maildir::message_path(base_dir, &account.id, "INBOX", *msg_num);
                maildir::save_message(&msg_path, &data);

                // Learn contacts
                crate::app().address_book.learn_from_addresses(&summary.to);

                batch.push(summary.clone());
                existing.push(summary);

                // Delete from server if configured
                if !account.leave_on_server {
                    let _ = client.dele(*msg_num);
                }

                // Deliver batch
                if batch.len() >= DELIVERY_BATCH {
                    deliver_message_batch(&account.id, "INBOX", &mut batch, false, total_on_server);
                }
            }
            Err(_) => continue,
        }
    }

    // Deliver remaining
    deliver_message_batch(&account.id, "INBOX", &mut batch, true, total_on_server);

    // Save index
    maildir::save_index(&idx_path, &existing);

    client.quit();
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper Functions
// ═══════════════════════════════════════════════════════════════════════════

fn deliver_message_batch(
    account_id: &str,
    folder_name: &str,
    batch: &mut Vec<MessageSummary>,
    is_final: bool,
    total_on_server: u32,
) {
    if batch.is_empty() && !is_final {
        return;
    }
    let ss = sync_state();
    ss.acquire();
    ss.message_batches.push(MessageBatch {
        account_id: String::from(account_id),
        folder_name: String::from(folder_name),
        messages: core::mem::take(batch),
        is_final,
        total_on_server,
    });
    ss.release();
    ss.messages_ready.store(true, Ordering::Release);
}

fn set_worker_status(text: &str) {
    let ss = sync_state();
    ss.acquire();
    ss.status_text = String::from(text);
    ss.release();
    ss.status_ready.store(true, Ordering::Release);
}

fn set_worker_error(text: &str) {
    let ss = sync_state();
    ss.acquire();
    ss.error_text = String::from(text);
    ss.release();
    ss.phase.store(SyncPhase::Error as u32, Ordering::Release);
    ss.worker_active.store(false, Ordering::Release);
}

/// Sanitize folder name for directory use (same as in maildir.rs).
fn sanitize_folder(folder: &str) -> String {
    let mut s = String::with_capacity(folder.len());
    for c in folder.chars() {
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => s.push('_'),
            _ => s.push(c),
        }
    }
    s
}
