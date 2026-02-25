// CSS tokenizer + parser for surf browser
// no_std compatible, uses alloc for String/Vec

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::Tag;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// @media rules: each contains a query and the rules inside it.
    pub media_rules: Vec<MediaRule>,
}

/// A @media rule: query + inner rules.
pub struct MediaRule {
    pub query: MediaQuery,
    pub rules: Vec<Rule>,
}

/// Parsed @media query.
pub struct MediaQuery {
    pub conditions: Vec<MediaCondition>,
}

/// A single media condition.
pub enum MediaCondition {
    MinWidth(i32),
    MaxWidth(i32),
    MinHeight(i32),
    MaxHeight(i32),
    /// `prefers-color-scheme: dark` etc.
    PrefersColorScheme(String),
}

pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone)]
pub enum Selector {
    Simple(SimpleSelector),
    Descendant(Box<Selector>, SimpleSelector),    // A B
    Child(Box<Selector>, SimpleSelector),         // A > B
    AdjacentSibling(Box<Selector>, SimpleSelector), // A + B
    GeneralSibling(Box<Selector>, SimpleSelector),  // A ~ B
    Universal,
}

#[derive(Clone)]
pub struct SimpleSelector {
    pub tag: Option<Tag>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    pub pseudo_classes: Vec<PseudoClass>,
}

/// Attribute selector: [attr], [attr=val], [attr~=val], etc.
#[derive(Clone)]
pub struct AttrSelector {
    pub name: String,
    pub op: AttrOp,
    pub value: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,     // [attr]
    Exact,      // [attr=val]
    Contains,   // [attr~=val] (word in space-separated)
    Prefix,     // [attr^=val]
    Suffix,     // [attr$=val]
    Substring,  // [attr*=val]
    DashMatch,  // [attr|=val]
}

/// Pseudo-class selectors.
#[derive(Clone)]
pub enum PseudoClass {
    Hover,
    Active,
    Focus,
    Visited,
    FirstChild,
    LastChild,
    NthChild(i32),
    NthLastChild(i32),
    FirstOfType,
    LastOfType,
    Not(Box<SimpleSelector>),
    Empty,
    Checked,
    Disabled,
    Enabled,
    Root,
}

pub struct Declaration {
    pub property: Property,
    pub value: CssValue,
    pub important: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Property {
    Display,
    Color,
    BackgroundColor,
    Background,
    FontSize,
    FontWeight,
    FontStyle,
    TextAlign,
    TextDecoration,
    TextIndent,
    LineHeight,
    VerticalAlign,
    Width,
    Height,
    MaxWidth,
    MinWidth,
    MaxHeight,
    MinHeight,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Border,
    BorderTop,
    BorderRight,
    BorderBottom,
    BorderLeft,
    BorderColor,
    BorderWidth,
    BorderStyle,
    BorderRadius,
    ListStyleType,
    WhiteSpace,
    Overflow,
    OverflowX,
    OverflowY,
    // Positioning
    Position,
    Top,
    Right,
    Bottom,
    Left,
    ZIndex,
    // Flexbox
    FlexDirection,
    FlexWrap,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    Gap,
    RowGap,
    ColumnGap,
    Order,
    // Box model
    BoxSizing,
    // Float
    Float,
    Clear,
    // Visual
    Opacity,
    Visibility,
    TextTransform,
    Cursor,
    // Table
    BorderCollapse,
    BorderSpacing,
    TableLayout,
    /// CSS custom property (--name). Value stored in Declaration.value as Keyword.
    CustomProperty(String),
}

#[derive(Clone)]
pub enum CssValue {
    Keyword(String),
    Color(u32),
    Length(i32, Unit),
    Percentage(i32),
    Number(i32),
    Auto,
    None,
    Inherit,
    /// `var(--name)` or `var(--name, fallback)`.
    Var(String, Option<Box<CssValue>>),
    /// `calc(expr)` — stores (px_component * 100, pct_component * 100).
    /// At layout time: result = (container_width * pct / 10000) + (px / 100).
    Calc(i32, i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Pt,
    Percent,
}

// ---------------------------------------------------------------------------
// Specificity
// ---------------------------------------------------------------------------

impl Selector {
    /// Returns (ids, classes, tags) specificity tuple.
    pub fn specificity(&self) -> (u32, u32, u32) {
        match self {
            Selector::Universal => (0, 0, 0),
            Selector::Simple(s) => s.specificity(),
            Selector::Descendant(ancestor, leaf)
            | Selector::Child(ancestor, leaf)
            | Selector::AdjacentSibling(ancestor, leaf)
            | Selector::GeneralSibling(ancestor, leaf) => {
                let (a1, b1, c1) = ancestor.specificity();
                let (a2, b2, c2) = leaf.specificity();
                (a1 + a2, b1 + b2, c1 + c2)
            }
        }
    }
}

impl SimpleSelector {
    fn specificity(&self) -> (u32, u32, u32) {
        let ids = if self.id.is_some() { 1 } else { 0 };
        let classes = self.classes.len() as u32
            + self.attrs.len() as u32
            + self.pseudo_classes.len() as u32;
        let tags = if self.tag.is_some() { 1 } else { 0 };
        (ids, classes, tags)
    }
}

// ---------------------------------------------------------------------------
// Low-level parser helpers
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input: input.as_bytes(), pos: 0 }
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> u8 {
        if self.eof() { 0 } else { self.input[self.pos] }
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
        let start = self.pos;
        while !self.eof() {
            let ch = self.peek();
            if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let bytes = &self.input[start..self.pos];
        String::from_utf8_lossy(bytes).into_owned()
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

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let mut p = Parser::new(css);
    let mut rules = Vec::new();
    let mut media_rules = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() {
            break;
        }

        // At-rules
        if p.peek() == b'@' {
            p.pos += 1;
            let keyword = p.read_ident();
            let kw_lower = keyword.to_ascii_lowercase();

            if kw_lower == "media" {
                // Parse @media query and inner rules.
                if let Some(mr) = parse_media_rule(&mut p) {
                    media_rules.push(mr);
                }
                continue;
            }

            // Skip other at-rules.
            loop {
                p.skip_whitespace();
                if p.eof() {
                    break;
                }
                if p.peek() == b'{' {
                    p.skip_block();
                    break;
                }
                if p.peek() == b';' {
                    p.pos += 1;
                    break;
                }
                p.pos += 1;
            }
            continue;
        }

        // Skip stray closing braces
        if p.peek() == b'}' {
            p.pos += 1;
            continue;
        }

        // Parse rule: selectors { declarations }
        if let Some(rule) = parse_rule(&mut p) {
            rules.push(rule);
        }
    }

    Stylesheet { rules, media_rules }
}

/// Parse a @media rule: query { rules }.
fn parse_media_rule(p: &mut Parser) -> Option<MediaRule> {
    p.skip_whitespace();

    // Read everything until '{' as the media query text.
    let query_start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let query_text = core::str::from_utf8(&p.input[query_start..p.pos]).unwrap_or("");
    let query = parse_media_query(query_text);

    if p.eof() { return None; }
    p.pos += 1; // consume '{'

    // Parse inner rules until matching '}'.
    let mut inner_rules = Vec::new();
    loop {
        p.skip_whitespace();
        if p.eof() { break; }
        if p.peek() == b'}' {
            p.pos += 1;
            break;
        }
        // Skip nested at-rules inside @media.
        if p.peek() == b'@' {
            p.pos += 1;
            let _kw = p.read_ident();
            loop {
                p.skip_whitespace();
                if p.eof() { break; }
                if p.peek() == b'{' { p.skip_block(); break; }
                if p.peek() == b';' { p.pos += 1; break; }
                p.pos += 1;
            }
            continue;
        }
        if let Some(rule) = parse_rule(p) {
            inner_rules.push(rule);
        }
    }

    Some(MediaRule { query, rules: inner_rules })
}

/// Parse a media query string like `screen and (max-width: 768px)`.
fn parse_media_query(text: &str) -> MediaQuery {
    let mut conditions = Vec::new();
    let trimmed = text.trim();

    // Split on "and" (case-insensitive).
    for part in split_and(trimmed) {
        let p = part.trim();
        if p.is_empty() { continue; }

        // Skip media types: "screen", "all", "print", "not", "only".
        let lower = p.to_ascii_lowercase();
        if lower == "screen" || lower == "all" || lower == "print"
            || lower == "not" || lower == "only"
        {
            continue;
        }

        // Parenthesized condition: (min-width: 768px)
        if p.starts_with('(') && p.ends_with(')') {
            let inner = &p[1..p.len() - 1];
            if let Some(cond) = parse_media_condition(inner) {
                conditions.push(cond);
            }
        }
    }

    MediaQuery { conditions }
}

/// Split a media query string on " and " (case-insensitive).
fn split_and(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;

    for i in 0..bytes.len() {
        // Check for " and " (with spaces).
        if i + 5 <= bytes.len() {
            let chunk = &bytes[i..i + 5];
            if (chunk[0] == b' ')
                && (chunk[1] | 32 == b'a')
                && (chunk[2] | 32 == b'n')
                && (chunk[3] | 32 == b'd')
                && (chunk[4] == b' ')
            {
                parts.push(&s[start..i]);
                start = i + 5;
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse a single media condition like `max-width: 768px`.
fn parse_media_condition(inner: &str) -> Option<MediaCondition> {
    let colon = inner.find(':')?;
    let name = inner[..colon].trim().to_ascii_lowercase();
    let value_str = inner[colon + 1..].trim();

    match name.as_str() {
        "min-width" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MinWidth(px))
        }
        "max-width" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MaxWidth(px))
        }
        "min-height" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MinHeight(px))
        }
        "max-height" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MaxHeight(px))
        }
        "prefers-color-scheme" => {
            Some(MediaCondition::PrefersColorScheme(String::from(value_str.trim())))
        }
        _ => None,
    }
}

