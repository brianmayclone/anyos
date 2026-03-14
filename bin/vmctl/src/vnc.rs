//! Built-in VNC/RFB server for vmctl.
//!
//! `cmd_vnc()` starts a VM (same setup as `cmd_run`) and simultaneously
//! opens a TCP listener on the given port (default 5900).  Each connecting
//! VNC client gets an RFB 3.8 session that:
//!
//! * Streams the guest VGA framebuffer as Raw-encoded FramebufferUpdates.
//! * Forwards KeyEvent messages as PS/2 scancodes via `ps2_key_press/release`.
//! * Forwards PointerEvent messages as PS/2 mouse events.
//!
//! Authentication: optional VNC password (type-2 DES challenge/response).
//! If no password is configured (`--vnc-pass` omitted) the server offers
//! security type 1 (None) so any client can connect without credentials.
//!
//! # Usage
//!
//! ```text
//! vmctl vnc [-r <mb>] [-d <disk>] [-i <iso>] [-b <bios>]
//!           [-p <port>]          VNC port (default 5900)
//!           [--vnc-pass <pw>]    Optional VNC password (max 8 chars)
//!           [-n]                 Enable E1000 NIC
//! ```

use alloc::vec::Vec;
use anyos_std::{net, println, process, sys};
use libcorevm_client::VmHandle;

use crate::{VmConfig, read_file, BIOS_PATH_COREVM, BIOS_PATH_SEABIOS};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// RFB tile size for dirty detection (pixels). 32×32 = 4 KB per tile.
const TILE: usize = 32;
/// Interval between framebuffer polls when the guest is changing (ms).
const ACTIVE_MS: u32 = 16; // ~60 FPS
/// Interval when nothing changed for a few frames.
const IDLE_MS: u32 = 100; // 10 FPS
/// Consecutive clean frames before entering idle.
const IDLE_THRESH: u32 = 5;
/// Maximum VGA framebuffer dimension we support.
const MAX_DIM: usize = 2048;

// ── Big-endian wire helpers ───────────────────────────────────────────────────

#[inline] fn be16(v: u16) -> [u8; 2] { v.to_be_bytes() }
#[inline] fn be32(v: u32) -> [u8; 4] { v.to_be_bytes() }
#[inline] fn from_be16(b: &[u8]) -> u16 { u16::from_be_bytes([b[0], b[1]]) }
#[inline] fn from_be32(b: &[u8]) -> u32 { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) }

// ── TCP helpers ───────────────────────────────────────────────────────────────

fn send_all(sock: u32, data: &[u8]) -> bool {
    let mut sent = 0;
    let mut retries = 0u32;
    while sent < data.len() {
        let n = net::tcp_send(sock, &data[sent..]);
        if n == 0 { return false; }
        if n == u32::MAX {
            retries += 1;
            if retries > 10_000 { return false; }
            process::sleep(1);
            continue;
        }
        retries = 0;
        sent += n as usize;
    }
    true
}

fn recv_exact(sock: u32, buf: &mut [u8]) -> bool {
    let mut got = 0;
    let mut tries = 0u32;
    while got < buf.len() {
        let n = net::tcp_recv(sock, &mut buf[got..]);
        if n == 0 { return false; }
        if n == u32::MAX {
            tries += 1;
            if tries > 30_000 { return false; } // 30 s
            process::sleep(1);
            continue;
        }
        tries = 0;
        got += n as usize;
    }
    true
}

/// Non-blocking read of up to `buf.len()` bytes.  Returns 0 if no data.
fn recv_nowait(sock: u32, buf: &mut [u8]) -> usize {
    let avail = net::tcp_recv_available(sock);
    if avail == 0 || avail == u32::MAX { return 0; }
    let n = net::tcp_recv(sock, buf);
    if n == u32::MAX { 0 } else { n as usize }
}

// ── DES for VNC auth ─────────────────────────────────────────────────────────
// Minimal single-DES (same algorithm as vncd/src/des.rs, inlined here so
// vmctl has no dependency on the vncd crate).

