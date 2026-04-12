struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> u8 {
        if self.eof() {
            0
        } else {
            self.input[self.pos]
        }
    }

    fn advance(&mut self) -> u8 {
        let ch = self.peek();
        self.pos += 1;
        ch
    }

    fn skip_whitespace(&mut self) {
        while !self.eof() {
            let ch = self.peek();
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
                self.pos += 1;
            } else if self.starts_with(b"/*") {
                self.skip_comment();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        self.pos += 2; // skip /*
        while !self.eof() {
            if self.starts_with(b"*/") {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        if self.pos + prefix.len() > self.input.len() {
            return false;
        }
        &self.input[self.pos..self.pos + prefix.len()] == prefix
    }

    fn read_ident(&mut self) -> String {
        let mut result = String::new();
        while !self.eof() {
            let ch = self.peek();
            if ch == b'\\' && self.pos + 1 < self.input.len() {
                // CSS escape: \X → literal X (simplified; full spec supports \HHHHHH)
                self.pos += 1;
                let escaped = self.peek();
                if escaped.is_ascii_hexdigit() {
                    // Hex escape: \XX or \XXXXXX — read up to 6 hex digits
                    let hex_start = self.pos;
                    let mut count = 0;
                    while !self.eof() && self.peek().is_ascii_hexdigit() && count < 6 {
                        self.pos += 1;
                        count += 1;
                    }
                    // Optional trailing space consumed per CSS spec
                    if !self.eof() && self.peek() == b' ' {
                        self.pos += 1;
                    }
                    let hex_str = &self.input[hex_start..hex_start + count];
                    if let Ok(s) = core::str::from_utf8(hex_str) {
                        if let Ok(cp) = u32::from_str_radix(s, 16) {
                            if let Some(c) = char::from_u32(cp) {
                                result.push(c);
                                continue;
                            }
                        }
                    }
                    // Fallback: skip
                } else {
                    // Simple escape: \: \. \/ etc → literal character
                    result.push(escaped as char);
                    self.pos += 1;
                }
            } else if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
                result.push(ch as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        result
    }

    /// Read until `stop` byte or EOF. Does NOT consume the stop byte.
    #[allow(dead_code)]
    fn read_until(&mut self, stop: u8) -> String {
        let start = self.pos;
        while !self.eof() && self.peek() != stop {
            self.pos += 1;
        }
        let bytes = &self.input[start..self.pos];
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Skip a balanced `{ ... }` block (including nested braces).
    fn skip_block(&mut self) {
        if self.peek() == b'{' {
            self.pos += 1;
        }
        let mut depth: u32 = 1;
        while !self.eof() && depth > 0 {
            match self.advance() {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stylesheet parser
// ---------------------------------------------------------------------------

/// Maximum number of CSS rules to prevent OOM from huge stylesheets.
const MAX_CSS_RULES: usize = 100_000;
/// Maximum total bytes of CSS property values to prevent memory explosion.
const MAX_CSS_MEMORY: usize = 128 * 1024 * 1024; // 128 MB
