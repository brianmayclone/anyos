#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

const PIPE_NAME: &str = "ami";
const MAX_READ_RETRIES: usize = 40;
const READ_CHUNK_SIZE: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AmiValue {
    String(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmiEventKind {
    Set,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmiItem {
    pub key: String,
    pub value: AmiValue,
    pub version: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmiEvent {
    pub watch_id: u32,
    pub kind: AmiEventKind,
    pub key: String,
    pub value: Option<AmiValue>,
    pub version: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AmiError {
    NotRunning,
    PipeCreateFailed,
    Disconnected,
    Timeout,
    Protocol(String),
    Remote(String),
    InvalidArgument(&'static str),
}

pub struct AmiClient {
    tid: u32,
    req_pipe: u32,
    reply_pipe: u32,
    service: String,
}

impl AmiClient {
    pub fn connect(service: &str) -> Result<Self, AmiError> {
        if service.is_empty() {
            return Err(AmiError::InvalidArgument("service must not be empty"));
        }
        if !is_valid_token(service) {
            return Err(AmiError::InvalidArgument(
                "service contains invalid characters",
            ));
        }

        #[cfg(feature = "host")]
        {
            Ok(Self {
                tid: 1,
                req_pipe: 1,
                reply_pipe: 1,
                service: service.to_string(),
            })
        }

        #[cfg(not(feature = "host"))]
        {
            let tid = libsyscall::get_tid();
            let req_pipe = pipe_open(PIPE_NAME);
            if req_pipe == 0 {
                return Err(AmiError::NotRunning);
            }

            let mut reply_name = String::from("ami-");
            push_u32(&mut reply_name, tid);
            let reply_pipe = pipe_create(&reply_name);
            if reply_pipe == 0 {
                return Err(AmiError::PipeCreateFailed);
            }

            let mut client = Self {
                tid,
                req_pipe,
                reply_pipe,
                service: service.to_string(),
            };
            client.hello()?;
            Ok(client)
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn set(&mut self, key: &str, value: AmiValue) -> Result<AmiItem, AmiError> {
        validate_key(key)?;
        let (type_name, value_str) = encode_value(&value)?;
        let mut cmd = String::from("SET ");
        cmd.push_str(key);
        cmd.push(' ');
        cmd.push_str(type_name);
        cmd.push(' ');
        cmd.push_str(&value_str);

        let line = self.request_single_line(&cmd)?;
        parse_set_or_del_ok(&line, "set", key, Some(value))
    }

    pub fn get(&mut self, key: &str) -> Result<AmiItem, AmiError> {
        validate_key(key)?;
        let mut cmd = String::from("GET ");
        cmd.push_str(key);
        let line = self.request_single_line(&cmd)?;
        parse_value_line(&line)
    }

    pub fn del(&mut self, key: &str) -> Result<(), AmiError> {
        validate_key(key)?;
        let mut cmd = String::from("DEL ");
        cmd.push_str(key);
        let line = self.request_single_line(&cmd)?;
        let _ = parse_set_or_del_ok(&line, "del", key, None)?;
        Ok(())
    }

    pub fn list(&mut self, prefix: &str) -> Result<Vec<AmiItem>, AmiError> {
        validate_prefix(prefix)?;
        let mut cmd = String::from("LIST ");
        cmd.push_str(prefix);
        let resp = self.request_multi_line(&cmd)?;
        parse_list_response(&resp)
    }

    pub fn watch(&mut self, prefix: &str) -> Result<u32, AmiError> {
        validate_prefix(prefix)?;
        let mut cmd = String::from("WATCH ");
        cmd.push_str(prefix);
        let line = self.request_single_line(&cmd)?;
        parse_watch_ok(&line)
    }

    pub fn unwatch(&mut self, watch_id: u32) -> Result<(), AmiError> {
        let mut cmd = String::from("UNWATCH ");
        push_u32(&mut cmd, watch_id);
        let line = self.request_single_line(&cmd)?;
        parse_unwatch_ok(&line, watch_id)
    }

    pub fn ping(&mut self) -> Result<(), AmiError> {
        let line = self.request_single_line("PING")?;
        if line == "PONG" {
            Ok(())
        } else {
            Err(AmiError::Protocol(String::from("expected PONG")))
        }
    }

    pub fn poll_event(&mut self, timeout_ms: u32) -> Result<Option<AmiEvent>, AmiError> {
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
                let n = pipe_read(self.reply_pipe, &mut chunk);
                if n > 0 && n != u32::MAX {
                    data.extend_from_slice(&chunk[..n as usize]);
                    if let Some(line) = take_first_line(&data) {
                        return parse_event_line(&line).map(Some);
                    }
                } else if n == u32::MAX {
                    return Err(AmiError::Disconnected);
                }

                if timeout_ms == 0 || is_deadline_reached(deadline) {
                    return Ok(None);
                }
                libsyscall::sleep(10);
            }
        }
    }

    fn hello(&mut self) -> Result<(), AmiError> {
        let mut cmd = String::from("HELLO ");
        cmd.push_str(&self.service);
        let line = self.request_single_line(&cmd)?;
        if line.starts_with("OK hello ") {
            Ok(())
        } else {
            Err(AmiError::Protocol(String::from(
                "expected HELLO acknowledgement",
            )))
        }
    }

    fn request_single_line(&mut self, command: &str) -> Result<String, AmiError> {
        let response = self.request_raw(command, false)?;
        let line = response.lines().next().unwrap_or("");
        if let Some(rest) = line.strip_prefix("ERR ") {
            return Err(AmiError::Remote(String::from(rest)));
        }
        if line.is_empty() {
            return Err(AmiError::Protocol(String::from("empty response")));
        }
        Ok(String::from(line))
    }

    fn request_multi_line(&mut self, command: &str) -> Result<String, AmiError> {
        let response = self.request_raw(command, true)?;
        if let Some(first) = response.lines().next() {
            if let Some(rest) = first.strip_prefix("ERR ") {
                return Err(AmiError::Remote(String::from(rest)));
            }
        }
        Ok(response)
    }

    fn request_raw(&mut self, command: &str, expect_end: bool) -> Result<String, AmiError> {
        #[cfg(feature = "host")]
        {
            let _ = command;
            let _ = expect_end;
            Err(AmiError::NotRunning)
        }

        #[cfg(not(feature = "host"))]
        {
            let req = format_request(self.tid, command);
            let written = pipe_write(self.req_pipe, req.as_bytes());
            if written == u32::MAX {
                return Err(AmiError::Disconnected);
            }

            let mut data = Vec::new();
            let mut chunk = [0u8; READ_CHUNK_SIZE];
            for _ in 0..MAX_READ_RETRIES {
                let n = pipe_read(self.reply_pipe, &mut chunk);
                if n > 0 && n != u32::MAX {
                    data.extend_from_slice(&chunk[..n as usize]);
                    let text = match core::str::from_utf8(&data) {
                        Ok(s) => s,
                        Err(_) => {
                            libsyscall::sleep(10);
                            continue;
                        }
                    };

                    if expect_end {
                        if text.lines().any(|line| line == "END") {
                            return Ok(String::from(text));
                        }
                    } else if text.contains('\n') {
                        return Ok(String::from(text));
                    }
                } else if n == u32::MAX {
                    return Err(AmiError::Disconnected);
                }
                libsyscall::sleep(10);
            }

            Err(AmiError::Timeout)
        }
    }
}

impl Drop for AmiClient {
    fn drop(&mut self) {
        #[cfg(not(feature = "host"))]
        {
            if self.reply_pipe != 0 && self.reply_pipe != u32::MAX {
                pipe_close(self.reply_pipe);
            }
        }
    }
}

fn parse_set_or_del_ok(
    line: &str,
    kind: &str,
    expected_key: &str,
    value: Option<AmiValue>,
) -> Result<AmiItem, AmiError> {
    let mut parts = line.split(' ');
    if parts.next() != Some("OK") || parts.next() != Some(kind) {
        return Err(AmiError::Protocol(String::from("unexpected OK response")));
    }
    let key = String::from(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing key")))?,
    );
    if key != expected_key {
        return Err(AmiError::Protocol(String::from("response key mismatch")));
    }
    let version = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing version")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing updated_at")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid updated_at")))?;
    Ok(AmiItem {
        key,
        value: value.unwrap_or(AmiValue::Bool(false)),
        version,
        updated_at,
    })
}

fn parse_value_line(line: &str) -> Result<AmiItem, AmiError> {
    let mut parts = line.splitn(7, ' ');
    if parts.next() != Some("VALUE") {
        return Err(AmiError::Protocol(String::from("expected VALUE")));
    }
    parse_item_fields(parts)
}

fn parse_item_line(line: &str) -> Result<AmiItem, AmiError> {
    let mut parts = line.splitn(7, ' ');
    if parts.next() != Some("ITEM") {
        return Err(AmiError::Protocol(String::from("expected ITEM")));
    }
    parse_item_fields(parts)
}

fn parse_item_fields<'a, I>(mut parts: I) -> Result<AmiItem, AmiError>
where
    I: Iterator<Item = &'a str>,
{
    let key = String::from(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing key")))?,
    );
    let ty = parts
        .next()
        .ok_or_else(|| AmiError::Protocol(String::from("missing type")))?;
    let raw = parts
        .next()
        .ok_or_else(|| AmiError::Protocol(String::from("missing value")))?;
    let version = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing version")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing updated_at")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid updated_at")))?;
    let value = decode_value(ty, raw)?;
    Ok(AmiItem {
        key,
        value,
        version,
        updated_at,
    })
}

fn parse_list_response(resp: &str) -> Result<Vec<AmiItem>, AmiError> {
    let mut items = Vec::new();
    for line in resp.lines() {
        if line.is_empty() {
            continue;
        }
        if line == "END" {
            return Ok(items);
        }
        items.push(parse_item_line(line)?);
    }
    Err(AmiError::Protocol(String::from(
        "missing END in LIST response",
    )))
}

fn parse_watch_ok(line: &str) -> Result<u32, AmiError> {
    let mut parts = line.split(' ');
    if parts.next() != Some("OK") || parts.next() != Some("watch") {
        return Err(AmiError::Protocol(String::from(
            "expected watch acknowledgement",
        )));
    }
    parse_u32(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing watch id")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid watch id")))
}

fn parse_unwatch_ok(line: &str, expected_watch_id: u32) -> Result<(), AmiError> {
    let mut parts = line.split(' ');
    if parts.next() != Some("OK") || parts.next() != Some("unwatch") {
        return Err(AmiError::Protocol(String::from(
            "expected unwatch acknowledgement",
        )));
    }
    let watch_id = parse_u32(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing watch id")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid watch id")))?;
    if watch_id != expected_watch_id {
        return Err(AmiError::Protocol(String::from("watch id mismatch")));
    }
    Ok(())
}

