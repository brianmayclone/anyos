//! anyOS Text-Mode Console
//!
//! Launched by the kernel when the `nogui` boot parameter is set.
//! Provides a classic login prompt followed by a command-line shell
//! entirely via the kernel framebuffer console (`SYS_CON_WRITE` /
//! `SYS_CON_READ`).  No compositor, no sessionhost, no GUI.
//!
//! Shell features shared with Terminal.app via `anyos_std::shell`:
//!   - POSIX tokenisation (quoting, backslash escapes)
//!   - Variable expansion (`$VAR`, `${VAR}`, `$(cmd)`, backticks)
//!   - Tilde expansion
//!   - Glob expansion (`*`, `?`, `[…]`)
//!   - Output redirect (`>`, `>>`, `2>`, `2>>`, `1>`, `1>>`, `2>&1`)
//!   - Input redirect (`<`)
//!   - Pipeline (`cmd1 | cmd2 | cmd3`)
//!
//! # Module layout
//!
//! | Module          | Responsibility                                      |
//! |-----------------|-----------------------------------------------------|
//! | [`io`]          | `print` / `println` / `read_line` / `read_password`|
//! | [`history`]     | Load and append `~/.history`                        |
//! | [`readline`]    | Interactive line editor with history + TAB          |
//! | [`completion`]  | TAB completion for commands and paths               |
//! | [`paths`]       | `resolve_path` / `normalize_path`                   |
//! | [`runner`]      | Spawn external commands and pipelines               |
//! | [`login`]       | Banner, authentication loop, environment setup      |
//! | [`shell_loop`]  | Interactive shell (built-ins + external dispatch)   |
//!
//! # Overall flow
//!
//! 1. [`config::apply_system_config`] — keyboard layout + console mode
//! 2. [`login::print_banner`] — clear screen + neofetch + hostname
//! 3. [`login::login_loop`] — authenticate, setup environment
//! 4. [`shell_loop::shell_loop`] — read / execute commands
//! 5. On `exit` / `logout`, go back to step 3

#![no_std]
#![no_main]

anyos_std::entry!(main);

mod completion;
mod config;
mod history;
mod io;
mod login;
mod paths;
mod readline;
mod runner;
mod shell_loop;

fn main() -> u32 {
    config::apply_system_config();

    // When launched with `--shell` (e.g. by `su`), skip the banner and login
    // prompt and drop straight into the shell using the already-set identity.
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");
    let shell_only = (0..args.pos_count).any(|i| args.positional[i] == "--shell");

    if shell_only {
        // Resolve current username from $USER (set by su before spawning us).
        let mut user_buf = [0u8; 32];
        let ulen = anyos_std::env::get("USER", &mut user_buf);
        let username = if ulen != u32::MAX && ulen > 0 {
            anyos_std::String::from(
                core::str::from_utf8(&user_buf[..ulen as usize]).unwrap_or("root")
            )
        } else {
            anyos_std::String::from("root")
        };
        shell_loop::shell_loop(&username);
        return 0;
    }

    login::print_banner();
    loop {
        let username = login::login_loop();
        shell_loop::shell_loop(&username);
    }
}