/// Parse a CSS pixel value like "768px" or "1024" into i32.
fn parse_px_value(s: &str) -> Option<i32> {
    let s = s.trim().trim_end_matches("px").trim();
    let mut val: i32 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            val = val * 10 + (*b - b'0') as i32;
        } else if *b == b'.' {
            break; // ignore fractional part
        } else {
            break;
        }
    }
    if val > 0 || s == "0" { Some(val) } else { None }
}

/// Evaluate a media query against viewport dimensions.
pub fn evaluate_media_query(query: &MediaQuery, viewport_width: i32, viewport_height: i32) -> bool {
    for cond in &query.conditions {
        let ok = match cond {
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinHeight(h) => viewport_height >= *h,
            MediaCondition::MaxHeight(h) => viewport_height <= *h,
            MediaCondition::PrefersColorScheme(scheme) => {
                // anyOS uses dark theme.
                scheme == "dark"
            }
        };
        if !ok { return false; }
    }
    true
}

fn parse_rule(p: &mut Parser) -> Option<Rule> {
    let selectors = parse_selector_list(p);
    if selectors.is_empty() {
        return Option::None;
    }

    p.skip_whitespace();
    if p.peek() != b'{' {
        // Malformed — skip to next brace or EOF
        while !p.eof() && p.peek() != b'{' && p.peek() != b'}' {
            p.pos += 1;
        }
        if p.peek() == b'{' {
            p.skip_block();
        }
        return Option::None;
    }
    p.pos += 1; // consume '{'

    let declarations = parse_declarations(p);

    // consume '}'
    p.skip_whitespace();
    if p.peek() == b'}' {
        p.pos += 1;
    }

    Some(Rule { selectors, declarations })
}

fn parse_selector_list(p: &mut Parser) -> Vec<Selector> {
    let mut selectors = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'{' {
            break;
        }

        let sel = parse_selector(p);
        selectors.push(sel);

        p.skip_whitespace();
        if p.peek() == b',' {
            p.pos += 1;
        } else {
            break;
        }
    }

    selectors
}

fn parse_selector(p: &mut Parser) -> Selector {
    p.skip_whitespace();

    let first = parse_simple_selector(p);
    let mut result = if is_universal(&first) {
        Selector::Universal
    } else {
        Selector::Simple(first)
    };

    loop {
        let had_space = skip_spaces_only(p);
        if p.eof() || p.peek() == b'{' || p.peek() == b',' {
            break;
        }
        // Check for explicit combinators: > + ~
        let combinator = if p.peek() == b'>' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'>')
        } else if p.peek() == b'+' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'+')
        } else if p.peek() == b'~' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'~')
        } else if had_space {
            Some(b' ')
        } else {
            None
        };
        match combinator {
            Some(b'>') => {
                let next = parse_simple_selector(p);
                result = Selector::Child(Box::new(result), next);
            }
            Some(b'+') => {
                let next = parse_simple_selector(p);
                result = Selector::AdjacentSibling(Box::new(result), next);
            }
            Some(b'~') => {
                let next = parse_simple_selector(p);
                result = Selector::GeneralSibling(Box::new(result), next);
            }
            Some(b' ') => {
                let next = parse_simple_selector(p);
                result = Selector::Descendant(Box::new(result), next);
            }
            _ => break,
        }
    }

    result
}

/// Skip spaces/tabs only (not newlines treated as whitespace in selectors,
/// but we do skip them). Returns true if any whitespace was consumed.
fn skip_spaces_only(p: &mut Parser) -> bool {
    let start = p.pos;
    while !p.eof() {
        let ch = p.peek();
        if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
            p.pos += 1;
        } else if p.starts_with(b"/*") {
            p.skip_comment();
        } else {
            break;
        }
    }
    p.pos > start
}

fn is_universal(s: &SimpleSelector) -> bool {
    s.tag.is_none() && s.id.is_none() && s.classes.is_empty()
        && s.attrs.is_empty() && s.pseudo_classes.is_empty()
}

