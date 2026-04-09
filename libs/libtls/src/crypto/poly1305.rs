//! Poly1305 MAC (RFC 8439).
//!
//! One-time authenticator using a 256-bit key (r || s).

pub const TAG_SIZE: usize = 16;

/// Compute Poly1305 MAC.
///
/// `key` is 32 bytes: first 16 bytes are `r` (clamped), last 16 are `s`.
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; TAG_SIZE] {
    // Parse r and clamp
    let mut r = [0u32; 5];
    r[0] = (le_u32(&key[0..4])) & 0x0FFFFFFF;
    r[1] = (le_u32(&key[3..7]) >> 2) & 0x0FFFFFFC;
    r[2] = (le_u32(&key[6..10]) >> 4) & 0x0FFFFFFC;
    r[3] = (le_u32(&key[9..13]) >> 6) & 0x0FFFFFFC;
    r[4] = (le_u32(&key[12..16]) >> 8) & 0x0FFFFFFC;

    // Parse s
    let s0 = le_u32(&key[16..20]) as u64;
    let s1 = le_u32(&key[20..24]) as u64;
    let s2 = le_u32(&key[24..28]) as u64;
    let s3 = le_u32(&key[28..32]) as u64;

    let mut h = [0u32; 5]; // Accumulator

    // Process message in 16-byte blocks
    let mut offset = 0;
    while offset < msg.len() {
        let mut block = [0u8; 17];
        let chunk = (msg.len() - offset).min(16);
        block[..chunk].copy_from_slice(&msg[offset..offset + chunk]);
        block[chunk] = 1; // Padding bit

        // Add block to accumulator
        h[0] = h[0].wrapping_add(le_u32(&block[0..4]));
        h[1] = h[1].wrapping_add(le_u32(&block[3..7]) >> 2);

        // Full precision: use u64 intermediates
        // h = (h + block) * r mod 2^130-5
        let h0 = h[0] as u64;
        let h1 = h[1] as u64;
        let h2 = h[2] as u64;
        let h3 = h[3] as u64;
        let h4 = h[4] as u64;

        // Reset h for accumulation with full block parsing
        let n0 = le_u32(&block[0..4]) as u64;
        let n1 = le_u32(&block[4..8]) as u64;
        let n2 = le_u32(&block[8..12]) as u64;
        let n3 = le_u32(&block[12..16]) as u64;
        let hibit = if chunk == 16 { 1u64 } else { 0 };

        // Use 130-bit accumulator: h = h + n
        let mut acc0 = (h[0] as u64) + n0;
        let mut acc1 = (h[1] as u64) + n1 + (acc0 >> 32);
        let mut acc2 = (h[2] as u64) + n2 + (acc1 >> 32);
        let mut acc3 = (h[3] as u64) + n3 + (acc2 >> 32);
        let mut acc4 = (h[4] as u64) + hibit + (acc3 >> 32);
        acc0 &= 0xFFFFFFFF;
        acc1 &= 0xFFFFFFFF;
        acc2 &= 0xFFFFFFFF;
        acc3 &= 0xFFFFFFFF;

        // Multiply: h * r using 26-bit limbs for overflow safety
        // This is a simplified approach -- use 64-bit multiply with carries
        let r0 = r[0] as u64;
        let r1 = r[1] as u64;
        let r2 = r[2] as u64;
        let r3 = r[3] as u64;
        let r4 = r[4] as u64;

        // 5*r for modular reduction (2^130 ≡ 5 mod 2^130-5)
        let s1r = r1.wrapping_mul(5);
        let s2r = r2.wrapping_mul(5);
        let s3r = r3.wrapping_mul(5);
        let s4r = r4.wrapping_mul(5);

        // This is getting complex. Use a simpler 130-bit multiply approach.
        // For correctness, we'll use a direct bignum multiply.
        // Convert to 26-bit limbs:
        let a0 = acc0 & 0x3FFFFFF;
        let a1 = ((acc0 >> 26) | (acc1 << 6)) & 0x3FFFFFF;
        let a2 = ((acc1 >> 20) | (acc2 << 12)) & 0x3FFFFFF;
        let a3 = ((acc2 >> 14) | (acc3 << 18)) & 0x3FFFFFF;
        let a4 = (acc3 >> 8) | (acc4 << 24);

        // r in 26-bit limbs
        let rr0 = (r[0] as u64) & 0x3FFFFFF;
        let rr1 = (((r[0] >> 26) as u64) | ((r[1] as u64) << 6)) & 0x3FFFFFF;
        let rr2 = (((r[1] >> 20) as u64) | ((r[2] as u64) << 12)) & 0x3FFFFFF;
        let rr3 = (((r[2] >> 14) as u64) | ((r[3] as u64) << 18)) & 0x3FFFFFF;
        let rr4 = ((r[3] >> 8) as u64) | ((r[4] as u64) << 24);

        let s1v = rr1 * 5;
        let s2v = rr2 * 5;
        let s3v = rr3 * 5;
        let s4v = rr4 * 5;

        let mut d0 = a0*rr0 + a1*s4v + a2*s3v + a3*s2v + a4*s1v;
        let mut d1 = a0*rr1 + a1*rr0 + a2*s4v + a3*s3v + a4*s2v;
        let mut d2 = a0*rr2 + a1*rr1 + a2*rr0 + a3*s4v + a4*s3v;
        let mut d3 = a0*rr3 + a1*rr2 + a2*rr1 + a3*rr0 + a4*s4v;
        let mut d4 = a0*rr4 + a1*rr3 + a2*rr2 + a3*rr1 + a4*rr0;

        // Carry propagation
        let mut c: u64;
        c = d0 >> 26; d0 &= 0x3FFFFFF; d1 += c;
        c = d1 >> 26; d1 &= 0x3FFFFFF; d2 += c;
        c = d2 >> 26; d2 &= 0x3FFFFFF; d3 += c;
        c = d3 >> 26; d3 &= 0x3FFFFFF; d4 += c;
        c = d4 >> 26; d4 &= 0x3FFFFFF; d0 += c * 5;
        c = d0 >> 26; d0 &= 0x3FFFFFF; d1 += c;

        // Convert back to 32-bit limbs
        h[0] = (d0 | (d1 << 26)) as u32;
        h[1] = ((d1 >> 6) | (d2 << 20)) as u32;
        h[2] = ((d2 >> 12) | (d3 << 14)) as u32;
        h[3] = ((d3 >> 18) | (d4 << 8)) as u32;
        h[4] = (d4 >> 24) as u32;

        offset += 16;
    }

    // Final reduction mod 2^130-5
    // h = h mod 2^128 + s
    let mut f0 = h[0] as u64 + s0;
    let mut f1 = h[1] as u64 + s1 + (f0 >> 32);
    let mut f2 = h[2] as u64 + s2 + (f1 >> 32);
    let f3 = h[3] as u64 + s3 + (f2 >> 32);

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
