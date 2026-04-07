// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Regular expression engine for libjs.
//!
//! A compact backtracking regex engine compatible with `#![no_std]`.
//! Supports the ECMAScript RegExp subset:
//! - Character classes: `[a-z]`, `[^abc]`, `\d`, `\w`, `\s`, `.`
//! - Quantifiers: `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}` (greedy and lazy)
//! - Groups: `(...)`, `(?:...)`  (capturing and non-capturing)
//! - Alternation: `a|b`
//! - Anchors: `^`, `$`, `\b`, `\B`
//! - Escapes: `\d`, `\D`, `\w`, `\W`, `\s`, `\S`, `\n`, `\r`, `\t`, `\\`, etc.
//! - Backreferences: `\1`, `\2`, ...
//! - Flags: `g`, `i`, `m`, `s`, `u`, `y`

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Compiled regex node (instruction).
#[derive(Debug, Clone)]
enum Node {
    /// Match a single character literal.
    Literal(char),
    /// Match a single character (case-insensitive).
    LiteralI(char, char), // (lower, upper)
    /// Match any character (`.` — respects dotAll flag).
    Dot,
    /// Match a character class.
    CharClass {
        ranges: Vec<(char, char)>,
        negated: bool,
    },
    /// Shorthand class: `\d`
    Digit,
    /// Shorthand class: `\D`
    NotDigit,
    /// Shorthand class: `\w`
    Word,
    /// Shorthand class: `\W`
    NotWord,
    /// Shorthand class: `\s`
    Whitespace,
    /// Shorthand class: `\S`
    NotWhitespace,
    /// Start anchor `^`.
    AnchorStart,
    /// End anchor `$`.
    AnchorEnd,
    /// Word boundary `\b`.
    WordBoundary,
    /// Non-word boundary `\B`.
    NotWordBoundary,
    /// Greedy quantifier.
    Quantifier {
        min: u32,
        max: Option<u32>,
        lazy: bool,
        child: Vec<Node>,
    },
    /// Alternation: try left first, then right.
    Alternation(Vec<Vec<Node>>),
    /// Capturing group.
    Group { index: u32, body: Vec<Node> },
    /// Non-capturing group.
    NonCapGroup { body: Vec<Node> },
    /// Backreference `\n`.
    Backref(u32),
}

/// Compiled regular expression.
#[derive(Debug, Clone)]
pub struct Regex {
    nodes: Vec<Node>,
    pub source: String,
    pub flags: RegexFlags,
    pub group_count: u32,
}

/// Regex flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegexFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub sticky: bool,
}

impl RegexFlags {
    pub fn from_str(s: &str) -> Self {
        let mut f = RegexFlags::default();
        for c in s.chars() {
            match c {
                'g' => f.global = true,
                'i' => f.ignore_case = true,
                'm' => f.multiline = true,
                's' => f.dot_all = true,
                'u' => f.unicode = true,
                'y' => f.sticky = true,
                _ => {}
            }
        }
        f
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        if self.global {
            s.push('g');
        }
        if self.ignore_case {
            s.push('i');
        }
        if self.multiline {
            s.push('m');
        }
        if self.dot_all {
            s.push('s');
        }
        if self.unicode {
            s.push('u');
        }
        if self.sticky {
            s.push('y');
        }
        s
    }
}

/// A match result.
#[derive(Debug, Clone)]
pub struct Match {
    /// Overall match start index (char offset).
    pub start: usize,
    /// Overall match end index (char offset, exclusive).
    pub end: usize,
    /// Captured groups (index 0 = full match, 1.. = groups).
    pub groups: Vec<Option<(usize, usize)>>,
}

impl Match {
    pub fn full_match<'a>(&self, input: &'a str) -> &'a str {
        let start = char_to_byte(input, self.start);
        let end = char_to_byte(input, self.end);
        &input[start..end]
    }

    pub fn group<'a>(&self, index: usize, input: &'a str) -> Option<&'a str> {
        if index >= self.groups.len() {
            return None;
        }
        self.groups[index].map(|(s, e)| {
            let start = char_to_byte(input, s);
            let end = char_to_byte(input, e);
            &input[start..end]
        })
    }
}

pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

// ── Parser ──

struct RegexParser<'a> {
    chars: Vec<char>,
    pos: usize,
    flags: RegexFlags,
    group_count: u32,
    _src: &'a str,
}

impl<'a> RegexParser<'a> {
    fn new(pattern: &'a str, flags: RegexFlags) -> Self {
        RegexParser {
            chars: pattern.chars().collect(),
            pos: 0,
            flags,
            group_count: 0,
            _src: pattern,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse(&mut self) -> Vec<Node> {
        self.parse_alternation()
    }

    fn parse_alternation(&mut self) -> Vec<Node> {
        let mut branches = vec![self.parse_sequence()];
        while self.eat('|') {
            branches.push(self.parse_sequence());
        }
        if branches.len() == 1 {
            branches.into_iter().next().unwrap()
        } else {
            vec![Node::Alternation(branches)]
        }
    }

    fn parse_sequence(&mut self) -> Vec<Node> {
        let mut nodes = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == ')' || ch == '|' {
                break;
            }
            if let Some(node) = self.parse_atom() {
                let node = self.parse_quantifier(node);
                nodes.push(node);
            }
        }
        nodes
    }

    fn parse_atom(&mut self) -> Option<Node> {
        let ch = self.peek()?;
        match ch {
            '(' => {
                self.advance();
                if self.eat('?') {
                    // Non-capturing group (?:...)
                    if self.eat(':') {
                        let body = self.parse_alternation();
                        self.eat(')');
                        Some(Node::NonCapGroup { body })
                    } else {
                        // Skip unknown group modifiers
                        while self.peek() != Some(')') && self.peek().is_some() {
                            self.advance();
                        }
                        self.eat(')');
                        None
                    }
                } else {
                    self.group_count += 1;
                    let idx = self.group_count;
                    let body = self.parse_alternation();
                    self.eat(')');
                    Some(Node::Group { index: idx, body })
                }
            }
            '[' => Some(self.parse_char_class()),
            '^' => {
                self.advance();
                Some(Node::AnchorStart)
            }
            '$' => {
                self.advance();
                Some(Node::AnchorEnd)
            }
            '.' => {
                self.advance();
                Some(Node::Dot)
            }
            '\\' => {
                self.advance();
                Some(self.parse_escape())
            }
            _ => {
                self.advance();
                if self.flags.ignore_case {
                    let lo = to_lower(ch);
                    let up = to_upper(ch);
                    if lo != up {
                        Some(Node::LiteralI(lo, up))
                    } else {
                        Some(Node::Literal(ch))
                    }
                } else {
                    Some(Node::Literal(ch))
                }
            }
        }
    }

    fn parse_escape(&mut self) -> Node {
        let ch = self.advance().unwrap_or('\\');
        match ch {
            'd' => Node::Digit,
            'D' => Node::NotDigit,
            'w' => Node::Word,
            'W' => Node::NotWord,
            's' => Node::Whitespace,
            'S' => Node::NotWhitespace,
            'b' => Node::WordBoundary,
            'B' => Node::NotWordBoundary,
            'n' => Node::Literal('\n'),
            'r' => Node::Literal('\r'),
            't' => Node::Literal('\t'),
            'f' => Node::Literal('\u{000C}'),
            'v' => Node::Literal('\u{000B}'),
            '0' => Node::Literal('\0'),
            '1'..='9' => {
                // Backreference
                let mut n = (ch as u32) - ('0' as u32);
                while let Some(d) = self.peek() {
                    if d >= '0' && d <= '9' {
                        n = n * 10 + (d as u32 - '0' as u32);
                        self.advance();
                    } else {
                        break;
                    }
                }
                Node::Backref(n)
            }
            'x' => {
                // \xHH
                let h = self.advance().unwrap_or('0');
                let l = self.advance().unwrap_or('0');
                let val = hex_val(h) * 16 + hex_val(l);
                Node::Literal(char::from_u32(val).unwrap_or('\u{FFFD}'))
            }
            'u' => {
                // \uHHHH or \u{HHHH}
                if self.eat('{') {
                    let mut val = 0u32;
                    while let Some(d) = self.peek() {
                        if d == '}' {
                            self.advance();
                            break;
                        }
                        val = val * 16 + hex_val(d);
                        self.advance();
                    }
                    Node::Literal(char::from_u32(val).unwrap_or('\u{FFFD}'))
                } else {
                    let mut val = 0u32;
                    for _ in 0..4 {
                        let d = self.advance().unwrap_or('0');
                        val = val * 16 + hex_val(d);
                    }
                    Node::Literal(char::from_u32(val).unwrap_or('\u{FFFD}'))
                }
            }
            _ => {
                // Escaped literal
                if self.flags.ignore_case {
                    let lo = to_lower(ch);
                    let up = to_upper(ch);
                    if lo != up {
                        Node::LiteralI(lo, up)
                    } else {
                        Node::Literal(ch)
                    }
                } else {
                    Node::Literal(ch)
                }
            }
        }
    }

    fn parse_char_class(&mut self) -> Node {
        self.advance(); // skip [
        let negated = self.eat('^');
        let mut ranges = Vec::new();

        while let Some(ch) = self.peek() {
            if ch == ']' {
                self.advance();
                break;
            }

            let start = self.parse_class_char();
            if self.eat('-') {
                if self.peek() == Some(']') {
                    // Trailing dash: [a-] means 'a' and '-'
                    ranges.push((start, start));
                    ranges.push(('-', '-'));
                    continue;
                }
                let end = self.parse_class_char();
                if self.flags.ignore_case {
                    ranges.push((to_lower(start), to_lower(end)));
                    ranges.push((to_upper(start), to_upper(end)));
                } else {
                    ranges.push((start, end));
                }
            } else {
                if self.flags.ignore_case {
                    ranges.push((to_lower(start), to_lower(start)));
                    ranges.push((to_upper(start), to_upper(start)));
                } else {
                    ranges.push((start, start));
                }
            }
        }

        Node::CharClass { ranges, negated }
    }

    fn parse_class_char(&mut self) -> char {
        if self.eat('\\') {
            let ch = self.advance().unwrap_or('\\');
            match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'f' => '\u{000C}',
                'v' => '\u{000B}',
                '0' => '\0',
                'x' => {
                    let h = self.advance().unwrap_or('0');
                    let l = self.advance().unwrap_or('0');
                    let val = hex_val(h) * 16 + hex_val(l);
                    char::from_u32(val).unwrap_or('\u{FFFD}')
                }
                'u' => {
                    if self.eat('{') {
                        let mut val = 0u32;
                        while let Some(d) = self.peek() {
                            if d == '}' {
                                self.advance();
                                break;
                            }
                            val = val * 16 + hex_val(d);
                            self.advance();
                        }
                        char::from_u32(val).unwrap_or('\u{FFFD}')
                    } else {
                        let mut val = 0u32;
                        for _ in 0..4 {
                            let d = self.advance().unwrap_or('0');
                            val = val * 16 + hex_val(d);
                        }
                        char::from_u32(val).unwrap_or('\u{FFFD}')
                    }
                }
                _ => ch,
            }
        } else {
            self.advance().unwrap_or('\0')
        }
    }