fn parse_simple_selector(p: &mut Parser) -> SimpleSelector {
    let mut tag = Option::None;
    let mut id = Option::None;
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudo_classes = Vec::new();

    if p.peek() == b'*' {
        p.pos += 1;
    } else if p.peek().is_ascii_alphabetic() {
        let name = p.read_ident();
        tag = Some(Tag::from_str(&name));
    }

    // Parse chained #id, .class, [attr], :pseudo (no spaces between them)
    loop {
        if p.peek() == b'#' {
            p.pos += 1;
            id = Some(p.read_ident());
        } else if p.peek() == b'.' {
            p.pos += 1;
            classes.push(p.read_ident());
        } else if p.peek() == b'[' {
            if let Some(attr) = parse_attr_selector(p) {
                attrs.push(attr);
            }
        } else if p.peek() == b':' && !p.starts_with(b"::") {
            p.pos += 1;
            if let Some(pc) = parse_pseudo_class(p) {
                pseudo_classes.push(pc);
            }
        } else {
            break;
        }
    }

    SimpleSelector { tag, id, classes, attrs, pseudo_classes }
}

fn parse_attr_selector(p: &mut Parser) -> Option<AttrSelector> {
    p.pos += 1; // skip '['
    skip_spaces_only(p);
    let name = p.read_ident();
    if name.is_empty() {
        while !p.eof() && p.peek() != b']' { p.pos += 1; }
        if p.peek() == b']' { p.pos += 1; }
        return Option::None;
    }
    skip_spaces_only(p);
    if p.peek() == b']' {
        p.pos += 1;
        return Some(AttrSelector { name, op: AttrOp::Exists, value: Option::None });
    }

    let op = if p.starts_with(b"~=") { p.pos += 2; AttrOp::Contains }
        else if p.starts_with(b"^=") { p.pos += 2; AttrOp::Prefix }
        else if p.starts_with(b"$=") { p.pos += 2; AttrOp::Suffix }
        else if p.starts_with(b"*=") { p.pos += 2; AttrOp::Substring }
        else if p.starts_with(b"|=") { p.pos += 2; AttrOp::DashMatch }
        else if p.peek() == b'=' { p.pos += 1; AttrOp::Exact }
        else {
            while !p.eof() && p.peek() != b']' { p.pos += 1; }
            if p.peek() == b']' { p.pos += 1; }
            return Option::None;
        };

    skip_spaces_only(p);
    let value = if p.peek() == b'"' || p.peek() == b'\'' {
        let quote = p.advance();
        let start = p.pos;
        while !p.eof() && p.peek() != quote { p.pos += 1; }
        let val = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
        if p.peek() == quote { p.pos += 1; }
        val
    } else {
        p.read_ident()
    };

    skip_spaces_only(p);
    if p.peek() == b']' { p.pos += 1; }
    Some(AttrSelector { name, op, value: Some(value) })
}

fn parse_pseudo_class(p: &mut Parser) -> Option<PseudoClass> {
    let name = p.read_ident();
    let lower = to_ascii_lower(&name);
    match lower.as_str() {
        "hover" => Some(PseudoClass::Hover),
        "active" => Some(PseudoClass::Active),
        "focus" => Some(PseudoClass::Focus),
        "visited" => Some(PseudoClass::Visited),
        "first-child" => Some(PseudoClass::FirstChild),
        "last-child" => Some(PseudoClass::LastChild),
        "first-of-type" => Some(PseudoClass::FirstOfType),
        "last-of-type" => Some(PseudoClass::LastOfType),
        "empty" => Some(PseudoClass::Empty),
        "checked" => Some(PseudoClass::Checked),
        "disabled" => Some(PseudoClass::Disabled),
        "enabled" => Some(PseudoClass::Enabled),
        "root" => Some(PseudoClass::Root),
        "nth-child" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let n = parse_nth_arg(p);
                skip_spaces_only(p);
                if p.peek() == b')' { p.pos += 1; }
                Some(PseudoClass::NthChild(n))
            } else {
                Some(PseudoClass::NthChild(1))
            }
        }
        "nth-last-child" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let n = parse_nth_arg(p);
                skip_spaces_only(p);
                if p.peek() == b')' { p.pos += 1; }
                Some(PseudoClass::NthLastChild(n))
            } else {
                Some(PseudoClass::NthLastChild(1))
            }
        }
        "not" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let inner = parse_simple_selector(p);
                skip_spaces_only(p);
                if p.peek() == b')' { p.pos += 1; }
                Some(PseudoClass::Not(Box::new(inner)))
            } else {
                Option::None
            }
        }
        _ => {
            // Skip unknown pseudo-class arguments
            if p.peek() == b'(' {
                let mut depth: u32 = 1;
                p.pos += 1;
                while !p.eof() && depth > 0 {
                    match p.advance() {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                }
            }
            Option::None
        }
    }
}

fn parse_nth_arg(p: &mut Parser) -> i32 {
    let start = p.pos;
    while !p.eof() && p.peek() != b')' {
        p.pos += 1;
    }
    let arg = core::str::from_utf8(&p.input[start..p.pos]).unwrap_or("");
    let arg = arg.trim();
    match arg {
        "odd" => 1,
        "even" => 2,
        _ => parse_int(arg).unwrap_or(1),
    }
}

// ---------------------------------------------------------------------------
// Declaration parser
// ---------------------------------------------------------------------------

fn parse_declarations(p: &mut Parser) -> Vec<Declaration> {
    let mut decls = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'}' {
            break;
        }

        let prop_name = p.read_ident();
        if prop_name.is_empty() {
            // Skip garbage character
            p.pos += 1;
            continue;
        }

        p.skip_whitespace();
        if p.peek() != b':' {
            // Skip to next ';' or '}'
            while !p.eof() && p.peek() != b';' && p.peek() != b'}' {
                p.pos += 1;
            }
            if p.peek() == b';' {
                p.pos += 1;
            }
            continue;
        }
        p.pos += 1; // consume ':'

        p.skip_whitespace();

        // Read value until ';' or '}'
        let value_str = read_value_str(p);

        if p.peek() == b';' {
            p.pos += 1;
        }

        if let Some(property) = parse_property(&prop_name) {
            let trimmed = value_str.trim();
            // Detect and strip !important
            let (trimmed, important) = strip_important(trimmed);
            // Expand shorthand properties into individual declarations.
            if is_expandable_shorthand(property) {
                let mut expanded = expand_shorthand(property, trimmed);
                if important {
                    for d in &mut expanded {
                        d.important = true;
                    }
                }
                for d in expanded {
                    decls.push(d);
                }
            } else {
                let value = parse_value(property, trimmed);
                decls.push(Declaration { property, value, important });
            }
        }
    }

    decls
}

/// Strip `!important` from end of a CSS value string.
fn strip_important(s: &str) -> (&str, bool) {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return (s, false);
    }
    // Check last 10 chars case-insensitively for "!important"
    let end = &bytes[bytes.len() - 10..];
    let matches = end[0] == b'!'
        && (end[1] == b'i' || end[1] == b'I')
        && (end[2] == b'm' || end[2] == b'M')
        && (end[3] == b'p' || end[3] == b'P')
        && (end[4] == b'o' || end[4] == b'O')
        && (end[5] == b'r' || end[5] == b'R')
        && (end[6] == b't' || end[6] == b'T')
        && (end[7] == b'a' || end[7] == b'A')
        && (end[8] == b'n' || end[8] == b'N')
        && (end[9] == b't' || end[9] == b'T');
    if matches {
        let trimmed = s[..s.len() - 10].trim_end();
        (trimmed, true)
    } else {
        (s, false)
    }
}

