#![no_std]
#![no_main]

anyos_std::entry!(main);

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0xFF, // invalid
    }
}

fn base64_encode(input: &[u8]) {
    let mut i = 0;
    let mut col = 0;
    let mut line = [0u8; 76];

    while i + 3 <= input.len() {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        let b2 = input[i + 2] as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        line[col] = B64_CHARS[((triple >> 18) & 0x3F) as usize];
        line[col + 1] = B64_CHARS[((triple >> 12) & 0x3F) as usize];
        line[col + 2] = B64_CHARS[((triple >> 6) & 0x3F) as usize];
        line[col + 3] = B64_CHARS[(triple & 0x3F) as usize];
        col += 4;
        i += 3;

        if col >= 76 {
            if let Ok(s) = core::str::from_utf8(&line[..col]) {
                anyos_std::println!("{}", s);
            }
            col = 0;
        }
    }

    let rem = input.len() - i;
    if rem == 1 {
        let b0 = input[i] as u32;
        line[col] = B64_CHARS[((b0 >> 2) & 0x3F) as usize];
        line[col + 1] = B64_CHARS[((b0 << 4) & 0x3F) as usize];
        line[col + 2] = b'=';
        line[col + 3] = b'=';
        col += 4;
    } else if rem == 2 {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        line[col] = B64_CHARS[((b0 >> 2) & 0x3F) as usize];
        line[col + 1] = B64_CHARS[(((b0 << 4) | (b1 >> 4)) & 0x3F) as usize];
        line[col + 2] = B64_CHARS[((b1 << 2) & 0x3F) as usize];
        line[col + 3] = b'=';
        col += 4;
    }

    if col > 0 {
        if let Ok(s) = core::str::from_utf8(&line[..col]) {
            anyos_std::println!("{}", s);
        }
    }
}

fn base64_decode(input: &[u8]) {
    let mut out = anyos_std::Vec::new();
    let mut buf = [0u8; 4];
    let mut buf_len = 0;

    for &c in input {
        if c == b'\n' || c == b'\r' || c == b' ' { continue; }
        if c == b'=' { break; }
        let v = b64_val(c);
        if v == 0xFF { continue; }
        buf[buf_len] = v;
        buf_len += 1;
        if buf_len == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            buf_len = 0;
        }
    }

    // Handle remaining
    if buf_len == 2 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
    } else if buf_len == 3 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
        out.push((buf[1] << 4) | (buf[2] >> 2));
    }

    // Write as raw bytes
    for &b in &out {
        anyos_std::print!("{}", b as char);
    }
}

fn read_all(fd: u32) -> (anyos_std::Vec<u8>, usize) {
    let mut file_buf = anyos_std::vec![0u8; 64 * 1024];
    let mut total: usize = 0;
    let mut read_buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;
        if total + n > file_buf.len() { break; }
        file_buf[total..total + n].copy_from_slice(&read_buf[..n]);
        total += n;
    }
    (file_buf, total)
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let decode = args.has(b'd');

    let fd = if args.pos_count > 0 {
        let path = args.positional[0];
        let f = anyos_std::fs::open(path, 0);
        if f == u32::MAX {
            anyos_std::println!("base64: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    let (file_buf, total) = read_all(fd);
    if fd != 0 { anyos_std::fs::close(fd); }

    if decode {
        base64_decode(&file_buf[..total]);
    } else {
        base64_encode(&file_buf[..total]);
    }
}
