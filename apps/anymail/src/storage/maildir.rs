// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Local mail storage (Maildir-inspired with libdb-backed indexes).
//!
//! Directory structure:
//!   $HOME/.anymail/accounts/<id>/mailindex.db
//!   $HOME/.anymail/accounts/<id>/<folder>/messages/<uid>.eml
//!   $HOME/.anymail/accounts/<id>/<folder>/index.json   (legacy import only)

use crate::mail::message::MessageSummary;
use crate::mail::rfc2822::EmailAddress;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libdb_client::Database;

const MAX_DB_TEXT: usize = 240;

/// Ensure the directory structure exists for an account.
pub fn ensure_dirs(base: &str, account_id: &str) {
    let acct_dir = alloc::format!("{}/accounts/{}", base, account_id);
    anyos_std::fs::mkdir(&acct_dir);

    for folder in &["INBOX", "Sent", "Drafts", "Trash", "Spam", "Archive"] {
        let folder_dir = alloc::format!("{}/{}", acct_dir, folder);
        anyos_std::fs::mkdir(&folder_dir);
        let msg_dir = alloc::format!("{}/messages", folder_dir);
        anyos_std::fs::mkdir(&msg_dir);
    }

    let _ = open_db(base, account_id);
}

/// Get the path for a folder's index file.
pub fn index_path(base: &str, account_id: &str, folder: &str) -> String {
    alloc::format!(
        "{}/accounts/{}/{}/index.json",
        base,
        account_id,
        sanitize_folder(folder)
    )
}

/// Get the path for a message's .eml file.
pub fn message_path(base: &str, account_id: &str, folder: &str, uid: u32) -> String {
    alloc::format!(
        "{}/accounts/{}/{}/messages/{}.eml",
        base,
        account_id,
        sanitize_folder(folder),
        uid
    )
}

/// Load the message index for a folder.
pub fn load_index(path: &str) -> Vec<MessageSummary> {
    let (base, account_id, folder) = match parse_index_path(path) {
        Some(parts) => parts,
        None => return load_legacy_index_json(path),
    };

    let Some(db) = open_db(base, account_id) else {
        return load_legacy_index_json(path);
    };

    if is_folder_empty(&db, folder) {
        let legacy = load_legacy_index_json(path);
        if !legacy.is_empty() {
            save_index(path, &legacy);
            let migrated = alloc::format!("{}.migrated", path);
            let _ = anyos_std::fs::rename(path, &migrated);
            return legacy;
        }
    }

    let sql = alloc::format!(
        "SELECT uid, message_id, from_name, from_addr, subject, date, size FROM msg_head WHERE folder = {}",
        sql_text(folder)
    );
    let Ok(head_rows) = db.query(&sql) else {
        return Vec::new();
    };

    let mut messages = Vec::new();
    for row in 0..head_rows.row_count() {
        let mut msg = MessageSummary::new();
        msg.uid = head_rows.get_int(row, 0).unwrap_or(0).max(0) as u32;
        msg.message_id = head_rows.get_text(row, 1).unwrap_or_default();
        msg.from = EmailAddress::with_name(
            &head_rows.get_text(row, 2).unwrap_or_default(),
            &head_rows.get_text(row, 3).unwrap_or_default(),
        );
        msg.subject = head_rows.get_text(row, 4).unwrap_or_default();
        msg.date = head_rows.get_text(row, 5).unwrap_or_default();
        msg.size = head_rows.get_int(row, 6).unwrap_or(0).max(0) as u64;
        messages.push(msg);
    }

    let meta_sql = alloc::format!(
        "SELECT uid, flags, in_reply_to, references_hdr, preview, category, to_list FROM msg_meta WHERE folder = {}",
        sql_text(folder)
    );
    if let Ok(meta_rows) = db.query(&meta_sql) {
        for row in 0..meta_rows.row_count() {
            let uid = meta_rows.get_int(row, 0).unwrap_or(0).max(0) as u32;
            if let Some(msg) = messages.iter_mut().find(|m| m.uid == uid) {
                msg.flags = meta_rows.get_int(row, 1).unwrap_or(0).max(0) as u32;
                msg.in_reply_to = meta_rows.get_text(row, 2).unwrap_or_default();
                msg.references = meta_rows.get_text(row, 3).unwrap_or_default();
                msg.preview = meta_rows.get_text(row, 4).unwrap_or_default();
                msg.category = meta_rows.get_text(row, 5).unwrap_or_default();
                msg.to = deserialize_addresses(&meta_rows.get_text(row, 6).unwrap_or_default());
            }
        }
    }

    for msg in &mut messages {
        if msg.category.is_empty() {
            msg.category = classify_message(msg, folder);
        }
    }

    messages.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| b.uid.cmp(&a.uid))
    });
    messages
}

