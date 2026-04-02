// CSS tokenizer + parser for surf browser
// no_std compatible, uses alloc for String/Vec

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::Tag;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// @media rules: each contains a query and the rules inside it.
    pub media_rules: Vec<MediaRule>,
    /// @keyframes blocks indexed by animation name.
    pub keyframes: Vec<KeyframeSet>,
    /// URLs from `@import url("...")` rules, in source order.
    pub imports: Vec<String>,
    /// `@font-face` declarations: (font_family, src_url).
    pub font_faces: Vec<FontFaceRule>,
}

/// A parsed `@font-face` rule.
#[derive(Clone)]
pub struct FontFaceRule {
    pub family: String,
    pub src_url: String,
    /// CSS font-weight (400 = normal, 700 = bold).
    pub weight: u32,
    /// CSS font-style: "normal" or "italic".
    pub italic: bool,
}

/// A complete `@keyframes name { … }` block.
#[derive(Clone)]
pub struct KeyframeSet {
    /// The animation name exactly as declared (case-sensitive after lowercase).
    pub name: String,
    /// Keyframe stops in declaration order (not necessarily sorted).
    pub stops: Vec<KeyframeStop>,
}

/// One stop inside a `@keyframes` block, e.g. `50% { opacity: 0; }`.
#[derive(Clone)]
pub struct KeyframeStop {
    /// Offset in the range 0–100 (percent).  `from` → 0, `to` → 100.
    pub offset: i32,
    pub declarations: Vec<Declaration>,
}

/// A @media rule: query + inner rules.
#[derive(Clone)]
pub struct MediaRule {
    pub query: MediaQuery,
    pub rules: Vec<Rule>,
}

/// Parsed @media query.
#[derive(Clone)]
pub struct MediaQuery {
    pub conditions: Vec<MediaCondition>,
    /// Media type restriction: None = all, Some("screen") = screen only, etc.
    pub media_type: MediaType,
}

/// Media type for @media rules.
#[derive(Clone, PartialEq)]
pub enum MediaType {
    /// Matches all media types (default, or `@media all`).
    All,
    /// Matches only screen (`@media screen`).
    Screen,
    /// Matches only print (`@media print`) — we never render for print.
    Print,
    /// Negated type: `@media not print` → matches everything except print.
    Not(Box<MediaType>),
}

/// A single media condition.
#[derive(Clone)]
pub enum MediaCondition {
    MinWidth(i32),
    MaxWidth(i32),
    MinHeight(i32),
    MaxHeight(i32),
    /// `prefers-color-scheme: dark` etc.
    PrefersColorScheme(String),
    /// `hover: hover` / `pointer: fine` / `prefers-reduced-motion: no-preference` etc.
    /// Stores whether we consider ourselves to satisfy this feature.
    Known(bool),
    /// Unknown media feature — treated as false (unknown = not supported).
    Unsupported,
}

#[derive(Clone)]
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
    pub pseudo_element: Option<PseudoElement>,
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

/// Pseudo-element (::before, ::after).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
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
    /// `:not(selector, ...)` — matches if NONE of the selectors match.
    Not(Vec<SimpleSelector>),
    /// `:is(selector, ...)` — matches if any selector in the list matches.
    Is(Vec<SimpleSelector>),
    /// `:where(selector, ...)` — same as :is() but zero specificity.
    Where(Vec<SimpleSelector>),
    /// `:has(selector)` — matches if the element has a descendant matching the selector.
    Has(Box<SimpleSelector>),
    Empty,
    Checked,
    Disabled,
    Enabled,
    Root,
    /// `:focus-visible` — like :focus but only when keyboard-navigated.
    FocusVisible,
    /// `:focus-within` — matches if the element or any descendant has focus.
    FocusWithin,
    /// `:placeholder-shown` — matches input elements showing placeholder text.
    PlaceholderShown,
}

#[derive(Clone)]
pub struct Declaration {
    pub property: Property,
    pub value: CssValue,
    pub important: bool,
}

#[derive(Clone, PartialEq, Eq)]
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
    ListStylePosition,
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
    // Typography (litehtml-inspired)
    FontFamily,
    LetterSpacing,
    WordSpacing,
    WordBreak,
    OverflowWrap,
    TextOverflow,
    // Per-side border properties
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomRightRadius,
    BorderBottomLeftRadius,
    // Outline (litehtml-inspired)
    Outline,
    OutlineColor,
    OutlineStyle,
    OutlineWidth,
    OutlineOffset,
    // Shadows (litehtml-inspired)
    BoxShadow,
    TextShadow,
    // Background extensions (litehtml-inspired)
    BackgroundImage,
    BackgroundPosition,
    BackgroundRepeat,
    BackgroundSize,
    // Transform
    Transform,
    TransformOrigin,
    // Content (for ::before/::after)
    Content,
    // Object-fit for replaced elements (img, video)
    ObjectFit,
    // Filter effects (litehtml-inspired)
    Filter,
    // Layout
    AspectRatio,
    Inset,
    ClipPath,
    Clip,
    // Text decoration sub-properties (CSS3)
    TextDecorationColor,
    TextDecorationStyle,
    TextDecorationThickness,
    TextUnderlineOffset,
    // Typography extras
    FontVariant,
    TabSize,
    // Counters
    CounterReset,
    CounterIncrement,
    // Display extensions
    // (display: contents handled in Display enum)
    // Table
    BorderCollapse,
    BorderSpacing,
    TableLayout,
    // Transitions
    Transition,
    TransitionProperty,
    TransitionDuration,
    TransitionTimingFunction,
    TransitionDelay,
    // Animations
    Animation,
    AnimationName,
    AnimationDuration,
    AnimationTimingFunction,
    AnimationDelay,
    AnimationIterationCount,
    AnimationDirection,
    AnimationFillMode,
    AnimationPlayState,
    // Grid container
    GridTemplateColumns,
    GridTemplateRows,
    GridTemplateAreas,
    GridTemplate,
    GridAutoColumns,
    GridAutoRows,
    GridAutoFlow,
    JustifyItems,
    // Grid item placement
    GridColumn,
    GridColumnStart,
    GridColumnEnd,
    GridRow,
    GridRowStart,
    GridRowEnd,
    GridArea,
    // Mask (parsed but not visually applied — makes @supports queries work)
    MaskImage,
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
    /// `currentColor` — resolved to the element's computed `color` property.
    CurrentColor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Pt,
    Percent,
    /// CSS `fr` unit (fractional share of free space in a grid).
    Fr,
    /// Viewport width (1vw = 1% of viewport width).
    Vw,
    /// Viewport height (1vh = 1% of viewport height).
    Vh,
    /// Minimum of vw and vh (1vmin = 1% of min(vw, vh)).
    Vmin,
    /// Maximum of vw and vh (1vmax = 1% of max(vw, vh)).
    Vmax,
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
        let tags = if self.tag.is_some() { 1 } else { 0 }
            + if self.pseudo_element.is_some() { 1 } else { 0 };
        (ids, classes, tags)
    }
}

impl Selector {
    /// Extract the pseudo-element from the innermost (rightmost) simple selector, if any.
    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        match self {
            Selector::Simple(s) => s.pseudo_element,
            Selector::Descendant(_, leaf)
            | Selector::Child(_, leaf)
            | Selector::AdjacentSibling(_, leaf)
            | Selector::GeneralSibling(_, leaf) => leaf.pseudo_element,
            Selector::Universal => None,
        }
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

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    crate::debug_surf!("[css] parse_stylesheet: {} bytes", css.len());
    let mut p = Parser::new(css);
    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();

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

            if kw_lower == "import" {
                // Parse @import url("...") or @import "..."
                p.skip_whitespace();
                let url = if p.starts_with(b"url(") {
                    p.pos += 4;
                    let q = if p.peek() == b'"' || p.peek() == b'\'' { p.advance() } else { 0 };
                    let start = p.pos;
                    while !p.eof() && p.peek() != b')' && (q == 0 || p.peek() != q) {
                        p.pos += 1;
                    }
                    let url = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
                    if q != 0 && p.peek() == q { p.pos += 1; }
                    if p.peek() == b')' { p.pos += 1; }
                    url
                } else if p.peek() == b'"' || p.peek() == b'\'' {
                    let q = p.advance();
                    let start = p.pos;
                    while !p.eof() && p.peek() != q { p.pos += 1; }
                    let url = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
                    if p.peek() == q { p.pos += 1; }
                    url
                } else {
                    String::new()
                };
                // Skip to semicolon
                while !p.eof() && p.peek() != b';' { p.pos += 1; }
                if p.peek() == b';' { p.pos += 1; }
                if !url.is_empty() {
                    imports.push(url);
                }
                continue;
            }

