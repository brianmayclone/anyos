#![no_std]
#![no_main]

anyos_std::entry!(main);

fn parse_u16(s: &str) -> Option<u16> {
    if s.is_empty() {
        return None;
    }
    let mut val: u16 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u16)?;
    }
    Some(val)
}

/// Find the next available GID >= 1000 by listing existing groups.
fn next_available_gid() -> u16 {
    let mut buf = [0u8; 2048];
    let len = anyos_std::users::listgroups(&mut buf);
    if len == u32::MAX || len == 0 {
        return 1000;
    }

    let data = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    let mut max_gid: u16 = 999;

    // Each line is "gid:groupname\n"
    for line in data.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            if let Some(gid) = parse_u16(&line[..colon]) {
                if gid > max_gid {
                    max_gid = gid;
                }
            }
        }
    }

    max_gid + 1
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 1 {
        anyos_std::println!("Usage: addgroup <name> [gid]");
        return;
    }

    let name = args.positional[0];

    let gid = if args.pos_count >= 2 {
        match parse_u16(args.positional[1]) {
            Some(g) => g,
            None => {
                anyos_std::println!("addgroup: invalid gid '{}'", args.positional[1]);
                return;
            }
        }
    } else {
        next_available_gid()
    };

    let ret = anyos_std::users::addgroup(name, gid);
    if ret == u32::MAX {
        anyos_std::println!("addgroup: failed to create group '{}'", name);
    } else {
        anyos_std::println!("addgroup: group '{}' created with gid {}", name, gid);
    }
}
