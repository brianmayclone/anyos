#![no_std]
#![no_main]

anyos_std::entry!(main);

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

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

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: base64 FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("base64: cannot open '{}'", path);
        return;
    }

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
    anyos_std::fs::close(fd);

    base64_encode(&file_buf[..total]);
}
