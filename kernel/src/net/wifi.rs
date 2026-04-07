//! 802.11 WiFi management — scan, auth, associate, WPA2 handshake.
//!
//! This module contains the hardware-independent 802.11 management logic.
//! A hardware driver (e.g. RTL8188EU) calls into this module to:
//!   - Report received management frames
//!   - Receive commands to transmit management frames
//!
//! After successful association + WPA2 handshake, the driver presents
//! itself as a plain `NetworkDriver` so the existing TCP/IP stack works
//! without modification.
//!
//! # State machine
//!
//! ```
//! Disconnected → Scanning → Associating → Authenticating (WPA2) → Connected
//! ```
//!
//! # WPA2 (CCMP/AES) support
//!
//! Implements the EAPOL 4-way handshake:
//!   1. ANonce  ← AP  (EAPOL Key frame 1)
//!   2. SNonce  → AP  (EAPOL Key frame 2, MIC over frame with PMK-derived PTK)
//!   3. GTK     ← AP  (EAPOL Key frame 3, encrypted with KEK)
//!   4. ACK     → AP  (EAPOL Key frame 4)
//!
//! CCMP encryption/decryption for data frames uses AES-128-CCM with a
//! 48-bit PN (packet number) replay counter.

use alloc::vec::Vec;
use crate::crypto::pbkdf2::pbkdf2_sha1;
use crate::crypto::wpa2::{compute_mic, derive_ptk, Ptk};
use crate::sync::spinlock::Spinlock;

// ── Types ─────────────────────────────────────────────────────────────────────

/// WiFi security mode of an AP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WifiSecurity {
    Open,
    Wpa2Personal,
}

/// WiFi driver state machine.
#[derive(Debug, Clone)]
pub enum WifiState {
    Disconnected,
    Scanning,
    Associating {
        bssid: [u8; 6],
        ssid: [u8; 32],
        ssid_len: usize,
        channel: u8,
    },
    Authenticating {
        bssid: [u8; 6],
        ssid: [u8; 32],
        ssid_len: usize,
    },
    Connected {
        bssid: [u8; 6],
        ssid: [u8; 32],
        ssid_len: usize,
        channel: u8,
    },
}

/// Information about a scanned BSS (access point).
#[derive(Debug, Clone)]
pub struct BssInfo {
    pub bssid: [u8; 6],
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub channel: u8,
    pub rssi: i8,
    pub security: WifiSecurity,
}

/// Configuration for connecting to a WPA2 network.
#[derive(Debug, Clone)]
pub struct WifiConfig {
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub password: [u8; 64],
    pub password_len: usize,
}

/// Parsed EAPOL Key frame fields.
#[derive(Debug, Clone)]
pub struct EapolKey {
    pub version: u8,
    /// Key information flags word.
    pub key_info: u16,
    pub key_length: u16,
    pub replay_counter: u64,
    /// Key nonce (ANonce or SNonce, 32 bytes).
    pub nonce: [u8; 32],
    /// MIC field (16 bytes, zeroed during MIC computation).
    pub mic: [u8; 16],
    pub key_data_len: u16,
}

// ── Global state ──────────────────────────────────────────────────────────────

static WIFI_STATE: Spinlock<WifiState> = Spinlock::new(WifiState::Disconnected);
static SCAN_RESULTS: Spinlock<Vec<BssInfo>> = Spinlock::new(Vec::new());
static WIFI_CONFIG: Spinlock<Option<WifiConfig>> = Spinlock::new(None);
static PTK: Spinlock<Option<Ptk>> = Spinlock::new(None);
static ANONCE: Spinlock<[u8; 32]> = Spinlock::new([0u8; 32]);
static SNONCE: Spinlock<[u8; 32]> = Spinlock::new([0u8; 32]);

// ── LFSR for SNonce generation ────────────────────────────────────────────────

/// Simple 64-bit Galois LFSR for pseudo-random nonce generation.
/// Taps at positions 64,63,61,60 (x^64 + x^63 + x^61 + x^60 + 1).
fn lfsr_next(state: &mut u64) -> u8 {
    let bit = ((*state >> 63) ^ (*state >> 62) ^ (*state >> 60) ^ (*state >> 59)) & 1;
    *state = (*state << 1) | bit;
    *state as u8
}

