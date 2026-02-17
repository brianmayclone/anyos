#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    // su [username] [password]
    // Default username is "root", default password is ""
    let username = if args.pos_count > 0 {
        args.positional[0]
    } else {
        "root"
    };

    let password = if args.pos_count > 1 {
        args.positional[1]
    } else {
        ""
    };

    if anyos_std::process::authenticate(username, password) {
        anyos_std::println!("Authentication successful for '{}'.", username);
    } else {
        anyos_std::println!("su: authentication failed for '{}'", username);
    }
}
