use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

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
            Self::Error => 0xFFF44747,   // Red
            Self::Warning => 0xFFCCA700, // Yellow/orange
            Self::Info => 0xFF3794FF,    // Blue
            Self::Hint => 0xFF75BEFF,    // Light blue
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
    pub end_line: u32,
    pub end_column: u32,
    pub message: String,
    /// Optional error code (e.g. "E0308" for Rust).
    pub code: Option<String>,
    /// Tool or subsystem that produced the diagnostic.
    pub source: String,
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

    pub fn remove_source(&mut self, source: &str) {
        self.diagnostics.retain(|d| d.source != source);
    }

    pub fn remove_source_for_file(&mut self, source: &str, file_path: &str) {
        self.diagnostics
            .retain(|d| d.source != source || d.file_path != file_path);
    }

    pub fn append_many(&mut self, mut diagnostics: Vec<Diagnostic>) {
        self.diagnostics.append(&mut diagnostics);
        self.deduplicate();
    }

    pub fn deduplicate(&mut self) {
        let mut unique: Vec<Diagnostic> = Vec::new();
        for diag in &self.diagnostics {
            if !unique.iter().any(|existing| existing.same_identity(diag)) {
                unique.push(diag.clone());
            }
        }
        self.diagnostics = unique;
    }

    /// Parse compiler output and append any detected diagnostics.
    pub fn parse_output(&mut self, output: &str) {
        if let Some(mut eslint) = try_parse_eslint_json_output(output) {
            self.diagnostics.append(&mut eslint);
            self.deduplicate();
            return;
        }

        let mut pending_rust: Option<usize> = None;
        let mut pending_python: Option<usize> = None;

        for line in output.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some((file, line_no, col)) = parse_rust_location(trimmed) {
                if let Some(idx) = pending_rust {
                    if let Some(diag) = self.diagnostics.get_mut(idx) {
                        if !diag.has_location() {
                            diag.file_path = file;
                            diag.line = line_no;
                            diag.column = col;
                            diag.end_line = line_no;
                            diag.end_column = col;
                        }
                    }
                }
                continue;
            }

            if let Some(diag) = try_parse_rust(trimmed) {
                self.diagnostics.push(diag);
                pending_rust = Some(self.diagnostics.len() - 1);
                pending_python = None;
            } else if let Some(diag) = try_parse_gcc(trimmed) {
                self.diagnostics.push(diag);
                pending_rust = None;
                pending_python = None;
            } else if let Some(diag) = try_parse_python(trimmed) {
                let is_trace_location = diag.message.is_empty() && diag.has_location();
                self.diagnostics.push(diag);
                pending_python = if is_trace_location {
                    Some(self.diagnostics.len() - 1)
                } else {
                    None
                };
                pending_rust = None;
            } else if let Some(idx) = pending_python {
                if is_python_error_line(trimmed) {
                    if let Some(diag) = self.diagnostics.get_mut(idx) {
                        diag.message = String::from(trimmed);
                    }
                    pending_python = None;
                }
            }
        }
        self.deduplicate();
    }

    /// Count diagnostics by severity.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// Get diagnostics for a specific file.
    pub fn for_file(&self, file_path: &str) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.file_path == file_path)
            .collect()
    }

    pub fn global(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| !d.has_location())
            .collect()
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
            parts.push(format!(
                "{} error{}",
                errors,
                if errors == 1 { "" } else { "s" }
            ));
        }
        if warnings > 0 {
            parts.push(format!(
                "{} warning{}",
                warnings,
                if warnings == 1 { "" } else { "s" }
            ));
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
//  Parser: ESLint JSON output
// ════════════════════════════════════════════════════════════════

fn try_parse_eslint_json_output(output: &str) -> Option<Vec<Diagnostic>> {
    let trimmed = output.trim();
    let json = if trimmed.starts_with('[') {
        trimmed
    } else {
        let start = trimmed.find('[')?;
        let end = trimmed.rfind(']')?;
        if end <= start {
            return None;
        }
        &trimmed[start..=end]
    };

    let value = Value::parse(json).ok()?;
    let results = value.as_array()?;
    let mut diagnostics = Vec::new();
    for result in results {
        let file_path = result["filePath"].as_str().unwrap_or("");
        let Some(messages) = result["messages"].as_array() else {
            continue;
        };
        for message in messages {
            let line = json_u32(&message["line"]).unwrap_or(1);
            let column = json_u32(&message["column"]).unwrap_or(1);
            let end_line = json_u32(&message["endLine"]).unwrap_or(line);
            let end_column = json_u32(&message["endColumn"]).unwrap_or(column.saturating_add(1));
            let severity = match json_u32(&message["severity"]).unwrap_or(2) {
                1 => Severity::Warning,
                2 => Severity::Error,
                _ => Severity::Info,
            };
            let text = message["message"].as_str().unwrap_or("ESLint diagnostic");
            let rule = message["ruleId"].as_str();
            diagnostics.push(Diagnostic {
                severity,
                file_path: String::from(file_path),
                line,
                column,
                end_line,
                end_column,
                message: String::from(text),
                code: rule.map(String::from),
                source: String::from("eslint"),
            });
        }
    }
    Some(diagnostics)
}

fn json_u32(value: &Value) -> Option<u32> {
    let value = value.as_u64()?;
    if value > u32::MAX as u64 {
        None
    } else {
        Some(value as u32)
    }
}

impl Diagnostic {
    pub fn has_location(&self) -> bool {
        !self.file_path.is_empty() && self.line > 0
    }

    fn same_identity(&self, other: &Diagnostic) -> bool {
        self.severity == other.severity
            && self.file_path == other.file_path
            && self.line == other.line
            && self.column == other.column
            && self.end_line == other.end_line
            && self.end_column == other.end_column
            && self.message == other.message
            && self.code == other.code
            && self.source == other.source
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
        end_line: 0,
        end_column: 0,
        message: String::from(rest),
        code,
        source: String::from("rust"),
    })
}

