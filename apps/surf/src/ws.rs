//! WebSocket connection management for the surf browser.
//!
//! Handles the TCP connection, HTTP Upgrade handshake, frame polling, and
//! callback delivery back into the libwebview JS runtime.

use anyos_std::net;
use alloc::string::String;
use alloc::vec::Vec;

use libwebview::js::websocket::{
    self, build_upgrade_request, parse_upgrade_response,
    encode_text_frame, encode_binary_frame, encode_close_frame, encode_ping_frame,
    encode_pong_frame, decode_frames, WsFrame,
};
use libwebview::js::{JsRuntime, PendingWsConnect, PendingWsSend, PendingWsClose};

// ═══════════════════════════════════════════════════════════
// Active connection state
// ═══════════════════════════════════════════════════════════

/// State of one live WebSocket connection.
pub struct WsConn {
    /// Matches the JS WebSocket object's `_ws_id`.
    pub id: u64,
    /// anyOS TCP socket handle.
    pub sock: u32,
    /// True if this socket is TLS-encrypted.
    pub is_tls: bool,
    /// Receive buffer: accumulates partial frames across poll calls.
    pub recv_buf: Vec<u8>,
    /// Index into `AppState::tabs` that owns this connection.
    pub tab_idx: usize,
}

// ═══════════════════════════════════════════════════════════
// Connect
// ═══════════════════════════════════════════════════════════