            if kw_lower == "font-face" {
                // Parse @font-face { font-family: ...; src: url(...); ... }
                p.skip_whitespace();
                if p.peek() == b'{' {
                    p.pos += 1;
                    let mut family = String::new();
                    let mut src_url = String::new();
                    let mut weight = 400u32;
                    let mut italic = false;
                    // Parse declarations until '}'
                    while !p.eof() && p.peek() != b'}' {
                        p.skip_whitespace();
                        if p.peek() == b'}' { break; }
                        let prop_name = p.read_ident();
                        p.skip_whitespace();
                        if p.peek() == b':' { p.pos += 1; }
                        p.skip_whitespace();
                        // Read value until ';' or '}'
                        let val_start = p.pos;
                        while !p.eof() && p.peek() != b';' && p.peek() != b'}' { p.pos += 1; }
                        let val = String::from_utf8_lossy(&p.input[val_start..p.pos]).into_owned();
                        if p.peek() == b';' { p.pos += 1; }
                        let prop_lower = prop_name.to_ascii_lowercase();
                        match prop_lower.as_str() {
                            "font-family" => {
                                family = val.trim().trim_matches('"').trim_matches('\'').into();
                            }
                            "src" => {
                                // Extract url(...) from src value
                                let v = val.trim();
                                if let Some(url_start) = v.find("url(") {
                                    let after = &v[url_start + 4..];
                                    let url_end = after.find(')').unwrap_or(after.len());
                                    let url = after[..url_end].trim().trim_matches('"').trim_matches('\'');
                                    src_url = String::from(url);
                                }
                            }
                            "font-weight" => {
                                let v = val.trim();
                                weight = match v {
                                    "bold" | "700" => 700,
                                    "normal" | "400" => 400,
                                    "100" => 100, "200" => 200, "300" => 300,
                                    "500" => 500, "600" => 600, "800" => 800, "900" => 900,
                                    _ => 400,
                                };
                            }
                            "font-style" => {
                                italic = val.trim() == "italic";
                            }
                            _ => {}
                        }
                    }
                    if p.peek() == b'}' { p.pos += 1; }
                    if !family.is_empty() && !src_url.is_empty() {
                        font_faces.push(FontFaceRule { family, src_url, weight, italic });
                    }
                }
                continue;
            }

            if kw_lower == "media" {
                // Parse @media query and inner rules.
                if let Some(mr) = parse_media_rule(&mut p) {
                    media_rules.push(mr);
                }
                continue;
            }

            if kw_lower == "keyframes" || kw_lower == "-webkit-keyframes" {
                if let Some(kf) = parse_keyframes(&mut p) {
                    keyframes.push(kf);
                }
                continue;
            }

