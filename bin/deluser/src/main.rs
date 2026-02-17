#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count == 0 {
        anyos_std::println!("Usage: deluser <username>");
        return;
    }

    let username = args.positional[0];

    // Look up uid from username via listusers
    let mut list_buf = [0u8; 1024];
    let len = anyos_std::users::listusers(&mut list_buf);
    if len == 0 || len == u32::MAX {
        anyos_std::println!("deluser: failed to list users");
        return;
    }

    let list = match core::str::from_utf8(&list_buf[..len as usize]) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("deluser: invalid user list data");
            return;
        }
    };

    // Parse "uid:username\n..." to find matching uid
    let mut found_uid: Option<u16> = None;
    for line in list.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let uid_str = &line[..colon];
            let name = &line[colon + 1..];
            if name == username {
                // Parse uid
                let mut uid: u16 = 0;
                let mut valid = true;
                for &b in uid_str.as_bytes() {
                    if b < b'0' || b > b'9' {
                        valid = false;
                        break;
                    }
                    uid = uid.wrapping_mul(10).wrapping_add((b - b'0') as u16);
                }
                if valid {
                    found_uid = Some(uid);
                }
                break;
            }
        }
    }

    match found_uid {
        Some(uid) => {
            let ret = anyos_std::users::deluser(uid);
            if ret == 0 {
                anyos_std::println!("User '{}' (uid={}) deleted.", username, uid);
            } else {
                anyos_std::println!("deluser: failed to delete user '{}'", username);
            }
        }
        None => {
            anyos_std::println!("deluser: user '{}' not found", username);
        }
    }
}
