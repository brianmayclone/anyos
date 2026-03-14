//! anyOS Text-Mode Console
//!
//! Launched by the kernel when the `nogui` boot parameter is set.
//! Provides a classic login prompt followed by a command-line shell
//! entirely via the kernel framebuffer console (`SYS_CON_WRITE` /
//! `SYS_CON_READ`).  No compositor, no sessionhost, no GUI.
//!
//! Flow:
//!   1. Print banner + hostname
//!   2. Login loop  — prompt for username / password, authenticate
//!   3. Shell loop  — read commands, execute via process::spawn + waitpid
//!   4. On `logout` / `exit`, return to step 2

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{process, sys, fs, ipc, env};
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;

// Unique pipe name counter (simple monotone ID per session).
static mut PIPE_COUNTER: u32 = 0;

fn next_pipe_name() -> String {
    let id = unsafe { PIPE_COUNTER };
    unsafe { PIPE_COUNTER = PIPE_COUNTER.wrapping_add(1); }
    format!("tc_out_{}", id)
}

// ─── OS Version ──────────────────────────────────────────────────────────────

const VERSION: &str = env!("ANYOS_VERSION");

// ─── Helpers: console I/O ────────────────────────────────────────────────────

fn print(s: &str) {
    sys::con_write(s);
}

fn println(s: &str) {
    sys::con_write(s);
    sys::con_write("\n");
}

