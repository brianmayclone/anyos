//! anyOS Text-Mode Console
//!
//! Launched by the kernel when the `nogui` boot parameter is set.
//! Provides a classic login prompt followed by a command-line shell
//! entirely via the kernel framebuffer console (`SYS_CON_WRITE` /
//! `SYS_CON_READ`).  No compositor, no sessionhost, no GUI.
//!
//! Shell features shared with Terminal.app via `anyos_std::shell`:
//!   - POSIX tokenisation (quoting, backslash escapes)
//!   - Variable expansion ($VAR, ${VAR}, $(cmd), backticks)
//!   - Tilde expansion
//!   - Glob expansion (*, ?, [...])
//!   - Output redirect (>, >>, 2>, 2>>, 1>, 1>>, 2>&1)
//!   - Input redirect (<)
//!   - Pipeline (cmd1 | cmd2 | cmd3)
//!
//! Flow:
//!   1. Print banner + hostname
//!   2. Login loop  — prompt for username / password, authenticate
//!   3. Shell loop  — read commands, execute via anyos_std::shell
//!   4. On `logout` / `exit`, return to step 2

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{process, sys, fs, ipc, env};
use anyos_std::shell::{self, Redirect};
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;

// ─── OS Version ──────────────────────────────────────────────────────────────

const VERSION: &str = env!("ANYOS_VERSION");

// ─── Helpers: console I/O ────────────────────────────────────────────────────

fn print(s: &str) { sys::con_write(s); }
fn println(s: &str) { sys::con_write(s); sys::con_write("\n"); }

