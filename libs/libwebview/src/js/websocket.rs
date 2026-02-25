//! Native WebSocket host object — RFC 6455.
//!
//! `new WebSocket(url[, protocols])` creates a WS object and records a
//! `PendingWsConnect` for the host application (surf) to handle.  The host
//! performs the TCP connection + HTTP Upgrade handshake and then calls back
//! into the JS runtime via `ws_opened` / `ws_message` / `ws_error` /
//! `ws_closed`.
//!
//! Sending is handled via `PendingWsSend` mutations that surf picks up each
//! poll cycle.  Frames are encoded here (RFC 6455, client-side masking).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::{get_bridge, arg_string};
use super::{PendingWsConnect, PendingWsSend, PendingWsClose};

// ═══════════════════════════════════════════════════════════
// WebSocket ID allocator
// ═══════════════════════════════════════════════════════════

static mut NEXT_WS_ID: u64 = 1;

fn alloc_ws_id() -> u64 {
    unsafe {
        let id = NEXT_WS_ID;
        NEXT_WS_ID += 1;
        id
    }
}

// ═══════════════════════════════════════════════════════════
// Public constructor
// ═══════════════════════════════════════════════════════════

/// Create the `WebSocket` global constructor.
pub fn make_ws_constructor() -> JsValue {
    native_fn("WebSocket", ws_ctor)
}

/// `new WebSocket(url[, protocols])` constructor body.
fn ws_ctor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let url = arg_string(args, 0);
    if url.is_empty() {
        return JsValue::Null;
    }

    // Optional sub-protocols (string or array of strings).
    let protocols: Vec<String> = match args.get(1) {
        Some(JsValue::String(s)) => {
            let s = s.clone();
            vec![s]
        }
        Some(JsValue::Array(arr)) => arr
            .borrow()
            .elements
            .iter()
            .map(|v| v.to_js_string())
            .collect(),
        _ => Vec::new(),
    };

    let ws_id = alloc_ws_id();

    let mut obj = JsObject::new();

    // readyState constants.
    obj.set(String::from("CONNECTING"), JsValue::Number(0.0));
    obj.set(String::from("OPEN"),       JsValue::Number(1.0));
    obj.set(String::from("CLOSING"),    JsValue::Number(2.0));
    obj.set(String::from("CLOSED"),     JsValue::Number(3.0));

    // State.
    obj.set(String::from("readyState"),     JsValue::Number(0.0)); // CONNECTING
    obj.set(String::from("url"),            JsValue::String(url.clone()));
    obj.set(String::from("protocol"),       JsValue::String(String::new()));
    obj.set(String::from("extensions"),     JsValue::String(String::new()));
    obj.set(String::from("bufferedAmount"), JsValue::Number(0.0));
    obj.set(String::from("binaryType"),     JsValue::String(String::from("blob")));

    // Internal: unique socket ID used to route callbacks back to this object.
    obj.set(String::from("_ws_id"), JsValue::Number(ws_id as f64));

    // Event handlers (set by the script).
    obj.set(String::from("onopen"),    JsValue::Null);
    obj.set(String::from("onmessage"), JsValue::Null);
    obj.set(String::from("onerror"),   JsValue::Null);
    obj.set(String::from("onclose"),   JsValue::Null);

    // Methods.
    obj.set(String::from("send"),              native_fn("send",              ws_send));
    obj.set(String::from("close"),             native_fn("close",             ws_close));
    obj.set(String::from("addEventListener"),  native_fn("addEventListener",  ws_add_event_listener));
    obj.set(String::from("removeEventListener"), native_fn("removeEventListener", ws_noop));
    obj.set(String::from("dispatchEvent"),     native_fn("dispatchEvent",     ws_noop));

    let ws_val = JsValue::Object(Rc::new(RefCell::new(obj)));

    // Record the pending connection request for surf to handle.
    if let Some(bridge) = get_bridge(vm) {
        bridge.pending_ws_connects.push(PendingWsConnect {
            id: ws_id,
            url,
            protocols,
        });
        // Keep a clone of the JsObject so we can deliver callbacks later.
        bridge.ws_registry.push((ws_id, ws_val.clone()));
    }

    ws_val
}

// ═══════════════════════════════════════════════════════════
// WebSocket methods
// ═══════════════════════════════════════════════════════════

