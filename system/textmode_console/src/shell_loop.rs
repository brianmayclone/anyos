//! The interactive shell loop executed after a successful login.
//!
//! # Prompt format
//!
//! ```
//! username@hostname:/current/path $ command
//! username@hostname:/current/path # command   ← root
//! ```
//!
//! The prompt suffix is `#` for UID 0 and `$` for all other users.
//!
//! # Built-in commands
//!
//! | Command          | Effect                                      |
//! |------------------|---------------------------------------------|
//! | `exit` / `logout`| Return to the login prompt                 |
//! | `cd [path]`      | Change directory (defaults to `$HOME`)      |
//! | `clear`          | Clear the screen via ANSI escape            |
//! | `export` / `set` | List or set environment variables           |
//! | `unset`          | Remove an environment variable              |
//!
//! All other commands are looked up on `$PATH` and executed via
//! [`runner::run_external`] or [`runner::run_pipeline`].

use anyos_std::{env, fs, process};
use anyos_std::shell;
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;

use crate::io::println;
use crate::login::hostname;
use crate::paths::resolve_path;
use crate::readline::read_line_interactive;
use crate::runner::{run_external, run_pipeline, sync_term_size};

/// Run the interactive shell for `username` until the user types `exit` or
/// `logout`, then return so the caller can restart the login loop.
pub fn shell_loop(username: &str) {
    // Start in the user's home directory.
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    let mut cwd = if hlen != u32::MAX && hlen > 0 {
        String::from(core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/"))
    } else {
        String::from("/")
    };
    env::set("PWD", &cwd);
    // Sync the kernel thread's CWD so spawned processes inherit the right directory.
    fs::chdir(&cwd);

    let mut pipe_counter: u32 = 0;

    loop {
        // ── Build prompt ───────────────────────────────────────────────────
        // Format: `user@host:/path # ` (root) or `user@host:/path $ ` (user)
        let hn = hostname();
        let suffix = if process::getuid() == 0 { "#" } else { "$" };

        let mut user_buf = [0u8; 32];
        let ulen = env::get("USER", &mut user_buf);
        let display_user = if ulen != u32::MAX && ulen > 0 {
            core::str::from_utf8(&user_buf[..ulen as usize]).unwrap_or(username)
        } else {
            username
        };

        let prompt = format!("{}@{}:{} {} ", display_user, hn, cwd, suffix);
        anyos_std::sys::con_write(&prompt);

        // ── Read input ─────────────────────────────────────────────────────
        let line = read_line_interactive(&prompt, &cwd);
        if line.is_empty() { continue; }
        let line_trimmed = line.trim();

        // ── Parse redirects ────────────────────────────────────────────────
        let (line_no_out, redirect)    = shell::parse_redirects(line_trimmed, &cwd);
        let (cmd_line, input_redirect) = shell::parse_input_redirect(&line_no_out, &cwd);
        let cmd_line = cmd_line.trim();
        if cmd_line.is_empty() { continue; }

        // ── Background suffix (&) ─────────────────────────────────────────
        let (cmd_line, background) = parse_background(cmd_line);
        let cmd_line = cmd_line.trim();
        if cmd_line.is_empty() { continue; }

        // ── Input redirect data ────────────────────────────────────────────
        let stdin_data: Option<String> = input_redirect.and_then(|ir| {
            fs::read_to_string(&ir.source).ok()
        });

        // ── Expand arguments ───────────────────────────────────────────────
        let expanded = shell::expand_args(cmd_line, &cwd);
        if expanded.is_empty() { continue; }
        let cmd = expanded[0].as_str();
        let args_expanded = &expanded[1..];

        // ── Dispatch ───────────────────────────────────────────────────────
        match cmd {
            "exit" | "logout" => {
                println("Logout.");
                return;
            }

            "cd" => builtin_cd(&mut cwd, args_expanded),

            "clear" => { anyos_std::sys::con_write("\x1b[2J\x1b[H"); },

            "export" | "set" => builtin_export(args_expanded),

            "unset" => {
                for arg in args_expanded { env::unset(arg.as_str()); }
            }

            _ => {
                // Strip `nohup` prefix (anyOS has no SIGHUP, so it is a no-op
                // except for the implied background execution).
                let (cmd, args_expanded, background) = if cmd == "nohup" {
                    if args_expanded.is_empty() {
                        println("nohup: missing command");
                        continue;
                    }
                    (args_expanded[0].as_str(), &args_expanded[1..], true)
                } else {
                    (cmd, args_expanded, background)
                };

                if shell::has_pipe(cmd_line) {
                    if background {
                        println("[bg] pipeline background not supported, running in foreground");
                    }
                    run_pipeline(cmd_line, &cwd, redirect, &mut pipe_counter);
                    sync_term_size();
                    continue;
                }

                // Inject `cwd` for `ls` when no non-flag argument is given.
                let mut effective_args: Vec<String> = args_expanded.to_vec();
                if cmd == "ls" && effective_args.iter().all(|a| a.starts_with('-')) {
                    effective_args.push(cwd.clone());
                }

                let args_str  = shell::join(&effective_args);
                let full_args = if args_str.is_empty() {
                    String::from(cmd)
                } else {
                    format!("{} {}", cmd, args_str)
                };
                let prog_path = shell::resolve_cmd_path(cmd, &cwd);

                if background {
                    let tid = process::spawn(&prog_path, &full_args);
                    if tid == u32::MAX {
                        let msg = format!("{}: command not found", cmd);
                        println(&msg);
                    } else {
                        let msg = format!("[bg] pid={}", tid);
                        println(&msg);
                    }
                } else {
                    run_external(&prog_path, &full_args, redirect, stdin_data, &mut pipe_counter);
                    sync_term_size();
                }
            }
        }
    }
}

// ─── Built-in implementations ────────────────────────────────────────────────

/// `cd [path]` — change the current working directory.
fn builtin_cd(cwd: &mut String, args: &[String]) {
    let cd_home_buf: String;
    let dest = if args.is_empty() {
        let mut hbuf = [0u8; 128];
        let hl = env::get("HOME", &mut hbuf);
        cd_home_buf = if hl != u32::MAX && hl > 0 {
            String::from(core::str::from_utf8(&hbuf[..hl as usize]).unwrap_or("/"))
        } else {
            String::from("/")
        };
        cd_home_buf.as_str()
    } else {
        args[0].as_str()
    };

    let resolved = resolve_path(cwd, dest);
    let mut stat_buf = [0u32; 7];
    if fs::stat(&resolved, &mut stat_buf) == 0 {
        if stat_buf[0] == 1 {
            *cwd = resolved.clone();
            env::set("PWD", cwd);
            fs::chdir(&resolved);
        } else {
            let msg = format!("cd: {}: Not a directory", dest);
            println(&msg);
        }
    } else {
        let msg = format!("cd: {}: No such file or directory", dest);
        println(&msg);
    }
}

/// `export` / `set` — list all variables or set `KEY=VALUE` pairs.
fn builtin_export(args: &[String]) {
    if args.is_empty() {
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
        for arg in args {
            let a = arg.as_str();
            if let Some(eq) = a.find('=') {
                let key = &a[..eq];
                let val = &a[eq + 1..];
                if !key.is_empty() { env::set(key, val); }
            }
            // `export KEY` (without `=`) is accepted silently.
        }
    }
}

// ─── Background flag parsing ──────────────────────────────────────────────────

/// Strip a trailing `&` from `cmd_line` and return `(stripped_line, true)` if
/// present, otherwise return `(cmd_line, false)`.
fn parse_background(cmd_line: &str) -> (&str, bool) {
    if cmd_line.ends_with(" &") || cmd_line.ends_with("\t&") {
        (&cmd_line[..cmd_line.len() - 2], true)
    } else if cmd_line.ends_with('&') && cmd_line.len() > 1 {
        (&cmd_line[..cmd_line.len() - 1], true)
    } else {
        (cmd_line, false)
    }
}
