use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::diagnostics::{Diagnostic, Severity};
use crate::logic::language::LanguageId;

pub const LIVE_SOURCE: &str = "live";
pub const LIVE_CHECK_SOURCE: &str = "live-check";

pub struct CheckCommand {
    pub command: String,
    pub args: String,
    pub working_dir: String,
    pub label: String,
}

pub fn analyze_buffer(file_path: &str, text: &str, language: LanguageId) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.extend(check_merge_markers(file_path, text));
    diagnostics.extend(check_delimiters(file_path, text, language));
    diagnostics.extend(check_text_quality(file_path, text, language));
    if diagnostics.len() > 200 {
        diagnostics.truncate(200);
    }
    diagnostics
}

fn check_merge_markers(file_path: &str, text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (idx, line) in text.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<<<<<<< ")
            || trimmed.starts_with("=======")
            || trimmed.starts_with(">>>>>>> ")
        {
            diagnostics.push(make_diag(
                Severity::Error,
                file_path,
                (idx + 1) as u32,
                1,
                "unresolved merge conflict marker",
            ));
        }
    }
    diagnostics
}

fn check_delimiters(file_path: &str, text: &str, language: LanguageId) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut stack: Vec<(u8, u32, u32)> = Vec::new();
    let mut line = 1u32;
    let mut col = 1u32;
    let mut string_quote: Option<u8> = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;

    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let next = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };

        if b == b'\n' {
            line += 1;
            col = 1;
            line_comment = false;
            escaped = false;
            i += 1;
            continue;
        }

        if line_comment {
            col += 1;
            i += 1;
            continue;
        }

        if block_comment {
            if b == b'*' && next == b'/' {
                block_comment = false;
                col += 2;
                i += 2;
            } else {
                col += 1;
                i += 1;
            }
            continue;
        }

        if let Some(q) = string_quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                string_quote = None;
            }
            col += 1;
            i += 1;
            continue;
        }

        if supports_c_comments(language) && b == b'/' && next == b'/' {
            line_comment = true;
            col += 2;
            i += 2;
            continue;
        }

        if supports_c_comments(language) && b == b'/' && next == b'*' {
            block_comment = true;
            col += 2;
            i += 2;
            continue;
        }

        if language == LanguageId::Python && b == b'#' {
            line_comment = true;
            col += 1;
            i += 1;
            continue;
        }

        if b == b'"' || (b == b'\'' && language != LanguageId::Rust) {
            string_quote = Some(b);
            col += 1;
            i += 1;
            continue;
        }

        match b {
            b'(' | b'[' | b'{' => stack.push((b, line, col)),
            b')' | b']' | b'}' => {
                let expected = opening_for(b);
                if let Some((open, _, _)) = stack.last().copied() {
                    if open == expected {
                        stack.pop();
                    } else {
                        diagnostics.push(make_diag(
                            Severity::Error,
                            file_path,
                            line,
                            col,
                            &format!("unexpected closing delimiter '{}'", b as char),
                        ));
                    }
                } else {
                    diagnostics.push(make_diag(
                        Severity::Error,
                        file_path,
                        line,
                        col,
                        &format!("unexpected closing delimiter '{}'", b as char),
                    ));
                }
            }
            _ => {}
        }

        col += 1;
        i += 1;
    }

    for (open, open_line, open_col) in stack {
        diagnostics.push(make_diag(
            Severity::Error,
            file_path,
            open_line,
            open_col,
            &format!("unclosed delimiter '{}'", open as char),
        ));
    }

    diagnostics
}

fn check_text_quality(file_path: &str, text: &str, language: LanguageId) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut saw_tabs = false;
    let mut saw_spaces = false;

    for (idx, line) in text.split('\n').enumerate() {
        let line_no = (idx + 1) as u32;
        let raw_len = line.len() as u32;
        if raw_len > 140 {
            diagnostics.push(make_diag(
                Severity::Hint,
                file_path,
                line_no,
                141,
                "line is longer than 140 columns",
            ));
        }
        if line.ends_with(' ') || line.ends_with('\t') {
            diagnostics.push(make_diag(
                Severity::Hint,
                file_path,
                line_no,
                raw_len.max(1),
                "trailing whitespace",
            ));
        }

        if language == LanguageId::Python {
            let mut indent_tabs = false;
            let mut indent_spaces = false;
            for b in line.bytes() {
                if b == b'\t' {
                    indent_tabs = true;
                    saw_tabs = true;
                } else if b == b' ' {
                    indent_spaces = true;
                    saw_spaces = true;
                } else {
                    break;
                }
            }
            if indent_tabs && indent_spaces {
                diagnostics.push(make_diag(
                    Severity::Warning,
                    file_path,
                    line_no,
                    1,
                    "mixed tabs and spaces in indentation",
                ));
            }
        }
    }

    if language == LanguageId::Python && saw_tabs && saw_spaces {
        diagnostics.push(make_diag(
            Severity::Info,
            file_path,
            1,
            1,
            "file uses both tabs and spaces for indentation",
        ));
    }

    diagnostics
}

fn supports_c_comments(language: LanguageId) -> bool {
    matches!(
        language,
        LanguageId::Rust
            | LanguageId::C
            | LanguageId::Cpp
            | LanguageId::JavaScript
            | LanguageId::TypeScript
            | LanguageId::Css
    )
}

fn opening_for(close: u8) -> u8 {
    match close {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        _ => close,
    }
}

fn make_diag(
    severity: Severity,
    file_path: &str,
    line: u32,
    column: u32,
    message: &str,
) -> Diagnostic {
    Diagnostic {
        severity,
        file_path: String::from(file_path),
        line,
        column,
        end_line: line,
        end_column: column,
        message: String::from(message),
        code: None,
        source: String::from(LIVE_SOURCE),
    }
}
