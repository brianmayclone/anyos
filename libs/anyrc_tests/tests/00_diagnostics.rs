use anyrc::diagnostics::{Span, SourceMap, Diagnostic, Level};

#[test]
fn span_contains_offset() {
    let span = Span::new(5, 10);
    assert_eq!(span.start(), 5);
    assert_eq!(span.end(), 10);
    assert_eq!(span.len(), 5);
}

#[test]
fn source_map_resolves_line_col() {
    let src = "fn main() {\n    let x = 5;\n}";
    let sm = SourceMap::new("test.rs".to_string(), src.to_string());
    let (line, col) = sm.line_col(Span::new(16, 17));
    assert_eq!(line, 2);
    assert_eq!(col, 5);
}

#[test]
fn diagnostic_formats_with_span() {
    let src = "fn main() {\n    let x = 5;\n}";
    let sm = SourceMap::new("test.rs".to_string(), src.to_string());
    let diag = Diagnostic::new(Level::Error, "test error", Span::new(16, 17));
    let output = diag.render(&sm);
    assert!(output.contains("error: test error"));
    assert!(output.contains("test.rs:2:5"));
}