/// `ws.send(data)` — queue a text frame to be sent by surf.
fn ws_send(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let data = arg_string(args, 0);
    let ws_id = get_this_ws_id(vm);
    if ws_id == 0 { return JsValue::Undefined; }

    // Only allowed in OPEN state (readyState == 1).
    let ready_state = get_this_prop(vm, "readyState").to_number() as u8;
    if ready_state != 1 { return JsValue::Undefined; }

    if let Some(bridge) = get_bridge(vm) {
        bridge.pending_ws_sends.push(PendingWsSend {
            id: ws_id,
            data: data.into_bytes(),
            is_binary: false,
        });
    }
    JsValue::Undefined
}

/// `ws.close([code[, reason]])` — initiate the closing handshake.
fn ws_close(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let ws_id = get_this_ws_id(vm);
    if ws_id == 0 { return JsValue::Undefined; }

    let ready_state = get_this_prop(vm, "readyState").to_number() as u8;
    if ready_state == 2 || ready_state == 3 { return JsValue::Undefined; }

    let code = args.first().map(|v| v.to_number() as u16).unwrap_or(1000);
    let reason = arg_string(args, 1);

    set_this_prop(vm, "readyState", JsValue::Number(2.0)); // CLOSING

    if let Some(bridge) = get_bridge(vm) {
        bridge.pending_ws_closes.push(PendingWsClose { id: ws_id, code, reason });
    }
    JsValue::Undefined
}

/// `ws.addEventListener(type, cb)` — convenience alias for `ws.onXxx = cb`.
fn ws_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event_type = arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Null);
    let prop_name = match event_type.as_str() {
        "open"    => "onopen",
        "message" => "onmessage",
        "error"   => "onerror",
        "close"   => "onclose",
        _         => return JsValue::Undefined,
    };
    set_this_prop(vm, prop_name, callback);
    JsValue::Undefined
}

fn ws_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }

// ═══════════════════════════════════════════════════════════
// RFC 6455 frame codec (client-side)
// ═══════════════════════════════════════════════════════════

/// Masking key counter — XORed with `ws_id` for per-connection uniqueness.
static mut MASK_COUNTER: u32 = 0x37A5_9C21;

/// Generate a pseudo-random 4-byte masking key.
/// Uses a simple LCG seeded with a global counter + the WS id.
fn next_mask_key(ws_id: u64) -> [u8; 4] {
    let seed = unsafe {
        MASK_COUNTER = MASK_COUNTER.wrapping_mul(1664525).wrapping_add(1013904223);
        MASK_COUNTER
    };
    let combined = seed ^ (ws_id as u32).wrapping_mul(2654435761);
    combined.to_le_bytes()
}

/// Encode a text (opcode 0x1) WebSocket frame, client-side masked (RFC 6455 §6.1).
pub fn encode_text_frame(data: &[u8], ws_id: u64) -> Vec<u8> {
    encode_frame(0x81, data, ws_id) // FIN=1, opcode=text
}

/// Encode a binary (opcode 0x2) WebSocket frame, client-side masked.
pub fn encode_binary_frame(data: &[u8], ws_id: u64) -> Vec<u8> {
    encode_frame(0x82, data, ws_id) // FIN=1, opcode=binary
}

/// Encode a close (opcode 0x8) frame with status code and reason.
pub fn encode_close_frame(code: u16, reason: &str, ws_id: u64) -> Vec<u8> {
    let mut payload: Vec<u8> = Vec::with_capacity(2 + reason.len());
    payload.push((code >> 8) as u8);
    payload.push((code & 0xFF) as u8);
    payload.extend_from_slice(reason.as_bytes());
    encode_frame(0x88, &payload, ws_id) // FIN=1, opcode=close
}

/// Encode a ping (opcode 0x9) frame.
pub fn encode_ping_frame(ws_id: u64) -> Vec<u8> {
    encode_frame(0x89, &[], ws_id)
}

/// Encode a pong (opcode 0xA) frame, echoing the ping payload.
pub fn encode_pong_frame(payload: &[u8], ws_id: u64) -> Vec<u8> {
    encode_frame(0x8A, payload, ws_id)
}

