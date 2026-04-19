//! Shared POSIX-style shell utilities.
//!
//! Used by both `terminal` (GUI) and `textmode_console` (nogui) to avoid
//! duplicating tokenization, expansion, redirect parsing, and pipeline
//! execution logic.
//!
//! # Pipeline execution
//! [`run_pipeline`] spawns all commands in a `cmd1 | cmd2 | cmd3` pipeline,
//! wiring up intermediate named pipes, and returns the last process TID plus
//! the display pipe ID.  The caller is responsible for draining the display
//! pipe and closing all pipes when done.
//!
//! # Redirect parsing
//! [`parse_redirects`] and [`parse_input_redirect`] strip redirect operators
//! from a command line and return the cleaned command together with a
//! [`Redirect`] / [`InputRedirect`] descriptor.  Use [`write_redirect`] to
//! flush captured output to the target file.

use crate::String;
use crate::Vec;
use crate::format;
use crate::{env, fs, icons, ipc, process};

// ─── Public types ─────────────────────────────────────────────────────────────

/// Output redirect descriptor.
#[derive(Clone)]
pub struct Redirect {
    pub target: String,
    pub append: bool,
}

/// Input redirect descriptor.
#[derive(Clone)]
pub struct InputRedirect {
    pub source: String,
}

/// Result of spawning a pipeline.
pub struct PipelineResult {
    /// TID of the last process in the pipeline.
    pub last_tid: u32,
    /// Display pipe ID — drain this to get the combined stdout output.
    pub display_pipe: u32,
    /// Intermediate pipe IDs (must be closed when the pipeline exits).
    pub extra_pipes: Vec<u32>,
}

// ─── Tokenizer ────────────────────────────────────────────────────────────────

/// Tokenize a shell command line respecting POSIX quoting rules.
///
/// - Single quotes `'...'`: literal content, no escaping.
/// - Double quotes `"..."`: supports `\"` and `\\` inside.
/// - Backslash outside quotes: escapes the next character.
/// - Whitespace separates tokens.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b' ' || bytes[i] == b'\t' { i += 1; continue; }

        let mut token = String::new();
        while i < len && bytes[i] != b' ' && bytes[i] != b'\t' {
            if bytes[i] == b'\'' {
                i += 1;
                while i < len && bytes[i] != b'\'' { token.push(bytes[i] as char); i += 1; }
                if i < len { i += 1; }
            } else if bytes[i] == b'"' {
                i += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < len {
                        let next = bytes[i + 1];
                        if next == b'"' || next == b'\\' { token.push(next as char); i += 2; continue; }
                    }
                    token.push(bytes[i] as char);
                    i += 1;
                }
                if i < len { i += 1; }
            } else if bytes[i] == b'\\' && i + 1 < len {
                i += 1;
                token.push(bytes[i] as char);
                i += 1;
            } else {
                token.push(bytes[i] as char);
                i += 1;
            }
        }
        if !token.is_empty() { tokens.push(token); }
    }
    tokens
}

/// Re-join tokens into a shell-safe string (quote tokens containing spaces).
pub fn join(tokens: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for token in tokens {
        if token.contains(' ') || token.contains('\t') || token.contains('"') || token.contains('\'') {
            let mut q = String::with_capacity(token.len() + 2);
            q.push('"');
            for ch in token.chars() {
                if ch == '"' || ch == '\\' { q.push('\\'); }
                q.push(ch);
            }
            q.push('"');
            parts.push(q);
        } else {
            parts.push(token.clone());
        }
    }
    parts.join(" ")
}

// ─── Redirect parsing ────────────────────────────────────────────────────────