/// Generate a pseudo-random 32-byte nonce using an LFSR seeded by timer ticks.
fn generate_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    // Seed from timer ticks + a compile-time constant to add entropy
    let mut lfsr_state = crate::arch::hal::timer_current_ticks() as u64;
    lfsr_state ^= 0xDEAD_BEEF_CAFE_1234;
    if lfsr_state == 0 {
        lfsr_state = 0x1234_5678_9ABC_DEF0;
    }
    for byte in nonce.iter_mut() {
        // Advance several times per byte to spread entropy
        for _ in 0..8 {
            *byte ^= lfsr_next(&mut lfsr_state);
        }
    }
    nonce
}

// ── PRF-384 / PTK derivation ──────────────────────────────────────────────────
//
// IEEE 802.11-2012 Section 11.6.1.2 — PRF-384
//
// PRF(K, A, B) = HMAC-SHA1(K, A || 0x00 || B || counter)
// Repeated with counter = 0,1,2 to produce 60 bytes; take first 48.
//
// PTK = PRF-384(PMK,
//               "Pairwise key expansion",
//               min(AP_MAC,STA_MAC) || max(AP_MAC,STA_MAC) ||
//               min(ANonce,SNonce) || max(ANonce,SNonce))
//
// KCK = PTK[0..16], KEK = PTK[16..32], TK = PTK[32..48]

// ── EAPOL frame parsing ───────────────────────────────────────────────────────
//
// EAPOL Key frame layout (802.1X-2010 / WPA2):
//
//  Offset  Len  Field
//  ------  ---  -----
//   0       1   Version
//   1       1   Type (3 = Key)
//   2       2   Length (big-endian)
//   4       1   Key Descriptor Type (2 = RSN/WPA2)
//   5       2   Key Information (bit flags, big-endian)
//   7       2   Key Length (big-endian)
//   9       8   Replay Counter (big-endian)
//  17      32   Key Nonce (ANonce or SNonce)
//  49      16   Key IV (usually zeroed)
//  65       8   Key RSC
//  73       8   Reserved
//  81      16   MIC (zeroed when computing MIC)
//  97       2   Key Data Length (big-endian)
//  99+      ?   Key Data
//
// Key Information bit fields (bits numbered from LSB = bit 0):
//   bit 3  = Pairwise (1) vs Group (0) key
//   bit 6  = Install
//   bit 7  = Key ACK  (AP→STA: AP generated ANonce)
//   bit 8  = Key MIC  (frame contains a valid MIC)
//   bit 9  = Secure
//   bit 13 = Encrypted Key Data (KEK used)

/// Parse an EAPOL Key frame from raw bytes.
/// Returns None if the frame is too short or not a Key frame.
pub fn parse_eapol_key(frame: &[u8]) -> Option<EapolKey> {
    // Minimum EAPOL Key frame: 99 bytes header (no key data)
    if frame.len() < 99 {
        return None;
    }
    // Type must be 3 (Key)
    if frame[1] != 3 {
        return None;
    }
    // Key Descriptor Type must be 2 (RSN)
    if frame[4] != 2 {
        return None;
    }

    let version = frame[0];
    let key_info = u16::from_be_bytes([frame[5], frame[6]]);
    let key_length = u16::from_be_bytes([frame[7], frame[8]]);
    let replay_counter = u64::from_be_bytes([
        frame[9], frame[10], frame[11], frame[12],
        frame[13], frame[14], frame[15], frame[16],
    ]);

    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&frame[17..49]);

    let mut mic = [0u8; 16];
    mic.copy_from_slice(&frame[81..97]);

    let key_data_len = u16::from_be_bytes([frame[97], frame[98]]);

    Some(EapolKey {
        version,
        key_info,
        key_length,
        replay_counter,
        nonce,
        mic,
        key_data_len,
    })
}

// ── 802.11 management frame building ─────────────────────────────────────────
//
// 802.11 MAC frame header (management):
//
//  Offset  Len  Field
//  ------  ---  -----
//   0       2   Frame Control
//               bits[1:0]  = protocol version (always 0)
//               bits[3:2]  = type (00 = management)
//               bits[7:4]  = subtype
//   2       2   Duration/ID
//   4       6   Address 1 (destination / receiver)
//  10       6   Address 2 (source / transmitter)
//  16       6   Address 3 (BSSID)
//  22       2   Sequence Control (fragment[3:0] | sequence[15:4])
//  24+      ?   Frame body (subtype dependent)
//
// Subtypes (management):
//   0x00 = Association Request
//   0x01 = Association Response
//   0x04 = Probe Request
//   0x05 = Probe Response
//   0x08 = Beacon
//   0x0B = Authentication
//   0x0C = Deauthentication