fn read_value_str(p: &mut Parser) -> String {
    let start = p.pos;
    let mut paren_depth: u32 = 0;
    while !p.eof() {
        let ch = p.peek();
        if ch == b'(' {
            paren_depth += 1;
            p.pos += 1;
        } else if ch == b')' {
            if paren_depth > 0 {
                paren_depth -= 1;
            }
            p.pos += 1;
        } else if (ch == b';' || ch == b'}') && paren_depth == 0 {
            break;
        } else {
            p.pos += 1;
        }
    }
    let bytes = &p.input[start..p.pos];
    String::from_utf8_lossy(bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Inline style parser
// ---------------------------------------------------------------------------

pub fn parse_inline_style(style: &str) -> Vec<Declaration> {
    let mut p = Parser::new(style);
    parse_declarations(&mut p)
}

// ---------------------------------------------------------------------------
// Property name matching
// ---------------------------------------------------------------------------

pub fn parse_property(name: &str) -> Option<Property> {
    // Convert to lowercase for comparison
    let mut buf = [0u8; 40];
    let len = name.len().min(40);
    for (i, &b) in name.as_bytes()[..len].iter().enumerate() {
        buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
    }
    let lower = core::str::from_utf8(&buf[..len]).ok()?;

    match lower {
        "display" => Some(Property::Display),
        "color" => Some(Property::Color),
        "background-color" => Some(Property::BackgroundColor),
        "background" => Some(Property::Background),
        "font-size" => Some(Property::FontSize),
        "font-weight" => Some(Property::FontWeight),
        "font-style" => Some(Property::FontStyle),
        "text-align" => Some(Property::TextAlign),
        "text-decoration" => Some(Property::TextDecoration),
        "text-indent" => Some(Property::TextIndent),
        "line-height" => Some(Property::LineHeight),
        "vertical-align" => Some(Property::VerticalAlign),
        "width" => Some(Property::Width),
        "height" => Some(Property::Height),
        "max-width" => Some(Property::MaxWidth),
        "min-width" => Some(Property::MinWidth),
        "max-height" => Some(Property::MaxHeight),
        "min-height" => Some(Property::MinHeight),
        "margin" => Some(Property::Margin),
        "margin-top" => Some(Property::MarginTop),
        "margin-right" => Some(Property::MarginRight),
        "margin-bottom" => Some(Property::MarginBottom),
        "margin-left" => Some(Property::MarginLeft),
        "padding" => Some(Property::Padding),
        "padding-top" => Some(Property::PaddingTop),
        "padding-right" => Some(Property::PaddingRight),
        "padding-bottom" => Some(Property::PaddingBottom),
        "padding-left" => Some(Property::PaddingLeft),
        "border" => Some(Property::Border),
        "border-top" => Some(Property::BorderTop),
        "border-right" => Some(Property::BorderRight),
        "border-bottom" => Some(Property::BorderBottom),
        "border-left" => Some(Property::BorderLeft),
        "border-color" => Some(Property::BorderColor),
        "border-width" => Some(Property::BorderWidth),
        "border-style" => Some(Property::BorderStyle),
        "border-radius" => Some(Property::BorderRadius),
        "border-collapse" => Some(Property::BorderCollapse),
        "border-spacing" => Some(Property::BorderSpacing),
        "list-style-type" => Some(Property::ListStyleType),
        "list-style" => Some(Property::ListStyleType),
        "white-space" => Some(Property::WhiteSpace),
        "overflow" => Some(Property::Overflow),
        "overflow-x" => Some(Property::OverflowX),
        "overflow-y" => Some(Property::OverflowY),
        // Positioning
        "position" => Some(Property::Position),
        "top" => Some(Property::Top),
        "right" => Some(Property::Right),
        "bottom" => Some(Property::Bottom),
        "left" => Some(Property::Left),
        "z-index" => Some(Property::ZIndex),
        // Flexbox
        "flex-direction" => Some(Property::FlexDirection),
        "flex-wrap" => Some(Property::FlexWrap),
        "justify-content" => Some(Property::JustifyContent),
        "align-items" => Some(Property::AlignItems),
        "align-self" => Some(Property::AlignSelf),
        "align-content" => Some(Property::AlignContent),
        "flex-grow" => Some(Property::FlexGrow),
        "flex-shrink" => Some(Property::FlexShrink),
        "flex-basis" => Some(Property::FlexBasis),
        "flex" => Some(Property::Flex),
        "gap" => Some(Property::Gap),
        "row-gap" => Some(Property::RowGap),
        "column-gap" => Some(Property::ColumnGap),
        "order" => Some(Property::Order),
        // Box model
        "box-sizing" => Some(Property::BoxSizing),
        // Float
        "float" => Some(Property::Float),
        "clear" => Some(Property::Clear),
        // Visual
        "opacity" => Some(Property::Opacity),
        "visibility" => Some(Property::Visibility),
        "text-transform" => Some(Property::TextTransform),
        "cursor" => Some(Property::Cursor),
        "table-layout" => Some(Property::TableLayout),
        _ => Option::None,
    }
}

// ---------------------------------------------------------------------------
// Value parser
// ---------------------------------------------------------------------------

pub fn parse_value(property: Property, value_str: &str) -> CssValue {
    let s = value_str.trim();
    if s.is_empty() {
        return CssValue::None;
    }

    // Check common keywords first
    let lower = to_ascii_lower(s);
    match lower.as_str() {
        "auto" => return CssValue::Auto,
        "none" => return CssValue::None,
        "inherit" => return CssValue::Inherit,
        "transparent" => return CssValue::Color(0x00000000),
        _ => {}
    }

    // var() — CSS custom property reference.
    if lower.starts_with("var(") {
        return parse_var_value(s);
    }

    // calc() — CSS math expression.
    if lower.starts_with("calc(") {
        return parse_calc_value(s);
    }

    // Color properties — try color parsing
    if is_color_property(property) {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    // Try color regardless of property if it starts with # or rgb
    if s.starts_with('#') || lower.starts_with("rgb") {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    // Try named colors for color properties
    if is_color_property(property) {
        if let Some(c) = named_color(&lower) {
            return CssValue::Color(c);
        }
    }

    // Try length/percentage/number
    if let Some(v) = try_parse_dimension(s) {
        return v;
    }

    // Fall back to keyword
    CssValue::Keyword(lower)
}

/// Parse `var(--name)` or `var(--name, fallback)`.
fn parse_var_value(s: &str) -> CssValue {
    // Strip "var(" and trailing ")".
    let inner = s.trim();
    let inner = if inner.starts_with("var(") || inner.starts_with("VAR(") {
        &inner[4..]
    } else { inner };
    let inner = inner.trim_end_matches(')').trim();

    // Split on first comma for fallback.
    if let Some(comma) = inner.find(',') {
        let name = inner[..comma].trim();
        let fallback_str = inner[comma + 1..].trim();
        let fallback = if fallback_str.is_empty() {
            None
        } else {
            Some(Box::new(parse_value(Property::Color, fallback_str)))
        };
        CssValue::Var(String::from(name), fallback)
    } else {
        CssValue::Var(String::from(inner), None)
    }
}

/// Parse `calc(expr)` into (px_component, pct_component).
/// Supports: `calc(100% - 32px)`, `calc(50% + 10px)`, `calc(16px * 2)`.
fn parse_calc_value(s: &str) -> CssValue {
    // Strip "calc(" and trailing ")".
    let inner = s.trim();
    let inner = if let Some(stripped) = inner.strip_prefix("calc(")
        .or_else(|| inner.strip_prefix("CALC("))
    {
        stripped
    } else { inner };
    let inner = inner.trim_end_matches(')').trim();

    // Try to find an operator (+ or - surrounded by spaces, or * or /).
    // We need to find the operator that splits the expression into two operands.
    let mut px: i32 = 0;
    let mut pct: i32 = 0;

    // Find the main binary operator (look for + or - with spaces).
    if let Some((left, op, right)) = split_calc_expr(inner) {
        let (lpx, lpct) = parse_calc_operand(left.trim());
        let (rpx, rpct) = parse_calc_operand(right.trim());

        match op {
            b'+' => { px = lpx + rpx; pct = lpct + rpct; }
            b'-' => { px = lpx - rpx; pct = lpct - rpct; }
            b'*' => {
                // Only one side should be a number (no unit).
                if lpct == 0 && rpct == 0 {
                    px = lpx * rpx / 100; // both are *100, one division to normalize
                } else {
                    px = lpx * rpx / 100;
                    pct = lpct * rpx / 100;
                }
            }
            b'/' => {
                if rpx != 0 {
                    px = lpx * 100 / rpx;
                    pct = lpct * 100 / rpx;
                }
            }
            _ => {}
        }
    } else {
        // Single operand — just parse it.
        let (p, pc) = parse_calc_operand(inner);
        px = p;
        pct = pc;
    }

    // If pure px, return as Length. If pure pct, return as Percentage.
    if pct == 0 {
        CssValue::Length(px, Unit::Px)
    } else if px == 0 {
        CssValue::Percentage(pct)
    } else {
        CssValue::Calc(px, pct)
    }
}

/// Split a calc expression on the main binary operator.
/// Handles `100% - 32px`, `50% + 10px`, `16px * 2`.
fn split_calc_expr(s: &str) -> Option<(&str, u8, &str)> {
    let bytes = s.as_bytes();
    // Look for ` + ` or ` - ` first (addition/subtraction have lower precedence).
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] == b'+' && bytes[i - 1] == b' ' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
            return Some((&s[..i - 1], b'+', &s[i + 2..]));
        }
        if bytes[i] == b'-' && bytes[i - 1] == b' ' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
            return Some((&s[..i - 1], b'-', &s[i + 2..]));
        }
    }
    // Look for * or / (no space requirement).
    for i in 0..bytes.len() {
        if bytes[i] == b'*' {
            return Some((&s[..i], b'*', &s[i + 1..]));
        }
        if bytes[i] == b'/' {
            return Some((&s[..i], b'/', &s[i + 1..]));
        }
    }
    None
}

