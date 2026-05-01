//! Interactive line editor with history navigation, cursor movement, and TAB
//! completion.
//!
//! [`read_line_interactive`] is the main entry point used by the shell loop.
//! It blocks until the user presses Enter and returns the trimmed input.

use anyos_std::String;
use anyos_std::{process, sys};

use crate::completion::handle_tab;
use crate::history::{history_append, history_load};

// ─── Number formatting ───────────────────────────────────────────────────────

/// Write the decimal representation of `n` into `buf` and return the number of
/// bytes written.  Used to build ANSI escape sequences without heap allocation.
pub fn format_usize_into(buf: &mut [u8], mut n: usize) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut len = 0;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    len
}

// ─── Display helpers ─────────────────────────────────────────────────────────

/// Redraw the characters from `cursor` to `line_len`, erase any trailing
/// garbage, then reposition the terminal cursor back to `cursor`.
pub fn redraw_from_cursor(line: &[u8], line_len: usize, cursor: usize) {
    let rest = core::str::from_utf8(&line[cursor..line_len]).unwrap_or("");
    sys::con_write(rest);
    sys::con_write("\x1b[0K"); // erase to end of line
    let move_back = line_len - cursor;
    if move_back > 0 {
        let mut esc = [0u8; 16];
        let n = format_usize_into(&mut esc, move_back);
        sys::con_write("\x1b[");
        sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
        sys::con_write("D");
    }
}

// ─── Interactive read_line ───────────────────────────────────────────────────

/// Read one line of input from the console with full editing support:
///
/// - Left / Right / Home / End: cursor movement
/// - Up / Down: history navigation
/// - TAB: command / path completion (delegates to [`handle_tab`])
/// - Backspace / Delete: character deletion
/// - Ctrl+C: cancel current input (returns empty string)
///
/// The returned string is trimmed of trailing whitespace, and the entry is
/// automatically appended to the on-disk history via [`history_append`].
pub fn read_line_interactive(prompt: &str, cwd: &str) -> String {
    let history = history_load();
    // `hist_idx == history.len()` means "current (unsaved) input".
    let mut hist_idx = history.len();
    let mut saved_line = [0u8; 512];
    let mut saved_len = 0usize;

    let mut line = [0u8; 512];
    let mut line_len = 0usize;
    let mut cursor = 0usize;

    loop {
        // Poll until a key is available.
        let key = loop {
            let k = sys::con_poll_key();
            if k != 0 {
                break k;
            }
            process::sleep(5);
        };

        match key {
            // ── Confirm ───────────────────────────────────────────────────────
            0x0A | 0x0D => {
                sys::con_write("\n");
                let s = core::str::from_utf8(&line[..line_len])
                    .unwrap_or("")
                    .trim_end();
                let result = String::from(s);
                history_append(&result);
                return result;
            }

            // ── Cancel (Ctrl+C) ───────────────────────────────────────────────
            0x03 => {
                sys::con_write("^C\n");
                return String::new();
            }

            // ── TAB completion ────────────────────────────────────────────────
            0x09 => {
                handle_tab(&mut line, &mut line_len, &mut cursor, prompt, cwd);
            }

            // ── Backspace ────────────────────────────────────────────────────
            0x08 | 0x7F => {
                if cursor > 0 {
                    for i in cursor - 1..line_len - 1 {
                        line[i] = line[i + 1];
                    }
                    line_len -= 1;
                    cursor -= 1;
                    sys::con_write("\x08");
                    redraw_from_cursor(&line, line_len, cursor);
                }
            }

            // ── Delete (forward) ─────────────────────────────────────────────
            sys::KEY_DELETE => {
                if cursor < line_len {
                    for i in cursor..line_len - 1 {
                        line[i] = line[i + 1];
                    }
                    line_len -= 1;
                    redraw_from_cursor(&line, line_len, cursor);
                }
            }

            // ── Cursor movement ───────────────────────────────────────────────
            sys::KEY_LEFT => {
                if cursor > 0 {
                    cursor -= 1;
                    sys::con_write("\x1b[D");
                }
            }
            sys::KEY_RIGHT => {
                if cursor < line_len {
                    cursor += 1;
                    sys::con_write("\x1b[C");
                }
            }
            sys::KEY_HOME => {
                if cursor > 0 {
                    let mut esc = [0u8; 16];
                    let n = format_usize_into(&mut esc, cursor);
                    sys::con_write("\x1b[");
                    sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
                    sys::con_write("D");
                    cursor = 0;
                }
            }
            sys::KEY_END => {
                if cursor < line_len {
                    let fwd = line_len - cursor;
                    let mut esc = [0u8; 16];
                    let n = format_usize_into(&mut esc, fwd);
                    sys::con_write("\x1b[");
                    sys::con_write(core::str::from_utf8(&esc[..n]).unwrap_or(""));
                    sys::con_write("C");
                    cursor = line_len;
                }
            }

            // ── History: previous ─────────────────────────────────────────────
            sys::KEY_UP => {
                if hist_idx == history.len() {
                    // Save current (unsaved) input before browsing backwards.
                    saved_len = line_len;
                    saved_line[..line_len].copy_from_slice(&line[..line_len]);
                }
                if hist_idx > 0 {
                    hist_idx -= 1;
                    load_history_entry(&history[hist_idx], &mut line, &mut line_len, &mut cursor);
                    redraw_whole_line(&line, line_len, prompt);
                }
            }

            // ── History: next / restore ───────────────────────────────────────
            sys::KEY_DOWN => {
                if hist_idx < history.len() {
                    hist_idx += 1;
                    if hist_idx == history.len() {
                        // Restore the input that was in progress before browsing.
                        line[..saved_len].copy_from_slice(&saved_line[..saved_len]);
                        line_len = saved_len;
                    } else {
                        load_history_entry(
                            &history[hist_idx],
                            &mut line,
                            &mut line_len,
                            &mut cursor,
                        );
                    }
                    cursor = line_len;
                    redraw_whole_line(&line, line_len, prompt);
                }
            }

            // ── Printable character ───────────────────────────────────────────
            ch if ch >= 0x20 => {
                if let Some(c) = char::from_u32(ch) {
                    if !c.is_control() {
                        let mut utf8_buf = [0u8; 4];
                        let encoded = c.encode_utf8(&mut utf8_buf);
                        let enc_len = encoded.len();
                        if line_len + enc_len <= line.len() - 1 {
                            // Shift existing content right by enc_len bytes
                            for i in (cursor..line_len).rev() {
                                line[i + enc_len] = line[i];
                            }
                            line[cursor..cursor + enc_len].copy_from_slice(encoded.as_bytes());
                            line_len += enc_len;
                            cursor += enc_len;
                            sys::con_write(encoded);
                            if cursor < line_len {
                                redraw_from_cursor(&line, line_len, cursor);
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Copy a history entry into `line`, updating `line_len` and `cursor`.
fn load_history_entry(entry: &str, line: &mut [u8], line_len: &mut usize, cursor: &mut usize) {
    let bytes = entry.as_bytes();
    let copy_len = bytes.len().min(line.len());
    line[..copy_len].copy_from_slice(&bytes[..copy_len]);
    *line_len = copy_len;
    *cursor = copy_len;
}

/// Redraw the entire line (`prompt + content`) and leave the terminal cursor
/// at the end of the content.
fn redraw_whole_line(line: &[u8], line_len: usize, prompt: &str) {
    sys::con_write("\r\x1b[2K");
    sys::con_write(prompt);
    sys::con_write(core::str::from_utf8(&line[..line_len]).unwrap_or(""));
}
