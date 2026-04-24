// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
#![no_std]

//! Shared shell functionality for Terminal and Shell applications.
//!
//! Provides: tab completion, environment loading, command history,
//! prompt generation, input line editing, directory listing helpers,
//! and a full shell interpreter with builtins.

pub mod interpreter;

use anyos_std::{String, Vec, format};
use anyos_std::fs;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Standard POSIX-like builtin command names for completion.
pub const BUILTIN_COMMANDS: &[&str] = &[
    ".", "alias", "bg", "break", "cd", "clear", "command", "continue",
    "echo", "eval", "exec", "exit", "export", "false",
    "fg", "getopts", "hash", "help", "jobs", "kill", "local",
    "printf", "pwd", "read", "readonly", "reboot", "return", "set",
    "shift", "shutdown", "source", "su", "test", "times", "trap", "true",
    "type", "ulimit", "umask", "uname", "unalias", "unset", "wait",
];

/// Maximum history entries before oldest are dropped.
pub const HISTORY_MAX: usize = 500;

/// Default history file name (relative to $HOME).
const HISTORY_FILENAME: &str = ".anysh_history";

// ─── File / Directory Helpers ───────────────────────────────────────────────

/// Read an entire file into a buffer.  Returns the number of bytes read.
pub fn read_file_to_buf(path: &str, buf: &mut [u8]) -> usize {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return 0;
    }
    let mut total = 0usize;
    loop {
        let n = fs::read(fd, &mut buf[total..]);
        if n == 0 || n == u32::MAX {
            break;
        }
        total += n as usize;
        if total >= buf.len() {
            break;
        }
    }
    fs::close(fd);
    total
}

/// List directory entries as `(name, is_directory)` pairs.
pub fn list_dir_entries(path: &str) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    let mut dir_buf = [0u8; 64 * 256]; // 256 entries max
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        return entries;
    }
    let max = (count as usize).min(dir_buf.len() / 64);
    for i in 0..max {
        let off = i * 64;
        let entry_type = dir_buf[off];
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len.min(56)];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            if name != "." && name != ".." {
                entries.push((String::from(name), entry_type == 1));
            }
        }
    }
    entries
}

// ─── Shell Script Loading ───────────────────────────────────────────────────

/// Parsed shell script ready for execution by a frontend shell.
pub struct ShellScript {
    pub path: String,
    pub commands: Vec<String>,
}

pub enum ShellStmt {
    Command(String),
    If {
        branches: Vec<(String, Vec<ShellStmt>)>,
        else_branch: Vec<ShellStmt>,
    },
    While {
        condition: String,
        body: Vec<ShellStmt>,
    },
    For {
        var: String,
        words: String,
        body: Vec<ShellStmt>,
    },
    Break,
    Continue,
}

pub enum ScriptControl {
    None,
    Break,
    Continue,
    Exit,
}

pub struct ScriptExecResult {
    pub status: u32,
    pub control: ScriptControl,
}

pub trait ScriptExecutor {
    fn run_command(&mut self, command: &str) -> ScriptExecResult;
    fn set_var(&mut self, name: &str, value: &str);
    fn expand_words(&mut self, words: &str) -> Vec<String>;
    fn error(&mut self, message: &str);
}

/// Errors returned while resolving or loading a shell script.
pub enum ScriptError {
    EmptyPath,
    NotFound(String),
    IsDirectory(String),
    ReadFailed(String),
    InvalidUtf8(String),
    Parse(String),
}

impl ScriptError {
    pub fn message(&self) -> String {
        match self {
            ScriptError::EmptyPath => String::from("sh: missing script path"),
            ScriptError::NotFound(path) => format!("sh: {}: not found", path),
            ScriptError::IsDirectory(path) => format!("sh: {}: is a directory", path),
            ScriptError::ReadFailed(path) => format!("sh: {}: cannot read", path),
            ScriptError::InvalidUtf8(path) => format!("sh: {}: invalid UTF-8", path),
            ScriptError::Parse(message) => format!("sh: {}", message),
        }
    }
}

/// Return true for paths/commands that should be treated as shell scripts.
pub fn is_shell_script_name(name: &str) -> bool {
    name.ends_with(".sh")
}

