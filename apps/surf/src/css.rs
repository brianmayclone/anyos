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
}

pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone)]
pub enum Selector {
    Simple(SimpleSelector),
    Descendant(Box<Selector>, SimpleSelector),
    Universal,
}

#[derive(Clone)]
pub struct SimpleSelector {
    pub tag: Option<Tag>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

pub struct Declaration {
    pub property: Property,
    pub value: CssValue,
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
            Selector::Descendant(ancestor, leaf) => {
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
        let classes = self.classes.len() as u32;
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

    loop {
        p.skip_whitespace();
        if p.eof() {
            break;
        }

        // Skip at-rules
        if p.peek() == b'@' {
            p.pos += 1;
            let _keyword = p.read_ident();
            // Skip to opening brace or semicolon
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

    Stylesheet { rules }
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
        // Check for whitespace (descendant combinator) but NOT '{' or ','
        let had_space = skip_spaces_only(p);
        if p.eof() || p.peek() == b'{' || p.peek() == b',' {
            break;
        }
        if !had_space {
            break;
        }
        // We have a descendant combinator
        let next = parse_simple_selector(p);
        result = Selector::Descendant(Box::new(result), next);
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
}

fn parse_simple_selector(p: &mut Parser) -> SimpleSelector {
    let mut tag = Option::None;
    let mut id = Option::None;
    let mut classes = Vec::new();

    if p.peek() == b'*' {
        p.pos += 1;
        // Universal — but may still have .class or #id attached
    } else if p.peek().is_ascii_alphabetic() {
        let name = p.read_ident();
        tag = Some(Tag::from_str(&name));
    }

    // Parse chained #id and .class (no spaces between them)
    loop {
        if p.peek() == b'#' {
            p.pos += 1;
            id = Some(p.read_ident());
        } else if p.peek() == b'.' {
            p.pos += 1;
            classes.push(p.read_ident());
        } else {
            break;
        }
    }

    SimpleSelector { tag, id, classes }
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
            let value = parse_value(property, value_str.trim());
            decls.push(Declaration { property, value });
        }
    }

    decls
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
    let mut buf = [0u8; 32];
    let len = name.len().min(32);
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
        "list-style-type" => Some(Property::ListStyleType),
        "white-space" => Some(Property::WhiteSpace),
        "overflow" => Some(Property::Overflow),
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

    // Shorthand: for margin/padding take the first value
    if is_shorthand_box(property) {
        let first = s.split(|c: char| c == ' ' || c == '\t').next().unwrap_or(s);
        if first != s {
            return parse_value(property, first);
        }
    }

    // Fall back to keyword
    CssValue::Keyword(lower)
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

fn is_shorthand_box(p: Property) -> bool {
    matches!(p, Property::Margin | Property::Padding)
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

fn named_color(name: &str) -> Option<u32> {
    match name {
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
        "darkgray" | "darkgrey" => Some(0xFFA9A9A9),
        "lightgray" | "lightgrey" => Some(0xFFD3D3D3),
        "silver" => Some(0xFFC0C0C0),
        "cyan" | "aqua" => Some(0xFF00FFFF),
        "magenta" | "fuchsia" => Some(0xFFFF00FF),
        "pink" => Some(0xFFFFC0CB),
        "brown" => Some(0xFFA52A2A),
        "navy" => Some(0xFF000080),
        "teal" => Some(0xFF008080),
        "maroon" => Some(0xFF800000),
        "olive" => Some(0xFF808000),
        "coral" => Some(0xFFFF7F50),
        "salmon" => Some(0xFFFA8072),
        "gold" => Some(0xFFFFD700),
        "ivory" => Some(0xFFFFFFF0),
        "tomato" => Some(0xFFFF6347),
        "transparent" => Some(0x00000000),
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

