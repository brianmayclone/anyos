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
pub const HISTORY_MAX: usize = 64;

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
