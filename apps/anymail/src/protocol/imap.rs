// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! IMAP4rev1 client implementation (RFC 3501).

use super::tcp_stream::{ConnError, TcpStream};
use crate::mail::message::{
    MessageSummary, FLAG_ANSWERED, FLAG_DELETED, FLAG_DRAFT, FLAG_FLAGGED, FLAG_SEEN,
};
use crate::mail::rfc2047;
use crate::mail::rfc2822;
use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ImapError {
    Connection(ConnError),
    AuthFailed(String),
    ServerError(String),
    ParseError(String),
    NotConnected,
}

impl From<ConnError> for ImapError {
    fn from(e: ConnError) -> Self {
        ImapError::Connection(e)
    }
}

#[derive(Clone)]
pub struct ImapFolder {
    pub name: String,
    pub delimiter: char,
    pub flags: Vec<String>,
    pub has_children: bool,
    pub special_use: SpecialUse,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SpecialUse {
    None,
    Inbox,
    Sent,
    Drafts,
    Trash,
    Spam,
    Archive,
    All,
}

pub struct FolderStatus {
    pub exists: u32,
    pub recent: u32,
    pub unseen: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
}

// ---------------------------------------------------------------------------
// IMAP Client
// ---------------------------------------------------------------------------

pub struct ImapClient {
    stream: Option<TcpStream>,
    tag_counter: u32,
    pub capabilities: Vec<String>,
    pub selected_folder: Option<String>,
}

impl ImapClient {
    pub fn new() -> Self {
        Self {
            stream: None,
            tag_counter: 0,
            capabilities: Vec::new(),
            selected_folder: None,
        }
    }

    /// Connect to an IMAP server.
    pub fn connect(&mut self, host: &str, port: u16, use_tls: bool) -> Result<(), ImapError> {
        let mut stream = TcpStream::connect(host, port, use_tls)?;

        // Read server greeting
        let greeting = stream.recv_line()?;
        if !greeting.starts_with("* OK") {
            return Err(ImapError::ServerError(greeting));
        }

        self.stream = Some(stream);

        // Get capabilities
        self.capability()?;

        Ok(())
    }

    /// STARTTLS upgrade.
    pub fn starttls(&mut self) -> Result<(), ImapError> {
        let resp = self.command("STARTTLS")?;
        if !resp.contains("OK") {
            return Err(ImapError::ServerError(resp));
        }
        self.stream_mut()?.upgrade_to_tls()?;
        self.capability()?;
        Ok(())
    }