            // @supports — evaluate the condition and include rules if supported.
            if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(&mut p) {
                    // @supports rules whose condition evaluates to true have their
                    // inner rules and media rules merged into the main lists.
                    for rule in sr.rules {
                        rules.push(rule);
                    }
                    for mr in sr.media_rules {
                        media_rules.push(mr);
                    }
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

    crate::debug_surf!("[css] parse_stylesheet done: {} rules, {} @media, {} @keyframes, {} imports",
        rules.len(), media_rules.len(), keyframes.len(), imports.len());
    Stylesheet { rules, media_rules, keyframes, imports, font_faces }
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
        // Handle nested at-rules inside @media.
        if p.peek() == b'@' {
            p.pos += 1;
            let kw = p.read_ident();
            let kw_lower = {
                let mut buf = [0u8; 32];
                let len = kw.len().min(32);
                for (i, &b) in kw.as_bytes()[..len].iter().enumerate() {
                    buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
                }
                String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
            };
            // Handle @supports nested inside @media.
            if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(p) {
                    for rule in sr.rules {
                        inner_rules.push(rule);
                    }
                    // Note: nested media rules inside @supports inside @media
                    // are dropped (three-level nesting not supported).
                }
            } else {
                // Skip other nested at-rules.
                loop {
                    p.skip_whitespace();
                    if p.eof() { break; }
                    if p.peek() == b'{' { p.skip_block(); break; }
                    if p.peek() == b';' { p.pos += 1; break; }
                    p.pos += 1;
                }
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
    let mut media_type = MediaType::All;
    let mut negated = false;

    // Split on "and" (case-insensitive).
    for part in split_and(trimmed) {
        let p = part.trim();
        if p.is_empty() { continue; }

        let lower = p.to_ascii_lowercase();

        // Track `not` modifier.
        if lower == "not" {
            negated = true;
            continue;
        }

        // Skip `only` modifier (has no effect on matching).
        if lower == "only" {
            continue;
        }

        // Recognize media types.
        if lower == "screen" {
            let mt = MediaType::Screen;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
            continue;
        }
        if lower == "print" {
            let mt = MediaType::Print;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
            continue;
        }
        if lower == "all" {
            let mt = MediaType::All;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
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

    MediaQuery { conditions, media_type }
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
/// Returns `Some(Unsupported)` for unknown features (so they evaluate to false).
fn parse_media_condition(inner: &str) -> Option<MediaCondition> {
    let inner = inner.trim();

    // Boolean media feature with no value: (color), (hover), etc.
    if !inner.contains(':') {
        let feature = inner.to_ascii_lowercase();
        return match feature.as_str() {
            // Features we know we support/don't support.
            "color" | "color-index" => Some(MediaCondition::Known(true)),
            "monochrome" => Some(MediaCondition::Known(false)),
            _ => Some(MediaCondition::Unsupported),
        };
    }

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
        // Interaction media features — we're a desktop browser with mouse.
        "hover" => Some(MediaCondition::Known(value_str == "hover")),
        "any-hover" => Some(MediaCondition::Known(value_str == "hover")),
        "pointer" => Some(MediaCondition::Known(value_str == "fine")),
        "any-pointer" => Some(MediaCondition::Known(value_str == "fine")),
        // Motion preferences — we don't animate, so treat as no-preference.
        "prefers-reduced-motion" => Some(MediaCondition::Known(value_str == "no-preference")),
        // Contrast preferences — we render standard contrast.
        "prefers-contrast" => Some(MediaCondition::Known(value_str == "no-preference")),
        // Data/update preferences.
        "prefers-reduced-data" | "prefers-reduced-transparency" => {
            Some(MediaCondition::Known(value_str == "no-preference"))
        }
        // Color gamut — we support sRGB.
        "color-gamut" => Some(MediaCondition::Known(value_str == "srgb")),
        // Resolution — assume standard 96dpi.
        "resolution" | "min-resolution" | "max-resolution" => {
            // Accept all — high-DPI media queries don't affect layout.
            Some(MediaCondition::Known(true))
        }
        // orientation — we're always landscape for wide viewports.
        "orientation" => {
            // True for landscape; false for portrait.
            Some(MediaCondition::Known(value_str == "landscape"))
        }
        // Dynamic viewport — unknown, skip.
        "dynamic-viewport-height" | "environment" => Some(MediaCondition::Unsupported),
        // Anything else unknown — treat as false.
        _ => Some(MediaCondition::Unsupported),
    }
}

/// Parse a CSS pixel value like "768px", "48rem", or "calc(640px - 1px)" into i32.
fn parse_px_value(s: &str) -> Option<i32> {
    let s = s.trim();

    // Handle calc() expressions — evaluate simple arithmetic at parse time.
    if s.to_ascii_lowercase().starts_with("calc(") {
        return eval_media_calc(s);
    }

    // Rem/em units — multiply by 16 (default root font size).
    if s.ends_with("rem") {
        let n = &s[..s.len() - 3];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }
    if s.ends_with("em") {
        let n = &s[..s.len() - 2];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }

    // Strip "px" and parse integer.
    let s = s.trim_end_matches("px").trim();
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

/// Parse a floating-point number string (no unit) into f32.
fn parse_float_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let mut result: f32 = 0.0;
    let mut frac: f32 = 0.0;
    let mut in_frac = false;
    let mut frac_div: f32 = 10.0;
    let mut has_digit = false;
    for b in s.as_bytes() {
        match b {
            b'0'..=b'9' => {
                has_digit = true;
                if in_frac {
                    frac += (*b - b'0') as f32 / frac_div;
                    frac_div *= 10.0;
                } else {
                    result = result * 10.0 + (*b - b'0') as f32;
                }
            }
            b'.' if !in_frac => { in_frac = true; }
            _ => break,
        }
    }
    if has_digit { Some(result + frac) } else { None }
}

/// Evaluate a simple calc() expression for @media conditions.
/// Only handles px/rem/em values with +, -, *, / operators.
/// Examples: "calc(640px - 1px)" → 639, "calc(48rem)" → 768
fn eval_media_calc(s: &str) -> Option<i32> {
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("calc(")?;
    // Strip trailing ')' — handle nested parens by finding the matching one.
    let inner = strip_outer_paren(inner)?;
    eval_calc_expr_px(inner)
}

/// Strip one layer of trailing ')' from a string that may have nested parens.
fn strip_outer_paren(s: &str) -> Option<&str> {
    // Just strip the last ')' — for simple media calc expressions this is enough.
    let s = s.trim();
    if s.ends_with(')') {
        Some(&s[..s.len() - 1])
    } else {
        Some(s)
    }
}

/// Evaluate a calc expression string to pixels (f32).
/// Supports px, rem, em units and +, -, *, / operators.
fn eval_calc_expr_px(s: &str) -> Option<i32> {
    let val = eval_calc_f32(s.trim())?;
    Some((val + 0.5) as i32)
}

/// Recursively evaluate a calc arithmetic expression, returning value in px as f32.
fn eval_calc_f32(s: &str) -> Option<f32> {
    let s = s.trim();

    // Find the last + or - operator (lowest precedence) respecting parentheses.
    // We scan right-to-left to get left-associativity.
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut split_pos: Option<usize> = None;
    let mut split_op: u8 = 0;
    // Scan right-to-left to handle: a - b - c = (a-b)-c correctly.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
                // Must be binary op, not unary (preceded by space or digit).
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' {
                    split_pos = Some(i);
                    split_op = bytes[i];
                    break;
                }
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let left = eval_calc_f32(&s[..pos])?;
        let right = eval_calc_f32(&s[pos + 1..])?;
        return Some(if split_op == b'+' { left + right } else { left - right });
    }

    // Find * or / at top level.
    depth = 0;
    split_pos = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                split_pos = Some(i);
                split_op = b;
                // Don't break — find last one for left-associativity.
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let left = eval_calc_f32(&s[..pos])?;
        let right = eval_calc_f32(&s[pos + 1..])?;
        return if split_op == b'*' {
            Some(left * right)
        } else if right != 0.0 {
            Some(left / right)
        } else {
            None
        };
    }

    // Atom — a number with optional unit.
    let s_lower = s.to_ascii_lowercase();
    let s_lower = s_lower.trim();

    // Nested calc()
    if s_lower.starts_with("calc(") {
        let inner = s_lower.strip_prefix("calc(")?;
        let inner = strip_outer_paren(inner)?;
        return eval_calc_f32(inner);
    }

    // Parenthesized expression
    if s_lower.starts_with('(') && s_lower.ends_with(')') {
        return eval_calc_f32(&s_lower[1..s_lower.len() - 1]);
    }

    if s_lower.ends_with("px") {
        return parse_float_px(&s_lower[..s_lower.len() - 2]);
    }
    if s_lower.ends_with("rem") {
        return parse_float_px(&s_lower[..s_lower.len() - 3]).map(|v| v * 16.0);
    }
    if s_lower.ends_with("em") {
        return parse_float_px(&s_lower[..s_lower.len() - 2]).map(|v| v * 16.0);
    }
    if s_lower.ends_with("vw") || s_lower.ends_with("vh") {
        // For media calc, treat vw/vh as 0 (viewport not known at parse time).
        return Some(0.0);
    }
    // Plain number (treat as px).
    let neg = s_lower.starts_with('-');
    let s2 = if neg { &s_lower[1..] } else { s_lower };
    parse_float_px(s2).map(|v| if neg { -v } else { v })
}

/// Evaluate a media query against viewport dimensions.
/// We always render as `screen` media type.
pub fn evaluate_media_query(query: &MediaQuery, viewport_width: i32, viewport_height: i32) -> bool {
    // Check media type first.  We are always "screen".
    match &query.media_type {
        MediaType::All => {}    // matches everything
        MediaType::Screen => {} // we ARE screen
        MediaType::Print => { return false; } // we are NOT print
        MediaType::Not(inner) => match inner.as_ref() {
            MediaType::Print => {}   // not print → we match (we're screen)
            MediaType::Screen => { return false; } // not screen → we don't match
            MediaType::All => { return false; } // not all → matches nothing
            MediaType::Not(_) => {}  // double negation → treat as all
        }
    }

    for cond in &query.conditions {
        let ok = match cond {
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinHeight(h) => viewport_height >= *h,
            MediaCondition::MaxHeight(h) => viewport_height <= *h,
            MediaCondition::PrefersColorScheme(scheme) => {
                // Report light theme — most sites default to dark-on-light text.
                scheme == "light"
            }
            MediaCondition::Known(v) => *v,
            MediaCondition::Unsupported => false,
        };
        if !ok { return false; }
    }
    true
}

/// Parse a `@keyframes name { stop { … } … }` block.
/// Parse `@supports (condition) { rules }`.
/// Evaluates whether the condition references supported properties.
/// Returns the inner rules if the condition is met, None otherwise.
/// Result of parsing a @supports block: plain rules + nested @media rules.
struct SupportsResult {
    rules: Vec<Rule>,
    media_rules: Vec<MediaRule>,
}

fn parse_supports_rule(p: &mut Parser) -> Option<SupportsResult> {
    p.skip_whitespace();

    // Read everything until '{' as the supports condition.
    let cond_start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let condition = core::str::from_utf8(&p.input[cond_start..p.pos]).unwrap_or("").trim();

    if p.eof() { return None; }
    p.pos += 1; // consume '{'

    // Parse inner rules (including nested @media).
    let mut inner_rules = Vec::new();
    let mut inner_media = Vec::new();
    loop {
        p.skip_whitespace();
        if p.eof() { break; }
        if p.peek() == b'}' {
            p.pos += 1;
            break;
        }
        if p.peek() == b'@' {
            p.pos += 1;
            let kw = p.read_ident();
            let kw_lower = {
                let mut buf = [0u8; 32];
                let len = kw.len().min(32);
                for (i, &b) in kw.as_bytes()[..len].iter().enumerate() {
                    buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
                }
                String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
            };
            if kw_lower == "media" {
                if let Some(mr) = parse_media_rule(p) {
                    inner_media.push(mr);
                }
            } else {
                // Skip other nested at-rules.
                loop {
                    p.skip_whitespace();
                    if p.eof() { break; }
                    if p.peek() == b'{' { p.skip_block(); break; }
                    if p.peek() == b';' { p.pos += 1; break; }
                    p.pos += 1;
                }
            }
            continue;
        }
        if let Some(rule) = parse_rule(p) {
            inner_rules.push(rule);
        }
    }

    // Evaluate the supports condition.
    if evaluate_supports_condition(condition) {
        Some(SupportsResult { rules: inner_rules, media_rules: inner_media })
    } else {
        None // condition not supported — discard rules
    }
}

/// Evaluate a simple @supports condition.
/// Supports: `(property: value)`, `not (...)`, `(...) and (...)`, `(...) or (...)`.
fn evaluate_supports_condition(cond: &str) -> bool {
    let cond = cond.trim();

    // Handle `not (...)`
    if cond.starts_with("not ") || cond.starts_with("not(") {
        let inner = cond[3..].trim().trim_start_matches('(').trim_end_matches(')');
        return !evaluate_supports_condition(inner);
    }

    // Handle `(...) and (...)`
    if cond.contains(") and (") {
        return cond.split(") and (")
            .all(|part| evaluate_supports_condition(part.trim_matches('(').trim_matches(')')));
    }

    // Handle `(...) or (...)`
    if cond.contains(") or (") {
        return cond.split(") or (")
            .any(|part| evaluate_supports_condition(part.trim_matches('(').trim_matches(')')));
    }

    // Simple `(property: value)` — check if property is known.
    let inner = cond.trim_matches('(').trim_matches(')').trim();
    if let Some(colon) = inner.find(':') {
        let prop_name = inner[..colon].trim();
        return parse_property(prop_name).is_some();
    }

    // Unknown condition — be conservative, assume supported.
    true
}

fn parse_keyframes(p: &mut Parser) -> Option<KeyframeSet> {
    p.skip_whitespace();

    // Read animation name (may be quoted or an ident).
    let name = if p.peek() == b'"' || p.peek() == b'\'' {
        p.pos += 1; // skip opening quote
        let start = p.pos;
        let q = p.input[p.pos - 1];
        while p.pos < p.input.len() && p.input[p.pos] != q {
            p.pos += 1;
        }
        let name = core::str::from_utf8(&p.input[start..p.pos]).unwrap_or("").to_ascii_lowercase();
        if !p.eof() { p.pos += 1; } // skip closing quote
        name
    } else {
        p.read_ident().to_ascii_lowercase()
    };

    if name.is_empty() {
        p.skip_block();
        return None;
    }

    p.skip_whitespace();
    if p.eof() || p.peek() != b'{' { return None; }
    p.pos += 1; // consume '{'

    let mut stops = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'}' {
            if !p.eof() { p.pos += 1; } // consume '}'
            break;
        }

        // Read keyframe selectors: `from`, `to`, `50%` separated by commas.
        let mut offsets: Vec<i32> = Vec::new();
        loop {
            p.skip_whitespace();
            let token_start = p.pos;
            while p.pos < p.input.len()
                && p.input[p.pos] != b','
                && p.input[p.pos] != b'{'
                && p.input[p.pos] != b'}' {
                p.pos += 1;
            }
            let token = core::str::from_utf8(&p.input[token_start..p.pos])
                .unwrap_or("").trim().to_ascii_lowercase();
            if !token.is_empty() {
                let offset = if token == "from" {
                    0
                } else if token == "to" {
                    100
                } else if let Some(pct_str) = token.strip_suffix('%') {
                    pct_str.trim().parse::<f32>().map(|v| v as i32).unwrap_or(0)
                } else {
                    0
                };
                offsets.push(offset);
            }
            p.skip_whitespace();
            if p.eof() || p.peek() != b',' { break; }
            p.pos += 1; // consume ','
        }

        p.skip_whitespace();
        if p.eof() || p.peek() != b'{' {
            while !p.eof() && p.peek() != b'}' { p.pos += 1; }
            continue;
        }
        // Parse the declarations block for this stop.
        let decls = parse_declarations_block(p);

        for offset in offsets {
            stops.push(KeyframeStop { offset, declarations: decls.clone() });
        }
    }

    stops.sort_by_key(|s| s.offset);
    Some(KeyframeSet { name, stops })
}