/// Resolve a script path against the current working directory.
pub fn resolve_script_path(path: &str, cwd: &str) -> String {
    let expanded = anyos_std::shell::expand_tilde(path);
    let p = expanded.as_str();
    if p.starts_with('/') {
        normalize_path(p)
    } else if cwd == "/" {
        normalize_path(&format!("/{}", p))
    } else {
        normalize_path(&format!("{}/{}", cwd, p))
    }
}

/// Load and parse a UTF-8 shell script file.
pub fn load_shell_script(path: &str, cwd: &str) -> Result<ShellScript, ScriptError> {
    if path.trim().is_empty() {
        return Err(ScriptError::EmptyPath);
    }

    let resolved = resolve_script_path(path.trim(), cwd);
    let mut stat_buf = [0u32; 7];
    if fs::stat(&resolved, &mut stat_buf) != 0 {
        return Err(ScriptError::NotFound(resolved));
    }
    if stat_buf[0] == 1 {
        return Err(ScriptError::IsDirectory(resolved));
    }

    let bytes = read_file_to_vec(&resolved)
        .ok_or_else(|| ScriptError::ReadFailed(resolved.clone()))?;
    let text = core::str::from_utf8(&bytes)
        .map_err(|_| ScriptError::InvalidUtf8(resolved.clone()))?;
    Ok(ShellScript {
        path: resolved,
        commands: parse_shell_script(text),
    })
}

/// Parse shell script text into executable command lines.
///
/// This intentionally stays small and frontend-agnostic: shebangs, comments,
/// blank lines, CRLF, and trailing `\` line continuations are handled here;
/// command execution, builtins, pipes, redirects, and job control remain in the
/// caller so Terminal.app and textmode_console can keep their own I/O model.
pub fn parse_shell_script(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut pending = String::new();

    for (idx, raw) in text.split('\n').enumerate() {
        let line = raw.trim_end_matches('\r');
        if idx == 0 && line.starts_with("#!") {
            continue;
        }

        let line = strip_inline_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        if line_has_continuation(line) {
            let continued = line[..line.len() - 1].trim_end();
            pending.push_str(continued);
            pending.push(' ');
            continue;
        }

        if pending.is_empty() {
            commands.push(String::from(line));
        } else {
            pending.push_str(line);
            let cmd = pending.trim();
            if !cmd.is_empty() {
                commands.push(String::from(cmd));
            }
            pending.clear();
        }
    }

    let tail = pending.trim();
    if !tail.is_empty() {
        commands.push(String::from(tail));
    }

    commands
}

/// Join parsed script commands into one shell chain separated by semicolons.
pub fn script_commands_to_chain(commands: &[String]) -> String {
    let mut out = String::new();
    for (idx, cmd) in commands.iter().enumerate() {
        if idx > 0 {
            out.push_str("; ");
        }
        out.push_str(cmd);
    }
    out
}

pub fn parse_shell_program(commands: &[String]) -> Result<Vec<ShellStmt>, ScriptError> {
    let mut idx = 0usize;
    let (program, stop) = parse_shell_block(commands, &mut idx, &[]);
    if let Some(token) = stop {
        return Err(ScriptError::Parse(format!("unexpected '{}'", token)));
    }
    Ok(program)
}

pub fn run_shell_program(
    program: &[ShellStmt],
    executor: &mut dyn ScriptExecutor,
) -> ScriptExecResult {
    run_shell_block(program, executor)
}

fn run_shell_block(program: &[ShellStmt], executor: &mut dyn ScriptExecutor) -> ScriptExecResult {
    let mut status = 0u32;
    for stmt in program {
        let result = run_shell_stmt(stmt, executor);
        status = result.status;
        executor.set_var("?", &format!("{}", status));
        match result.control {
            ScriptControl::None => {}
            _ => return result,
        }
    }
    ScriptExecResult { status, control: ScriptControl::None }
}

