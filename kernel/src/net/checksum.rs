/// Internet checksum (RFC 1071) — ones-complement sum of 16-bit words.

pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    // Sum 16-bit words
    while i + 1 < data.len() {
        let word = ((data[i] as u32) << 8) | (data[i + 1] as u32);
        sum += word;
        i += 2;
    }

    // Handle odd byte
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }

    // Fold 32-bit sum to 16 bits
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

/// Pseudo-header checksum for TCP/UDP
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