/// Frame control for Authentication frame (type=management, subtype=0x0B).
const FC_AUTH: [u8; 2] = [0xB0, 0x00];
/// Frame control for Association Request (type=management, subtype=0x00).
const FC_ASSOC_REQ: [u8; 2] = [0x00, 0x00];
/// Frame control for Probe Request (type=management, subtype=0x04).
const FC_PROBE_REQ: [u8; 2] = [0x40, 0x00];

/// Write a 6-byte 802.11 MAC header address field into `buf` at `offset`.
fn write_addr(buf: &mut [u8], offset: usize, addr: &[u8; 6]) {
    buf[offset..offset + 6].copy_from_slice(addr);
}

/// Build a wildcard Probe Request frame (all channels, broadcast SSID).
/// Returns a 64-byte fixed-size array padded with zeroes.
pub fn build_probe_request(our_mac: &[u8; 6]) -> [u8; 64] {
    let broadcast = [0xFFu8; 6];
    let mut frame = [0u8; 64];

    // Frame Control: Probe Request
    frame[0..2].copy_from_slice(&FC_PROBE_REQ);
    // Duration: 0
    frame[2] = 0;
    frame[3] = 0;
    // Address 1: broadcast
    write_addr(&mut frame, 4, &broadcast);
    // Address 2: our MAC
    write_addr(&mut frame, 10, our_mac);
    // Address 3: broadcast BSSID
    write_addr(&mut frame, 16, &broadcast);
    // Sequence Control: 0
    frame[22] = 0;
    frame[23] = 0;

    // Frame body: SSID element (wildcard = length 0)
    // Element ID=0 (SSID), Length=0
    frame[24] = 0;   // Element ID: SSID
    frame[25] = 0;   // Length: 0 (wildcard)

    // Supported Rates element (Element ID=1)
    // Rates: 1, 2, 5.5, 11, 6, 9, 12, 18 Mbps (basic set)
    frame[26] = 1;   // Element ID: Supported Rates
    frame[27] = 8;   // Length
    frame[28] = 0x82; // 1 Mbps   (basic)
    frame[29] = 0x84; // 2 Mbps   (basic)
    frame[30] = 0x8B; // 5.5 Mbps (basic)
    frame[31] = 0x96; // 11 Mbps  (basic)
    frame[32] = 0x0C; // 6 Mbps
    frame[33] = 0x12; // 9 Mbps
    frame[34] = 0x18; // 12 Mbps
    frame[35] = 0x24; // 18 Mbps

    frame
}

/// Build an Open System Authentication Request frame.
/// Returns a 30-byte fixed-size array.
///
/// Authentication frame body:
///   Authentication Algorithm Number = 0 (Open System)
///   Authentication Transaction Sequence Number = 1 (request)
///   Status Code = 0 (reserved in request)
pub fn build_auth_request(bssid: &[u8; 6], our_mac: &[u8; 6], seq: u16) -> [u8; 30] {
    let mut frame = [0u8; 30];

    // Frame Control: Authentication
    frame[0..2].copy_from_slice(&FC_AUTH);
    // Duration
    frame[2] = 0;
    frame[3] = 0;
    // Address 1: AP (destination)
    write_addr(&mut frame, 4, bssid);
    // Address 2: our MAC (source)
    write_addr(&mut frame, 10, our_mac);
    // Address 3: BSSID
    write_addr(&mut frame, 16, bssid);
    // Sequence Control
    let seq_ctrl = (seq << 4) & 0xFFF0;
    let seq_bytes = seq_ctrl.to_le_bytes();
    frame[22] = seq_bytes[0];
    frame[23] = seq_bytes[1];

    // Authentication Algorithm: Open System (0)
    frame[24] = 0x00;
    frame[25] = 0x00;
    // Authentication Transaction Sequence: 1
    frame[26] = 0x01;
    frame[27] = 0x00;
    // Status Code: 0 (success/reserved in request)
    frame[28] = 0x00;
    frame[29] = 0x00;

    frame
}

