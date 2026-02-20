//! Number and size formatting utilities.

pub fn fmt_size<'a>(buf: &'a mut [u8; 16], size: u32) -> &'a str {
    if size >= 1024 * 1024 {
        let mb = size / (1024 * 1024);
        let frac = (size % (1024 * 1024)) / (1024 * 100);
        let mut p = 0;
        p = write_u32(buf, p, mb);
        buf[p] = b'.'; p += 1;
        p = write_u32(buf, p, frac.min(9));
        buf[p..p + 3].copy_from_slice(b" MB");
        p += 3;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    } else if size >= 1024 {
        let kb = size / 1024;
        let mut p = 0;
        p = write_u32(buf, p, kb);
        buf[p..p + 3].copy_from_slice(b" KB");
        p += 3;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    } else {
        let mut p = 0;
        p = write_u32(buf, p, size);
        buf[p..p + 2].copy_from_slice(b" B");
        p += 2;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    }
}

pub fn fmt_item_count<'a>(buf: &'a mut [u8; 16], count: usize) -> &'a str {
    let mut p = 0;
    p = write_u32(buf, p, count as u32);
    buf[p..p + 6].copy_from_slice(b" items");
    p += 6;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn write_u32(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        buf[pos] = b'0';
        return pos + 1;
    }
    let mut v = val;
    let mut tmp = [0u8; 10];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[pos + i] = tmp[n - 1 - i];
    }
    pos + n
}
