//! Zero-allocation INI parser operating on borrowed `&str` slices.

/// A parsed INI document. Borrows the original text.
pub struct Ini<'a> {
    text: &'a str,
}

/// A single key=value entry with its section context.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub section: &'a str,
    pub key: &'a str,
    pub value: &'a str,
}

/// Iterator over all entries in an INI document.
pub struct Entries<'a> {
    lines: &'a str,
    current_section: &'a str,
}

/// Iterator over entries within a specific section.
pub struct SectionEntries<'a> {
    inner: Entries<'a>,
    target: &'a str,
}

/// Iterator over unique section names.
pub struct Sections<'a> {
    lines: &'a str,
}

// ── Construction & lookup ──────────────────────────────────────────────────

impl<'a> Ini<'a> {
    /// Parse an INI document from a string slice. No allocation.
    pub fn parse(text: &'a str) -> Self {
        Self { text }
    }

    /// Look up a value: `ini.get("section", "key")`.
    /// For global entries use `""` as section.
    pub fn get(&self, section: &str, key: &str) -> Option<&'a str> {
        self.entries()
            .find(|e| eq_ci(e.section, section) && eq_ci(e.key, key))
            .map(|e| e.value)
    }

    pub fn get_u32(&self, section: &str, key: &str) -> Option<u32> {
        self.get(section, key).and_then(parse_u32)
    }

    pub fn get_i32(&self, section: &str, key: &str) -> Option<i32> {
        self.get(section, key).and_then(parse_i32)
    }

    /// Parse boolean: yes/true/1/on → true, no/false/0/off → false.
    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.get(section, key).and_then(parse_bool)
    }

    /// Parse hex u32 (e.g. `0xFF00AA55`).
    pub fn get_hex(&self, section: &str, key: &str) -> Option<u32> {
        self.get(section, key).and_then(parse_hex)
    }

    pub fn entries(&self) -> Entries<'a> {
        Entries { lines: self.text, current_section: "" }
    }

    pub fn section(&self, name: &'a str) -> SectionEntries<'a> {
        SectionEntries { inner: self.entries(), target: name }
    }

    pub fn sections(&self) -> Sections<'a> {
        Sections { lines: self.text }
    }

    pub fn has_section(&self, name: &str) -> bool {
        self.sections().any(|s| eq_ci(s, name))
    }
}

// ── Entry iterator ─────────────────────────────────────────────────────────

impl<'a> Iterator for Entries<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        loop {
            let line = next_line(&mut self.lines)?;
            let trimmed = trim(line);
            if trimmed.is_empty() || is_comment(trimmed) { continue; }
            if let Some(name) = parse_section_header(trimmed) {
                self.current_section = name;
                continue;
            }
            if let Some((key, value)) = parse_kv(trimmed) {
                return Some(Entry { section: self.current_section, key, value });
            }
        }
    }
}

impl<'a> Iterator for SectionEntries<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        loop {
            let entry = self.inner.next()?;
            if eq_ci(entry.section, self.target) {
                return Some(entry);
            }
        }
    }
}

impl<'a> Iterator for Sections<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        loop {
            let line = next_line(&mut self.lines)?;
            let trimmed = trim(line);
            if let Some(name) = parse_section_header(trimmed) {
                return Some(name);
            }
        }
    }
}

// ── Line parsing ───────────────────────────────────────────────────────────

fn next_line<'a>(remaining: &mut &'a str) -> Option<&'a str> {
    if remaining.is_empty() { return None; }
    let bytes = remaining.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end] != b'\n' { end += 1; }
    let line_end = if end > 0 && bytes[end.saturating_sub(1)] == b'\r' { end - 1 } else { end };
    let line = &remaining[..line_end];
    *remaining = if end < bytes.len() { &remaining[end + 1..] } else { "" };
    Some(line)
}

fn is_comment(line: &str) -> bool {
    let b = line.as_bytes();
    !b.is_empty() && (b[0] == b'#' || b[0] == b';')
}

fn parse_section_header(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.is_empty() || bytes[0] != b'[' { return None; }
    let mut i = 1;
    while i < bytes.len() && bytes[i] != b']' { i += 1; }
    if i >= bytes.len() { return None; }
    Some(trim(&line[1..i]))
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut eq_pos = 0;
    while eq_pos < bytes.len() && bytes[eq_pos] != b'=' { eq_pos += 1; }
    if eq_pos == 0 || eq_pos >= bytes.len() { return None; }
    let key = trim(&line[..eq_pos]);
    let value = trim(&line[eq_pos + 1..]);
    if key.is_empty() { return None; }
    Some((key, strip_quotes(value)))
}

// ── Value parsing helpers ──────────────────────────────────────────────────

pub fn parse_bool(s: &str) -> Option<bool> {
    let s = trim(s);
    if eq_ci(s, "yes") || eq_ci(s, "true") || eq_ci(s, "1") || eq_ci(s, "on") {
        Some(true)
    } else if eq_ci(s, "no") || eq_ci(s, "false") || eq_ci(s, "0") || eq_ci(s, "off") {
        Some(false)
    } else {
        None
    }
}

pub fn parse_u32(s: &str) -> Option<u32> {
    let s = trim(s);
    if s.is_empty() { return None; }
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' { return None; }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

pub fn parse_i32(s: &str) -> Option<i32> {
    let s = trim(s);
    if s.is_empty() { return None; }
    let (neg, digits) = if s.as_bytes()[0] == b'-' { (true, &s[1..]) } else { (false, s) };
    let abs = parse_u32(digits)?;
    if neg {
        if abs > 0x8000_0000 { return None; }
        Some(-(abs as i32))
    } else {
        if abs > 0x7FFF_FFFF { return None; }
        Some(abs as i32)
    }
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let s = trim(s);
    let s = if s.len() >= 2 && s.as_bytes()[0] == b'0' && (s.as_bytes()[1] == b'x' || s.as_bytes()[1] == b'X') {
        &s[2..]
    } else {
        s
    };
    if s.is_empty() || s.len() > 8 { return None; }
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        val = (val << 4) | digit as u32;
    }
    Some(val)
}

pub fn parse_u16(s: &str) -> Option<u16> {
    parse_u32(s).and_then(|v| if v <= 0xFFFF { Some(v as u16) } else { None })
}

// ── String helpers ─────────────────────────────────────────────────────────

pub fn trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_ws(bytes[start]) { start += 1; }
    let mut end = bytes.len();
    while end > start && is_ws(bytes[end - 1]) { end -= 1; }
    &s[start..end]
}

fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn eq_ci(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    for i in 0..ab.len() {
        if to_lower(ab[i]) != to_lower(bb[i]) { return false; }
    }
    true
}

fn to_lower(b: u8) -> u8 { if b >= b'A' && b <= b'Z' { b + 32 } else { b } }
fn is_ws(b: u8) -> bool { b == b' ' || b == b'\t' || b == b'\r' }
