#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

const PIPE_NAME: &str = "confd";
const READ_CHUNK_SIZE: usize = 512;
const SINGLE_LINE_TIMEOUT_MS: u32 = 5_000;
const MULTI_LINE_TIMEOUT_MS: u32 = 30_000;
const READ_IDLE_GRACE_MS: u32 = 500;
const REQUEST_POLL_SLEEP_MS: u32 = 1;
const EVENT_POLL_SLEEP_MS: u32 = 10;
static NEXT_CLIENT_SEQ: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryScope {
    System,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfTarget {
    Scope(RegistryScope),
    User(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfValue {
    String(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfValueRef<'a> {
    String(&'a str),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultEntry<'a> {
    pub path: &'a str,
    pub value: ConfValueRef<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MigrationEntry<'a> {
    pub version: u32,
    pub path: &'a str,
    pub value: ConfValueRef<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationStep<'a> {
    Set(MigrationEntry<'a>),
    Delete {
        version: u32,
        path: &'a str,
    },
    Rename {
        version: u32,
        from: &'a str,
        to: &'a str,
    },
    Copy {
        version: u32,
        from: &'a str,
        to: &'a str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistryManifest<'a> {
    pub namespace: &'a str,
    pub scope: RegistryScope,
    pub version: u32,
    pub directories: &'a [&'a str],
    pub defaults: &'a [DefaultEntry<'a>],
    pub migrations: &'a [MigrationStep<'a>],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Directory,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfItem {
    pub scope: RegistryScope,
    pub path: String,
    pub kind: NodeKind,
    pub value: Option<ConfValue>,
    pub version: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfEvent {
    pub watch_id: u32,
    pub action: String,
    pub item: ConfItem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfAuditEntry {
    pub seq: u64,
    pub actor_uid: u16,
    pub owner_uid: u16,
    pub actor_name: String,
    pub tid: u32,
    pub action: String,
    pub scope: RegistryScope,
    pub path: String,
    pub status: String,
    pub detail: String,
    pub version: u64,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfError {
    NotRunning,
    PipeCreateFailed,
    Disconnected,
    Timeout,
    Protocol(String),
    Remote(String),
    InvalidArgument(&'static str),
}

pub struct ConfClient {
    tid: u32,
    req_pipe: u32,
    reply_pipe: u32,
    reply_pipe_name: String,
    client_name: String,
}

impl ConfClient {
    pub fn connect(client_name: &str) -> Result<Self, ConfError> {
        if client_name.is_empty() {
            return Err(ConfError::InvalidArgument("client_name must not be empty"));
        }
        if !is_valid_token(client_name) {
            return Err(ConfError::InvalidArgument("client_name contains invalid characters"));
        }

        #[cfg(feature = "host")]
        {
            Ok(Self {
                tid: 1,
                req_pipe: 1,
                reply_pipe: 1,
                reply_pipe_name: String::from("confd-1-host"),
                client_name: client_name.to_string(),
            })
        }

        #[cfg(not(feature = "host"))]
        {
            let tid = libsyscall::get_tid();
            let req_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
            if req_pipe == 0 {
                return Err(ConfError::NotRunning);
            }

            let seq = NEXT_CLIENT_SEQ.fetch_add(1, Ordering::Relaxed);
            let reply_name = format!("confd-{}-{}", tid, seq);
            let reply_pipe = anyos_std::ipc::pipe_create(&reply_name);
            if reply_pipe == 0 {
                return Err(ConfError::PipeCreateFailed);
            }

            let mut client = Self {
                tid,
                req_pipe,
                reply_pipe,
                reply_pipe_name: reply_name,
                client_name: client_name.to_string(),
            };
            client.hello()?;
            Ok(client)
        }
    }

    pub fn mkdir(&mut self, scope: RegistryScope, path: &str) -> Result<ConfItem, ConfError> {
        self.mkdir_target(ConfTarget::Scope(scope), path)
    }

    pub fn mkdir_for_user(&mut self, uid: u16, path: &str) -> Result<ConfItem, ConfError> {
        self.mkdir_target(ConfTarget::User(uid), path)
    }

    pub fn mkdir_target(&mut self, target: ConfTarget, path: &str) -> Result<ConfItem, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("MKDIR ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let line = self.request_single_line(&cmd)?;
        parse_simple_ok(&line, "mkdir", target_scope(target), path)
    }

    pub fn set(&mut self, scope: RegistryScope, path: &str, value: ConfValue) -> Result<ConfItem, ConfError> {
        self.set_target(ConfTarget::Scope(scope), path, value)
    }

    pub fn set_for_user(&mut self, uid: u16, path: &str, value: ConfValue) -> Result<ConfItem, ConfError> {
        self.set_target(ConfTarget::User(uid), path, value)
    }

    pub fn set_target(&mut self, target: ConfTarget, path: &str, value: ConfValue) -> Result<ConfItem, ConfError> {
        validate_path(path)?;
        let (type_name, value_text) = encode_value(&value)?;
        let mut cmd = String::from("SET ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        cmd.push(' ');
        cmd.push_str(type_name);
        cmd.push(' ');
        cmd.push_str(&value_text);
        let line = self.request_single_line(&cmd)?;
        parse_simple_ok(&line, "set", target_scope(target), path)
    }

    pub fn get(&mut self, scope: RegistryScope, path: &str) -> Result<ConfItem, ConfError> {
        self.get_target(ConfTarget::Scope(scope), path)
    }

    pub fn get_for_user(&mut self, uid: u16, path: &str) -> Result<ConfItem, ConfError> {
        self.get_target(ConfTarget::User(uid), path)
    }

    pub fn get_target(&mut self, target: ConfTarget, path: &str) -> Result<ConfItem, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("GET ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let line = self.request_single_line(&cmd)?;
        parse_item_line(&line)
    }

    pub fn del(&mut self, scope: RegistryScope, path: &str) -> Result<ConfItem, ConfError> {
        self.del_target(ConfTarget::Scope(scope), path)
    }

    pub fn del_for_user(&mut self, uid: u16, path: &str) -> Result<ConfItem, ConfError> {
        self.del_target(ConfTarget::User(uid), path)
    }

    pub fn del_target(&mut self, target: ConfTarget, path: &str) -> Result<ConfItem, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("DEL ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let line = self.request_single_line(&cmd)?;
        parse_simple_ok(&line, "del", target_scope(target), path)
    }

    pub fn list(&mut self, scope: RegistryScope, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        self.list_target(ConfTarget::Scope(scope), path)
    }

    pub fn list_for_user(&mut self, uid: u16, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        self.list_target(ConfTarget::User(uid), path)
    }

    pub fn list_target(&mut self, target: ConfTarget, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("LIST ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let response = self.request_multi_line(&cmd)?;
        parse_list_response(&response)
    }

    pub fn list_children(&mut self, scope: RegistryScope, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        self.list_children_target(ConfTarget::Scope(scope), path)
    }

    pub fn list_children_for_user(&mut self, uid: u16, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        self.list_children_target(ConfTarget::User(uid), path)
    }

    pub fn list_children_target(&mut self, target: ConfTarget, path: &str) -> Result<Vec<ConfItem>, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("LISTCHILDREN ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let response = self.request_multi_line(&cmd)?;
        parse_list_response(&response)
    }

    pub fn watch(&mut self, scope: RegistryScope, path: &str) -> Result<u32, ConfError> {
        self.watch_target(ConfTarget::Scope(scope), path)
    }

    pub fn watch_for_user(&mut self, uid: u16, path: &str) -> Result<u32, ConfError> {
        self.watch_target(ConfTarget::User(uid), path)
    }

    pub fn watch_target(&mut self, target: ConfTarget, path: &str) -> Result<u32, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("WATCH ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        let line = self.request_single_line(&cmd)?;
        parse_watch_ok(&line)
    }

    pub fn audit(&mut self, scope: RegistryScope, path: &str, limit: u32) -> Result<Vec<ConfAuditEntry>, ConfError> {
        self.audit_target(ConfTarget::Scope(scope), path, limit)
    }

    pub fn audit_for_user(&mut self, uid: u16, path: &str, limit: u32) -> Result<Vec<ConfAuditEntry>, ConfError> {
        self.audit_target(ConfTarget::User(uid), path, limit)
    }

    pub fn audit_target(&mut self, target: ConfTarget, path: &str, limit: u32) -> Result<Vec<ConfAuditEntry>, ConfError> {
        validate_path(path)?;
        let mut cmd = String::from("AUDIT ");
        cmd.push_str(&target_name(target));
        cmd.push(' ');
        cmd.push_str(path);
        cmd.push(' ');
        cmd.push_str(&format!("{}", limit.max(1).min(500)));
        let response = self.request_multi_line(&cmd)?;
        parse_audit_response(&response)
    }

    pub fn unwatch(&mut self, watch_id: u32) -> Result<(), ConfError> {
        let mut cmd = String::from("UNWATCH ");
        cmd.push_str(&format!("{}", watch_id));
        let line = self.request_single_line(&cmd)?;
        if line == format!("OK unwatch {}", watch_id) {
            Ok(())
        } else {
            Err(ConfError::Protocol(String::from("unexpected unwatch response")))
        }
    }

    pub fn ping(&mut self) -> Result<(), ConfError> {
        let line = self.request_single_line("PING")?;
        if line == "PONG" {
            Ok(())
        } else {
            Err(ConfError::Protocol(String::from("expected PONG")))
        }
    }

    pub fn register_manifest(&mut self, manifest: &RegistryManifest<'_>) -> Result<(u32, u32), ConfError> {
        validate_path(manifest.namespace)?;
        let mut payload = String::new();

        for dir in manifest.directories {
            validate_path(dir)?;
            append_manifest_field(&mut payload, "D", dir, None, None);
        }
        for default in manifest.defaults {
            validate_path(default.path)?;
            let (value_type, value_text) = encode_value_ref(default.value);
            append_manifest_field(&mut payload, "K", default.path, Some(value_type), Some(&value_text));
        }
        for migration in manifest.migrations {
            match migration {
                MigrationStep::Set(entry) => {
                    validate_path(entry.path)?;
                    let (value_type, value_text) = encode_value_ref(entry.value);
                    if !payload.is_empty() {
                        payload.push(';');
                    }
                    payload.push_str("M|");
                    payload.push_str(&format!("{}", entry.version));
                    payload.push('|');
                    payload.push_str(entry.path);
                    payload.push('|');
                    payload.push_str(value_type);
                    payload.push('|');
                    payload.push_str(&value_text);
                }
                MigrationStep::Delete { version, path } => {
                    validate_path(path)?;
                    if !payload.is_empty() {
                        payload.push(';');
                    }
                    payload.push_str("X|");
                    payload.push_str(&format!("{}", version));
                    payload.push('|');
                    payload.push_str(path);
                }
                MigrationStep::Rename { version, from, to } => {
                    validate_path(from)?;
                    validate_path(to)?;
                    if !payload.is_empty() {
                        payload.push(';');
                    }
                    payload.push_str("R|");
                    payload.push_str(&format!("{}", version));
                    payload.push('|');
                    payload.push_str(from);
                    payload.push('|');
                    payload.push_str(to);
                }
                MigrationStep::Copy { version, from, to } => {
                    validate_path(from)?;
                    validate_path(to)?;
                    if !payload.is_empty() {
                        payload.push(';');
                    }
                    payload.push_str("C|");
                    payload.push_str(&format!("{}", version));
                    payload.push('|');
                    payload.push_str(from);
                    payload.push('|');
                    payload.push_str(to);
                }
            }
        }

        let mut cmd = String::from("REGISTER ");
        cmd.push_str(scope_name(manifest.scope));
        cmd.push(' ');
        cmd.push_str(manifest.namespace);
        cmd.push(' ');
        cmd.push_str(&format!("{}", manifest.version));
        cmd.push(' ');
        cmd.push_str(&payload);

        let line = self.request_single_line(&cmd)?;
        parse_register_ok(&line, manifest.scope, manifest.namespace)
    }

    pub fn poll_event(&mut self, timeout_ms: u32) -> Result<Option<ConfEvent>, ConfError> {
        #[cfg(feature = "host")]
        {
            let _ = timeout_ms;
            Ok(None)
        }

        #[cfg(not(feature = "host"))]
        {
            let deadline = libsyscall::uptime_ms().wrapping_add(timeout_ms);
            let mut data = Vec::new();
            let mut chunk = [0u8; READ_CHUNK_SIZE];

            loop {
                let n = anyos_std::ipc::pipe_read(self.reply_pipe, &mut chunk);
                if n > 0 && n != u32::MAX {
                    data.extend_from_slice(&chunk[..n as usize]);
                    if let Some(line) = take_first_line(&data) {
                        return parse_event_line(&line).map(Some);
                    }
                } else if n == u32::MAX {
                    return Err(ConfError::Disconnected);
                }

                if timeout_ms == 0 || deadline_reached(deadline) {
                    return Ok(None);
                }
                anyos_std::process::sleep(EVENT_POLL_SLEEP_MS);
            }
        }
    }

    fn hello(&mut self) -> Result<(), ConfError> {
        let mut cmd = String::from("HELLO ");
        cmd.push_str(&self.client_name);
        let line = self.request_single_line(&cmd)?;
        if line.starts_with("OK hello ") {
            Ok(())
        } else {
            Err(ConfError::Protocol(String::from("expected HELLO acknowledgement")))
        }
    }

    fn request_single_line(&mut self, command: &str) -> Result<String, ConfError> {
        let response = self.request_raw(command, false)?;
        let line = response.lines().next().unwrap_or("");
        if let Some(rest) = line.strip_prefix("ERR ") {
            return Err(ConfError::Remote(String::from(rest)));
        }
        if line.is_empty() {
            return Err(ConfError::Protocol(String::from("empty response")));
        }
        Ok(String::from(line))
    }

    fn request_multi_line(&mut self, command: &str) -> Result<String, ConfError> {
        let response = self.request_raw(command, true)?;
        if let Some(first) = response.lines().next() {
            if let Some(rest) = first.strip_prefix("ERR ") {
                return Err(ConfError::Remote(String::from(rest)));
            }
        }
        Ok(response)
    }

    fn request_raw(&mut self, command: &str, expect_end: bool) -> Result<String, ConfError> {
        #[cfg(feature = "host")]
        {
            let _ = command;
            let _ = expect_end;
            Err(ConfError::NotRunning)
        }

        #[cfg(not(feature = "host"))]
        {
            let mut line = String::new();
            line.push_str(&self.tid.to_string());
            line.push('\t');
            line.push_str(&self.reply_pipe_name);
            line.push('\t');
            line.push_str(command);
            line.push('\n');
            if anyos_std::ipc::pipe_write(self.req_pipe, line.as_bytes()) == 0 {
                return Err(ConfError::Disconnected);
            }

            let mut data = Vec::new();
            let mut chunk = [0u8; READ_CHUNK_SIZE];
            let timeout_ms = if expect_end {
                MULTI_LINE_TIMEOUT_MS
            } else {
                SINGLE_LINE_TIMEOUT_MS
            };
            let overall_deadline = libsyscall::uptime_ms().wrapping_add(timeout_ms);
            let mut idle_deadline = libsyscall::uptime_ms().wrapping_add(READ_IDLE_GRACE_MS);

            loop {
                let n = anyos_std::ipc::pipe_read(self.reply_pipe, &mut chunk);
                if n == u32::MAX {
                    return Err(ConfError::Disconnected);
                }
                if n > 0 {
                    data.extend_from_slice(&chunk[..n as usize]);
                    idle_deadline = libsyscall::uptime_ms().wrapping_add(READ_IDLE_GRACE_MS);
                    if !expect_end {
                        if let Some(line) = take_first_line(&data) {
                            return Ok(line);
                        }
                    } else if data.ends_with(b"END\n") || data.starts_with(b"ERR ") {
                        let text = core::str::from_utf8(&data)
                            .map_err(|_| ConfError::Protocol(String::from("invalid UTF-8")))?;
                        return Ok(String::from(text));
                    }
                }
                if deadline_reached(overall_deadline)
                    || (!data.is_empty() && deadline_reached(idle_deadline))
                {
                    break;
                }
                anyos_std::process::sleep(REQUEST_POLL_SLEEP_MS);
            }

            Err(ConfError::Timeout)
        }
    }
}

impl Drop for ConfClient {
    fn drop(&mut self) {
        #[cfg(not(feature = "host"))]
        {
            if self.reply_pipe != 0 {
                anyos_std::ipc::pipe_close(self.reply_pipe);
            }
        }
    }
}

fn scope_name(scope: RegistryScope) -> &'static str {
    match scope {
        RegistryScope::System => "system",
        RegistryScope::User => "user",
    }
}

fn target_name(target: ConfTarget) -> String {
    match target {
        ConfTarget::Scope(scope) => String::from(scope_name(scope)),
        ConfTarget::User(uid) => format!("user@{}", uid),
    }
}

fn target_scope(target: ConfTarget) -> RegistryScope {
    match target {
        ConfTarget::Scope(scope) => scope,
        ConfTarget::User(_) => RegistryScope::User,
    }
}

fn validate_path(path: &str) -> Result<(), ConfError> {
    if path.is_empty() {
        return Ok(());
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return Err(ConfError::InvalidArgument("invalid path"));
    }
    if !path.split('/').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    }) {
        return Err(ConfError::InvalidArgument("invalid path"));
    }
    Ok(())
}

fn is_valid_token(token: &str) -> bool {
    token
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'/' | b'_' | b'-'))
}

fn encode_value(value: &ConfValue) -> Result<(&'static str, String), ConfError> {
    Ok(match value {
        ConfValue::String(s) => ("string", escape_value(s)),
        ConfValue::Int(v) => ("int", format!("{}", *v)),
        ConfValue::Bool(v) => ("bool", if *v { String::from("1") } else { String::from("0") }),
    })
}

fn encode_value_ref(value: ConfValueRef<'_>) -> (&'static str, String) {
    match value {
        ConfValueRef::String(s) => ("string", escape_value(s)),
        ConfValueRef::Int(v) => ("int", format!("{}", v)),
        ConfValueRef::Bool(v) => ("bool", if v { String::from("1") } else { String::from("0") }),
    }
}

fn append_manifest_field(
    out: &mut String,
    kind: &str,
    path: &str,
    value_type: Option<&str>,
    value_text: Option<&str>,
) {
    if !out.is_empty() {
        out.push(';');
    }
    out.push_str(kind);
    out.push('|');
    out.push_str(path);
    if let Some(value_type) = value_type {
        out.push('|');
        out.push_str(value_type);
    }
    if let Some(value_text) = value_text {
        out.push('|');
        out.push_str(value_text);
    }
}

fn escape_value(value: &str) -> String {
    if value.is_empty() {
        return String::from("%empty");
    }
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            ';' => out.push_str("%3B"),
            '|' => out.push_str("%7C"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_value(value: &str) -> String {
    if value == "%empty" {
        return String::new();
    }
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((hi << 4 | lo) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

fn parse_simple_ok(
    line: &str,
    verb: &str,
    scope: RegistryScope,
    path: &str,
) -> Result<ConfItem, ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("OK") || parts.next() != Some(verb) {
        return Err(ConfError::Protocol(String::from("unexpected acknowledgement")));
    }
    if parts.next() != Some(scope_name(scope)) {
        return Err(ConfError::Protocol(String::from("scope mismatch")));
    }
    if parts.next() != Some(path) {
        return Err(ConfError::Protocol(String::from("path mismatch")));
    }
    let version = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing timestamp")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid timestamp")))?;
    Ok(ConfItem {
        scope,
        path: String::from(path),
        kind: NodeKind::Value,
        value: None,
        version,
        updated_at,
    })
}

fn parse_item_line(line: &str) -> Result<ConfItem, ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("ITEM") {
        return Err(ConfError::Protocol(String::from("expected ITEM response")));
    }
    let scope = parse_scope(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing scope")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid scope")))?;
    let path = String::from(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing path")))?);
    let kind = match parts.next() {
        Some("dir") => NodeKind::Directory,
        Some("value") => NodeKind::Value,
        _ => return Err(ConfError::Protocol(String::from("invalid kind"))),
    };
    let value_type = parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing value type")))?;
    let raw_value = parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing value")))?;
    let version = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing timestamp")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid timestamp")))?;

    let value = match value_type {
        "none" => None,
        "string" => Some(ConfValue::String(unescape_value(raw_value))),
        "int" => Some(ConfValue::Int(
            parse_i64(raw_value).ok_or_else(|| ConfError::Protocol(String::from("invalid int")))?,
        )),
        "bool" => Some(ConfValue::Bool(matches!(raw_value, "1" | "true" | "TRUE"))),
        _ => return Err(ConfError::Protocol(String::from("invalid value type"))),
    };

    Ok(ConfItem {
        scope,
        path,
        kind,
        value,
        version,
        updated_at,
    })
}

fn parse_list_response(response: &str) -> Result<Vec<ConfItem>, ConfError> {
    let mut items = Vec::new();
    for line in response.lines() {
        if line == "END" || line.is_empty() {
            continue;
        }
        items.push(parse_item_line(line)?);
    }
    Ok(items)
}

fn parse_watch_ok(line: &str) -> Result<u32, ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("OK") || parts.next() != Some("watch") {
        return Err(ConfError::Protocol(String::from("expected watch acknowledgement")));
    }
    parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing watch id")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid watch id")))
}