    /// Login with username and password.
    pub fn login(&mut self, user: &str, pass: &str) -> Result<(), ImapError> {
        // Escape password (quote if needed)
        let cmd = alloc::format!("LOGIN \"{}\" \"{}\"", escape_imap(user), escape_imap(pass));
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::AuthFailed(resp));
        }
        // Update capabilities from login response
        self.capability()?;
        Ok(())
    }

    /// List folders.
    pub fn list_folders(&mut self) -> Result<Vec<ImapFolder>, ImapError> {
        let (untagged, _) = self.command_full("LIST \"\" \"*\"")?;
        let mut folders = Vec::new();

        for line in &untagged {
            if let Some(folder) = parse_list_response(line) {
                folders.push(folder);
            }
        }

        // Sort: INBOX first, then alphabetically
        folders.sort_by(|a, b| {
            if a.special_use == SpecialUse::Inbox {
                return core::cmp::Ordering::Less;
            }
            if b.special_use == SpecialUse::Inbox {
                return core::cmp::Ordering::Greater;
            }
            a.name.cmp(&b.name)
        });

        Ok(folders)
    }

    /// Select a folder (makes it the current folder for operations).
    pub fn select(&mut self, folder: &str) -> Result<FolderStatus, ImapError> {
        let cmd = alloc::format!("SELECT \"{}\"", escape_imap(folder));
        let (untagged, tagged) = self.command_full(&cmd)?;

        if tagged.contains("NO") || tagged.contains("BAD") {
            return Err(ImapError::ServerError(tagged));
        }

        let mut status = FolderStatus {
            exists: 0,
            recent: 0,
            unseen: 0,
            uid_validity: 0,
            uid_next: 0,
        };

        for line in &untagged {
            if line.contains("EXISTS") {
                status.exists = extract_number(line);
            } else if line.contains("RECENT") {
                status.recent = extract_number(line);
            } else if line.contains("UNSEEN") {
                status.unseen = extract_number(line);
            } else if line.contains("UIDVALIDITY") {
                status.uid_validity = extract_bracket_number(line, "UIDVALIDITY");
            } else if line.contains("UIDNEXT") {
                status.uid_next = extract_bracket_number(line, "UIDNEXT");
            }
        }

        self.selected_folder = Some(String::from(folder));
        Ok(status)
    }

    /// Fetch message headers for a range (e.g., "1:*" or "1:50").
    pub fn fetch_headers(&mut self, range: &str) -> Result<Vec<MessageSummary>, ImapError> {
        let cmd = alloc::format!(
            "UID FETCH {} (UID FLAGS BODY.PEEK[HEADER.FIELDS (FROM TO SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES CONTENT-TYPE)])",
            range
        );
        let (untagged, _) = self.command_full(&cmd)?;

        let mut messages = Vec::new();
        let mut i = 0;

        while i < untagged.len() {
            let line = &untagged[i];
            if line.contains("FETCH") {
                let mut msg = MessageSummary::new();

                // Extract UID
                if let Some(uid) = extract_fetch_field(line, "UID") {
                    msg.uid = uid.trim().parse().unwrap_or(0);
                }

                // Extract FLAGS
                if let Some(flags_str) = extract_flags(line) {
                    msg.flags = parse_flags(&flags_str);
                }

                // Read literal header data (may span multiple untagged lines)
                let mut header_text = String::new();
                i += 1;
                while i < untagged.len() {
                    let hl = &untagged[i];
                    // End of fetch response: line starts with ")" or is empty
                    if hl.trim() == ")" || hl.trim().is_empty() {
                        break;
                    }
                    header_text.push_str(hl);
                    header_text.push('\n');
                    i += 1;
                }

                // Parse headers
                parse_header_fields(&header_text, &mut msg);
                messages.push(msg);
            }
            i += 1;
        }

        Ok(messages)
    }

    /// Fetch the full body of a message by UID.
    pub fn fetch_body(&mut self, uid: u32) -> Result<Vec<u8>, ImapError> {
        let cmd = alloc::format!("UID FETCH {} (BODY[])", uid);
        let tag = self.next_tag();
        let tag_str = alloc::format!("A{:04}", tag);

        self.send_tagged(&tag_str, &cmd)?;

        // Read until we find the tagged response
        let mut body_data = Vec::new();
        let mut reading_literal = false;
        let mut literal_remaining: usize = 0;

        loop {
            if reading_literal && literal_remaining > 0 {
                // Read literal data
                let chunk = self.stream_mut()?.recv_exact(literal_remaining)?;
                body_data.extend_from_slice(&chunk);
                literal_remaining = 0;
                reading_literal = false;
                continue;
            }

            let line = self.stream_mut()?.recv_line()?;

            // Check if this is the tagged response
            if line.starts_with(&tag_str) {
                break;
            }

            // Check for literal size indicator {NNN}
            if let Some(lit_size) = extract_literal_size(&line) {
                reading_literal = true;
                literal_remaining = lit_size;
                continue;
            }

            // Accumulate header lines as body data too
            if !line.starts_with("*") || body_data.is_empty() {
                body_data.extend_from_slice(line.as_bytes());
                body_data.push(b'\n');
            }
        }

        Ok(body_data)
    }

    /// Search for messages matching criteria.
    /// Common criteria: "UNSEEN", "FLAGGED", "ALL", "SUBJECT \"text\"", etc.
    pub fn search(&mut self, criteria: &str) -> Result<Vec<u32>, ImapError> {
        let cmd = alloc::format!("UID SEARCH {}", criteria);
        let (untagged, _) = self.command_full(&cmd)?;

        let mut uids = Vec::new();
        for line in &untagged {
            if line.contains("SEARCH") {
                // * SEARCH 1 2 3 4 5
                let rest = if let Some(pos) = line.find("SEARCH") {
                    &line[pos + 6..]
                } else {
                    ""
                };
                for tok in rest.split_whitespace() {
                    if let Ok(uid) = tok.parse::<u32>() {
                        uids.push(uid);
                    }
                }
            }
        }

        Ok(uids)
    }

    /// Store flags on a message (by UID).
    /// action: "+FLAGS" to add, "-FLAGS" to remove, "FLAGS" to set.
    pub fn store_flags(&mut self, uid: u32, action: &str, flags: &str) -> Result<(), ImapError> {
        let cmd = alloc::format!("UID STORE {} {} ({})", uid, action, flags);
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Mark a message as seen.
    pub fn mark_seen(&mut self, uid: u32) -> Result<(), ImapError> {
        self.store_flags(uid, "+FLAGS", "\\Seen")
    }

    /// Mark a message as flagged (starred).
    pub fn mark_flagged(&mut self, uid: u32, flagged: bool) -> Result<(), ImapError> {
        let action = if flagged { "+FLAGS" } else { "-FLAGS" };
        self.store_flags(uid, action, "\\Flagged")
    }

    /// Mark a message as deleted.
    pub fn mark_deleted(&mut self, uid: u32) -> Result<(), ImapError> {
        self.store_flags(uid, "+FLAGS", "\\Deleted")
    }

    /// Copy a message to another folder.
    pub fn copy(&mut self, uid: u32, dest_folder: &str) -> Result<(), ImapError> {
        let cmd = alloc::format!("UID COPY {} \"{}\"", uid, escape_imap(dest_folder));
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Move a message to another folder (COPY + STORE \Deleted + EXPUNGE).
    pub fn move_message(&mut self, uid: u32, dest_folder: &str) -> Result<(), ImapError> {
        self.copy(uid, dest_folder)?;
        self.mark_deleted(uid)?;
        self.expunge()?;
        Ok(())
    }

    /// Expunge deleted messages from the current folder.
    pub fn expunge(&mut self) -> Result<(), ImapError> {
        let resp = self.command("EXPUNGE")?;
        if resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Create a new folder.
    pub fn create_folder(&mut self, folder: &str) -> Result<(), ImapError> {
        let cmd = alloc::format!("CREATE \"{}\"", escape_imap(folder));
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Delete a folder.
    pub fn delete_folder(&mut self, folder: &str) -> Result<(), ImapError> {
        let cmd = alloc::format!("DELETE \"{}\"", escape_imap(folder));
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Rename a folder.
    pub fn rename_folder(&mut self, old_name: &str, new_name: &str) -> Result<(), ImapError> {
        let cmd = alloc::format!(
            "RENAME \"{}\" \"{}\"",
            escape_imap(old_name),
            escape_imap(new_name)
        );
        let resp = self.command(&cmd)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }
        Ok(())
    }

    /// Append a message to a folder (e.g., save to Sent or Drafts).
    pub fn append(&mut self, folder: &str, flags: &str, message: &[u8]) -> Result<(), ImapError> {
        let cmd = alloc::format!(
            "APPEND \"{}\" ({}) {{{}}}",
            escape_imap(folder),
            flags,
            message.len()
        );
        let tag = self.next_tag();
        let tag_str = alloc::format!("A{:04}", tag);

        self.send_tagged(&tag_str, &cmd)?;

        // Server sends continuation request "+"
        let cont = self.stream_mut()?.recv_line()?;
        if !cont.starts_with('+') {
            return Err(ImapError::ServerError(cont));
        }

        // Send literal message data
        self.stream_mut()?.send_all(message)?;
        self.stream_mut()?.send_line("")?;

        // Read tagged response
        let resp = self.read_tagged_response(&tag_str)?;
        if resp.contains("NO") || resp.contains("BAD") {
            return Err(ImapError::ServerError(resp));
        }

        Ok(())
    }

    /// Get CAPABILITY.
    pub fn capability(&mut self) -> Result<(), ImapError> {
        let (untagged, _) = self.command_full("CAPABILITY")?;
        self.capabilities.clear();
        for line in &untagged {
            if line.contains("CAPABILITY") {
                for cap in line.split_whitespace() {
                    if cap != "*" && cap != "CAPABILITY" {
                        self.capabilities.push(String::from(cap));
                    }
                }
            }
        }
        Ok(())
    }

    /// NOOP (keep-alive, also gets new message notifications).
    pub fn noop(&mut self) -> Result<Vec<String>, ImapError> {
        let (untagged, _) = self.command_full("NOOP")?;
        Ok(untagged)
    }

    /// Logout and disconnect.
    pub fn logout(&mut self) {
        if self.stream.is_some() {
            let _ = self.command("LOGOUT");
            if let Some(mut s) = self.stream.take() {
                s.close();
            }
        }
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    // --- Internal helpers ---

    fn stream_mut(&mut self) -> Result<&mut TcpStream, ImapError> {
        self.stream.as_mut().ok_or(ImapError::NotConnected)
    }

    fn next_tag(&mut self) -> u32 {
        self.tag_counter += 1;
        self.tag_counter
    }

    fn send_tagged(&mut self, tag: &str, cmd: &str) -> Result<(), ImapError> {
        let line = alloc::format!("{} {}", tag, cmd);
        self.stream_mut()?.send_line(&line)?;
        Ok(())
    }

    /// Send a command and return tagged response.
    fn command(&mut self, cmd: &str) -> Result<String, ImapError> {
        let (_, tagged) = self.command_full(cmd)?;
        Ok(tagged)
    }

    /// Send a command and return (untagged_lines, tagged_response).
    fn command_full(&mut self, cmd: &str) -> Result<(Vec<String>, String), ImapError> {
        let tag = self.next_tag();
        let tag_str = alloc::format!("A{:04}", tag);

        self.send_tagged(&tag_str, cmd)?;

        let mut untagged = Vec::new();

        loop {
            let line = self.stream_mut()?.recv_line()?;

            if line.starts_with(&tag_str) {
                return Ok((untagged, line));
            }

            untagged.push(line);
        }
    }

    fn read_tagged_response(&mut self, tag: &str) -> Result<String, ImapError> {
        loop {
            let line = self.stream_mut()?.recv_line()?;
            if line.starts_with(tag) {
                return Ok(line);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn parse_list_response(line: &str) -> Option<ImapFolder> {
    // * LIST (\HasNoChildren) "/" "INBOX"
    if !line.starts_with("* LIST") && !line.starts_with("* list") {
        return None;
    }

    // Extract flags in parentheses
    let paren_start = line.find('(')?;
    let paren_end = line.find(')')?;
    let flags_str = &line[paren_start + 1..paren_end];
    let flags: Vec<String> = flags_str
        .split_whitespace()
        .map(|s| String::from(s))
        .collect();

    let has_children = flags.iter().any(|f| f == "\\HasChildren");

    // Extract delimiter
    let rest = &line[paren_end + 1..].trim_start();
    let delimiter = if rest.starts_with("\"") {
        rest.chars().nth(1).unwrap_or('/')
    } else if rest.starts_with("NIL") {
        '/'
    } else {
        '/'
    };

    // Extract folder name (last quoted string or last word)
    let name = if let Some(last_quote_start) = line.rfind('"') {
        let before = &line[..last_quote_start];
        if let Some(prev_quote) = before.rfind('"') {
            String::from(&line[prev_quote + 1..last_quote_start])
        } else {
            String::from(&line[last_quote_start + 1..])
        }
    } else {
        // Unquoted name (last word)
        let parts: Vec<&str> = line.split_whitespace().collect();
        String::from(*parts.last().unwrap_or(&""))
    };

    // Determine special use
    let special_use = detect_special_use(&name, &flags);

    Some(ImapFolder {
        name,
        delimiter,
        flags,
        has_children,
        special_use,
    })
}

fn detect_special_use(name: &str, flags: &[String]) -> SpecialUse {
    // Check flags first (RFC 6154)
    for f in flags {
        match f.as_str() {
            "\\Sent" => return SpecialUse::Sent,
            "\\Drafts" => return SpecialUse::Drafts,
            "\\Trash" | "\\Junk" => return SpecialUse::Trash,
            "\\Archive" | "\\All" => return SpecialUse::Archive,
            _ => {}
        }
    }

    // Fallback: detect by name
    let lower = to_lower(name);
    if lower == "inbox" {
        return SpecialUse::Inbox;
    }
    if lower == "sent" || lower.contains("sent") {
        return SpecialUse::Sent;
    }
    if lower == "drafts" || lower.contains("draft") || lower.contains("entwurf") {
        return SpecialUse::Drafts;
    }
    if lower == "trash"
        || lower.contains("trash")
        || lower.contains("papierkorb")
        || lower.contains("deleted")
    {
        return SpecialUse::Trash;
    }
    if lower == "spam" || lower == "junk" || lower.contains("spam") || lower.contains("junk") {
        return SpecialUse::Spam;
    }
    if lower == "archive" || lower.contains("archiv") {
        return SpecialUse::Archive;
    }

    SpecialUse::None
}

fn parse_header_fields(header_text: &str, msg: &mut MessageSummary) {
    let mut current_name = String::new();
    let mut current_value = String::new();

    for line in header_text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        // Continuation line
        if line.starts_with(' ') || line.starts_with('\t') {
            current_value.push(' ');
            current_value.push_str(line.trim());
            continue;
        }

        // Process previous header
        if !current_name.is_empty() {
            apply_header(msg, &current_name, &current_value);
        }

        // Parse new header
        if let Some(colon) = line.find(':') {
            current_name = String::from(&line[..colon]);
            current_value = String::from(line[colon + 1..].trim());
        }
    }

    // Process last header
    if !current_name.is_empty() {
        apply_header(msg, &current_name, &current_value);
    }
}

fn apply_header(msg: &mut MessageSummary, name: &str, value: &str) {
    let lower = to_lower(name);
    match lower.as_str() {
        "from" => {
            msg.from = rfc2822::parse_address(&rfc2047::decode_header(value));
        }
        "to" => {
            msg.to = rfc2822::parse_address_list(&rfc2047::decode_header(value));
        }
        "subject" => {
            msg.subject = rfc2047::decode_header(value);
        }
        "date" => {
            msg.date = rfc2822::parse_date(value);
        }
        "message-id" => {
            msg.message_id = String::from(value.trim());
        }
        "in-reply-to" => {
            msg.in_reply_to = String::from(value.trim());
        }
        "references" => {
            msg.references = String::from(value.trim());
        }
        _ => {}
    }
}

fn extract_fetch_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let pattern = alloc::format!("{} ", field);
    if let Some(pos) = line.find(pattern.as_str()) {
        let start = pos + pattern.len();
        let rest = &line[start..];
        // Find end (space or parenthesis)
        let end = rest
            .find(|c: char| c == ' ' || c == ')')
            .unwrap_or(rest.len());
        Some(&rest[..end])
    } else {
        None
    }
}

fn extract_flags(line: &str) -> Option<String> {
    // Look for FLAGS (...) in the line
    if let Some(pos) = line.find("FLAGS") {
        let rest = &line[pos + 5..];
        if let Some(open) = rest.find('(') {
            if let Some(close) = rest.find(')') {
                return Some(String::from(&rest[open + 1..close]));
            }
        }
    }
    None
}

fn parse_flags(flags_str: &str) -> u32 {
    let mut flags = 0u32;
    for flag in flags_str.split_whitespace() {
        match flag {
            "\\Seen" => flags |= FLAG_SEEN,
            "\\Answered" => flags |= FLAG_ANSWERED,
            "\\Flagged" => flags |= FLAG_FLAGGED,
            "\\Deleted" => flags |= FLAG_DELETED,
            "\\Draft" => flags |= FLAG_DRAFT,
            _ => {}
        }
    }
    flags
}

fn extract_number(line: &str) -> u32 {
    // "* 42 EXISTS" -> 42
    for tok in line.split_whitespace() {
        if let Ok(n) = tok.parse::<u32>() {
            return n;
        }
    }
    0
}

fn extract_bracket_number(line: &str, key: &str) -> u32 {
    // "[UIDVALIDITY 12345]" or "UIDVALIDITY 12345"
    if let Some(pos) = line.find(key) {
        let rest = &line[pos + key.len()..];
        for tok in rest.split(|c: char| !c.is_ascii_digit()) {
            if !tok.is_empty() {
                if let Ok(n) = tok.parse::<u32>() {
                    return n;
                }
            }
        }
    }
    0
}

fn extract_literal_size(line: &str) -> Option<usize> {
    // Look for {NNN} at end of line
    if let Some(open) = line.rfind('{') {
        if let Some(close) = line.rfind('}') {
            if close > open {
                let num_str = &line[open + 1..close];
                return num_str.parse().ok();
            }
        }
    }
    None
}

fn escape_imap(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
