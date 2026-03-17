//! External process execution: single commands and pipelines.
//!
//! Both [`run_external`] and [`run_pipeline`] follow the same pump loop:
//!
//! 1. Spawn the child process with a named IPC pipe for stdout.
//! 2. Pump bytes from the pipe to the console (or a redirect target).
//! 3. Check for Ctrl+C — kill the child and drain remaining output.
//! 4. Check whether the child has exited; if so, drain and break.
//! 5. Sleep briefly before the next iteration.
//!
//! After every external command the terminal mode is reset via
//! `sys::con_set_mode(0)` so that a crashed application cannot leave the
//! console in a broken state (e.g. cursor hidden, auto-scroll off).

use anyos_std::{ipc, process, sys};
use anyos_std::shell::{self, Redirect};
use anyos_std::String;
use anyos_std::format;

use crate::io::println;

// ─── Single command ───────────────────────────────────────────────────────────

/// Spawn `path` with `full_args`, pump its stdout to the console (or
/// `redirect`), and wait for it to exit.
///
/// - `redirect`: when `Some`, stdout bytes go to the redirect target instead
///   of the console.
/// - `stdin_data`: when `Some`, these bytes are written to the child's stdin
///   pipe (implements `<` input redirection).
/// - `pipe_counter`: monotonically-increasing counter used to generate unique
///   pipe names; updated in place.
pub fn run_external(
    path:         &str,
    full_args:    &str,
    redirect:     Option<Redirect>,
    stdin_data:   Option<String>,
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
    *pipe_counter  = pipe_counter.wrapping_add(1);
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

    // Feed stdin data for input redirection (`< file`).
    if let Some(data) = stdin_data {
        ipc::pipe_write(stdin_pipe, data.as_bytes());
    }

    pump_pipe(stdout_pipe, tid, redirect);

    ipc::pipe_close(stdout_pipe);
    ipc::pipe_close(stdin_pipe);
    sys::con_set_mode(0);
}

// ─── Pipeline ────────────────────────────────────────────────────────────────

/// Run a shell pipeline (`cmd1 | cmd2 | …`) and pump the final stage's output
/// to the console (or `redirect`).
pub fn run_pipeline(
    line:         &str,
    cwd:          &str,
    redirect:     Option<Redirect>,
    pipe_counter: &mut u32,
) {
    let result = match shell::run_pipeline(line, cwd, pipe_counter) {
        Some(r) => r,
        None    => { println("pipeline: command not found"); return; }
    };

    pump_pipe(result.display_pipe, result.last_tid, redirect);

    ipc::pipe_close(result.display_pipe);
    for p in result.extra_pipes { ipc::pipe_close(p); }
    sys::con_set_mode(0);
}

// ─── Terminal size sync ───────────────────────────────────────────────────────

/// Re-read the console dimensions from the kernel and update `$COLUMNS` /
/// `$LINES`.  Should be called after every external command because the user
/// may have run the `mode` built-in to resize the terminal.
pub fn sync_term_size() {
    use anyos_std::env;
    use anyos_std::format;
    let (cols, rows) = sys::con_get_size();
    if cols > 0 && rows > 0 {
        env::set("COLUMNS", &format!("{}", cols));
        env::set("LINES",   &format!("{}", rows));
    }
}

// ─── Internal pump helper ────────────────────────────────────────────────────

/// Read all output from `pipe` and write it to the console (or `redirect`),
/// handling Ctrl+C and process exit.
fn pump_pipe(pipe: u32, tid: u32, redirect: Option<Redirect>) {
    let mut redirect = redirect;
    let mut buf = [0u8; 512];

    'pump: loop {
        // Drain all currently available bytes.
        loop {
            let n = ipc::pipe_read(pipe, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                write_output(s, &mut redirect);
            }
        }

        // Ctrl+C: kill child, drain remaining output, stop.
        if sys::con_poll_key() == 0x03 {
            process::kill(tid);
            sys::con_write("\n^C\n");
            drain_pipe(pipe, &mut redirect, &mut buf);
            break 'pump;
        }

        // Child has exited: drain remaining output and stop.
        if process::try_waitpid(tid) != process::STILL_RUNNING {
            drain_pipe(pipe, &mut redirect, &mut buf);
            break;
        }

        process::sleep(10);
    }
}

/// Write `s` to the console or the redirect target.
fn write_output(s: &str, redirect: &mut Option<Redirect>) {
    match redirect {
        Some(ref mut r) => shell::write_redirect(r, s),
        None            => { sys::con_write(s); },
    }
}

/// Read and discard/forward all remaining bytes from `pipe`.
fn drain_pipe(pipe: u32, redirect: &mut Option<Redirect>, buf: &mut [u8]) {
    loop {
        let n = ipc::pipe_read(pipe, buf);
        if n == 0 || n == u32::MAX { break; }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            write_output(s, redirect);
        }
    }
}