fn read_line() -> String {
    let mut buf = [0u8; 256];
    let n = sys::con_read_line(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("").trim_end();
    String::from(s)
}

fn read_password() -> String {
    let mut buf = [0u8; 128];
    let n = sys::con_read_password(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("");
    String::from(s)
}

// ─── Hostname helper ─────────────────────────────────────────────────────────

fn hostname() -> String {
    let mut buf = [0u8; 64];
    let n = sys::get_hostname(&mut buf) as usize;
    if n > 0 && n <= 64 {
        String::from(core::str::from_utf8(&buf[..n]).unwrap_or("anyos"))
    } else {
        String::from("anyos")
    }
}

// ─── Banner ──────────────────────────────────────────────────────────────────

fn print_banner() {
    println("\x1b[2J\x1b[H");
    println("┌──────────────────────────────────────────┐");
    let ver_line = format!("│          .anyOS {}            │", VERSION);
    println(&ver_line);
    println("│       Text Console (nogui mode)          │");
    println("└──────────────────────────────────────────┘");
    println("");
    let banner = format!("{} console login", hostname());
    println(&banner);
    println("");
}

// ─── Environment setup ───────────────────────────────────────────────────────

fn setup_environment(username: &str) {
    let uid = process::getuid();
    env::set("USER",    username);
    env::set("LOGNAME", username);
    let home = if uid == 0 { String::from("/root") } else { format!("/home/{}", username) };
    env::set("HOME",  &home);
    env::set("SHELL", "/System/bin/sh");
    env::set("PATH",  "/System/bin:/System/sbin:/bin:/usr/bin");
    env::set("TERM",  "ansi");
    let (cols, rows) = sys::con_get_size();
    if cols > 0 && rows > 0 {
        env::set("COLUMNS", &format!("{}", cols));
        env::set("LINES",   &format!("{}", rows));
    }
    env::set("UID", &format!("{}", uid));

    // Parse /System/etc/profile for additional KEY=VALUE lines.
    if let Ok(profile) = fs::read_to_string("/System/etc/profile") {
        for line in profile.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some(eq) = line.find('=') {
                let key = &line[..eq];
                let val = line[eq + 1..].trim_matches('"').trim_matches('\'');
                if !key.is_empty() { env::set(key, val); }
            }
        }
    }
}

// ─── Login loop ──────────────────────────────────────────────────────────────

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

// ─── Path helpers ─────────────────────────────────────────────────────────────

fn resolve_path(cwd: &str, rel: &str) -> String {
    if rel.starts_with('/') { return normalize_path(rel); }
    let base = if cwd.ends_with('/') { format!("{}{}", cwd, rel) } else { format!("{}/{}", cwd, rel) };
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
    if parts.is_empty() { return String::from("/"); }
    let mut out = String::new();
    for p in &parts { out.push('/'); out.push_str(p); }
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
                if line.len() + name.len() + 2 > 78 { println(&line); line = String::new(); }
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

// ─── External process runner ─────────────────────────────────────────────────

/// Spawn an external program, redirect its stdout through a named pipe to the
/// console, and wait.  Ctrl+C (0x03 from con_poll_key) kills the child.
/// If `redirect` is set, output goes to a file instead of the console.
/// If `stdin_data` is set, it is written to the child's stdin pipe.
fn run_external(
    path: &str,
    full_args: &str,
    redirect: Option<Redirect>,
    stdin_data: Option<String>,
    pipe_counter: &mut u32,
) {
    let pipe_name = format!("tc:{}", *pipe_counter);
    *pipe_counter = pipe_counter.wrapping_add(1);
    let stdout_pipe = ipc::pipe_create(&pipe_name);
    if stdout_pipe == 0 || stdout_pipe == u32::MAX {
        let msg = format!("{}: failed to create output pipe", path);
        println(&msg);
        return;
    }

    let stdin_name = format!("tc:in:{}", *pipe_counter);
    *pipe_counter = pipe_counter.wrapping_add(1);
    let stdin_pipe = ipc::pipe_create(&stdin_name);

    let tid = process::spawn_piped_full(path, full_args, stdout_pipe, stdin_pipe);
    if tid == u32::MAX {
        ipc::pipe_close(stdout_pipe);
        ipc::pipe_close(stdin_pipe);
        let cmd = path.rsplit('/').next().unwrap_or(path);
        let msg = format!("{}: command not found", cmd);
        println(&msg);
        return;
    }

    // Feed stdin data if requested (e.g. from input redirect)
    if let Some(data) = stdin_data {
        ipc::pipe_write(stdin_pipe, data.as_bytes());
    }

    let mut redirect = redirect;
    let mut buf = [0u8; 512];
    'pump: loop {
        loop {
            let n = ipc::pipe_read(stdout_pipe, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                match redirect {
                    Some(ref mut r) => shell::write_redirect(r, s),
                    None => { sys::con_write(s); },
                }
            }
        }

        // Ctrl+C
        let key = sys::con_poll_key();
        if key == 0x03 {
            process::kill(tid);
            sys::con_write("\n^C\n");
            // drain
            loop {
                let n = ipc::pipe_read(stdout_pipe, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    match redirect { Some(ref mut r) => shell::write_redirect(r, s), None => { sys::con_write(s); } }
                }
            }
            break 'pump;
        }

        let exit = process::try_waitpid(tid);
        if exit != process::STILL_RUNNING {
            // drain remaining
            loop {
                let n = ipc::pipe_read(stdout_pipe, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    match redirect { Some(ref mut r) => shell::write_redirect(r, s), None => { sys::con_write(s); } }
                }
            }
            break;
        }

        process::sleep(10);
    }

    ipc::pipe_close(stdout_pipe);
    ipc::pipe_close(stdin_pipe);
}

/// Run a pipeline (cmd1 | cmd2 | ...) and pump the display pipe to console.
fn run_pipeline(line: &str, cwd: &str, redirect: Option<Redirect>, pipe_counter: &mut u32) {
    let result = match shell::run_pipeline(line, cwd, pipe_counter) {
        Some(r) => r,
        None => {
            println("pipeline: command not found");
            return;
        }
    };

    let mut redirect = redirect;
    let mut buf = [0u8; 512];
    'pump: loop {
        loop {
            let n = ipc::pipe_read(result.display_pipe, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                match redirect { Some(ref mut r) => shell::write_redirect(r, s), None => { sys::con_write(s); } }
            }
        }

        let key = sys::con_poll_key();
        if key == 0x03 {
            process::kill(result.last_tid);
            sys::con_write("\n^C\n");
            break 'pump;
        }

        let exit = process::try_waitpid(result.last_tid);
        if exit != process::STILL_RUNNING {
            loop {
                let n = ipc::pipe_read(result.display_pipe, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    match redirect { Some(ref mut r) => shell::write_redirect(r, s), None => { sys::con_write(s); } }
                }
            }
            break;
        }

        process::sleep(10);
    }

    ipc::pipe_close(result.display_pipe);
    for p in result.extra_pipes { ipc::pipe_close(p); }
}

// ─── Shell loop ──────────────────────────────────────────────────────────────