// ════════════════════════════════════════════════════════════════
//  Parser: GCC / cc output
// ════════════════════════════════════════════════════════════════

/// Parse GCC/Clang output format:
///   file.c:10:5: error: expected ';'
///   file.c:10:5: warning: unused variable
fn try_parse_gcc(line: &str) -> Option<Diagnostic> {
    let (prefix, severity, message) = if let Some(i) = line.find(": fatal error:") {
        (&line[..i], Severity::Error, line[i + 15..].trim())
    } else if let Some(i) = line.find(": error:") {
        (&line[..i], Severity::Error, line[i + 8..].trim())
    } else if let Some(i) = line.find(": warning:") {
        (&line[..i], Severity::Warning, line[i + 10..].trim())
    } else if let Some(i) = line.find(": note:") {
        (&line[..i], Severity::Info, line[i + 7..].trim())
    } else {
        return None;
    };

    let (file, line_no, col) = parse_file_line_col(prefix)?;

    Some(Diagnostic {
        severity,
        file_path: file,
        line: line_no,
        column: col,
        end_line: line_no,
        end_column: col,
        message: String::from(message),
        code: None,
        source: String::from("c/c++"),
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
        if !after_quote.starts_with("\", line ") {
            return None;
        }
        let line_str = &after_quote[8..];
        let line_end = line_str
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(line_str.len());
        let line_no: u32 = parse_u32(&line_str[..line_end])?;

        return Some(Diagnostic {
            severity: Severity::Error,
            file_path: String::from(file),
            line: line_no,
            column: 0,
            end_line: line_no,
            end_column: 0,
            message: String::new(), // Message comes on the next line
            code: None,
            source: String::from("python"),
        });
    }

    // "SyntaxError: message" or "NameError: message"
    if is_python_error_line(line) {
        let colon = line.find(':')?;
        let error_type = &line[..colon];
        let message = &line[colon + 1..].trim();
        return Some(Diagnostic {
            severity: Severity::Error,
            file_path: String::new(),
            line: 0,
            column: 0,
            end_line: 0,
            end_column: 0,
            message: format!("{}: {}", error_type, message),
            code: None,
            source: String::from("python"),
        });
    }

    None
}

fn is_python_error_line(line: &str) -> bool {
    line.contains("Error:") && !line.starts_with(' ')
}

fn parse_rust_location(line: &str) -> Option<(String, u32, u32)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("--> ") {
        return None;
    }
    parse_file_line_col(trimmed[4..].trim())
}