fn run_shell_stmt(stmt: &ShellStmt, executor: &mut dyn ScriptExecutor) -> ScriptExecResult {
    match stmt {
        ShellStmt::Command(command) => executor.run_command(command),
        ShellStmt::If { branches, else_branch } => {
            for (condition, body) in branches {
                let cond = executor.run_command(condition);
                if matches!(cond.control, ScriptControl::Exit) {
                    return cond;
                }
                if cond.status == 0 {
                    return run_shell_block(body, executor);
                }
            }
            run_shell_block(else_branch, executor)
        }
        ShellStmt::While { condition, body } => {
            let mut status = 0u32;
            loop {
                let cond = executor.run_command(condition);
                if matches!(cond.control, ScriptControl::Exit) {
                    return cond;
                }
                if cond.status != 0 {
                    return ScriptExecResult { status, control: ScriptControl::None };
                }
                let result = run_shell_block(body, executor);
                status = result.status;
                match result.control {
                    ScriptControl::None => {}
                    ScriptControl::Continue => continue,
                    ScriptControl::Break => return ScriptExecResult { status: 0, control: ScriptControl::None },
                    ScriptControl::Exit => return result,
                }
            }
        }
        ShellStmt::For { var, words, body } => {
            let mut status = 0u32;
            let values = executor.expand_words(words);
            for value in values {
                executor.set_var(var, &value);
                let result = run_shell_block(body, executor);
                status = result.status;
                match result.control {
                    ScriptControl::None => {}
                    ScriptControl::Continue => continue,
                    ScriptControl::Break => return ScriptExecResult { status: 0, control: ScriptControl::None },
                    ScriptControl::Exit => return result,
                }
            }
            ScriptExecResult { status, control: ScriptControl::None }
        }
        ShellStmt::Break => ScriptExecResult { status: 0, control: ScriptControl::Break },
        ShellStmt::Continue => ScriptExecResult { status: 0, control: ScriptControl::Continue },
    }
}

fn parse_shell_block(
    commands: &[String],
    idx: &mut usize,
    stop_words: &[&str],
) -> (Vec<ShellStmt>, Option<String>) {
    let mut program = Vec::new();
    while *idx < commands.len() {
        let line = commands[*idx].trim();
        if let Some(stop) = matching_stop_word(line, stop_words) {
            return (program, Some(String::from(stop)));
        }
        if line.is_empty() {
            *idx += 1;
            continue;
        }

        if let Some(condition) = parse_header(line, "if", "then") {
            *idx += 1;
            skip_standalone_terminator(commands, idx, "then");
            let (then_branch, stop) = parse_shell_block(commands, idx, &["elif", "else", "fi"]);
            let mut branches = Vec::new();
            branches.push((condition, then_branch));
            let mut else_branch = Vec::new();
            let mut current_stop = stop;

            while let Some(stop_word) = current_stop {
                if stop_word == "elif" {
                    let elif_line = commands[*idx].trim();
                    let elif_condition = parse_header(elif_line, "elif", "then")
                        .unwrap_or_else(|| String::from(""));
                    *idx += 1;
                    skip_standalone_terminator(commands, idx, "then");
                    let (branch, next_stop) = parse_shell_block(commands, idx, &["elif", "else", "fi"]);
                    branches.push((elif_condition, branch));
                    current_stop = next_stop;
                    continue;
                }
                if stop_word == "else" {
                    *idx += 1;
                    let (branch, next_stop) = parse_shell_block(commands, idx, &["fi"]);
                    else_branch = branch;
                    current_stop = next_stop;
                    continue;
                }
                if stop_word == "fi" {
                    *idx += 1;
                    break;
                }
                break;
            }

            program.push(ShellStmt::If { branches, else_branch });
            continue;
        }

        if let Some(condition) = parse_header(line, "while", "do") {
            *idx += 1;
            skip_standalone_terminator(commands, idx, "do");
            let (body, stop) = parse_shell_block(commands, idx, &["done"]);
            if stop.is_some() {
                *idx += 1;
            }
            program.push(ShellStmt::While { condition, body });
            continue;
        }

        if let Some((var, words)) = parse_for_header(line) {
            *idx += 1;
            skip_standalone_terminator(commands, idx, "do");
            let (body, stop) = parse_shell_block(commands, idx, &["done"]);
            if stop.is_some() {
                *idx += 1;
            }
            program.push(ShellStmt::For { var, words, body });
            continue;
        }

        if line == "break" {
            program.push(ShellStmt::Break);
            *idx += 1;
            continue;
        }

        if line == "continue" {
            program.push(ShellStmt::Continue);
            *idx += 1;
            continue;
        }

        program.push(ShellStmt::Command(String::from(line)));
        *idx += 1;
    }
    (program, None)
}