/// Parse a single calc operand into (px * 100, pct * 100).
fn parse_calc_operand(s: &str) -> (i32, i32) {
    let s = s.trim();
    if s.ends_with('%') {
        let num = &s[..s.len() - 1];
        let val = parse_fixed_100(num);
        (0, val)
    } else if s.ends_with("px") {
        let num = &s[..s.len() - 2];
        let val = parse_fixed_100(num);
        (val, 0)
    } else if s.ends_with("em") {
        let num = &s[..s.len() - 2];
        let val = parse_fixed_100(num);
        // Treat em as px * 16 (approximate).
        (val * 16, 0)
    } else if s.ends_with("rem") {
        let num = &s[..s.len() - 3];
        let val = parse_fixed_100(num);
        (val * 16, 0)
    } else {
        // Pure number.
        let val = parse_fixed_100(s);
        (val, 0)
    }
}

/// Parse a number string into fixed-point * 100.
fn parse_fixed_100(s: &str) -> i32 {
    let s = s.trim();
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };
    let mut int_part: i32 = 0;
    let mut frac_part: i32 = 0;
    let mut in_frac = false;
    let mut frac_digits = 0;
    for b in s.as_bytes() {
        if *b == b'.' {
            in_frac = true;
            continue;
        }
        if *b >= b'0' && *b <= b'9' {
            if in_frac {
                if frac_digits < 2 {
                    frac_part = frac_part * 10 + (*b - b'0') as i32;
                    frac_digits += 1;
                }
            } else {
                int_part = int_part * 10 + (*b - b'0') as i32;
            }
        } else {
            break;
        }
    }
    // Pad fraction to 2 digits.
    while frac_digits < 2 {
        frac_part *= 10;
        frac_digits += 1;
    }
    let val = int_part * 100 + frac_part;
    if neg { -val } else { val }
}

fn is_color_property(p: Property) -> bool {
    matches!(
        p,
        Property::Color
            | Property::BackgroundColor
            | Property::Background
            | Property::BorderColor
    )
}

/// Check if a property is a shorthand that should be expanded in the parser.
fn is_expandable_shorthand(p: Property) -> bool {
    matches!(
        p,
        Property::Margin | Property::Padding | Property::Border
        | Property::Flex | Property::Gap | Property::Overflow
        | Property::Background
    )
}

/// Expand a shorthand property into individual declarations.
fn expand_shorthand(property: Property, value_str: &str) -> Vec<Declaration> {
    match property {
        Property::Margin => expand_box_shorthand(
            value_str,
            Property::MarginTop, Property::MarginRight,
            Property::MarginBottom, Property::MarginLeft,
        ),
        Property::Padding => expand_box_shorthand(
            value_str,
            Property::PaddingTop, Property::PaddingRight,
            Property::PaddingBottom, Property::PaddingLeft,
        ),
        Property::Border => expand_border_shorthand(value_str),
        Property::Flex => expand_flex_shorthand(value_str),
        Property::Gap => expand_gap_shorthand(value_str),
        Property::Overflow => expand_overflow_shorthand(value_str),
        Property::Background => expand_background_shorthand(value_str),
        _ => {
            let value = parse_value(property, value_str);
            let mut v = Vec::new();
            v.push(Declaration { property, value, important: false });
            v
        }
    }
}

/// Expand margin/padding shorthand: 1 value → all, 2 → TB/LR, 3 → T/LR/B, 4 → T/R/B/L.
fn expand_box_shorthand(
    value_str: &str,
    top: Property, right: Property, bottom: Property, left: Property,
) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let (t, r, b, l) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    let mut v = Vec::with_capacity(4);
    v.push(Declaration { property: top, value: parse_value(top, t), important: false });
    v.push(Declaration { property: right, value: parse_value(right, r), important: false });
    v.push(Declaration { property: bottom, value: parse_value(bottom, b), important: false });
    v.push(Declaration { property: left, value: parse_value(left, l), important: false });
    v
}