/// Build an Association Request frame for WPA2 Personal.
/// Returns a Vec<u8> because the length depends on the SSID.
///
/// Association Request body:
///   Capability Information (2 bytes): ESS + Privacy + ShortPreamble + ShortSlotTime
///   Listen Interval (2 bytes): 10
///   SSID element (variable)
///   Supported Rates element
///   RSN (WPA2) element
pub fn build_assoc_request(
    bssid: &[u8; 6],
    our_mac: &[u8; 6],
    ssid: &[u8],
    ssid_len: usize,
    seq: u16,
) -> Vec<u8> {
    let ssid = &ssid[..ssid_len];
    let mut frame = Vec::with_capacity(24 + 4 + 2 + ssid_len + 12 + 26);

    // 802.11 header (24 bytes)
    frame.extend_from_slice(&FC_ASSOC_REQ);
    frame.push(0x00); // Duration lo
    frame.push(0x00); // Duration hi
    frame.extend_from_slice(bssid);        // Address 1: AP
    frame.extend_from_slice(our_mac);      // Address 2: STA
    frame.extend_from_slice(bssid);        // Address 3: BSSID
    let seq_ctrl = (seq << 4) & 0xFFF0;
    frame.extend_from_slice(&seq_ctrl.to_le_bytes());

    // Capability Information: ESS(bit0) | Privacy(bit4) | ShortPreamble(bit5) | ShortSlotTime(bit10)
    // = 0x0431
    frame.push(0x31);
    frame.push(0x04);

    // Listen Interval: 10 TBTT
    frame.push(0x0A);
    frame.push(0x00);

    // SSID element
    frame.push(0x00);           // Element ID: SSID
    frame.push(ssid_len as u8); // Length
    frame.extend_from_slice(ssid);

    // Supported Rates (same as probe request)
    frame.push(0x01); // Element ID
    frame.push(0x08); // Length = 8
    frame.extend_from_slice(&[0x82, 0x84, 0x8B, 0x96, 0x0C, 0x12, 0x18, 0x24]);

    // Extended Supported Rates (54 Mbps, etc.)
    frame.push(0x32); // Element ID: Extended Supported Rates
    frame.push(0x04); // Length = 4
    frame.extend_from_slice(&[0x30, 0x48, 0x60, 0x6C]); // 24, 36, 48, 54 Mbps

    // RSN (WPA2) Information Element
    // Ref: IEEE 802.11-2012 Section 8.4.2.27
    //   Element ID = 48 (0x30)
    //   Version = 1
    //   Group Cipher Suite: 00-0F-AC-04 (CCMP-128)
    //   Pairwise Cipher Suite Count: 1
    //   Pairwise Cipher Suite: 00-0F-AC-04 (CCMP-128)
    //   AKM Suite Count: 1
    //   AKM Suite: 00-0F-AC-02 (PSK)
    //   RSN Capabilities: 0x0000
    frame.push(0x30); // Element ID: RSN
    frame.push(0x14); // Length = 20
    frame.extend_from_slice(&[
        0x01, 0x00,             // Version: 1
        0x00, 0x0F, 0xAC, 0x04, // Group Cipher: CCMP-128
        0x01, 0x00,             // Pairwise Suite Count: 1
        0x00, 0x0F, 0xAC, 0x04, // Pairwise Cipher: CCMP-128
        0x01, 0x00,             // AKM Count: 1
        0x00, 0x0F, 0xAC, 0x02, // AKM: PSK
        0x00, 0x00,             // RSN Capabilities
    ]);

    frame
}

// ── 802.11 Beacon / Probe Response parsing ────────────────────────────────────
//
// Management frame body for Beacon/Probe Response:
//   Offset  Len  Field
//   ------  ---  -----
//    24      8   Timestamp
//    32      2   Beacon Interval
//    34      2   Capability Information
//    36+     ?   Tagged parameters (TLV: tag, length, value)
//
// Key tagged parameters:
//   Tag 0  = SSID
//   Tag 1  = Supported Rates
//   Tag 3  = DS Parameter Set (channel)
//   Tag 48 = RSN Information Element (WPA2)

