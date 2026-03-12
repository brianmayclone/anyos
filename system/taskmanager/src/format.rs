pub use anyos_std::fmt::{fmt_u32, fmt_pct, fmt_mem_pages, fmt_bytes};

pub fn isqrt_ceil(n: usize) -> usize {
    if n <= 1 { return 1; }
    let mut x = 1;
    while x * x < n { x += 1; }
    x
}

pub fn trim_leading_spaces(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    &b[start..]
}

pub fn parse_u32_bytes(s: &[u8]) -> Option<u32> {
    if s.is_empty() { return None; }
    let mut val = 0u32;
    for &b in s {
        if b < b'0' || b > b'9' { return None; }
        val = val * 10 + (b - b'0') as u32;
    }
    Some(val)
}