/// Parse a `{ declaration; ... }` block and return the declarations.
/// Expects the opening `{` to be the next character; consumes through the matching `}`.
fn parse_declarations_block(p: &mut Parser) -> Vec<Declaration> {
    if p.eof() || p.peek() != b'{' { return Vec::new(); }
    p.pos += 1; // consume '{'
    let start = p.pos;
    let mut depth = 1u32;
    while p.pos < p.input.len() {
        match p.input[p.pos] {
            b'{' => { depth += 1; p.pos += 1; }
            b'}' => {
                depth -= 1;
                p.pos += 1;
                if depth == 0 { break; }
            }
            _ => { p.pos += 1; }
        }
    }
    let block_text = core::str::from_utf8(&p.input[start..p.pos.saturating_sub(1)]).unwrap_or("");
    let mut inner = Parser::new(block_text);
    parse_declarations(&mut inner)
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
        && s.pseudo_element.is_none()
}

fn parse_simple_selector(p: &mut Parser) -> SimpleSelector {
    let mut tag = Option::None;
    let mut id = Option::None;
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudo_classes = Vec::new();
    let mut pseudo_element = Option::None;

    if p.peek() == b'*' {
        p.pos += 1;
    } else if p.peek().is_ascii_alphabetic() {
        let name = p.read_ident();
        tag = Some(Tag::from_str(&name));
    }

    // Parse chained #id, .class, [attr], :pseudo, ::pseudo-element (no spaces between them)
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
        } else if p.starts_with(b"::") {
            p.pos += 2;
            let name = p.read_ident();
            let lower = to_ascii_lower(&name);
            match lower.as_str() {
                "before" => pseudo_element = Some(PseudoElement::Before),
                "after" => pseudo_element = Some(PseudoElement::After),
                _ => {
                    // Skip unknown pseudo-element arguments
                    if p.peek() == b'(' {
                        let mut depth: u32 = 1;
                        p.pos += 1;
                        while !p.eof() && depth > 0 {
                            if p.peek() == b'(' { depth += 1; }
                            if p.peek() == b')' { depth -= 1; }
                            p.pos += 1;
                        }
                    }
                }
            }
        } else if p.peek() == b':' {
            // Single colon — could also be legacy :before/:after syntax
            p.pos += 1;
            let name = p.read_ident();
            let lower = to_ascii_lower(&name);
            match lower.as_str() {
                "before" => pseudo_element = Some(PseudoElement::Before),
                "after" => pseudo_element = Some(PseudoElement::After),
                _ => {
                    // Re-parse as pseudo-class by feeding the already-read name
                    if let Some(pc) = parse_pseudo_class_from_name(&lower, p) {
                        pseudo_classes.push(pc);
                    }
                }
            }
        } else {
            break;
        }
    }

    SimpleSelector { tag, id, classes, attrs, pseudo_classes, pseudo_element }
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
    parse_pseudo_class_from_name(&lower, p)
}

fn parse_pseudo_class_from_name(lower: &str, p: &mut Parser) -> Option<PseudoClass> {
    match lower {
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
                // Use parse_selector_list_in_parens so :not(.a, .b) works.
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Not(selectors))
            } else {
                Option::None
            }
        }
        "is" | "matches" | "-webkit-any" | "-moz-any" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Is(selectors))
            } else { Option::None }
        }
        "where" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Where(selectors))
            } else { Option::None }
        }
        "has" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let inner = parse_simple_selector(p);
                skip_spaces_only(p);
                if p.peek() == b')' { p.pos += 1; }
                Some(PseudoClass::Has(Box::new(inner)))
            } else { Option::None }
        }
        "focus-visible" => Some(PseudoClass::FocusVisible),
        "focus-within" => Some(PseudoClass::FocusWithin),
        "placeholder-shown" => Some(PseudoClass::PlaceholderShown),
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

