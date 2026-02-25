//! RFB 003.008 protocol state machine for the VNC daemon.
//!
//! # Session lifecycle
//! ```text
//! VersionHandshake → SecurityNegotiation → VncAuth → ClientInit
//!   → ServerInit → LoginScreen (OS credential prompt) → MainLoop
//! ```
//!
//! **VersionHandshake** — exchange `"RFB 003.008\n"` strings.
//!
//! **SecurityNegotiation** — server offers `[1u8, 2u8]` (None + VNC Auth).
//! Client selects type 2; server sends 16-byte DES challenge, client responds.
//! The global VNC password from `config.rs` is used for this step.
//!
//! **LoginScreen** — after VNC auth passes, the session enters an anyOS login
//! prompt rendered with `login_ui.rs`.  The user types their OS username and
//! password; credentials are verified via `process::authenticate()`.  Up to
//! `MAX_LOGIN_ATTEMPTS` failures are allowed before the connection is closed.
//!
//! **MainLoop** — real desktop pixels are streamed via `capture_screen`.
//! Keyboard events are mapped with `input::map_keysym` and injected via
//! `CMD_INJECT_KEY`.  Mouse events go via `CMD_INJECT_POINTER`.

use anyos_std::net;
use anyos_std::process;
use anyos_std::sys;

use crate::config::VncConfig;
use crate::des;
use crate::input::{self, ModifierState};
use crate::login_ui::{self, LoginState, LOGIN_H, LOGIN_W};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// Maximum failed OS login attempts before the connection is dropped.
const MAX_LOGIN_ATTEMPTS: u32 = 3;

/// Minimum interval between framebuffer updates sent to the client (ms).
/// Caps the screen capture rate at ~30 fps.
const MIN_FRAME_INTERVAL_MS: u32 = 33;

/// Maximum screen dimension we support (guards heap allocation).
const MAX_SCREEN_DIM: usize = 2048;

// ── Big-endian wire helpers ───────────────────────────────────────────────────

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

fn from_be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn from_be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

// ── TCP helpers ───────────────────────────────────────────────────────────────

/// Send all bytes in `data`, retrying until complete or error.
/// Returns `false` if the socket was closed or an error occurred.
fn send_all(sock: u32, data: &[u8]) -> bool {
    let mut sent = 0usize;
    while sent < data.len() {
        let n = net::tcp_send(sock, &data[sent..]);
        if n == u32::MAX || n == 0 {
            return false;
        }
        sent += n as usize;
    }
    true
}

/// Receive exactly `buf.len()` bytes.
/// Returns `false` on EOF or error.
fn recv_exact(sock: u32, buf: &mut [u8]) -> bool {
    let mut received = 0usize;
    while received < buf.len() {
        let n = net::tcp_recv(sock, &mut buf[received..]);
        if n == 0 || n == u32::MAX {
            return false;
        }
        received += n as usize;
    }
    true
}

// ── RFB pixel format ──────────────────────────────────────────────────────────

/// Build the 16-byte ServerPixelFormat block for ARGB8888 little-endian
/// (same layout as `capture_screen` output and the compositor framebuffer).
fn pixel_format_block() -> [u8; 16] {
    [
        32,       // bits-per-pixel
        24,       // depth
        0,        // big-endian flag: 0 = little-endian
        1,        // true-colour flag: 1 = yes
        0, 0xFF,  // red-max   (255) BE16
        0, 0xFF,  // green-max (255) BE16
        0, 0xFF,  // blue-max  (255) BE16
        16,       // red-shift   (ARGB: A=24, R=16, G=8, B=0)
        8,        // green-shift
        0,        // blue-shift
        0, 0, 0,  // padding (3 bytes)
    ]
}

// ── FramebufferUpdate helpers ─────────────────────────────────────────────────