/// Expand `border: <width> <style> <color>` shorthand.
fn expand_border_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double"
            | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            decls.push(Declaration {
                property: Property::BorderStyle, value: CssValue::Keyword(lower), important: false,
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: Property::BorderColor, value: CssValue::Color(c), important: false,
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: Property::BorderColor, value: CssValue::Color(c), important: false,
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: Property::BorderWidth, value: dim, important: false,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: Property::BorderWidth, value: CssValue::Keyword(lower), important: false,
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: Property::BorderStyle, value: CssValue::None, important: false,
            });
            decls.push(Declaration {
                property: Property::BorderWidth, value: CssValue::Length(0, Unit::Px), important: false,
            });
        }
    }
    decls
}

/// Expand `flex: <grow> [<shrink>] [<basis>]` shorthand.
fn expand_flex_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();

    match lower.as_str() {
        "none" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(0), important: false });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(0), important: false });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto, important: false });
            return decls;
        }
        "auto" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(100), important: false });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto, important: false });
            return decls;
        }
        _ => {}
    }

    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return decls;
    }

    decls.push(Declaration {
        property: Property::FlexGrow, value: parse_value(Property::FlexGrow, parts[0]), important: false,
    });

    if parts.len() >= 2 {
        if let Some(dim) = try_parse_dimension(parts[1]) {
            if matches!(dim, CssValue::Length(_, _) | CssValue::Percentage(_)) {
                decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
                decls.push(Declaration { property: Property::FlexBasis, value: dim, important: false });
            } else {
                decls.push(Declaration { property: Property::FlexShrink, value: dim, important: false });
            }
        } else {
            decls.push(Declaration {
                property: Property::FlexShrink, value: parse_value(Property::FlexShrink, parts[1]), important: false,
            });
        }
    }

    if parts.len() >= 3 {
        decls.push(Declaration {
            property: Property::FlexBasis, value: parse_value(Property::FlexBasis, parts[2]), important: false,
        });
    }

    decls
}

/// Expand `gap: <row> [<column>]` shorthand.
fn expand_gap_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration { property: Property::RowGap, value: parse_value(Property::RowGap, parts[0]), important: false });
    let col = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::ColumnGap, value: parse_value(Property::ColumnGap, col), important: false });
    decls
}

/// Expand `overflow: <x> [<y>]` shorthand.
fn expand_overflow_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration { property: Property::OverflowX, value: parse_value(Property::OverflowX, parts[0]), important: false });
    let y = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::OverflowY, value: parse_value(Property::OverflowY, y), important: false });
    decls
}

/// Expand `background` shorthand — extract color and ignore image/repeat/position.
fn expand_background_shorthand(value_str: &str) -> Vec<Declaration> {
    let s = value_str.trim();
    let lower = to_ascii_lower(s);

    // Handle simple keywords.
    if lower == "none" || lower == "transparent" {
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Color(0x00000000),
            important: false,
        });
        return v;
    }
    if lower == "inherit" {
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Inherit,
            important: false,
        });
        return v;
    }

    // Scan tokens for a color value; skip url(...), gradient functions, and keywords
    // like no-repeat, center, cover, etc.
    let mut found_color: Option<u32> = None;
    let parts: Vec<&str> = split_background_tokens(s);
    for part in &parts {
        let pl = to_ascii_lower(part);
        // Skip url(...) and gradient functions.
        if pl.starts_with("url(") || pl.starts_with("linear-gradient(")
            || pl.starts_with("radial-gradient(") || pl.starts_with("conic-gradient(")
            || pl.starts_with("repeating-") {
            continue;
        }
        // Skip layout/repeat keywords.
        if matches!(pl.as_str(),
            "no-repeat" | "repeat" | "repeat-x" | "repeat-y"
            | "center" | "left" | "right" | "top" | "bottom"
            | "cover" | "contain" | "fixed" | "scroll" | "local"
            | "border-box" | "padding-box" | "content-box"
        ) {
            continue;
        }
        // Skip if it looks like a size (e.g., 100%, 50px, 0).
        if pl.ends_with('%') || pl.ends_with("px") || pl.ends_with("em")
            || pl.ends_with("rem") || pl.ends_with("vw") || pl.ends_with("vh") {
            continue;
        }
        // Try parsing as a color.
        if pl == "transparent" {
            found_color = Some(0x00000000);
            continue;
        }
        if let Some(c) = try_parse_color(part) {
            found_color = Some(c);
            continue;
        }
        if let Some(c) = named_color(&pl) {
            found_color = Some(c);
            continue;
        }
    }

    let mut v = Vec::new();
    if let Some(c) = found_color {
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Color(c),
            important: false,
        });
    }
    v
}

/// Split a `background` shorthand value into tokens, respecting parentheses.
fn split_background_tokens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            b',' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

