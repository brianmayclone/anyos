#![no_std]
#![no_main]

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

anyos_std::entry!(main);

// ─── Address ────────────────────────────────────────────────────────────────

/// A line address for sed commands.
enum Address {
    /// No address — applies to every line.
    None,
    /// Single line number.
    Line(u32),
    /// Last line (`$`).
    Last,
    /// Lines matching a pattern (between /pattern/).
    Pattern(Vec<u8>),
    /// Line range: start,end.
    Range(Box<Address>, Box<Address>),
}

impl Address {
    fn matches(&self, line_no: u32, line: &[u8], is_last: bool) -> bool {
        match self {
            Address::None => true,
            Address::Line(n) => line_no == *n,
            Address::Last => is_last,
            Address::Pattern(pat) => find_bytes(line, pat).is_some(),
            Address::Range(start, end) => {
                start.matches(line_no, line, is_last) || end.matches(line_no, line, is_last)
            }
        }
    }
}

// ─── Commands ───────────────────────────────────────────────────────────────

enum Command {
    /// s/pattern/replacement/flags
    Substitute {
        addr: Address,
        pattern: Vec<u8>,
        replacement: Vec<u8>,
        global: bool,
    },
    /// d — delete line
    Delete { addr: Address },
    /// p — print line
    Print { addr: Address },
    /// q — quit
    Quit { addr: Address },
    /// a\ text — append text after line
    Append { addr: Address, text: String },
    /// i\ text — insert text before line
    Insert { addr: Address, text: String },
    /// y/src/dst/ — transliterate characters
    Transliterate {
        addr: Address,
        src: Vec<u8>,
        dst: Vec<u8>,
    },
}

// ─── Byte Search ────────────────────────────────────────────────────────────

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

// ─── Parsing ────────────────────────────────────────────────────────────────

fn parse_u32(s: &[u8]) -> (u32, usize) {
    let mut val = 0u32;
    let mut i = 0;
    while i < s.len() && s[i] >= b'0' && s[i] <= b'9' {
        val = val.saturating_mul(10).saturating_add((s[i] - b'0') as u32);
        i += 1;
    }
    (val, i)
}

/// Parse an address from the beginning of `s`. Returns (Address, bytes consumed).
fn parse_address(s: &[u8]) -> (Address, usize) {
    if s.is_empty() {
        return (Address::None, 0);
    }

    let (first, consumed) = parse_single_address(s);
    if matches!(first, Address::None) {
        return (Address::None, 0);
    }

    // Check for range: addr1,addr2
    if consumed < s.len() && s[consumed] == b',' {
        let (second, consumed2) = parse_single_address(&s[consumed + 1..]);
        if !matches!(second, Address::None) {
            return (
                Address::Range(Box::new(first), Box::new(second)),
                consumed + 1 + consumed2,
            );
        }
    }

    (first, consumed)
}

fn parse_single_address(s: &[u8]) -> (Address, usize) {
    if s.is_empty() {
        return (Address::None, 0);
    }

    if s[0] == b'$' {
        return (Address::Last, 1);
    }

    if s[0] >= b'0' && s[0] <= b'9' {
        let (n, consumed) = parse_u32(s);
        return (Address::Line(n), consumed);
    }

    if s[0] == b'/' {
        // /pattern/
        let mut i = 1;
        while i < s.len() && s[i] != b'/' {
            if s[i] == b'\\' && i + 1 < s.len() {
                i += 2; // skip escaped char
            } else {
                i += 1;
            }
        }
        let pattern = s[1..i].to_vec();
        let end = if i < s.len() { i + 1 } else { i };
        return (Address::Pattern(pattern), end);
    }

    (Address::None, 0)
}

/// Extract delimited text: given `s` starting right after delimiter,
/// read until `delim` (handling `\delim` escapes). Returns (content, bytes consumed including closing delim).
fn extract_delimited(s: &[u8], delim: u8) -> (Vec<u8>, usize) {
    let mut result = Vec::new();
    let mut i = 0;
    while i < s.len() && s[i] != delim {
        if s[i] == b'\\' && i + 1 < s.len() {
            let next = s[i + 1];
            if next == delim {
                result.push(delim);
                i += 2;
            } else if next == b'n' {
                result.push(b'\n');
                i += 2;
            } else if next == b't' {
                result.push(b'\t');
                i += 2;
            } else if next == b'\\' {
                result.push(b'\\');
                i += 2;
            } else {
                result.push(b'\\');
                result.push(next);
                i += 2;
            }
        } else {
            result.push(s[i]);
            i += 1;
        }
    }
    let consumed = if i < s.len() { i + 1 } else { i }; // skip closing delimiter
    (result, consumed)
}

