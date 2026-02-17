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

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 2 {
        anyos_std::println!("Usage: chown <uid>[:<gid>] <path>");
        anyos_std::println!("  If gid is omitted, it defaults to uid");
        return;
    }

    let owner_str = args.positional[0];
    let path = args.positional[1];

    // Parse uid:gid or just uid
    let (uid, gid) = if let Some(colon_pos) = owner_str.find(':') {
        let uid_str = &owner_str[..colon_pos];
        let gid_str = &owner_str[colon_pos + 1..];
        let uid = match parse_u16(uid_str) {
            Some(u) => u,
            None => {
                anyos_std::println!("chown: invalid uid '{}'", uid_str);
                return;
            }
        };
        let gid = match parse_u16(gid_str) {
            Some(g) => g,
            None => {
                anyos_std::println!("chown: invalid gid '{}'", gid_str);
                return;
            }
        };
        (uid, gid)
    } else {
        let uid = match parse_u16(owner_str) {
            Some(u) => u,
            None => {
                anyos_std::println!("chown: invalid uid '{}'", owner_str);
                return;
            }
        };
        (uid, uid)
    };

    let ret = anyos_std::fs::chown(path, uid, gid);
    if ret == u32::MAX {
        anyos_std::println!("chown: failed to change owner of '{}'", path);
    } else {
        anyos_std::println!("chown: owner of '{}' changed to {}:{}", path, uid, gid);
    }
}
