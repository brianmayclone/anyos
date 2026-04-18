//! SNTP protocol implementation (RFC 4330).
//!
//! NTP uses UDP port 123 with 48-byte packets.
//! We implement SNTPv4 (client mode) which is sufficient for clock sync.

use anyos_std::{net, sys};

pub const NTP_PORT: u16 = 123;
/// Local port used by the daemon for sending/receiving NTP packets.
pub const LOCAL_PORT: u16 = 1123;
pub const NTP_PACKET_SIZE: usize = 48;

/// Seconds between 1900-01-01 and 1970-01-01 (NTP epoch offset).
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

/// NTP timestamp: seconds since 1900-01-01 + fractional part.
#[derive(Clone, Copy, Default)]
pub struct NtpTimestamp {
    pub seconds: u32,
    pub fraction: u32,
}

impl NtpTimestamp {
    pub fn is_zero(&self) -> bool {
        self.seconds == 0 && self.fraction == 0
    }
}

/// Parsed NTP response.
pub struct NtpResponse {
    pub stratum: u8,
    pub precision: i8,
    pub root_delay: u32,
    pub root_dispersion: u32,
    pub reference_id: [u8; 4],
    pub reference_ts: NtpTimestamp,
    pub originate_ts: NtpTimestamp,
    pub receive_ts: NtpTimestamp,
    pub transmit_ts: NtpTimestamp,
}

/// Build an SNTP client request packet.
pub fn build_request(buf: &mut [u8; NTP_PACKET_SIZE]) {
    // Zero the packet.
    for b in buf.iter_mut() { *b = 0; }
    // LI=0, VN=4, Mode=3 (client) → byte 0 = 0b00_100_011 = 0x23
    buf[0] = 0x23;
    // Set transmit timestamp from system clock for origin matching.
    let ts = get_system_ntp_time();
    buf[40] = (ts.seconds >> 24) as u8;
    buf[41] = (ts.seconds >> 16) as u8;
    buf[42] = (ts.seconds >> 8) as u8;
    buf[43] = ts.seconds as u8;
    buf[44] = (ts.fraction >> 24) as u8;
    buf[45] = (ts.fraction >> 16) as u8;
    buf[46] = (ts.fraction >> 8) as u8;
    buf[47] = ts.fraction as u8;
}

/// Parse an NTP response packet.
pub fn parse_response(buf: &[u8; NTP_PACKET_SIZE]) -> Option<NtpResponse> {
    let li_vn_mode = buf[0];
    let mode = li_vn_mode & 0x07;
    // Mode 4 = server, Mode 5 = broadcast
    if mode != 4 && mode != 5 {
        return None;
    }
    let stratum = buf[1];
    if stratum == 0 {
        return None; // Kiss-of-Death
    }

    Some(NtpResponse {
        stratum,
        precision: buf[3] as i8,
        root_delay: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        root_dispersion: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        reference_id: [buf[12], buf[13], buf[14], buf[15]],
        reference_ts: read_ts(&buf[16..24]),
        originate_ts: read_ts(&buf[24..32]),
        receive_ts: read_ts(&buf[32..40]),
        transmit_ts: read_ts(&buf[40..48]),
    })
}

/// Calculate clock offset in milliseconds from NTP response.
/// offset = ((T2 - T1) + (T3 - T4)) / 2
/// where T1 = originate (our send time), T2 = receive (server recv),
///       T3 = transmit (server send), T4 = our receive time.
///
/// Returns offset in milliseconds (positive = our clock is behind).
pub fn calculate_offset(resp: &NtpResponse, t4: NtpTimestamp) -> i64 {
    let t1 = ts_to_ms(resp.originate_ts);
    let t2 = ts_to_ms(resp.receive_ts);
    let t3 = ts_to_ms(resp.transmit_ts);
    let t4_ms = ts_to_ms(t4);

    // offset = ((T2 - T1) + (T3 - T4)) / 2
    let a = t2.wrapping_sub(t1) as i64;
    let b = t3.wrapping_sub(t4_ms) as i64;
    (a + b) / 2
}

/// Calculate round-trip delay in milliseconds.
/// delay = (T4 - T1) - (T3 - T2)
pub fn calculate_delay(resp: &NtpResponse, t4: NtpTimestamp) -> i64 {
    let t1 = ts_to_ms(resp.originate_ts);
    let t2 = ts_to_ms(resp.receive_ts);
    let t3 = ts_to_ms(resp.transmit_ts);
    let t4_ms = ts_to_ms(t4);

    let total = t4_ms.wrapping_sub(t1) as i64;
    let server = t3.wrapping_sub(t2) as i64;
    total - server
}