/// Parse a single sed command expression.
fn parse_command(expr: &[u8]) -> Option<Command> {
    let (addr, mut pos) = parse_address(expr);

    // Skip whitespace between address and command
    while pos < expr.len() && (expr[pos] == b' ' || expr[pos] == b'\t') {
        pos += 1;
    }

    if pos >= expr.len() {
        return None;
    }

    let cmd_char = expr[pos];
    pos += 1;

    match cmd_char {
        b's' => {
            // s/pattern/replacement/[flags]
            if pos >= expr.len() {
                return None;
            }
            let delim = expr[pos];
            pos += 1;

            let (pattern, consumed1) = extract_delimited(&expr[pos..], delim);
            pos += consumed1;

            let (replacement, consumed2) = extract_delimited(&expr[pos..], delim);
            pos += consumed2;

            // Parse flags
            let mut global = false;
            while pos < expr.len() {
                match expr[pos] {
                    b'g' => global = true,
                    _ => break,
                }
                pos += 1;
            }

            Some(Command::Substitute {
                addr,
                pattern,
                replacement,
                global,
            })
        }
        b'd' => Some(Command::Delete { addr }),
        b'p' => Some(Command::Print { addr }),
        b'q' => Some(Command::Quit { addr }),
        b'a' => {
            // a\ text  or  a text
            if pos < expr.len() && expr[pos] == b'\\' {
                pos += 1;
            }
            while pos < expr.len() && (expr[pos] == b' ' || expr[pos] == b'\t') {
                pos += 1;
            }
            let text = core::str::from_utf8(&expr[pos..]).unwrap_or("");
            Some(Command::Append {
                addr,
                text: String::from(text),
            })
        }
        b'i' => {
            // i\ text
            if pos < expr.len() && expr[pos] == b'\\' {
                pos += 1;
            }
            while pos < expr.len() && (expr[pos] == b' ' || expr[pos] == b'\t') {
                pos += 1;
            }
            let text = core::str::from_utf8(&expr[pos..]).unwrap_or("");
            Some(Command::Insert {
                addr,
                text: String::from(text),
            })
        }
        b'y' => {
            // y/src/dst/
            if pos >= expr.len() {
                return None;
            }
            let delim = expr[pos];
            pos += 1;
            let (src, consumed1) = extract_delimited(&expr[pos..], delim);
            pos += consumed1;
            let (dst, consumed2) = extract_delimited(&expr[pos..], delim);
            pos += consumed2;
            let _ = pos;
            if src.len() != dst.len() {
                anyos_std::println!("sed: y: source and dest must be same length");
                return None;
            }
            Some(Command::Transliterate { addr, src, dst })
        }
        _ => {
            anyos_std::println!("sed: unknown command: '{}'", cmd_char as char);
            None
        }
    }
}

/// Parse all commands from a sed expression string (`;`-separated or from `-e` args).
fn parse_commands(expr: &str) -> Vec<Command> {
    let mut commands = Vec::new();
    for part in expr.split(';') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(cmd) = parse_command(trimmed.as_bytes()) {
            commands.push(cmd);
        }
    }
    commands
}

// ─── Substitution ───────────────────────────────────────────────────────────

/// Perform substitution on a line. Returns the modified line.
fn substitute(line: &[u8], pattern: &[u8], replacement: &[u8], global: bool) -> Vec<u8> {
    if pattern.is_empty() {
        return line.to_vec();
    }

    let mut result = Vec::with_capacity(line.len());
    let mut pos = 0;

    loop {
        match find_bytes(&line[pos..], pattern) {
            Some(offset) => {
                // Copy everything before the match
                result.extend_from_slice(&line[pos..pos + offset]);
                // Insert replacement (handle & for matched text)
                for &b in replacement {
                    if b == b'&' {
                        result.extend_from_slice(pattern);
                    } else {
                        result.push(b);
                    }
                }
                pos += offset + pattern.len();
                if !global {
                    // Only first match
                    result.extend_from_slice(&line[pos..]);
                    return result;
                }
            }
            None => {
                result.extend_from_slice(&line[pos..]);
                break;
            }
        }
    }

    result
}

/// Transliterate bytes in a line (y command).
fn transliterate(line: &[u8], src: &[u8], dst: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(line.len());
    for &b in line {
        let mut replaced = false;
        for j in 0..src.len() {
            if b == src[j] {
                result.push(dst[j]);
                replaced = true;
                break;
            }
        }
        if !replaced {
            result.push(b);
        }
    }
    result
}

// ─── Execution ──────────────────────────────────────────────────────────────

fn read_file(fd: u32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    data
}

fn print_bytes(data: &[u8]) {
    // Write raw bytes to stdout
    anyos_std::fs::write(1, data);
}

