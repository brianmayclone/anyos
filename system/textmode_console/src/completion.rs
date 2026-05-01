//! TAB completion for commands and file paths.
//!
//! When the user presses TAB the shell distinguishes two cases:
//! - **First token** on the line → complete against built-in command names and
//!   executables found on `$PATH` plus `/System/bin`.
//! - **Any subsequent token** → complete against file system paths relative to
//!   the current working directory.
//!
//! In both cases a common prefix is filled in immediately; if multiple matches
//! exist they are printed below the current input line and the prompt is
//! redrawn.

use anyos_std::format;
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::{env, fs, sys};

use crate::readline::format_usize_into;

/// Built-in command names that are not on `$PATH` but should appear in
/// TAB completion.
const TC_BUILTINS: &[&str] = &["cd", "clear", "exit", "logout", "export", "set", "unset"];

// ─── Directory listing ───────────────────────────────────────────────────────

/// Return the entries of `path` as `(name, is_dir)` pairs.
pub fn list_dir_entries(path: &str) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    let mut dir_buf = [0u8; 64 * 256];
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
            if !name.is_empty() && name != "." && name != ".." {
                entries.push((String::from(name), entry_type == 1));
            }
        }
    }
    entries
}

// ─── Common prefix ───────────────────────────────────────────────────────────

/// Find the longest common prefix shared by all strings in `items`.
pub fn common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = &items[0];
    let mut len = first.len();
    for item in &items[1..] {
        len = len.min(item.len());
        for (i, (a, b)) in first.bytes().zip(item.bytes()).enumerate() {
            if i >= len {
                break;
            }
            if a != b {
                len = i;
                break;
            }
        }
    }
    String::from(&first[..len])
}

// ─── Completion candidates ───────────────────────────────────────────────────

/// Return all command names that start with `prefix`.
///
/// Searches built-ins, every directory listed in `$PATH`, and `/System/bin`.
pub fn complete_command(prefix: &str) -> Vec<String> {
    let mut matches: Vec<String> = Vec::new();

    for &b in TC_BUILTINS {
        if b.starts_with(prefix) {
            matches.push(String::from(b));
        }
    }

    let mut path_buf = [0u8; 256];
    let plen = env::get("PATH", &mut path_buf);
    if plen != u32::MAX && plen > 0 {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..plen as usize]) {
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() {
                    continue;
                }
                for (name, _) in list_dir_entries(dir) {
                    if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
                        matches.push(name);
                    }
                }
            }
        }
    }

    // Always include /System/bin even when it is not on PATH yet.
    for (name, _) in list_dir_entries("/System/bin") {
        if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
            matches.push(name);
        }
    }

    matches.sort_unstable();
    matches
}

/// Return all file/directory completions for the partially-typed `word`,
/// resolved relative to `cwd`.
pub fn complete_path(word: &str, cwd: &str) -> Vec<String> {
    let (dir_prefix, file_prefix) = if let Some(pos) = word.rfind('/') {
        (&word[..pos + 1], &word[pos + 1..])
    } else {
        ("", word)
    };

    let search_dir = if dir_prefix.is_empty() {
        String::from(cwd)
    } else if dir_prefix.starts_with('/') {
        let p = dir_prefix.trim_end_matches('/');
        if p.is_empty() {
            String::from("/")
        } else {
            String::from(p)
        }
    } else {
        if cwd == "/" {
            format!("/{}", dir_prefix.trim_end_matches('/'))
        } else {
            format!("{}/{}", cwd, dir_prefix.trim_end_matches('/'))
        }
    };

    let mut matches: Vec<String> = Vec::new();
    for (name, is_dir) in list_dir_entries(&search_dir) {
        if name.starts_with(file_prefix) {
            let completion = if is_dir {
                format!("{}{}/", dir_prefix, name)
            } else {
                format!("{}{}", dir_prefix, name)
            };
            matches.push(completion);
        }
    }
    matches.sort_unstable();
    matches
}

// ─── TAB handler ─────────────────────────────────────────────────────────────

/// Handle a TAB keypress: expand the current word in `line` and update the
/// display in place.
///
/// `line` / `line_len` / `cursor` are updated when a unique completion is
/// inserted.  `prompt` and `cwd` are needed for display redraws.
pub fn handle_tab(
    line: &mut [u8],
    line_len: &mut usize,
    cursor: &mut usize,
    prompt: &str,
    cwd: &str,
) {
    let input = core::str::from_utf8(&line[..*line_len]).unwrap_or("");
    let before = &input[..*cursor];
    let word_start = before.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let word = &before[word_start..];
    let is_command = !before[..word_start].contains(' ');

    let completions = if is_command {
        complete_command(word)
    } else {
        complete_path(word, cwd)
    };
    if completions.is_empty() {
        return;
    }

    let match_len = word.len();

    if completions.len() == 1 {
        let completion = &completions[0];
        if completion.len() > match_len {
            insert_str(line, line_len, cursor, &completion[match_len..]);
        }
        // Add a trailing space for non-directory completions.
        if !completion.ends_with('/') {
            insert_byte(line, line_len, cursor, b' ');
        }
        redraw_line(line, *line_len, *cursor, prompt);
    } else {
        // Fill in the common prefix, then list all matches.
        let common = common_prefix(&completions);
        if common.len() > match_len {
            insert_str(line, line_len, cursor, &common[match_len..]);
        }
        sys::con_write("\n");
        for c in &completions {
            sys::con_write(c.as_str());
            sys::con_write("  ");
        }
        sys::con_write("\n");
        redraw_line(line, *line_len, *cursor, prompt);
    }
}

// ─── Internal display helpers ────────────────────────────────────────────────

/// Insert every byte of `s` at `*cursor` in `line`.
fn insert_str(line: &mut [u8], line_len: &mut usize, cursor: &mut usize, s: &str) {
    for b in s.bytes() {
        insert_byte(line, line_len, cursor, b);
    }
}

/// Insert a single byte at `*cursor` in `line` (if space allows).
fn insert_byte(line: &mut [u8], line_len: &mut usize, cursor: &mut usize, b: u8) {
    if *line_len < line.len() - 1 {
        for i in (*cursor..*line_len).rev() {
            line[i + 1] = line[i];
        }
        line[*cursor] = b;
        *line_len += 1;
        *cursor += 1;
    }
}

/// Redraw `prompt + line content` and reposition the hardware cursor.
fn redraw_line(line: &[u8], line_len: usize, cursor: usize, prompt: &str) {
    sys::con_write("\r\x1b[2K");
    sys::con_write(prompt);
    sys::con_write(core::str::from_utf8(&line[..line_len]).unwrap_or(""));
    if cursor < line_len {
        let back = line_len - cursor;
        let mut esc = [0u8; 16];
        let n = format_usize_into(&mut esc, back);
        sys::con_write("\x1b[");
        sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
        sys::con_write("D");
    }
}