/// Parse a comma-separated list of simple selectors inside parentheses: `(sel1, sel2, ...)`
fn parse_selector_list_in_parens(p: &mut Parser) -> Vec<SimpleSelector> {
    let mut selectors = Vec::new();
    if p.peek() != b'(' { return selectors; }
    p.pos += 1; // consume '('
    loop {
        skip_spaces_only(p);
        if p.eof() || p.peek() == b')' {
            if p.peek() == b')' { p.pos += 1; }
            break;
        }
        let sel = parse_simple_selector(p);
        selectors.push(sel);
        skip_spaces_only(p);
        if p.peek() == b',' {
            p.pos += 1;
        }
    }
    selectors
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

        // Check for CSS nesting: if the next char is a selector start (.#&*[)
        // or if we see a combinator, skip to the nested block.
        let ch = p.peek();
        if ch == b'.' || ch == b'#' || ch == b'&' || ch == b'*' || ch == b'[' || ch == b'>' || ch == b'+' || ch == b'~' {
            // Skip nested rule (CSS nesting).
            while !p.eof() && p.peek() != b'{' && p.peek() != b'}' {
                p.pos += 1;
            }
            if p.peek() == b'{' {
                p.skip_block();
            }
            continue;
        }

        let prop_name = p.read_ident();
        if prop_name.is_empty() {
            // Skip garbage character
            p.pos += 1;
            continue;
        }

        p.skip_whitespace();
        if p.peek() != b':' {
            // Could be a nested rule (CSS nesting) — skip the entire block.
            // Also handles selectors that look like property names (e.g. ".child { ... }").
            while !p.eof() && p.peek() != b';' && p.peek() != b'}' && p.peek() != b'{' {
                p.pos += 1;
            }
            if p.peek() == b'{' {
                p.skip_block(); // Skip the nested { ... } block
            } else if p.peek() == b';' {
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

        // Custom properties (--*) — store raw value as Keyword.
        if prop_name.starts_with("--") {
            let trimmed = value_str.trim();
            let (trimmed, important) = strip_important(trimmed);
            decls.push(Declaration {
                property: Property::CustomProperty(String::from(&prop_name)),
                value: CssValue::Keyword(String::from(trimmed)),
                important,
            });
        } else if prop_name == "font" || prop_name == "Font" {
            // `font` shorthand: [style] [variant] [weight] size[/line-height] family
            // Extract font-size and font-family from the shorthand.
            let trimmed = value_str.trim();
            let (trimmed, important) = strip_important(trimmed);
            let expanded = expand_font_shorthand(trimmed);
            for mut d in expanded {
                d.important = important;
                decls.push(d);
            }
        } else if let Some(property) = parse_property(&prop_name) {
            let trimmed = value_str.trim();
            // Detect and strip !important
            let (trimmed, important) = strip_important(trimmed);
            // Expand shorthand properties into individual declarations.
            if is_expandable_shorthand(&property) {
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
                let value = parse_value(&property, trimmed);
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
        // Per-side border width
        "border-top-width" => Some(Property::BorderTopWidth),
        "border-right-width" => Some(Property::BorderRightWidth),
        "border-bottom-width" => Some(Property::BorderBottomWidth),
        "border-left-width" => Some(Property::BorderLeftWidth),
        // Per-side border color
        "border-top-color" => Some(Property::BorderTopColor),
        "border-right-color" => Some(Property::BorderRightColor),
        "border-bottom-color" => Some(Property::BorderBottomColor),
        "border-left-color" => Some(Property::BorderLeftColor),
        // Per-side border style
        "border-top-style" => Some(Property::BorderTopStyle),
        "border-right-style" => Some(Property::BorderRightStyle),
        "border-bottom-style" => Some(Property::BorderBottomStyle),
        "border-left-style" => Some(Property::BorderLeftStyle),
        // Per-corner border radius
        "border-top-left-radius" => Some(Property::BorderTopLeftRadius),
        "border-top-right-radius" => Some(Property::BorderTopRightRadius),
        "border-bottom-right-radius" => Some(Property::BorderBottomRightRadius),
        "border-bottom-left-radius" => Some(Property::BorderBottomLeftRadius),
        "list-style-type" => Some(Property::ListStyleType),
        "list-style" => Some(Property::ListStyleType),
        "list-style-position" => Some(Property::ListStylePosition),
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
        // Typography
        "font-family" => Some(Property::FontFamily),
        "letter-spacing" => Some(Property::LetterSpacing),
        "word-spacing" => Some(Property::WordSpacing),
        "word-break" => Some(Property::WordBreak),
        "overflow-wrap" | "word-wrap" => Some(Property::OverflowWrap),
        "text-overflow" => Some(Property::TextOverflow),
        // Outline
        "outline" => Some(Property::Outline),
        "outline-color" => Some(Property::OutlineColor),
        "outline-style" => Some(Property::OutlineStyle),
        "outline-width" => Some(Property::OutlineWidth),
        "outline-offset" => Some(Property::OutlineOffset),
        // Shadows
        "box-shadow" => Some(Property::BoxShadow),
        "text-shadow" => Some(Property::TextShadow),
        // Background extensions
        "background-image" => Some(Property::BackgroundImage),
        "background-position" => Some(Property::BackgroundPosition),
        "background-repeat" => Some(Property::BackgroundRepeat),
        "background-size" => Some(Property::BackgroundSize),
        // Transform
        "transform" => Some(Property::Transform),
        "transform-origin" => Some(Property::TransformOrigin),
        // Content
        "content" => Some(Property::Content),
        "object-fit" => Some(Property::ObjectFit),
        // Filter
        "filter" | "-webkit-filter" => Some(Property::Filter),
        // Layout
        "aspect-ratio" => Some(Property::AspectRatio),
        "inset" => Some(Property::Inset),
        "clip-path" | "-webkit-clip-path" => Some(Property::ClipPath),
        "clip" => Some(Property::Clip),
        // Text decoration sub-properties
        "text-decoration-color" => Some(Property::TextDecorationColor),
        "text-decoration-style" => Some(Property::TextDecorationStyle),
        "text-decoration-thickness" => Some(Property::TextDecorationThickness),
        "text-underline-offset" => Some(Property::TextUnderlineOffset),
        // Typography extras
        "font-variant" => Some(Property::FontVariant),
        "tab-size" | "-moz-tab-size" => Some(Property::TabSize),
        // Counters
        "counter-reset" => Some(Property::CounterReset),
        "counter-increment" => Some(Property::CounterIncrement),
        // Transitions
        "transition"                  => Some(Property::Transition),
        "transition-property"         => Some(Property::TransitionProperty),
        "transition-duration"         => Some(Property::TransitionDuration),
        "transition-timing-function"  => Some(Property::TransitionTimingFunction),
        "transition-delay"            => Some(Property::TransitionDelay),
        // Animations
        "animation"                   => Some(Property::Animation),
        "animation-name"              => Some(Property::AnimationName),
        "animation-duration"          => Some(Property::AnimationDuration),
        "animation-timing-function"   => Some(Property::AnimationTimingFunction),
        "animation-delay"             => Some(Property::AnimationDelay),
        "animation-iteration-count"   => Some(Property::AnimationIterationCount),
        "animation-direction"         => Some(Property::AnimationDirection),
        "animation-fill-mode"         => Some(Property::AnimationFillMode),
        "animation-play-state"        => Some(Property::AnimationPlayState),
        // Grid
        "grid-template-columns" => Some(Property::GridTemplateColumns),
        "grid-template-rows"    => Some(Property::GridTemplateRows),
        "grid-template-areas"   => Some(Property::GridTemplateAreas),
        "grid-template"         => Some(Property::GridTemplate),
        "grid-auto-columns"     => Some(Property::GridAutoColumns),
        "grid-auto-rows"        => Some(Property::GridAutoRows),
        "grid-auto-flow"        => Some(Property::GridAutoFlow),
        "justify-items"         => Some(Property::JustifyItems),
        "grid-column"           => Some(Property::GridColumn),
        "grid-column-start"     => Some(Property::GridColumnStart),
        "grid-column-end"       => Some(Property::GridColumnEnd),
        "grid-row"              => Some(Property::GridRow),
        "grid-row-start"        => Some(Property::GridRowStart),
        "grid-row-end"          => Some(Property::GridRowEnd),
        "grid-area"             => Some(Property::GridArea),
        // Mask (parsed for @supports evaluation, not visually applied)
        "mask-image" | "-webkit-mask-image" | "mask" | "-webkit-mask" => Some(Property::MaskImage),
        _ => Option::None,
    }
}

// ---------------------------------------------------------------------------
// Value parser
// ---------------------------------------------------------------------------

pub fn parse_value(property: &Property, value_str: &str) -> CssValue {
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

    // min(), max(), clamp() — CSS comparison functions.
    if lower.starts_with("min(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Min);
    }
    if lower.starts_with("max(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Max);
    }
    if lower.starts_with("clamp(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Clamp);
    }

    // currentColor keyword — resolves to the element's computed `color` property.
    if lower == "currentcolor" {
        return CssValue::CurrentColor;
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

    // Fall back to keyword.
    // Grid placement properties use <custom-ident> which is case-sensitive per spec (§7.3).
    // Preserve original case for these properties.
    let is_case_sensitive = matches!(property,
        Property::GridColumn | Property::GridColumnStart | Property::GridColumnEnd
        | Property::GridRow | Property::GridRowStart | Property::GridRowEnd
        | Property::GridArea | Property::GridTemplateAreas
        | Property::FontFamily | Property::Content
    );
    if is_case_sensitive {
        CssValue::Keyword(String::from(s))
    } else {
        CssValue::Keyword(lower)
    }
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
            Some(Box::new(parse_value(&Property::Color, fallback_str)))
        };
        CssValue::Var(String::from(name), fallback)
    } else {
        CssValue::Var(String::from(inner), None)
    }
}

/// Parse `calc(expr)` into a CssValue.
/// Evaluates pure-px expressions to Length, pure-% to Percentage, mixed to Calc.
fn parse_calc_value(s: &str) -> CssValue {
    let s = s.trim();
    // Strip outer "calc(" and matching ")" — find the matching closing paren.
    let lower = s.to_ascii_lowercase();
    let inner = if lower.starts_with("calc(") {
        // Find the matching closing paren (last ')' in simple expressions).
        let without_prefix = &s[5..]; // after "calc("
        // Strip trailing ')'.
        let inner = without_prefix.trim_end_matches(')').trim();
        inner
    } else {
        s
    };

    // Use the more precise 2-component evaluator: (px*100, pct*100).
    let (px, pct) = eval_calc_components(inner);

    if pct == 0 {
        CssValue::Length(px, Unit::Px)
    } else if px == 0 {
        CssValue::Percentage(pct)
    } else {
        CssValue::Calc(px, pct)
    }
}

/// Evaluate a calc() expression into two components: (px * 100, pct * 100).
/// Supports: +, -, *, /, nested parens, rem/em/px/% units.
fn eval_calc_components(s: &str) -> (i32, i32) {
    let s = s.trim();

    // Find the last + or - operator at depth 0 (left-to-right lowest precedence).
    // Scan right-to-left so we handle e.g. "a - b - c" as "(a-b)-c".
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut split_i: Option<usize> = None;
    let mut split_op: u8 = 0;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' || prev == b'%' {
                    split_i = Some(i);
                    split_op = bytes[i];
                    break;
                }
            }
            _ => {}
        }
    }
    if let Some(pos) = split_i {
        let (lpx, lpct) = eval_calc_components(&s[..pos]);
        let (rpx, rpct) = eval_calc_components(&s[pos + 1..]);
        return if split_op == b'+' {
            (lpx + rpx, lpct + rpct)
        } else {
            (lpx - rpx, lpct - rpct)
        };
    }

    // Find * or / at top level (scan left for last occurrence for left-assoc).
    depth = 0;
    split_i = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                split_i = Some(i);
                split_op = b;
            }
            _ => {}
        }
    }
    if let Some(pos) = split_i {
        let (lpx, lpct) = eval_calc_components(&s[..pos]);
        let (rpx, rpct) = eval_calc_components(&s[pos + 1..]);
        if split_op == b'*' {
            // One side is always a pure number (no %).
            if lpct == 0 && rpct == 0 {
                // Both px-like, treat as: (lpx/100) * (rpx/100) * 100 = lpx*rpx/100
                return (lpx * rpx / 100, 0);
            } else if lpct == 0 {
                // Right has pct, left is multiplier
                let mul = lpx; // *100 fixed point
                return (0, lpct * mul / 100);
            } else {
                let mul = rpx;
                return (lpx * mul / 100, lpct * mul / 100);
            }
        } else {
            // Division: right must be a plain number.
            let div = rpx; // *100 fixed point
            if div != 0 {
                return (lpx * 100 / div, lpct * 100 / div);
            } else {
                return (0, 0);
            }
        }
    }

    // Atom.
    parse_calc_operand(s)
}

/// Split a calc expression on the main binary operator (respects parentheses).
/// Handles `100% - 32px`, `50% + 10px`, `16px * 2`.
fn split_calc_expr(s: &str) -> Option<(&str, u8, &str)> {
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
    // Look for ` + ` or ` - ` first (addition/subtraction have lower precedence).
    // Scan right-to-left for left-associativity.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => { if depth > 0 { depth -= 1; } }
            b'+' | b'-' if depth == 0 && i > 0 => {
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' {
                    let left = s[..i].trim_end();
                    let right = s[i + 1..].trim_start();
                    return Some((left, bytes[i], right));
                }
            }
            _ => {}
        }
    }
    // Look for * or / at top level.
    depth = 0;
    let mut last_mul_div: Option<(usize, u8)> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b'*' | b'/' if depth == 0 => { last_mul_div = Some((i, b)); }
            _ => {}
        }
    }
    if let Some((i, op)) = last_mul_div {
        return Some((&s[..i], op, &s[i + 1..]));
    }
    None
}

