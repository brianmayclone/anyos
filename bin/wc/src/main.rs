#![no_std]
#![no_main]

anyos_std::entry!(main);

fn wc_fd(fd: u32, lines: &mut u32, words: &mut u32, bytes: &mut u32) {
    let mut in_word = false;
    let mut read_buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        for &b in &read_buf[..n as usize] {
            *bytes += 1;
            if b == b'\n' { *lines += 1; }
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                in_word = false;
            } else if !in_word {
                in_word = true;
                *words += 1;
            }
        }
    }
}

fn print_counts(l: u32, w: u32, b: u32, show_l: bool, show_w: bool, show_b: bool, name: &str) {
    if show_l { anyos_std::print!("{:>7} ", l); }
    if show_w { anyos_std::print!("{:>7} ", w); }
    if show_b { anyos_std::print!("{:>7} ", b); }
    anyos_std::println!("{}", name);
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let flag_l = args.has(b'l');
    let flag_w = args.has(b'w');
    let flag_c = args.has(b'c');
    // If no flags specified, show all three
    let (show_l, show_w, show_b) = if !flag_l && !flag_w && !flag_c {
        (true, true, true)
    } else {
        (flag_l, flag_w, flag_c)
    };

    if args.pos_count == 0 {
        // stdin
        let (mut l, mut w, mut b) = (0u32, 0u32, 0u32);
        wc_fd(0, &mut l, &mut w, &mut b);
        print_counts(l, w, b, show_l, show_w, show_b, "");
        return;
    }

    let mut total_l: u32 = 0;
    let mut total_w: u32 = 0;
    let mut total_b: u32 = 0;

    for i in 0..args.pos_count {
        let path = args.positional[i];
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            anyos_std::println!("wc: {}: No such file or directory", path);
            continue;
        }
        let (mut l, mut w, mut b) = (0u32, 0u32, 0u32);
        wc_fd(fd, &mut l, &mut w, &mut b);
        anyos_std::fs::close(fd);
        total_l += l;
        total_w += w;
        total_b += b;
        print_counts(l, w, b, show_l, show_w, show_b, path);
    }

    if args.pos_count > 1 {
        print_counts(total_l, total_w, total_b, show_l, show_w, show_b, "total");
    }
}