fn run_sed(data: &[u8], commands: &[Command], suppress: bool) {
    // Split into lines
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            lines.push(&data[start..i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        lines.push(&data[start..]);
    }

    let total = lines.len() as u32;

    // Track range state per command (is the range currently active?)
    let mut range_active = alloc::vec![false; commands.len()];

    for (idx, &line) in lines.iter().enumerate() {
        let line_no = (idx as u32) + 1;
        let is_last = line_no == total;
        let mut current: Vec<u8> = line.to_vec();
        let mut deleted = false;
        let mut extra_before: Option<String> = None;
        let mut extra_after: Option<String> = None;
        let mut quit = false;

        for (ci, cmd) in commands.iter().enumerate() {
            let (addr, action) = match cmd {
                Command::Substitute { addr, .. } => (addr, 's'),
                Command::Delete { addr } => (addr, 'd'),
                Command::Print { addr } => (addr, 'p'),
                Command::Quit { addr } => (addr, 'q'),
                Command::Append { addr, .. } => (addr, 'a'),
                Command::Insert { addr, .. } => (addr, 'i'),
                Command::Transliterate { addr, .. } => (addr, 'y'),
            };

            // Range matching logic
            let matched = match addr {
                Address::Range(start_addr, end_addr) => {
                    if range_active[ci] {
                        // Already in range, check if we've hit the end
                        if end_addr.matches(line_no, &current, is_last) {
                            range_active[ci] = false;
                        }
                        true
                    } else if start_addr.matches(line_no, &current, is_last) {
                        range_active[ci] = true;
                        true
                    } else {
                        false
                    }
                }
                other => other.matches(line_no, &current, is_last),
            };

            if !matched {
                continue;
            }

            match (action, cmd) {
                ('s', Command::Substitute { pattern, replacement, global, .. }) => {
                    current = substitute(&current, pattern, replacement, *global);
                }
                ('d', _) => {
                    deleted = true;
                    break;
                }
                ('p', _) => {
                    print_bytes(&current);
                    print_bytes(b"\n");
                }
                ('q', _) => {
                    quit = true;
                }
                ('a', Command::Append { text, .. }) => {
                    extra_after = Some(text.clone());
                }
                ('i', Command::Insert { text, .. }) => {
                    extra_before = Some(text.clone());
                }
                ('y', Command::Transliterate { src, dst, .. }) => {
                    current = transliterate(&current, src, dst);
                }
                _ => {}
            }
        }

        if let Some(text) = extra_before {
            anyos_std::println!("{}", text);
        }

        if !deleted && !suppress {
            print_bytes(&current);
            print_bytes(b"\n");
        }

        if let Some(text) = extra_after {
            anyos_std::println!("{}", text);
        }

        if quit {
            return;
        }
    }
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"ef");

    if args.has(b'h') || (args.pos_count == 0 && args.opt(b'e').is_none() && args.opt(b'f').is_none()) {
        anyos_std::println!("Usage: sed [-n] [-e script] [-f file] [script] [input...]");
        anyos_std::println!("Commands:");
        anyos_std::println!("  s/pat/rep/[g]  Substitute pattern");
        anyos_std::println!("  d              Delete line");
        anyos_std::println!("  p              Print line");
        anyos_std::println!("  q              Quit");
        anyos_std::println!("  a\\ text        Append text after line");
        anyos_std::println!("  i\\ text        Insert text before line");
        anyos_std::println!("  y/src/dst/     Transliterate characters");
        anyos_std::println!("Addresses:");
        anyos_std::println!("  N              Line number");
        anyos_std::println!("  $              Last line");
        anyos_std::println!("  /pattern/      Lines matching pattern");
        anyos_std::println!("  N,M            Range");
        return;
    }

    let suppress = args.has(b'n');

    // Collect commands
    let mut commands = Vec::new();

    // -e script
    if let Some(expr) = args.opt(b'e') {
        commands.extend(parse_commands(expr));
    }

    // -f script-file
    if let Some(file) = args.opt(b'f') {
        if let Ok(content) = anyos_std::fs::read_to_string(file) {
            for line in content.split('\n') {
                let line = line.trim();
                if !line.is_empty() {
                    commands.extend(parse_commands(line));
                }
            }
        } else {
            anyos_std::println!("sed: cannot read script file '{}'", file);
            return;
        }
    }

    // Determine positional argument layout:
    // If no -e or -f was given, first positional is the script
    let mut file_start = 0;
    if commands.is_empty() {
        if args.pos_count == 0 {
            anyos_std::println!("sed: no script specified");
            return;
        }
        commands = parse_commands(args.positional[0]);
        file_start = 1;
    }

    if commands.is_empty() {
        anyos_std::println!("sed: no valid commands");
        return;
    }

    // Process files, or stdin if none given
    if file_start >= args.pos_count {
        // Read from stdin
        let data = read_file(0);
        run_sed(&data, &commands, suppress);
    } else {
        for i in file_start..args.pos_count {
            let path = args.positional[i];
            let fd = anyos_std::fs::open(path, 0);
            if fd == u32::MAX {
                anyos_std::println!("sed: {}: No such file or directory", path);
                continue;
            }
            let data = read_file(fd);
            anyos_std::fs::close(fd);
            run_sed(&data, &commands, suppress);
        }
    }
}