fn parse_file_line_col(loc: &str) -> Option<(String, u32, u32)> {
    let last_sep = loc.rfind(':')?;
    let last_num = parse_u32(&loc[last_sep + 1..])?;
    let before_last = &loc[..last_sep];

    let (file, line_no, col) = if let Some(line_sep) = before_last.rfind(':') {
        if let Some(line_no) = parse_u32(&before_last[line_sep + 1..]) {
            (before_last[..line_sep].trim(), line_no, last_num)
        } else {
            (before_last.trim(), last_num, 0)
        }
    } else {
        (before_last.trim(), last_num, 0)
    };

    if file.is_empty() {
        return None;
    }
    Some((String::from(file), line_no, col))
}

/// Simple string-to-u32 parser (no_std safe).
fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}

/// Update diagnostics with file location from a " --> file:line:col" line.
/// This is called during incremental output parsing.
#[allow(dead_code)]
pub fn try_parse_location(line: &str, diagnostics: &mut Vec<Diagnostic>) {
    if let Some((file, line_no, col)) = parse_rust_location(line) {
        // Update the most recent diagnostic that has no file path
        for diag in diagnostics.iter_mut().rev() {
            if !diag.has_location() {
                diag.file_path = file;
                diag.line = line_no;
                diag.column = col;
                diag.end_line = line_no;
                diag.end_column = col;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_message_and_location_are_merged() {
        let mut set = DiagnosticSet::new();
        set.parse_output("error[E0308]: mismatched types\n  --> src/main.rs:10:5\n");

        assert_eq!(set.diagnostics.len(), 1);
        let diag = &set.diagnostics[0];
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.code.as_deref(), Some("E0308"));
        assert_eq!(diag.file_path.as_str(), "src/main.rs");
        assert_eq!(diag.line, 10);
        assert_eq!(diag.column, 5);
        assert_eq!(diag.message.as_str(), "mismatched types");
    }

    #[test]
    fn gcc_path_line_column_is_parsed_from_right() {
        let mut set = DiagnosticSet::new();
        set.parse_output("/tmp/foo:bar/main.c:22:9: warning: unused variable 'x'\n");

        assert_eq!(set.diagnostics.len(), 1);
        let diag = &set.diagnostics[0];
        assert_eq!(diag.severity, Severity::Warning);
        assert_eq!(diag.file_path.as_str(), "/tmp/foo:bar/main.c");
        assert_eq!(diag.line, 22);
        assert_eq!(diag.column, 9);
    }

    #[test]
    fn python_traceback_location_gets_error_message() {
        let mut set = DiagnosticSet::new();
        set.parse_output("  File \"script.py\", line 7\n    bad\nSyntaxError: invalid syntax\n");

        assert_eq!(set.diagnostics.len(), 1);
        let diag = &set.diagnostics[0];
        assert_eq!(diag.file_path.as_str(), "script.py");
        assert_eq!(diag.line, 7);
        assert_eq!(diag.message.as_str(), "SyntaxError: invalid syntax");
    }

    #[test]
    fn eslint_json_output_is_parsed() {
        let mut set = DiagnosticSet::new();
        set.parse_output(
            r#"[{"filePath":"/app/src/main.js","messages":[{"ruleId":"no-undef","severity":2,"message":"'ui' is not defined.","line":3,"column":5,"endLine":3,"endColumn":7}],"errorCount":1,"warningCount":0}]"#,
        );

        assert_eq!(set.diagnostics.len(), 1);
        let diag = &set.diagnostics[0];
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.file_path.as_str(), "/app/src/main.js");
        assert_eq!(diag.line, 3);
        assert_eq!(diag.column, 5);
        assert_eq!(diag.end_column, 7);
        assert_eq!(diag.code.as_deref(), Some("no-undef"));
        assert_eq!(diag.message.as_str(), "'ui' is not defined.");
    }
}
