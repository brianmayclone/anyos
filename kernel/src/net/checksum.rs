//! Internet checksum (RFC 1071) -- ones-complement sum of 16-bit words.
//! Used by IP, ICMP, TCP, and UDP headers.

/// Compute the Internet checksum over a byte slice.
///
/// Returns the ones-complement of the ones-complement sum of all 16-bit words.
/// Uses 64-bit accumulator with 8-byte inner loop for 4x throughput vs naive 2-byte.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u64 = 0;
    let mut i = 0;
    let len = data.len();

    // Fast path: sum 8 bytes (four 16-bit words) per iteration using u64 accumulator.
    // Deferred folding: u64 can accumulate ~2^48 bytes without overflow.
    while i + 7 < len {
        // Read 4 big-endian u16 words as two u32 reads
        let w0 = ((data[i] as u64) << 8) | (data[i + 1] as u64);
        let w1 = ((data[i + 2] as u64) << 8) | (data[i + 3] as u64);
        let w2 = ((data[i + 4] as u64) << 8) | (data[i + 5] as u64);
        let w3 = ((data[i + 6] as u64) << 8) | (data[i + 7] as u64);
        sum += w0 + w1 + w2 + w3;
        i += 8;
    }

    // Remaining 16-bit words
    while i + 1 < len {
        let word = ((data[i] as u64) << 8) | (data[i + 1] as u64);
        sum += word;
        i += 2;
    }

    // Handle odd byte
    if i < len {
        sum += (data[i] as u64) << 8;
    }

    // Fold 64-bit sum to 16 bits
    sum = (sum & 0xFFFF) + (sum >> 16);
    sum = (sum & 0xFFFF) + (sum >> 16);
    sum = (sum & 0xFFFF) + (sum >> 16);

    !(sum as u16)
}

/// Compute the pseudo-header partial sum for TCP/UDP checksum calculation (IPv4).
///
/// The caller must add the segment data to this sum and fold to 16 bits.
pub fn pseudo_header_checksum(src: &[u8; 4], dst: &[u8; 4], protocol: u8, length: u16) -> u32 {
    let mut sum: u32 = 0;
    sum += ((src[0] as u32) << 8) | (src[1] as u32);
    sum += ((src[2] as u32) << 8) | (src[3] as u32);
    sum += ((dst[0] as u32) << 8) | (dst[1] as u32);
    sum += ((dst[2] as u32) << 8) | (dst[3] as u32);
    sum += protocol as u32;
    sum += length as u32;
    sum
}

/// Compute the pseudo-header partial sum for IPv6 TCP/UDP/ICMPv6 checksum (RFC 8200 §8.1).
///
/// IPv6 pseudo-header:
///   Source Address (16 bytes)
///   Destination Address (16 bytes)
///   Upper-Layer Packet Length (4 bytes, u32)
///   zero (3 bytes) + Next Header (1 byte)
pub fn pseudo_header_checksum_v6(
    src: &[u8; 16],
    dst: &[u8; 16],
    next_header: u8,
    length: u32,
) -> u32 {
    let mut sum: u32 = 0;
    // Source address (8 x u16 words)
    for i in (0..16).step_by(2) {
        sum += ((src[i] as u32) << 8) | (src[i + 1] as u32);
    }
    // Destination address (8 x u16 words)
    for i in (0..16).step_by(2) {
        sum += ((dst[i] as u32) << 8) | (dst[i + 1] as u32);
    }
    // Upper-layer packet length (32-bit, 2 x u16 words)
    sum += (length >> 16) & 0xFFFF;
    sum += length & 0xFFFF;
    // Next header (zero-padded to 16-bit word)
    sum += next_header as u32;
    sum
}