/// Read a line of visible input into a fresh String.
fn read_line() -> String {
    let mut buf = [0u8; 256];
    let n = sys::con_read_line(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("").trim_end();
    String::from(s)
}

/// Read a line of password input (masked with `*`).
fn read_password() -> String {
    let mut buf = [0u8; 128];
    let n = sys::con_read_password(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("");
    String::from(s)
}

// ─── Banner ──────────────────────────────────────────────────────────────────

fn print_banner() {
    println("\x1b[2J\x1b[H"); // VT100 clear screen — textcon ignores escape codes but harmless
    println("┌──────────────────────────────────────────┐");
    let ver_line = format!("│          .anyOS {}            │", VERSION);
    println(&ver_line);
    println("│       Text Console (nogui mode)          │");
    println("└──────────────────────────────────────────┘");
    println("");

    let mut hn_buf = [0u8; 64];
    let hn_len = sys::get_hostname(&mut hn_buf) as usize;
    let hostname = if hn_len > 0 && hn_len <= 64 {
        core::str::from_utf8(&hn_buf[..hn_len]).unwrap_or("anyos")
    } else {
        "anyos"
    };
    let banner = format!("{} console login", hostname);
    println(&banner);
    println("");
}

// ─── Environment setup ───────────────────────────────────────────────────────

/// Set up standard environment variables for the logged-in user session.
/// Called once after successful authentication.
fn setup_environment(username: &str) {
    let uid = process::getuid();

    // USER / LOGNAME
    env::set("USER", username);
    env::set("LOGNAME", username);

    // HOME
    let home = if uid == 0 {
        String::from("/root")
    } else {
        format!("/home/{}", username)
    };
    env::set("HOME", &home);

    // SHELL
    env::set("SHELL", "/System/bin/sh");

    // PATH
    env::set("PATH", "/System/bin:/System/sbin:/bin:/usr/bin");

    // TERM — signal that we are a basic text terminal
    env::set("TERM", "ansi");

    // COLUMNS / LINES — derived from framebuffer resolution ÷ font size
    let (cols, rows) = sys::con_get_size();
    if cols > 0 && rows > 0 {
        env::set("COLUMNS", &format!("{}", cols));
        env::set("LINES",   &format!("{}", rows));
    }

    // UID as string (some scripts check this)
    let uid_str = format!("{}", uid);
    env::set("UID", &uid_str);

    // Parse /System/etc/profile for additional env vars (KEY=VALUE lines).
    if let Ok(profile) = fs::read_to_string("/System/etc/profile") {
        for line in profile.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            // Strip optional "export " prefix
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some(eq) = line.find('=') {
                let key = &line[..eq];
                let val = line[eq + 1..].trim_matches('"').trim_matches('\'');
                if !key.is_empty() {
                    env::set(key, val);
                }
            }
        }
    }
}

// ─── Login loop ──────────────────────────────────────────────────────────────

/// Returns the username on successful authentication.
fn login_loop() -> String {
    loop {
        print("login: ");
        let username = read_line();
        if username.is_empty() { continue; }

        print("Password: ");
        let password = read_password();

        if process::authenticate(&username, &password) {
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

// ─── Shell loop ──────────────────────────────────────────────────────────────

/// Returns true if the user typed `logout` or `exit`.
fn shell_loop(username: &str) -> bool {
    let mut cwd = String::from("/");

    loop {
        // Build prompt:  username@hostname:/path$
        let mut hn_buf = [0u8; 64];
        let hn_len = sys::get_hostname(&mut hn_buf) as usize;
        let hostname = if hn_len > 0 && hn_len <= 64 {
            core::str::from_utf8(&hn_buf[..hn_len]).unwrap_or("anyos")
        } else {
            "anyos"
        };
        let prompt_suffix = if process::getuid() == 0 { "#" } else { "$" };
        let prompt = format!("{}@{}:{} {} ", username, hostname, cwd, prompt_suffix);
        print(&prompt);

        let line = read_line();
        if line.is_empty() { continue; }

        // Tokenise: split on whitespace, preserve args_str as the
        // remainder after the first token (preserves quoted strings etc.)
        let line_trimmed = line.trim_start();
        let tokens: Vec<&str> = line_trimmed.split_ascii_whitespace().collect();
        let cmd = tokens[0];
        // args_str = everything after the first whitespace-delimited token
        let args_str = match line_trimmed.find(|c: char| c.is_ascii_whitespace()) {
            Some(idx) => line_trimmed[idx..].trim_start(),
            None => "",
        };

        match cmd {
            "exit" | "logout" => {
                println("Logout.");
                return true;
            }
            "halt" | "shutdown" => {
                println("Shutting down...");
                process::shutdown(); // diverges
            }
            "reboot" => {
                println("Rebooting...");
                process::reboot(); // diverges
            }
            "clear" => {
                // Print 50 blank lines to simulate clear
                for _ in 0..50 { println(""); }
            }
            "pwd" => {
                println(&cwd);
            }
            "cd" => {
                let dest = if args_str.is_empty() {
                    // No argument — go to home directory or root
                    let uid = process::getuid();
                    if uid == 0 { "/root" } else { "/" }
                } else {
                    args_str
                };
                // Resolve relative paths
                let resolved = resolve_path(&cwd, dest);
                // Verify the directory exists (stat_buf[0] == 1 means directory)
                let mut stat_buf = [0u32; 7];
                if fs::stat(&resolved, &mut stat_buf) == 0 {
                    if stat_buf[0] == 1 {
                        cwd = resolved;
                    } else {
                        let msg = format!("cd: {}: Not a directory", dest);
                        println(&msg);
                    }
                } else {
                    let msg = format!("cd: {}: No such file or directory", dest);
                    println(&msg);
                }
            }
            "ls" => {
                let dir_s;
                let dir = if args_str.is_empty() {
                    cwd.as_str()
                } else if args_str.starts_with('/') {
                    args_str
                } else {
                    dir_s = resolve_path(&cwd, args_str);
                    dir_s.as_str()
                };
                list_dir(dir);
            }
            "cat" => {
                if args_str.is_empty() {
                    println("Usage: cat <file>");
                } else {
                    let path = if args_str.starts_with('/') {
                        String::from(args_str)
                    } else {
                        resolve_path(&cwd, args_str)
                    };
                    cat_file(path.as_str());
                }
            }
            "echo" => {
                println(args_str);
            }
            "whoami" => {
                println(username);
            }
            "uname" => {
                let msg = format!("anyOS {} x86_64", VERSION);
                println(&msg);
            }
            "dmesg" => {
                let mut buf = [0u8; 8192];
                let n = sys::dmesg(&mut buf) as usize;
                if n > 0 {
                    if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                        print(s);
                    }
                }
            }
            "help" => {
                println("Available built-in commands:");
                println("  cd [dir]     Change current directory");
                println("  ls [dir]     List directory contents");
                println("  cat <file>   Print file contents");
                println("  pwd          Print working directory");
                println("  echo [text]  Print text");
                println("  whoami       Print current username");
                println("  uname        Print system name");
                println("  dmesg        Print kernel log");
                println("  clear        Clear the screen");
                println("  halt         Halt the system");
                println("  reboot       Reboot the system");
                println("  exit/logout  Log out");
                println("  <path>       Run an executable (e.g. /System/bin/sh)");
            }
            _ => {
                // Try to execute as an external program
                let prog_path = if cmd.starts_with('/') {
                    String::from(cmd)
                } else if cmd.contains('/') {
                    resolve_path(&cwd, cmd)
                } else {
                    // Search /System/bin/
                    let candidate = format!("/System/bin/{}", cmd);
                    let mut _sb = [0u32; 7];
                    if fs::stat(&candidate, &mut _sb) == 0 {
                        candidate
                    } else {
                        String::from(cmd)
                    }
                };

                let full_args = if args_str.is_empty() {
                    String::from(cmd)
                } else {
                    format!("{} {}", cmd, args_str)
                };

                run_external(&prog_path, &full_args);
            }
        }
    }
}

// ─── External program runner ─────────────────────────────────────────────────

/// Spawn an external program, bridge its stdout to the text console, and wait.
///
/// stdout: named pipe → we read from it and feed to con_write.
/// stdin:  named pipe → we feed keyboard input to it (line-by-line).
fn run_external(path: &str, args: &str) {
    // Create a named stdout pipe for the child process.
    let pipe_name = next_pipe_name();
    let stdout_pipe = ipc::pipe_create(&pipe_name);
    if stdout_pipe == 0 || stdout_pipe == u32::MAX {
        let msg = format!("{}: failed to create output pipe", path);
        println(&msg);
        return;
    }

    // Spawn the child with stdout redirected to our pipe.
    let tid = process::spawn_piped(path, args, stdout_pipe);
    if tid == u32::MAX {
        ipc::pipe_close(stdout_pipe);
        let cmd = path.rsplit('/').next().unwrap_or(path);
        let msg = format!("{}: command not found", cmd);
        println(&msg);
        return;
    }

    // Pump output from the pipe to the console until the process exits.
    // Also polls keyboard for Ctrl+C to kill the child.
    let mut buf = [0u8; 512];
    'pump: loop {
        // Drain all available bytes from the pipe.
        loop {
            let n = ipc::pipe_read(stdout_pipe, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                sys::con_write(s);
            }
        }

        // Poll keyboard — Ctrl+C (0x03) kills the child.
        let key = sys::con_poll_key();
        if key == 0x03 {
            process::kill(tid);
            sys::con_write("\n^C\n");
            // Drain remaining output
            loop {
                let n = ipc::pipe_read(stdout_pipe, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    sys::con_write(s);
                }
            }
            break 'pump;
        }

        // Check if process has exited.
        let exit = process::try_waitpid(tid);
        if exit != process::STILL_RUNNING {
            // Drain any remaining output after exit.
            loop {
                let n = ipc::pipe_read(stdout_pipe, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    sys::con_write(s);
                }
            }
            break;
        }

        // Sleep briefly so we don't busy-spin.
        process::sleep(10);
    }

    ipc::pipe_close(stdout_pipe);
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

fn resolve_path(cwd: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        return normalize_path(rel);
    }
    let base = if cwd.ends_with('/') {
        format!("{}{}", cwd, rel)
    } else {
        format!("{}/{}", cwd, rel)
    };
    normalize_path(&base)
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut out = String::new();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    out
}

// ─── Built-in `ls` ───────────────────────────────────────────────────────────

fn list_dir(path: &str) {
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut names: Vec<String> = entries.map(|e| e.name.clone()).collect();
            names.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
            let mut line = String::new();
            for name in &names {
                if line.len() + name.len() + 2 > 78 {
                    println(&line);
                    line = String::new();
                }
                if !line.is_empty() { line.push_str("  "); }
                line.push_str(name.as_str());
            }
            if !line.is_empty() { println(&line); }
        }
        Err(_) => {
            let msg = format!("ls: {}: No such file or directory", path);
            println(&msg);
        }
    }
}

// ─── Built-in `cat` ──────────────────────────────────────────────────────────

fn cat_file(path: &str) {
    match fs::read_to_string(path) {
        Ok(s) => print(&s),
        Err(_) => {
            let msg = format!("cat: {}: No such file or directory", path);
            println(&msg);
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> u32 {
    print_banner();
    loop {
        let username = login_loop();
        shell_loop(&username);
    }
}
