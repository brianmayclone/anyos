//! Login flow: banner, authentication loop, and environment setup.
//!
//! # Flow
//!
//! 1. [`print_banner`] — clear screen, run `neofetch`, print hostname.
//! 2. [`login_loop`] — repeatedly prompt for username + password until
//!    authentication succeeds, then call [`setup_environment`].
//! 3. Return the authenticated username to the caller (the shell loop).

use anyos_std::{env, fs, process, sys};
use anyos_std::String;
use anyos_std::format;

use crate::io::{print, println, read_line, read_password};
use crate::runner::run_external;

// ─── Hostname ─────────────────────────────────────────────────────────────────

/// Return the system hostname, falling back to `"anyos"` on failure.
pub fn hostname() -> String {
    let mut buf = [0u8; 64];
    let n = sys::get_hostname(&mut buf) as usize;
    if n > 0 && n <= 64 {
        String::from(core::str::from_utf8(&buf[..n]).unwrap_or("anyos"))
    } else {
        String::from("anyos")
    }
}

// ─── Banner ───────────────────────────────────────────────────────────────────

/// Clear the screen, run `neofetch` to display system information, and print
/// the hostname as a login header.
pub fn print_banner() {
    sys::con_write("\x1b[2J\x1b[H");
    let mut pc = 0u32;
    run_external("/System/bin/neofetch", "neofetch", None, None, &mut pc);
    println("");
    let banner = format!("{} console login", hostname());
    println(&banner);
    println("");
}

// ─── Environment setup ────────────────────────────────────────────────────────

/// Populate the environment for the newly logged-in user.
///
/// Sets the standard variables (`USER`, `HOME`, `SHELL`, `PATH`, …) and then
/// overlays any key-value pairs found in `/System/env` and `~/.env`.
pub fn setup_environment(username: &str) {
    let uid = process::getuid();

    // root lives at /, everyone else at /Users/<name>.
    let home = if uid == 0 {
        String::from("/")
    } else {
        format!("/Users/{}", username)
    };

    env::set("USER",    username);
    env::set("LOGNAME", username);
    env::set("HOME",    &home);
    env::set("PWD",     &home);
    env::set("SHELL",   "/System/bin/sh");
    env::set("PATH",    "/System/bin:/System/sbin:/bin:/usr/bin");
    env::set("TERM",    "ansi");
    env::set("UID",     &format!("{}", uid));

    let (cols, rows) = sys::con_get_size();
    if cols > 0 && rows > 0 {
        env::set("COLUMNS", &format!("{}", cols));
        env::set("LINES",   &format!("{}", rows));
    }

    // Overlay with values from /System/env, then ~/.env (optional).
    for path in ["/System/env", &format!("{}/.env", home)] {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.split('\n') {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }
                let line = line.strip_prefix("export ").unwrap_or(line);
                if let Some(eq) = line.find('=') {
                    let key = &line[..eq];
                    let val = line[eq + 1..].trim_matches('"').trim_matches('\'');
                    // Never let config files overwrite identity-critical variables
                    // that were set from the authenticated session above.
                    if !key.is_empty()
                        && key != "USER" && key != "LOGNAME" && key != "HOME"
                        && key != "PWD"  && key != "UID"
                    {
                        env::set(key, val);
                    }
                }
            }
        }
    }
}

// ─── Login loop ───────────────────────────────────────────────────────────────

/// Block until a valid username / password pair is entered.
///
/// On success, the kernel has already applied the new UID/GID (via
/// `process::authenticate`).  This function additionally calls
/// `process::set_identity` for non-root users so that all threads in this
/// process inherit the correct identity, then calls [`setup_environment`] and
/// returns the username.
pub fn login_loop() -> String {
    loop {
        print("login: ");
        let username = read_line();
        if username.is_empty() { continue; }

        print("Password: ");
        let password = read_password();

        if process::authenticate(&username, &password) {
            let uid = process::getuid();
            if uid != 0 {
                process::set_identity(uid);
            }
            println("");
            let welcome = format!("Welcome, {}!", username);
            println(&welcome);
            println("");
            setup_environment(&username);
            return username;
        } else {
            println("");
            println("Login incorrect");
            println("");
        }
    }
}