#[rustfmt::skip]
const IP:  [u8; 64] = [58,50,42,34,26,18,10,2,60,52,44,36,28,20,12,4,62,54,46,38,30,22,14,6,64,56,48,40,32,24,16,8,57,49,41,33,25,17,9,1,59,51,43,35,27,19,11,3,61,53,45,37,29,21,13,5,63,55,47,39,31,23,15,7];
#[rustfmt::skip]
const FP:  [u8; 64] = [40,8,48,16,56,24,64,32,39,7,47,15,55,23,63,31,38,6,46,14,54,22,62,30,37,5,45,13,53,21,61,29,36,4,44,12,52,20,60,28,35,3,43,11,51,19,59,27,34,2,42,10,50,18,58,26,33,1,41,9,49,17,57,25];
#[rustfmt::skip]
const PC1: [u8; 56] = [57,49,41,33,25,17,9,1,58,50,42,34,26,18,10,2,59,51,43,35,27,19,11,3,60,52,44,36,63,55,47,39,31,23,15,7,62,54,46,38,30,22,14,6,61,53,45,37,29,21,13,5,28,20,12,4];
#[rustfmt::skip]
const PC2: [u8; 48] = [14,17,11,24,1,5,3,28,15,6,21,10,23,19,12,4,26,8,16,7,27,20,13,2,41,52,31,37,47,55,30,40,51,45,33,48,44,49,39,56,34,53,46,42,50,36,29,32];
#[rustfmt::skip]
const SH:  [u8; 16] = [1,1,2,2,2,2,2,2,1,2,2,2,2,2,2,1];
#[rustfmt::skip]
const E:   [u8; 48] = [32,1,2,3,4,5,4,5,6,7,8,9,8,9,10,11,12,13,12,13,14,15,16,17,16,17,18,19,20,21,20,21,22,23,24,25,24,25,26,27,28,29,28,29,30,31,32,1];
#[rustfmt::skip]
const PP:  [u8; 32] = [16,7,20,21,29,12,28,17,1,15,23,26,5,18,31,10,2,8,24,14,32,27,3,9,19,13,30,6,22,11,4,25];
#[rustfmt::skip]
const SB:  [[u8;64];8] = [
    [14,4,13,1,2,15,11,8,3,10,6,12,5,9,0,7,0,15,7,4,14,2,13,1,10,6,12,11,9,5,3,8,4,1,14,8,13,6,2,11,15,12,9,7,3,10,5,0,15,12,8,2,4,9,1,7,5,11,3,14,10,0,6,13],
    [15,1,8,14,6,11,3,4,9,7,2,13,12,0,5,10,3,13,4,7,15,2,8,14,12,0,1,10,6,9,11,5,0,14,7,11,10,4,13,1,5,8,12,6,9,3,2,15,13,8,10,1,3,15,4,2,11,6,7,12,0,5,14,9],
    [10,0,9,14,6,3,15,5,1,13,12,7,11,4,2,8,13,7,0,9,3,4,6,10,2,8,5,14,12,11,15,1,13,6,4,9,8,15,3,0,11,1,2,12,5,10,14,7,1,10,13,0,6,9,8,7,4,15,14,3,11,5,2,12],
    [7,13,14,3,0,6,9,10,1,2,8,5,11,12,4,15,13,8,11,5,6,15,0,3,4,7,2,12,1,10,14,9,10,6,9,0,12,11,7,13,15,1,3,14,5,2,8,4,3,15,0,6,10,1,13,8,9,4,5,11,12,7,2,14],
    [2,12,4,1,7,10,11,6,8,5,3,15,13,0,14,9,14,11,2,12,4,7,13,1,5,0,15,10,3,9,8,6,4,2,1,11,10,13,7,8,15,9,12,5,6,3,0,14,11,8,12,7,1,14,2,13,6,15,0,9,10,4,5,3],
    [12,1,10,15,9,2,6,8,0,13,3,4,14,7,5,11,10,15,4,2,7,12,9,5,6,1,13,14,0,11,3,8,9,14,15,5,2,8,12,3,7,0,4,10,1,13,11,6,4,3,2,12,9,5,15,10,11,14,1,7,6,0,8,13],
    [4,11,2,14,15,0,8,13,3,12,9,7,5,10,6,1,13,0,11,7,4,9,1,10,14,3,5,12,2,15,8,6,1,4,11,13,12,3,7,14,10,15,6,8,0,5,9,2,6,11,13,8,1,4,10,7,9,5,0,15,14,2,3,12],
    [13,2,8,4,6,15,11,1,10,9,3,14,5,0,12,7,1,15,13,8,10,3,7,4,12,5,6,11,0,14,9,2,7,11,4,1,9,12,14,2,0,6,10,13,15,3,5,8,2,1,14,7,4,10,8,13,15,12,9,0,3,5,6,11],
];

#[inline] fn gb64(b: u64, p: u8) -> u64 { (b >> (64 - p)) & 1 }
#[inline] fn sb64(b: u64, p: u8, v: u64) -> u64 { b | (v << (64 - p)) }

fn perm64(b: u64, t: &[u8]) -> u64 {
    let mut o = 0u64;
    for (i, &s) in t.iter().enumerate() { o = sb64(o, (i+1) as u8, gb64(b, s)); }
    o
}