/// Send a FramebufferUpdate with a single Raw-encoded rectangle.
///
/// `pixels` must contain exactly `w * h` ARGB u32 values.
/// The function converts them to little-endian wire format in place.
fn send_framebuffer_update(sock: u32, x: u16, y: u16, w: u16, h: u16, pixels: &[u32]) -> bool {
    let n_pixels = w as usize * h as usize;
    if pixels.len() < n_pixels {
        return false;
    }

    // Header: type=0 (FramebufferUpdate), padding, number-of-rectangles=1
    let hdr: [u8; 4] = [0, 0, 0, 1];
    if !send_all(sock, &hdr) {
        return false;
    }

    // Rectangle header: x, y, w, h (BE16 each), encoding=0 (Raw) (BE32)
    let mut rect_hdr = [0u8; 12];
    rect_hdr[0..2].copy_from_slice(&be16(x));
    rect_hdr[2..4].copy_from_slice(&be16(y));
    rect_hdr[4..6].copy_from_slice(&be16(w));
    rect_hdr[6..8].copy_from_slice(&be16(h));
    // encoding = 0 (Raw)
    rect_hdr[8..12].copy_from_slice(&be32(0));
    if !send_all(sock, &rect_hdr) {
        return false;
    }

    // Pixel data: convert ARGB u32 → 4 bytes little-endian per pixel.
    // We reinterpret the ARGB pixels as raw bytes on a little-endian CPU.
    // capture_screen already delivers ARGB in little-endian byte order,
    // which matches our ServerPixelFormat (little-endian flag = 0 means
    // "server is little-endian, data is in native byte order").
    let byte_data = unsafe {
        core::slice::from_raw_parts(pixels.as_ptr() as *const u8, n_pixels * 4)
    };
    send_all(sock, byte_data)
}

// ── Login screen helpers ──────────────────────────────────────────────────────

/// Render the login overlay into a caller-supplied screen-size buffer.
///
/// The `screen_buf` must hold `sw * sh` ARGB pixels. The login panel is
/// centered; the rest of the buffer is filled with a dark background color.
fn render_login_overlay(
    screen_buf: &mut [u32],
    sw: usize,
    sh: usize,
    state: &LoginState<'_>,
) {
    // Fill entire buffer with dark background.
    for px in screen_buf.iter_mut() {
        *px = 0xFF1C1C1E;
    }

    // Render the 640×480 login panel into a temporary buffer.
    let mut panel = [0u32; LOGIN_W * LOGIN_H];
    login_ui::render(&mut panel, state);

    // Blit panel centered in screen_buf.
    let panel_x = if sw > LOGIN_W { (sw - LOGIN_W) / 2 } else { 0 };
    let panel_y = if sh > LOGIN_H { (sh - LOGIN_H) / 2 } else { 0 };
    let copy_w = LOGIN_W.min(sw);
    let copy_h = LOGIN_H.min(sh);

    for row in 0..copy_h {
        let dst_off = (panel_y + row) * sw + panel_x;
        let src_off = row * LOGIN_W;
        screen_buf[dst_off..dst_off + copy_w]
            .copy_from_slice(&panel[src_off..src_off + copy_w]);
    }
}

// ── Main session handler ──────────────────────────────────────────────────────