/// Strip output redirect operators from `line` and return (clean_cmd, redirect).
///
/// Handles: `>`, `>>`, `2>`, `2>>`, `1>`, `1>>`, `2>&1` (merged to stdout).
/// Relative paths are resolved against `cwd`.
pub fn parse_redirects(line: &str, cwd: &str) -> (String, Option<Redirect>) {
    // Check longer patterns first to avoid partial matches
    let patterns: &[(&str, bool)] = &[
        ("2>>", true),
        ("1>>", true),
        ("2>&1", false), // merge stderr to stdout → /dev/null redirect
        ("2>",  false),
        ("1>",  false),
        (">>",  true),
        (">",   false),
    ];

    for &(pattern, is_append) in patterns {
        if let Some(pos) = line.find(pattern) {
            // Don't match inside a token that starts with a letter/digit differently
            // (basic check: char before must be whitespace, start-of-line, or a digit)
            let cmd_part = line[..pos].trim();
            let after = &line[pos + pattern.len()..];

            // 2>&1: redirect stderr to stdout — treat as /dev/null for display
            if pattern == "2>&1" {
                return (String::from(cmd_part), Some(Redirect { target: String::from("/dev/null"), append: false }));
            }

            let target = after.split_whitespace().next().unwrap_or("");
            if target.is_empty() { continue; }

            let abs_target = resolve_redirect_path(target, cwd);
            // Anything after "operator target" is part of cmd (shouldn't happen but handle it)
            let rest = after[target.len()..].trim();
            let clean = if rest.is_empty() {
                String::from(cmd_part)
            } else {
                format!("{} {}", cmd_part, rest)
            };
            return (clean, Some(Redirect { target: abs_target, append: is_append }));
        }
    }
    (String::from(line), None)
}

/// Strip input redirect `< file` from `line`. Returns (clean_cmd, redirect).
pub fn parse_input_redirect(line: &str, cwd: &str) -> (String, Option<InputRedirect>) {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'<' { continue; }
        // Skip `<<` (here-doc) and `<(`
        if i + 1 < bytes.len() && (bytes[i + 1] == b'<' || bytes[i + 1] == b'(') { continue; }
        // Skip `2<` etc.
        if i > 0 && bytes[i - 1] >= b'0' && bytes[i - 1] <= b'9' { continue; }

        let cmd_part = line[..i].trim();
        let target_part = line[i + 1..].trim();
        let target = target_part.split_whitespace().next().unwrap_or("");
        if target.is_empty() { continue; }
        let abs = resolve_redirect_path(target, cwd);
        let rest = target_part[target.len()..].trim();
        let clean = if rest.is_empty() {
            String::from(cmd_part)
        } else {
            format!("{} {}", cmd_part, rest)
        };
        return (clean, Some(InputRedirect { source: abs }));
    }
    (String::from(line), None)
}

fn resolve_redirect_path(target: &str, cwd: &str) -> String {
    if target.starts_with('/') {
        String::from(target)
    } else if cwd == "/" {
        format!("/{}", target)
    } else {
        format!("{}/{}", cwd, target)
    }
}

/// Write data to a redirect target file. On first `>` call the file is
/// truncated; subsequent calls (after `append` flips to true) append.
pub fn write_redirect(redirect: &mut Redirect, data: &str) {
    if redirect.target == "/dev/null" { return; }
    if redirect.append {
        let existing = fs::read_to_string(&redirect.target).unwrap_or_default();
        let combined = format!("{}{}", existing, data);
        let _ = fs::write_bytes(&redirect.target, combined.as_bytes());
    } else {
        let _ = fs::write_bytes(&redirect.target, data.as_bytes());
        redirect.append = true; // next chunk appends
    }
}

// ─── PATH resolution ─────────────────────────────────────────────────────────

/// Search PATH env var for `cmd`. Returns the full path or None.
pub fn resolve_from_path(cmd: &str) -> Option<String> {
    let mut path_buf = [0u8; 256];
    let len = env::get("PATH", &mut path_buf);
    if len == u32::MAX { return None; }
    let path_str = core::str::from_utf8(&path_buf[..len as usize]).ok()?;
    let mut stat_buf = [0u32; 7];
    for dir in path_str.split(':') {
        let dir = dir.trim();
        if dir.is_empty() { continue; }
        let candidate = format!("{}/{}", dir, cmd);
        if fs::stat(&candidate, &mut stat_buf) == 0 && stat_buf[0] == 0 {
            return Some(candidate);
        }
    }
    None
}