fn parse_register_ok(line: &str, scope: RegistryScope, namespace: &str) -> Result<(u32, u32), ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("OK") || parts.next() != Some("register") {
        return Err(ConfError::Protocol(String::from("expected register acknowledgement")));
    }
    if parts.next() != Some(scope_name(scope)) {
        return Err(ConfError::Protocol(String::from("scope mismatch")));
    }
    if parts.next() != Some(namespace) {
        return Err(ConfError::Protocol(String::from("namespace mismatch")));
    }
    let schema_version = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing schema version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid schema version")))?;
    let applied_version = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing applied version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid applied version")))?;
    Ok((schema_version, applied_version))
}

fn parse_event_line(line: &str) -> Result<ConfEvent, ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("EVENT") {
        return Err(ConfError::Protocol(String::from("expected EVENT")));
    }
    let watch_id = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing watch id")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid watch id")))?;
    let action = String::from(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing action")))?);
    let scope = parse_scope(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing scope")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid scope")))?;
    let path = String::from(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing path")))?);
    let kind = match parts.next() {
        Some("dir") => NodeKind::Directory,
        Some("value") => NodeKind::Value,
        _ => return Err(ConfError::Protocol(String::from("invalid kind"))),
    };
    let value_type = parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing value type")))?;
    let raw_value = parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing value")))?;
    let version = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing timestamp")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid timestamp")))?;

    let value = match value_type {
        "none" => None,
        "string" => Some(ConfValue::String(unescape_value(raw_value))),
        "int" => Some(ConfValue::Int(
            parse_i64(raw_value).ok_or_else(|| ConfError::Protocol(String::from("invalid int")))?,
        )),
        "bool" => Some(ConfValue::Bool(matches!(raw_value, "1" | "true" | "TRUE"))),
        _ => return Err(ConfError::Protocol(String::from("invalid value type"))),
    };

    Ok(ConfEvent {
        watch_id,
        action,
        item: ConfItem {
            scope,
            path,
            kind,
            value,
            version,
            updated_at,
        },
    })
}

