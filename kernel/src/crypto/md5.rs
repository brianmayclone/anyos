//! Pure Rust MD5 implementation (RFC 1321) for no_std.
//!
//! Used for password hashing in the user database.

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
    0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
    0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// Compute the MD5 hash of `input`, returning 16 raw bytes.
pub fn md5(input: &[u8]) -> [u8; 16] {
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // Pre-processing: pad message to 64-byte blocks
    let bit_len = (input.len() as u64).wrapping_mul(8);
    // message + 0x80 + zeros + 8-byte length
    let padded_len = ((input.len() + 1 + 8 + 63) / 64) * 64;
    // Use a stack buffer for small messages, heap for large
    let mut buf = [0u8; 512];
    let padded: &[u8] = if padded_len <= 512 {
        buf[..input.len()].copy_from_slice(input);
        buf[input.len()] = 0x80;
        // zeros are already there
        let len_off = padded_len - 8;
        buf[len_off..len_off + 8].copy_from_slice(&bit_len.to_le_bytes());
        &buf[..padded_len]
    } else {
        // For very large inputs, use alloc
        let mut v = alloc::vec![0u8; padded_len];
        v[..input.len()].copy_from_slice(input);
        v[input.len()] = 0x80;
        let len_off = padded_len - 8;
        v[len_off..len_off + 8].copy_from_slice(&bit_len.to_le_bytes());
        // Leak and process — only used for user database ops, not hot path
        let leaked = alloc::boxed::Box::leak(v.into_boxed_slice());
        leaked
    };

    // Process each 64-byte chunk
    for chunk in padded.chunks_exact(64) {
        let mut m = [0u32; 16];
        for j in 0..16 {
            m[j] = u32::from_le_bytes([
                chunk[j * 4],
                chunk[j * 4 + 1],
                chunk[j * 4 + 2],
                chunk[j * 4 + 3],
            ]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };

            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                (a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g]))
                    .rotate_left(S[i]),
            );
            a = temp;
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut result = [0u8; 16];
    result[0..4].copy_from_slice(&a0.to_le_bytes());
    result[4..8].copy_from_slice(&b0.to_le_bytes());
    result[8..12].copy_from_slice(&c0.to_le_bytes());
    result[12..16].copy_from_slice(&d0.to_le_bytes());
    result
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Compute MD5 and return as 32-byte hex string (lowercase ASCII).
pub fn md5_hex(input: &[u8]) -> [u8; 32] {
    let raw = md5(input);
    let mut hex = [0u8; 32];
    for (i, &byte) in raw.iter().enumerate() {
        hex[i * 2] = HEX_CHARS[(byte >> 4) as usize];
        hex[i * 2 + 1] = HEX_CHARS[(byte & 0x0f) as usize];
    }
    hex
}
