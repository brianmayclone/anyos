#![no_std]
#![no_main]

use anyos_std::{println, fs, process, shell, sys};
use anyos_std::String;

anyos_std::entry!(main);

fn print_usage() {
    println!("Usage: time COMMAND [ARG]...");
    println!();
    println!("Run COMMAND and report the elapsed wall-clock time on stderr.");
}

fn main() {
    let mut args_buf = [0u8; 512];
    let raw = process::args(&mut args_buf).trim();

    if raw.is_empty() || raw == "--help" || raw == "-h" {
        print_usage();
        return;
    }

    // Split into command + remaining args at first whitespace
    let bytes = raw.as_bytes();
    let mut split = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b' ' || b == b'\t' {
            split = i;
            break;
        }
    }
    let cmd = &raw[..split];
    let cmd_args = raw[split..].trim_start();

    // Resolve cwd for relative path lookup.
    let mut cwd_buf = [0u8; 256];
    let n = fs::getcwd(&mut cwd_buf);
    let cwd = if (n as i32) > 0 {
        core::str::from_utf8(&cwd_buf[..n as usize]).unwrap_or("/")
    } else {
        "/"
    };

    let path = shell::resolve_cmd_path(cmd, cwd);

    let start = sys::uptime_ms();
    let tid = process::spawn(&path, cmd_args);
    if tid == u32::MAX {
        println!("time: failed to start '{}'", cmd);
        return;
    }

    let _exit = process::waitpid(tid);
    let elapsed_ms = sys::uptime_ms().wrapping_sub(start);

    // real    Xm Y.YYYs
    let mut line = String::new();
    line.push_str("\nreal\t");
    let secs_total = elapsed_ms / 1000;
    let ms_part = elapsed_ms % 1000;
    let mins = secs_total / 60;
    let secs = secs_total % 60;
    push_u32(&mut line, mins);
    line.push_str("m");
    push_u32(&mut line, secs);
    line.push('.');
    if ms_part < 100 { line.push('0'); }
    if ms_part < 10  { line.push('0'); }
    push_u32(&mut line, ms_part);
    line.push_str("s");
    println!("{}", line);
}

fn push_u32(s: &mut String, val: u32) {
    if val >= 10 {
        push_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}
