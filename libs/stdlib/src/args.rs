//! Lightweight Linux-style argument parser for no_std programs.
//!
//! Supports short flags (`-l`, `-a`), combined flags (`-la`),
//! flags with values (`-n 10`), positional arguments, and `--` to stop parsing.
//!
//! # Example
//! ```ignore
//! let mut buf = [0u8; 256];
//! let raw = anyos_std::process::args(&mut buf);
//! let args = anyos_std::args::parse(raw, b"nc"); // -n and -c take values
//! let path = args.first_or(".");
//! let lines = args.opt_u32(b'n', 10);
//! if args.has(b'l') { /* long format */ }
//! ```

const MAX_FLAGS: usize = 16;
const MAX_POSITIONAL: usize = 32;
const MAX_OPTS: usize = 8;

/// Parsed command-line arguments.
pub struct ParsedArgs<'a> {
    flags: [u8; MAX_FLAGS],
    flag_count: usize,
    opts: [(u8, &'a str); MAX_OPTS],
    opt_count: usize,
    /// Positional (non-flag) arguments.
    pub positional: [&'a str; MAX_POSITIONAL],
    pub pos_count: usize,
}

impl<'a> ParsedArgs<'a> {
    /// Check if a short flag is set (e.g., `has(b'l')`).
    pub fn has(&self, flag: u8) -> bool {
        for i in 0..self.flag_count {
            if self.flags[i] == flag {
                return true;
            }
        }
        false
    }

    /// Get the value for an option flag (e.g., `opt(b'n')` -> `Some("10")`).
    pub fn opt(&self, flag: u8) -> Option<&'a str> {
        for i in 0..self.opt_count {
            if self.opts[i].0 == flag {
                return Some(self.opts[i].1);
            }
        }
        None
    }

    /// Get the first positional argument, or `default` if none.
    pub fn first_or(&self, default: &'a str) -> &'a str {
        if self.pos_count > 0 {
            self.positional[0]
        } else {
            default
        }
    }

    /// Get positional argument at `idx`, or `None`.
    pub fn pos(&self, idx: usize) -> Option<&'a str> {
        if idx < self.pos_count {
            Some(self.positional[idx])
        } else {
            None
        }
    }

    /// Parse a `u32` from an option value, or return `default`.
    pub fn opt_u32(&self, flag: u8, default: u32) -> u32 {
        match self.opt(flag) {
            Some(s) => parse_u32_str(s).unwrap_or(default),
            None => default,
        }
    }
}

/// Parse a decimal string to u32.
fn parse_u32_str(s: &str) -> Option<u32> {
    let mut n: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

/// Parse a raw argument string into flags, options, and positional args.
///
/// `opts_with_values` lists flag characters that consume the next token as a value.
///
/// # Examples
/// - `parse("-la file.txt", b"")` → flags `[l, a]`, positional `["file.txt"]`
/// - `parse("-n 5 file.txt", b"n")` → opts `[(n, "5")]`, positional `["file.txt"]`
/// - `parse("-- -f", b"")` → positional `["-f"]` (no flags after `--`)
pub fn parse<'a>(raw: &'a str, opts_with_values: &[u8]) -> ParsedArgs<'a> {
    let mut result = ParsedArgs {
        flags: [0; MAX_FLAGS],
        flag_count: 0,
        opts: [(0, ""); MAX_OPTS],
        opt_count: 0,
        positional: [""; MAX_POSITIONAL],
        pos_count: 0,
    };

    // Collect tokens into a fixed array for indexed access
    const MAX_TOKENS: usize = 32;
    let mut tokens: [&str; MAX_TOKENS] = [""; MAX_TOKENS];
    let mut token_count = 0;
    for token in raw.split_ascii_whitespace() {
        if token_count < MAX_TOKENS {
            tokens[token_count] = token;
            token_count += 1;
        }
    }

    let mut i = 0;
    let mut stop_flags = false;

    while i < token_count {
        let token = tokens[i];

        if stop_flags || !token.starts_with('-') || token.len() < 2 {
            // Positional argument (or single "-" which means stdin in some tools)
            if result.pos_count < MAX_POSITIONAL {
                result.positional[result.pos_count] = token;
                result.pos_count += 1;
            }
            i += 1;
            continue;
        }

        if token == "--" {
            stop_flags = true;
            i += 1;
            continue;
        }

        // Parse flags from this token (e.g., "-la" or "-n")
        let bytes = token.as_bytes();
        let mut j = 1; // skip the leading '-'
        while j < bytes.len() {
            let ch = bytes[j];

            if opts_with_values.contains(&ch) {
                // This flag takes a value
                if j + 1 < bytes.len() {
                    // Value is the rest of this token: e.g., "-n5"
                    if result.opt_count < MAX_OPTS {
                        let val_start = j + 1;
                        let val = core::str::from_utf8(&bytes[val_start..]).unwrap_or("");
                        result.opts[result.opt_count] = (ch, val);
                        result.opt_count += 1;
                    }
                    break; // consumed rest of token
                } else if i + 1 < token_count {
                    // Value is the next token: e.g., "-n 5"
                    if result.opt_count < MAX_OPTS {
                        result.opts[result.opt_count] = (ch, tokens[i + 1]);
                        result.opt_count += 1;
                    }
                    i += 1; // skip the value token
                    break;
                }
                // No value available — treat as a flag
                if result.flag_count < MAX_FLAGS {
                    result.flags[result.flag_count] = ch;
                    result.flag_count += 1;
                }
            } else {
                // Simple boolean flag
                if result.flag_count < MAX_FLAGS {
                    result.flags[result.flag_count] = ch;
                    result.flag_count += 1;
                }
            }
            j += 1;
        }

        i += 1;
    }

    result
}