fn rot28(h: u32, n: u8) -> u32 { ((h << n) | (h >> (28-n))) & 0x0FFF_FFFF }

fn des_subkeys(key64: u64) -> [u64; 16] {
    let pm = perm64(key64, &PC1);
    let mut c = ((pm >> 36) as u32) & 0x0FFF_FFFF;
    let mut d = ((pm >>  8) as u32) & 0x0FFF_FFFF;
    let mut sk = [0u64; 16];
    for (r, &sh) in SH.iter().enumerate() {
        c = rot28(c, sh); d = rot28(d, sh);
        let cd = ((c as u64) << 28) | (d as u64);
        // PC2
        let mut o = 0u64;
        for (i, &s) in PC2.iter().enumerate() {
            let bit = (cd >> (56 - s)) & 1;
            o = sb64(o, (i+1) as u8, bit);
        }
        sk[r] = o;
    }
    sk
}

fn des_feistel(r: u32, k: u64) -> u32 {
    let r64 = (r as u64) << 32;
    let mut ex = 0u64;
    for (i, &s) in E.iter().enumerate() { let b = (r64 >> (64-s)) & 1; ex = sb64(ex, (i+1) as u8, b); }
    let xd = ex ^ k;
    let mut so = 0u32;
    for i in 0..8usize {
        let g = ((xd >> (64 - (i*6+6))) & 0x3F) as usize;
        let row = ((g >> 4) & 2) | (g & 1);
        let col = (g >> 1) & 0xF;
        so = (so << 4) | (SB[i][row * 16 + col] as u32);
    }
    let s64 = (so as u64) << 32;
    let mut po = 0u32;
    for (i, &s) in PP.iter().enumerate() { po |= (((s64 >> (64-s)) & 1) as u32) << (31-i); }
    po
}

fn des_encrypt(block: u64, sk: &[u64; 16]) -> u64 {
    let pm = perm64(block, &IP);
    let mut l = (pm >> 32) as u32; let mut r = pm as u32;
    for &k in sk { let f = des_feistel(r, k); let nr = l ^ f; l = r; r = nr; }
    perm64(((r as u64) << 32) | (l as u64), &FP)
}

fn rev_byte(b: u8) -> u8 {
    let mut v = b;
    v = ((v & 0xF0) >> 4) | ((v & 0x0F) << 4);
    v = ((v & 0xCC) >> 2) | ((v & 0x33) << 2);
    v = ((v & 0xAA) >> 1) | ((v & 0x55) << 1);
    v
}

/// Encrypt a 16-byte VNC challenge with an 8-byte password (in-place).
fn vnc_des_encrypt(pw: &[u8; 8], chal: &mut [u8; 16]) {
    let mut key = 0u64;
    for (i, &b) in pw.iter().enumerate() { key |= (rev_byte(b) as u64) << (56 - i*8); }
    let sk = des_subkeys(key);
    for blk in 0..2 {
        let off = blk * 8;
        let mut b64 = 0u64;
        for i in 0..8 { b64 |= (chal[off+i] as u64) << (56 - i*8); }
        let enc = des_encrypt(b64, &sk);
        for i in 0..8 { chal[off+i] = ((enc >> (56-i*8)) & 0xFF) as u8; }
    }
}

fn vnc_verify(pw: &[u8; 8], chal: &[u8; 16], resp: &[u8; 16]) -> bool {
    let mut expected = *chal;
    vnc_des_encrypt(pw, &mut expected);
    let mut diff = 0u8;
    for i in 0..16 { diff |= expected[i] ^ resp[i]; }
    diff == 0
}

// ── RFB pixel format block ────────────────────────────────────────────────────

fn pixel_format_block() -> [u8; 16] {
    [
        32, 24,
        0,          // little-endian
        1,          // true-colour
        0, 0xFF,    // red-max
        0, 0xFF,    // green-max
        0, 0xFF,    // blue-max
        16, 8, 0,   // R/G/B shifts (ARGB: A=24, R=16, G=8, B=0)
        0, 0, 0,    // padding
    ]
}

// ── Framebuffer update helpers ────────────────────────────────────────────────

fn tile_dirty(cur: &[u32], prev: &[u32], stride: usize, tx: usize, ty: usize, tw: usize, th: usize) -> bool {
    for row in ty..ty+th {
        let off = row * stride + tx;
        if cur[off..off+tw] != prev[off..off+tw] { return true; }
    }
    false
}

