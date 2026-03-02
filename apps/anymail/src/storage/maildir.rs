//! Local mail storage (Maildir-inspired).
//!
//! Directory structure:
//!   $HOME/.anymail/accounts/<id>/<folder>/index.json
//!   $HOME/.anymail/accounts/<id>/<folder>/messages/<uid>.eml

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use crate::mail::message::MessageSummary;
use crate::mail::rfc2822::EmailAddress;

/// Ensure the directory structure exists for an account.
pub fn ensure_dirs(base: &str, account_id: &str) {
    let acct_dir = alloc::format!("{}/accounts/{}", base, account_id);
    anyos_std::fs::mkdir(&acct_dir);

    for folder in &["INBOX", "Sent", "Drafts", "Trash", "Spam"] {
        let folder_dir = alloc::format!("{}/{}", acct_dir, folder);
        anyos_std::fs::mkdir(&folder_dir);
        let msg_dir = alloc::format!("{}/messages", folder_dir);
        anyos_std::fs::mkdir(&msg_dir);
    }
}

/// Get the path for a folder's index file.
pub fn index_path(base: &str, account_id: &str, folder: &str) -> String {
    alloc::format!("{}/accounts/{}/{}/index.json", base, account_id, sanitize_folder(folder))
}

/// Get the path for a message's .eml file.
pub fn message_path(base: &str, account_id: &str, folder: &str, uid: u32) -> String {
    alloc::format!("{}/accounts/{}/{}/messages/{}.eml", base, account_id, sanitize_folder(folder), uid)
}

/// Load the message index for a folder.
pub fn load_index(path: &str) -> Vec<MessageSummary> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX { return Vec::new(); }

    let mut buf = alloc::vec![0u8; 256 * 1024];
    let mut total = 0usize;
    loop {
        let mut chunk = [0u8; 4096];
        let n = anyos_std::fs::read(fd, &mut chunk);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;
        if total + n > buf.len() { break; }
        buf[total..total + n].copy_from_slice(&chunk[..n]);
        total += n;
    }
    anyos_std::fs::close(fd);

    if total == 0 { return Vec::new(); }

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
            msg.in_reply_to = String::from(item["in_reply_to"].as_str().unwrap_or(""));
            msg.references = String::from(item["references"].as_str().unwrap_or(""));
            msg.preview = String::from(item["preview"].as_str().unwrap_or(""));
            messages.push(msg);
        }
    }

    messages
}

/// Save the message index for a folder.
pub fn save_index(path: &str, messages: &[MessageSummary]) {
    let mut root = Value::new_object();
    let mut arr = Value::new_array();

    for msg in messages {
        let mut obj = Value::new_object();
        obj.set("uid", (msg.uid as i64).into());
        obj.set("message_id", msg.message_id.as_str().into());
        obj.set("from_name", msg.from.name.as_str().into());
        obj.set("from_addr", msg.from.address.as_str().into());
        obj.set("subject", msg.subject.as_str().into());
        obj.set("date", msg.date.as_str().into());
        obj.set("size", (msg.size as i64).into());
        obj.set("flags", (msg.flags as i64).into());
        obj.set("in_reply_to", msg.in_reply_to.as_str().into());
        obj.set("references", msg.references.as_str().into());
        obj.set("preview", msg.preview.as_str().into());
        arr.push(obj);
    }

    root.set("messages", arr);
    let json_str = root.to_json_string_pretty();
    let _ = anyos_std::fs::write_bytes(path, json_str.as_bytes());
}

/// Save a raw message to disk.
pub fn save_message(path: &str, data: &[u8]) {
    let _ = anyos_std::fs::write_bytes(path, data);
}

/// Load a raw message from disk.
pub fn load_message(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX { return None; }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut chunk);
        if n == 0 || n == u32::MAX { break; }
        buf.extend_from_slice(&chunk[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(buf)
}

/// Delete a message file from disk.
pub fn delete_message(path: &str) {
    anyos_std::fs::unlink(path);
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