/// Save the message index for a folder.
pub fn save_index(path: &str, messages: &[MessageSummary]) {
    let Some((base, account_id, folder)) = parse_index_path(path) else {
        return;
    };
    let Some(db) = open_db(base, account_id) else {
        return;
    };

    let _ = db.exec(&alloc::format!(
        "DELETE FROM msg_head WHERE folder = {}",
        sql_text(folder)
    ));
    let _ = db.exec(&alloc::format!(
        "DELETE FROM msg_meta WHERE folder = {}",
        sql_text(folder)
    ));

    for msg in messages {
        let category = if msg.category.is_empty() {
            classify_message(msg, folder)
        } else {
            truncate_text(&msg.category)
        };
        let head_sql = alloc::format!(
            "INSERT INTO msg_head (folder, uid, message_id, from_name, from_addr, subject, date, size) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            sql_text(folder),
            msg.uid,
            sql_text(&msg.message_id),
            sql_text(&msg.from.name),
            sql_text(&msg.from.address),
            sql_text(&msg.subject),
            sql_text(&msg.date),
            msg.size as u64
        );
        let _ = db.exec(&head_sql);

        let meta_sql = alloc::format!(
            "INSERT INTO msg_meta (folder, uid, flags, in_reply_to, references_hdr, preview, category, to_list) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            sql_text(folder),
            msg.uid,
            msg.flags as u32,
            sql_text(&msg.in_reply_to),
            sql_text(&msg.references),
            sql_text(&msg.preview),
            sql_text(&category),
            sql_text(&serialize_addresses(&msg.to))
        );
        let _ = db.exec(&meta_sql);
    }
    let _ = db.flush();
}

/// Save a raw message to disk.
pub fn save_message(path: &str, data: &[u8]) {
    let _ = anyos_std::fs::write_bytes(path, data);
}

/// Load a raw message from disk.
pub fn load_message(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut chunk);
        if n == 0 || n == u32::MAX {
            break;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(buf)
}

/// Delete a message file from disk.
pub fn delete_message(path: &str) {
    anyos_std::fs::unlink(path);
}

pub fn move_message(base: &str, account_id: &str, from_folder: &str, to_folder: &str, uid: u32) {
    let old_path = message_path(base, account_id, from_folder, uid);
    let new_path = message_path(base, account_id, to_folder, uid);
    if anyos_std::fs::rename(&old_path, &new_path) != 0 {
        if let Some(raw) = load_message(&old_path) {
            save_message(&new_path, &raw);
            delete_message(&old_path);
        }
    }
}

