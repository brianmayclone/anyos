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

/// Look up a group name in the group list and return its GID.
fn find_gid_by_name(name: &str) -> Option<u16> {
    let mut buf = [0u8; 2048];
    let len = anyos_std::users::listgroups(&mut buf);
    if len == u32::MAX || len == 0 {
        return None;
    }

    let data = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");

    // Each line is "gid:groupname\n"
    for line in data.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let group_name = &line[colon + 1..];
            if group_name == name {
                return parse_u16(&line[..colon]);
            }
        }
    }

    None
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 1 {
        anyos_std::println!("Usage: delgroup <name>");
        return;
    }

    let name = args.positional[0];

    let gid = match find_gid_by_name(name) {
        Some(g) => g,
        None => {
            anyos_std::println!("delgroup: group '{}' not found", name);
            return;
        }
    };

    let ret = anyos_std::users::delgroup(gid);
    if ret == u32::MAX {
        anyos_std::println!("delgroup: failed to delete group '{}' (gid {})", name, gid);
    } else {
        anyos_std::println!("delgroup: group '{}' (gid {}) deleted", name, gid);
    }
}
