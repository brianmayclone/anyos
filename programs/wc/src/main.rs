#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        anyos_std::println!("Usage: wc <file>");
        return;
    }

    let path = args.trim();
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("wc: cannot open '{}'", path);
        return;
    }

    let mut lines: u32 = 0;
    let mut words: u32 = 0;
    let mut bytes: u32 = 0;
    let mut in_word = false;
    let mut read_buf = [0u8; 512];

    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        for &b in &read_buf[..n as usize] {
            bytes += 1;
            if b == b'\n' { lines += 1; }
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                in_word = false;
            } else if !in_word {
                in_word = true;
                words += 1;
            }
        }
    }

    anyos_std::fs::close(fd);
    anyos_std::println!("  {} {} {} {}", lines, words, bytes, path);
}
