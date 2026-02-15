#![no_std]
#![no_main]

anyos_std::entry!(main);

fn parse_i32(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let (neg, digits) = if s.as_bytes()[0] == b'-' {
        (true, &s[1..])
    } else {
        (false, s)
    };
    let mut val: i32 = 0;
    for &b in digits.as_bytes() {
        if b < b'0' || b > b'9' { return None; }
        val = val.wrapping_mul(10).wrapping_add((b - b'0') as i32);
    }
    Some(if neg { -val } else { val })
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    let parts: alloc::vec::Vec<&str> = args.split_ascii_whitespace().collect();

    let (first, incr, last) = match parts.len() {
        1 => {
            let l = parse_i32(parts[0]).unwrap_or(1);
            (1i32, 1i32, l)
        }
        2 => {
            let f = parse_i32(parts[0]).unwrap_or(1);
            let l = parse_i32(parts[1]).unwrap_or(1);
            (f, if f <= l { 1 } else { -1 }, l)
        }
        3 => {
            let f = parse_i32(parts[0]).unwrap_or(1);
            let i = parse_i32(parts[1]).unwrap_or(1);
            let l = parse_i32(parts[2]).unwrap_or(1);
            (f, i, l)
        }
        _ => {
            anyos_std::println!("Usage: seq [FIRST [INCREMENT]] LAST");
            return;
        }
    };

    if incr == 0 {
        anyos_std::println!("seq: increment must not be zero");
        return;
    }

    let mut val = first;
    if incr > 0 {
        while val <= last {
            anyos_std::println!("{}", val);
            val += incr;
        }
    } else {
        while val >= last {
            anyos_std::println!("{}", val);
            val += incr;
        }
    }
}