fn matching_stop_word<'a>(line: &str, stop_words: &'a [&str]) -> Option<&'a str> {
    for word in stop_words {
        if line == *word || line.starts_with(&format!("{} ", word)) {
            return Some(*word);
        }
    }
    None
}

fn parse_header(line: &str, keyword: &str, terminator: &str) -> Option<String> {
    let rest = line.strip_prefix(keyword)?;
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let rest = rest.trim();
    if rest == terminator {
        return Some(String::new());
    }
    if let Some(before) = strip_trailing_keyword(rest, terminator) {
        return Some(before);
    }
    Some(String::from(rest))
}

fn parse_for_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("for ")?;
    let rest = rest.trim();
    let rest = strip_trailing_keyword(rest, "do").unwrap_or_else(|| String::from(rest));
    let mut parts = rest.splitn(2, char::is_whitespace);
    let var = parts.next().unwrap_or("").trim();
    if !is_valid_assignment_name(var) {
        return None;
    }
    let tail = parts.next().unwrap_or("").trim();
    if tail.is_empty() {
        return Some((String::from(var), String::from("$@")));
    }
    if tail == "in" {
        return Some((String::from(var), String::new()));
    }
    let words = tail.strip_prefix("in ").unwrap_or(tail).trim();
    Some((String::from(var), String::from(words)))
}

fn skip_standalone_terminator(commands: &[String], idx: &mut usize, terminator: &str) {
    if *idx < commands.len() && commands[*idx].trim() == terminator {
        *idx += 1;
    }
}

fn strip_trailing_keyword(text: &str, keyword: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed == keyword {
        return Some(String::new());
    }
    let semi_keyword = format!("; {}", keyword);
    if trimmed.ends_with(&semi_keyword) {
        let len = trimmed.len() - semi_keyword.len();
        return Some(String::from(trimmed[..len].trim()));
    }
    let bare_keyword = format!(" {}", keyword);
    if trimmed.ends_with(&bare_keyword) {
        let len = trimmed.len() - bare_keyword.len();
        return Some(String::from(trimmed[..len].trim()));
    }
    None
}

/// Apply `$0`, `$1` ... `$9` and `$#` for a script invocation.
pub fn set_script_args(script_path: &str, args: &[String]) {
    anyos_std::env::set("0", script_path);
    anyos_std::env::set("#", &format!("{}", args.len()));
    anyos_std::env::set("@", &anyos_std::shell::join(args));
    for i in 1..=9 {
        anyos_std::env::unset(&format!("{}", i));
    }
    for (idx, arg) in args.iter().take(9).enumerate() {
        anyos_std::env::set(&format!("{}", idx + 1), arg);
    }
}

/// Return KEY=VALUE assignments that are valid standalone shell statements.
pub fn parse_assignment(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.contains(' ') || trimmed.contains('\t') {
        return None;
    }
    let eq = trimmed.find('=')?;
    let key = &trimmed[..eq];
    if !is_valid_assignment_name(key) {
        return None;
    }
    Some((key, &trimmed[eq + 1..]))
}

fn read_file_to_vec(path: &str) -> Option<Vec<u8>> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == u32::MAX {
            fs::close(fd);
            return None;
        }
        if n == 0 {
            break;
        }
        for b in &buf[..n as usize] {
            out.push(*b);
        }
    }
    fs::close(fd);
    Some(out)
}

fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => { in_single = !in_single; i += 1; }
            b'"' if !in_single => { in_double = !in_double; i += 1; }
            b'\\' if !in_single => { i += 2; }
            b'#' if !in_single && !in_double => {
                if i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t' {
                    return &line[..i];
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    line
}

fn line_has_continuation(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut count = 0usize;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'\\' { count += 1; } else { break; }
    }
    count % 2 == 1
}

fn is_valid_assignment_name(name: &str) -> bool {
    let mut chars = name.bytes();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first == b'_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == b'_' || c.is_ascii_alphanumeric())
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut out = String::new();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    out
}

// ─── Tab Completion ─────────────────────────────────────────────────────────

