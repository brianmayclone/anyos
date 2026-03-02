#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::fs;

fn read_all(fd: u32) -> (anyos_std::Vec<u8>, usize) {
    let mut file_buf = anyos_std::vec![0u8; 64 * 1024];
    let mut total: usize = 0;
    let mut read_buf = [0u8; 512];
    loop {
        let n = fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;
        if total + n > file_buf.len() { break; }
        file_buf[total..total + n].copy_from_slice(&read_buf[..n]);
        total += n;
    }
    (file_buf, total)
}

fn parse_num(s: &str) -> u32 {
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            n = n * 10 + (b - b'0') as u32;
        }
    }
    n
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"nc");

    let max_lines = args.opt_u32(b'n', 10);
    let byte_mode = args.opt(b'c');
    let follow = args.has(b'f');

    // -f only works with a file, not stdin
    let path_str = if args.pos_count > 0 {
        args.positional[0]
    } else {
        ""
    };

    let fd = if !path_str.is_empty() {
        let f = fs::open(path_str, 0);
        if f == u32::MAX {
            anyos_std::println!("tail: cannot open '{}'", path_str);
            return;
        }
        f
    } else {
        0 // stdin
    };

    let (file_buf, total) = read_all(fd);

    let data = &file_buf[..total];

    if let Some(c_val) = byte_mode {
        let max_bytes = parse_num(c_val);
        let n = if max_bytes == 0 { 512 } else { max_bytes as usize };
        let start = if total > n { total - n } else { 0 };
        if let Ok(s) = core::str::from_utf8(&data[start..]) {
            anyos_std::print!("{}", s);
        }
        if !follow || fd == 0 {
            if fd != 0 { fs::close(fd); }
            return;
        }
    } else {
        // Line mode: print last N lines
        let mut line_count: u32 = 0;
        let mut start = 0;
        for i in (0..total).rev() {
            if data[i] == b'\n' {
                line_count += 1;
                if line_count >= max_lines + 1 {
                    start = i + 1;
                    break;
                }
            }
        }

        if let Ok(s) = core::str::from_utf8(&data[start..]) {
            anyos_std::print!("{}", s);
        }

        if !follow || fd == 0 {
            if fd != 0 { fs::close(fd); }
            return;
        }
    }

    // -f mode: close and reopen the file each poll cycle so we always
    // get the current inode content (handles log rotation too).
    // We remember the byte offset where we left off.
    let mut offset: u32 = total as u32;

    // Close the fd we used for initial read; we'll reopen each iteration.
    if fd != 0 { fs::close(fd); }

    let mut read_buf = [0u8; 4096];
    loop {
        anyos_std::process::sleep(250);

        let follow_fd = fs::open(path_str, 0);
        if follow_fd == u32::MAX {
            // File temporarily unavailable (rotation?), wait
            continue;
        }

        // Check current file size via seek to end
        let file_size = fs::lseek(follow_fd, 0, fs::SEEK_END);
        if file_size == u32::MAX {
            fs::close(follow_fd);
            continue;
        }

        if file_size < offset {
            // File was truncated or rotated — restart from beginning
            offset = 0;
        }

        if file_size > offset {
            // Seek to where we left off
            fs::lseek(follow_fd, offset as i32, fs::SEEK_SET);

            // Read and print all new data
            loop {
                let n = fs::read(follow_fd, &mut read_buf);
                if n == 0 || n == u32::MAX { break; }
                let n = n as usize;
                if let Ok(s) = core::str::from_utf8(&read_buf[..n]) {
                    anyos_std::print!("{}", s);
                }
                offset += n as u32;
            }
        }

        fs::close(follow_fd);
    }
}