/// Process one `PendingWsConnect` from the JS runtime.
///
/// Performs the DNS lookup, TCP connect, optional TLS handshake, and the
/// HTTP/1.1 Upgrade exchange.  On success calls `runtime.ws_opened` and
/// pushes the connection into `conns`.  On failure calls `runtime.ws_error`.
pub fn handle_connect(
    req: PendingWsConnect,
    conns: &mut Vec<WsConn>,
    runtime: &mut JsRuntime,
    cookies: &crate::http::CookieJar,
    tab_idx: usize,
) {
    let (host, port, _path, is_tls, upgrade_bytes, _ws_key) =
        match build_upgrade_request(&req.url, req.id, &req.protocols) {
            Some(t) => t,
            None => {
                anyos_std::println!("[ws] invalid ws URL: {}", req.url);
                runtime.ws_error(req.id);
                return;
            }
        };

    // DNS.
    let mut ip = [0u8; 4];
    if net::dns(&host, &mut ip) != 0 {
        anyos_std::println!("[ws] DNS failed for {}", host);
        runtime.ws_error(req.id);
        return;
    }

    // TCP connect.
    let sock = net::tcp_connect(&ip, port, 5000);
    if sock == u32::MAX {
        anyos_std::println!("[ws] TCP connect failed to {}:{}", host, port);
        runtime.ws_error(req.id);
        return;
    }

    // TLS handshake if wss://.
    if is_tls {
        if crate::tls::connect(sock, &host) != 0 {
            anyos_std::println!("[ws] TLS handshake failed for {}", host);
            net::tcp_close(sock);
            runtime.ws_error(req.id);
            return;
        }
    }

    // Send HTTP Upgrade request.
    if !tcp_send_all(sock, is_tls, &upgrade_bytes) {
        anyos_std::println!("[ws] send Upgrade failed");
        net::tcp_close(sock);
        runtime.ws_error(req.id);
        return;
    }

    // Read HTTP response (101 Switching Protocols).
    let mut resp_buf = Vec::new();
    for _ in 0..50 {
        let mut tmp = [0u8; 2048];
        let n = tcp_recv(sock, is_tls, &mut tmp);
        if n == 0 { break; }
        resp_buf.extend_from_slice(&tmp[..n]);
        // The HTTP headers end with \r\n\r\n.
        if resp_buf.windows(4).any(|w| w == b"\r\n\r\n") { break; }
    }

    match parse_upgrade_response(&resp_buf) {
        Some(protocol) => {
            anyos_std::println!("[ws] connected to {} (proto='{}')", req.url, protocol);
            runtime.ws_opened(req.id, &protocol);
            conns.push(WsConn {
                id: req.id,
                sock,
                is_tls,
                recv_buf: Vec::new(),
                tab_idx,
            });
        }
        None => {
            anyos_std::println!("[ws] Upgrade handshake rejected for {}", req.url);
            net::tcp_close(sock);
            runtime.ws_error(req.id);
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Poll
// ═══════════════════════════════════════════════════════════

/// Poll all active connections for incoming data.
///
/// Decodes WebSocket frames and delivers messages to the JS runtime.
/// Automatically handles Ping frames (sends Pong) and Close frames.
/// Returns a list of IDs that were cleanly closed and should be removed.
pub fn poll_connections(conns: &mut Vec<WsConn>, runtime: &mut JsRuntime) -> Vec<u64> {
    let mut to_close = Vec::new();

    for conn in conns.iter_mut() {
        // Non-blocking read.
        let mut tmp = [0u8; 8192];
        let n = tcp_recv_nonblock(conn.sock, conn.is_tls, &mut tmp);
        if n > 0 {
            conn.recv_buf.extend_from_slice(&tmp[..n]);
        }

        if conn.recv_buf.is_empty() { continue; }

        let (frames, consumed) = decode_frames(&conn.recv_buf);
        if consumed > 0 {
            conn.recv_buf.drain(..consumed);
        }

        for frame in frames {
            match frame.opcode {
                0x1 => {
                    // Text frame.
                    let text = core::str::from_utf8(&frame.payload).unwrap_or("[invalid utf8]");
                    runtime.ws_message(conn.id, text);
                }
                0x2 => {
                    // Binary frame.
                    runtime.ws_message_binary(conn.id, &frame.payload);
                }
                0x8 => {
                    // Close frame — send echo and clean up.
                    let (code, reason) = frame.close_info();
                    let close_frame = encode_close_frame(code, reason, conn.id);
                    let _ = tcp_send_all(conn.sock, conn.is_tls, &close_frame);
                    net::tcp_close(conn.sock);
                    runtime.ws_closed(conn.id, code, reason, true);
                    to_close.push(conn.id);
                }
                0x9 => {
                    // Ping — reply with Pong.
                    let pong = encode_pong_frame(&frame.payload, conn.id);
                    let _ = tcp_send_all(conn.sock, conn.is_tls, &pong);
                }
                0xA => { /* Pong — ignore */ }
                _ => {}
            }
        }
    }

    to_close
}

// ═══════════════════════════════════════════════════════════
// Send / close
// ═══════════════════════════════════════════════════════════

/// Process pending `ws.send()` calls from the JS runtime.
pub fn handle_sends(sends: Vec<PendingWsSend>, conns: &mut Vec<WsConn>) {
    for send in sends {
        if let Some(conn) = conns.iter().find(|c| c.id == send.id) {
            let frame = if send.is_binary {
                encode_binary_frame(&send.data, send.id)
            } else {
                encode_text_frame(&send.data, send.id)
            };
            tcp_send_all(conn.sock, conn.is_tls, &frame);
        }
    }
}

/// Process pending `ws.close()` calls from the JS runtime.
/// Sends a Close frame and marks the connection for removal.
pub fn handle_closes(
    closes: Vec<PendingWsClose>,
    conns: &mut Vec<WsConn>,
    runtime: &mut JsRuntime,
) -> Vec<u64> {
    let mut removed = Vec::new();
    for close in closes {
        if let Some(conn) = conns.iter().find(|c| c.id == close.id) {
            let frame = encode_close_frame(close.code, &close.reason, close.id);
            let _ = tcp_send_all(conn.sock, conn.is_tls, &frame);
            net::tcp_close(conn.sock);
            runtime.ws_closed(close.id, close.code, &close.reason, true);
            removed.push(close.id);
        }
    }
    removed
}

/// Remove closed connection IDs from the pool and close their sockets.
pub fn remove_connections(conns: &mut Vec<WsConn>, ids: &[u64]) {
    conns.retain(|c| {
        if ids.contains(&c.id) {
            net::tcp_close(c.sock);
            false
        } else {
            true
        }
    });
}

// ═══════════════════════════════════════════════════════════
// Low-level TCP helpers
// ═══════════════════════════════════════════════════════════

/// Send all bytes, handling partial sends.  Returns `true` on success.
fn tcp_send_all(sock: u32, is_tls: bool, data: &[u8]) -> bool {
    if is_tls {
        crate::tls::send(data) >= 0
    } else {
        net::tcp_send(sock, data) != u32::MAX
    }
}

/// Blocking receive — reads up to `buf.len()` bytes.
fn tcp_recv(sock: u32, is_tls: bool, buf: &mut [u8]) -> usize {
    if is_tls {
        let n = crate::tls::recv(buf);
        if n <= 0 { 0 } else { n as usize }
    } else {
        let n = net::tcp_recv(sock, buf);
        if n == u32::MAX { 0 } else { n as usize }
    }
}

/// Non-blocking receive — returns 0 if no data is available.
fn tcp_recv_nonblock(sock: u32, is_tls: bool, buf: &mut [u8]) -> usize {
    // Use tcp_status to check if data is available before blocking.
    let status = net::tcp_status(sock);
    if status == 0 || status == u32::MAX { return 0; }
    tcp_recv(sock, is_tls, buf)
}
