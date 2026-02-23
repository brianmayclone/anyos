#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

anyos_std::entry!(main);

const CRONTAB_DIR: &str = "/System/etc/crond";

fn usage() {
    anyos_std::println!("Usage: crontab [-l] [-r] [-e] [file]");
    anyos_std::println!("  -l        List crontab entries");
    anyos_std::println!("  -r        Remove crontab");
    anyos_std::println!("  -e        Edit crontab (opens in vi)");
    anyos_std::println!("  file      Install crontab from file");
    anyos_std::println!("");
    anyos_std::println!("Crontab format:");
    anyos_std::println!("  minute hour day month weekday command");
    anyos_std::println!("  *      *    *   *     *       /System/bin/echo hello");
    anyos_std::println!("  */5    *    *   *     *       /System/bin/date");
    anyos_std::println!("  0      12   *   *     1-5     /System/bin/echo lunch");
}

/// Get the crontab file path for the current user.
fn user_crontab() -> String {
    // Use UID-based naming; default to "root" for uid 0
    let uid = anyos_std::process::getuid();
    format!("{}/{}", CRONTAB_DIR, uid)
}

/// List all entries from the user's crontab.
fn list_crontab() {
    let path = user_crontab();
    match anyos_std::fs::read_to_string(&path) {
        Ok(content) => {
            if content.is_empty() {
                anyos_std::println!("crontab: no crontab for current user");
            } else {
                anyos_std::print!("{}", content);
            }
        }
        Err(_) => {
            anyos_std::println!("crontab: no crontab for current user");
        }
    }
}

/// Remove the user's crontab.
fn remove_crontab() {
    let path = user_crontab();
    if anyos_std::fs::unlink(&path) == u32::MAX {
        anyos_std::println!("crontab: no crontab to remove");
    } else {
        anyos_std::println!("crontab: removed");
    }
}

/// Edit the crontab using vi.
fn edit_crontab() {
    let path = user_crontab();
    // Ensure directory and file exist
    anyos_std::fs::mkdir(CRONTAB_DIR);
    // Create file if it doesn't exist
    let fd = anyos_std::fs::open(&path, 4); // O_CREATE
    if fd != u32::MAX {
        anyos_std::fs::close(fd);
    }
    // Open in vi
    anyos_std::process::exec("/System/bin/vi", &path);
}

/// Install a crontab from a file.
fn install_crontab(file: &str) {
    match anyos_std::fs::read_to_string(file) {
        Ok(content) => {
            // Validate: count non-empty, non-comment lines
            let mut valid_lines = 0u32;
            for line in content.split('\n') {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    valid_lines += 1;
                }
            }

            // Ensure directory exists
            anyos_std::fs::mkdir(CRONTAB_DIR);

            // Write to user's crontab file
            let path = user_crontab();
            if anyos_std::fs::write_bytes(&path, content.as_bytes()).is_ok() {
                anyos_std::println!("crontab: installed {} entries", valid_lines);
            } else {
                anyos_std::println!("crontab: failed to write {}", path);
            }
        }
        Err(_) => {
            anyos_std::println!("crontab: cannot open '{}'", file);
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.has(b'h') {
        usage();
        return;
    }

    if args.has(b'l') {
        list_crontab();
    } else if args.has(b'r') {
        remove_crontab();
    } else if args.has(b'e') {
        edit_crontab();
    } else if args.pos_count > 0 {
        install_crontab(args.positional[0]);
    } else {
        usage();
    }
}