/// Find the longest common prefix among a set of strings.
pub fn longest_common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = &items[0];
    let mut len = first.len();
    for item in &items[1..] {
        len = len.min(item.len());
        for (i, (a, b)) in first.bytes().zip(item.bytes()).enumerate() {
            if i >= len { break; }
            if a != b {
                len = i;
                break;
            }
        }
    }
    String::from(&first[..len])
}

/// Complete a command name (first word on the line).
///
/// Searches `builtins`, PATH directories, `/System/bin`, and `/System/sbin`.
/// Pass `BUILTIN_COMMANDS` for a shell with built-in command support,
/// or `&[]` for a shell that delegates to a subprocess.
pub fn complete_command(prefix: &str, builtins: &[&str]) -> Vec<String> {
    let mut matches = Vec::new();

    // Caller-provided builtins only
    for &b in builtins {
        if b.starts_with(prefix) && !matches.iter().any(|m: &String| m.as_str() == b) {
            matches.push(String::from(b));
        }
    }

    // Scan PATH directories
    let mut path_buf = [0u8; 512];
    let plen = anyos_std::env::get("PATH", &mut path_buf);
    if plen != u32::MAX && (plen as usize) <= path_buf.len() {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..plen as usize]) {
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() { continue; }
                for (name, _) in list_dir_entries(dir) {
                    if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
                        matches.push(name);
                    }
                }
            }
        }
    }

    // Always scan /System/bin and /System/sbin even if not in PATH
    for dir in &["/System/bin", "/System/sbin"] {
        for (name, _) in list_dir_entries(dir) {
            if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
                matches.push(name);
            }
        }
    }

    matches.sort();
    matches
}

/// Complete a file or directory path (argument position).
///
/// `word` is the partial path typed so far, `cwd` is the current working directory.
pub fn complete_path(word: &str, cwd: &str) -> Vec<String> {
    let (dir_prefix, file_prefix) = if let Some(slash_pos) = word.rfind('/') {
        (&word[..slash_pos + 1], &word[slash_pos + 1..])
    } else {
        ("", word)
    };

    let search_dir = if dir_prefix.is_empty() {
        String::from(cwd)
    } else if dir_prefix.starts_with('/') {
        let p = dir_prefix.trim_end_matches('/');
        if p.is_empty() { String::from("/") } else { String::from(p) }
    } else {
        if cwd == "/" {
            format!("/{}", dir_prefix.trim_end_matches('/'))
        } else {
            format!("{}/{}", cwd, dir_prefix.trim_end_matches('/'))
        }
    };

    let entries = list_dir_entries(&search_dir);
    let mut matches = Vec::new();
    for (name, is_dir) in entries {
        if name.starts_with(file_prefix) {
            let completion = if is_dir {
                format!("{}{}/", dir_prefix, name)
            } else {
                format!("{}{}", dir_prefix, name)
            };
            matches.push(completion);
        }
    }
    matches.sort();
    matches
}

/// Result of a tab-completion attempt.
pub enum CompletionResult {
    /// No matches found.
    None,
    /// Exactly one match — insert the remaining text.
    /// Fields: (text_to_insert, is_directory)
    Single(String, bool),
    /// Multiple matches — insert common prefix (may be empty), plus all matches for display.
    /// Fields: (common_prefix_to_insert, all_matches)
    Multiple(String, Vec<String>),
}

/// Perform tab completion on the current input line.
///
/// - `before_cursor`: the input text before the cursor position
/// - `cwd`: current working directory
/// - `builtins`: builtin command names to include in completion
///   (pass `BUILTIN_COMMANDS` for embedded shells, `&[]` for subprocess shells)
pub fn complete(before_cursor: &str, cwd: &str, builtins: &[&str]) -> CompletionResult {
    let word_start = before_cursor.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let word = &before_cursor[word_start..];
    let is_command = !before_cursor[..word_start].contains(|c: char| c != ' ');

    // Strip redirect operators so "< file" and "> file" complete paths
    let stripped = word.trim_start_matches(|c: char| c == '<' || c == '>');
    let prefix_len = word.len() - stripped.len();

    let completions = if is_command {
        complete_command(word, builtins)
    } else if prefix_len > 0 {
        complete_path(stripped, cwd)
    } else {
        complete_path(word, cwd)
    };

    if completions.is_empty() {
        return CompletionResult::None;
    }

    let match_len = if prefix_len > 0 { stripped.len() } else { word.len() };

    if completions.len() == 1 {
        let completion = &completions[0];
        let remaining = if completion.len() > match_len {
            String::from(&completion[match_len..])
        } else {
            String::new()
        };
        let is_dir = completion.ends_with('/');
        CompletionResult::Single(remaining, is_dir)
    } else {
        let common = longest_common_prefix(&completions);
        let to_insert = if common.len() > match_len {
            String::from(&common[match_len..])
        } else {
            String::new()
        };
        CompletionResult::Multiple(to_insert, completions)
    }
}