fn shell_loop(username: &str) {
    let mut cwd = String::from("/");
    let mut pipe_counter: u32 = 0;

    loop {
        // Prompt: user@host:/path $
        let hn = hostname();
        let prompt_suffix = if process::getuid() == 0 { "#" } else { "$" };
        let prompt = format!("{}@{}:{} {} ", username, hn, cwd, prompt_suffix);
        print(&prompt);

        let line = read_line();
        if line.is_empty() { continue; }
        let line_trimmed = line.trim();

        // Strip output redirect
        let (line_no_out, redirect) = shell::parse_redirects(line_trimmed, &cwd);
        // Strip input redirect
        let (cmd_line, input_redirect) = shell::parse_input_redirect(&line_no_out, &cwd);
        let cmd_line = cmd_line.trim();
        if cmd_line.is_empty() { continue; }

        // Read stdin data from input redirect file
        let stdin_data: Option<String> = input_redirect.and_then(|ir| {
            fs::read_to_string(&ir.source).ok()
        });

        // First token is the command, rest are args (after full POSIX expansion)
        let expanded = shell::expand_args(cmd_line, &cwd);
        if expanded.is_empty() { continue; }
        let cmd = expanded[0].as_str();
        // Rebuild args string from expanded tokens (skip argv[0])
        let args_expanded = &expanded[1..];

        match cmd {
            "exit" | "logout" => {
                println("Logout.");
                return;
            }
            "halt" | "shutdown" => {
                println("Shutting down...");
                process::shutdown();
            }
            "reboot" => {
                println("Rebooting...");
                process::reboot();
            }
            "clear" => {
                sys::con_write("\x1b[2J\x1b[H");
            }
            "pwd" => {
                println(&cwd);
            }
            "cd" => {
                let dest = if args_expanded.is_empty() {
                    let uid = process::getuid();
                    if uid == 0 { "/root" } else { "/" }
                } else {
                    args_expanded[0].as_str()
                };
                let resolved = resolve_path(&cwd, dest);
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
                // If any args that are not flags → treat as path, else use cwd
                let dir = if args_expanded.iter().any(|a| !a.starts_with('-')) {
                    let path_arg = args_expanded.iter().find(|a| !a.starts_with('-')).unwrap();
                    if path_arg.starts_with('/') {
                        path_arg.clone()
                    } else {
                        resolve_path(&cwd, path_arg.as_str())
                    }
                } else {
                    cwd.clone()
                };
                list_dir(&dir);
            }
            "cat" => {
                if args_expanded.is_empty() {
                    println("Usage: cat <file>");
                } else {
                    for arg in args_expanded {
                        let path = if arg.starts_with('/') {
                            arg.clone()
                        } else {
                            resolve_path(&cwd, arg.as_str())
                        };
                        cat_file(&path);
                    }
                }
            }
            "echo" => {
                let out = shell::join(args_expanded);
                println(&out);
            }
            "whoami" => { println(username); }
            "uname" => {
                let msg = format!("anyOS {} x86_64", VERSION);
                println(&msg);
            }
            "dmesg" => {
                let mut buf = [0u8; 8192];
                let n = sys::dmesg(&mut buf) as usize;
                if n > 0 {
                    if let Ok(s) = core::str::from_utf8(&buf[..n]) { print(s); }
                }
            }
            "help" => {
                println("Built-in commands:");
                println("  cd [dir]     Change directory");
                println("  ls [dir]     List directory");
                println("  cat <file>   Print file");
                println("  pwd          Print working directory");
                println("  echo [text]  Print text");
                println("  whoami       Print username");
                println("  uname        Print system info");
                println("  dmesg        Print kernel log");
                println("  clear        Clear screen");
                println("  halt         Halt system");
                println("  reboot       Reboot system");
                println("  exit/logout  Log out");
                println("  <cmd> [args] Run external program");
                println("  cmd1 | cmd2  Pipeline");
                println("  cmd > file   Output redirect");
                println("  cmd < file   Input redirect");
            }
            _ => {
                // Check for pipeline
                if shell::has_pipe(cmd_line) {
                    run_pipeline(cmd_line, &cwd, redirect, &mut pipe_counter);
                    continue;
                }

                // External program
                let prog_path = shell::resolve_cmd_path(cmd, &cwd);
                // Rebuild full args string: argv[0] + expanded args
                let args_str = shell::join(args_expanded);
                let full_args = if args_str.is_empty() {
                    String::from(cmd)
                } else {
                    format!("{} {}", cmd, args_str)
                };
                run_external(&prog_path, &full_args, redirect, stdin_data, &mut pipe_counter);
            }
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