fn parse_event_line(line: &str) -> Result<AmiEvent, AmiError> {
    let mut parts = line.splitn(8, ' ');
    if parts.next() != Some("EVENT") {
        return Err(AmiError::Protocol(String::from("expected EVENT")));
    }
    let watch_id = parse_u32(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing watch id")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid watch id")))?;
    let kind = match parts
        .next()
        .ok_or_else(|| AmiError::Protocol(String::from("missing event kind")))?
    {
        "set" => AmiEventKind::Set,
        "delete" => AmiEventKind::Delete,
        _ => return Err(AmiError::Protocol(String::from("invalid event kind"))),
    };
    let key = String::from(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing key")))?,
    );
    let ty = parts
        .next()
        .ok_or_else(|| AmiError::Protocol(String::from("missing type")))?;
    let raw = parts
        .next()
        .ok_or_else(|| AmiError::Protocol(String::from("missing value")))?;
    let version = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing version")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid version")))?;
    let updated_at = parse_u64(
        parts
            .next()
            .ok_or_else(|| AmiError::Protocol(String::from("missing updated_at")))?,
    )
    .ok_or_else(|| AmiError::Protocol(String::from("invalid updated_at")))?;
    let value = if kind == AmiEventKind::Delete {
        None
    } else {
        Some(decode_value(ty, raw)?)
    };
    Ok(AmiEvent {
        watch_id,
        kind,
        key,
        value,
        version,
        updated_at,
    })
}