// ─── Environment Loading ────────────────────────────────────────────────────

/// An alias definition collected from env files.
#[derive(Clone)]
pub struct AliasDef {
    pub name: String,
    pub value: String,
}

/// Source an env file.  Supports:
/// - `KEY=VALUE`
/// - `export KEY=VALUE`
/// - `alias NAME=VALUE` / `alias NAME='VALUE'`
/// - `source /path/to/file`
/// - `# comments`
///
/// `depth` prevents infinite recursion (max 4 levels).
/// Alias definitions are collected into `aliases` if provided.
pub fn source_env_file(path: &str, depth: u32, mut aliases: Option<&mut Vec<AliasDef>>) {
    if depth > 4 {
        return;
    }
    let mut data = [0u8; 4096];
    let total = read_file_to_buf(path, &mut data);
    if total == 0 {
        return;
    }

    let text = match core::str::from_utf8(&data[..total]) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle 'source /path/to/file'
        if line.starts_with("source ") {
            let target = line[7..].trim();
            if !target.is_empty() {
                source_env_file(target, depth + 1, aliases.as_deref_mut());
            }
            continue;
        }

        // Handle 'alias NAME=VALUE' or "alias NAME='VALUE'"
        if line.starts_with("alias ") {
            if let Some(ref mut al) = aliases.as_deref_mut() {
                let alias_def = line[6..].trim();
                if let Some(eq) = alias_def.find('=') {
                    let name = alias_def[..eq].trim();
                    let mut val = alias_def[eq + 1..].trim();
                    // Strip surrounding quotes
                    if (val.starts_with('\'') && val.ends_with('\''))
                        || (val.starts_with('"') && val.ends_with('"'))
                    {
                        if val.len() >= 2 {
                            val = &val[1..val.len() - 1];
                        }
                    }
                    if !name.is_empty() {
                        if let Some(existing) = al.iter_mut().find(|a| a.name == name) {
                            existing.value = String::from(val);
                        } else {
                            al.push(AliasDef { name: String::from(name), value: String::from(val) });
                        }
                    }
                }
            }
            continue;
        }

        // Strip optional 'export ' prefix
        let assignment = if line.starts_with("export ") {
            line[7..].trim()
        } else {
            line
        };

        if let Some(eq) = assignment.find('=') {
            let key = assignment[..eq].trim();
            let val = assignment[eq + 1..].trim();
            if !key.is_empty() {
                anyos_std::env::set(key, val);
            }
        }
    }
}

/// Load system and user env files.
///
/// Returns collected alias definitions.  Also sets HOME and USER env vars.
pub fn load_dotenv() -> Vec<AliasDef> {
    let mut aliases = Vec::new();

    // 1. System environment
    source_env_file("/System/env", 0, Some(&mut aliases));

    // 2. User environment
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 32];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            if username != "root" {
                let user_env = format!("/Users/{}/env", username);
                source_env_file(&user_env, 0, Some(&mut aliases));
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
                anyos_std::env::set("USER", username);
            }
        }
    }

    aliases
}

// ─── Command History ────────────────────────────────────────────────────────

/// A simple command history with up/down navigation.
pub struct History {
    entries: Vec<String>,
    index: Option<usize>,
    max_entries: usize,
}

impl History {
    pub fn new() -> Self {
        History {
            entries: Vec::new(),
            index: None,
            max_entries: HISTORY_MAX,
        }
    }

    /// Add a command to history (skips duplicates of the last entry and empty strings).
    pub fn push(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.entries.last().map_or(false, |last| last.as_str() == trimmed) {
            return;
        }
        self.entries.push(String::from(trimmed));
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
        self.index = None;
    }