fn try_parse_color(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    if bytes.first() == Some(&b'#') {
        return parse_hex_color(&s[1..]);
    }
    let lower = to_ascii_lower(s);
    if lower.starts_with("rgba(") && lower.ends_with(')') {
        return parse_rgba_func(&lower[5..lower.len() - 1]);
    }
    if lower.starts_with("rgb(") && lower.ends_with(')') {
        return parse_rgb_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("hsla(") && lower.ends_with(')') {
        return parse_hsla_func(&lower[5..lower.len() - 1]);
    }
    if lower.starts_with("hsl(") && lower.ends_with(')') {
        return parse_hsl_func(&lower[4..lower.len() - 1]);
    }
    named_color(&lower)
}

fn parse_hex_color(hex: &str) -> Option<u32> {
    let len = hex.len();
    match len {
        3 => {
            // #RGB -> AARRGGBB
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            Some(0xFF000000 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        4 => {
            // #RGBA
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            let a = hex_digit(hex.as_bytes()[3])? as u32;
            Some((a * 17) << 24 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        6 => {
            // #RRGGBB
            let v = parse_hex_u32(hex)?;
            Some(0xFF000000 | v)
        }
        8 => {
            // #RRGGBBAA
            let v = parse_hex_u32(hex)?;
            let rr = (v >> 24) & 0xFF;
            let gg = (v >> 16) & 0xFF;
            let bb = (v >> 8) & 0xFF;
            let aa = v & 0xFF;
            Some(aa << 24 | rr << 16 | gg << 8 | bb)
        }
        _ => Option::None,
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => Option::None,
    }
}

fn parse_hex_u32(hex: &str) -> Option<u32> {
    let mut val: u32 = 0;
    for &b in hex.as_bytes() {
        val = val.checked_shl(4)?;
        val |= hex_digit(b)? as u32;
    }
    Some(val)
}

fn parse_rgb_func(args: &str) -> Option<u32> {
    let parts = split_args(args);
    if parts.len() < 3 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    Some(0xFF000000 | (r << 16) | (g << 8) | b)
}

fn parse_rgba_func(args: &str) -> Option<u32> {
    let parts = split_args(args);
    if parts.len() < 4 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    // Alpha: could be 0.0-1.0 or 0-255
    let a_str = parts[3].trim();
    let a = if a_str.contains('.') {
        // Fractional: multiply by 255
        let fp = parse_fixed_point(a_str)?;
        ((fp * 255) / 100).max(0).min(255) as u32
    } else {
        parse_int(a_str)?.max(0).min(255) as u32
    };
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

fn parse_color_component(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.ends_with('%') {
        let pct = parse_int(&t[..t.len() - 1])?;
        Some(((pct.max(0).min(100) as u32) * 255) / 100)
    } else {
        Some(parse_int(t)?.max(0) as u32)
    }
}

fn split_args(s: &str) -> Vec<&str> {
    // Split on ',' or whitespace-separated (modern CSS syntax)
    if s.contains(',') {
        s.split(',').collect()
    } else {
        s.split_whitespace().collect()
    }
}

fn parse_hsl_func(args: &str) -> Option<u32> {
    let parts = split_args(args);
    if parts.len() < 3 { return Option::None; }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(0xFF000000 | (r << 16) | (g << 8) | b)
}

fn parse_hsla_func(args: &str) -> Option<u32> {
    let parts = split_args(args);
    if parts.len() < 4 { return Option::None; }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let a_str = parts[3].trim();
    let a = if a_str.contains('.') {
        let fp = parse_fixed_point(a_str)?;
        ((fp * 255) / 100).max(0).min(255) as u32
    } else if a_str.ends_with('%') {
        let pct = parse_int(&a_str[..a_str.len() - 1])?;
        ((pct.max(0).min(100) as u32) * 255) / 100
    } else {
        parse_int(a_str)?.max(0).min(255) as u32
    };
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

fn parse_hue(s: &str) -> Option<i32> {
    let t = s.trim();
    // Hue can be a number (degrees) or have "deg" suffix.
    let t = if t.ends_with("deg") { &t[..t.len() - 3] } else { t };
    parse_int(t.trim())
}

fn parse_percent_val(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.ends_with('%') {
        parse_int(&t[..t.len() - 1])
    } else {
        parse_int(t)
    }
}

/// Convert HSL to RGB. h in degrees [0..360], s and l in percent [0..100].
/// Returns (r, g, b) each in [0..255].
fn hsl_to_rgb(h: i32, s: i32, l: i32) -> (u32, u32, u32) {
    let h = ((h % 360) + 360) % 360;
    let s = s.max(0).min(100);
    let l = l.max(0).min(100);

    if s == 0 {
        let v = (l * 255 / 100) as u32;
        return (v, v, v);
    }

    // Use fixed-point * 1000 arithmetic.
    let l1000 = l as i64 * 10; // l in 0..1000
    let s1000 = s as i64 * 10;

    let q = if l1000 < 500 {
        l1000 * (1000 + s1000) / 1000
    } else {
        l1000 + s1000 - (l1000 * s1000 / 1000)
    };
    let p = 2 * l1000 - q;

    let r = hue_to_rgb_channel(p, q, h as i64 + 120);
    let g = hue_to_rgb_channel(p, q, h as i64);
    let b = hue_to_rgb_channel(p, q, h as i64 - 120);

    (r as u32, g as u32, b as u32)
}

fn hue_to_rgb_channel(p: i64, q: i64, mut h: i64) -> i64 {
    if h < 0 { h += 360; }
    if h >= 360 { h -= 360; }

    let val = if h < 60 {
        p + (q - p) * h / 60
    } else if h < 180 {
        q
    } else if h < 240 {
        p + (q - p) * (240 - h) / 60
    } else {
        p
    };

    (val * 255 / 1000).max(0).min(255)
}

fn named_color(name: &str) -> Option<u32> {
    match name {
        // Basic colors
        "black" => Some(0xFF000000),
        "white" => Some(0xFFFFFFFF),
        "red" => Some(0xFFFF0000),
        "green" => Some(0xFF008000),
        "lime" => Some(0xFF00FF00),
        "blue" => Some(0xFF0000FF),
        "yellow" => Some(0xFFFFFF00),
        "orange" => Some(0xFFFFA500),
        "purple" => Some(0xFF800080),
        "gray" | "grey" => Some(0xFF808080),
        "silver" => Some(0xFFC0C0C0),
        "cyan" | "aqua" => Some(0xFF00FFFF),
        "magenta" | "fuchsia" => Some(0xFFFF00FF),
        "navy" => Some(0xFF000080),
        "teal" => Some(0xFF008080),
        "maroon" => Some(0xFF800000),
        "olive" => Some(0xFF808000),
        "transparent" => Some(0x00000000),
        // Reds/pinks
        "indianred" => Some(0xFFCD5C5C),
        "lightcoral" => Some(0xFFF08080),
        "salmon" => Some(0xFFFA8072),
        "darksalmon" => Some(0xFFE9967A),
        "lightsalmon" => Some(0xFFFFA07A),
        "crimson" => Some(0xFFDC143C),
        "firebrick" => Some(0xFFB22222),
        "darkred" => Some(0xFF8B0000),
        "pink" => Some(0xFFFFC0CB),
        "lightpink" => Some(0xFFFFB6C1),
        "hotpink" => Some(0xFFFF69B4),
        "deeppink" => Some(0xFFFF1493),
        "mediumvioletred" => Some(0xFFC71585),
        "palevioletred" => Some(0xFFDB7093),
        // Oranges
        "coral" => Some(0xFFFF7F50),
        "tomato" => Some(0xFFFF6347),
        "orangered" => Some(0xFFFF4500),
        "darkorange" => Some(0xFFFF8C00),
        // Yellows
        "gold" => Some(0xFFFFD700),
        "lightyellow" => Some(0xFFFFFFE0),
        "lemonchiffon" => Some(0xFFFFFACD),
        "papayawhip" => Some(0xFFFFEFD5),
        "moccasin" => Some(0xFFFFE4B5),
        "peachpuff" => Some(0xFFFFDAB9),
        "palegoldenrod" => Some(0xFFEEE8AA),
        "khaki" => Some(0xFFF0E68C),
        "darkkhaki" => Some(0xFFBDB76B),
        // Greens
        "lawngreen" => Some(0xFF7CFC00),
        "chartreuse" => Some(0xFF7FFF00),
        "limegreen" => Some(0xFF32CD32),
        "forestgreen" => Some(0xFF228B22),
        "darkgreen" => Some(0xFF006400),
        "greenyellow" => Some(0xFFADFF2F),
        "yellowgreen" => Some(0xFF9ACD32),
        "springgreen" => Some(0xFF00FF7F),
        "mediumspringgreen" => Some(0xFF00FA9A),
        "lightgreen" => Some(0xFF90EE90),
        "palegreen" => Some(0xFF98FB98),
        "darkseagreen" => Some(0xFF8FBC8F),
        "mediumseagreen" => Some(0xFF3CB371),
        "seagreen" => Some(0xFF2E8B57),
        "olivedrab" => Some(0xFF6B8E23),
        "darkolivegreen" => Some(0xFF556B2F),
        // Cyans
        "lightcyan" => Some(0xFFE0FFFF),
        "paleturquoise" => Some(0xFFAFEEEE),
        "aquamarine" => Some(0xFF7FFFD4),
        "turquoise" => Some(0xFF40E0D0),
        "mediumturquoise" => Some(0xFF48D1CC),
        "darkturquoise" => Some(0xFF00CED1),
        "lightseagreen" => Some(0xFF20B2AA),
        "cadetblue" => Some(0xFF5F9EA0),
        "darkcyan" => Some(0xFF008B8B),
        // Blues
        "lightsteelblue" => Some(0xFFB0C4DE),
        "powderblue" => Some(0xFFB0E0E6),
        "lightblue" => Some(0xFFADD8E6),
        "skyblue" => Some(0xFF87CEEB),
        "lightskyblue" => Some(0xFF87CEFA),
        "deepskyblue" => Some(0xFF00BFFF),
        "dodgerblue" => Some(0xFF1E90FF),
        "cornflowerblue" => Some(0xFF6495ED),
        "steelblue" => Some(0xFF4682B4),
        "royalblue" => Some(0xFF4169E1),
        "mediumblue" => Some(0xFF0000CD),
        "darkblue" => Some(0xFF00008B),
        "midnightblue" => Some(0xFF191970),
        // Purples
        "lavender" => Some(0xFFE6E6FA),
        "thistle" => Some(0xFFD8BFD8),
        "plum" => Some(0xFFDDA0DD),
        "violet" => Some(0xFFEE82EE),
        "orchid" => Some(0xFFDA70D6),
        "mediumorchid" => Some(0xFFBA55D3),
        "mediumpurple" => Some(0xFF9370DB),
        "rebeccapurple" => Some(0xFF663399),
        "blueviolet" => Some(0xFF8A2BE2),
        "darkviolet" => Some(0xFF9400D3),
        "darkorchid" => Some(0xFF9932CC),
        "darkmagenta" => Some(0xFF8B008B),
        "indigo" => Some(0xFF4B0082),
        "slateblue" => Some(0xFF6A5ACD),
        "darkslateblue" => Some(0xFF483D8B),
        "mediumslateblue" => Some(0xFF7B68EE),
        // Browns
        "brown" => Some(0xFFA52A2A),
        "cornsilk" => Some(0xFFFFF8DC),
        "blanchedalmond" => Some(0xFFFFEBCD),
        "bisque" => Some(0xFFFFE4C4),
        "navajowhite" => Some(0xFFFFDEAD),
        "wheat" => Some(0xFFF5DEB3),
        "burlywood" => Some(0xFFDEB887),
        "tan" => Some(0xFFD2B48C),
        "rosybrown" => Some(0xFFBC8F8F),
        "sandybrown" => Some(0xFFF4A460),
        "goldenrod" => Some(0xFFDAA520),
        "darkgoldenrod" => Some(0xFFB8860B),
        "peru" => Some(0xFFCD853F),
        "chocolate" => Some(0xFFD2691E),
        "saddlebrown" => Some(0xFF8B4513),
        "sienna" => Some(0xFFA0522D),
        // Whites
        "snow" => Some(0xFFFFFAFA),
        "honeydew" => Some(0xFFF0FFF0),
        "mintcream" => Some(0xFFF5FFFA),
        "azure" => Some(0xFFF0FFFF),
        "aliceblue" => Some(0xFFF0F8FF),
        "ghostwhite" => Some(0xFFF8F8FF),
        "whitesmoke" => Some(0xFFF5F5F5),
        "seashell" => Some(0xFFFFF5EE),
        "beige" => Some(0xFFF5F5DC),
        "oldlace" => Some(0xFFFDF5E6),
        "floralwhite" => Some(0xFFFFFAF0),
        "ivory" => Some(0xFFFFFFF0),
        "antiquewhite" => Some(0xFFFAEBD7),
        "linen" => Some(0xFFFAF0E6),
        "lavenderblush" => Some(0xFFFFF0F5),
        "mistyrose" => Some(0xFFFFE4E1),
        // Grays
        "gainsboro" => Some(0xFFDCDCDC),
        "lightgray" | "lightgrey" => Some(0xFFD3D3D3),
        "darkgray" | "darkgrey" => Some(0xFFA9A9A9),
        "dimgray" | "dimgrey" => Some(0xFF696969),
        "lightslategray" | "lightslategrey" => Some(0xFF778899),
        "slategray" | "slategrey" => Some(0xFF708090),
        "darkslategray" | "darkslategrey" => Some(0xFF2F4F4F),
        _ => Option::None,
    }
}

// ---------------------------------------------------------------------------
// Dimension / number parsing
// ---------------------------------------------------------------------------

fn try_parse_dimension(s: &str) -> Option<CssValue> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }

    // Must start with a digit, '+', '-', or '.'
    let first = bytes[0];
    if !(first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.') {
        return Option::None;
    }

    // Find where the numeric part ends
    let mut i = 0;
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }

    if i == 0 || (i == 1 && (bytes[0] == b'-' || bytes[0] == b'+' || bytes[0] == b'.')) {
        return Option::None;
    }

    let num_str = core::str::from_utf8(&bytes[..i]).ok()?;
    let suffix = core::str::from_utf8(&bytes[i..]).ok()?.trim();
    let val = parse_fixed_point(num_str)?; // value * 100

    if suffix.is_empty() {
        // Pure number
        if val == 0 {
            // 0 with no unit = 0px
            return Some(CssValue::Length(0, Unit::Px));
        }
        return Some(CssValue::Number(val));
    }

    let lower_suffix = to_ascii_lower(suffix);
    match lower_suffix.as_str() {
        "px" => Some(CssValue::Length(val, Unit::Px)),
        "em" => Some(CssValue::Length(val, Unit::Em)),
        "rem" => Some(CssValue::Length(val, Unit::Rem)),
        "pt" => Some(CssValue::Length(val, Unit::Pt)),
        "%" => Some(CssValue::Percentage(val)),
        _ => Option::None,
    }
}

/// Parse a decimal string to fixed-point * 100.
/// "1.5" -> 150, "10" -> 1000, "-3.25" -> -325, "0.5" -> 50
fn parse_fixed_point(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }

    let mut i = 0;
    let negative = if bytes[i] == b'-' {
        i += 1;
        true
    } else if bytes[i] == b'+' {
        i += 1;
        false
    } else {
        false
    };

    let mut integer_part: i32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        integer_part = integer_part.wrapping_mul(10).wrapping_add((bytes[i] - b'0') as i32);
        i += 1;
    }

    let mut frac: i32 = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        // Read up to 2 decimal digits
        let d1 = if i < bytes.len() && bytes[i].is_ascii_digit() {
            let d = (bytes[i] - b'0') as i32;
            i += 1;
            d
        } else {
            0
        };
        let d2 = if i < bytes.len() && bytes[i].is_ascii_digit() {
            let d = (bytes[i] - b'0') as i32;
            // Skip remaining digits
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            d
        } else {
            0
        };
        frac = d1 * 10 + d2;
    }

    let val = integer_part * 100 + frac;
    Some(if negative { -val } else { val })
}

fn parse_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }
    let mut i = 0;
    let neg = if bytes[0] == b'-' {
        i += 1;
        true
    } else {
        false
    };
    let mut val: i32 = 0;
    if i >= bytes.len() {
        return Option::None;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        val = val * 10 + (bytes[i] - b'0') as i32;
        i += 1;
    }
    Some(if neg { -val } else { val })
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn to_ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

