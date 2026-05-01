//! In-memory representation of boot.cfg.
//!
//! The file is stored as a flat array of text lines so that comments and
//! blank lines are preserved exactly on write-back.  Sections are identified
//! by `[name]` header lines; everything before the first section header is
//! treated as global flags.

pub const MAX_LINES: usize = 256;
pub const MAX_LINE: usize = 128;

// ─── Line ─────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
pub struct Line {
    pub buf: [u8; MAX_LINE],
    pub len: usize,
}

impl Line {
    pub const fn empty() -> Self {
        Line {
            buf: [0u8; MAX_LINE],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn from_str(s: &str) -> Self {
        let mut l = Line::empty();
        let n = s.len().min(MAX_LINE);
        l.buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        l.len = n;
        l
    }
}

// ─── Config ───────────────────────────────────────────────────────────────────

pub struct Config {
    pub lines: [Line; MAX_LINES],
    pub count: usize,
}

impl Config {
    pub fn new() -> Self {
        Config {
            lines: [Line::empty(); MAX_LINES],
            count: 0,
        }
    }

    pub fn push(&mut self, s: &str) {
        if self.count < MAX_LINES {
            self.lines[self.count] = Line::from_str(s);
            self.count += 1;
        }
    }

    pub fn insert(&mut self, pos: usize, s: &str) {
        if self.count >= MAX_LINES {
            return;
        }
        let mut i = self.count;
        while i > pos {
            self.lines[i] = self.lines[i - 1];
            i -= 1;
        }
        self.lines[pos] = Line::from_str(s);
        self.count += 1;
    }

    pub fn remove_line(&mut self, pos: usize) {
        if pos >= self.count {
            return;
        }
        let mut i = pos;
        while i + 1 < self.count {
            self.lines[i] = self.lines[i + 1];
            i += 1;
        }
        self.count -= 1;
    }
}

// ─── Query helpers ────────────────────────────────────────────────────────────

/// Line index of `[name]` header, or `MAX_LINES` if not found.
pub fn find_section(cfg: &Config, name: &str) -> usize {
    let mut hdr = [0u8; MAX_LINE];
    let len = make_header(name, &mut hdr);
    let needle = core::str::from_utf8(&hdr[..len]).unwrap_or("");
    for i in 0..cfg.count {
        if cfg.lines[i].as_str() == needle {
            return i;
        }
    }
    MAX_LINES
}

/// Exclusive end of the section starting at `start`
/// (i.e. index of the next `[section]` header, or `cfg.count`).
pub fn section_end(cfg: &Config, start: usize) -> usize {
    let mut i = start + 1;
    while i < cfg.count {
        let s = cfg.lines[i].as_str();
        if is_section_header(s) {
            return i;
        }
        i += 1;
    }
    cfg.count
}

/// Index of the first `[section]` header, or `MAX_LINES` if none.
pub fn first_section(cfg: &Config) -> usize {
    for i in 0..cfg.count {
        if is_section_header(cfg.lines[i].as_str()) {
            return i;
        }
    }
    MAX_LINES
}

/// Find `key=…` within `[start, end)`. Returns `MAX_LINES` if not found.
pub fn find_key_in(cfg: &Config, start: usize, end: usize, key: &str) -> usize {
    for i in start..end {
        let s = cfg.lines[i].as_str();
        if s.len() > key.len() && s.as_bytes()[key.len()] == b'=' && &s[..key.len()] == key {
            return i;
        }
    }
    MAX_LINES
}

/// Extract the value from a `key=value` line.
pub fn line_value(line: &str) -> &str {
    if let Some(pos) = line.find('=') {
        &line[pos + 1..]
    } else {
        ""
    }
}

/// Extract the name from a `[name]` line.
pub fn section_name(line: &str) -> &str {
    if line.len() >= 2 && line.starts_with('[') && line.ends_with(']') {
        &line[1..line.len() - 1]
    } else {
        line
    }
}

/// Count how many `[section]` headers exist.
pub fn count_entries(cfg: &Config) -> usize {
    cfg.lines[..cfg.count]
        .iter()
        .filter(|l| is_section_header(l.as_str()))
        .count()
}

/// True when `s` is a `[…]` section header.
#[inline]
pub fn is_section_header(s: &str) -> bool {
    s.starts_with('[') && s.ends_with(']') && s.len() >= 3
}

// ─── Builder helpers ──────────────────────────────────────────────────────────

/// Write `[name]` into `buf`. Returns the byte length.
pub fn make_header(name: &str, buf: &mut [u8; MAX_LINE]) -> usize {
    buf[0] = b'[';
    let nb = name.as_bytes();
    let nb_len = nb.len().min(MAX_LINE - 2);
    buf[1..1 + nb_len].copy_from_slice(&nb[..nb_len]);
    buf[1 + nb_len] = b']';
    2 + nb_len
}

/// Write `key=value` into `buf`. Returns byte length, or 0 on overflow.
pub fn make_kv(key: &str, value: &str, buf: &mut [u8; MAX_LINE]) -> usize {
    let kb = key.as_bytes();
    let vb = value.as_bytes();
    let total = kb.len() + 1 + vb.len();
    if total >= MAX_LINE {
        return 0;
    }
    buf[..kb.len()].copy_from_slice(kb);
    buf[kb.len()] = b'=';
    buf[kb.len() + 1..kb.len() + 1 + vb.len()].copy_from_slice(vb);
    total
}
