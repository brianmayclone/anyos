//! Poly1305 MAC (RFC 8439).
//!
//! One-time authenticator using a 256-bit key (r || s).

pub const TAG_SIZE: usize = 16;

/// Compute Poly1305 MAC.
///
/// `key` is 32 bytes: first 16 bytes are `r` (clamped), last 16 are `s`.
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; TAG_SIZE] {
    let r0 = (le_u32(&key[0..4]) as u64) & 0x3ffffff;
    let r1 = ((le_u32(&key[3..7]) >> 2) as u64) & 0x3ffff03;
    let r2 = ((le_u32(&key[6..10]) >> 4) as u64) & 0x3ffc0ff;
    let r3 = ((le_u32(&key[9..13]) >> 6) as u64) & 0x3f03fff;
    let r4 = ((le_u32(&key[12..16]) >> 8) as u64) & 0x00fffff;
    let rr1_5 = r1 * 5;
    let rr2_5 = r2 * 5;
    let rr3_5 = r3 * 5;
    let rr4_5 = r4 * 5;

    // Parse s
    let s0 = le_u32(&key[16..20]) as u64;
    let s1 = le_u32(&key[20..24]) as u64;
    let s2 = le_u32(&key[24..28]) as u64;
    let s3 = le_u32(&key[28..32]) as u64;

    let mut h0 = 0u64;
    let mut h1 = 0u64;
    let mut h2 = 0u64;
    let mut h3 = 0u64;
    let mut h4 = 0u64;

    // Process message in 16-byte blocks
    let mut offset = 0;
    while offset < msg.len() {
        let mut block = [0u8; 17];
        let chunk = (msg.len() - offset).min(16);
        block[..chunk].copy_from_slice(&msg[offset..offset + chunk]);
        block[chunk] = 1; // Padding bit

        let n0 = le_u32(&block[0..4]) as u64;
        let n1 = le_u32(&block[4..8]) as u64;
        let n2 = le_u32(&block[8..12]) as u64;
        let n3 = le_u32(&block[12..16]) as u64;

        h0 += n0 & 0x3ffffff;
        h1 += ((n0 >> 26) | (n1 << 6)) & 0x3ffffff;
        h2 += ((n1 >> 20) | (n2 << 12)) & 0x3ffffff;
        h3 += ((n2 >> 14) | (n3 << 18)) & 0x3ffffff;
        let hibit = if chunk == 16 { 1 << 24 } else { 0 };
        h4 += (n3 >> 8) | hibit;

        let mut d0 = h0 * r0 + h1 * rr4_5 + h2 * rr3_5 + h3 * rr2_5 + h4 * rr1_5;
        let mut d1 = h0 * r1 + h1 * r0 + h2 * rr4_5 + h3 * rr3_5 + h4 * rr2_5;
        let mut d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * rr4_5 + h4 * rr3_5;
        let mut d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * rr4_5;
        let mut d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        // Carry propagation
        let mut c: u64;
        c = d0 >> 26;
        d0 &= 0x3FFFFFF;
        d1 += c;
        c = d1 >> 26;
        d1 &= 0x3FFFFFF;
        d2 += c;
        c = d2 >> 26;
        d2 &= 0x3FFFFFF;
        d3 += c;
        c = d3 >> 26;
        d3 &= 0x3FFFFFF;
        d4 += c;
        c = d4 >> 26;
        d4 &= 0x3FFFFFF;
        d0 += c * 5;
        c = d0 >> 26;
        d0 &= 0x3FFFFFF;
        d1 += c;

        h0 = d0;
        h1 = d1;
        h2 = d2;
        h3 = d3;
        h4 = d4;

        offset += 16;
    }

    let mut c = h1 >> 26;
    h1 &= 0x3ffffff;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x3ffffff;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x3ffffff;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x3ffffff;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x3ffffff;
    h1 += c;

    let mut g0 = h0 + 5;
    c = g0 >> 26;
    g0 &= 0x3ffffff;
    let mut g1 = h1 + c;
    c = g1 >> 26;
    g1 &= 0x3ffffff;
    let mut g2 = h2 + c;
    c = g2 >> 26;
    g2 &= 0x3ffffff;
    let mut g3 = h3 + c;
    c = g3 >> 26;
    g3 &= 0x3ffffff;
    let g4 = (h4 + c).wrapping_sub(1 << 26);

    let mask = (g4 >> 63).wrapping_sub(1);
    let not_mask = !mask;
    h0 = (h0 & not_mask) | (g0 & mask);
    h1 = (h1 & not_mask) | (g1 & mask);
    h2 = (h2 & not_mask) | (g2 & mask);
    h3 = (h3 & not_mask) | (g3 & mask);
    h4 = (h4 & not_mask) | (g4 & mask);

    h0 = (h0 | (h1 << 26)) & 0xffff_ffff;
    h1 = ((h1 >> 6) | (h2 << 20)) & 0xffff_ffff;
    h2 = ((h2 >> 12) | (h3 << 14)) & 0xffff_ffff;
    h3 = ((h3 >> 18) | (h4 << 8)) & 0xffff_ffff;

    let f0 = h0 + s0;
    let f1 = h1 + s1 + (f0 >> 32);
    let f2 = h2 + s2 + (f1 >> 32);
    let f3 = h3 + s3 + (f2 >> 32);

    let mut tag = [0u8; 16];
    tag[0..4].copy_from_slice(&(f0 as u32).to_le_bytes());
    tag[4..8].copy_from_slice(&(f1 as u32).to_le_bytes());
    tag[8..12].copy_from_slice(&(f2 as u32).to_le_bytes());
    tag[12..16].copy_from_slice(&(f3 as u32).to_le_bytes());
    tag
}

fn le_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

#[cfg(test)]
mod tests {
    use super::poly1305;

    #[test]
    fn rfc8439_test_vector() {
        let key = [
            0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5,
            0x06, 0xa8, 0x01, 0x03, 0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf,
            0x41, 0x49, 0xf5, 0x1b,
        ];
        let msg = b"Cryptographic Forum Research Group";
        let expected = [
            0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01,
            0x27, 0xa9,
        ];

        assert_eq!(poly1305(&key, msg), expected);
    }
}