fn encode_value(value: &AmiValue) -> Result<(&'static str, String), AmiError> {
    match value {
        AmiValue::String(s) => {
            if s.contains('\n') || s.contains('\t') {
                return Err(AmiError::InvalidArgument(
                    "string values must not contain tabs or newlines",
                ));
            }
            Ok(("string", s.clone()))
        }
        AmiValue::Int(v) => Ok(("int", v.to_string())),
        AmiValue::Bool(v) => Ok((
            "bool",
            if *v {
                String::from("true")
            } else {
                String::from("false")
            },
        )),
    }
}

fn decode_value(ty: &str, raw: &str) -> Result<AmiValue, AmiError> {
    match ty {
        "string" => Ok(AmiValue::String(String::from(raw))),
        "int" => parse_i64(raw)
            .map(AmiValue::Int)
            .ok_or_else(|| AmiError::Protocol(String::from("invalid int value"))),
        "bool" => match raw {
            "true" => Ok(AmiValue::Bool(true)),
            "false" => Ok(AmiValue::Bool(false)),
            _ => Err(AmiError::Protocol(String::from("invalid bool value"))),
        },
        _ => Err(AmiError::Protocol(String::from("unknown value type"))),
    }
}

fn validate_key(key: &str) -> Result<(), AmiError> {
    if key.is_empty() {
        return Err(AmiError::InvalidArgument("key must not be empty"));
    }
    if !key.bytes().all(is_valid_key_byte) {
        return Err(AmiError::InvalidArgument("key contains invalid characters"));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), AmiError> {
    if prefix.is_empty() {
        return Ok(());
    }
    if !prefix.bytes().all(is_valid_key_byte) {
        return Err(AmiError::InvalidArgument(
            "prefix contains invalid characters",
        ));
    }
    Ok(())
}

fn is_valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
}

