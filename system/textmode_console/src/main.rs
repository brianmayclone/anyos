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

use anyos_std::{process, sys, fs, ipc, env, kbd};
use anyos_std::shell::{self, Redirect};
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;

// ─── System config (keyboard layout + console mode) ──────────────────────────

/// Read /System/etc/inputmon.conf and apply the keyboard layout.
/// Read CONSOLE_MODE from /System/env and resize if set.
fn apply_system_config() {
    // Keyboard layout from /System/etc/inputmon.conf
    if let Ok(conf) = fs::read_to_string("/System/etc/inputmon.conf") {
        let mut in_keyboard = false;
        for line in conf.split('\n') {
            let line = line.trim();
            if line == "[keyboard]" { in_keyboard = true; continue; }
            if line.starts_with('[') { in_keyboard = false; continue; }
            if in_keyboard {
                if let Some(val) = line.strip_prefix("layout=") {
                    if let Ok(id) = val.trim().parse::<u32>() {
                        kbd::set_layout(id);
                    }
                }
            }
        }
    }

    // Console mode: check CONSOLE_MODE in /System/env
    // Predefined modes matching bin/mode: 1=80x25, 2=120x37, 3=160x50
    const MODES: [(u32, u32); 3] = [(80, 25), (120, 37), (160, 50)];
    if let Ok(conf) = fs::read_to_string("/System/env") {
        for line in conf.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some(val) = line.strip_prefix("CONSOLE_MODE=") {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if let Ok(mode) = val.parse::<usize>() {
                    if mode >= 1 && mode <= MODES.len() {
                        let (cols, rows) = MODES[mode - 1];
                        sys::con_resize(cols, rows);
                    }
                }
            }
        }
    }
}

// ─── Helpers: console I/O ────────────────────────────────────────────────────

fn print(s: &str) { sys::con_write(s); }
fn println(s: &str) { sys::con_write(s); sys::con_write("\n"); }

// ─── History ─────────────────────────────────────────────────────────────────

const HISTORY_MAX: usize = 200;
const HISTORY_FILE: &str = "/.history"; // relative to HOME, resolved at runtime

/// Load history from ~/.history. Returns entries oldest-first.
fn history_load() -> Vec<String> {
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    if hlen == u32::MAX || hlen == 0 { return Vec::new(); }
    let home = core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/");
    let path = format!("{}{}", home, HISTORY_FILE);
    match fs::read_to_string(&path) {
        Ok(s) => s.split('\n').filter(|l| !l.is_empty()).map(String::from).collect(),
        Err(_) => Vec::new(),
    }
}

/// Append one entry to ~/.history (keeps last HISTORY_MAX lines).
fn history_append(entry: &str) {
    if entry.is_empty() { return; }
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    if hlen == u32::MAX || hlen == 0 { return; }
    let home = core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/");
    let path = format!("{}{}", home, HISTORY_FILE);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<&str> = existing.split('\n').filter(|l| !l.is_empty()).collect();
    lines.push(entry);
    if lines.len() > HISTORY_MAX { lines = lines[lines.len() - HISTORY_MAX..].to_vec(); }
    let mut out = String::new();
    for l in &lines { out.push_str(l); out.push('\n'); }
    let _ = fs::write_bytes(&path, out.as_bytes());
}

// ─── Interactive read_line with history + cursor movement ────────────────────

fn format_usize_into(buf: &mut [u8], mut n: usize) -> usize {
    if n == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 20];
    let mut len = 0;
    while n > 0 { tmp[len] = b'0' + (n % 10) as u8; len += 1; n /= 10; }
    for i in 0..len { buf[i] = tmp[len - 1 - i]; }
    len
}

