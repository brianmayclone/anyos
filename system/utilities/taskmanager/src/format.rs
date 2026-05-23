pub use anyos_std::fmt::{fmt_bytes, fmt_mem_pages, fmt_pct, fmt_u32};

pub fn trim_leading_spaces(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    &b[start..]
}