/// Send a full-screen raw FramebufferUpdate.
fn send_full_update(sock: u32, fb: &[u32], sw: usize, sh: usize, buf: &mut Vec<u8>) -> bool {
    buf.clear();
    // FramebufferUpdate header: type=0, padding=0, n_rects=1 (BE16)
    buf.extend_from_slice(&[0, 0]);
    buf.extend_from_slice(&be16(1));
    // Rectangle header: x=0, y=0, w, h, encoding=Raw(0)
    buf.extend_from_slice(&be16(0));
    buf.extend_from_slice(&be16(0));
    buf.extend_from_slice(&be16(sw as u16));
    buf.extend_from_slice(&be16(sh as u16));
    buf.extend_from_slice(&be32(0)); // Raw encoding
    // Pixel data (ARGB little-endian → RFB BGRA? No: we told the client LE, R=16,G=8,B=0)
    for &px in &fb[..sw*sh] {
        buf.extend_from_slice(&px.to_le_bytes());
    }
    send_all(sock, buf)
}

/// Send dirty tiles as a FramebufferUpdate with Raw rectangles.
/// Returns false on connection error.
fn send_dirty_update(
    sock: u32,
    cur: &[u32],
    prev: &mut [u32],
    sw: usize,
    sh: usize,
    buf: &mut Vec<u8>,
) -> bool {
    let tiles_x = (sw + TILE - 1) / TILE;
    let tiles_y = (sh + TILE - 1) / TILE;

    // Collect dirty rectangles (tile-granular).  Max 512 tiles.
    let mut rects: [(u16,u16,u16,u16); 512] = [(0,0,0,0); 512];
    let mut n_rects = 0usize;

    for ty_i in 0..tiles_y {
        let ty = ty_i * TILE;
        let th = TILE.min(sh - ty);
        for tx_i in 0..tiles_x {
            let tx = tx_i * TILE;
            let tw = TILE.min(sw - tx);
            if tile_dirty(cur, prev, sw, tx, ty, tw, th) {
                if n_rects < rects.len() {
                    rects[n_rects] = (tx as u16, ty as u16, tw as u16, th as u16);
                    n_rects += 1;
                }
            }
        }
    }

    if n_rects == 0 { return true; }

    buf.clear();
    buf.extend_from_slice(&[0, 0]); // type=FramebufferUpdate, padding
    buf.extend_from_slice(&be16(n_rects as u16));

    for &(rx, ry, rw, rh) in &rects[..n_rects] {
        buf.extend_from_slice(&be16(rx));
        buf.extend_from_slice(&be16(ry));
        buf.extend_from_slice(&be16(rw));
        buf.extend_from_slice(&be16(rh));
        buf.extend_from_slice(&be32(0)); // Raw
        let (rx, ry, rw, rh) = (rx as usize, ry as usize, rw as usize, rh as usize);
        for row in ry..ry+rh {
            let off = row * sw + rx;
            let row_bytes = unsafe {
                core::slice::from_raw_parts(cur[off..].as_ptr() as *const u8, rw * 4)
            };
            buf.extend_from_slice(row_bytes);
        }
    }

    // Update prev for next comparison.
    for &(rx, ry, rw, rh) in &rects[..n_rects] {
        let (rx, ry, rw, rh) = (rx as usize, ry as usize, rw as usize, rh as usize);
        for row in ry..ry+rh {
            let off = row * sw + rx;
            prev[off..off+rw].copy_from_slice(&cur[off..off+rw]);
        }
    }

    send_all(sock, buf)
}

// ── KeySym → PS/2 scancode (set 1) ───────────────────────────────────────────
// Maps X11 keysyms (from RFB KeyEvent) to PS/2 set-1 scancodes.
// Only the make code is given; release = make | 0x80.