/// Parse a single calc operand into (px * 100, pct * 100).
fn parse_calc_operand(s: &str) -> (i32, i32) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let lower = lower.trim();

    // Nested calc() or parenthesized expression.
    if lower.starts_with("calc(") || (lower.starts_with('(') && lower.ends_with(')')) {
        let inner = if lower.starts_with("calc(") {
            &lower[5..lower.len() - 1]
        } else {
            &lower[1..lower.len() - 1]
        };
        return eval_calc_components(inner);
    }
    // min()/max()/clamp() as operand — evaluate to px approximation.
    if lower.starts_with("min(") || lower.starts_with("max(") || lower.starts_with("clamp(") {
        let func = if lower.starts_with("clamp(") { CssMathFunc::Clamp }
            else if lower.starts_with("min(") { CssMathFunc::Min }
            else { CssMathFunc::Max };
        match eval_min_max_clamp(lower, func) {
            CssValue::Length(v, Unit::Px) => return (v * 100, 0),
            CssValue::Percentage(p) => return (0, p),
            CssValue::Calc(px, pct) => return (px, pct),
            _ => {}
        }
    }

    if lower.ends_with('%') {
        let num = &lower[..lower.len() - 1];
        let val = parse_fixed_100(num);
        (0, val)
    } else if lower.ends_with("px") {
        let num = &lower[..lower.len() - 2];
        let val = parse_fixed_100(num);
        (val, 0)
    } else if lower.ends_with("rem") {
        let num = &lower[..lower.len() - 3];
        let val = parse_fixed_100(num);
        (val * 16, 0)
    } else if lower.ends_with("em") {
        let num = &lower[..lower.len() - 2];
        let val = parse_fixed_100(num);
        // Treat em as px * 16 (approximate).
        (val * 16, 0)
    } else if lower.ends_with("vw") || lower.ends_with("vh") || lower.ends_with("vmin") || lower.ends_with("vmax") {
        // Viewport units in calc — treated as percentage-like (resolved at layout time).
        let suffix_len = if lower.ends_with("vmin") || lower.ends_with("vmax") { 4 } else { 2 };
        let num = &lower[..lower.len() - suffix_len];
        let val = parse_fixed_100(num);
        (0, val)
    } else {
        // Pure number.
        let val = parse_fixed_100(s);
        (val, 0)
    }
}

enum CssMathFunc { Min, Max, Clamp }

/// Parse and evaluate min(), max(), clamp() CSS functions.
fn parse_min_max_clamp_value(s: &str, func: CssMathFunc) -> CssValue {
    let lower = s.to_ascii_lowercase();
    eval_min_max_clamp(&lower, func)
}

/// Split top-level comma-separated arguments (respecting parentheses).
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
    let mut start = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b',' if depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

/// Evaluate a min/max/clamp function expression. Expects lowercase input.
fn eval_min_max_clamp(s: &str, func: CssMathFunc) -> CssValue {
    // Find opening paren.
    let paren_start = match s.find('(') {
        Some(i) => i,
        None => return CssValue::None,
    };
    let inner = s[paren_start + 1..].trim_end_matches(')').trim();
    let args: Vec<&str> = split_top_level_commas(inner);

    // Evaluate each arg as a calc-like expression.
    let vals: Vec<(i32, i32)> = args.iter().map(|a| eval_calc_components(a)).collect();

    match func {
        CssMathFunc::Min => {
            // If all pure-px, return the minimum.
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let min_px = vals.iter().map(|(px, _)| *px).min().unwrap_or(0);
                return CssValue::Length(min_px / 100, Unit::Px);
            }
            // Mixed: return first arg as approximation.
            let (px, pct) = vals.first().copied().unwrap_or((0, 0));
            if pct == 0 { CssValue::Length(px / 100, Unit::Px) }
            else if px == 0 { CssValue::Percentage(pct) }
            else { CssValue::Calc(px, pct) }
        }
        CssMathFunc::Max => {
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let max_px = vals.iter().map(|(px, _)| *px).max().unwrap_or(0);
                return CssValue::Length(max_px / 100, Unit::Px);
            }
            let (px, pct) = vals.last().copied().unwrap_or((0, 0));
            if pct == 0 { CssValue::Length(px / 100, Unit::Px) }
            else if px == 0 { CssValue::Percentage(pct) }
            else { CssValue::Calc(px, pct) }
        }
        CssMathFunc::Clamp => {
            // clamp(min, val, max) — use val if all are pure-px, then clamp.
            if vals.len() >= 3 {
                let (min_px, min_pct) = vals[0];
                let (val_px, val_pct) = vals[1];
                let (max_px, max_pct) = vals[2];
                // If all pure-px, fully resolve.
                if min_pct == 0 && val_pct == 0 && max_pct == 0 {
                    let v = val_px.max(min_px).min(max_px);
                    return CssValue::Length(v / 100, Unit::Px);
                }
                // Otherwise return the middle (val) as best approximation.
                if val_pct == 0 { CssValue::Length(val_px / 100, Unit::Px) }
                else if val_px == 0 { CssValue::Percentage(val_pct) }
                else { CssValue::Calc(val_px, val_pct) }
            } else {
                // Malformed — return first arg.
                let (px, pct) = vals.first().copied().unwrap_or((0, 0));
                if pct == 0 { CssValue::Length(px / 100, Unit::Px) }
                else { CssValue::Percentage(pct) }
            }
        }
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

fn is_color_property(p: &Property) -> bool {
    matches!(
        p,
        Property::Color
            | Property::BackgroundColor
            | Property::Background
            | Property::BorderColor
            | Property::BorderTopColor
            | Property::BorderRightColor
            | Property::BorderBottomColor
            | Property::BorderLeftColor
            | Property::OutlineColor
            | Property::TextDecorationColor
    )
}

/// Check if a property is a shorthand that should be expanded in the parser.
fn is_expandable_shorthand(p: &Property) -> bool {
    matches!(
        p,
        Property::Margin | Property::Padding | Property::Border
        | Property::BorderTop | Property::BorderRight
        | Property::BorderBottom | Property::BorderLeft
        | Property::BorderRadius
        | Property::Outline
        | Property::Flex | Property::Gap | Property::Overflow
        | Property::Background
        | Property::TextDecoration
        | Property::Inset
        | Property::GridTemplate
        | Property::GridTemplateAreas
    )
}

/// Expand a shorthand property into individual declarations.
fn expand_shorthand(property: Property, value_str: &str) -> Vec<Declaration> {
    // If the ENTIRE value is a single var() call, don't expand the shorthand —
    // instead emit a single declaration with the primary property and var() value.
    // The var() will be resolved at style resolution time by apply_author_rules.
    let trimmed_lower = to_ascii_lower(value_str.trim());
    if trimmed_lower.starts_with("var(") && !trimmed_lower.contains(')') ||
       (trimmed_lower.starts_with("var(") && trimmed_lower.ends_with(')') && trimmed_lower.matches(')').count() == 1) {
        let primary = match &property {
            Property::Background => Property::BackgroundColor,
            Property::Outline => Property::OutlineColor,
            _ => property.clone(),
        };
        let var_val = parse_var_value(value_str.trim());
        return alloc::vec![Declaration { property: primary, value: var_val, important: false }];
    }

    match &property {
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
        Property::BorderTop => expand_border_side_shorthand(value_str,
            Property::BorderTopWidth, Property::BorderTopStyle, Property::BorderTopColor),
        Property::BorderRight => expand_border_side_shorthand(value_str,
            Property::BorderRightWidth, Property::BorderRightStyle, Property::BorderRightColor),
        Property::BorderBottom => expand_border_side_shorthand(value_str,
            Property::BorderBottomWidth, Property::BorderBottomStyle, Property::BorderBottomColor),
        Property::BorderLeft => expand_border_side_shorthand(value_str,
            Property::BorderLeftWidth, Property::BorderLeftStyle, Property::BorderLeftColor),
        Property::BorderRadius => expand_border_radius_shorthand(value_str),
        Property::Outline => expand_outline_shorthand(value_str),
        Property::TextDecoration => expand_text_decoration_shorthand(value_str),
        Property::Inset => expand_box_shorthand(
            value_str,
            Property::Top, Property::Right, Property::Bottom, Property::Left,
        ),
        Property::GridTemplate => expand_grid_template_shorthand(value_str),
        Property::GridTemplateAreas => expand_grid_template_areas(value_str),
        _ => {
            let value = parse_value(&property, value_str);
            let mut v = Vec::new();
            v.push(Declaration { property: property.clone(), value, important: false });
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
    let v_t = parse_value(&top, t);
    v.push(Declaration { property: top, value: v_t, important: false });
    let v_r = parse_value(&right, r);
    v.push(Declaration { property: right, value: v_r, important: false });
    let v_b = parse_value(&bottom, b);
    v.push(Declaration { property: bottom, value: v_b, important: false });
    let v_l = parse_value(&left, l);
    v.push(Declaration { property: left, value: v_l, important: false });
    v
}

/// Reassemble var() calls that were split across whitespace-delimited parts.
/// E.g. ["1px", "solid", "var(", "--color-grey", ")"] → ["1px", "solid", "var( --color-grey )"]
fn reassemble_var_parts(parts: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let lower = parts[i].to_ascii_lowercase();
        if lower.starts_with("var(") && !lower.ends_with(')') {
            // Collect parts until we find one ending with ')'
            let mut combined = String::from(parts[i]);
            i += 1;
            while i < parts.len() {
                combined.push(' ');
                combined.push_str(parts[i]);
                if parts[i].ends_with(')') {
                    i += 1;
                    break;
                }
                i += 1;
            }
            result.push(combined);
        } else {
            result.push(String::from(parts[i]));
            i += 1;
        }
    }
    result
}

/// Expand `border: <width> <style> <color>` shorthand.
/// Sets both the unified properties AND per-side properties (like litehtml).
fn expand_border_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    // Reassemble var() calls that span multiple whitespace-split parts.
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    let mut width_val: Option<CssValue> = None;
    let mut style_val: Option<CssValue> = None;
    let mut color_val: Option<CssValue> = None;
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double"
            | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            style_val = Some(CssValue::Keyword(lower));
        } else if lower.starts_with("var(") {
            // var() reference — store as Var for later resolution.
            color_val = Some(parse_var_value(part));
        } else if let Some(c) = try_parse_color(part) {
            color_val = Some(CssValue::Color(c));
        } else if let Some(c) = named_color(&lower) {
            color_val = Some(CssValue::Color(c));
        } else if let Some(dim) = try_parse_dimension(part) {
            width_val = Some(dim);
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            width_val = Some(CssValue::Keyword(lower));
        } else if lower == "none" {
            style_val = Some(CssValue::None);
            width_val = Some(CssValue::Length(0, Unit::Px));
        }
    }
    // Emit unified properties
    if let Some(ref sv) = style_val {
        decls.push(Declaration { property: Property::BorderStyle, value: sv.clone(), important: false });
    }
    if let Some(ref cv) = color_val {
        decls.push(Declaration { property: Property::BorderColor, value: cv.clone(), important: false });
    }
    if let Some(ref wv) = width_val {
        decls.push(Declaration { property: Property::BorderWidth, value: wv.clone(), important: false });
    }
    // Emit per-side properties for consistent per-side override support
    for side_w in &[Property::BorderTopWidth, Property::BorderRightWidth,
                    Property::BorderBottomWidth, Property::BorderLeftWidth] {
        if let Some(ref wv) = width_val {
            decls.push(Declaration { property: side_w.clone(), value: wv.clone(), important: false });
        }
    }
    for side_s in &[Property::BorderTopStyle, Property::BorderRightStyle,
                    Property::BorderBottomStyle, Property::BorderLeftStyle] {
        if let Some(ref sv) = style_val {
            decls.push(Declaration { property: side_s.clone(), value: sv.clone(), important: false });
        }
    }
    for side_c in &[Property::BorderTopColor, Property::BorderRightColor,
                    Property::BorderBottomColor, Property::BorderLeftColor] {
        if let Some(ref cv) = color_val {
            decls.push(Declaration { property: side_c.clone(), value: cv.clone(), important: false });
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
        property: Property::FlexGrow, value: parse_value(&Property::FlexGrow, parts[0]), important: false,
    });

    // CSS spec: `flex: <number>` is shorthand for `flex: <number> 1 0`.
    // If only one value, set shrink=1 and basis=0 (not auto).
    if parts.len() == 1 {
        decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
        decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Length(0, Unit::Px), important: false });
        return decls;
    }

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
                property: Property::FlexShrink, value: parse_value(&Property::FlexShrink, parts[1]), important: false,
            });
        }
    }

    if parts.len() >= 3 {
        decls.push(Declaration {
            property: Property::FlexBasis, value: parse_value(&Property::FlexBasis, parts[2]), important: false,
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
    decls.push(Declaration { property: Property::RowGap, value: parse_value(&Property::RowGap, parts[0]), important: false });
    let col = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::ColumnGap, value: parse_value(&Property::ColumnGap, col), important: false });
    decls
}

