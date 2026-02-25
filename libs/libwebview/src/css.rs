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
            // Expand shorthand properties into individual declarations.
            if is_expandable_shorthand(property) {
                let expanded = expand_shorthand(property, trimmed);
                for d in expanded {
                    decls.push(d);
                }
            } else {
                let value = parse_value(property, trimmed);
                decls.push(Declaration { property, value });
            }
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
        _ => {
            let value = parse_value(property, value_str);
            let mut v = Vec::new();
            v.push(Declaration { property, value });
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
    v.push(Declaration { property: top, value: parse_value(top, t) });
    v.push(Declaration { property: right, value: parse_value(right, r) });
    v.push(Declaration { property: bottom, value: parse_value(bottom, b) });
    v.push(Declaration { property: left, value: parse_value(left, l) });
    v
}

/// Expand `border: <width> <style> <color>` shorthand.
fn expand_border_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    for part in &parts {
        let lower = to_ascii_lower(part);
        // Check if it's a border style keyword.
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double"
            | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            decls.push(Declaration {
                property: Property::BorderStyle,
                value: CssValue::Keyword(lower),
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: Property::BorderColor,
                value: CssValue::Color(c),
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: Property::BorderColor,
                value: CssValue::Color(c),
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: Property::BorderWidth,
                value: dim,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: Property::BorderWidth,
                value: CssValue::Keyword(lower),
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: Property::BorderStyle,
                value: CssValue::None,
            });
            decls.push(Declaration {
                property: Property::BorderWidth,
                value: CssValue::Length(0, Unit::Px),
            });
        }
    }
    decls
}

/// Expand `flex: <grow> [<shrink>] [<basis>]` shorthand.
fn expand_flex_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();

    // Handle keyword values.
    match lower.as_str() {
        "none" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(0) });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(0) });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto });
            return decls;
        }
        "auto" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(100) });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100) });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto });
            return decls;
        }
        _ => {}
    }

    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return decls;
    }

    // First value: flex-grow (number).
    decls.push(Declaration {
        property: Property::FlexGrow,
        value: parse_value(Property::FlexGrow, parts[0]),
    });

    if parts.len() >= 2 {
        // Could be shrink or basis.
        if let Some(dim) = try_parse_dimension(parts[1]) {
            // If it has a unit, it's flex-basis.
            if matches!(dim, CssValue::Length(_, _) | CssValue::Percentage(_)) {
                decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100) });
                decls.push(Declaration { property: Property::FlexBasis, value: dim });
            } else {
                decls.push(Declaration { property: Property::FlexShrink, value: dim });
            }
        } else {
            decls.push(Declaration {
                property: Property::FlexShrink,
                value: parse_value(Property::FlexShrink, parts[1]),
            });
        }
    }

    if parts.len() >= 3 {
        decls.push(Declaration {
            property: Property::FlexBasis,
            value: parse_value(Property::FlexBasis, parts[2]),
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
    decls.push(Declaration { property: Property::RowGap, value: parse_value(Property::RowGap, parts[0]) });
    let col = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::ColumnGap, value: parse_value(Property::ColumnGap, col) });
    decls
}

/// Expand `overflow: <x> [<y>]` shorthand.
fn expand_overflow_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration { property: Property::OverflowX, value: parse_value(Property::OverflowX, parts[0]) });
    let y = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::OverflowY, value: parse_value(Property::OverflowY, y) });
    decls
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