    /// Navigate up (older).  Returns the history entry or None if already at top.
    pub fn up(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = match self.index {
            None => self.entries.len() - 1,
            Some(0) => return Some(&self.entries[0]),
            Some(i) => i - 1,
        };
        self.index = Some(idx);
        Some(&self.entries[idx])
    }

    /// Navigate down (newer).  Returns the history entry, or None if past the end
    /// (meaning the user should see an empty input line).
    pub fn down(&mut self) -> Option<&str> {
        match self.index {
            None => None,
            Some(i) => {
                if i + 1 >= self.entries.len() {
                    self.index = None;
                    None
                } else {
                    self.index = Some(i + 1);
                    Some(&self.entries[i + 1])
                }
            }
        }
    }

    /// Reset the navigation index (e.g. after submitting a command).
    pub fn reset_index(&mut self) {
        self.index = None;
    }

    /// Access all entries (e.g. for a `history` builtin command).
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Return the path to the history file: `$HOME/.anysh_history`.
    fn history_path() -> Option<String> {
        let mut buf = [0u8; 128];
        let len = anyos_std::env::get("HOME", &mut buf);
        if len == u32::MAX {
            return None;
        }
        let home = core::str::from_utf8(&buf[..len as usize]).ok()?;
        Some(format!("{}/{}", home, HISTORY_FILENAME))
    }

    /// Load history from `$HOME/.anysh_history`.
    pub fn load(&mut self) {
        let path = match Self::history_path() {
            Some(p) => p,
            None => return,
        };
        let mut buf = [0u8; 32768];
        let n = read_file_to_buf(&path, &mut buf);
        if n == 0 {
            return;
        }
        let text = match core::str::from_utf8(&buf[..n]) {
            Ok(s) => s,
            Err(_) => return,
        };
        for line in text.split('\n') {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                self.entries.push(String::from(trimmed));
            }
        }
        // Keep only the last max_entries
        while self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
        self.index = None;
    }

    /// Save history to `$HOME/.anysh_history`.
    pub fn save(&self) {
        let path = match Self::history_path() {
            Some(p) => p,
            None => return,
        };
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(entry.as_str());
            out.push('\n');
        }
        let _ = fs::write_bytes(&path, out.as_bytes());
    }
}

// ─── Input Line Editing ─────────────────────────────────────────────────────

/// A UTF-8-aware editable input line with cursor tracking.
pub struct InputLine {
    pub text: String,
    /// Cursor position in characters (not bytes).
    pub cursor: usize,
}

impl InputLine {
    pub fn new() -> Self {
        InputLine {
            text: String::new(),
            cursor: 0,
        }
    }

    /// Number of characters (not bytes) in the text.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Convert character-index cursor to byte-index in the UTF-8 string.
    pub fn cursor_byte_pos(&self) -> usize {
        self.text.char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    /// Text before cursor (for completion).
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor_byte_pos()]
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.cursor_byte_pos();
        if byte_pos >= self.text.len() {
            self.text.push(c);
        } else {
            self.text.insert(byte_pos, c);
        }
        self.cursor += 1;
    }

    /// Insert a string at the cursor position.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_pos = self.cursor_byte_pos();
            self.text.remove(byte_pos);
        }
    }

    /// Delete the character at the cursor (Delete key).
    pub fn delete(&mut self) {
        if self.cursor < self.char_count() {
            let byte_pos = self.cursor_byte_pos();
            self.text.remove(byte_pos);
        }
    }

    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start of line.
    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end of line.
    pub fn cursor_end(&mut self) {
        self.cursor = self.char_count();
    }

    /// Replace the entire text (e.g. when navigating history).
    pub fn set_text(&mut self, s: &str) {
        self.text = String::from(s);
        self.cursor = self.char_count();
    }

    /// Clear the input line.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
}

// ─── Prompt Generation ──────────────────────────────────────────────────────

/// Generate a shell prompt string: `cwd> `
pub fn make_prompt(cwd: &str) -> String {
    format!("{}> ", cwd)
}

/// Generate a prompt with username: `user@cwd> `
pub fn make_prompt_with_user(cwd: &str) -> String {
    let mut name_buf = [0u8; 32];
    let uid = anyos_std::process::getuid();
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            return format!("{}@{}> ", username, cwd);
        }
    }
    make_prompt(cwd)
}
