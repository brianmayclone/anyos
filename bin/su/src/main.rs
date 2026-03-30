#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{process, sys};
use anyos_std::format;

fn main() -> u32 {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("su - Switch user\n\nUsage: su [USERNAME]");
        return 0;
    }

    // su [username]  — default target is root
    let username = if args.pos_count > 0 {
        args.positional[0]
    } else {
        "root"
    };

    // Always prompt for the target user's password interactively.
    let prompt = format!("Password for {}: ", username);
    sys::con_write(&prompt);
    let mut pw_buf = [0u8; 128];
    let pw_len = sys::con_read_password(&mut pw_buf);
    sys::con_write("\n");
    let password = core::str::from_utf8(&pw_buf[..pw_len]).unwrap_or("");

    if !process::authenticate(username, password) {
        sys::con_write("su: authentication failed\n");
        return 1;
    }

    // Resolve the UID that authenticate() just set.
    let uid = process::getuid();

    // Apply the new identity to this process so all future syscalls run as
    // the target user.
    process::set_identity(uid);

    // Resolve the canonical username for the new UID (may differ in casing).
    let mut name_buf = [0u8; 32];
    let nlen = process::getusername(uid, &mut name_buf);
    let resolved = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize]).unwrap_or(username)
    } else {
        username
    };

    // Update environment variables so the shell (and its children) see the
    // correct identity.
    let home = if uid == 0 {
        anyos_std::String::from("/")
    } else {
        format!("/Users/{}", resolved)
    };

    anyos_std::env::set("USER",    resolved);
    anyos_std::env::set("LOGNAME", resolved);
    anyos_std::env::set("HOME",    &home);
    anyos_std::env::set("PWD",     &home);
    anyos_std::env::set("UID",     &format!("{}", uid));

    // Spawn a new textmode_console in shell-only mode as the new user.
    // It inherits the updated environment and the new UID, and skips
    // the banner and login prompt via the --shell flag.
    let console = "/System/bin/textmode_console";
    let tid = process::spawn(console, &format!("{} --shell", console));
    if tid == u32::MAX {
        sys::con_write("su: could not start shell\n");
        return 1;
    }

    // Wait for the shell to exit.
    loop {
        let exit = process::try_waitpid(tid);
        if exit != process::STILL_RUNNING { return 0; }
        process::sleep(10);
    }
}