fn keysym_to_ps2(ks: u32) -> Option<u8> {
    // ASCII letters
    if ks >= 0x61 && ks <= 0x7A {
        return Some(match ks as u8 - b'a' {
            0 => 0x1E, 1 => 0x30, 2 => 0x2E, 3 => 0x20, 4 => 0x12,
            5 => 0x21, 6 => 0x22, 7 => 0x23, 8 => 0x17, 9 => 0x24,
            10=> 0x25, 11=> 0x26, 12=> 0x32, 13=> 0x31, 14=> 0x18,
            15=> 0x19, 16=> 0x10, 17=> 0x13, 18=> 0x1F, 19=> 0x14,
            20=> 0x16, 21=> 0x2F, 22=> 0x11, 23=> 0x2D, 24=> 0x15,
            25=> 0x2C, _ => return None,
        });
    }
    // Upper-case → same scancode as lowercase
    if ks >= 0x41 && ks <= 0x5A {
        return keysym_to_ps2(ks + 0x20);
    }
    Some(match ks {
        // Digits
        0x30 => 0x0B, 0x31 => 0x02, 0x32 => 0x03, 0x33 => 0x04,
        0x34 => 0x05, 0x35 => 0x06, 0x36 => 0x07, 0x37 => 0x08,
        0x38 => 0x09, 0x39 => 0x0A,
        // Space / punctuation
        0x20 => 0x39,                    // Space
        0x0D | 0xFF0D | 0xFF8D => 0x1C,  // Return
        0x09 | 0xFF09 => 0x0F,           // Tab
        0x1B | 0xFF1B => 0x01,           // Escape
        0xFF08 => 0x0E,                  // BackSpace
        0xFFFF => 0x53,                  // Delete
        // Function keys
        0xFFBE => 0x3B, 0xFFBF => 0x3C, 0xFFC0 => 0x3D, 0xFFC1 => 0x3E,
        0xFFC2 => 0x3F, 0xFFC3 => 0x40, 0xFFC4 => 0x41, 0xFFC5 => 0x42,
        0xFFC6 => 0x43, 0xFFC7 => 0x44, 0xFFC8 => 0x57, 0xFFC9 => 0x58,
        // Arrows
        0xFF51 => 0x4B, 0xFF52 => 0x48, 0xFF53 => 0x4D, 0xFF54 => 0x50,
        // Nav
        0xFF50 => 0x47, 0xFF57 => 0x4F, // Home / End
        0xFF55 => 0x49, 0xFF56 => 0x51, // PgUp / PgDn
        0xFF63 => 0x52,                  // Insert
        // Modifiers
        0xFFE1 | 0xFFE2 => 0x2A,        // Shift (left/right → left shift code)
        0xFFE3 | 0xFFE4 => 0x1D,        // Ctrl
        0xFFE9 | 0xFFEA => 0x38,        // Alt
        0xFFE5 => 0x3A,                 // Caps Lock
        // Punctuation (unshifted)
        0x2D => 0x0C, // -
        0x3D => 0x0D, // =
        0x5B => 0x1A, // [
        0x5D => 0x1B, // ]
        0x5C => 0x2B, // backslash
        0x3B => 0x27, // ;
        0x27 => 0x28, // '
        0x60 => 0x29, // `
        0x2C => 0x33, // ,
        0x2E => 0x34, // .
        0x2F => 0x35, // /
        _ => return None,
    })
}

// ── RFB session ───────────────────────────────────────────────────────────────