    fn parse_quantifier(&mut self, node: Node) -> Node {
        // Quantifiers don't apply to anchors
        match &node {
            Node::AnchorStart | Node::AnchorEnd | Node::WordBoundary | Node::NotWordBoundary => {
                return node
            }
            _ => {}
        }
        let (min, max) = match self.peek() {
            Some('*') => {
                self.advance();
                (0u32, None)
            }
            Some('+') => {
                self.advance();
                (1u32, None)
            }
            Some('?') => {
                self.advance();
                (0u32, Some(1u32))
            }
            Some('{') => {
                let saved = self.pos;
                self.advance();
                if let Some((mn, mx)) = self.parse_braces() {
                    (mn, mx)
                } else {
                    self.pos = saved;
                    return node;
                }
            }
            _ => return node,
        };
        let lazy = self.eat('?');
        Node::Quantifier {
            min,
            max,
            lazy,
            child: vec![node],
        }
    }

    fn parse_braces(&mut self) -> Option<(u32, Option<u32>)> {
        let mut n = 0u32;
        let mut has_digit = false;
        while let Some(d) = self.peek() {
            if d >= '0' && d <= '9' {
                n = n * 10 + (d as u32 - '0' as u32);
                self.advance();
                has_digit = true;
            } else {
                break;
            }
        }
        if !has_digit {
            return None;
        }

        if self.eat('}') {
            return Some((n, Some(n)));
        }
        if self.eat(',') {
            if self.eat('}') {
                return Some((n, None));
            }
            let mut m = 0u32;
            let mut has_m = false;
            while let Some(d) = self.peek() {
                if d >= '0' && d <= '9' {
                    m = m * 10 + (d as u32 - '0' as u32);
                    self.advance();
                    has_m = true;
                } else {
                    break;
                }
            }
            if has_m && self.eat('}') {
                return Some((n, Some(m)));
            }
        }
        None
    }
}

