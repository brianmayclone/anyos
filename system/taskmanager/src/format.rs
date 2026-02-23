pub fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 12]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

pub fn fmt_pct<'a>(buf: &'a mut [u8; 12], pct_x10: u32) -> &'a str {
    let whole = pct_x10 / 10;
    let frac = pct_x10 % 10;
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, whole);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'.'; p += 1;
    buf[p] = b'0' + frac as u8; p += 1;
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

pub fn fmt_mem_pages<'a>(buf: &'a mut [u8; 16], pages: u32) -> &'a str {
    let kib = pages * 4;
    let mut t = [0u8; 12];
    let mut p = 0;
    if kib >= 1024 {
        let mib = kib / 1024;
        let frac = (kib % 1024) * 10 / 1024;
        let s = fmt_u32(&mut t, mib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'.'; p += 1;
        buf[p] = b'0' + frac as u8; p += 1;
        buf[p] = b'M'; p += 1;
    } else {
        let s = fmt_u32(&mut t, kib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'K'; p += 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

pub fn fmt_bytes<'a>(buf: &'a mut [u8; 20], bytes: u64) -> &'a str {
    let mut t = [0u8; 12];
    let mut p = 0;
    if bytes >= 1024 * 1024 {
        let mib = bytes / (1024 * 1024);
        let frac = ((bytes % (1024 * 1024)) * 10 / (1024 * 1024)) as u32;
        let s = fmt_u32(&mut t, mib as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'.'; p += 1;
        buf[p] = b'0' + frac as u8; p += 1;
        buf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
    } else if bytes >= 1024 {
        let kib = bytes / 1024;
        let s = fmt_u32(&mut t, kib as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p..p + 4].copy_from_slice(b" KiB"); p += 4;
    } else {
        let s = fmt_u32(&mut t, bytes as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p..p + 2].copy_from_slice(b" B"); p += 2;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

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