/// Parse a Beacon or Probe Response management frame, extract BSS info,
/// and add/update the SCAN_RESULTS list.
fn parse_beacon_or_probe(frame: &[u8]) {
    // Minimum: 24-byte 802.11 header + 12 bytes fixed fields = 36
    if frame.len() < 36 {
        return;
    }

    // Extract BSSID from Address 3 (bytes 16..22)
    let mut bssid = [0u8; 6];
    bssid.copy_from_slice(&frame[16..22]);

    // Tagged parameters start at offset 36 (after timestamp+interval+capability)
    let mut ssid = [0u8; 32];
    let mut ssid_len = 0usize;
    let mut channel = 1u8;
    let mut security = WifiSecurity::Open;

    let mut offset = 36;
    while offset + 1 < frame.len() {
        let tag_id = frame[offset];
        let tag_len = frame[offset + 1] as usize;
        if offset + 2 + tag_len > frame.len() {
            break;
        }
        let tag_data = &frame[offset + 2..offset + 2 + tag_len];

        match tag_id {
            // SSID
            0 => {
                let copy_len = tag_len.min(32);
                ssid[..copy_len].copy_from_slice(&tag_data[..copy_len]);
                ssid_len = copy_len;
            }
            // DS Parameter Set (channel)
            3 if tag_len >= 1 => {
                channel = tag_data[0];
            }
            // RSN IE — presence means WPA2
            48 => {
                security = WifiSecurity::Wpa2Personal;
            }
            _ => {}
        }

        offset += 2 + tag_len;
    }

    if ssid_len == 0 {
        // Hidden SSID — still record it but with empty name
    }

    // Update or add to scan results
    let mut results = SCAN_RESULTS.lock();
    if let Some(existing) = results.iter_mut().find(|b| b.bssid == bssid) {
        existing.channel = channel;
        existing.security = security;
        // Update SSID if we got a non-empty one
        if ssid_len > 0 {
            existing.ssid = ssid;
            existing.ssid_len = ssid_len;
        }
    } else {
        results.push(BssInfo {
            bssid,
            ssid,
            ssid_len,
            channel,
            rssi: -70, // Default; actual RSSI would come from PHY layer
            security,
        });
    }
}

// ── EAPOL frame building ──────────────────────────────────────────────────────

/// Build EAPOL Key frame 2 (STA→AP): SNonce + MIC.
/// This is the STA's response to Message 1 (AP's ANonce).
fn build_eapol_key_frame2(
    replay_counter: u64,
    snonce: &[u8; 32],
    kck: &[u8; 16],
    our_mac: &[u8; 6],
    ap_mac: &[u8; 6],
) -> Vec<u8> {
    // Ethernet header (14 bytes) + 802.1X header (4 bytes) + EAPOL Key body (95 bytes min)
    // Key info for message 2: Pairwise(bit3) | MIC(bit8) = 0x0108
    let key_info: u16 = (1 << 3) | (1 << 8);

    let mut frame = Vec::with_capacity(14 + 4 + 99);

    // Ethernet header (for the encapsulating Ethernet frame)
    frame.extend_from_slice(ap_mac);    // Destination
    frame.extend_from_slice(our_mac);   // Source
    frame.push(0x88); frame.push(0x8E); // EtherType: 802.1X EAPOL

    // 802.1X header
    frame.push(0x02);  // Version
    frame.push(0x03);  // Type: EAPOL-Key
    // Length of the EAPOL Key body (95 bytes: 4 + 91)
    let body_len: u16 = 95;
    frame.extend_from_slice(&body_len.to_be_bytes());

    // EAPOL Key body (95 bytes)
    frame.push(0x02); // Key Descriptor Type: RSN
    frame.extend_from_slice(&key_info.to_be_bytes());
    frame.push(0x00); frame.push(0x10); // Key Length: 16 (CCMP)
    frame.extend_from_slice(&replay_counter.to_be_bytes());
    frame.extend_from_slice(snonce);    // 32-byte SNonce
    // Key IV (16 bytes, zeroed)
    for _ in 0..16 { frame.push(0x00); }
    // Key RSC (8 bytes, zeroed)
    for _ in 0..8 { frame.push(0x00); }
    // Reserved (8 bytes, zeroed)
    for _ in 0..8 { frame.push(0x00); }
    // MIC placeholder (16 bytes, zeroed for now)
    let mic_offset = frame.len();
    for _ in 0..16 { frame.push(0x00); }
    // Key Data Length (0 for message 2)
    frame.push(0x00); frame.push(0x00);

    // Compute MIC over the entire EAPOL frame starting at 802.1X header
    // (i.e., from offset 14 where the 802.1X header begins)
    let mic = compute_mic(kck, &frame[14..]);
    frame[mic_offset..mic_offset + 16].copy_from_slice(&mic);

    frame
}