fn parse_audit_response(response: &str) -> Result<Vec<ConfAuditEntry>, ConfError> {
    let mut items = Vec::new();
    for line in response.lines() {
        if line == "END" || line.is_empty() {
            continue;
        }
        items.push(parse_audit_line(line)?);
    }
    Ok(items)
}

fn parse_audit_line(line: &str) -> Result<ConfAuditEntry, ConfError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("AUDIT") {
        return Err(ConfError::Protocol(String::from("expected AUDIT response")));
    }
    let seq = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing seq")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid seq")))?;
    let actor_uid = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing actor uid")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid actor uid")))? as u16;
    let owner_uid = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing owner uid")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid owner uid")))? as u16;
    let actor_name = unescape_value(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing actor name")))?);
    let tid = parse_u32(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing tid")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid tid")))?;
    let action = unescape_value(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing action")))?);
    let scope = parse_scope(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing scope")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid scope")))?;
    let path = String::from(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing path")))?);
    let status = unescape_value(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing status")))?);
    let detail = unescape_value(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing detail")))?);
    let version = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing version")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid version")))?;
    let at_ms = parse_u64(parts.next().ok_or_else(|| ConfError::Protocol(String::from("missing timestamp")))?)
        .ok_or_else(|| ConfError::Protocol(String::from("invalid timestamp")))?;

    Ok(ConfAuditEntry {
        seq,
        actor_uid,
        owner_uid,
        actor_name,
        tid,
        action,
        scope,
        path,
        status,
        detail,
        version,
        at_ms,
    })
}

fn parse_scope(raw: &str) -> Option<RegistryScope> {
    match raw {
        "system" => Some(RegistryScope::System),
        "user" => Some(RegistryScope::User),
        _ => None,
    }
}

fn take_first_line(data: &[u8]) -> Option<String> {
    let pos = data.iter().position(|b| *b == b'\n')?;
    let line = core::str::from_utf8(&data[..pos]).ok()?;
    Some(String::from(line))
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.parse().ok()
}

fn parse_u64(raw: &str) -> Option<u64> {
    raw.parse().ok()
}

fn parse_i64(raw: &str) -> Option<i64> {
    raw.parse().ok()
}

fn deadline_reached(deadline: u32) -> bool {
    libsyscall::uptime_ms().wrapping_sub(deadline) < 0x8000_0000
}
