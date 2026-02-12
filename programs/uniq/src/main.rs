#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    let count_mode = args.contains("-c");
    let path = args.split_ascii_whitespace()
        .find(|s| !s.starts_with('-'))
        .unwrap_or("");

    if path.is_empty() {
        anyos_std::println!("Usage: uniq [-c] FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("uniq: cannot open '{}'", path);
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

    let text = core::str::from_utf8(&file_buf[..total]).unwrap_or("");
    let mut prev = "";
    let mut count: u32 = 0;

    for line in text.lines() {
        if line == prev {
            count += 1;
        } else {
            if count > 0 {
                if count_mode {
                    anyos_std::println!("{:>7} {}", count, prev);
                } else {
                    anyos_std::println!("{}", prev);
                }
            }
            prev = line;
            count = 1;
        }
    }
    if count > 0 {
        if count_mode {
            anyos_std::println!("{:>7} {}", count, prev);
        } else {
            anyos_std::println!("{}", prev);
        }
    }
}