fn is_valid_key_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-')
}

fn format_request(tid: u32, command: &str) -> String {
    let mut req = String::new();
    push_u32(&mut req, tid);
    req.push('\t');
    req.push_str(command);
    req.push('\n');
    req
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn parse_u64(s: &str) -> Option<u64> {
    let mut val = 0u64;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(val)
}

fn parse_i64(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (neg, start) = if bytes[0] == b'-' {
        (true, 1usize)
    } else {
        (false, 0usize)
    };
    if start >= bytes.len() {
        return None;
    }
    let mut val = 0i64;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    Some(if neg { -val } else { val })
}

fn push_u32(out: &mut String, mut v: u32) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        out.push(buf[n] as char);
    }
}

fn take_first_line(data: &[u8]) -> Option<String> {
    let pos = data.iter().position(|&b| b == b'\n')?;
    core::str::from_utf8(&data[..pos]).ok().map(String::from)
}

#[cfg(not(feature = "host"))]
fn is_deadline_reached(deadline: u32) -> bool {
    libsyscall::uptime_ms().wrapping_sub(deadline) < 0x8000_0000
}

#[cfg(not(feature = "host"))]
fn pipe_create(name: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = name.len().min(256);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    libsyscall::syscall1(libsyscall::SYS_PIPE_CREATE, buf.as_ptr() as u64) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_open(name: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = name.len().min(256);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    libsyscall::syscall1(libsyscall::SYS_PIPE_OPEN, buf.as_ptr() as u64) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    libsyscall::syscall3(
        libsyscall::SYS_PIPE_READ,
        pipe_id as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
    ) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_write(pipe_id: u32, data: &[u8]) -> u32 {
    libsyscall::syscall3(
        libsyscall::SYS_PIPE_WRITE,
        pipe_id as u64,
        data.as_ptr() as u64,
        data.len() as u64,
    ) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_close(pipe_id: u32) -> u32 {
    libsyscall::syscall1(libsyscall::SYS_PIPE_CLOSE, pipe_id as u64) as u32
}