pub fn search_messages(
    base: &str,
    account_id: &str,
    folder: Option<&str>,
    query: &str,
) -> Vec<MessageSummary> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let Some(db) = open_db(base, account_id) else {
        return Vec::new();
    };

    let needle = sql_like_pattern(trimmed);
    let folder_filter = if let Some(folder) = folder {
        alloc::format!(" AND folder = {}", sql_text(folder))
    } else {
        String::new()
    };

    let head_sql = alloc::format!(
        "SELECT uid, folder, message_id, from_name, from_addr, subject, date, size FROM msg_head WHERE (subject LIKE {} OR from_name LIKE {} OR from_addr LIKE {}){}",
        needle, needle, needle, folder_filter
    );
    let meta_sql = alloc::format!(
        "SELECT uid, folder, flags, in_reply_to, references_hdr, preview, category, to_list FROM msg_meta WHERE (preview LIKE {} OR references_hdr LIKE {} OR to_list LIKE {}){}",
        needle, needle, needle, folder_filter
    );

    let mut messages = Vec::new();
    if let Ok(head_rows) = db.query(&head_sql) {
        for row in 0..head_rows.row_count() {
            let mut msg = MessageSummary::new();
            let folder_name = head_rows.get_text(row, 1).unwrap_or_default();
            msg.uid = head_rows.get_int(row, 0).unwrap_or(0).max(0) as u32;
            msg.message_id = head_rows.get_text(row, 2).unwrap_or_default();
            msg.from = EmailAddress::with_name(
                &head_rows.get_text(row, 3).unwrap_or_default(),
                &head_rows.get_text(row, 4).unwrap_or_default(),
            );
            msg.subject = head_rows.get_text(row, 5).unwrap_or_default();
            msg.date = head_rows.get_text(row, 6).unwrap_or_default();
            msg.size = head_rows.get_int(row, 7).unwrap_or(0).max(0) as u64;
            msg.category = classify_message(&msg, &folder_name);
            messages.push(msg);
        }
    }

    if let Ok(meta_rows) = db.query(&meta_sql) {
        for row in 0..meta_rows.row_count() {
            let uid = meta_rows.get_int(row, 0).unwrap_or(0).max(0) as u32;
            let folder_name = meta_rows.get_text(row, 1).unwrap_or_default();
            if let Some(msg) = messages.iter_mut().find(|m| m.uid == uid) {
                msg.flags = meta_rows.get_int(row, 2).unwrap_or(0).max(0) as u32;
                msg.in_reply_to = meta_rows.get_text(row, 3).unwrap_or_default();
                msg.references = meta_rows.get_text(row, 4).unwrap_or_default();
                msg.preview = meta_rows.get_text(row, 5).unwrap_or_default();
                msg.category = meta_rows
                    .get_text(row, 6)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| classify_message(msg, &folder_name));
                msg.to = deserialize_addresses(&meta_rows.get_text(row, 7).unwrap_or_default());
            }
        }
    }

    messages.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.uid.cmp(&a.uid)));
    messages
}

pub fn classify_message(message: &MessageSummary, folder: &str) -> String {
    let folder_upper = to_upper(folder);
    if folder_upper == "SPAM" || message.is_junk() {
        return String::from("Junk");
    }
    if folder_upper == "TRASH" {
        return String::from("Trash");
    }
    if folder_upper == "SENT" {
        return String::from("Sent");
    }
    if folder_upper == "DRAFTS" {
        return String::from("Drafts");
    }
    if folder_upper == "ARCHIVE" {
        return String::from("Archive");
    }

    let mut haystack = to_lower(&message.subject);
    haystack.push(' ');
    haystack.push_str(&to_lower(&message.preview));
    haystack.push(' ');
    haystack.push_str(&to_lower(&message.from.address));
    haystack.push(' ');
    haystack.push_str(&to_lower(&message.from.name));

    if contains_any(
        &haystack,
        &[
            "invoice", "receipt", "payment", "order", "shipment", "tracking", "renewal",
            "refund", "subscription", "statement", "bill",
        ],
    ) {
        return String::from("Transactions");
    }
    if contains_any(
        &haystack,
        &[
            "newsletter", "sale", "discount", "offer", "promo", "deal", "coupon", "marketing",
            "launch", "shop now",
        ],
    ) {
        return String::from("Promotions");
    }
    if contains_any(
        &haystack,
        &[
            "update", "digest", "notification", "alert", "summary", "activity", "news",
            "security", "password", "sign-in", "signin",
        ],
    ) {
        return String::from("Updates");
    }
    String::from("Primary")
}

/// Sanitize folder name for use as directory name (replace / with _).
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

fn open_db(base: &str, account_id: &str) -> Option<Database> {
    if !libdb_client::init() {
        return None;
    }
    let path = alloc::format!("{}/accounts/{}/mailindex.db", base, account_id);
    let db = Database::open(&path)?;
    let _ = create_schema(&db);
    Some(db)
}

fn create_schema(db: &Database) -> Result<(), String> {
    exec_schema(
        db,
        "CREATE TABLE msg_head (folder TEXT, uid INTEGER, message_id TEXT, from_name TEXT, from_addr TEXT, subject TEXT, date TEXT, size INTEGER)",
    )?;
    exec_schema(
        db,
        "CREATE TABLE msg_meta (folder TEXT, uid INTEGER, flags INTEGER, in_reply_to TEXT, references_hdr TEXT, preview TEXT, category TEXT, to_list TEXT)",
    )?;
    Ok(())
}

fn exec_schema(db: &Database, sql: &str) -> Result<(), String> {
    match db.exec(sql) {
        Ok(_) => Ok(()),
        Err(err) if err.contains("already exists") => Ok(()),
        Err(err) => Err(err),
    }
}

