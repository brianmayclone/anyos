#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Read a line from stdin (fd 0) by polling the pipe.
/// Returns the number of bytes read (excluding newline).
/// If `echo` is false, typed characters are not printed (password mode).
fn read_line(buf: &mut [u8], echo: bool) -> usize {
    let mut pos = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let n = anyos_std::fs::read(0, &mut byte);
        if n == 0 {
            // No data yet — yield and try again
            anyos_std::process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            break; // stdin closed or error
        }
        match byte[0] {
            b'\n' | b'\r' => {
                anyos_std::print!("\n");
                break;
            }
            8 | 127 => {
                // Backspace / DEL
                if pos > 0 {
                    pos -= 1;
                    if echo {
                        anyos_std::print!("\x08 \x08"); // erase char
                    } else {
                        anyos_std::print!("\x08 \x08"); // erase the '*'
                    }
                }
            }
            c if c >= b' ' => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    if echo {
                        anyos_std::print!("{}", c as char);
                    } else {
                        anyos_std::print!("*");
                    }
                }
            }
            _ => {}
        }
    }
    pos
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"po");

    let caller_uid = anyos_std::process::getuid();
    let is_root = caller_uid == 0;

    // Determine target username
    let username = if args.pos_count > 0 {
        args.positional[0]
    } else {
        // Default: change own password — get current username
        let mut name_buf = [0u8; 32];
        let nlen = anyos_std::process::getusername(caller_uid, &mut name_buf);
        if nlen == u32::MAX || nlen == 0 {
            anyos_std::println!("passwd: cannot determine current user");
            return;
        }
        // Copy to a static buffer since we need it beyond this scope
        unsafe {
            static mut OWN_NAME: [u8; 32] = [0u8; 32];
            let len = nlen as usize;
            OWN_NAME[..len].copy_from_slice(&name_buf[..len]);
            core::str::from_utf8_unchecked(&OWN_NAME[..len])
        }
    };

    // Check if -p flag was provided (CLI mode)
    if let Some(new_password) = args.opt(b'p') {
        // CLI mode: passwd <user> -p <new> [-o <old>]
        if new_password.is_empty() {
            anyos_std::println!("passwd: password cannot be empty");
            return;
        }
        let old_password = args.opt(b'o').unwrap_or("");
        if !is_root && old_password.is_empty() {
            anyos_std::println!("passwd: non-root users must provide -o <old_password>");
            return;
        }
        let ret = anyos_std::users::chpasswd(username, old_password, new_password);
        if ret == 0 {
            anyos_std::println!("passwd: password updated for '{}'", username);
        } else if !is_root {
            anyos_std::println!("passwd: authentication failed or permission denied");
        } else {
            anyos_std::println!("passwd: failed to update password for '{}'", username);
        }
        return;
    }

    // Interactive mode: prompt for passwords via stdin
    anyos_std::println!("Changing password for '{}'", username);

    // Non-root must authenticate with old password
    let mut old_pw_buf = [0u8; 64];
    let old_pw_len;
    if !is_root {
        anyos_std::print!("Current password: ");
        old_pw_len = read_line(&mut old_pw_buf, true);
        if old_pw_len == 0 {
            anyos_std::println!("passwd: aborted");
            return;
        }
    } else {
        old_pw_len = 0;
    }

    // Read new password
    anyos_std::print!("New password: ");
    let mut new_pw_buf = [0u8; 64];
    let new_pw_len = read_line(&mut new_pw_buf, true);
    if new_pw_len == 0 {
        anyos_std::println!("passwd: password cannot be empty");
        return;
    }

    // Confirm new password
    anyos_std::print!("Confirm new password: ");
    let mut confirm_buf = [0u8; 64];
    let confirm_len = read_line(&mut confirm_buf, true);

    if new_pw_len != confirm_len || new_pw_buf[..new_pw_len] != confirm_buf[..confirm_len] {
        anyos_std::println!("passwd: passwords do not match");
        return;
    }

    let old_pw = core::str::from_utf8(&old_pw_buf[..old_pw_len]).unwrap_or("");
    let new_pw = core::str::from_utf8(&new_pw_buf[..new_pw_len]).unwrap_or("");

    let ret = anyos_std::users::chpasswd(username, old_pw, new_pw);
    if ret == 0 {
        anyos_std::println!("passwd: password updated for '{}'", username);
    } else if !is_root {
        anyos_std::println!("passwd: authentication failed or permission denied");
    } else {
        anyos_std::println!("passwd: failed to update password for '{}'", username);
    }
}