/// Build EAPOL Key frame 4 (STA→AP): final ACK, no key data.
/// Key info: Pairwise(bit3) | MIC(bit8) | Secure(bit9) = 0x0308
fn build_eapol_key_frame4(
    replay_counter: u64,
    kck: &[u8; 16],
    our_mac: &[u8; 6],
    ap_mac: &[u8; 6],
) -> Vec<u8> {
    let key_info: u16 = (1 << 3) | (1 << 8) | (1 << 9);

    let mut frame = Vec::with_capacity(14 + 4 + 99);

    // Ethernet header
    frame.extend_from_slice(ap_mac);
    frame.extend_from_slice(our_mac);
    frame.push(0x88); frame.push(0x8E);

    // 802.1X header
    frame.push(0x02);
    frame.push(0x03);
    let body_len: u16 = 95;
    frame.extend_from_slice(&body_len.to_be_bytes());

    // EAPOL Key body
    frame.push(0x02); // Key Descriptor Type: RSN
    frame.extend_from_slice(&key_info.to_be_bytes());
    frame.push(0x00); frame.push(0x10); // Key Length: 16
    frame.extend_from_slice(&replay_counter.to_be_bytes());
    // Key Nonce: zeroed in message 4
    for _ in 0..32 { frame.push(0x00); }
    // Key IV (16 bytes)
    for _ in 0..16 { frame.push(0x00); }
    // Key RSC (8 bytes)
    for _ in 0..8 { frame.push(0x00); }
    // Reserved (8 bytes)
    for _ in 0..8 { frame.push(0x00); }
    // MIC placeholder
    let mic_offset = frame.len();
    for _ in 0..16 { frame.push(0x00); }
    // Key Data Length: 0
    frame.push(0x00); frame.push(0x00);

    // Compute and insert MIC
    let mic = compute_mic(kck, &frame[14..]);
    frame[mic_offset..mic_offset + 16].copy_from_slice(&mic);

    frame
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Initiate a passive/active scan. Sets state to Scanning.
pub fn start_scan() {
    let mut state = WIFI_STATE.lock();
    *state = WifiState::Scanning;
    SCAN_RESULTS.lock().clear();
    crate::serial_verbose_println!("[wifi] Scanning...");
}

/// Get a snapshot of current scan results.
pub fn get_scan_results() -> Vec<BssInfo> {
    SCAN_RESULTS.lock().clone()
}

/// Configure and initiate a connection to a WPA2 Personal network.
pub fn connect(ssid: &[u8], password: &[u8]) {
    let mut ssid_arr = [0u8; 32];
    let ssid_len = ssid.len().min(32);
    ssid_arr[..ssid_len].copy_from_slice(&ssid[..ssid_len]);

    let mut password_arr = [0u8; 64];
    let password_len = password.len().min(64);
    password_arr[..password_len].copy_from_slice(&password[..password_len]);

    let config = WifiConfig {
        ssid: ssid_arr,
        ssid_len,
        password: password_arr,
        password_len,
    };

    *WIFI_CONFIG.lock() = Some(config);

    // Generate SNonce now so it's ready for the handshake
    *SNONCE.lock() = generate_nonce();

    // Transition to scanning (driver will send probe requests / switch channels)
    let mut state = WIFI_STATE.lock();
    *state = WifiState::Scanning;

    crate::serial_verbose_println!("[wifi] Connecting to SSID (len={})", ssid_len);
}

/// Disconnect from the current network.
pub fn disconnect() {
    let mut state = WIFI_STATE.lock();
    *state = WifiState::Disconnected;
    *PTK.lock() = None;
    crate::serial_verbose_println!("[wifi] Disconnected");
}

/// Get the current WiFi state (cloned snapshot).
pub fn get_state() -> WifiState {
    WIFI_STATE.lock().clone()
}

/// Get a copy of the pending connect config (ssid bytes, ssid_len), if any.
/// Used by the driver to match beacons against the target network.
pub fn get_connect_ssid() -> Option<([u8; 32], usize)> {
    let guard = WIFI_CONFIG.lock();
    guard.as_ref().map(|c| (c.ssid, c.ssid_len))
}

/// Check whether the station is connected (handshake complete).
pub fn is_connected() -> bool {
    matches!(*WIFI_STATE.lock(), WifiState::Connected { .. })
}

/// Get the BSSID of the connected AP, if connected.
pub fn connected_bssid() -> Option<[u8; 6]> {
    match *WIFI_STATE.lock() {
        WifiState::Connected { bssid, .. } => Some(bssid),
        _ => None,
    }
}

/// Handle a received Beacon frame. Updates scan results.
pub fn handle_beacon(frame: &[u8]) {
    parse_beacon_or_probe(frame);

    // If we are scanning and have a target SSID, check if we found it
    let config_opt = WIFI_CONFIG.lock().clone();
    if let Some(config) = config_opt {
        let results = SCAN_RESULTS.lock();
        for bss in results.iter() {
            if bss.ssid_len == config.ssid_len
                && bss.ssid[..bss.ssid_len] == config.ssid[..config.ssid_len]
            {
                // Found our target — state will be updated when driver
                // calls handle_auth_response after sending auth request
                let state = WIFI_STATE.lock();
                if matches!(*state, WifiState::Scanning) {
                    crate::serial_verbose_println!(
                        "[wifi] Target SSID found on channel {}", bss.channel
                    );
                }
                break;
            }
        }
    }
}

/// Handle a received Probe Response frame. Same as beacon.
pub fn handle_probe_response(frame: &[u8]) {
    parse_beacon_or_probe(frame);
}

/// Handle an Authentication Response (frame from AP).
/// If successful, transitions to Associating and sends the Association Request.
pub fn handle_auth_response(frame: &[u8], tx_fn: &mut dyn FnMut(&[u8])) {
    // Minimum: 24-byte header + 6-byte auth body = 30 bytes
    if frame.len() < 30 {
        return;
    }

    // Auth algorithm (2 bytes at offset 24): must be 0 (Open System)
    let algo = u16::from_le_bytes([frame[24], frame[25]]);
    // Auth transaction seq (2 bytes at offset 26): must be 2 (response)
    let seq = u16::from_le_bytes([frame[26], frame[27]]);
    // Status code (2 bytes at offset 28): must be 0 (success)
    let status = u16::from_le_bytes([frame[28], frame[29]]);

    if algo != 0 || seq != 2 || status != 0 {
        crate::serial_verbose_println!(
            "[wifi] Auth response: algo={} seq={} status={} — failed",
            algo, seq, status
        );
        return;
    }

    crate::serial_verbose_println!("[wifi] Authentication successful");

    // Extract BSSID from address 2 (frame[10..16]) — that's the AP source
    let mut bssid = [0u8; 6];
    bssid.copy_from_slice(&frame[10..16]);

    // Look up the BSS info for this AP
    let (ssid, ssid_len, channel) = {
        let results = SCAN_RESULTS.lock();
        if let Some(bss) = results.iter().find(|b| b.bssid == bssid) {
            (bss.ssid, bss.ssid_len, bss.channel)
        } else {
            ([0u8; 32], 0, 1)
        }
    };

    // Transition to Associating
    {
        let mut state = WIFI_STATE.lock();
        *state = WifiState::Associating { bssid, ssid, ssid_len, channel };
    }

    // Get our own MAC from the WiFi driver
    let our_mac = crate::drivers::network::with_wifi(|d| d.get_mac()).unwrap_or([0u8; 6]);

    // Build and send Association Request
    let assoc_frame = build_assoc_request(&bssid, &our_mac, &ssid, ssid_len, 3);
    tx_fn(&assoc_frame);

    crate::serial_verbose_println!("[wifi] Association Request sent");
}

/// Handle an Association Response (frame from AP).
/// If successful, transitions to Authenticating (waiting for EAPOL).
pub fn handle_assoc_response(frame: &[u8], _tx_fn: &mut dyn FnMut(&[u8])) {
    // Minimum: 24-byte header + 6-byte assoc body = 30 bytes
    if frame.len() < 30 {
        return;
    }

    // Status code at offset 26 (after capability[2] + aid[2])
    // Actually: capability[24..26], status[26..28], AID[28..30]
    let status = u16::from_le_bytes([frame[26], frame[27]]);

    if status != 0 {
        crate::serial_verbose_println!("[wifi] Association response status={} — failed", status);
        return;
    }

    crate::serial_verbose_println!("[wifi] Association successful — waiting for EAPOL handshake");

    // Extract BSSID from Address 2 (AP source)
    let mut bssid = [0u8; 6];
    bssid.copy_from_slice(&frame[10..16]);

    let (ssid, ssid_len) = {
        let state = WIFI_STATE.lock();
        match &*state {
            WifiState::Associating { ssid, ssid_len, .. } => (*ssid, *ssid_len),
            _ => ([0u8; 32], 0),
        }
    };

    let mut state = WIFI_STATE.lock();
    *state = WifiState::Authenticating { bssid, ssid, ssid_len };
}

/// Handle an EAPOL frame and drive the WPA2 4-way handshake.
/// `frame` is the Ethernet frame starting from the Ethernet header.
pub fn handle_eapol(frame: &[u8], tx_fn: &mut dyn FnMut(&[u8])) {
    // Ethernet header: 14 bytes (dst[6] + src[6] + ethertype[2])
    // EAPOL header: 4 bytes (version[1] + type[1] + length[2])
    // EAPOL Key body starts at offset 18
    if frame.len() < 18 {
        return;
    }

    // Verify EtherType = 0x888E (EAPOL)
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    if ethertype != 0x888E {
        return;
    }

    // 802.1X type must be 3 (EAPOL-Key)
    if frame[15] != 3 {
        return;
    }

    // Parse EAPOL Key frame starting at offset 14 (802.1X header)
    let eapol_data = &frame[14..];
    let key = match parse_eapol_key(eapol_data) {
        Some(k) => k,
        None => {
            crate::serial_verbose_println!("[wifi] EAPOL: failed to parse key frame");
            return;
        }
    };

    let key_info = key.key_info;

    // Bit definitions
    let pairwise    = (key_info >> 3) & 1 != 0;
    let _install    = (key_info >> 6) & 1 != 0;
    let key_ack     = (key_info >> 7) & 1 != 0;
    let key_mic     = (key_info >> 8) & 1 != 0;
    let _secure     = (key_info >> 9) & 1 != 0;
    let _enc_kd     = (key_info >> 13) & 1 != 0;

    // Extract AP MAC from Ethernet source field (frame[6..12])
    let mut ap_mac = [0u8; 6];
    ap_mac.copy_from_slice(&frame[6..12]);

    let our_mac = crate::drivers::network::with_wifi(|d| d.get_mac()).unwrap_or([0u8; 6]);

    if pairwise && key_ack && !key_mic {
        // ── Message 1: AP → STA (ANonce) ─────────────────────────────────────
        crate::serial_verbose_println!("[wifi] EAPOL M1: received ANonce");

        // Store ANonce
        *ANONCE.lock() = key.nonce;

        // Retrieve the SNonce we generated earlier
        let snonce = *SNONCE.lock();

        // Derive PMK from passphrase + SSID
        let (passphrase, passphrase_len, ssid, ssid_len) = {
            let config_lock = WIFI_CONFIG.lock();
            match &*config_lock {
                Some(cfg) => (cfg.password, cfg.password_len, cfg.ssid, cfg.ssid_len),
                None => {
                    crate::serial_verbose_println!("[wifi] EAPOL M1: no config, ignoring");
                    return;
                }
            }
        };

        let pmk = pbkdf2_sha1(
            &passphrase[..passphrase_len],
            &ssid[..ssid_len],
        );

        // Derive PTK
        let ptk = derive_ptk(&pmk, &key.nonce, &snonce, &ap_mac, &our_mac);
        crate::serial_verbose_println!("[wifi] EAPOL M1: PTK derived");

        *PTK.lock() = Some(ptk.clone());

        // Build and send Message 2
        let msg2 = build_eapol_key_frame2(
            key.replay_counter,
            &snonce,
            &ptk.kck,
            &our_mac,
            &ap_mac,
        );
        tx_fn(&msg2);
        crate::serial_verbose_println!("[wifi] EAPOL M2: SNonce sent");

    } else if pairwise && key_mic && key_ack {
        // ── Message 3: AP → STA (GTK + install) ──────────────────────────────
        crate::serial_verbose_println!("[wifi] EAPOL M3: received GTK (install key)");

        // Retrieve PTK to build Message 4
        let ptk = match PTK.lock().clone() {
            Some(p) => p,
            None => {
                crate::serial_verbose_println!("[wifi] EAPOL M3: no PTK, dropping");
                return;
            }
        };

        // Build and send Message 4 (acknowledgement)
        let msg4 = build_eapol_key_frame4(
            key.replay_counter,
            &ptk.kck,
            &our_mac,
            &ap_mac,
        );
        tx_fn(&msg4);
        crate::serial_verbose_println!("[wifi] EAPOL M4: ACK sent");

        // Transition to Connected — read state fields first, then look up channel
        let (bssid, ssid, ssid_len) = {
            let state = WIFI_STATE.lock();
            match &*state {
                WifiState::Authenticating { bssid, ssid, ssid_len } => {
                    (*bssid, *ssid, *ssid_len)
                }
                WifiState::Connected { bssid, ssid, ssid_len, .. } => {
                    (*bssid, *ssid, *ssid_len)
                }
                _ => {
                    crate::serial_verbose_println!("[wifi] EAPOL M3: unexpected state");
                    return;
                }
            }
        };
        // Look up channel from scan results (separate lock acquisition)
        let channel = SCAN_RESULTS.lock()
            .iter()
            .find(|b| b.bssid == bssid)
            .map(|b| b.channel)
            .unwrap_or(1);

        let mut state = WIFI_STATE.lock();
        *state = WifiState::Connected { bssid, ssid, ssid_len, channel };

        crate::serial_verbose_println!(
            "[wifi] WPA2 handshake complete — connected to {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
        );
    } else {
        crate::serial_verbose_println!(
            "[wifi] EAPOL: unhandled key_info={:#06x}", key_info
        );
    }
}
