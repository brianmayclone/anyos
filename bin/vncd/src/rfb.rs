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
use anyos_std::println;

use crate::config::VncConfig;
use crate::des;
use crate::input::{self, ModifierState};
use crate::login_ui::{self, LoginState, LOGIN_H, LOGIN_W};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// Maximum failed OS login attempts before the connection is dropped.
const MAX_LOGIN_ATTEMPTS: u32 = 3;

/// Minimum interval between framebuffer updates when actively changing (ms).
const MIN_FRAME_MS: u32 = 16; // ~60 FPS target

/// Frame check interval when nothing has changed for several frames (ms).
const IDLE_FRAME_MS: u32 = 100; // 10 FPS when idle

/// Number of consecutive clean (no-dirty-tile) frames before switching to idle.
const IDLE_THRESHOLD: u32 = 5;

/// Maximum screen dimension we support (guards heap allocation).
const MAX_SCREEN_DIM: usize = 2048;

/// Tile size for dirty-rectangle detection (pixels).
/// Smaller tiles = more granular updates (4 KB/tile vs 16 KB at 64px).
const TILE_SIZE: usize = 32;

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

/// Send all bytes in `data`, yielding when the send buffer is full.
/// Returns `false` if the socket was closed (EOF).
fn send_all(sock: u32, data: &[u8]) -> bool {
    let mut sent = 0usize;
    while sent < data.len() {
        let n = net::tcp_send(sock, &data[sent..]);
        if n == 0 {
            return false;
        }
        if n == u32::MAX {
            // Send buffer full — yield CPU (don't sleep!) to let TCP
            // stack process ACKs and free buffer space.
            process::yield_cpu();
            continue;
        }
        sent += n as usize;
    }
    true
}