fn parse_index_path(path: &str) -> Option<(&str, &str, &str)> {
    let marker = "/accounts/";
    let start = path.find(marker)?;
    let base = &path[..start];
    let rest = &path[start + marker.len()..];
    let slash1 = rest.find('/')?;
    let account_id = &rest[..slash1];
    let rest = &rest[slash1 + 1..];
    let slash2 = rest.find('/')?;
    let folder = &rest[..slash2];
    Some((base, account_id, folder))
}

fn is_folder_empty(db: &Database, folder: &str) -> bool {
    let sql = alloc::format!(
        "SELECT uid FROM msg_head WHERE folder = {} LIMIT 1",
        sql_text(folder)
    );
    match db.query(&sql) {
        Ok(result) => result.row_count() == 0,
        Err(_) => true,
    }
}

fn load_legacy_index_json(path: &str) -> Vec<MessageSummary> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return Vec::new();
    }

    let mut buf = alloc::vec![0u8; 256 * 1024];
    let mut total = 0usize;
    loop {
        let mut chunk = [0u8; 4096];
        let n = anyos_std::fs::read(fd, &mut chunk);
        if n == 0 || n == u32::MAX {
            break;
        }
        let n = n as usize;
        if total + n > buf.len() {
            break;
        }
        buf[total..total + n].copy_from_slice(&chunk[..n]);
        total += n;
    }
    anyos_std::fs::close(fd);

    let text = match core::str::from_utf8(&buf[..total]) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let json = match Value::parse(text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();
    if let Some(arr) = json["messages"].as_array() {
        for item in arr {
            let mut msg = MessageSummary::new();
            msg.uid = item["uid"].as_i64().unwrap_or(0) as u32;
            msg.message_id = String::from(item["message_id"].as_str().unwrap_or(""));
            msg.from = EmailAddress::with_name(
                item["from_name"].as_str().unwrap_or(""),
                item["from_addr"].as_str().unwrap_or(""),
            );
            msg.subject = String::from(item["subject"].as_str().unwrap_or(""));
            msg.date = String::from(item["date"].as_str().unwrap_or(""));
            msg.size = item["size"].as_i64().unwrap_or(0) as u64;
            msg.flags = item["flags"].as_i64().unwrap_or(0) as u32;
            msg.category = String::from(item["category"].as_str().unwrap_or(""));
            msg.in_reply_to = String::from(item["in_reply_to"].as_str().unwrap_or(""));
            msg.references = String::from(item["references"].as_str().unwrap_or(""));
            msg.preview = String::from(item["preview"].as_str().unwrap_or(""));
            messages.push(msg);
        }
    }
    messages
}

fn serialize_addresses(addresses: &[EmailAddress]) -> String {
    let mut out = String::new();
    for (idx, addr) in addresses.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if !addr.name.is_empty() {
            out.push_str(&addr.name);
            out.push('<');
            out.push_str(&addr.address);
            out.push('>');
        } else {
            out.push_str(&addr.address);
        }
    }
    truncate_text(&out)
}

fn deserialize_addresses(text: &str) -> Vec<EmailAddress> {
    let mut result = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(open) = trimmed.rfind('<') {
            if trimmed.ends_with('>') && open < trimmed.len() - 1 {
                result.push(EmailAddress::with_name(
                    trimmed[..open].trim(),
                    &trimmed[open + 1..trimmed.len() - 1],
                ));
                continue;
            }
        }
        result.push(EmailAddress::new(trimmed));
    }
    result
}

fn sql_text(text: &str) -> String {
    let mut out = String::from("'");
    for c in truncate_text(text).chars() {
        if c == '\'' {
            out.push('\'');
            out.push('\'');
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn sql_like_pattern(text: &str) -> String {
    let mut escaped = String::new();
    for c in truncate_text(text).chars() {
        match c {
            '\'' => {
                escaped.push('\'');
                escaped.push('\'');
            }
            '%' => {
                escaped.push('\\');
                escaped.push('%');
            }
            '_' => {
                escaped.push('\\');
                escaped.push('_');
            }
            _ => escaped.push(c),
        }
    }
    alloc::format!("'%{}%'", escaped)
}

fn truncate_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > MAX_DB_TEXT {
            break;
        }
        out.push(ch);
    }
    out
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn to_lower(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            r.push((c as u8 + 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}

fn to_upper(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'a' && c <= 'z' {
            r.push((c as u8 - 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}