/// Run a single VNC client session on `sock`.
/// `handle` is the running VM.  `password` is None for no-auth.
fn run_session(sock: u32, handle: &VmHandle, sw: usize, sh: usize, password: Option<&[u8; 8]>) {
    // ── 1. Version handshake ─────────────────────────────────────────────
    if !send_all(sock, b"RFB 003.008\n") { return; }
    let mut ver = [0u8; 12];
    if !recv_exact(sock, &mut ver) { return; }
    // Accept any 3.x client.

    // ── 2. Security negotiation ──────────────────────────────────────────
    if password.is_some() {
        // Offer type 2 (VNC Auth)
        if !send_all(sock, &[1u8, 2u8]) { return; }
        let mut sec = [0u8; 1];
        if !recv_exact(sock, &mut sec) { return; }
        if sec[0] != 2 {
            // Client rejected — send failure
            send_all(sock, &be32(1));
            return;
        }
        // Send 16-byte challenge
        let mut challenge = [0u8; 16];
        let mut rng = [0u8; 16];
        let _ = sys::random(&mut rng);
        challenge.copy_from_slice(&rng);
        if !send_all(sock, &challenge) { return; }
        // Receive response
        let mut response = [0u8; 16];
        if !recv_exact(sock, &mut response) { return; }
        let pw = password.unwrap();
        if !vnc_verify(pw, &challenge, &response) {
            send_all(sock, &be32(1)); // SecurityResult: failed
            return;
        }
        send_all(sock, &be32(0)); // SecurityResult: OK
    } else {
        // Offer type 1 (None)
        if !send_all(sock, &[1u8, 1u8]) { return; }
        let mut sec = [0u8; 1];
        if !recv_exact(sock, &mut sec) { return; }
        // RFB 3.8: send SecurityResult even for None
        if !send_all(sock, &be32(0)) { return; }
    }

    // ── 3. ClientInit ────────────────────────────────────────────────────
    let mut ci = [0u8; 1];
    if !recv_exact(sock, &mut ci) { return; }
    // shared-flag byte — ignore

    // ── 4. ServerInit ────────────────────────────────────────────────────
    // framebuffer-width(2) + framebuffer-height(2) + pixel-format(16) + name-len(4) + name
    let name = b"vmctl-vm";
    let mut si = Vec::new();
    si.extend_from_slice(&be16(sw as u16));
    si.extend_from_slice(&be16(sh as u16));
    si.extend_from_slice(&pixel_format_block());
    si.extend_from_slice(&be32(name.len() as u32));
    si.extend_from_slice(name);
    if !send_all(sock, &si) { return; }

    // ── 5. Main loop ─────────────────────────────────────────────────────
    let n_pixels = sw * sh;
    let mut screen_buf: Vec<u32> = alloc::vec![0u32; n_pixels];
    let mut prev_buf:   Vec<u32> = alloc::vec![0u32; n_pixels];
    let mut send_buf:   Vec<u8>  = Vec::with_capacity(sw * 4 * 32 + 64);

    let mut pending_update = false;
    let mut clean_frames = 0u32;
    let mut first_frame = true;

    // RFB client message types:
    // 0 = SetPixelFormat (20 bytes total)
    // 2 = SetEncodings (variable)
    // 3 = FramebufferUpdateRequest (10 bytes)
    // 4 = KeyEvent (8 bytes)
    // 5 = PointerEvent (6 bytes)
    // 6 = ClientCutText (variable)

    loop {
        // ── Poll client messages (non-blocking) ──────────────────────────
        let mut hdr = [0u8; 1];
        let n = recv_nowait(sock, &mut hdr);
        if n > 0 {
            match hdr[0] {
                0 => { // SetPixelFormat — read and ignore (we keep our format)
                    let mut rest = [0u8; 19];
                    if !recv_exact(sock, &mut rest) { return; }
                }
                2 => { // SetEncodings
                    let mut buf4 = [0u8; 3];
                    if !recv_exact(sock, &mut buf4) { return; }
                    let count = from_be16(&buf4[1..]) as usize;
                    let mut enc_buf = alloc::vec![0u8; count * 4];
                    if !recv_exact(sock, &mut enc_buf) { return; }
                    // We only support Raw — ignore what the client requests.
                }
                3 => { // FramebufferUpdateRequest
                    let mut rest = [0u8; 9];
                    if !recv_exact(sock, &mut rest) { return; }
                    pending_update = true;
                }
                4 => { // KeyEvent: down(1) + pad(2) + keysym(4)
                    let mut rest = [0u8; 7];
                    if !recv_exact(sock, &mut rest) { return; }
                    let down  = rest[0] != 0;
                    let keysym = from_be32(&rest[3..7]);
                    if let Some(sc) = keysym_to_ps2(keysym) {
                        if down {
                            handle.ps2_key_press(sc);
                        } else {
                            handle.ps2_key_release(sc);
                        }
                    }
                }
                5 => { // PointerEvent: buttons(1) + x(2) + y(2)
                    let mut rest = [0u8; 5];
                    if !recv_exact(sock, &mut rest) { return; }
                    let buttons = rest[0];
                    let x = from_be16(&rest[1..3]) as i16;
                    let y = from_be16(&rest[3..5]) as i16;
                    handle.ps2_mouse_move(x, y, buttons);
                }
                6 => { // ClientCutText
                    let mut buf7 = [0u8; 7];
                    if !recv_exact(sock, &mut buf7) { return; }
                    let len = from_be32(&buf7[3..7]) as usize;
                    let mut text = alloc::vec![0u8; len];
                    if !recv_exact(sock, &mut text) { return; }
                }
                _ => { return; } // unknown message type — close
            }
        }

        // ── Capture VGA framebuffer from guest ───────────────────────────
        if pending_update || first_frame {
            let frame_opt = handle.vga_framebuffer();
            let frame_updated = if let Some((pixels, _fw, _fh, _bpp)) = frame_opt {
                let n_u32 = (pixels.len() / 4).min(n_pixels);
                for i in 0..n_u32 {
                    screen_buf[i] = u32::from_le_bytes([
                        pixels[i*4], pixels[i*4+1], pixels[i*4+2], pixels[i*4+3],
                    ]);
                }
                true
            } else {
                // No framebuffer yet (e.g. guest still in real-mode) — blank
                for px in screen_buf.iter_mut() { *px = 0; }
                false
            };

            if first_frame {
                if !send_full_update(sock, &screen_buf, sw, sh, &mut send_buf) { return; }
                prev_buf.copy_from_slice(&screen_buf);
                first_frame = false;
                pending_update = false;
                continue;
            }

            if frame_updated {
                let has_dirty = (0..sw*sh).any(|i| screen_buf[i] != prev_buf[i]);
                if has_dirty {
                    clean_frames = 0;
                    if !send_dirty_update(sock, &screen_buf, &mut prev_buf, sw, sh, &mut send_buf) {
                        return;
                    }
                } else {
                    clean_frames += 1;
                }
            }
            pending_update = false;
        }

        // Adaptive sleep: fast when active, slow when idle.
        if clean_frames >= IDLE_THRESH {
            process::sleep(IDLE_MS);
        } else {
            process::sleep(ACTIVE_MS);
        }
    }
}