/// Expand `overflow: <x> [<y>]` shorthand.
fn expand_overflow_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration { property: Property::OverflowX, value: parse_value(&Property::OverflowX, parts[0]), important: false });
    let y = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration { property: Property::OverflowY, value: parse_value(&Property::OverflowY, y), important: false });
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

    // Handle var() — store as Var for later resolution by apply_author_rules.
    if lower.starts_with("var(") {
        let var_val = parse_var_value(s);
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: var_val,
            important: false,
        });
        return v;
    }

    // Scan tokens for a color value; skip url(...), gradient functions, and keywords
    // like no-repeat, center, cover, etc.
    let mut found_color: Option<u32> = None;
    let mut found_var: Option<CssValue> = None;
    let raw_parts: Vec<&str> = split_background_tokens(s);
    let parts = reassemble_var_parts(&raw_parts.iter().map(|s| *s).collect::<Vec<&str>>());
    for part in &parts {
        let pl = to_ascii_lower(part);
        // Handle var() reference within background shorthand.
        if pl.starts_with("var(") {
            found_var = Some(parse_var_value(part));
            continue;
        }
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
    } else if let Some(var_val) = found_var {
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: var_val,
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

/// Expand `border-top/right/bottom/left: <width> <style> <color>` per-side shorthand.
fn expand_border_side_shorthand(
    value_str: &str,
    width_prop: Property, style_prop: Property, color_prop: Property,
) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double"
            | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            decls.push(Declaration {
                property: style_prop.clone(), value: CssValue::Keyword(lower), important: false,
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: style_prop.clone(), value: CssValue::None, important: false,
            });
            decls.push(Declaration {
                property: width_prop.clone(), value: CssValue::Length(0, Unit::Px), important: false,
            });
        } else if lower.starts_with("var(") {
            decls.push(Declaration {
                property: color_prop.clone(), value: parse_var_value(part), important: false,
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: color_prop.clone(), value: CssValue::Color(c), important: false,
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: color_prop.clone(), value: CssValue::Color(c), important: false,
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: width_prop.clone(), value: dim, important: false,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: width_prop.clone(), value: CssValue::Keyword(lower), important: false,
            });
        }
    }
    decls
}

/// Expand `border-radius: <tl> [<tr>] [<br>] [<bl>]` shorthand.
fn expand_border_radius_shorthand(value_str: &str) -> Vec<Declaration> {
    // Ignore elliptical syntax (/) for simplicity — only use the first set.
    let s = if let Some(pos) = value_str.find('/') {
        &value_str[..pos]
    } else {
        value_str
    };
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let (tl, tr, br, bl) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    let mut v = Vec::with_capacity(4);
    v.push(Declaration { property: Property::BorderTopLeftRadius,     value: parse_value(&Property::BorderTopLeftRadius, tl),     important: false });
    v.push(Declaration { property: Property::BorderTopRightRadius,    value: parse_value(&Property::BorderTopRightRadius, tr),    important: false });
    v.push(Declaration { property: Property::BorderBottomRightRadius, value: parse_value(&Property::BorderBottomRightRadius, br), important: false });
    v.push(Declaration { property: Property::BorderBottomLeftRadius,  value: parse_value(&Property::BorderBottomLeftRadius, bl),  important: false });
    v
}

/// Expand `outline: <width> <style> <color>` shorthand.
fn expand_outline_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double"
            | "groove" | "ridge" | "inset" | "outset") {
            decls.push(Declaration {
                property: Property::OutlineStyle, value: CssValue::Keyword(lower), important: false,
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: Property::OutlineStyle, value: CssValue::None, important: false,
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: Property::OutlineColor, value: CssValue::Color(c), important: false,
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: Property::OutlineColor, value: CssValue::Color(c), important: false,
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: Property::OutlineWidth, value: dim, important: false,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: Property::OutlineWidth, value: CssValue::Keyword(lower), important: false,
            });
        }
    }
    decls
}

/// Expand `text-decoration: <line> [<style>] [<color>]` shorthand (CSS3).
/// We keep it simple: extract underline/line-through/none and store as keyword.
fn expand_text_decoration_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();
    // Extract the line value (underline, line-through, overline, none)
    let line_kw = if lower.contains("underline") {
        "underline"
    } else if lower.contains("line-through") {
        "line-through"
    } else if lower.contains("overline") {
        "overline"
    } else if lower.contains("none") {
        "none"
    } else {
        "none"
    };
    decls.push(Declaration {
        property: Property::TextDecoration,
        value: CssValue::Keyword(String::from(line_kw)),
        important: false,
    });
    decls
}

// ---------------------------------------------------------------------------
// Color parsing
/// Expand `font` shorthand: `[style] [variant] [weight] size[/line-height] family`
/// Extracts font-size and font-family, ignoring style/variant/weight for simplicity.
/// Expand `grid-template: rows / columns` shorthand.
///
/// Handles two forms (CSS Grid §7.4):
/// - Simple: `track-list / track-list`  →  rows / columns
/// - Interleaved: `"area row" track-size "area row" track-size / columns`
///   Extracts grid-template-areas + grid-template-rows + grid-template-columns.
fn expand_grid_template_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let s = value_str.trim();

    if let Some(slash_pos) = find_grid_template_slash(s) {
        let rows_str = s[..slash_pos].trim();
        let cols_str = s[slash_pos + 1..].trim();

        // If rows_str contains quoted strings, it's the interleaved areas+rows format:
        // "area col1 col2" row-track-size ...
        if rows_str.contains('\'') || rows_str.contains('"') {
            let (area_rows, row_tracks) = parse_interleaved_areas_rows(rows_str);
            if !area_rows.is_empty() {
                // Join quoted area strings into one grid-template-areas value.
                let mut areas_val = String::new();
                for (i, r) in area_rows.iter().enumerate() {
                    if i > 0 { areas_val.push(' '); }
                    areas_val.push_str(r);
                }
                decls.push(Declaration {
                    property: Property::GridTemplateAreas,
                    value: CssValue::Keyword(areas_val),
                    important: false,
                });
            }
            if !row_tracks.is_empty() {
                let tracks_val = row_tracks.join(" ");
                decls.push(Declaration {
                    property: Property::GridTemplateRows,
                    value: CssValue::Keyword(tracks_val),
                    important: false,
                });
            }
        } else if !rows_str.is_empty() {
            decls.push(Declaration {
                property: Property::GridTemplateRows,
                value: CssValue::Keyword(String::from(rows_str)),
                important: false,
            });
        }
        if !cols_str.is_empty() {
            decls.push(Declaration {
                property: Property::GridTemplateColumns,
                value: CssValue::Keyword(String::from(cols_str)),
                important: false,
            });
        }
    } else {
        // No slash — might be just rows or areas.
        if s.contains('\'') || s.contains('"') {
            return expand_grid_template_areas(s);
        }
        decls.push(Declaration {
            property: Property::GridTemplateRows,
            value: CssValue::Keyword(String::from(s)),
            important: false,
        });
    }
    decls
}