/// Search /Applications/ for an app bundle matching `cmd` (case-insensitive).
/// Returns the full path to the executable inside the bundle, e.g.
/// `/Applications/Notepad.app/Notepad`, or None if not found.
pub fn resolve_from_applications(cmd: &str) -> Option<String> {
    let bundle = icons::find_app_bundle_by_stem(cmd)?;
    let stem = bundle
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".app"))?;
    let candidate = format!("{}/{}", bundle, stem);
    let mut stat_buf = [0u32; 7];
    if fs::stat(&candidate, &mut stat_buf) == 0 && stat_buf[0] == 0 {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve command to full path (absolute, relative ./, or PATH search).
pub fn resolve_cmd_path(cmd: &str, cwd: &str) -> String {
    if cmd.starts_with('/') {
        String::from(cmd)
    } else if cmd.starts_with("./") || cmd.starts_with("../") {
        let rel = cmd.strip_prefix("./").unwrap_or(cmd);
        if cwd == "/" { format!("/{}", rel) } else { format!("{}/{}", cwd, rel) }
    } else {
        if let Some(p) = resolve_from_path(cmd) {
            return p;
        }
        if let Some(p) = resolve_from_applications(cmd) {
            return p;
        }
        // Check /System/sbin/ for admin tools (fdisk, mkfs.corefs, etc.)
        let sbin_path = format!("/System/sbin/{}", cmd);
        {
            let mut st = [0u32; 7];
            if crate::fs::stat(&sbin_path, &mut st) == 0 {
                return sbin_path;
            }
        }
        format!("/System/bin/{}", cmd)
    }
}

// ─── Variable & tilde expansion ──────────────────────────────────────────────

/// Expand `~` or `~/` to the HOME env var value.
pub fn expand_tilde(token: &str) -> String {
    if !token.starts_with('~') { return String::from(token); }
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    if hlen == u32::MAX || hlen == 0 { return String::from(token); }
    let home = core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("");
    if token.len() == 1 {
        String::from(home)
    } else if token.as_bytes()[1] == b'/' {
        format!("{}{}", home, &token[1..])
    } else {
        String::from(token)
    }
}

/// Expand `$VAR`, `${VAR}`, `$(cmd)`, `` `cmd` ``, `$$`, `$?` in a single token.
pub fn expand_vars(token: &str) -> String {
    let bytes = token.as_bytes();
    let len = bytes.len();
    let mut result = String::new();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'$' && i + 1 < len {
            if bytes[i + 1] == b'(' {
                let start = i + 2;
                let mut depth = 1;
                let mut end = start;
                while end < len && depth > 0 {
                    if bytes[end] == b'(' { depth += 1; }
                    if bytes[end] == b')' { depth -= 1; }
                    if depth > 0 { end += 1; }
                }
                if depth == 0 {
                    result.push_str(&capture_command_output(&token[start..end]));
                    i = end + 1;
                } else {
                    result.push('$'); result.push('(');
                    i = start;
                }
            } else if bytes[i + 1] == b'{' {
                let start = i + 2;
                let mut end = start;
                while end < len && bytes[end] != b'}' { end += 1; }
                if end < len {
                    let var_name = &token[start..end];
                    let mut val_buf = [0u8; 512];
                    let vlen = env::get(var_name, &mut val_buf);
                    if vlen != u32::MAX {
                        if let Ok(v) = core::str::from_utf8(&val_buf[..vlen as usize]) {
                            result.push_str(v);
                        }
                    }
                    i = end + 1;
                } else {
                    result.push('$'); result.push('{');
                    i = start;
                }
            } else if bytes[i + 1] == b'?' {
                result.push('0'); i += 2;
            } else if bytes[i + 1] == b'$' {
                result.push_str(&format!("{}", process::getpid()));
                i += 2;
            } else {
                let start = i + 1;
                let mut end = start;
                while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') { end += 1; }
                if end > start {
                    let var_name = &token[start..end];
                    let mut val_buf = [0u8; 512];
                    let vlen = env::get(var_name, &mut val_buf);
                    if vlen != u32::MAX {
                        if let Ok(v) = core::str::from_utf8(&val_buf[..vlen as usize]) {
                            result.push_str(v);
                        }
                    }
                    i = end;
                } else {
                    result.push('$'); i += 1;
                }
            }
        } else if bytes[i] == b'`' {
            let start = i + 1;
            let mut end = start;
            while end < len && bytes[end] != b'`' { end += 1; }
            if end < len {
                result.push_str(&capture_command_output(&token[start..end]));
                i = end + 1;
            } else {
                result.push('`'); i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn expand_vars_vec(tokens: Vec<String>) -> Vec<String> {
    tokens.into_iter().map(|t| expand_vars(&t)).collect()
}

fn expand_tildes_vec(tokens: Vec<String>) -> Vec<String> {
    tokens.into_iter().map(|t| expand_tilde(&t)).collect()
}

// ─── Glob expansion ──────────────────────────────────────────────────────────

fn has_glob_chars(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() { i += 2; }
        else if b[i] == b'*' || b[i] == b'?' || b[i] == b'[' { return true; }
        else { i += 1; }
    }
    false
}

fn glob_match(name: &str, pattern: &str) -> bool {
    let pat = pattern.as_bytes();
    let nam = name.as_bytes();
    let mut pi = 0;
    let mut ni = 0;
    let mut star_pi = usize::MAX;
    let mut star_ni: usize = 0;
    while ni < nam.len() {
        if pi < pat.len() && pat[pi] == b'*' {
            star_pi = pi; star_ni = ni; pi += 1;
        } else if pi < pat.len() && pat[pi] == b'?' {
            pi += 1; ni += 1;
        } else if pi < pat.len() && pat[pi] == b'[' {
            pi += 1;
            let negate = pi < pat.len() && (pat[pi] == b'!' || pat[pi] == b'^');
            if negate { pi += 1; }
            let mut matched = false;
            let ch = nam[ni];
            while pi < pat.len() && pat[pi] != b']' {
                if pi + 2 < pat.len() && pat[pi + 1] == b'-' {
                    if ch >= pat[pi] && ch <= pat[pi + 2] { matched = true; }
                    pi += 3;
                } else {
                    if ch == pat[pi] { matched = true; }
                    pi += 1;
                }
            }
            if pi < pat.len() { pi += 1; }
            if matched == negate {
                if star_pi != usize::MAX { pi = star_pi + 1; star_ni += 1; ni = star_ni; }
                else { return false; }
            } else { ni += 1; }
        } else if pi < pat.len() && pat[pi] == nam[ni] {
            pi += 1; ni += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1; star_ni += 1; ni = star_ni;
        } else { return false; }
    }
    while pi < pat.len() && pat[pi] == b'*' { pi += 1; }
    pi == pat.len()
}

fn expand_glob_token(token: &str, cwd: &str) -> Vec<String> {
    let (dir_to_read, pattern, user_prefix) = if let Some(pos) = token.rfind('/') {
        let dir_part = &token[..=pos];
        let pat_part = &token[pos + 1..];
        let full_dir = if dir_part.starts_with('/') {
            String::from(dir_part)
        } else if cwd == "/" {
            format!("/{}", dir_part)
        } else {
            format!("{}/{}", cwd, dir_part)
        };
        (full_dir, pat_part, dir_part)
    } else {
        (String::from(cwd), token, "")
    };
    if pattern.is_empty() { return Vec::new(); }

    let mut dir_buf = [0u8; 64 * 128];
    let count = fs::readdir(&dir_to_read, &mut dir_buf);
    if count == u32::MAX { return Vec::new(); }

    let mut matches: Vec<String> = Vec::new();
    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > dir_buf.len() { break; }
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len.min(56)];
        let name = match core::str::from_utf8(name_bytes) { Ok(n) => n, Err(_) => continue };
        if name.is_empty() || name == "." || name == ".." { continue; }
        if !pattern.starts_with('.') && name.starts_with('.') { continue; }
        if glob_match(name, pattern) {
            if user_prefix.is_empty() { matches.push(String::from(name)); }
            else { matches.push(format!("{}{}", user_prefix, name)); }
        }
    }
    matches.sort_unstable();
    matches
}

fn expand_globs_vec(tokens: Vec<String>, cwd: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for tok in tokens {
        if has_glob_chars(&tok) {
            let expanded = expand_glob_token(&tok, cwd);
            if expanded.is_empty() { result.push(tok); }
            else { result.extend(expanded); }
        } else {
            result.push(tok);
        }
    }
    result
}

// ─── Command substitution ────────────────────────────────────────────────────

/// Spawn `cmd`, capture stdout, return as String (trailing newlines stripped).
pub fn capture_command_output(cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.is_empty() { return String::new(); }
    let mut parts = cmd.splitn(2, ' ');
    let prog = parts.next().unwrap_or("");
    let args_str = parts.next().unwrap_or("");
    let path = resolve_cmd_path(prog, "/");
    let full_args = if args_str.is_empty() { String::from(prog) } else { format!("{} {}", prog, args_str) };
    let pipe_name = format!("subst:{}:{}", prog, process::getpid());
    let pipe_id = ipc::pipe_create(&pipe_name);
    let tid = process::spawn_piped(&path, &full_args, pipe_id);
    if tid == u32::MAX { ipc::pipe_close(pipe_id); return String::new(); }
    let mut output = String::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = ipc::pipe_read(pipe_id, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) { output.push_str(s); }
    }
    ipc::pipe_close(pipe_id);
    while output.ends_with('\n') { output.pop(); }
    while output.ends_with('\r') { output.pop(); }
    output
}