/// Full-featured readline: history navigation, left/right cursor, Shift+arrow scroll.
fn read_line_interactive(prompt: &str) -> String {
    let history = history_load();
    // hist_idx: history.len() = current input, < len = browsing history
    let mut hist_idx = history.len();
    // saved_line: preserves current input while browsing history
    let mut saved_line = [0u8; 512];
    let mut saved_len = 0usize;

    let mut line = [0u8; 512];
    let mut line_len = 0usize;
    let mut cursor = 0usize; // byte position within line

    loop {
        let key = loop {
            let k = sys::con_poll_key();
            if k != 0 { break k; }
            process::sleep(5);
        };

        match key {
            // Enter
            0x0A | 0x0D => {
                sys::con_write("\n");
                let s = core::str::from_utf8(&line[..line_len]).unwrap_or("").trim_end();
                let result = String::from(s);
                history_append(&result);
                return result;
            }

            // Ctrl+C
            0x03 => {
                sys::con_write("^C\n");
                return String::new();
            }

            // Backspace
            0x08 | 0x7F => {
                if cursor > 0 {
                    // Remove byte at cursor-1
                    for i in cursor - 1..line_len - 1 { line[i] = line[i + 1]; }
                    line_len -= 1;
                    cursor -= 1;
                    sys::con_write("\x08"); // move back one
                    redraw_from_cursor(&line, line_len, cursor);
                }
            }

            // Delete (forward)
            sys::KEY_DELETE => {
                if cursor < line_len {
                    for i in cursor..line_len - 1 { line[i] = line[i + 1]; }
                    line_len -= 1;
                    redraw_from_cursor(&line, line_len, cursor);
                }
            }

            // Cursor LEFT
            sys::KEY_LEFT => {
                if cursor > 0 {
                    cursor -= 1;
                    sys::con_write("\x1b[D");
                }
            }

            // Cursor RIGHT
            sys::KEY_RIGHT => {
                if cursor < line_len {
                    cursor += 1;
                    sys::con_write("\x1b[C");
                }
            }

            // Home
            sys::KEY_HOME => {
                if cursor > 0 {
                    let mut esc = [0u8; 16];
                    let n = format_usize_into(&mut esc, cursor);
                    sys::con_write("\x1b[");
                    sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
                    sys::con_write("D");
                    cursor = 0;
                }
            }

            // End
            sys::KEY_END => {
                if cursor < line_len {
                    let fwd = line_len - cursor;
                    let mut esc = [0u8; 16];
                    let n = format_usize_into(&mut esc, fwd);
                    sys::con_write("\x1b[");
                    sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
                    sys::con_write("C");
                    cursor = line_len;
                }
            }

            // Cursor UP — history previous
            sys::KEY_UP => {
                if hist_idx == history.len() {
                    // Save current input before browsing
                    saved_len = line_len;
                    saved_line[..line_len].copy_from_slice(&line[..line_len]);
                }
                if hist_idx > 0 {
                    hist_idx -= 1;
                    let entry = history[hist_idx].as_bytes();
                    let copy_len = entry.len().min(line.len());
                    line[..copy_len].copy_from_slice(&entry[..copy_len]);
                    line_len = copy_len;
                    cursor = line_len;
                    // Redraw whole line
                    sys::con_write("\r\x1b[2K");
                    sys::con_write(prompt);
                    sys::con_write(core::str::from_utf8(&line[..line_len]).unwrap_or(""));
                }
            }

            // Cursor DOWN — history next / restore
            sys::KEY_DOWN => {
                if hist_idx < history.len() {
                    hist_idx += 1;
                    if hist_idx == history.len() {
                        // Restore saved input
                        line[..saved_len].copy_from_slice(&saved_line[..saved_len]);
                        line_len = saved_len;
                    } else {
                        let entry = history[hist_idx].as_bytes();
                        let copy_len = entry.len().min(line.len());
                        line[..copy_len].copy_from_slice(&entry[..copy_len]);
                        line_len = copy_len;
                    }
                    cursor = line_len;
                    sys::con_write("\r\x1b[2K");
                    sys::con_write(prompt);
                    sys::con_write(core::str::from_utf8(&line[..line_len]).unwrap_or(""));
                }
            }

            // Shift+UP — scroll viewport up one line
            sys::KEY_SHIFT_UP => {
                sys::con_write("\x1b[1T");
            }

            // Shift+DOWN — scroll viewport down one line
            sys::KEY_SHIFT_DOWN => {
                sys::con_write("\x1b[1S");
            }

            // Normal printable character
            ch if ch >= 0x20 && ch < 0x7F => {
                if line_len < line.len() - 1 {
                    // Insert at cursor
                    for i in (cursor..line_len).rev() { line[i + 1] = line[i]; }
                    line[cursor] = ch as u8;
                    line_len += 1;
                    cursor += 1;
                    if cursor == line_len {
                        // Appending at end — simple echo
                        let b = [ch as u8];
                        sys::con_write(core::str::from_utf8(&b).unwrap_or(""));
                    } else {
                        // Mid-line insert — redraw from cursor
                        let b = [ch as u8];
                        sys::con_write(core::str::from_utf8(&b).unwrap_or(""));
                        redraw_from_cursor(&line, line_len, cursor);
                    }
                }
            }

            _ => {}
        }
    }
}