/// Core frame encoder — applies MASK bit and masking key to the payload.
fn encode_frame(first_byte: u8, payload: &[u8], ws_id: u64) -> Vec<u8> {
    let mask = next_mask_key(ws_id);
    let plen = payload.len();

    // Header size: 2 bytes + 2 or 8 bytes for extended length + 4 mask bytes.
    let header_len = if plen < 126 { 2 } else if plen < 65536 { 4 } else { 10 };
    let mut frame = Vec::with_capacity(header_len + 4 + plen);

    // Byte 0: FIN + opcode.
    frame.push(first_byte);

    // Byte 1: MASK=1 + payload length.
    if plen < 126 {
        frame.push(0x80 | plen as u8);
    } else if plen < 65536 {
        frame.push(0x80 | 126u8);
        frame.push((plen >> 8) as u8);
        frame.push((plen & 0xFF) as u8);
    } else {
        frame.push(0x80 | 127u8);
        for i in (0..8).rev() {
            frame.push(((plen >> (i * 8)) & 0xFF) as u8);
        }
    }

    // 4-byte masking key.
    frame.extend_from_slice(&mask);

    // Masked payload.
    for (i, &b) in payload.iter().enumerate() {
        frame.push(b ^ mask[i % 4]);
    }

    frame
}

// ═══════════════════════════════════════════════════════════
// Frame decoder (server → client, NOT masked per RFC)
// ═══════════════════════════════════════════════════════════

/// A decoded WebSocket frame.
#[derive(Clone)]
pub struct WsFrame {
    pub opcode: u8,
    pub payload: Vec<u8>,
    pub fin: bool,
}

impl WsFrame {
    /// True if this is a text data frame (opcode 0x1 or continuation).
    pub fn is_text(&self)   -> bool { self.opcode == 0x1 || self.opcode == 0x0 }
    /// True if this is a binary data frame.
    pub fn is_binary(&self) -> bool { self.opcode == 0x2 }
    /// True if this is a close control frame.
    pub fn is_close(&self)  -> bool { self.opcode == 0x8 }
    /// True if this is a ping control frame.
    pub fn is_ping(&self)   -> bool { self.opcode == 0x9 }
    /// True if this is a pong control frame.
    pub fn is_pong(&self)   -> bool { self.opcode == 0xA }

    /// Extract the close code and reason from a close frame payload.
    pub fn close_info(&self) -> (u16, &str) {
        if self.payload.len() >= 2 {
            let code = ((self.payload[0] as u16) << 8) | self.payload[1] as u16;
            let reason = core::str::from_utf8(&self.payload[2..]).unwrap_or("");
            (code, reason)
        } else {
            (1000, "")
        }
    }
}

/// Decode as many complete frames as possible from `buf`.
/// Returns the decoded frames and the number of bytes consumed.
/// Incomplete frames leave the remaining bytes in `buf` for the next call.
pub fn decode_frames(buf: &[u8]) -> (Vec<WsFrame>, usize) {
    let mut frames = Vec::new();
    let mut pos = 0usize;

    while pos < buf.len() {
        // Need at least 2 header bytes.
        if pos + 2 > buf.len() { break; }

        let b0 = buf[pos];
        let b1 = buf[pos + 1];
        let fin    = (b0 & 0x80) != 0;
        let opcode = b0 & 0x0F;
        let masked = (b1 & 0x80) != 0;
        let len7   = (b1 & 0x7F) as usize;

        let (payload_len, header_extra): (usize, usize) = if len7 < 126 {
            (len7, 0)
        } else if len7 == 126 {
            if pos + 4 > buf.len() { break; } // wait for more data
            let n = ((buf[pos + 2] as usize) << 8) | buf[pos + 3] as usize;
            (n, 2)
        } else {
            if pos + 10 > buf.len() { break; }
            let mut n = 0usize;
            for i in 0..8 {
                n = (n << 8) | buf[pos + 2 + i] as usize;
            }
            (n, 8)
        };

        let header_len = 2 + header_extra + if masked { 4 } else { 0 };

        // Wait until the full frame is in the buffer.
        if pos + header_len + payload_len > buf.len() { break; }

        let mask_offset = 2 + header_extra;
        let data_offset = mask_offset + if masked { 4 } else { 0 };

        let raw = &buf[pos + data_offset .. pos + data_offset + payload_len];
        let payload: Vec<u8> = if masked {
            let mk = &buf[pos + mask_offset .. pos + mask_offset + 4];
            raw.iter().enumerate().map(|(i, &b)| b ^ mk[i % 4]).collect()
        } else {
            raw.to_vec()
        };

        frames.push(WsFrame { opcode, payload, fin });
        pos += header_len + payload_len;
    }

    (frames, pos)
}