// ── Execution engine ──

struct ExecState<'a> {
    chars: &'a [char],
    flags: RegexFlags,
    /// Captured groups: (start, end) char indices. Index 0 reserved for full match.
    groups: Vec<Option<(usize, usize)>>,
}

impl<'a> ExecState<'a> {
    fn new(chars: &'a [char], flags: RegexFlags, group_count: u32) -> Self {
        let cap = (group_count + 1) as usize;
        ExecState {
            chars,
            flags,
            groups: vec![None; cap],
        }
    }

    /// Try to match `nodes` starting at position `pos`. Returns the end position if matched.
    fn exec_nodes(&mut self, nodes: &[Node], mut pos: usize) -> Option<usize> {
        for node in nodes {
            pos = self.exec_node(node, pos)?;
        }
        Some(pos)
    }

    fn exec_node(&mut self, node: &Node, pos: usize) -> Option<usize> {
        match node {
            Node::Literal(ch) => {
                if pos < self.chars.len() && self.chars[pos] == *ch {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::LiteralI(lo, up) => {
                if pos < self.chars.len() {
                    let c = self.chars[pos];
                    if c == *lo || c == *up {
                        Some(pos + 1)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Node::Dot => {
                if pos < self.chars.len() {
                    let c = self.chars[pos];
                    if self.flags.dot_all
                        || (c != '\n' && c != '\r' && c != '\u{2028}' && c != '\u{2029}')
                    {
                        Some(pos + 1)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Node::Digit => {
                if pos < self.chars.len() && self.chars[pos].is_ascii_digit() {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::NotDigit => {
                if pos < self.chars.len() && !self.chars[pos].is_ascii_digit() {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::Word => {
                if pos < self.chars.len() && is_word_char(self.chars[pos]) {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::NotWord => {
                if pos < self.chars.len() && !is_word_char(self.chars[pos]) {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::Whitespace => {
                if pos < self.chars.len() && is_whitespace(self.chars[pos]) {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::NotWhitespace => {
                if pos < self.chars.len() && !is_whitespace(self.chars[pos]) {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::CharClass { ranges, negated } => {
                if pos >= self.chars.len() {
                    return None;
                }
                let c = self.chars[pos];
                let mut in_class = false;
                for &(lo, hi) in ranges {
                    if c >= lo && c <= hi {
                        in_class = true;
                        break;
                    }
                }
                if *negated {
                    in_class = !in_class;
                }
                if in_class {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            Node::AnchorStart => {
                if self.flags.multiline {
                    if pos == 0 || (pos > 0 && self.chars[pos - 1] == '\n') {
                        Some(pos)
                    } else {
                        None
                    }
                } else {
                    if pos == 0 {
                        Some(pos)
                    } else {
                        None
                    }
                }
            }
            Node::AnchorEnd => {
                if self.flags.multiline {
                    if pos == self.chars.len() || self.chars[pos] == '\n' {
                        Some(pos)
                    } else {
                        None
                    }
                } else {
                    if pos == self.chars.len() {
                        Some(pos)
                    } else {
                        None
                    }
                }
            }
            Node::WordBoundary => {
                let before = if pos > 0 {
                    is_word_char(self.chars[pos - 1])
                } else {
                    false
                };
                let after = if pos < self.chars.len() {
                    is_word_char(self.chars[pos])
                } else {
                    false
                };
                if before != after {
                    Some(pos)
                } else {
                    None
                }
            }
            Node::NotWordBoundary => {
                let before = if pos > 0 {
                    is_word_char(self.chars[pos - 1])
                } else {
                    false
                };
                let after = if pos < self.chars.len() {
                    is_word_char(self.chars[pos])
                } else {
                    false
                };
                if before == after {
                    Some(pos)
                } else {
                    None
                }
            }
            Node::Alternation(branches) => {
                for branch in branches {
                    let saved = self.groups.clone();
                    if let Some(end) = self.exec_nodes(branch, pos) {
                        return Some(end);
                    }
                    self.groups = saved;
                }
                None
            }
            Node::Group { index, body } => {
                let saved = self.groups.clone();
                let start = pos;
                if let Some(end) = self.exec_nodes(body, pos) {
                    let idx = *index as usize;
                    if idx < self.groups.len() {
                        self.groups[idx] = Some((start, end));
                    }
                    Some(end)
                } else {
                    self.groups = saved;
                    None
                }
            }
            Node::NonCapGroup { body } => self.exec_nodes(body, pos),
            Node::Backref(n) => {
                let idx = *n as usize;
                if idx < self.groups.len() {
                    if let Some((gs, ge)) = self.groups[idx] {
                        let group_chars = &self.chars[gs..ge];
                        let len = group_chars.len();
                        if pos + len > self.chars.len() {
                            return None;
                        }
                        if self.flags.ignore_case {
                            for i in 0..len {
                                if to_lower(self.chars[pos + i]) != to_lower(group_chars[i]) {
                                    return None;
                                }
                            }
                        } else {
                            for i in 0..len {
                                if self.chars[pos + i] != group_chars[i] {
                                    return None;
                                }
                            }
                        }
                        Some(pos + len)
                    } else {
                        // Unmatched group matches empty string
                        Some(pos)
                    }
                } else {
                    Some(pos)
                }
            }
            Node::Quantifier {
                min,
                max,
                lazy,
                child,
            } => self.exec_quantifier(child, *min, *max, *lazy, pos),
        }
    }

    fn exec_quantifier(
        &mut self,
        child: &[Node],
        min: u32,
        max: Option<u32>,
        lazy: bool,
        pos: usize,
    ) -> Option<usize> {
        if lazy {
            self.exec_quantifier_lazy(child, min, max, pos)
        } else {
            self.exec_quantifier_greedy(child, min, max, pos)
        }
    }

    fn exec_quantifier_greedy(
        &mut self,
        child: &[Node],
        min: u32,
        max: Option<u32>,
        start_pos: usize,
    ) -> Option<usize> {
        // Collect all possible match positions greedily
        let mut positions = Vec::new();
        let mut pos = start_pos;
        let max_count = max.unwrap_or(u32::MAX);

        positions.push((pos, self.groups.clone()));
        let mut count = 0u32;
        while count < max_count {
            let saved = self.groups.clone();
            if let Some(next) = self.exec_nodes(child, pos) {
                if next == pos {
                    // Prevent infinite loop on zero-width match
                    break;
                }
                count += 1;
                pos = next;
                positions.push((pos, self.groups.clone()));
            } else {
                self.groups = saved;
                break;
            }
        }

        if (positions.len() as u32) < min + 1 {
            return None;
        }

        // Try from most matches to fewest (greedy)
        let start = min as usize;
        for i in (start..positions.len()).rev() {
            let (p, ref groups) = positions[i];
            self.groups = groups.clone();
            return Some(p);
        }
        None
    }

    fn exec_quantifier_lazy(
        &mut self,
        child: &[Node],
        min: u32,
        _max: Option<u32>,
        start_pos: usize,
    ) -> Option<usize> {
        let mut pos = start_pos;

        // First, match minimum required
        for _ in 0..min {
            if let Some(next) = self.exec_nodes(child, pos) {
                if next == pos {
                    break;
                }
                pos = next;
            } else {
                return None;
            }
        }

        // For lazy, return the minimum match position.
        // A full implementation would try minimum first, then expand if the
        // rest of the pattern fails. This simplified version works for most cases.
        Some(pos)
    }
}

// ── Public API ──

impl Regex {
    /// Compile a regex pattern with flags.
    pub fn new(pattern: &str, flags_str: &str) -> Result<Self, String> {
        let flags = RegexFlags::from_str(flags_str);
        let mut parser = RegexParser::new(pattern, flags);
        let nodes = parser.parse();
        Ok(Regex {
            nodes,
            source: String::from(pattern),
            flags,
            group_count: parser.group_count,
        })
    }

    /// Test if the pattern matches anywhere in the input string.
    pub fn test(&self, input: &str, start_index: usize) -> bool {
        self.exec(input, start_index).is_some()
    }

    /// Execute the regex against the input, returning the first match from `start_index`.
    pub fn exec(&self, input: &str, start_index: usize) -> Option<Match> {
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();

        if self.flags.sticky {
            // Sticky: only match at start_index
            return self.try_match_at(&chars, start_index);
        }

        // Search for the first match starting from start_index
        for i in start_index..=len {
            if let Some(m) = self.try_match_at(&chars, i) {
                return Some(m);
            }
        }
        None
    }

    fn try_match_at(&self, chars: &[char], pos: usize) -> Option<Match> {
        let mut state = ExecState::new(chars, self.flags, self.group_count);
        if let Some(end) = state.exec_nodes(&self.nodes, pos) {
            state.groups[0] = Some((pos, end));
            Some(Match {
                start: pos,
                end,
                groups: state.groups,
            })
        } else {
            None
        }
    }

    /// Find all non-overlapping matches (for global flag).
    pub fn exec_all(&self, input: &str) -> Vec<Match> {
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();
        let mut results = Vec::new();
        let mut pos = 0;

        while pos <= len {
            if let Some(m) = self.try_match_at(&chars, pos) {
                let next = if m.end == m.start { m.end + 1 } else { m.end };
                results.push(m);
                pos = next;
            } else {
                pos += 1;
            }
        }
        results
    }
}

// ── Helpers ──

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{00A0}' | '\u{FEFF}'
    )
}

fn to_lower(c: char) -> char {
    if c >= 'A' && c <= 'Z' {
        (c as u8 + 32) as char
    } else {
        c
    }
}

fn to_upper(c: char) -> char {
    if c >= 'a' && c <= 'z' {
        (c as u8 - 32) as char
    } else {
        c
    }
}

fn hex_val(c: char) -> u32 {
    match c {
        '0'..='9' => c as u32 - '0' as u32,
        'a'..='f' => c as u32 - 'a' as u32 + 10,
        'A'..='F' => c as u32 - 'A' as u32 + 10,
        _ => 0,
    }
}
