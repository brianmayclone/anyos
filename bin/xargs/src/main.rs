#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::fs;
use anyos_std::process;
use anyos_std::format;

anyos_std::entry!(main);

/// Read all available data from a file descriptor into a Vec<u8>.
fn read_all(fd: u32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    data
}

/// Resolve a command name via PATH.
fn resolve_command(cmd: &str) -> String {
    if cmd.starts_with('/') || cmd.starts_with("./") {
        return String::from(cmd);
    }
    let mut path_buf = [0u8; 256];
    let len = anyos_std::env::get("PATH", &mut path_buf);
    if len != u32::MAX {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..len as usize]) {
            let mut stat_buf = [0u32; 7];
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() { continue; }
                let candidate = format!("{}/{}", dir, cmd);
                if fs::stat(&candidate, &mut stat_buf) == 0 && stat_buf[0] == 0 {
                    return candidate;
                }
            }
        }
    }
    format!("/System/bin/{}", cmd)
}

/// Split input into items by whitespace, respecting single and double quotes.
fn split_items(input: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if ch == '\'' || ch == '"' {
            let quote = ch;
            chars.next(); // consume opening quote
            let inner_start = start + 1;
            let mut end = inner_start;
            while let Some(&(i, c)) = chars.peek() {
                if c == quote {
                    end = i;
                    chars.next(); // consume closing quote
                    break;
                }
                end = i + c.len_utf8();
                chars.next();
            }
            let item = &input[inner_start..end];
            if !item.is_empty() {
                items.push(item);
            }
        } else {
            let mut end = start;
            while let Some(&(i, c)) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                end = i + c.len_utf8();
                chars.next();
            }
            let item = &input[start..end];
            if !item.is_empty() {
                items.push(item);
            }
        }
    }

    items
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);
    let parsed = anyos_std::args::parse(args_str, b"ndIPs");

    // Options:
    // -n N  max args per command invocation
    // -d D  delimiter (single char)
    // -I R  replace string (like -I{})
    // -P N  max parallel processes (simplified: we run sequentially)
    // -0    null delimiter (not applicable without binary stdin)

    let max_args = parsed.opt_u32(b'n', 0) as usize; // 0 = unlimited
    let replace_str = parsed.opt(b'I');
    let delimiter = parsed.opt(b'd');

    // The command is the first positional arg (or "echo" by default)
    let base_cmd = if parsed.pos_count > 0 {
        parsed.pos(0).unwrap_or("echo")
    } else {
        "echo"
    };
    // Additional fixed args from positional args
    let mut fixed_args: Vec<&str> = Vec::new();
    for i in 1..parsed.pos_count {
        if let Some(a) = parsed.pos(i) {
            fixed_args.push(a);
        }
    }

    // Read all stdin
    let stdin_data = read_all(0);
    let stdin_str = match core::str::from_utf8(&stdin_data) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("xargs: invalid UTF-8 on stdin");
            return;
        }
    };

    // Split input into items
    let items = if let Some(d) = delimiter {
        let d_char = d.chars().next().unwrap_or('\n');
        stdin_str.split(d_char)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
    } else {
        // Default: split by whitespace/newlines
        split_items(stdin_str)
    };

    if items.is_empty() {
        return;
    }

    let cmd_path = resolve_command(base_cmd);

    if let Some(repl) = replace_str {
        // -I mode: run command once per item, replacing {} in args
        for item in &items {
            let mut full = String::from(base_cmd);
            for arg in &fixed_args {
                full.push(' ');
                if arg.contains(repl) {
                    let replaced = arg.replace(repl, item);
                    full.push_str(&replaced);
                } else {
                    full.push_str(arg);
                }
            }
            let tid = process::spawn(&cmd_path, &full);
            if tid != u32::MAX {
                process::waitpid(tid);
            }
        }
    } else if max_args > 0 {
        // -n mode: batch items in groups of max_args
        let mut i = 0;
        while i < items.len() {
            let end = (i + max_args).min(items.len());
            let batch = &items[i..end];

            let mut full = String::from(base_cmd);
            for arg in &fixed_args {
                full.push(' ');
                full.push_str(arg);
            }
            for item in batch {
                full.push(' ');
                full.push_str(item);
            }

            let tid = process::spawn(&cmd_path, &full);
            if tid != u32::MAX {
                process::waitpid(tid);
            }
            i = end;
        }
    } else {
        // Default: pass all items as arguments to a single invocation
        let mut full = String::from(base_cmd);
        for arg in &fixed_args {
            full.push(' ');
            full.push_str(arg);
        }
        for item in &items {
            full.push(' ');
            full.push_str(item);
        }

        let tid = process::spawn(&cmd_path, &full);
        if tid != u32::MAX {
            process::waitpid(tid);
        }
    }
}