// ═══════════════════════════════════════════════════════════
// HTTP Upgrade handshake helpers
// ═══════════════════════════════════════════════════════════

/// Base64 character table.
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as base64 (no line wrapping).
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((triple >> 18) & 63) as usize] as char);
        out.push(B64[((triple >> 12) & 63) as usize] as char);
        out.push(if i + 1 < input.len() { B64[((triple >> 6) & 63) as usize] as char } else { '=' });
        out.push(if i + 2 < input.len() { B64[(triple & 63) as usize] as char } else { '=' });
        i += 3;
    }
    out
}

/// Generate a random 16-byte nonce for the `Sec-WebSocket-Key` header.
/// Uses a simple LCG seeded by the current WS ID counter.
pub fn generate_ws_key(ws_id: u64) -> String {
    let mut bytes = [0u8; 16];
    let mut state: u64 = ws_id.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    for chunk in bytes.chunks_mut(8) {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let b = state.to_le_bytes();
        for (d, s) in chunk.iter_mut().zip(b.iter()) {
            *d = *s;
        }
    }
    base64_encode(&bytes)
}

/// Build the HTTP/1.1 Upgrade request for a WebSocket URL.
///
/// `ws_url` — the full `ws://` or `wss://` URL.
/// Returns `(host, port, path, is_tls, request_bytes, ws_key)`.
pub fn build_upgrade_request(
    ws_url: &str,
    ws_id: u64,
    protocols: &[String],
) -> Option<(String, u16, String, bool, Vec<u8>, String)> {
    // Parse ws[s]://host[:port]/path
    let (is_tls, after_scheme) = if ws_url.starts_with("wss://") {
        (true, &ws_url[6..])
    } else if ws_url.starts_with("ws://") {
        (false, &ws_url[5..])
    } else {
        return None;
    };

    let (host_port, path) = if let Some(p) = after_scheme.find('/') {
        (&after_scheme[..p], &after_scheme[p..])
    } else {
        (after_scheme, "/")
    };

    let (host, port) = if let Some(p) = host_port.rfind(':') {
        let maybe_port = &host_port[p + 1..];
        if maybe_port.bytes().all(|b| b.is_ascii_digit()) {
            let port: u16 = maybe_port.parse().unwrap_or(if is_tls { 443 } else { 80 });
            (&host_port[..p], port)
        } else {
            (host_port, if is_tls { 443 } else { 80 })
        }
    } else {
        (host_port, if is_tls { 443 } else { 80 })
    };

    let ws_key = generate_ws_key(ws_id);

    let mut req = String::new();
    req.push_str("GET ");
    req.push_str(path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(host);
    req.push_str("\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ");
    req.push_str(&ws_key);
    req.push_str("\r\nSec-WebSocket-Version: 13\r\nOrigin: null\r\n");

    if !protocols.is_empty() {
        req.push_str("Sec-WebSocket-Protocol: ");
        for (i, p) in protocols.iter().enumerate() {
            if i > 0 { req.push_str(", "); }
            req.push_str(p);
        }
        req.push_str("\r\n");
    }

    req.push_str("\r\n");

    Some((
        String::from(host),
        port,
        String::from(path),
        is_tls,
        req.into_bytes(),
        ws_key,
    ))
}

/// Check if the HTTP response is a valid 101 Switching Protocols.
/// Returns the negotiated sub-protocol (if any) on success.
pub fn parse_upgrade_response(response: &[u8]) -> Option<String> {
    let text = core::str::from_utf8(response).ok()?;
    // Must start with "HTTP/1.1 101"
    if !text.starts_with("HTTP/1.1 101") && !text.starts_with("HTTP/1.0 101") {
        return None;
    }
    // Extract Sec-WebSocket-Protocol if present.
    let protocol = text.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("sec-websocket-protocol:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|v| String::from(v.trim()))
        .unwrap_or_else(String::new);
    Some(protocol)
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

fn get_this_ws_id(vm: &Vm) -> u64 {
    get_this_prop(vm, "_ws_id").to_number() as u64
}

fn get_this_prop(vm: &Vm, name: &str) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        return obj.borrow().get(name);
    }
    JsValue::Undefined
}

fn set_this_prop(vm: &Vm, name: &str, val: JsValue) {
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from(name), val);
    }
}
