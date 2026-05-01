// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Long-option CLI parser tailored for the CoreFS userspace tools.
//!
//! The short-flag parser in [`anyos_std::args`] covers classic POSIX
//! syntax (`-l`, `-n 5`, …) but the CoreFS tools use GNU-style long
//! options exclusively (`--device 0`, `--capacity 16777216`,
//! `--json`, `--repair`, `--mode full`, …).  This module provides a
//! minimal, allocation-free parser for that dialect.
//!
//! # Grammar
//!
//! ```text
//! args   := token*
//! token  := flag | option | positional
//! flag   := "--name"
//! option := "--name" value         (value is the next token, *not* starting with `-`)
//! option := "--name=value"
//! ```
//!
//! Subcommands and free positionals are simply the tokens that are not
//! consumed as `--flag` / `--option`.

use alloc::string::String;
use alloc::vec::Vec;

/// A parsed command line.
#[derive(Debug, Default, Clone)]
pub struct LongArgs {
    /// Parsed long options (name → value).  Flags without a value are
    /// stored as `(name, "")`.
    pub options: Vec<(String, String)>,
    /// Free-standing positional tokens (incl. subcommand names).
    pub positional: Vec<String>,
}

impl LongArgs {
    /// Retrieve the value of `--name` (or `None` if absent or boolean).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Return `true` if `--name` was given (with or without value).
    pub fn has(&self, name: &str) -> bool {
        self.options.iter().any(|(k, _)| k == name)
    }

    /// Parse a `u32` option (`--device 0`).
    pub fn get_u32(&self, name: &str) -> Option<u32> {
        self.get(name).and_then(parse_u64).map(|n| n as u32)
    }

    /// Parse a `u64` option (`--capacity 16777216`).
    pub fn get_u64(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(parse_u64)
    }

    /// Return the n-th positional token (0-indexed) if any.
    pub fn positional_at(&self, idx: usize) -> Option<&str> {
        self.positional.get(idx).map(|s| s.as_str())
    }
}

/// Parse a raw argument string into [`LongArgs`].
///
/// Token separator is ASCII whitespace; double-quoted and
/// single-quoted tokens are kept verbatim.
pub fn parse(raw: &str) -> LongArgs {
    let tokens = tokenize(raw);
    let mut out = LongArgs::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if let Some(body) = tok.strip_prefix("--") {
            if body.is_empty() {
                // '--' sentinel: remaining tokens are positional.
                for t in &tokens[i + 1..] {
                    out.positional.push(t.clone());
                }
                break;
            }
            if let Some(eq) = body.find('=') {
                let (name, value) = body.split_at(eq);
                out.options.push((name.into(), value[1..].into()));
                i += 1;
                continue;
            }
            // "--name" → look ahead for a non-flag value.
            if i + 1 < tokens.len() && !tokens[i + 1].starts_with('-') {
                out.options.push((body.into(), tokens[i + 1].clone()));
                i += 2;
                continue;
            }
            out.options.push((body.into(), String::new()));
            i += 1;
            continue;
        }
        out.positional.push(tok.clone());
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a decimal unsigned 64-bit integer.  Accepts an optional
/// `_` digit separator and the suffixes `k`/`K`, `m`/`M`, `g`/`G`,
/// `t`/`T` (powers of 1024) — useful for `--capacity 16M`.
pub fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (digits, multiplier): (&[u8], u64) = match bytes.last().copied() {
        Some(b'k') | Some(b'K') => (&bytes[..bytes.len() - 1], 1024),
        Some(b'm') | Some(b'M') => (&bytes[..bytes.len() - 1], 1024 * 1024),
        Some(b'g') | Some(b'G') => (&bytes[..bytes.len() - 1], 1024 * 1024 * 1024),
        Some(b't') | Some(b'T') => (&bytes[..bytes.len() - 1], 1024u64.pow(4)),
        _ => (bytes, 1),
    };
    let mut n: u64 = 0;
    for &b in digits {
        if b == b'_' {
            continue;
        }
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    n.checked_mul(multiplier)
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in raw.chars() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                cur.push(ch);
            }
            continue;
        }
        if in_double {
            if ch == '"' {
                in_double = false;
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '\'' => in_single = true,
            '"' => in_double = true,
            c if c.is_ascii_whitespace() => {
                if !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// Convenience helpers used by many tool binaries.
pub fn parse_device_id(args: &LongArgs) -> Option<u32> {
    args.get_u32("device")
}

pub fn parse_capacity(args: &LongArgs) -> Option<u64> {
    args.get_u64("capacity")
}

pub fn flag_present(args: &LongArgs, name: &str) -> bool {
    args.has(name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_option_with_space() {
        let a = parse("--device 0 --capacity 1024");
        assert_eq!(a.get("device"), Some("0"));
        assert_eq!(a.get_u32("device"), Some(0));
        assert_eq!(a.get_u64("capacity"), Some(1024));
    }

    #[test]
    fn parses_option_with_equals() {
        let a = parse("--device=3 --label=root");
        assert_eq!(a.get_u32("device"), Some(3));
        assert_eq!(a.get("label"), Some("root"));
    }

    #[test]
    fn boolean_flag_has_no_value() {
        let a = parse("--json --repair");
        assert!(a.has("json"));
        assert!(a.has("repair"));
        assert_eq!(a.get("json"), Some(""));
    }

    #[test]
    fn suffix_multipliers() {
        assert_eq!(parse_u64("16M"), Some(16 * 1024 * 1024));
        assert_eq!(parse_u64("2G"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_u64("1_000"), Some(1000));
    }

    #[test]
    fn subcommand_and_flags_mix() {
        let a = parse("superblock --device 0 --json");
        assert_eq!(a.positional_at(0), Some("superblock"));
        assert_eq!(a.get_u32("device"), Some(0));
        assert!(a.has("json"));
    }

    #[test]
    fn quoted_positional_preserved() {
        let a = parse("create --name \"my volume\"");
        assert_eq!(a.positional_at(0), Some("create"));
        assert_eq!(a.get("name"), Some("my volume"));
    }

    #[test]
    fn double_dash_stops_parsing() {
        let a = parse("--device 0 -- --not-a-flag");
        assert_eq!(a.get_u32("device"), Some(0));
        assert_eq!(a.positional_at(0), Some("--not-a-flag"));
    }

    #[test]
    fn helpers_work() {
        let a = parse("--device 7 --capacity 4k --json");
        assert_eq!(parse_device_id(&a), Some(7));
        assert_eq!(parse_capacity(&a), Some(4096));
        assert!(flag_present(&a, "json"));
        assert!(!flag_present(&a, "repair"));
    }
}