// ── VM setup helper ───────────────────────────────────────────────────────────

fn setup_vm(config: &VmConfig) -> Option<VmHandle> {
    if !libcorevm_client::init() {
        println!("[vmctl-vnc] ERROR: Failed to load libcorevm.so");
        return None;
    }

    let handle = match VmHandle::new(config.ram_mb) {
        Ok(h) => h,
        Err(e) => {
            println!("[vmctl-vnc] ERROR: {}", e);
            return None;
        }
    };

    handle.create_vcpu(0);
    handle.setup_standard_devices();
    handle.setup_ahci(2);

    if config.net_enabled {
        handle.setup_e1000(&config.mac_address);
    }

    // Disk
    if !config.disk_image.is_empty() {
        let fd = anyos_std::fs::open(&config.disk_image, 0);
        if fd != u32::MAX {
            let size = anyos_std::fs::lseek(fd, 0, 2) as u64;
            anyos_std::fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_disk(0, fd as i32, size);
                println!("[vmctl-vnc] Disk: {} ({} B)", config.disk_image, size);
            } else { anyos_std::fs::close(fd); }
        } else {
            println!("[vmctl-vnc] WARNING: cannot open disk: {}", config.disk_image);
        }
    }

    // ISO
    if !config.iso_image.is_empty() {
        let fd = anyos_std::fs::open(&config.iso_image, 0);
        if fd != u32::MAX {
            let size = anyos_std::fs::lseek(fd, 0, 2) as u64;
            anyos_std::fs::lseek(fd, 0, 0);
            if size > 0 {
                handle.ahci_attach_cdrom(1, fd as i32, size);
                println!("[vmctl-vnc] ISO: {} ({} B)", config.iso_image, size);
            } else { anyos_std::fs::close(fd); }
        }
    }

    // BIOS
    let bios_path = if config.bios_type == "seabios" { BIOS_PATH_SEABIOS } else { BIOS_PATH_COREVM };
    let bios = read_file(bios_path);
    if bios.is_empty() {
        println!("[vmctl-vnc] ERROR: BIOS not found at {}", bios_path);
        return None;
    }
    if config.bios_type == "seabios" {
        handle.load_binary(0xC0000, &bios);
        handle.load_binary(0xFFFC_0000, &bios);
    } else {
        handle.load_binary(0xF0000, &bios);
    }

    // Initial CPU state: real mode, CS:IP = F000:FFF0
    let mut sregs = handle.get_vcpu_sregs(0);
    sregs.cs.base = 0xF0000; sregs.cs.selector = 0xF000;
    sregs.cs.limit = 0xFFFF;  sregs.cs.type_ = 0x0B;
    sregs.cs.present = 1;     sregs.cs.s = 1;
    sregs.ds.base = 0; sregs.ds.selector = 0; sregs.ds.limit = 0xFFFF;
    sregs.ds.type_ = 0x03; sregs.ds.present = 1; sregs.ds.s = 1;
    sregs.es = sregs.ds; sregs.ss = sregs.ds;
    sregs.fs = sregs.ds; sregs.gs = sregs.ds;
    handle.set_vcpu_sregs(0, &sregs);
    let mut regs = handle.get_vcpu_regs(0);
    regs.rip = 0xFFF0; regs.rflags = 0x2;
    handle.set_vcpu_regs(0, &regs);

    Some(handle)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run a VM and serve its display as VNC on `port`.