/// Redraw from current cursor position to end of line, then reposition.
fn redraw_from_cursor(line: &[u8], line_len: usize, cursor: usize) {
    let rest = core::str::from_utf8(&line[cursor..line_len]).unwrap_or("");
    sys::con_write(rest);
    sys::con_write("\x1b[0K"); // erase rest
    // Move cursor back
    let move_back = line_len - cursor;
    if move_back > 0 {
        let mut esc = [0u8; 16];
        let n = format_usize_into(&mut esc, move_back);
        sys::con_write("\x1b[");
        sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
        sys::con_write("D");
    }
}

fn read_line() -> String {
    // read_line without prompt context — used for login username only
    // (shell_loop uses read_line_interactive directly)
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
    sys::con_write("\x1b[2J\x1b[H");
    // neofetch displays system info (distro, kernel, uptime, etc.)
    let mut pc = 0u32;
    run_external("/System/bin/neofetch", "neofetch", None, None, &mut pc);
    println("");
    let banner = format!("{} console login", hostname());
    println(&banner);
    println("");
}

// ─── Environment setup ───────────────────────────────────────────────────────

fn setup_environment(username: &str) {
    let uid = process::getuid();

    // Resolve actual username from kernel (validates identity was set correctly)
    let mut name_buf = [0u8; 32];
    let nlen = process::getusername(uid, &mut name_buf);
    let resolved_name = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize]).unwrap_or(username)
    } else {
        username
    };

    // Home directory: root → /, others → /Users/<name> (like compositor)
    let home = if uid == 0 {
        String::from("/")
    } else {
        format!("/Users/{}", resolved_name)
    };

    env::set("USER",    resolved_name);
    env::set("LOGNAME", resolved_name);
    env::set("HOME",    &home);
    env::set("PWD",     &home);
    env::set("SHELL",   "/System/bin/sh");
    env::set("PATH",    "/System/bin:/System/sbin:/bin:/usr/bin");
    env::set("TERM",    "ansi");
    let (cols, rows) = sys::con_get_size();
    if cols > 0 && rows > 0 {
        env::set("COLUMNS", &format!("{}", cols));
        env::set("LINES",   &format!("{}", rows));
    }
    env::set("UID", &format!("{}", uid));

    // Load /System/env (always), then ~/.env (optional) — KEY=VALUE syntax.
    for path in ["/System/env", &format!("{}/.env", home)] {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.split('\n') {
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
            // authenticate() already set uid/gid in the kernel.
            // Call set_identity() explicitly — same as compositor — to ensure
            // all threads in this process inherit the correct identity.
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
    // Restore cursor + auto-scroll in case the app disabled them and crashed/was killed.
    sys::con_set_mode(0);
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
    // Restore cursor + auto-scroll in case the app disabled them and crashed/was killed.
    sys::con_set_mode(0);
}

// ─── Shell loop ──────────────────────────────────────────────────────────────

fn shell_loop(username: &str) {
    // Start in the user's home directory ($HOME set by setup_environment)
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    let mut cwd = if hlen != u32::MAX && hlen > 0 {
        String::from(core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/"))
    } else {
        String::from("/")
    };
    env::set("PWD", &cwd);
    let mut pipe_counter: u32 = 0;

    loop {
        // Prompt: user@host:/path $ — read USER from env (set after identity switch)
        let hn = hostname();
        let prompt_suffix = if process::getuid() == 0 { "#" } else { "$" };
        let mut user_buf = [0u8; 32];
        let ulen = env::get("USER", &mut user_buf);
        let display_user = if ulen != u32::MAX && ulen > 0 {
            core::str::from_utf8(&user_buf[..ulen as usize]).unwrap_or(username)
        } else { username };
        let prompt = format!("{}@{}:{} {} ", display_user, hn, cwd, prompt_suffix);
        print(&prompt);

        let line = read_line_interactive(&prompt);
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
            // ── True shell builtins (cannot be external programs) ──────────────
            "exit" | "logout" => {
                println("Logout.");
                return;
            }
            "cd" => {
                let cd_home_buf: String;
                let dest = if args_expanded.is_empty() {
                    let mut hbuf = [0u8; 128];
                    let hl = env::get("HOME", &mut hbuf);
                    cd_home_buf = if hl != u32::MAX && hl > 0 {
                        String::from(core::str::from_utf8(&hbuf[..hl as usize]).unwrap_or("/"))
                    } else {
                        String::from("/")
                    };
                    cd_home_buf.as_str()
                } else {
                    args_expanded[0].as_str()
                };
                let resolved = resolve_path(&cwd, dest);
                let mut stat_buf = [0u32; 7];
                if fs::stat(&resolved, &mut stat_buf) == 0 {
                    if stat_buf[0] == 1 {
                        cwd = resolved;
                        env::set("PWD", &cwd);
                    } else {
                        let msg = format!("cd: {}: Not a directory", dest);
                        println(&msg);
                    }
                } else {
                    let msg = format!("cd: {}: No such file or directory", dest);
                    println(&msg);
                }
            }
            // clear: ANSI sequence direct to framebuffer console — no external needed
            "clear" => {
                sys::con_write("\x1b[2J\x1b[H");
            }
            // export / set: list all env vars, or set KEY=VALUE
            "export" | "set" => {
                if args_expanded.is_empty() {
                    // List all environment variables
                    let mut buf = [0u8; 4096];
                    let n = env::list(&mut buf);
                    if n > 0 {
                        let avail = (n as usize).min(buf.len());
                        let s = core::str::from_utf8(&buf[..avail]).unwrap_or("");
                        for var in s.split('\0').filter(|v| !v.is_empty()) {
                            let msg = format!("export {}", var);
                            println(&msg);
                        }
                    }
                } else {
                    for arg in args_expanded {
                        let a = arg.as_str();
                        if let Some(eq) = a.find('=') {
                            let key = &a[..eq];
                            let val = &a[eq + 1..];
                            if !key.is_empty() { env::set(key, val); }
                        }
                        // export KEY (no =) — silently accepted, var already in env
                    }
                }
            }
            // unset: remove env variable
            "unset" => {
                for arg in args_expanded {
                    env::unset(arg.as_str());
                }
            }
            _ => {
                // Check for pipeline
                if shell::has_pipe(cmd_line) {
                    run_pipeline(cmd_line, &cwd, redirect, &mut pipe_counter);
                    continue;
                }

                // External program
                let prog_path = shell::resolve_cmd_path(cmd, &cwd);
                // For `ls` with no path argument, inject cwd so it lists the right directory.
                let mut effective_args: Vec<String> = args_expanded.to_vec();
                if cmd == "ls" && effective_args.iter().all(|a| a.starts_with('-')) {
                    effective_args.push(cwd.clone());
                }
                let args_str = shell::join(&effective_args);
                let full_args = if args_str.is_empty() {
                    format!("{} {}", cmd, cwd)
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
    apply_system_config();
    print_banner();
    loop {
        let username = login_loop();
        shell_loop(&username);
    }
}