/// Receive exactly `buf.len()` bytes, yielding when no data is available.
/// Returns `false` on EOF (connection closed).
fn recv_exact(sock: u32, buf: &mut [u8]) -> bool {
    let mut received = 0usize;
    while received < buf.len() {
        let n = net::tcp_recv(sock, &mut buf[received..]);
        if n == 0 {
            return false;
        }
        if n == u32::MAX {
            // No data yet — yield CPU to let TCP stack receive segments.
            process::yield_cpu();
            continue;
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

/// Send a FramebufferUpdate with a single Raw-encoded rectangle covering the full screen.
/// Assembles header + rect header + pixel data into `send_buf` for a single TCP write.
fn send_full_update(sock: u32, w: u16, h: u16, pixels: &[u32], send_buf: &mut anyos_std::Vec<u8>) -> bool {
    let n_pixels = w as usize * h as usize;
    if pixels.len() < n_pixels {
        return false;
    }

    send_buf.clear();

    // FramebufferUpdate header: type=0, padding=0, number-of-rectangles=1
    send_buf.extend_from_slice(&[0, 0, 0, 1]);

    // Rectangle header: x=0, y=0, w, h, encoding=0 (Raw)
    let mut rect_hdr = [0u8; 12];
    rect_hdr[4..6].copy_from_slice(&be16(w));
    rect_hdr[6..8].copy_from_slice(&be16(h));
    send_buf.extend_from_slice(&rect_hdr);

    // Pixel data.
    let byte_data = unsafe {
        core::slice::from_raw_parts(pixels.as_ptr() as *const u8, n_pixels * 4)
    };
    send_buf.extend_from_slice(byte_data);

    send_all(sock, send_buf)
}

/// Check if every pixel in a tile is the same solid color.
/// Returns Some(color) if solid, None if mixed.
fn tile_solid_color(fb: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize) -> Option<u32> {
    let first = fb[y * stride + x];
    for row in y..y + h {
        let off = row * stride + x;
        for col in 0..w {
            if fb[off + col] != first {
                return None;
            }
        }
    }
    Some(first)
}

/// Append an RRE-encoded rectangle for a solid-color tile.
///
/// RRE (Rise-and-Run-length Encoding, encoding type 2) with zero subrects
/// sends just the background color — 20 bytes total vs 16 KB for a 64x64 Raw tile.
/// All VNC clients support RRE.
fn append_rre_solid_rect(out: &mut anyos_std::Vec<u8>, x: usize, y: usize, w: usize, h: usize, color: u32) {
    // Rectangle header: x, y, w, h (BE16 each), encoding=2 (RRE, BE32)
    out.extend_from_slice(&be16(x as u16));
    out.extend_from_slice(&be16(y as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&be32(2)); // encoding = RRE

    // RRE payload: num-subrects (BE32) = 0, background-pixel (4 bytes LE)
    out.extend_from_slice(&be32(0)); // zero subrectangles
    out.extend_from_slice(&color.to_le_bytes()); // background pixel (same byte order as Raw)
}

/// Append a raw rectangle (header + pixel data) to a byte buffer.
fn append_raw_rect(out: &mut anyos_std::Vec<u8>, fb: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize) {
    // Rectangle header: x, y, w, h (BE16 each), encoding=0 (Raw, BE32)
    out.extend_from_slice(&be16(x as u16));
    out.extend_from_slice(&be16(y as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&[0, 0, 0, 0]); // encoding = Raw

    // Pixel data row by row.
    for row in y..y + h {
        let off = row * stride + x;
        let row_bytes = unsafe {
            core::slice::from_raw_parts(fb[off..].as_ptr() as *const u8, w * 4)
        };
        out.extend_from_slice(row_bytes);
    }
}

/// Append the best encoding for a dirty tile: RRE for solid-color, Raw otherwise.
fn append_tile_rect(out: &mut anyos_std::Vec<u8>, fb: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize) {
    if let Some(color) = tile_solid_color(fb, stride, x, y, w, h) {
        // Solid color: 20 bytes (RRE) instead of ~16 KB (Raw). ~800x smaller.
        append_rre_solid_rect(out, x, y, w, h, color);
    } else {
        append_raw_rect(out, fb, stride, x, y, w, h);
    }
}

/// Check if a tile differs between `cur` and `prev` framebuffers.
fn tile_dirty(cur: &[u32], prev: &[u32], stride: usize, tx: usize, ty: usize, tw: usize, th: usize) -> bool {
    for row in ty..ty + th {
        let off = row * stride + tx;
        if cur[off..off + tw] != prev[off..off + tw] {
            return true;
        }
    }
    false
}

/// Send a FramebufferUpdate containing only the dirty tiles.
/// All tile data is collected into one buffer and sent in a single TCP write.
///
/// Returns:
///   -1  — connection error (caller should break)
///    0  — nothing dirty, nothing was sent
///   >0  — number of dirty tiles sent
///
/// Updates `prev` with `cur` for dirty regions.
fn send_dirty_update(
    sock: u32,
    cur: &[u32],
    prev: &mut [u32],
    sw: usize,
    sh: usize,
    full: bool,
    send_buf: &mut anyos_std::Vec<u8>,
) -> i32 {
    if full {
        // Full (non-incremental) update: send everything in one call.
        prev.copy_from_slice(&cur[..sw * sh]);
        return if send_full_update(sock, sw as u16, sh as u16, cur, send_buf) { 1 } else { -1 };
    }

    send_buf.clear();

    // Scan for dirty tiles and build the complete message in send_buf.
    let tiles_x = (sw + TILE_SIZE - 1) / TILE_SIZE;
    let tiles_y = (sh + TILE_SIZE - 1) / TILE_SIZE;
    let mut n_dirty: u16 = 0;

    // Reserve space for the FramebufferUpdate header (4 bytes).
    // We'll fill in the rectangle count after scanning.
    send_buf.extend_from_slice(&[0u8; 4]);

    for ty_idx in 0..tiles_y {
        for tx_idx in 0..tiles_x {
            let tx = tx_idx * TILE_SIZE;
            let ty = ty_idx * TILE_SIZE;
            let tw = TILE_SIZE.min(sw - tx);
            let th = TILE_SIZE.min(sh - ty);

            if tile_dirty(cur, prev, sw, tx, ty, tw, th) {
                // Append this tile's rect — uses RRE for solid tiles, Raw otherwise.
                append_tile_rect(send_buf, cur, sw, tx, ty, tw, th);
                n_dirty += 1;

                // Update prev buffer for this tile.
                for row in ty..ty + th {
                    let off = row * sw + tx;
                    prev[off..off + tw].copy_from_slice(&cur[off..off + tw]);
                }
            }
        }
    }

    if n_dirty == 0 {
        // Nothing changed — don't send anything.
        // The caller keeps update_requested=true and checks again later.
        return 0;
    }

    // Patch the rectangle count into the header.
    let count_bytes = be16(n_dirty);
    send_buf[2] = count_bytes[0];
    send_buf[3] = count_bytes[1];

    // Single TCP send for the entire update.
    if send_all(sock, send_buf) { n_dirty as i32 } else { -1 }
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
    panel: &mut [u32],       // pre-allocated LOGIN_W * LOGIN_H buffer (heap)
) {
    // Fill entire buffer with dark background.
    for px in screen_buf.iter_mut() {
        *px = 0xFF1C1C1E;
    }

    // Render the 640×480 login panel into the pre-allocated buffer.
    login_ui::render(panel, state);

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
        net::tcp_close(sock);
        return;
    }
    let mut selected = [0u8; 1];
    if !recv_exact(sock, &mut selected) {
        net::tcp_close(sock);
        return;
    }
    if selected[0] != 2 {
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

    // Compute expected locally for DES verification.
    let mut expected = challenge;
    des::vnc_encrypt_challenge(&cfg.password, &mut expected);

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
    // Probe with a tiny buffer — kernel now writes dimensions even on "buf too small".
    let mut screen_info = [0u32; 3]; // [width, height, pitch_bytes]
    let mut tmp_buf = [0u32; 4];
    let _ = sys::capture_screen(&mut tmp_buf, &mut screen_info);
    let sw = if screen_info[0] > 0 { (screen_info[0] as usize).min(MAX_SCREEN_DIM) } else { 1024 };
    let sh = if screen_info[1] > 0 { (screen_info[1] as usize).min(MAX_SCREEN_DIM) } else { 768 };

    // ── 6. ServerInit ─────────────────────────────────────────────────────────
    // framebuffer-width (BE16), framebuffer-height (BE16),
    // pixel-format (16 bytes), name-length (BE32), name-string.
    let mut server_init = [0u8; 4 + 16 + 4 + 12]; // 36 bytes (room for name up to 12 chars)
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

    // Pre-allocate the 640×480 login panel on the HEAP (not stack!).
    // Stack-allocating 1.2 MB would overflow the process stack.
    let mut login_panel: anyos_std::Vec<u32> = anyos_std::vec![0u32; LOGIN_W * LOGIN_H];

    // Previous-frame buffer and send buffer for tile-based dirty detection.
    let mut login_prev: anyos_std::Vec<u32> = anyos_std::vec![0u32; sw * sh];
    let mut login_send_buf: anyos_std::Vec<u8> = anyos_std::Vec::new();
    let mut login_first_frame = true;

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
    let mut last_update = sys::uptime_ms().wrapping_sub(MIN_FRAME_MS + 1);

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

        // Send login screen frame if needed (tile-based dirty detection).
        if pending_update && now.wrapping_sub(last_update) >= MIN_FRAME_MS {
            let uname = &username_buf[..username_len];
            let state = LoginState {
                username: uname,
                password_len,
                cursor_in_username,
                cursor_visible,
                error_msg: login_error,
            };
            render_login_overlay(&mut screen_buf, sw, sh, &state, &mut login_panel);
            let rc = send_dirty_update(
                sock, &screen_buf, &mut login_prev, sw, sh,
                login_first_frame, &mut login_send_buf,
            );
            login_first_frame = false;
            if rc < 0 {
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
                let mut _spf = [0u8; 19]; // 3 padding + 16 pixel-format
                if !recv_exact(sock, &mut _spf) {
                    net::tcp_close(sock);
                    return;
                }
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
    println!("vncd: desktop loop ({}x{})", sw, sh);

    // Free login-phase buffers — no longer needed.
    drop(login_panel);
    drop(login_prev);
    drop(login_send_buf);

    // Previous-frame buffer for dirty-tile detection.
    let mut prev_buf: anyos_std::Vec<u32> = anyos_std::vec![0u32; n_pixels];
    // Reusable send buffer — pre-allocate with generous capacity to avoid
    // per-frame allocations.
    let mut send_buf: anyos_std::Vec<u8> = anyos_std::Vec::new();

    let mut mods = ModifierState::default();
    let mut last_frame_ms: u32 = 0;
    let mut update_requested = true; // send first desktop frame immediately
    let mut need_full = true; // first frame is always full

    // Adaptive frame interval: fast when active, slow when idle.
    let mut consecutive_clean: u32 = 0;

    // Direct framebuffer access: after the first capture_screen call maps
    // the GPU framebuffer at 0x30000000, we read directly from that mapped
    // memory — no syscall, no 3 MB kernel→user copy per frame.
    let mut fb_mapped = false;
    let fb_pitch: usize = if screen_info[2] > 0 { screen_info[2] as usize } else { sw * 4 };
    let fb_contiguous = fb_pitch == sw * 4;

    loop {
        // ── Phase A: drain ALL queued client messages ─────────────────────────
        // Processing all pending messages before sending a frame ensures that
        // rapid mouse/keyboard input doesn't starve frame updates.
        loop {
            let mut msg_type = [0u8; 1];
            let n = net::tcp_recv(sock, &mut msg_type);
            if n == 0 {
                // EOF — client disconnected.
                net::tcp_close(sock);
                return;
            }
            if n == u32::MAX {
                break; // no more queued data
            }

            match msg_type[0] {
                // SetPixelFormat (type 0): ignore.
                0 => {
                    let mut _rest = [0u8; 19];
                    if !recv_exact(sock, &mut _rest) { net::tcp_close(sock); return; }
                }
                // SetEncodings (type 2): read and discard.
                2 => {
                    let mut enc_hdr = [0u8; 3];
                    if !recv_exact(sock, &mut enc_hdr) { net::tcp_close(sock); return; }
                    let count = from_be16(&enc_hdr[1..3]) as usize;
                    let mut enc_buf = [0u8; 4];
                    for _ in 0..count {
                        if !recv_exact(sock, &mut enc_buf) { net::tcp_close(sock); return; }
                    }
                }
                // FramebufferUpdateRequest (type 3).
                3 => {
                    let mut fbu_rest = [0u8; 9];
                    if !recv_exact(sock, &mut fbu_rest) { net::tcp_close(sock); return; }
                    let incremental = fbu_rest[0] != 0;
                    if !incremental {
                        need_full = true;
                    }
                    update_requested = true;
                }
                // KeyEvent (type 4).
                4 => {
                    let mut key_rest = [0u8; 7];
                    if !recv_exact(sock, &mut key_rest) { net::tcp_close(sock); return; }
                    let down = key_rest[0] != 0;
                    let keysym = from_be32(&key_rest[3..7]);
                    if !mods.update(keysym, down) {
                        input::inject_key(comp_chan, keysym, down, &mods);
                    }
                    // Reset idle counter — expect screen changes after input.
                    consecutive_clean = 0;
                }
                // PointerEvent (type 5).
                5 => {
                    let mut ptr_rest = [0u8; 5];
                    if !recv_exact(sock, &mut ptr_rest) { net::tcp_close(sock); return; }
                    let buttons = ptr_rest[0];
                    let x = from_be16(&ptr_rest[1..3]);
                    let y = from_be16(&ptr_rest[3..5]);
                    input::inject_pointer(comp_chan, x, y, buttons);
                    // Reset idle counter — expect screen changes after input.
                    consecutive_clean = 0;
                }
                // ClientCutText (type 6): ignore.
                6 => {
                    let mut cut_hdr = [0u8; 7];
                    if !recv_exact(sock, &mut cut_hdr) { net::tcp_close(sock); return; }
                    let text_len = from_be32(&cut_hdr[3..7]) as usize;
                    let mut discard = [0u8; 64];
                    let mut remaining = text_len;
                    while remaining > 0 {
                        let chunk = remaining.min(64);
                        if !recv_exact(sock, &mut discard[..chunk]) { net::tcp_close(sock); return; }
                        remaining -= chunk;
                    }
                }
                _ => { net::tcp_close(sock); return; }
            }
        }

        // ── Phase B: send frame update if requested ──────────────────────────
        if update_requested {
            let now = sys::uptime_ms();
            let interval = if consecutive_clean > IDLE_THRESHOLD { IDLE_FRAME_MS } else { MIN_FRAME_MS };
            if now.wrapping_sub(last_frame_ms) >= interval {
                let rc = if !fb_mapped {
                    // First frame: call capture_screen to establish FB mapping
                    // at 0x30000000 and get the initial contents.
                    let mut info = [0u32; 3];
                    let ok = sys::capture_screen(&mut screen_buf, &mut info);
                    if !ok || info[0] == 0 || info[1] == 0 {
                        break;
                    }
                    fb_mapped = true;
                    send_dirty_update(sock, &screen_buf, &mut prev_buf, sw, sh, need_full, &mut send_buf)
                } else {
                    // Subsequent frames: read directly from the mapped GPU
                    // framebuffer — zero-copy when pitch == width*4.
                    let cur = if fb_contiguous {
                        unsafe {
                            core::slice::from_raw_parts(0x3000_0000 as *const u32, sw * sh)
                        }
                    } else {
                        // pitch != width*4: row-by-row copy (still no syscall).
                        unsafe {
                            let src = 0x3000_0000 as *const u8;
                            for y in 0..sh {
                                let src_row = src.add(y * fb_pitch);
                                let dst_off = y * sw;
                                core::ptr::copy_nonoverlapping(
                                    src_row as *const u32,
                                    screen_buf[dst_off..].as_mut_ptr(),
                                    sw,
                                );
                            }
                        }
                        &screen_buf
                    };
                    send_dirty_update(sock, cur, &mut prev_buf, sw, sh, need_full, &mut send_buf)
                };

                if rc < 0 {
                    break; // connection error
                }
                last_frame_ms = now;
                if rc > 0 {
                    // Dirty tiles were sent — cycle complete.
                    update_requested = false;
                    need_full = false;
                    consecutive_clean = 0;
                } else {
                    // Nothing dirty — don't respond yet, keep update_requested
                    // true and re-check on the next iteration.  This avoids the
                    // tight request→empty-response→request polling loop.
                    consecutive_clean = consecutive_clean.saturating_add(1);
                }
            }
        }

        // ── Phase C: yield or sleep based on activity ────────────────────────
        if consecutive_clean > IDLE_THRESHOLD {
            // Idle: sleep to save CPU.  The scheduler will wake us after ~1 ms.
            process::sleep(1);
        } else {
            // Active: yield immediately to minimise latency.
            process::yield_cpu();
        }
    }

    net::tcp_close(sock);
}