// ─── Full expansion pipeline ─────────────────────────────────────────────────

/// Tokenize + expand vars + expand tildes + expand globs.
/// This is the canonical POSIX-style argument expansion pipeline.
pub fn expand_args(raw: &str, cwd: &str) -> Vec<String> {
    if raw.is_empty() { return Vec::new(); }
    let tokens = tokenize(raw);
    let tokens = expand_vars_vec(tokens);
    let tokens = expand_tildes_vec(tokens);
    expand_globs_vec(tokens, cwd)
}

// ─── Pipeline execution ──────────────────────────────────────────────────────

/// Execute `cmd1 | cmd2 | ... | cmdN`.
///
/// Creates N named pipes, spawns all processes with stdin/stdout wired up,
/// and returns a [`PipelineResult`] with the display pipe and extra pipes to
/// clean up.  Returns `None` if any command fails to spawn (pipes are closed).
///
/// `pipe_counter` is a mutable counter used to generate unique pipe names.
pub fn run_pipeline(line: &str, cwd: &str, pipe_counter: &mut u32) -> Option<PipelineResult> {
    run_pipeline_err(line, cwd, pipe_counter).ok()
}

/// Like [`run_pipeline`] but returns the name of the failed command on error.
pub fn run_pipeline_err(line: &str, cwd: &str, pipe_counter: &mut u32) -> Result<PipelineResult, String> {
    let segments: Vec<&str> = split_pipe_segments(line);
    if segments.len() < 2 { return Err(String::from("pipeline")); }

    let n = segments.len();
    let mut pipes: Vec<u32> = Vec::new();
    for i in 0..n {
        let name = format!("sh:pipe:{}:{}", *pipe_counter, i);
        *pipe_counter = pipe_counter.wrapping_add(1);
        pipes.push(ipc::pipe_create(&name));
    }

    let display_pipe = pipes[n - 1];
    let mut last_tid = 0u32;

    for (i, segment) in segments.iter().enumerate() {
        let segment = segment.trim();
        if segment.is_empty() { continue; }
        let mut parts = segment.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").trim();
        let raw_args = parts.next().unwrap_or("").trim();
        if cmd.is_empty() { continue; }

        let mut arg_tokens = expand_args(raw_args, cwd);
        // Inject cwd for bare `ls`
        if cmd == "ls" {
            if arg_tokens.is_empty() {
                arg_tokens.push(String::from("--color"));
                arg_tokens.push(String::from(cwd));
            } else if arg_tokens.iter().all(|t| t.starts_with('-')) {
                arg_tokens.insert(0, String::from("--color"));
                arg_tokens.push(String::from(cwd));
            } else {
                arg_tokens.insert(0, String::from("--color"));
            }
        }

        let path = resolve_cmd_path(cmd, cwd);
        let quoted_args = join(&arg_tokens);
        let full_args = if quoted_args.is_empty() { String::from(cmd) } else { format!("{} {}", cmd, quoted_args) };

        let stdin_pipe = if i > 0 { pipes[i - 1] } else { 0 };
        let stdout_pipe = pipes[i];

        let tid = process::spawn_piped_full(&path, &full_args, stdout_pipe, stdin_pipe);
        if tid == u32::MAX {
            for &p in &pipes { ipc::pipe_close(p); }
            return Err(String::from(cmd));
        }
        last_tid = tid;
    }

    let extra_pipes: Vec<u32> = pipes[..n - 1].to_vec();
    Ok(PipelineResult { last_tid, display_pipe, extra_pipes })
}

/// Split on unquoted `|` characters (respects single/double quoting).
pub fn split_pipe_segments(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut segments: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut in_sq = false;
    let mut in_dq = false;
    while i < len {
        match bytes[i] {
            b'\'' if !in_dq => { in_sq = !in_sq; i += 1; }
            b'"'  if !in_sq => { in_dq = !in_dq; i += 1; }
            b'\\' if !in_sq => { i += 2; }
            b'|'  if !in_sq && !in_dq => {
                segments.push(&line[start..i]);
                start = i + 1;
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    segments.push(&line[start..]);
    segments
}

/// Returns true if `line` contains an unquoted `|` character.
pub fn has_pipe(line: &str) -> bool {
    split_pipe_segments(line).len() > 1
}
