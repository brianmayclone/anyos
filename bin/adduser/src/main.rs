#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"pcd");

    if args.pos_count == 0 {
        anyos_std::println!("Usage: adduser <username> [-p password] [-c fullname] [-d homedir]");
        return;
    }

    let username = args.positional[0];
    let password = args.opt(b'p').unwrap_or("");
    let fullname = args.opt(b'c').unwrap_or(username);

    // Build default homedir if not specified
    let mut homedir_buf = [0u8; 128];
    let homedir = match args.opt(b'd') {
        Some(d) => d,
        None => {
            let prefix = b"/Users/";
            let uname = username.as_bytes();
            let total = prefix.len() + uname.len();
            if total < homedir_buf.len() {
                homedir_buf[..prefix.len()].copy_from_slice(prefix);
                homedir_buf[prefix.len()..total].copy_from_slice(uname);
                core::str::from_utf8(&homedir_buf[..total]).unwrap_or("/Users/unknown")
            } else {
                "/Users/unknown"
            }
        }
    };

    let ret = anyos_std::users::adduser(username, password, fullname, homedir);
    if ret == 0 {
        anyos_std::println!("User '{}' created.", username);
    } else {
        anyos_std::println!("adduser: failed to create user '{}'", username);
    }
}