/// Run a complete VNC session on `sock`.
///
/// Returns when the connection closes normally or on error.
/// Intended to be called from a forked child process.
pub fn run_session(sock: u32, cfg: &VncConfig, comp_chan: u32) {
    // ── 1. Version handshake ──────────────────────────────────────────────────
    if !send_all(sock, b"RFB 003.008\n") {
        net::tcp_close(sock);
        return;
    }
    let mut client_ver = [0u8; 12];
    if !recv_exact(sock, &mut client_ver) {
        net::tcp_close(sock);
        return;
    }
    // Accept any 003.xxx client — we always speak 003.008.

    // ── 2. Security negotiation — offer VNC Auth (type 2) ────────────────────
    // Server sends: number-of-security-types, then types[].
    if !send_all(sock, &[1u8, 2u8]) {
        // 1 type, type-id = 2 (VNCAuth)
        net::tcp_close(sock);
        return;
    }
    let mut selected = [0u8; 1];
    if !recv_exact(sock, &mut selected) || selected[0] != 2 {
        // Client must select VNCAuth.
        net::tcp_close(sock);
        return;
    }

    // ── 3. VNC auth — DES challenge / response ────────────────────────────────
    // Generate a 16-byte challenge from the uptime (deterministic enough for a
    // challenge that is only used once per session; no need for /dev/random).
    let t0 = sys::uptime_ms();
    let t1 = sys::uptime();
    let mut challenge = [0u8; 16];
    // Mix time values into all 16 bytes.
    for i in 0..4 {
        challenge[i]     = (t0 >> (i * 8)) as u8;
        challenge[i + 4] = (t1 >> (i * 8)) as u8;
        challenge[i + 8] = ((t0 ^ t1) >> (i * 8)) as u8 ^ 0xA5;
        challenge[i + 12] = ((t0.wrapping_add(t1)) >> (i * 8)) as u8 ^ 0x5A;
    }

    if !send_all(sock, &challenge) {
        net::tcp_close(sock);
        return;
    }
    let mut response = [0u8; 16];
    if !recv_exact(sock, &mut response) {
        net::tcp_close(sock);
        return;
    }

    let auth_ok = des::vnc_verify_response(&cfg.password, &challenge, &response);
    // SecurityResult: 0 = OK, 1 = failed (BE32)
    let security_result = if auth_ok { be32(0) } else { be32(1) };
    let _ = send_all(sock, &security_result);
    if !auth_ok {
        // Optionally send a reason string.
        let reason = b"VNC password incorrect";
        let _ = send_all(sock, &be32(reason.len() as u32));
        let _ = send_all(sock, reason);
        net::tcp_close(sock);
        return;
    }

    // ── 4. ClientInit ─────────────────────────────────────────────────────────
    let mut client_init = [0u8; 1];
    if !recv_exact(sock, &mut client_init) {
        net::tcp_close(sock);
        return;
    }
    // shared flag (ignored — we support only one active client at a time per config)

    // ── 5. Capture screen to get dimensions ──────────────────────────────────
    let mut screen_info = [0u32; 2];
    let mut tmp_buf = [0u32; 4]; // tiny probe buf — real allocation below
    let _ = sys::capture_screen(&mut tmp_buf, &mut screen_info);
    let sw = (screen_info[0] as usize).clamp(1, MAX_SCREEN_DIM);
    let sh = (screen_info[1] as usize).clamp(1, MAX_SCREEN_DIM);

    // ── 6. ServerInit ─────────────────────────────────────────────────────────
    // framebuffer-width (BE16), framebuffer-height (BE16),
    // pixel-format (16 bytes), name-length (BE32), name-string.
    let mut server_init = [0u8; 4 + 16 + 4 + 8]; // 32 bytes
    server_init[0..2].copy_from_slice(&be16(sw as u16));
    server_init[2..4].copy_from_slice(&be16(sh as u16));
    server_init[4..20].copy_from_slice(&pixel_format_block());
    let name = b"anyOS VNC";
    server_init[20..24].copy_from_slice(&be32(name.len() as u32));
    server_init[24..24 + name.len()].copy_from_slice(name);
    if !send_all(sock, &server_init[..24 + name.len()]) {
        net::tcp_close(sock);
        return;
    }

    // ── 7. Allocate pixel buffers ─────────────────────────────────────────────
    let n_pixels = sw * sh;
    let mut screen_buf: anyos_std::Vec<u32> = anyos_std::vec![0u32; n_pixels];

    // ── 8. OS login screen phase ──────────────────────────────────────────────
    let mut username_buf = [0u8; 64];
    let mut username_len = 0usize;
    let mut password_buf = [0u8; 64];
    let mut password_len = 0usize;
    let mut cursor_in_username = true;
    let mut login_attempts = 0u32;
    let mut login_error: &[u8] = b"";
    let mut cursor_visible = true;
    let mut last_blink = sys::uptime_ms();
    let mut last_update = sys::uptime_ms().wrapping_sub(MIN_FRAME_INTERVAL_MS + 1);

    let mut in_login = true;
    let mut pending_update = true; // send initial frame immediately

    while in_login {
        // Blink cursor every 500ms.
        let now = sys::uptime_ms();
        if now.wrapping_sub(last_blink) >= 500 {
            cursor_visible = !cursor_visible;
            last_blink = now;
            pending_update = true;
        }

        // Send login screen frame if needed.
        if pending_update && now.wrapping_sub(last_update) >= MIN_FRAME_INTERVAL_MS {
            let uname = &username_buf[..username_len];
            let state = LoginState {
                username: uname,
                password_len,
                cursor_in_username,
                cursor_visible,
                error_msg: login_error,
            };
            render_login_overlay(&mut screen_buf, sw, sh, &state);
            let ok = send_framebuffer_update(sock, 0, 0, sw as u16, sh as u16, &screen_buf);
            if !ok {
                net::tcp_close(sock);
                return;
            }
            last_update = now;
            pending_update = false;
        }

        // Read a client message (non-blocking spin with yield).
        // We peek one byte first; if no data, yield and retry.
        let mut msg_type = [0u8; 1];
        let n = net::tcp_recv(sock, &mut msg_type);
        if n == 0 {
            // EOF — client disconnected.
            net::tcp_close(sock);
            return;
        }
        if n == u32::MAX {
            // No data yet — yield and retry.
            process::yield_cpu();
            continue;
        }

        match msg_type[0] {
            // FramebufferUpdateRequest (type 3): client wants a frame.
            3 => {
                let mut fbu_rest = [0u8; 9];
                if !recv_exact(sock, &mut fbu_rest) {
                    net::tcp_close(sock);
                    return;
                }
                pending_update = true;
            }
            // KeyEvent (type 4): collect username / password characters.
            4 => {
                let mut key_rest = [0u8; 7]; // down(1) + pad(2) + keysym(4)
                if !recv_exact(sock, &mut key_rest) {
                    net::tcp_close(sock);
                    return;
                }
                let down = key_rest[0] != 0;
                let keysym = from_be32(&key_rest[3..7]);

                if !down {
                    continue; // only process key-down events in login phase
                }

                match keysym {
                    // Tab — switch between username and password fields.
                    0xFF09 => {
                        cursor_in_username = !cursor_in_username;
                        pending_update = true;
                    }
                    // Enter — attempt login.
                    0xFF0D | 0xFF8D => {
                        let uname = &username_buf[..username_len];
                        let pw = &password_buf[..password_len];

                        // Convert byte slices to str (ASCII login only).
                        let uname_str = core::str::from_utf8(uname).unwrap_or("");
                        let pw_str = core::str::from_utf8(pw).unwrap_or("");

                        // Check root restriction.
                        let is_root = uname_str == "root";
                        if is_root && !cfg.allow_root {
                            login_error = b"Root access denied";
                            password_len = 0;
                            pending_update = true;
                            login_attempts += 1;
                        } else if !cfg.is_user_allowed(uname) {
                            login_error = b"User not in allowed list";
                            password_len = 0;
                            pending_update = true;
                            login_attempts += 1;
                        } else if !process::authenticate(uname_str, pw_str) {
                            login_error = b"Invalid credentials";
                            password_len = 0;
                            pending_update = true;
                            login_attempts += 1;
                        } else {
                            // Authenticated!
                            in_login = false;
                        }

                        if login_attempts >= MAX_LOGIN_ATTEMPTS {
                            // Too many failures — close connection.
                            let _ = send_all(sock, &[]);
                            net::tcp_close(sock);
                            return;
                        }
                    }
                    // Escape — abort / cancel.
                    0xFF1B => {
                        net::tcp_close(sock);
                        return;
                    }
                    // Backspace — delete last character in active field.
                    0xFF08 => {
                        if cursor_in_username && username_len > 0 {
                            username_len -= 1;
                        } else if !cursor_in_username && password_len > 0 {
                            password_len -= 1;
                        }
                        pending_update = true;
                    }
                    // Printable ASCII — append to active field.
                    0x0020..=0x007E => {
                        let ch = keysym as u8;
                        if cursor_in_username && username_len < username_buf.len() {
                            username_buf[username_len] = ch;
                            username_len += 1;
                            pending_update = true;
                        } else if !cursor_in_username && password_len < password_buf.len() {
                            password_buf[password_len] = ch;
                            password_len += 1;
                            pending_update = true;
                        }
                    }
                    _ => {}
                }
            }
            // PointerEvent (type 5): ignore during login.
            5 => {
                let mut _rest = [0u8; 5];
                let _ = recv_exact(sock, &mut _rest);
            }
            // ClientCutText (type 6): ignore.
            6 => {
                let mut cut_hdr = [0u8; 7]; // pad(3) + length(4)
                if !recv_exact(sock, &mut cut_hdr) {
                    net::tcp_close(sock);
                    return;
                }
                let text_len = from_be32(&cut_hdr[3..7]) as usize;
                let mut discard = [0u8; 64];
                let mut remaining = text_len;
                while remaining > 0 {
                    let chunk = remaining.min(64);
                    if !recv_exact(sock, &mut discard[..chunk]) {
                        net::tcp_close(sock);
                        return;
                    }
                    remaining -= chunk;
                }
            }
            // SetPixelFormat (type 0): ignore (we always use our format).
            0 => {
                let mut _rest = [0u8; 19];
                let _ = recv_exact(sock, &mut _rest);
            }
            // SetEncodings (type 2): read and discard.
            2 => {
                let mut enc_hdr = [0u8; 3]; // pad(1) + count(2)
                if !recv_exact(sock, &mut enc_hdr) {
                    net::tcp_close(sock);
                    return;
                }
                let count = from_be16(&enc_hdr[1..3]) as usize;
                let mut enc_buf = [0u8; 4];
                for _ in 0..count {
                    if !recv_exact(sock, &mut enc_buf) {
                        net::tcp_close(sock);
                        return;
                    }
                }
            }
            _ => {
                // Unknown message type — close for safety.
                net::tcp_close(sock);
                return;
            }
        }
    }

    // ── 9. Main loop — stream live desktop ───────────────────────────────────
    let mut mods = ModifierState::default();
    let mut last_frame_ms = sys::uptime_ms().wrapping_sub(MIN_FRAME_INTERVAL_MS + 1);
    let mut update_requested = false;

    loop {
        // Non-blocking message read.
        let mut msg_type = [0u8; 1];
        let n = net::tcp_recv(sock, &mut msg_type);
        if n == 0 {
            // EOF.
            break;
        }
        if n == u32::MAX {
            // No data — yield and check if we should send a frame.
            process::yield_cpu();
            // If an update was requested and enough time has passed, capture and send.
            if update_requested {
                let now = sys::uptime_ms();
                if now.wrapping_sub(last_frame_ms) >= MIN_FRAME_INTERVAL_MS {
                    if capture_and_send(sock, &mut screen_buf, sw, sh) {
                        last_frame_ms = now;
                        update_requested = false;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }

        match msg_type[0] {
            // SetPixelFormat: ignore.
            0 => {
                let mut _rest = [0u8; 19];
                if !recv_exact(sock, &mut _rest) { break; }
            }
            // SetEncodings: read and discard.
            2 => {
                let mut enc_hdr = [0u8; 3];
                if !recv_exact(sock, &mut enc_hdr) { break; }
                let count = from_be16(&enc_hdr[1..3]) as usize;
                let mut enc_buf = [0u8; 4];
                for _ in 0..count {
                    if !recv_exact(sock, &mut enc_buf) { break; }
                }
            }
            // FramebufferUpdateRequest.
            3 => {
                let mut fbu_rest = [0u8; 9]; // incremental(1) + x(2)+y(2)+w(2)+h(2)
                if !recv_exact(sock, &mut fbu_rest) { break; }
                // Mark that the client wants an update; we send it on the next tick.
                update_requested = true;
            }
            // KeyEvent.
            4 => {
                let mut key_rest = [0u8; 7];
                if !recv_exact(sock, &mut key_rest) { break; }
                let down = key_rest[0] != 0;
                let keysym = from_be32(&key_rest[3..7]);

                // Update modifier tracking; if it was a modifier key, skip injection.
                if !mods.update(keysym, down) {
                    input::inject_key(comp_chan, keysym, down, &mods);
                }
            }
            // PointerEvent.
            5 => {
                let mut ptr_rest = [0u8; 5]; // buttons(1) + x(2) + y(2)
                if !recv_exact(sock, &mut ptr_rest) { break; }
                let buttons = ptr_rest[0];
                let x = from_be16(&ptr_rest[1..3]);
                let y = from_be16(&ptr_rest[3..5]);
                input::inject_pointer(comp_chan, x, y, buttons);
            }
            // ClientCutText: ignore (we have our own clipboard IPC).
            6 => {
                let mut cut_hdr = [0u8; 7];
                if !recv_exact(sock, &mut cut_hdr) { break; }
                let text_len = from_be32(&cut_hdr[3..7]) as usize;
                let mut discard = [0u8; 64];
                let mut remaining = text_len;
                while remaining > 0 {
                    let chunk = remaining.min(64);
                    if !recv_exact(sock, &mut discard[..chunk]) { break; }
                    remaining -= chunk;
                }
            }
            _ => {
                // Unknown type — close.
                break;
            }
        }
    }

    net::tcp_close(sock);
}

/// Capture the current screen and send it as a full FramebufferUpdate.
fn capture_and_send(sock: u32, buf: &mut [u32], sw: usize, sh: usize) -> bool {
    let mut info = [0u32; 2];
    let ok = sys::capture_screen(buf, &mut info);
    if !ok || info[0] == 0 || info[1] == 0 {
        return false;
    }
    let actual_w = info[0] as usize;
    let actual_h = info[1] as usize;
    // If resolution changed, send only the intersection we have.
    let send_w = actual_w.min(sw) as u16;
    let send_h = actual_h.min(sh) as u16;
    send_framebuffer_update(sock, 0, 0, send_w, send_h, &buf[..send_w as usize * send_h as usize])
}