/// Compute the corrected time as a syscall buffer [year_lo, year_hi, month, day, hour, min, sec, 0].
/// Takes the current system time and applies offset_ms correction.
pub fn corrected_time_buf(offset_ms: i64) -> [u8; 8] {
    // Get current system time as Unix seconds.
    let mut time_buf = [0u8; 8];
    sys::time(&mut time_buf);
    let year = time_buf[0] as u32 | ((time_buf[1] as u32) << 8);
    let month = time_buf[2] as u32;
    let day = time_buf[3] as u32;
    let hour = time_buf[4] as u32;
    let min = time_buf[5] as u32;
    let sec = time_buf[6] as u32;

    let unix_secs = date_to_unix(year, month, day, hour, min, sec) as i64;
    // Apply offset (positive offset = our clock is behind, so add).
    let corrected = (unix_secs * 1000 + offset_ms) / 1000;
    let corrected = if corrected < 0 { 0u64 } else { corrected as u64 };

    let (y, mo, d, h, mi, s) = unix_to_date(corrected);
    let year_bytes = (y as u16).to_le_bytes();
    [year_bytes[0], year_bytes[1], mo as u8, d as u8, h as u8, mi as u8, s as u8, 0]
}

/// Convert Unix timestamp to (year, month, day, hour, min, sec).
fn unix_to_date(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec = (secs % 60) as u32; secs /= 60;
    let min = (secs % 60) as u32; secs /= 60;
    let hour = (secs % 24) as u32; secs /= 24;
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366u64 } else { 365 };
        if secs < days_in_year { break; }
        secs -= days_in_year;
        year += 1;
    }
    let month_days: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for m in 0..12 {
        let mut d = month_days[m] as u64;
        if m == 1 && is_leap(year) { d += 1; }
        if secs < d { break; }
        secs -= d;
        month += 1;
    }
    let day = secs as u32 + 1;
    (year, month, day, hour, min, sec)
}

/// Query a single NTP server. Returns (offset_ms, delay_ms, stratum) or None.
pub fn query_server(server_ip: &[u8; 4]) -> Option<(i64, i64, u8)> {
    let mut request = [0u8; NTP_PACKET_SIZE];
    build_request(&mut request);

    let sent = net::udp_sendto(server_ip, NTP_PORT, LOCAL_PORT, &request, 0);
    if sent == u32::MAX {
        return None;
    }

    // Wait for response (timeout is set on the socket).
    let mut recv_buf = [0u8; 8 + NTP_PACKET_SIZE + 16]; // header + payload + margin
    let n = net::udp_recvfrom(LOCAL_PORT, &mut recv_buf);
    let t4 = get_system_ntp_time();

    if n == 0 || n == u32::MAX || (n as usize) < 8 + NTP_PACKET_SIZE {
        return None;
    }

    // Parse received packet (skip 8-byte UDP header: src_ip:4, src_port:2, len:2).
    let mut pkt = [0u8; NTP_PACKET_SIZE];
    pkt.copy_from_slice(&recv_buf[8..8 + NTP_PACKET_SIZE]);

    let resp = parse_response(&pkt)?;
    if resp.transmit_ts.is_zero() {
        return None;
    }

    let offset = calculate_offset(&resp, t4);
    let delay = calculate_delay(&resp, t4);

    Some((offset, delay, resp.stratum))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_ts(buf: &[u8]) -> NtpTimestamp {
    NtpTimestamp {
        seconds: u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]),
        fraction: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
    }
}

fn ts_to_ms(ts: NtpTimestamp) -> u64 {
    let secs = ts.seconds as u64;
    // fraction is in units of 2^-32 seconds.
    let frac_ms = (ts.fraction as u64 * 1000) >> 32;
    secs * 1000 + frac_ms
}

/// Get current system time as NTP timestamp (seconds since 1900-01-01).
pub fn get_system_ntp_time() -> NtpTimestamp {
    let mut time_buf = [0u8; 8];
    sys::time(&mut time_buf);
    // time_buf: [year_lo, year_hi, month, day, hour, min, sec, 0]
    let year = time_buf[0] as u32 | ((time_buf[1] as u32) << 8);
    let month = time_buf[2] as u32;
    let day = time_buf[3] as u32;
    let hour = time_buf[4] as u32;
    let min = time_buf[5] as u32;
    let sec = time_buf[6] as u32;

    // Convert to Unix timestamp, then add NTP epoch offset.
    let unix_secs = date_to_unix(year, month, day, hour, min, sec);
    let ntp_secs = unix_secs + NTP_EPOCH_OFFSET;

    // Use uptime_ms fractional part for sub-second precision.
    let ms = sys::uptime_ms() % 1000;
    let fraction = ((ms as u64) << 32) / 1000;

    NtpTimestamp {
        seconds: ntp_secs as u32,
        fraction: fraction as u32,
    }
}

/// Convert date/time to Unix timestamp (seconds since 1970-01-01).
fn date_to_unix(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> u64 {
    // Days from 1970-01-01 to given date.
    let mut days: u64 = 0;
    // Add full years.
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    // Add full months in current year.
    let month_days: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        let d = month_days[(m - 1) as usize];
        days += d as u64;
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    // Add days in current month.
    days += (day as u64).saturating_sub(1);

    days * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
