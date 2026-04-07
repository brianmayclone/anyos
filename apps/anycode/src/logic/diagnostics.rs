use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

// ════════════════════════════════════════════════════════════════
//  Diagnostics — compiler output parsing for errors/warnings
// ════════════════════════════════════════════════════════════════

/// Severity level of a diagnostic.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }

    pub fn icon_color(&self) -> u32 {
        match self {
            Self::Error => 0xFFF44747,    // Red
            Self::Warning => 0xFFCCA700,  // Yellow/orange
            Self::Info => 0xFF3794FF,     // Blue
            Self::Hint => 0xFF75BEFF,     // Light blue
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Error => "circle-x",
            Self::Warning => "alert-triangle",
            Self::Info => "info-circle",
            Self::Hint => "bulb",
        }
    }
}

/// A single diagnostic entry parsed from compiler output.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
    /// Optional error code (e.g. "E0308" for Rust).
    pub code: Option<String>,
}

/// Aggregated diagnostics from a build run.
pub struct DiagnosticSet {
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticSet {
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.diagnostics.clear();
    }

    /// Parse compiler output and append any detected diagnostics.
    pub fn parse_output(&mut self, output: &str) {
        for line in output.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Try each parser in order
            if let Some(diag) = try_parse_rust(trimmed) {
                self.diagnostics.push(diag);
            } else if let Some(diag) = try_parse_gcc(trimmed) {
                self.diagnostics.push(diag);
            } else if let Some(diag) = try_parse_python(trimmed) {
                self.diagnostics.push(diag);
            }
        }
    }

    /// Count diagnostics by severity.
    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count()
    }

    /// Get diagnostics for a specific file.
    pub fn for_file(&self, file_path: &str) -> Vec<&Diagnostic> {
        self.diagnostics.iter().filter(|d| d.file_path == file_path).collect()
    }

    /// Build a summary string: "2 errors, 3 warnings"
    pub fn summary(&self) -> String {
        let errors = self.error_count();
        let warnings = self.warning_count();
        if errors == 0 && warnings == 0 {
            return String::from("No problems");
        }
        let mut parts = Vec::new();
        if errors > 0 {
            parts.push(format!("{} error{}", errors, if errors == 1 { "" } else { "s" }));
        }
        if warnings > 0 {
            parts.push(format!("{} warning{}", warnings, if warnings == 1 { "" } else { "s" }));
        }
        let mut result = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                result.push_str(", ");
            }
            result.push_str(part);
        }
        result
    }
}

// ════════════════════════════════════════════════════════════════
//  Parser: Rust / anyrc output
// ════════════════════════════════════════════════════════════════

/// Parse Rust compiler output format:
///   error[E0308]: mismatched types
///    --> src/main.rs:10:5
///   warning: unused variable: `x`
///    --> src/lib.rs:42:9
fn try_parse_rust(line: &str) -> Option<Diagnostic> {
    // Format: "error[E0308]: message" or "warning: message"
    let (severity, rest, code): (Severity, &str, Option<String>) = if line.starts_with("error[") {
        let bracket_end = line.find(']')?;
        let code = String::from(&line[6..bracket_end]);
        let after = line[bracket_end + 1..].trim_start_matches(':');
        (Severity::Error, after.trim(), Some(code))
    } else if line.starts_with("error: ") {
        (Severity::Error, &line[7..], None)
    } else if line.starts_with("warning[") {
        let bracket_end = line.find(']')?;
        let code = String::from(&line[8..bracket_end]);
        let after = line[bracket_end + 1..].trim_start_matches(':');
        (Severity::Warning, after.trim(), Some(code))
    } else if line.starts_with("warning: ") {
        (Severity::Warning, &line[9..], None)
    } else if line.starts_with("note: ") {
        (Severity::Info, &line[6..], None)
    } else {
        return None;
    };

    // This is a message-only line without file location.
    // Return it with empty file path — the next " --> file:line:col" line
    // will be picked up separately.
    Some(Diagnostic {
        severity,
        file_path: String::new(),
        line: 0,
        column: 0,
        message: String::from(rest),
        code,
    })
}

// ════════════════════════════════════════════════════════════════
//  Parser: GCC / cc output
// ════════════════════════════════════════════════════════════════

/// Parse GCC/Clang output format:
///   file.c:10:5: error: expected ';'
///   file.c:10:5: warning: unused variable
fn try_parse_gcc(line: &str) -> Option<Diagnostic> {
    // Format: file:line:col: severity: message
    let parts: Vec<&str> = line.splitn(5, ':').collect();
    if parts.len() < 5 {
        return None;
    }

    let file = parts[0].trim();
    let line_no: u32 = parse_u32(parts[1].trim())?;
    let col: u32 = parse_u32(parts[2].trim())?;
    let severity_str = parts[3].trim();
    let message = parts[4].trim();

    let severity = match severity_str {
        "error" | "fatal error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" => Severity::Info,
        _ => return None,
    };

    Some(Diagnostic {
        severity,
        file_path: String::from(file),
        line: line_no,
        column: col,
        message: String::from(message),
        code: None,
    })
}

// ════════════════════════════════════════════════════════════════
//  Parser: Python tracebacks
// ════════════════════════════════════════════════════════════════

/// Parse Python error output:
///   File "script.py", line 10
///     SyntaxError: invalid syntax
fn try_parse_python(line: &str) -> Option<Diagnostic> {
    // "  File "path", line N"
    if line.trim_start().starts_with("File \"") {
        let start = line.find('"')? + 1;
        let end = line[start..].find('"')? + start;
        let file = &line[start..end];

        let after_quote = &line[end..];
        if !after_quote.starts_with("\", line ") { return None; }
        let line_str = &after_quote[8..];
        let line_end = line_str.find(|c: char| !c.is_ascii_digit()).unwrap_or(line_str.len());
        let line_no: u32 = parse_u32(&line_str[..line_end])?;

        return Some(Diagnostic {
            severity: Severity::Error,
            file_path: String::from(file),
            line: line_no,
            column: 0,
            message: String::new(), // Message comes on the next line
            code: None,
        });
    }

    // "SyntaxError: message" or "NameError: message"
    if line.contains("Error:") && !line.starts_with(' ') {
        let colon = line.find(':')?;
        let error_type = &line[..colon];
        let message = &line[colon + 1..].trim();
        return Some(Diagnostic {
            severity: Severity::Error,
            file_path: String::new(),
            line: 0,
            column: 0,
            message: format!("{}: {}", error_type, message),
            code: None,
        });
    }

    None
}

/// Simple string-to-u32 parser (no_std safe).
fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}

/// Update diagnostics with file location from a " --> file:line:col" line.
/// This is called during incremental output parsing.
pub fn try_parse_location(line: &str, diagnostics: &mut Vec<Diagnostic>) {
    let trimmed = line.trim();
    if !trimmed.starts_with("--> ") {
        return;
    }
    let loc = &trimmed[4..];
    let parts: Vec<&str> = loc.splitn(3, ':').collect();
    if parts.len() >= 2 {
        let file = parts[0];
        let line_no: u32 = parse_u32(parts[1]).unwrap_or(0);
        let col: u32 = if parts.len() >= 3 {
            parse_u32(parts[2]).unwrap_or(0)
        } else {
            0
        };

        // Update the most recent diagnostic that has no file path
        for diag in diagnostics.iter_mut().rev() {
            if diag.file_path.is_empty() {
                diag.file_path = String::from(file);
                diag.line = line_no;
                diag.column = col;
                break;
            }
        }
    }
}
