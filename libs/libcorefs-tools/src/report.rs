// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Structured report rendering for the CoreFS CLI tools.
//!
//! Every tool produces a report value (e.g. `MkfsReport`, `FsckReport`,
//! `ScrubReport`) and is expected to offer both a **human-readable**
//! plain-text rendering and a **machine-parseable** JSON rendering
//! (selected via `--json`).
//!
//! To avoid pulling in `serde_json` — which does not compile on the
//! stock `x86_64-anyos-user.json` target without a bunch of `std`
//! patching — this module provides a minimal [`JsonBuilder`] that
//! mirrors the subset of the JSON grammar we actually need: objects,
//! arrays, strings, signed/unsigned numbers, booleans, and `null`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Trait implemented by every tool report.
pub trait Report {
    /// Short one-line summary (used by the `summary` subcommand).
    fn summary(&self) -> String;
    /// Multi-line plain-text rendering for humans.
    fn render_text(&self) -> String;
    /// Machine-parseable JSON rendering.
    fn render_json(&self) -> String;
}

/// Write a report to stdout in plain-text form.
#[cfg(target_os = "none")]
pub fn print_report_text<R: Report>(report: &R) {
    anyos_std::println!("{}", report.render_text());
}

/// Write a report to stdout as a JSON document.
#[cfg(target_os = "none")]
pub fn print_report_json<R: Report>(report: &R) {
    anyos_std::println!("{}", report.render_json());
}

/// Emit either text or JSON depending on the `json` flag.
#[cfg(target_os = "none")]
pub fn print_report<R: Report>(report: &R, json: bool) {
    if json {
        print_report_json(report);
    } else {
        print_report_text(report);
    }
}

/// Host-side placeholder used by `cargo check` for anyOS-only tools.
#[cfg(not(target_os = "none"))]
pub fn print_report<R: Report>(_report: &R, _json: bool) {}

// ---------------------------------------------------------------------------
// JsonBuilder — minimal hand-rolled JSON writer.
// ---------------------------------------------------------------------------

/// Incremental JSON writer.
///
/// Keeps track of the current scope (object / array) to emit the
/// correct commas and delimiters.  The caller is responsible for
/// balancing `begin_*` and `end_*` calls.
pub struct JsonBuilder {
    out: String,
    scope: Vec<Scope>,
}

#[derive(Copy, Clone, Debug)]
struct Scope {
    kind: ScopeKind,
    count: usize,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ScopeKind {
    Object,
    Array,
}

impl Default for JsonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonBuilder {
    pub fn new() -> Self {
        Self {
            out: String::new(),
            scope: Vec::new(),
        }
    }

    pub fn finish(self) -> String {
        self.out
    }

    pub fn begin_object(&mut self) {
        self.separator_if_needed();
        self.out.push('{');
        self.scope.push(Scope {
            kind: ScopeKind::Object,
            count: 0,
        });
    }

    pub fn end_object(&mut self) {
        debug_assert!(matches!(
            self.scope.last(),
            Some(Scope {
                kind: ScopeKind::Object,
                ..
            })
        ));
        self.scope.pop();
        self.out.push('}');
    }

    pub fn begin_array(&mut self) {
        self.separator_if_needed();
        self.out.push('[');
        self.scope.push(Scope {
            kind: ScopeKind::Array,
            count: 0,
        });
    }

    pub fn end_array(&mut self) {
        debug_assert!(matches!(
            self.scope.last(),
            Some(Scope {
                kind: ScopeKind::Array,
                ..
            })
        ));
        self.scope.pop();
        self.out.push(']');
    }

    pub fn key(&mut self, name: &str) {
        // Emit "," between sibling key/value pairs.
        if let Some(top) = self.scope.last_mut() {
            debug_assert_eq!(top.kind, ScopeKind::Object);
            if top.count > 0 {
                self.out.push(',');
            }
            top.count += 1;
        }
        escape_string(&mut self.out, name);
        self.out.push(':');
    }

    fn separator_if_needed(&mut self) {
        if let Some(top) = self.scope.last_mut() {
            if top.kind == ScopeKind::Array {
                if top.count > 0 {
                    self.out.push(',');
                }
                top.count += 1;
            }
            // Objects rely on explicit key() calls to manage commas.
        }
    }

    // -- value helpers ------------------------------------------------------

    pub fn string(&mut self, value: &str) {
        self.separator_if_needed();
        escape_string(&mut self.out, value);
    }

    pub fn u64(&mut self, value: u64) {
        self.separator_if_needed();
        self.out.push_str(&value.to_string());
    }

    pub fn i64(&mut self, value: i64) {
        self.separator_if_needed();
        self.out.push_str(&value.to_string());
    }

    pub fn bool(&mut self, value: bool) {
        self.separator_if_needed();
        self.out.push_str(if value { "true" } else { "false" });
    }

    pub fn null(&mut self) {
        self.separator_if_needed();
        self.out.push_str("null");
    }

    // -- convenience: key+value one-shots -----------------------------------

    pub fn kv_string(&mut self, name: &str, value: &str) {
        self.key(name);
        escape_string(&mut self.out, value);
    }

    pub fn kv_u64(&mut self, name: &str, value: u64) {
        self.key(name);
        self.out.push_str(&value.to_string());
    }

    pub fn kv_i64(&mut self, name: &str, value: i64) {
        self.key(name);
        self.out.push_str(&value.to_string());
    }

    pub fn kv_bool(&mut self, name: &str, value: bool) {
        self.key(name);
        self.out.push_str(if value { "true" } else { "false" });
    }
}

fn escape_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object() {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.end_object();
        assert_eq!(b.finish(), "{}");
    }

    #[test]
    fn flat_object() {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_string("label", "root");
        b.kv_u64("size", 1024);
        b.kv_bool("clean", true);
        b.end_object();
        assert_eq!(b.finish(), r#"{"label":"root","size":1024,"clean":true}"#);
    }

    #[test]
    fn nested_structures() {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.key("issues");
        b.begin_array();
        b.begin_object();
        b.kv_string("severity", "warning");
        b.kv_string("message", "bitmap drift");
        b.end_object();
        b.begin_object();
        b.kv_string("severity", "error");
        b.kv_string("message", "bad crc");
        b.end_object();
        b.end_array();
        b.kv_u64("total", 2);
        b.end_object();
        let s = b.finish();
        assert!(s.starts_with(r#"{"issues":[{"severity":"warning""#));
        assert!(s.ends_with(r#""total":2}"#));
    }

    #[test]
    fn escapes_special_chars() {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_string("msg", "line1\nline2\t\"quoted\"");
        b.end_object();
        assert_eq!(b.finish(), r#"{"msg":"line1\nline2\t\"quoted\""}"#);
    }
}