/// Parse the interleaved `grid-template` rows format:
/// `"area col1 col2" track-size "area col1 col2" track-size ...`
///
/// Returns (Vec of quoted area row strings, Vec of row track size strings).
/// Area rows without an explicit track size get "auto" inserted.
fn parse_interleaved_areas_rows(s: &str) -> (Vec<String>, Vec<String>) {
    let mut area_rows: Vec<String> = Vec::new();
    let mut row_tracks: Vec<String> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip whitespace and newlines.
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
        if i >= bytes.len() { break; }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            // Quoted area row: collect including the quotes.
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != quote { i += 1; }
            if i < bytes.len() { i += 1; } // skip closing quote
            area_rows.push(String::from(&s[start..i]));

            // Look ahead: is the next non-whitespace token a track size (not a quote)?
            let mut j = i;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') { j += 1; }
            if j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'"' {
                // Consume the track size token (respects parentheses).
                let track_start = j;
                let mut depth: u32 = 0;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => { if depth > 0 { depth -= 1; } }
                        b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                if track_start < j {
                    row_tracks.push(String::from(&s[track_start..j]));
                } else {
                    row_tracks.push(String::from("auto"));
                }
                i = j;
            } else {
                // No explicit row track size — use auto.
                row_tracks.push(String::from("auto"));
            }
        } else {
            // Non-quoted token outside an area row — skip (e.g. line names [...]).
            while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
        }
    }

    (area_rows, row_tracks)
}

/// Find the '/' in a grid-template value that separates rows from columns.
/// Must skip '/' inside parentheses (e.g. `minmax(0,1fr)`).
fn find_grid_template_slash(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: u32 = 0;
    let mut in_quote = false;
    let mut quote_char: u8 = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                if !in_quote {
                    in_quote = true;
                    quote_char = bytes[i];
                } else if bytes[i] == quote_char {
                    in_quote = false;
                }
            }
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote => { if depth > 0 { depth -= 1; } }
            b'/' if !in_quote && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Parse `grid-template-areas` value.
/// Example: `'header header' 'sidebar content' 'footer footer'`
/// Each quoted string defines one row. Area names map to grid positions.
/// Emits a GridTemplateAreas keyword value that the style resolver will parse.
fn expand_grid_template_areas(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    // Store the raw areas string as a keyword — the grid layout engine will parse it.
    decls.push(Declaration {
        property: Property::GridTemplateAreas,
        value: CssValue::Keyword(String::from(value_str.trim())),
        important: false,
    });
    decls
}

fn expand_font_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() { return decls; }

    let style_weight_keywords = [
        "normal", "italic", "oblique",    // font-style
        "bold", "bolder", "lighter",      // font-weight
        "small-caps",                      // font-variant
        "100", "200", "300", "400", "500", "600", "700", "800", "900",
    ];

    let mut font_size_idx = None;
    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        // font-style / font-weight / font-variant keywords → skip
        if style_weight_keywords.contains(&lower.as_str()) {
            // Emit font-weight if bold
            if lower == "bold" || lower == "bolder" {
                decls.push(Declaration {
                    property: Property::FontWeight,
                    value: CssValue::Keyword(String::from("bold")),
                    important: false,
                });
            } else if lower == "italic" || lower == "oblique" {
                decls.push(Declaration {
                    property: Property::FontStyle,
                    value: CssValue::Keyword(lower),
                    important: false,
                });
            }
            continue;
        }
        // This must be the font-size (possibly with /line-height)
        font_size_idx = Some(i);
        break;
    }

    if let Some(si) = font_size_idx {
        let size_part = parts[si];
        // Handle size/line-height (e.g. "14px/1.5")
        let (size_str, lh_str) = if let Some(slash) = size_part.find('/') {
            (&size_part[..slash], Some(&size_part[slash + 1..]))
        } else {
            (size_part, None)
        };

        let size_val = parse_value(&Property::FontSize, size_str);
        decls.push(Declaration { property: Property::FontSize, value: size_val, important: false });

        if let Some(lh) = lh_str {
            let lh_val = parse_value(&Property::LineHeight, lh);
            decls.push(Declaration { property: Property::LineHeight, value: lh_val, important: false });
        }

        // Everything after the font-size is the font-family
        if si + 1 < parts.len() {
            let family = parts[si + 1..].join(" ");
            decls.push(Declaration {
                property: Property::FontFamily,
                value: CssValue::Keyword(family),
                important: false,
            });
        }
    }

    decls
}

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
    // Modern CSS: rgb(R G B) or rgb(R G B / alpha)
    // Tailwind: rgb(R G B/var(--tw-bg-opacity,1))
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    if let Some(alpha_str) = alpha_part {
        let a = parse_alpha_value(alpha_str);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else if parts.len() >= 4 {
        // Legacy: rgb(R, G, B, A) with comma syntax
        let a = parse_alpha_value(parts[3]);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else {
        Some(0xFF000000 | (r << 16) | (g << 8) | b)
    }
}

fn parse_rgba_func(args: &str) -> Option<u32> {
    // rgba() is identical to rgb() in modern CSS
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    let a = if let Some(alpha_str) = alpha_part {
        parse_alpha_value(alpha_str)
    } else if parts.len() >= 4 {
        parse_alpha_value(parts[3])
    } else {
        255u32
    };
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

/// Split "R G B / alpha" or "R G B/alpha" into color part and optional alpha.
/// Handles var() references by not splitting on / inside parentheses.
fn split_color_alpha(args: &str) -> (&str, Option<&str>) {
    // Find the `/` that separates color from alpha, respecting parentheses
    let mut depth: u32 = 0;
    let bytes = args.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'(' { depth += 1; }
        else if b == b')' { depth = depth.saturating_sub(1); }
        else if b == b'/' && depth == 0 {
            let color = args[..i].trim();
            let alpha = args[i + 1..].trim();
            return (color, Some(alpha));
        }
    }
    (args, Option::None)
}

/// Parse an alpha value string. Handles fractional (0.0-1.0), integer (0-255),
/// var() references (default to 1.0), and percentage.
fn parse_alpha_value(s: &str) -> u32 {
    let t = s.trim();
    // If it's a var() or other unresolvable expression, default to fully opaque
    if t.starts_with("var(") || t.contains("var(") {
        return 255;
    }
    if t.ends_with('%') {
        if let Some(pct) = parse_int(&t[..t.len() - 1]) {
            return ((pct.max(0).min(100) as u32) * 255) / 100;
        }
        return 255;
    }
    if t.contains('.') {
        if let Some(fp) = parse_fixed_point(t) {
            return ((fp * 255) / 100).max(0).min(255) as u32;
        }
        return 255;
    }
    // Integer alpha: if <= 1, treat as 0 or 1 (fraction)
    if let Some(v) = parse_int(t) {
        if v <= 1 { return (v.max(0) as u32) * 255; }
        return v.max(0).min(255) as u32;
    }
    255 // default: fully opaque
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
    // Modern CSS: hsl(H S L) or hsl(H S L / alpha)
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return Option::None; }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    if let Some(alpha_str) = alpha_part {
        let a = parse_alpha_value(alpha_str);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else if parts.len() >= 4 {
        let a = parse_alpha_value(parts[3]);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else {
        Some(0xFF000000 | (r << 16) | (g << 8) | b)
    }
}

fn parse_hsla_func(args: &str) -> Option<u32> {
    // hsla() is identical to hsl() in modern CSS
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return Option::None; }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = if let Some(alpha_str) = alpha_part {
        parse_alpha_value(alpha_str)
    } else if parts.len() >= 4 {
        parse_alpha_value(parts[3])
    } else {
        255u32
    };
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
        // `fr` unit for CSS Grid fractional tracks (stored as Length with Fr unit)
        "fr" => Some(CssValue::Length(val, Unit::Fr)),
        // Viewport units
        "vw" => Some(CssValue::Length(val, Unit::Vw)),
        "vh" => Some(CssValue::Length(val, Unit::Vh)),
        "vmin" => Some(CssValue::Length(val, Unit::Vmin)),
        "vmax" => Some(CssValue::Length(val, Unit::Vmax)),
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

// ---------------------------------------------------------------------------
// Public wrappers for use by style.rs shadow/gradient parsing
// ---------------------------------------------------------------------------

/// Public wrapper for `try_parse_color`.
pub fn try_parse_color_pub(s: &str) -> Option<u32> {
    try_parse_color(s)
}

/// Public wrapper for `named_color`.
pub fn named_color_pub(name: &str) -> Option<u32> {
    named_color(name)
}

/// Public wrapper for `try_parse_dimension`.
pub fn try_parse_dimension_pub(s: &str) -> Option<CssValue> {
    try_parse_dimension(s)
}

