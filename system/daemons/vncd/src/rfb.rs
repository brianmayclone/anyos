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
use anyos_std::ipc;
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

/// Send all bytes in `data`, sleeping briefly when the send buffer is full.
/// Returns `false` if the socket was closed (EOF).
fn send_all(sock: u32, data: &[u8]) -> bool {
    let mut sent = 0usize;
    let mut retries = 0u32;
    while sent < data.len() {
        let n = net::tcp_send(sock, &data[sent..]);
        if n == 0 {
            return false;
        }
        if n == u32::MAX {
            // Send buffer full — sleep 1 ms to let TCP stack process ACKs.
            retries += 1;
            if retries > 10000 {
                return false;
            }
            process::sleep(1);
            continue;
        }
        retries = 0;
        sent += n as usize;
    }
    true
}

/// Receive exactly `buf.len()` bytes, sleeping briefly when no data is available.
/// Returns `false` on EOF (connection closed) or timeout (30 seconds).
fn recv_exact(sock: u32, buf: &mut [u8]) -> bool {
    let mut received = 0usize;
    let mut timeout_count = 0u32;
    const RECV_TIMEOUT_MS: u32 = 30000; // 30 second timeout (matches VNC client TCP timeout)
    const RECV_SLEEP_MS: u32 = 1;
    const MAX_RETRIES: u32 = RECV_TIMEOUT_MS / RECV_SLEEP_MS;

    while received < buf.len() {
        let n = net::tcp_recv(sock, &mut buf[received..]);
        if n == 0 {
            // Connection closed by remote
            return false;
        }
        if n == u32::MAX {
            // No data yet — sleep 1 ms to let TCP stack receive segments.
            timeout_count += 1;
            if timeout_count > MAX_RETRIES {
                return false;
            }
            process::sleep(RECV_SLEEP_MS);
            continue;
        }
        timeout_count = 0; // reset on successful receive
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
/// RRE (encoding type 2) with zero subrects sends just the background color —
/// 20 bytes total vs 4 KB for a 32×32 Raw tile.
fn append_rre_solid_rect(out: &mut anyos_std::Vec<u8>, x: usize, y: usize, w: usize, h: usize, color: u32) {
    out.extend_from_slice(&be16(x as u16));
    out.extend_from_slice(&be16(y as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&be32(2)); // encoding = RRE
    out.extend_from_slice(&be32(0)); // zero subrectangles
    out.extend_from_slice(&color.to_le_bytes());
}

/// Append a raw rectangle (header + pixel data).
fn append_raw_rect(out: &mut anyos_std::Vec<u8>, fb: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize) {
    out.extend_from_slice(&be16(x as u16));
    out.extend_from_slice(&be16(y as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&[0, 0, 0, 0]); // encoding = Raw
    for row in y..y + h {
        let off = row * stride + x;
        let row_bytes = unsafe {
            core::slice::from_raw_parts(fb[off..].as_ptr() as *const u8, w * 4)
        };
        out.extend_from_slice(row_bytes);
    }
}

/// Append a Zlib-encoded rectangle (header + compressed pixel data).
///
/// Uses the persistent `ZlibEncoder` for cross-rectangle dictionary context.
/// Each rectangle's compressed data ends with SYNC_FLUSH so the client can
/// decompress immediately.
fn append_zlib_rect(
    out: &mut anyos_std::Vec<u8>,
    raw_buf: &mut anyos_std::Vec<u8>,
    encoder: &mut crate::compress::ZlibEncoder,
    fb: &[u32],
    stride: usize,
    x: usize, y: usize, w: usize, h: usize,
) {
    // Rectangle header: x, y, w, h, encoding=6 (Zlib)
    out.extend_from_slice(&be16(x as u16));
    out.extend_from_slice(&be16(y as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&be32(6)); // encoding = Zlib

    // Collect raw pixel data into contiguous buffer for deflate
    raw_buf.clear();
    for row in y..y + h {
        let off = row * stride + x;
        let row_bytes = unsafe {
            core::slice::from_raw_parts(fb[off..].as_ptr() as *const u8, w * 4)
        };
        raw_buf.extend_from_slice(row_bytes);
    }

    // Placeholder for compressed data length (4 bytes BE32)
    let length_pos = out.len();
    out.extend_from_slice(&[0u8; 4]);

    // Compress pixel data with persistent zlib stream
    let data_start = out.len();
    encoder.compress(raw_buf, out);

    // Patch the compressed data length
    let compressed_len = (out.len() - data_start) as u32;
    let len_bytes = be32(compressed_len);
    out[length_pos] = len_bytes[0];
    out[length_pos + 1] = len_bytes[1];
    out[length_pos + 2] = len_bytes[2];
    out[length_pos + 3] = len_bytes[3];
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

/// Append a CopyRect rectangle.
///
/// CopyRect (encoding type 1) tells the client to copy pixels from
/// `(src_x, src_y)` in its own framebuffer to `(dst_x, dst_y)`.
/// Only 4 bytes of payload (source position) — vs thousands for pixel data.
fn append_copyrect(out: &mut anyos_std::Vec<u8>, dx: usize, dy: usize, w: usize, h: usize, sx: usize, sy: usize) {
    out.extend_from_slice(&be16(dx as u16));
    out.extend_from_slice(&be16(dy as u16));
    out.extend_from_slice(&be16(w as u16));
    out.extend_from_slice(&be16(h as u16));
    out.extend_from_slice(&be32(1)); // encoding = CopyRect
    out.extend_from_slice(&be16(sx as u16));
    out.extend_from_slice(&be16(sy as u16));
}

// ── CopyRect motion detection ────────────────────────────────────────────────

/// Check if a rectangular region in `cur` at `(dx, dy)` matches `prev` at
/// `(sx, sy)` exactly (pixel-perfect match).
fn region_matches(
    cur: &[u32], prev: &[u32], stride: usize,
    dx: usize, dy: usize, sx: usize, sy: usize, w: usize, h: usize,
) -> bool {
    for row in 0..h {
        let cur_off = (dy + row) * stride + dx;
        let prev_off = (sy + row) * stride + sx;
        if cur[cur_off..cur_off + w] != prev[prev_off..prev_off + w] {
            return false;
        }
    }
    true
}

/// Detect if the dirty region is a uniform translation of content from `prev`.
///
/// Picks a sample tile from the dirty bounding box in `cur` and searches for
/// it in `prev` at nearby offsets.  If found, verifies the entire bounding box
/// matches at that offset.
///
/// Returns `Some((dx, dy))` = the motion vector (signed pixel offset), or
/// `None` if no uniform motion is detected.
///
/// Only searches offsets up to `MAX_SEARCH` pixels in each direction to
/// keep the cost bounded.
fn detect_motion(
    cur: &[u32], prev: &[u32], stride: usize,
    bbox_x: usize, bbox_y: usize, bbox_w: usize, bbox_h: usize,
    sw: usize, sh: usize,
) -> Option<(i32, i32)> {
    const MAX_SEARCH: i32 = 128; // max pixels to search in each direction
    const SAMPLE_SIZE: usize = 16; // sample block size for initial search

    // Sample a small block from the center of the dirty bounding box
    let sample_w = SAMPLE_SIZE.min(bbox_w);
    let sample_h = SAMPLE_SIZE.min(bbox_h);
    let sample_x = bbox_x + (bbox_w - sample_w) / 2;
    let sample_y = bbox_y + (bbox_h - sample_h) / 2;

    // Extract the sample's first row signature for fast rejection
    let sig_off = sample_y * stride + sample_x;
    if sig_off + sample_w > cur.len() { return None; }

    let mut best_dx: i32 = 0;
    let mut best_dy: i32 = 0;
    let mut found = false;

    // Search nearby offsets in prev for the sample block
    // Use a spiral-like pattern: check small offsets first (more likely for
    // window dragging which is typically a few pixels per frame)
    for dist in 1..=MAX_SEARCH {
        if found { break; }
        // Check all offsets at Manhattan distance `dist`
        for dy in -dist..=dist {
            if found { break; }
            for &dx in &[-dist, dist] {
                // Candidate source position in prev
                let sx = sample_x as i32 + dx;
                let sy = sample_y as i32 + dy;
                if sx < 0 || sy < 0 { continue; }
                let sx = sx as usize;
                let sy = sy as usize;
                if sx + sample_w > sw || sy + sample_h > sh { continue; }

                // Quick check: does the sample match at this offset?
                if region_matches(cur, prev, stride, sample_x, sample_y, sx, sy, sample_w, sample_h) {
                    best_dx = dx;
                    best_dy = dy;
                    found = true;
                    break;
                }
            }
            if found { break; }
            // Also check horizontal offsets within this distance
            if dy == -dist || dy == dist { continue; } // already checked corners
            for &dx in &[-dist, dist] {
                let sx = sample_x as i32 + dx;
                let sy = sample_y as i32 + dy;
                if sx < 0 || sy < 0 { continue; }
                let sx = sx as usize;
                let sy = sy as usize;
                if sx + sample_w > sw || sy + sample_h > sh { continue; }
                if region_matches(cur, prev, stride, sample_x, sample_y, sx, sy, sample_w, sample_h) {
                    best_dx = dx;
                    best_dy = dy;
                    found = true;
                    break;
                }
            }
        }
    }

    if !found { return None; }

    // Verify: does the ENTIRE bounding box match at this offset?
    // (The sample might match but the full region might not — e.g., two windows
    //  overlapping, or only part of the screen moved.)
    let src_x = bbox_x as i32 + best_dx;
    let src_y = bbox_y as i32 + best_dy;
    if src_x < 0 || src_y < 0 { return None; }
    let src_x = src_x as usize;
    let src_y = src_y as usize;
    if src_x + bbox_w > sw || src_y + bbox_h > sh { return None; }

    if region_matches(cur, prev, stride, bbox_x, bbox_y, src_x, src_y, bbox_w, bbox_h) {
        Some((best_dx, best_dy))
    } else {
        None
    }
}

/// Send a FramebufferUpdate using CopyRect + Zlib encoding.
///
/// First attempts CopyRect motion detection on the dirty bounding box.
/// If motion is detected, sends a CopyRect for the moved region, then
/// sends remaining dirty areas (edges/newly exposed) via Zlib.
/// Falls back to per-row-strip Zlib if no motion detected.
///
/// Returns:
///   -1  — connection error
///    0  — nothing dirty
///   >0  — number of rectangles sent
fn send_dirty_update_zlib(
    sock: u32,
    cur: &[u32],
    prev: &mut [u32],
    sw: usize,
    sh: usize,
    full: bool,
    send_buf: &mut anyos_std::Vec<u8>,
    raw_buf: &mut anyos_std::Vec<u8>,
    encoder: &mut crate::compress::ZlibEncoder,
    use_copyrect: bool,
) -> i32 {
    let tiles_x = (sw + TILE_SIZE - 1) / TILE_SIZE;
    let tiles_y = (sh + TILE_SIZE - 1) / TILE_SIZE;

    if full {
        send_buf.clear();
        send_buf.extend_from_slice(&[0, 0, 0, 1]); // 1 rectangle
        append_zlib_rect(send_buf, raw_buf, encoder, cur, sw, 0, 0, sw, sh);
        prev.copy_from_slice(&cur[..sw * sh]);
        return if send_all(sock, send_buf) { 1 } else { -1 };
    }

    // ── Phase 1: Build dirty tile map and compute bounding box ───────────

    // Flat array: dirty[ty_idx * tiles_x + tx_idx]
    let map_size = tiles_x * tiles_y;
    // Use a stack-allocated bitmap for up to 2048 tiles (covers 2048×2048 at 32px)
    // For larger screens, skip CopyRect and go straight to per-tile encoding.
    let mut dirty_map = [0u8; 4096]; // 1 byte per tile, plenty for most screens
    let mut n_dirty_tiles = 0u16;
    let mut bb_x0 = tiles_x;
    let mut bb_y0 = tiles_y;
    let mut bb_x1 = 0usize;
    let mut bb_y1 = 0usize;

    for ty_idx in 0..tiles_y {
        let ty = ty_idx * TILE_SIZE;
        let th = TILE_SIZE.min(sh - ty);
        for tx_idx in 0..tiles_x {
            let tx = tx_idx * TILE_SIZE;
            let tw = TILE_SIZE.min(sw - tx);
            if tile_dirty(cur, prev, sw, tx, ty, tw, th) {
                let map_idx = ty_idx * tiles_x + tx_idx;
                if map_idx < dirty_map.len() {
                    dirty_map[map_idx] = 1;
                }
                n_dirty_tiles += 1;
                if tx_idx < bb_x0 { bb_x0 = tx_idx; }
                if tx_idx + 1 > bb_x1 { bb_x1 = tx_idx + 1; }
                if ty_idx < bb_y0 { bb_y0 = ty_idx; }
                if ty_idx + 1 > bb_y1 { bb_y1 = ty_idx + 1; }
            }
        }
    }

    if n_dirty_tiles == 0 {
        return 0;
    }

    send_buf.clear();
    send_buf.extend_from_slice(&[0u8; 4]); // header placeholder
    let mut n_rects: u16 = 0;

    // ── Phase 2: Try CopyRect motion detection ───────────────────────────

    let bbox_x = bb_x0 * TILE_SIZE;
    let bbox_y = bb_y0 * TILE_SIZE;
    let bbox_w = (bb_x1 * TILE_SIZE).min(sw) - bbox_x;
    let bbox_h = (bb_y1 * TILE_SIZE).min(sh) - bbox_y;

    // Only attempt CopyRect if: client supports it, enough dirty tiles
    // to justify the search cost, and the dirty map fits in our buffer.
    let copyrect_applied = if use_copyrect && n_dirty_tiles >= 4 && map_size <= dirty_map.len() {
        if let Some((dx, dy)) = detect_motion(cur, prev, sw, bbox_x, bbox_y, bbox_w, bbox_h, sw, sh) {
            // CopyRect source position (where the content was in prev)
            let src_x = (bbox_x as i32 + dx) as usize;
            let src_y = (bbox_y as i32 + dy) as usize;

            // Emit CopyRect — MUST come first so client applies it before
            // subsequent rects overwrite the source area.
            append_copyrect(send_buf, bbox_x, bbox_y, bbox_w, bbox_h, src_x, src_y);
            n_rects += 1;

            // Update prev to reflect what the client now has after CopyRect.
            // The client copied from (src_x,src_y) to (bbox_x,bbox_y), so
            // prev at the destination now matches prev at the source.
            // We need to copy within prev to keep it in sync.
            //
            // Handle overlapping copies correctly with row-by-row direction:
            if dy > 0 || (dy == 0 && dx > 0) {
                // Copy bottom-to-top / right-to-left to avoid overlap corruption
                for row in (0..bbox_h).rev() {
                    let dst_off = (bbox_y + row) * sw + bbox_x;
                    let src_off = (src_y + row) * sw + src_x;
                    // Use a temporary row buffer for overlapping copy
                    for col in (0..bbox_w).rev() {
                        prev[dst_off + col] = prev[src_off + col];
                    }
                }
            } else {
                for row in 0..bbox_h {
                    let dst_off = (bbox_y + row) * sw + bbox_x;
                    let src_off = (src_y + row) * sw + src_x;
                    for col in 0..bbox_w {
                        prev[dst_off + col] = prev[src_off + col];
                    }
                }
            }

            // Mark tiles within the CopyRect area as clean (if they now match)
            for ty_idx in bb_y0..bb_y1 {
                let ty = ty_idx * TILE_SIZE;
                let th = TILE_SIZE.min(sh - ty);
                for tx_idx in bb_x0..bb_x1 {
                    let tx = tx_idx * TILE_SIZE;
                    let tw = TILE_SIZE.min(sw - tx);
                    let map_idx = ty_idx * tiles_x + tx_idx;
                    if map_idx < dirty_map.len() && dirty_map[map_idx] != 0 {
                        if !tile_dirty(cur, prev, sw, tx, ty, tw, th) {
                            dirty_map[map_idx] = 0;
                        }
                    }
                }
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    let _ = copyrect_applied; // suppress unused warning

    // ── Phase 3: Send remaining dirty tiles as Zlib/RRE row strips ───────

    for ty_idx in 0..tiles_y {
        let ty = ty_idx * TILE_SIZE;
        let th = TILE_SIZE.min(sh - ty);

        let mut tx_idx = 0;
        while tx_idx < tiles_x {
            let map_idx = ty_idx * tiles_x + tx_idx;
            let is_dirty = if map_idx < dirty_map.len() {
                dirty_map[map_idx] != 0
            } else {
                let tx = tx_idx * TILE_SIZE;
                let tw = TILE_SIZE.min(sw - tx);
                tile_dirty(cur, prev, sw, tx, ty, tw, th)
            };

            if !is_dirty {
                tx_idx += 1;
                continue;
            }

            // Extend run of dirty tiles
            let run_start = tx_idx;
            let mut run_end = tx_idx + 1;
            while run_end < tiles_x {
                let next_map = ty_idx * tiles_x + run_end;
                let next_dirty = if next_map < dirty_map.len() {
                    dirty_map[next_map] != 0
                } else {
                    let ntx = run_end * TILE_SIZE;
                    let ntw = TILE_SIZE.min(sw - ntx);
                    tile_dirty(cur, prev, sw, ntx, ty, ntw, th)
                };
                if !next_dirty { break; }
                run_end += 1;
            }

            let rx = run_start * TILE_SIZE;
            let rw = (run_end * TILE_SIZE).min(sw) - rx;

            if let Some(color) = tile_solid_color(cur, sw, rx, ty, rw, th) {
                append_rre_solid_rect(send_buf, rx, ty, rw, th, color);
            } else {
                append_zlib_rect(send_buf, raw_buf, encoder, cur, sw, rx, ty, rw, th);
            }
            n_rects += 1;

            for row in ty..ty + th {
                let off = row * sw + rx;
                prev[off..off + rw].copy_from_slice(&cur[off..off + rw]);
            }

            tx_idx = run_end;
        }
    }

    if n_rects == 0 {
        return 0;
    }

    let count_bytes = be16(n_rects);
    send_buf[2] = count_bytes[0];
    send_buf[3] = count_bytes[1];

    if send_all(sock, send_buf) { n_rects as i32 } else { -1 }
}

/// Send a FramebufferUpdate using Raw/RRE encoding (fallback, no compression).
fn send_dirty_update_raw(
    sock: u32,
    cur: &[u32],
    prev: &mut [u32],
    sw: usize,
    sh: usize,
    full: bool,
    send_buf: &mut anyos_std::Vec<u8>,
) -> i32 {
    if full {
        // Full update as raw
        let n_pixels = sw * sh;
        send_buf.clear();
        send_buf.extend_from_slice(&[0, 0, 0, 1]);
        let mut rect_hdr = [0u8; 12];
        rect_hdr[4..6].copy_from_slice(&be16(sw as u16));
        rect_hdr[6..8].copy_from_slice(&be16(sh as u16));
        send_buf.extend_from_slice(&rect_hdr);
        let byte_data = unsafe {
            core::slice::from_raw_parts(cur.as_ptr() as *const u8, n_pixels * 4)
        };
        send_buf.extend_from_slice(byte_data);
        prev.copy_from_slice(&cur[..n_pixels]);
        let result = send_all(sock, send_buf);
        return if result { 1 } else { -1 };
    }

    let tiles_x = (sw + TILE_SIZE - 1) / TILE_SIZE;
    let tiles_y = (sh + TILE_SIZE - 1) / TILE_SIZE;
    let mut n_dirty: u16 = 0;

    send_buf.clear();
    send_buf.extend_from_slice(&[0u8; 4]);

    for ty_idx in 0..tiles_y {
        for tx_idx in 0..tiles_x {
            let tx = tx_idx * TILE_SIZE;
            let ty = ty_idx * TILE_SIZE;
            let tw = TILE_SIZE.min(sw - tx);
            let th = TILE_SIZE.min(sh - ty);

            if tile_dirty(cur, prev, sw, tx, ty, tw, th) {
                if let Some(color) = tile_solid_color(cur, sw, tx, ty, tw, th) {
                    append_rre_solid_rect(send_buf, tx, ty, tw, th, color);
                } else {
                    append_raw_rect(send_buf, cur, sw, tx, ty, tw, th);
                }
                n_dirty += 1;

                for row in ty..ty + th {
                    let off = row * sw + tx;
                    prev[off..off + tw].copy_from_slice(&cur[off..off + tw]);
                }
            }
        }
    }

    if n_dirty == 0 { return 0; }

    let count_bytes = be16(n_dirty);
    send_buf[2] = count_bytes[0];
    send_buf[3] = count_bytes[1];

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
    let mut login_use_zlib = false; // set by SetEncodings during login
    let mut login_use_copyrect = false;

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
            let rc = send_dirty_update_raw(
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

        // Read a client message (non-blocking check first).
        let avail = net::tcp_recv_available(sock);
        if avail == u32::MAX - 1 || avail == u32::MAX {
            net::tcp_close(sock);
            return;
        }
        if avail == 0 {
            // No data yet — sleep briefly instead of busy-waiting.
            process::sleep(MIN_FRAME_MS);
            continue;
        }

        let mut msg_type = [0u8; 1];
        let n = net::tcp_recv(sock, &mut msg_type);
        if n == 0 {
            net::tcp_close(sock);
            return;
        }
        if n == u32::MAX {
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
            // SetEncodings (type 2): parse for Zlib support.
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
                    let enc_type = i32::from_be_bytes(enc_buf);
                    if enc_type == 1 { login_use_copyrect = true; }
                    if enc_type == 6 { login_use_zlib = true; }
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
    println!("vncd: authentication successful, entering desktop loop ({}x{})", sw, sh);

    // Free login-phase buffers — no longer needed.
    drop(login_panel);
    drop(login_prev);
    drop(login_send_buf);

    // Previous-frame buffer for dirty-tile detection.
    let mut prev_buf: anyos_std::Vec<u32> = anyos_std::vec![0u32; n_pixels];
    // Reusable send buffer.
    let mut send_buf: anyos_std::Vec<u8> = anyos_std::Vec::new();
    // Reusable raw pixel buffer for Zlib compression.
    let mut raw_buf: anyos_std::Vec<u8> = anyos_std::Vec::new();
    // Persistent zlib encoder (dictionary resets per call but adler-32 persists).
    let mut zlib_encoder = crate::compress::ZlibEncoder::new();
    // Whether client supports Zlib encoding (type 6) and CopyRect (type 1).
    // Initialized from login-phase SetEncodings; updated if client resends.
    // For now, use Raw encoding for desktop to debug (Zlib may have issues).
    let mut use_zlib = false;  // Disable Zlib temporarily for debugging
    let mut use_copyrect = login_use_copyrect;

    let mut mods = ModifierState::default();
    let mut last_frame_ms: u32 = 0;
    let mut update_requested = true; // send first desktop frame immediately
    let mut need_full = true; // first frame is always full

    // Adaptive frame interval: fast when active, slow when idle.
    let mut consecutive_clean: u32 = 0;

    // Clipboard state: use a dedicated compositor reply channel.
    const CMD_REGISTER_SUB: u32 = 0x100C;
    let reply_chan = ipc::evt_chan_create("vncd.reply");
    let sub_id = ipc::evt_chan_subscribe(reply_chan, 0);
    let tid = process::getpid();
    ipc::evt_chan_emit(comp_chan, &[CMD_REGISTER_SUB, tid, sub_id, reply_chan, 0]);
    let mut last_clipboard: anyos_std::Vec<u8> = anyos_std::Vec::new();
    let mut last_clipboard_check: u32 = 0;
    const CLIPBOARD_POLL_MS: u32 = 500;

    // Direct framebuffer access: after the first capture_screen call maps
    // the GPU framebuffer at 0x30000000, we read directly from that mapped
    // memory — no syscall, no 3 MB kernel→user copy per frame.
    let mut fb_mapped = false;
    let fb_pitch: usize = if screen_info[2] > 0 { screen_info[2] as usize } else { sw * 4 };
    let fb_contiguous = fb_pitch == sw * 4;

    loop {
        // ── Phase A: drain ALL queued client messages (non-blocking) ────────
        loop {
            // Check if data is available before calling the blocking tcp_recv.
            // tcp_recv blocks for up to 3 s in the kernel; polling first
            // avoids burning CPU in the kernel's poll_yield loop.
            let avail = net::tcp_recv_available(sock);
            if avail == 0 {
                break; // no data pending — move on to frame update
            }
            if avail == u32::MAX - 1 {
                // EOF — remote closed
                net::tcp_close(sock);
                return;
            }
            if avail == u32::MAX {
                // error / invalid socket
                net::tcp_close(sock);
                return;
            }

            let mut msg_type = [0u8; 1];
            let n = net::tcp_recv(sock, &mut msg_type);
            if n == 0 {
                net::tcp_close(sock);
                return;
            }
            if n == u32::MAX {
                break;
            }

            match msg_type[0] {
                // SetPixelFormat (type 0): ignore.
                0 => {
                    let mut _rest = [0u8; 19];
                    if !recv_exact(sock, &mut _rest) { net::tcp_close(sock); return; }
                }
                // SetEncodings (type 2): parse to detect compression support.
                2 => {
                    let mut enc_hdr = [0u8; 3];
                    if !recv_exact(sock, &mut enc_hdr) { net::tcp_close(sock); return; }
                    let count = from_be16(&enc_hdr[1..3]) as usize;
                    use_zlib = false;
                    use_copyrect = false;
                    let mut enc_buf = [0u8; 4];
                    for _ in 0..count {
                        if !recv_exact(sock, &mut enc_buf) { net::tcp_close(sock); return; }
                        let enc_type = i32::from_be_bytes(enc_buf);
                        if enc_type == 1 { use_copyrect = true; }
                        if enc_type == 6 { use_zlib = true; }
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
                    mods.update(keysym, down);
                    // Always inject — including modifier keys so the
                    // compositor/apps can track Ctrl/Shift/Alt state
                    // independently via keycode observations.
                    input::inject_key(comp_chan, keysym, down, &mods);
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
                    consecutive_clean = 0;
                }
                // ClientCutText (type 6): forward to compositor clipboard.
                6 => {
                    let mut cut_hdr = [0u8; 7];
                    if !recv_exact(sock, &mut cut_hdr) { net::tcp_close(sock); return; }
                    let text_len = from_be32(&cut_hdr[3..7]) as usize;
                    if text_len == 0 {
                        // Empty clipboard — nothing to do.
                    } else if text_len > 65536 {
                        // Too large — discard to prevent memory exhaustion.
                        let mut discard = [0u8; 64];
                        let mut remaining = text_len;
                        while remaining > 0 {
                            let chunk = remaining.min(64);
                            if !recv_exact(sock, &mut discard[..chunk]) { net::tcp_close(sock); return; }
                            remaining -= chunk;
                        }
                    } else {
                        let mut text_buf: anyos_std::Vec<u8> = anyos_std::Vec::with_capacity(text_len);
                        text_buf.resize(text_len, 0u8);
                        if !recv_exact(sock, &mut text_buf) { net::tcp_close(sock); return; }
                        crate::clipboard::set_compositor_clipboard(comp_chan, &text_buf);
                    }
                }
                _ => {
                    net::tcp_close(sock);
                    return;
                }
            }
        }

        // ── Phase B: send frame update if requested ──────────────────────────
        if update_requested {
            let now = sys::uptime_ms();
            let interval = if consecutive_clean > IDLE_THRESHOLD { IDLE_FRAME_MS } else { MIN_FRAME_MS };
            if now.wrapping_sub(last_frame_ms) >= interval {
                // Get current framebuffer
                // Always use screen_buf for safety; don't directly reference 0x30000000.
                let cur = {
                    if !fb_mapped {
                        let mut info = [0u32; 3];
                        let ok = sys::capture_screen(&mut screen_buf, &mut info);
                        if !ok {
                            break;
                        }
                        // If kernel returns 0x0 (e.g., due to buffer size mismatch),
                        // just skip this frame and try again next iteration.
                        if info[0] == 0 || info[1] == 0 {
                            update_requested = false;
                            process::sleep(MIN_FRAME_MS);
                            continue;
                        }
                        fb_mapped = true;
                    } else {
                        // Refresh framebuffer from GPU memory into screen_buf
                        if fb_contiguous {
                            unsafe {
                                let src = 0x3000_0000 as *const u32;
                                core::ptr::copy_nonoverlapping(src, screen_buf.as_mut_ptr(), sw * sh);
                            }
                        } else {
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
                        }
                    }
                    &screen_buf as &[u32]
                };

                // Send update — Zlib-compressed if client supports it, Raw otherwise
                let rc = if use_zlib {
                    send_dirty_update_zlib(
                        sock, cur, &mut prev_buf, sw, sh, need_full,
                        &mut send_buf, &mut raw_buf, &mut zlib_encoder,
                        use_copyrect,
                    )
                } else {
                    send_dirty_update_raw(
                        sock, cur, &mut prev_buf, sw, sh, need_full,
                        &mut send_buf,
                    )
                };

                if rc < 0 {
                    break;
                }
                last_frame_ms = now;
                if rc > 0 {
                    update_requested = false;
                    need_full = false;
                    consecutive_clean = 0;
                } else {
                    consecutive_clean = consecutive_clean.saturating_add(1);
                }
            }
        }

        // ── Phase C: check for clipboard changes → send ServerCutText ──────
        let clip_now = sys::uptime_ms();
        if clip_now.wrapping_sub(last_clipboard_check) >= CLIPBOARD_POLL_MS {
            last_clipboard_check = clip_now;
            let mut clip_buf = [0u8; 4096];
            let clip_len = crate::clipboard::get_compositor_clipboard(comp_chan, reply_chan, sub_id, &mut clip_buf);
            if clip_len > 0 {
                let clip_data = &clip_buf[..clip_len];
                if clip_data != last_clipboard.as_slice() {
                    last_clipboard.clear();
                    last_clipboard.extend_from_slice(clip_data);
                    // ServerCutText: type(1) + padding(3) + length(4) + text
                    let mut msg: anyos_std::Vec<u8> = anyos_std::Vec::with_capacity(8 + clip_len);
                    msg.push(3u8); // type = ServerCutText
                    msg.push(0); msg.push(0); msg.push(0); // padding
                    let len_bytes = (clip_len as u32).to_be_bytes();
                    msg.extend_from_slice(&len_bytes);
                    msg.extend_from_slice(clip_data);
                    if !send_all(sock, &msg) {
                        break;
                    }
                }
            }
        }

        // ── Phase D: sleep based on activity level ───────────────────────────
        // Active: short sleep to keep latency low while not busy-spinning.
        // Idle:   sleep the full idle interval — nothing is changing on screen.
        if consecutive_clean > IDLE_THRESHOLD {
            process::sleep(IDLE_FRAME_MS);
        } else {
            process::sleep(MIN_FRAME_MS);
        }
    }

    ipc::evt_chan_unsubscribe(comp_chan, sub_id);
    net::tcp_close(sock);
}
