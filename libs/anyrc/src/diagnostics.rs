#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> u32 { self.start }
    pub fn end(&self) -> u32 { self.end }
    pub fn len(&self) -> u32 { self.end - self.start }
    pub fn dummy() -> Self { Self { start: 0, end: 0 } }
}

pub struct SourceMap {
    filename: String,
    source: String,
    line_starts: Vec<u32>,
}

impl SourceMap {
    pub fn new(filename: String, source: String) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Self { filename, source, line_starts }
    }

    pub fn filename(&self) -> &str { &self.filename }
    pub fn source(&self) -> &str { &self.source }

    pub fn line_col(&self, span: Span) -> (u32, u32) {
        let offset = span.start();
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - self.line_starts[line];
        (line as u32 + 1, col + 1)
    }

    pub fn line_text(&self, line: u32) -> &str {
        let idx = (line - 1) as usize;
        let start = self.line_starts[idx] as usize;
        let end = self.line_starts.get(idx + 1)
            .map(|&s| s as usize - 1)
            .unwrap_or(self.source.len());
        &self.source[start..end]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
}

#[derive(Debug)]
pub struct Diagnostic {
    pub level: Level,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(level: Level, message: &str, span: Span) -> Self {
        Self { level, message: message.to_string(), span }
    }

    pub fn render(&self, sm: &SourceMap) -> String {
        let (line, col) = sm.line_col(self.span);
        let level_str = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
        };
        let line_text = sm.line_text(line);
        let underline = " ".repeat((col - 1) as usize) + &"^".repeat(self.span.len().max(1) as usize);
        format!(
            "{}: {}\n  --> {}:{}:{}\n   |\n{:>3}| {}\n   | {}",
            level_str, self.message,
            sm.filename(), line, col,
            line, line_text,
            underline,
        )
    }
}
