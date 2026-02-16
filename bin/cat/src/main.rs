#![no_std]
#![no_main]

anyos_std::entry!(main);

fn cat_fd(fd: u32, number: bool, number_nonblank: bool, show_ends: bool, line_num: &mut u32) {
    let mut buf = [0u8; 512];
    let mut at_start = true;
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        for i in 0..n as usize {
            let b = buf[i];
            if at_start {
                if number_nonblank {
                    if b != b'\n' {
                        *line_num += 1;
                        anyos_std::print!("{:>6}\t", *line_num);
                    }
                } else if number {
                    *line_num += 1;
                    anyos_std::print!("{:>6}\t", *line_num);
                }
                at_start = false;
            }
            if b == b'\n' {
                if show_ends {
                    anyos_std::print!("$");
                }
                anyos_std::print!("\n");
                at_start = true;
            } else {
                anyos_std::print!("{}", b as char);
            }
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let number = args.has(b'n');
    let number_nonblank = args.has(b'b');
    let show_ends = args.has(b'E');

    let mut line_num: u32 = 0;

    if args.pos_count == 0 {
        // stdin
        cat_fd(0, number, number_nonblank, show_ends, &mut line_num);
        return;
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            anyos_std::println!("cat: {}: No such file or directory", path);
            continue;
        }
        cat_fd(fd, number, number_nonblank, show_ends, &mut line_num);
        anyos_std::fs::close(fd);
    }
}
