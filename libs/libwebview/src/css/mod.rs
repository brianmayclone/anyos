// CSS tokenizer + parser for surf browser
// no_std compatible, uses alloc for String/Vec

use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::dom::Tag;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// Ordered layer names from lowest to highest normal-priority precedence.
    pub layer_order: Vec<String>,
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontDisplay {
    Auto,
    Block,
    Swap,
    Fallback,
    Optional,
}

#[derive(Clone)]
pub struct FontFaceRule {
    pub family: String,
    pub src_url: String,
    /// CSS font-weight (400 = normal, 700 = bold).
    pub weight: u32,
    /// CSS font-style: "normal" or "italic".
    pub italic: bool,
    /// CSS font-display behavior for scheduling/fallback decisions.
    pub display: FontDisplay,
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

#[derive(Clone)]
pub struct ContainerQuery {
    pub name: Option<String>,
    pub conditions: Vec<ContainerCondition>,
}

#[derive(Clone)]
pub enum ContainerCondition {
    MinWidth(i32),
    MaxWidth(i32),
    MinHeight(i32),
    MaxHeight(i32),
    Width(i32),
    Height(i32),
    MinInlineSize(i32),
    MaxInlineSize(i32),
    MinBlockSize(i32),
    MaxBlockSize(i32),
    InlineSize(i32),
    BlockSize(i32),
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
    /// Full layer name (e.g. "framework.components") if declared inside `@layer`.
    pub layer_name: Option<String>,
    /// Global layer order assigned during stylesheet preparation.
    pub layer_index: Option<usize>,
    /// Optional `@container` query that must match for this rule to apply.
    pub container_query: Option<ContainerQuery>,
}

#[derive(Clone)]
pub enum Selector {
    Simple(SimpleSelector),
    Descendant(Box<Selector>, SimpleSelector),      // A B
    Child(Box<Selector>, SimpleSelector),           // A > B
    AdjacentSibling(Box<Selector>, SimpleSelector), // A + B
    GeneralSibling(Box<Selector>, SimpleSelector),  // A ~ B
    Universal,
}

#[derive(Clone)]
pub struct SimpleSelector {
    pub tag: Option<Tag>,
    /// Raw tag name for `Tag::Unknown` custom elements (e.g. "a-analytics").
    /// When `Some`, `simple_matches` additionally checks the DOM node's stored
    /// custom tag name so that `a-analytics {}` only matches `<a-analytics>`,
    /// not all unknown elements.
    pub custom_tag: Option<String>,
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
    Exists,    // [attr]
    Exact,     // [attr=val]
    Contains,  // [attr~=val] (word in space-separated)
    Prefix,    // [attr^=val]
    Suffix,    // [attr$=val]
    Substring, // [attr*=val]
    DashMatch, // [attr|=val]
}

/// Pseudo-element (::before, ::after).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    /// Unknown pseudo-element (e.g. ::-webkit-datetime-edit). Never matches.
    Unknown,
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
    /// `:required` — form elements with required attribute.
    Required,
    /// `:optional` — form elements without required attribute.
    Optional,
    /// `:read-only` — elements that are not editable.
    ReadOnly,
    /// `:read-write` — elements that are user-editable.
    ReadWrite,
    /// `:valid` — form elements that pass constraint validation.
    Valid,
    /// `:invalid` — form elements that fail constraint validation.
    Invalid,
    /// `:in-range` — number/range/date inputs within min/max bounds.
    InRange,
    /// `:out-of-range` — number/range/date inputs outside min/max bounds.
    OutOfRange,
    /// `:default` — default submit button or initially-checked radio/checkbox.
    Default,
    /// `:indeterminate` — checkbox/radio/progress with no definite state.
    Indeterminate,
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
    Direction,
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
    JustifySelf,
    PlaceItems,
    PlaceSelf,
    PlaceContent,
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
    // Mask
    MaskImage,
    MaskPosition,
    MaskRepeat,
    MaskSize,
    MaskClip,
    MaskOrigin,
    // Pointer events
    PointerEvents,
    // User interaction
    UserSelect,
    // Backdrop filter
    BackdropFilter,
    // CSS Logical Properties (inline = left+right, block = top+bottom for LTR)
    PaddingInline,
    PaddingBlock,
    MarginInline,
    MarginBlock,
    // Additional properties for modern CSS
    Appearance,
    AccentColor,
    BackgroundClip,
    ColorScheme,
    ContainerType,
    ContainerName,
    ScrollBehavior,
    Resize,
    ObjectPosition,
    /// CSS custom property (--name). Value stored in Declaration.value as Keyword.
    CustomProperty(String),
}

#[derive(Clone, PartialEq)]
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
        let classes =
            self.classes.len() as u32 + self.attrs.len() as u32 + self.pseudo_classes.len() as u32;
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

include!("ast.rs");
include!("lexer.rs");
include!("parser_core.rs");
include!("parser_ast.rs");
include!("stylesheet.rs");
include!("at_rules_media.rs");
include!("at_rules.rs");
include!("selectors.rs");
include!("declarations.rs");
include!("values.rs");
include!("shorthand.rs");
include!("shorthand_grid.rs");
include!("color.rs");
include!("color_named.rs");

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
        integer_part = integer_part
            .wrapping_mul(10)
            .wrapping_add((bytes[i] - b'0') as i32);
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