pub fn cmd_vnc(config: VmConfig, port: u16, password: Option<&[u8; 8]>) {
    println!("[vmctl-vnc] Starting VM '{}' ({} MiB)...", config.name, config.ram_mb);

    let handle = match setup_vm(&config) {
        Some(h) => h,
        None => { anyos_std::process::exit(1); }
    };

    // Determine display dimensions from the VGA mode (default 1024×768).
    let (mut sw, mut sh) = {
        let (w, h, _) = handle.vga_get_mode();
        if w > 0 && h > 0 && w <= MAX_DIM as u32 && h <= MAX_DIM as u32 {
            (w as usize, h as usize)
        } else {
            (1024, 768)
        }
    };

    // Start TCP listener.
    let listener = net::tcp_listen(port, 4);
    if listener == u32::MAX {
        println!("[vmctl-vnc] ERROR: tcp_listen failed on port {}", port);
        anyos_std::process::exit(1);
    }

    // Query local IP for the connection message.
    let mut net_cfg = [0u8; 24];
    let (a, b, c, d) = if net::get_config(&mut net_cfg) == 0 {
        (net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3])
    } else {
        (0, 0, 0, 0)
    };
    println!("[vmctl-vnc] VNC listening on {}.{}.{}.{}:{}", a, b, c, d, port);
    println!("[vmctl-vnc] Connect with any VNC viewer. Press Ctrl+C to stop.");
    println!("[vmctl-vnc] Auth: {}", if password.is_some() { "VNC password" } else { "none (open)" });

    // Track the current VNC client socket (single session).
    let mut client_sock: u32 = u32::MAX;
    let mut client_pid:  u32 = u32::MAX;

    let mut exit_count: u64 = 0;

    loop {
        // ── Run one VM-exit step ─────────────────────────────────────────
        let exit = handle.run_vcpu(0);
        match exit {
            libcorevm_client::VmExitReason::IoIn { port, size } => {
                let mut data = [0u8; 4];
                handle.handle_io_exit(port, 0, size, &mut data[..size as usize]);
            }
            libcorevm_client::VmExitReason::IoOut { port, size, data } => {
                let bytes = data.to_le_bytes();
                let mut buf = [0u8; 4];
                buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
                handle.handle_io_exit(port, 1, size, &mut buf[..size as usize]);
            }
            libcorevm_client::VmExitReason::MmioRead { addr, size } => {
                let mut data = [0u8; 8];
                handle.handle_mmio_exit(addr, 0, size, &mut data[..size as usize], 0, 0);
            }
            libcorevm_client::VmExitReason::MmioWrite { addr, size, data } => {
                let bytes = data.to_le_bytes();
                let mut buf = [0u8; 8];
                buf[..size as usize].copy_from_slice(&bytes[..size as usize]);
                handle.handle_mmio_exit(addr, 1, size, &mut buf[..size as usize], 0, 0);
            }
            libcorevm_client::VmExitReason::Shutdown => {
                println!("[vmctl-vnc] Guest shutdown");
                break;
            }
            libcorevm_client::VmExitReason::Error => {
                println!("[vmctl-vnc] VM error");
                break;
            }
            _ => {}
        }
        exit_count += 1;

        // Drain serial output to stdout periodically.
        if exit_count % 500 == 0 {
            let serial = handle.serial_take_output();
            if !serial.is_empty() {
                if let Ok(t) = core::str::from_utf8(&serial) {
                    anyos_std::print!("{}", t);
                }
            }
        }

        // ── Accept new VNC client (non-blocking) ─────────────────────────
        // Reap finished client child.
        if client_pid != u32::MAX {
            if process::try_waitpid(client_pid) != process::STILL_RUNNING {
                client_pid  = u32::MAX;
                client_sock = u32::MAX;
            }
        }

        // Only accept one client at a time.
        if client_sock == u32::MAX && exit_count % 100 == 0 {
            let (sock, ip, _port) = net::tcp_accept_nowait(listener);
            if sock != u32::MAX {
                println!("[vmctl-vnc] VNC client connected from {}.{}.{}.{}",
                    ip[0], ip[1], ip[2], ip[3]);

                // Refresh screen dimensions before forking.
                let (fw, fh, _) = handle.vga_get_mode();
                if fw > 0 && fh > 0 && fw <= MAX_DIM as u32 && fh <= MAX_DIM as u32 {
                    sw = fw as usize;
                    sh = fh as usize;
                }

                // Fork child to handle the RFB session without blocking the VM.
                let sw_c = sw;
                let sh_c = sh;
                let pw_copy: Option<[u8; 8]> = password.copied();

                let tid = process::fork();
                if tid == 0 {
                    // Child: run RFB session, then exit.
                    let pw_ref = pw_copy.as_ref();
                    run_session(sock, &handle, sw_c, sh_c, pw_ref.map(|p| p as &[u8;8]));
                    net::tcp_close(sock);
                    process::exit(0);
                } else if tid != u32::MAX {
                    client_pid  = tid;
                    client_sock = sock;
                } else {
                    println!("[vmctl-vnc] fork failed — closing client");
                    net::tcp_close(sock);
                }
            }
        }
    }

    // Clean up.
    if client_pid != u32::MAX { process::kill(client_pid); }
    net::tcp_close(listener);
    println!("[vmctl-vnc] Done.");
}
