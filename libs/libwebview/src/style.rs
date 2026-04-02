//! Style resolution: takes a DOM tree + CSS stylesheets and computes
//! the final `ComputedStyle` for every node.
//!
//! Cascade order: initial values -> UA defaults -> author rules (by
//! specificity) -> inline styles.  Inheritable properties that are not
//! explicitly set by any declaration are inherited from the parent node.

use alloc::vec;
use alloc::vec::Vec;

use alloc::string::String;

use crate::css::{
    AttrOp, CssValue, Declaration, PseudoClass, PseudoElement, Property, Rule, Selector,
    SimpleSelector, Stylesheet, Unit,
};
use crate::dom::{Dom, NodeId, NodeType, Tag};

// Viewport dimensions for resolving vh/vw/vmin/vmax units.
// Set at the start of resolve_styles() and read by resolve_length().
static mut VIEWPORT_W: i32 = 800;
static mut VIEWPORT_H: i32 = 600;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    ListItem,
    TableRow,
    TableCell,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    /// `display: flow-root` — block-level box that establishes a new BFC.
    /// CSS Display Module Level 3 §2.
    FlowRoot,
    /// `display: contents` — the element itself generates no box, but its
    /// children participate in the parent's layout as if they were direct children.
    Contents,
    None,
}

/// CSS `text-decoration-style` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationStyle {
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// CSS `font-variant` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontVariantVal {
    Normal,
    SmallCaps,
}

/// Parsed CSS `filter` function list.
#[derive(Clone, PartialEq)]
pub struct FilterVal {
    pub blur_px: i32,
    pub brightness: i32,   // percent * 100 (10000 = 100%)
    pub contrast: i32,     // percent * 100
    pub grayscale: i32,    // percent * 100
    pub saturate: i32,     // percent * 100
    pub sepia: i32,        // percent * 100
    pub opacity: i32,      // percent * 100
    pub hue_rotate: i32,   // degrees
    pub invert: i32,       // percent * 100
}

impl FilterVal {
    pub fn none() -> Self {
        FilterVal {
            blur_px: 0, brightness: 10000, contrast: 10000,
            grayscale: 0, saturate: 10000, sepia: 0,
            opacity: 10000, hue_rotate: 0, invert: 0,
        }
    }
    pub fn is_none(&self) -> bool {
        self.blur_px == 0 && self.brightness == 10000 && self.contrast == 10000
        && self.grayscale == 0 && self.saturate == 10000 && self.sepia == 0
        && self.opacity == 10000 && self.hue_rotate == 0 && self.invert == 0
    }
}

/// CSS `clip-path` value (basic shapes only).
#[derive(Clone, PartialEq)]
pub enum ClipPathVal {
    None,
    /// `circle(radius at cx cy)` — all in px.
    Circle { radius: i32, cx: i32, cy: i32 },
    /// `inset(top right bottom left [round radius])` — all in px.
    Inset { top: i32, right: i32, bottom: i32, left: i32, radius: i32 },
}

/// CSS timing function for transitions and animations.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    StepStart,
    StepEnd,
}

/// One parsed `transition` item (per-property).
#[derive(Clone)]
pub struct TransitionDef {
    /// CSS property name to animate (lowercase, e.g. `"opacity"`).
    pub property: String,
    /// Transition duration in milliseconds.
    pub duration_ms: u32,
    /// Timing function.
    pub timing: TimingFunction,
    /// Delay before the transition starts (ms).
    pub delay_ms: u32,
}

/// Parsed `animation` item (one per `animation:` layer).
#[derive(Clone)]
pub struct AnimationDef {
    /// Matches a `@keyframes` block by name.
    pub name: String,
    /// Animation duration in milliseconds.
    pub duration_ms: u32,
    /// Timing function.
    pub timing: TimingFunction,
    /// Delay before animation starts (ms).
    pub delay_ms: u32,
    /// 0 = infinite, otherwise finite repeat count.
    pub iteration_count: u32,
    /// `true` = alternates direction on each iteration.
    pub alternate: bool,
}

/// A single track sizing function for `grid-template-columns` / `grid-template-rows`.
#[derive(Clone, PartialEq)]
pub enum GridTrackSize {
    /// Fixed pixel size.
    Px(i32),
    /// Fractional unit (×100 fixed-point, e.g. 1fr = 100).
    Fr(i32),
    /// Percentage of the grid container width (×100 fixed-point).
    Percent(i32),
    /// `auto` — shrink/grow to fit content.
    Auto,
    /// `min-content` — minimum intrinsic size.
    MinContent,
    /// `max-content` — maximum intrinsic size.
    MaxContent,
    /// `minmax(min, max)` — clamped range; `min` and `max` are pixel values or -1 for fr.
    /// Stored as (min_px, max_px_or_fr_times_100, is_fr_max).
    Minmax { min_px: i32, max_px: i32, max_is_fr: bool },
    /// `repeat(auto-fill, minmax(min_px, 1fr))` — resolved at layout time.
    AutoFill { min_px: i32 },
    /// `repeat(auto-fit, minmax(min_px, 1fr))` — resolved at layout time.
    AutoFit { min_px: i32 },
}

/// Resolved line address for `grid-column-start/end` etc.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GridLine {
    /// Automatic placement.
    Auto,
    /// Explicit 1-based line number (may be negative for lines from the end).
    Index(i32),
    /// `span N` — spans N tracks.
    Span(i32),
    /// Named grid area — resolved at layout time against grid-template-areas.
    Named(String),
}

/// A named grid area parsed from `grid-template-areas`.
/// Positions are 1-based line numbers (like CSS grid lines).
#[derive(Clone, PartialEq)]
pub struct GridArea {
    pub name: String,
    pub row_start: i32,
    pub col_start: i32,
    pub row_end: i32,
    pub col_end: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FloatVal {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClearVal {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    Nowrap,
    Wrap,
    WrapReverse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Debug)]
pub enum OverflowVal {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontWeight { Normal, Bold }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontStyleVal { Normal, Italic }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextAlignVal { Left, Center, Right, Justify }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextDeco { None, Underline, LineThrough, Overline }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListStyle {
    None, Disc, Circle, Square, Decimal,
    LowerAlpha, UpperAlpha, LowerLatin, UpperLatin,
    LowerRoman, UpperRoman,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListStylePosition { Outside, Inside }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace { Normal, Pre, Nowrap, PreWrap }

/// CSS `border-style` values (litehtml-inspired).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BorderStyleVal { None, Solid, Dashed, Dotted, Double, Groove, Ridge, Inset, Outset, Hidden }

/// CSS `word-break` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WordBreak { Normal, BreakAll, KeepAll }

/// CSS `overflow-wrap` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OverflowWrapVal { Normal, BreakWord, Anywhere }

/// CSS `text-overflow` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowVal { Clip, Ellipsis }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObjectFit {
    /// Scale to fill; aspect ratio NOT preserved.
    Fill,
    /// Scale to fit inside; aspect ratio preserved; may have empty space.
    Contain,
    /// Scale to cover; aspect ratio preserved; may be clipped.
    Cover,
    /// Use intrinsic size (no scaling).
    None,
    /// Use the smaller of Contain and None.
    ScaleDown,
}

/// CSS `vertical-align` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    /// Length offset in px from baseline.
    Length(i32),
}

/// Parsed `box-shadow` value (litehtml-inspired).
#[derive(Clone, PartialEq)]
pub struct BoxShadowVal {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: i32,
    pub spread: i32,
    pub color: u32,
    pub inset: bool,
}

/// Parsed `text-shadow` value.
#[derive(Clone, PartialEq)]
pub struct TextShadowVal {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: i32,
    pub color: u32,
}

/// Per-side border (litehtml-style).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BorderSide {
    pub width: i32,
    pub style: BorderStyleVal,
    pub color: u32,
}

impl BorderSide {
    pub fn none() -> Self {
        BorderSide { width: 0, style: BorderStyleVal::None, color: 0xFF000000 }
    }
}

/// Background-image value.
#[derive(Clone, PartialEq)]
pub enum BackgroundImageVal {
    None,
    Url(String),
    LinearGradient { angle_deg: i32, stops: Vec<GradientStop> },
}

/// A color stop in a gradient.
#[derive(Clone, PartialEq)]
pub struct GradientStop {
    pub color: u32,
    /// Position as percentage * 100 (fixed-point). -1 = auto.
    pub position: i32,
}

/// CSS `background-size` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundSizeVal {
    Auto,
    Cover,
    Contain,
    /// (width, height) in px. -1 = auto for that dimension.
    Explicit(i32, i32),
}

/// CSS `background-repeat` values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeatVal {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

// ---------------------------------------------------------------------------
// ComputedStyle
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: u32,              // 0xAARRGGBB
    pub background_color: u32,   // 0xAARRGGBB (0 = transparent)
    pub font_size: i32,          // pixels
    pub font_weight: FontWeight,
    pub font_style: FontStyleVal,
    pub text_align: TextAlignVal,
    pub text_decoration: TextDeco,
    pub line_height: i32,        // pixels (0 = auto -> 1.2 * font_size)
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    /// true if margin-left was explicitly `auto`
    pub margin_left_auto: bool,
    /// true if margin-right was explicitly `auto`
    pub margin_right_auto: bool,
    pub padding_top: i32,
    pub padding_right: i32,
    pub padding_bottom: i32,
    pub padding_left: i32,
    pub border_width: i32,
    pub border_color: u32,
    pub border_radius: i32,
    pub width: Option<i32>,      // None = auto
    pub height: Option<i32>,     // None = auto
    pub max_width: Option<i32>,
    pub min_width: i32,
    pub max_height: Option<i32>,
    pub min_height: i32,
    pub list_style: ListStyle,
    pub list_style_position: ListStylePosition,
    pub white_space: WhiteSpace,
    // Positioning
    pub position: Position,
    pub top: Option<i32>,
    pub right_offset: Option<i32>,
    pub bottom_offset: Option<i32>,
    pub left_offset: Option<i32>,
    pub z_index: i32,
    /// Whether z-index is `auto` (true) or an explicit integer (false).
    /// Per CSS2 §9.9.1, positioned elements with explicit z-index (including 0)
    /// create a new stacking context; `auto` does not.
    pub z_index_auto: bool,
    // Flexbox
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: Option<AlignItems>,
    pub flex_grow: i32,          // fixed-point * 100
    pub flex_shrink: i32,        // fixed-point * 100
    pub flex_basis: Option<i32>, // None = auto, Some(px)
    pub row_gap: i32,
    pub column_gap: i32,
    pub align_content: AlignContent,
    pub order: i32,
    // Grid container
    /// Track sizes for `grid-template-columns` (empty = no explicit columns).
    pub grid_template_columns: Vec<GridTrackSize>,
    /// Track sizes for `grid-template-rows` (empty = no explicit rows).
    pub grid_template_rows: Vec<GridTrackSize>,
    /// Named grid areas: each entry is (name, row_start, col_start, row_end, col_end) 1-based.
    pub grid_template_areas: Vec<GridArea>,
    /// Default column size for implicitly created tracks.
    pub grid_auto_columns: GridTrackSize,
    /// Default row size for implicitly created tracks.
    pub grid_auto_rows: GridTrackSize,
    /// `grid-auto-flow`: false = row, true = column.
    pub grid_auto_flow_column: bool,
    /// `justify-items` alignment for grid items along the inline axis.
    pub justify_items: AlignItems,
    // Grid item placement
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,
    // Box model
    pub box_sizing: BoxSizing,
    // Table
    pub border_collapse: bool,  // true = collapse, false = separate
    pub border_spacing: i32,    // px (CSS border-spacing)
    // Float
    pub float: FloatVal,
    pub clear: ClearVal,
    // Visual
    pub opacity: i32,            // 0..255 (255 = fully opaque)
    pub visibility: Visibility,
    pub text_transform: TextTransform,
    // Overflow
    pub overflow_x: OverflowVal,
    pub overflow_y: OverflowVal,
    // Width/height percentages (stored as fixed-point * 100, None if not percentage)
    pub width_pct: Option<i32>,
    pub height_pct: Option<i32>,
    // calc() components: (px * 100, pct * 100) for width/height
    pub width_calc: Option<(i32, i32)>,
    pub height_calc: Option<(i32, i32)>,
    // Intrinsic sizing keywords for width.
    pub width_max_content: bool,  // width: max-content
    pub width_min_content: bool,  // width: min-content
    pub width_fit_content: bool,  // width: fit-content
    // Typography (litehtml-inspired)
    pub font_family: Option<String>,
    pub letter_spacing: i32,     // px (0 = normal)
    pub word_spacing: i32,       // px (0 = normal)
    pub text_indent: i32,        // px
    pub vertical_align: VerticalAlign,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrapVal,
    pub text_overflow: TextOverflowVal,
    // Per-side borders (litehtml-inspired)
    pub border_top: BorderSide,
    pub border_right: BorderSide,
    pub border_bottom: BorderSide,
    pub border_left: BorderSide,
    pub border_top_left_radius: i32,
    pub border_top_right_radius: i32,
    pub border_bottom_right_radius: i32,
    pub border_bottom_left_radius: i32,
    // Outline (litehtml-inspired)
    pub outline_width: i32,
    pub outline_style: BorderStyleVal,
    pub outline_color: u32,
    pub outline_offset: i32,
    // Shadows (litehtml-inspired)
    pub box_shadows: Vec<BoxShadowVal>,
    pub text_shadows: Vec<TextShadowVal>,
    // Background extensions (litehtml-inspired)
    pub background_image: BackgroundImageVal,
    pub background_size: BackgroundSizeVal,
    pub background_repeat: BackgroundRepeatVal,
    pub background_position_x: i32,  // px or pct*100
    pub background_position_y: i32,
    // Content (for ::before/::after)
    pub content: Option<String>,
    /// URL for `content: url("...")` in pseudo-elements.
    pub content_url: Option<String>,
    // Object-fit for replaced elements (img, video)
    pub object_fit: ObjectFit,
    // CSS transform — translate offsets (resolved to px).
    pub transform_tx: i32,
    pub transform_ty: i32,
    // Filter effects (litehtml-inspired)
    pub filter: FilterVal,
    // Aspect ratio (width / height as fixed-point * 100, 0 = auto)
    pub aspect_ratio: i32,
    // Text decoration sub-properties (CSS3)
    pub text_decoration_color: u32,   // 0 = use text color
    pub text_decoration_style: TextDecorationStyle,
    pub text_decoration_thickness: i32, // px, 0 = auto
    pub text_underline_offset: i32,   // px, 0 = auto
    // Typography extras
    pub font_variant: FontVariantVal,
    pub tab_size: i32,  // number of spaces (default 8)
    // Clip path
    pub clip_path: ClipPathVal,
    /// CSS `clip: rect(top, right, bottom, left)` for absolutely positioned elements.
    /// Values in px*100 fixed-point. None = no clip.
    pub clip_rect: Option<[i32; 4]>,
    // CSS counters
    pub counter_reset: Option<String>,
    pub counter_increment: Option<String>,
    // Transitions
    pub transitions: Vec<TransitionDef>,
    // Animations
    pub animations: Vec<AnimationDef>,
}

// ---------------------------------------------------------------------------
// Pseudo-element styles (::before / ::after)
// ---------------------------------------------------------------------------

/// Stores resolved ::before and ::after styles for each DOM node.
/// Indexed by node ID (same length as the main styles Vec).
pub struct PseudoStyles {
    /// ::before style + content per node. None if no ::before rule matched.
    pub before: Vec<Option<ComputedStyle>>,
    /// ::after style + content per node. None if no ::after rule matched.
    pub after: Vec<Option<ComputedStyle>>,
}

impl PseudoStyles {
    pub fn empty(count: usize) -> Self {
        let mut before = Vec::with_capacity(count);
        let mut after = Vec::with_capacity(count);
        for _ in 0..count {
            before.push(None);
            after.push(None);
        }
        PseudoStyles { before, after }
    }
}

// Bitflags for tracking which inheritable properties were explicitly set.
const SET_COLOR: u16      = 1 << 0;
const SET_FONT_SIZE: u16  = 1 << 1;
const SET_FONT_WEIGHT: u16 = 1 << 2;
const SET_FONT_STYLE: u16 = 1 << 3;
const SET_TEXT_ALIGN: u16 = 1 << 4;
const SET_LINE_HEIGHT: u16 = 1 << 5;
const SET_WHITE_SPACE: u16 = 1 << 6;
const SET_LIST_STYLE: u16 = 1 << 7;
const SET_TEXT_DECO: u16  = 1 << 8;
const SET_VISIBILITY: u16 = 1 << 9;
const SET_TEXT_TRANSFORM: u16 = 1 << 10;
const SET_LETTER_SPACING: u16 = 1 << 11;
const SET_WORD_SPACING: u16   = 1 << 12;
const SET_WORD_BREAK: u16     = 1 << 13;
const SET_OVERFLOW_WRAP: u16  = 1 << 14;
const SET_LIST_STYLE_POS: u16 = 1 << 15;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

/// Reasonable defaults: black text, transparent background (light-theme base).
pub fn default_style() -> ComputedStyle {
    ComputedStyle {
        display: Display::Block,
        color: 0xFF000000,
        background_color: 0,
        font_size: 16,
        font_weight: FontWeight::Normal,
        font_style: FontStyleVal::Normal,
        text_align: TextAlignVal::Left,
        text_decoration: TextDeco::None,
        line_height: 0,
        margin_top: 0, margin_right: 0, margin_bottom: 0, margin_left: 0,
        margin_left_auto: false, margin_right_auto: false,
        padding_top: 0, padding_right: 0, padding_bottom: 0, padding_left: 0,
        border_width: 0,
        border_color: 0xFF808080,
        border_radius: 0,
        width: Option::None,
        height: Option::None,
        max_width: Option::None,
        min_width: 0,
        max_height: Option::None,
        min_height: 0,
        list_style: ListStyle::None,
        list_style_position: ListStylePosition::Outside,
        white_space: WhiteSpace::Normal,
        // Positioning
        position: Position::Static,
        top: Option::None,
        right_offset: Option::None,
        bottom_offset: Option::None,
        left_offset: Option::None,
        z_index: 0,
        z_index_auto: true,
        // Flexbox
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Nowrap,
        justify_content: JustifyContent::FlexStart,
        align_items: AlignItems::Stretch,
        align_self: Option::None,
        flex_grow: 0,
        flex_shrink: 100, // default 1.0 = 100 in fixed-point
        flex_basis: Option::None, // auto
        row_gap: 0,
        column_gap: 0,
        align_content: AlignContent::Stretch,
        order: 0,
        // Grid container
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        grid_template_areas: Vec::new(),
        grid_auto_columns: GridTrackSize::Auto,
        grid_auto_rows: GridTrackSize::Auto,
        grid_auto_flow_column: false,
        justify_items: AlignItems::Stretch,
        // Grid item placement
        grid_column_start: GridLine::Auto,
        grid_column_end: GridLine::Auto,
        grid_row_start: GridLine::Auto,
        grid_row_end: GridLine::Auto,
        // Box model
        box_sizing: BoxSizing::ContentBox,
        // Table
        border_collapse: false,
        border_spacing: 2,
        // Float
        float: FloatVal::None,
        clear: ClearVal::None,
        // Visual
        opacity: 255,
        visibility: Visibility::Visible,
        text_transform: TextTransform::None,
        // Overflow
        overflow_x: OverflowVal::Visible,
        overflow_y: OverflowVal::Visible,
        // Typography
        font_family: Option::None,
        letter_spacing: 0,
        word_spacing: 0,
        text_indent: 0,
        vertical_align: VerticalAlign::Baseline,
        word_break: WordBreak::Normal,
        overflow_wrap: OverflowWrapVal::Normal,
        text_overflow: TextOverflowVal::Clip,
        // Per-side borders
        border_top: BorderSide::none(),
        border_right: BorderSide::none(),
        border_bottom: BorderSide::none(),
        border_left: BorderSide::none(),
        border_top_left_radius: 0,
        border_top_right_radius: 0,
        border_bottom_right_radius: 0,
        border_bottom_left_radius: 0,
        // Outline
        outline_width: 0,
        outline_style: BorderStyleVal::None,
        outline_color: 0xFF000000,
        outline_offset: 0,
        // Shadows
        box_shadows: Vec::new(),
        text_shadows: Vec::new(),
        // Background extensions
        background_image: BackgroundImageVal::None,
        background_size: BackgroundSizeVal::Auto,
        background_repeat: BackgroundRepeatVal::Repeat,
        background_position_x: 0,
        background_position_y: 0,
        // Content
        content: Option::None,
        object_fit: ObjectFit::Fill,
        transform_tx: 0,
        transform_ty: 0,
        // Filter
        filter: FilterVal::none(),
        // Aspect ratio
        aspect_ratio: 0,
        // Text decoration sub-properties
        text_decoration_color: 0,
        text_decoration_style: TextDecorationStyle::Solid,
        text_decoration_thickness: 0,
        text_underline_offset: 0,
        // Typography extras
        font_variant: FontVariantVal::Normal,
        tab_size: 8,
        // Clip path
        clip_path: ClipPathVal::None,
        clip_rect: Option::None,
        // Counters
        content_url: Option::None,
        counter_reset: Option::None,
        counter_increment: Option::None,
        // Percentages
        width_pct: Option::None,
        height_pct: Option::None,
        width_max_content: false,
        width_min_content: false,
        width_fit_content: false,
        // Calc
        width_calc: Option::None,
        height_calc: Option::None,
        // Transitions & Animations
        transitions: Vec::new(),
        animations: Vec::new(),
    }
}

/// User-agent stylesheet: hardcoded browser defaults per HTML tag.
/// Returns the base style AND a bitfield indicating which inheritable
/// properties the UA explicitly sets (so inheritance does not clobber them).
fn ua_style_and_flags(tag: Tag) -> (ComputedStyle, u16) {
    let mut s = default_style();
    let mut flags: u16 = 0;
    match tag {
        Tag::Body => {
            s.margin_top = 8; s.margin_right = 8;
            s.margin_bottom = 8; s.margin_left = 8;
        }
        Tag::H1 => {
            s.font_size = 32; s.font_weight = FontWeight::Bold;
            s.margin_top = 21; s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H2 => {
            s.font_size = 24; s.font_weight = FontWeight::Bold;
            s.margin_top = 19; s.margin_bottom = 19;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H3 => {
            s.font_size = 19; s.font_weight = FontWeight::Bold;
            s.margin_top = 18; s.margin_bottom = 18;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H4 => {
            s.font_size = 16; s.font_weight = FontWeight::Bold;
            s.margin_top = 21; s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H5 => {
            s.font_size = 13; s.font_weight = FontWeight::Bold;
            s.margin_top = 22; s.margin_bottom = 22;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H6 => {
            s.font_size = 11; s.font_weight = FontWeight::Bold;
            s.margin_top = 24; s.margin_bottom = 24;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::P => {
            s.margin_top = 16; s.margin_bottom = 16;
        }
        Tag::A => {
            s.display = Display::Inline;
            s.color = 0xFF007AFF;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_COLOR | SET_TEXT_DECO;
        }
        Tag::Em | Tag::I => {
            s.display = Display::Inline;
            s.font_style = FontStyleVal::Italic;
            flags |= SET_FONT_STYLE;
        }
        Tag::Strong | Tag::B => {
            s.display = Display::Inline;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::U => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_TEXT_DECO;
        }
        Tag::Code => {
            s.display = Display::Inline;
        }
        Tag::Pre => {
            s.white_space = WhiteSpace::Pre;
            flags |= SET_WHITE_SPACE;
        }
        Tag::Blockquote => { s.margin_left = 40; }
        Tag::Ul => {
            s.margin_top = 16; s.margin_bottom = 16; s.padding_left = 40;
            // UA list-style: disc is inherited by <li> children.
            // Setting the flag here prevents <ul> from inheriting list-style from its
            // ancestors; <li> children inherit from <ul> because <li> has no flag.
            s.list_style = ListStyle::Disc;
            flags |= SET_LIST_STYLE;
        }
        Tag::Ol => {
            s.margin_top = 16; s.margin_bottom = 16; s.padding_left = 40;
            s.list_style = ListStyle::Decimal;
            flags |= SET_LIST_STYLE;
        }
        Tag::Li => {
            s.display = Display::ListItem;
            // No SET_LIST_STYLE flag: <li> inherits list-style from its parent (<ul>/<ol>).
            // This allows `list-style: none` on the parent to propagate via CSS inheritance.
            s.list_style = ListStyle::Disc; // fallback if orphan (no <ul>/<ol> parent)
        }
        Tag::Hr => {
            s.border_width = 1; s.margin_top = 8; s.margin_bottom = 8;
        }
        Tag::Img | Tag::Br | Tag::Span | Tag::Label => {
            s.display = Display::Inline;
        }
        Tag::Input | Tag::Button | Tag::Select | Tag::Textarea => {
            s.display = Display::Inline;
        }
        Tag::Table => { s.border_width = 1; }
        Tag::Tr => { s.display = Display::TableRow; }
        Tag::Td => {
            s.display = Display::TableCell;
            s.padding_top = 4; s.padding_right = 4;
            s.padding_bottom = 4; s.padding_left = 4;
        }
        Tag::Th => {
            s.display = Display::TableCell;
            s.font_weight = FontWeight::Bold;
            s.padding_top = 4; s.padding_right = 4;
            s.padding_bottom = 4; s.padding_left = 4;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Head | Tag::Title | Tag::Meta | Tag::Link | Tag::Style | Tag::Script
        | Tag::Noscript | Tag::Template => {
            s.display = Display::None;
        }
        // Inline semantic text elements
        Tag::Small => { s.display = Display::Inline; s.font_size = 13; flags |= SET_FONT_SIZE; }
        Tag::S | Tag::Del => { s.display = Display::Inline; s.text_decoration = TextDeco::LineThrough; flags |= SET_TEXT_DECO; }
        Tag::Ins => { s.display = Display::Inline; s.text_decoration = TextDeco::Underline; flags |= SET_TEXT_DECO; }
        Tag::Mark => {
            s.display = Display::Inline;
            s.background_color = 0xFFFFFF00; // yellow highlight
            s.color = 0xFF000000;
            flags |= SET_COLOR;
        }
        Tag::Sub | Tag::Sup | Tag::Kbd | Tag::Samp | Tag::Var | Tag::Abbr
        | Tag::Cite | Tag::Dfn | Tag::Q | Tag::Time | Tag::Bdi | Tag::Bdo
        | Tag::Data | Tag::Ruby | Tag::Rt | Tag::Rp | Tag::Wbr | Tag::Nobr | Tag::Tt => {
            s.display = Display::Inline;
        }
        // Definition list
        Tag::Dl => { s.margin_top = 16; s.margin_bottom = 16; }
        Tag::Dt => { s.font_weight = FontWeight::Bold; flags |= SET_FONT_WEIGHT; }
        Tag::Dd => { s.margin_left = 40; }
        // Figure
        Tag::Figure => { s.margin_top = 16; s.margin_bottom = 16; s.margin_left = 40; s.margin_right = 40; }
        Tag::Figcaption => { s.text_align = TextAlignVal::Center; flags |= SET_TEXT_ALIGN; }
        // Details/Summary
        Tag::Details => {}
        Tag::Summary => { s.display = Display::Block; s.font_weight = FontWeight::Bold; flags |= SET_FONT_WEIGHT; }
        // Dialog
        Tag::Dialog => { s.display = Display::Block; s.position = Position::Absolute; }
        // Sectioning
        Tag::Aside | Tag::Hgroup | Tag::Address => {}
        // Table extensions
        Tag::Tfoot => { s.display = Display::TableRow; }
        Tag::Caption => { s.text_align = TextAlignVal::Center; flags |= SET_TEXT_ALIGN; }
        // Form elements
        Tag::Fieldset => { s.border_width = 1; s.padding_top = 8; s.padding_right = 12; s.padding_bottom = 8; s.padding_left = 12; }
        Tag::Legend => { s.display = Display::Inline; s.font_weight = FontWeight::Bold; flags |= SET_FONT_WEIGHT; }
        Tag::Optgroup => {}
        Tag::Datalist | Tag::Output => { s.display = Display::Inline; }
        Tag::Progress | Tag::Meter => { s.display = Display::Inline; }
        // Deprecated
        Tag::Center => { s.text_align = TextAlignVal::Center; flags |= SET_TEXT_ALIGN; }
        Tag::Font => { s.display = Display::Inline; }
        // Block-level elements that just use defaults.
        Tag::Div | Tag::Section | Tag::Article | Tag::Header | Tag::Footer
        | Tag::Nav | Tag::Main | Tag::Form | Tag::Thead | Tag::Tbody => {}
        _ => {}
    }
    (s, flags)
}

/// Public convenience: returns only the `ComputedStyle` (no flags).
pub fn user_agent_styles(tag: Tag) -> ComputedStyle {
    ua_style_and_flags(tag).0
}

// ---------------------------------------------------------------------------
// Rule index — O(1) lookup by tag / ID / class
// ---------------------------------------------------------------------------

/// Number of Tag enum variants (used to size the tag bucket array).
const TAG_COUNT: usize = 128; // Tag enum has ~100 variants, 128 is safe

/// Pre-built index for fast rule lookup by the leaf selector's tag, ID, or class.
///
/// Instead of checking all N rules against every DOM node (O(nodes × rules)),
/// we partition rules into buckets so that for a given `<div id="foo" class="bar baz">`
/// we only check rules whose leaf selector requires `div`, `#foo`, `.bar`, or `.baz`
/// — plus the "wildcard" rules that have no tag/id/class restriction.
struct RuleIndex {
    /// `by_tag[tag_discriminant]` = rule indices whose leaf selector requires that tag.
    by_tag: [Vec<usize>; TAG_COUNT],
    /// Rules whose leaf selector requires a specific ID.  Key = id string.
    by_id: Vec<(String, Vec<usize>)>,
    /// Rules whose leaf selector requires a specific class.  Key = class string.
    by_class: Vec<(String, Vec<usize>)>,
    /// Rules with no tag/id/class restriction (universal, attribute-only, pseudo-only).
    wildcard: Vec<usize>,
    /// Total number of rules (for bitset sizing).
    rule_count: usize,
}

impl RuleIndex {
    /// Build the rule index from the collected rules.
    fn build(all_rules: &[(&Rule, usize)]) -> Self {
        const EMPTY_VEC: Vec<usize> = Vec::new();
        let mut idx = RuleIndex {
            by_tag: [EMPTY_VEC; TAG_COUNT],
            by_id: Vec::new(),
            by_class: Vec::new(),
            wildcard: Vec::new(),
            rule_count: all_rules.len(),
        };

        for (rule_idx, (rule, _order)) in all_rules.iter().enumerate() {
            // A rule can have multiple selectors (comma-separated).
            // We must put the rule in every bucket that any of its selectors' leaves require.
            let mut added_to_any = false;

            for sel in &rule.selectors {
                match leaf_simple(sel) {
                    Some(leaf) => {
                        // Index by the most specific leaf attribute (tag > id > class).
                        let mut indexed = false;

                        if let Some(tag) = leaf.tag {
                            let t = tag as usize;
                            if t < TAG_COUNT {
                                idx.by_tag[t].push(rule_idx);
                                indexed = true;
                            }
                        }
                        if let Some(ref id) = leaf.id {
                            push_keyed(&mut idx.by_id, id, rule_idx);
                            indexed = true;
                        }
                        for cls in &leaf.classes {
                            push_keyed(&mut idx.by_class, cls, rule_idx);
                            indexed = true;
                        }

                        if !indexed {
                            // Attribute-only or pseudo-only selector — goes to wildcard.
                            if !idx.wildcard.contains(&rule_idx) {
                                idx.wildcard.push(rule_idx);
                            }
                        }
                        added_to_any = true;
                    }
                    None => {
                        // Universal selector — matches any element.
                        if !idx.wildcard.contains(&rule_idx) {
                            idx.wildcard.push(rule_idx);
                        }
                        added_to_any = true;
                    }
                }
            }

            if !added_to_any {
                idx.wildcard.push(rule_idx);
            }
        }

        idx
    }

    /// Get candidate rule indices for a node with the given tag, id, and classes.
    /// Returns a deduplicated list of rule indices to check.
    /// Uses a bitset for O(1) deduplication instead of Vec::contains() O(n).
    fn candidates(&self, tag: Tag, id_attr: Option<&str>, class_attr: Option<&str>,
                  buf: &mut Vec<usize>, seen: &mut Vec<u64>) {
        buf.clear();

        // Reset the bitset (one bit per rule index, packed into u64 words).
        let words_needed = (self.rule_count + 63) / 64;
        seen.clear();
        seen.resize(words_needed, 0u64);

        // Tag bucket.
        let t = tag as usize;
        if t < TAG_COUNT {
            for &ri in &self.by_tag[t] {
                let word = ri / 64;
                let bit = 1u64 << (ri % 64);
                if word < seen.len() && seen[word] & bit == 0 {
                    seen[word] |= bit;
                    buf.push(ri);
                }
            }
        }

        // ID bucket.
        if let Some(id) = id_attr {
            if let Some((_, indices)) = self.by_id.iter().find(|(k, _)| eq_ignore_ascii_case(k, id)) {
                for &ri in indices {
                    let word = ri / 64;
                    let bit = 1u64 << (ri % 64);
                    if word < seen.len() && seen[word] & bit == 0 {
                        seen[word] |= bit;
                        buf.push(ri);
                    }
                }
            }
        }

        // Class buckets.
        if let Some(classes) = class_attr {
            for (cls_key, indices) in &self.by_class {
                if has_class(classes, cls_key) {
                    for &ri in indices {
                        let word = ri / 64;
                        let bit = 1u64 << (ri % 64);
                        if word < seen.len() && seen[word] & bit == 0 {
                            seen[word] |= bit;
                            buf.push(ri);
                        }
                    }
                }
            }
        }

        // Wildcard rules (always checked).
        for &ri in &self.wildcard {
            let word = ri / 64;
            let bit = 1u64 << (ri % 64);
            if word < seen.len() && seen[word] & bit == 0 {
                seen[word] |= bit;
                buf.push(ri);
            }
        }
    }
}

/// Extract the leaf (rightmost) SimpleSelector from a combinator chain.
/// Returns None for Universal selectors.
fn leaf_simple(sel: &Selector) -> Option<&SimpleSelector> {
    match sel {
        Selector::Simple(s) => Some(s),
        Selector::Descendant(_, leaf)
        | Selector::Child(_, leaf)
        | Selector::AdjacentSibling(_, leaf)
        | Selector::GeneralSibling(_, leaf) => Some(leaf),
        Selector::Universal => None,
    }
}

/// Push `value` into the keyed bucket list.
fn push_keyed(buckets: &mut Vec<(String, Vec<usize>)>, key: &str, value: usize) {
    if let Some((_, vec)) = buckets.iter_mut().find(|(k, _)| eq_ignore_ascii_case(k, key)) {
        vec.push(value);
    } else {
        let mut v = Vec::new();
        v.push(value);
        buckets.push((String::from(key), v));
    }
}

// ---------------------------------------------------------------------------
// Selector matching
// ---------------------------------------------------------------------------

/// Check if a CSS selector matches a DOM element node.
fn selector_matches(selector: &Selector, dom: &Dom, node_id: NodeId) -> bool {
    // Skip selectors that target ::before/::after — those are handled separately.
    if selector.pseudo_element().is_some() {
        return false;
    }
    selector_matches_base(selector, dom, node_id)
}

/// Match a selector against a node, ignoring the pseudo-element part.
/// Used both for normal matching (via selector_matches) and pseudo-element resolution.
fn selector_matches_base(selector: &Selector, dom: &Dom, node_id: NodeId) -> bool {
    // Bounds check to prevent crashes from corrupted node indices.
    if node_id >= dom.nodes.len() {
        return false;
    }
    match selector {
        Selector::Universal => {
            matches!(dom.nodes[node_id].node_type, NodeType::Element { .. })
        }
        Selector::Simple(simple) => simple_matches(simple, dom, node_id),
        Selector::Descendant(ancestor_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id) {
                return false;
            }
            let mut cur = dom.nodes[node_id].parent;
            while let Some(pid) = cur {
                if pid >= dom.nodes.len() { break; }
                if selector_matches_base(ancestor_sel, dom, pid) {
                    return true;
                }
                cur = dom.nodes[pid].parent;
            }
            false
        }
        Selector::Child(parent_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id) {
                return false;
            }
            if let Some(pid) = dom.nodes[node_id].parent {
                if pid >= dom.nodes.len() { return false; }
                selector_matches_base(parent_sel, dom, pid)
            } else {
                false
            }
        }
        Selector::AdjacentSibling(prev_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id) {
                return false;
            }
            // Find preceding sibling element
            if let Some(sib) = preceding_element_sibling(dom, node_id) {
                selector_matches_base(prev_sel, dom, sib)
            } else {
                false
            }
        }
        Selector::GeneralSibling(prev_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id) {
                return false;
            }
            // Check all preceding sibling elements
            let mut sib = preceding_element_sibling(dom, node_id);
            while let Some(sid) = sib {
                if selector_matches_base(prev_sel, dom, sid) {
                    return true;
                }
                sib = preceding_element_sibling(dom, sid);
            }
            false
        }
    }
}

/// Find the immediately preceding element sibling of `node_id`.
fn preceding_element_sibling(dom: &Dom, node_id: NodeId) -> Option<NodeId> {
    let parent = dom.nodes[node_id].parent?;
    let children = &dom.nodes[parent].children;
    let pos = children.iter().position(|&c| c == node_id)?;
    // Walk backwards from pos-1 to find first element
    for i in (0..pos).rev() {
        if matches!(dom.nodes[children[i]].node_type, NodeType::Element { .. }) {
            return Some(children[i]);
        }
    }
    Option::None
}

fn simple_matches(sel: &SimpleSelector, dom: &Dom, node_id: NodeId) -> bool {
    if node_id >= dom.nodes.len() { return false; }
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (*tag, attrs),
        NodeType::Text(_) => return false,
    };

    // Tag check.
    if let Some(sel_tag) = sel.tag {
        if sel_tag != tag {
            return false;
        }
    }

    // ID check.
    if let Some(ref sel_id) = sel.id {
        let node_id_attr = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "id"));
        match node_id_attr {
            Some(a) if eq_ignore_ascii_case(&a.value, sel_id) => {}
            _ => return false,
        }
    }

    // Class check: every selector class must be present on the node.
    if !sel.classes.is_empty() {
        let class_attr = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "class"));
        let class_str = match class_attr {
            Some(a) => &a.value,
            Option::None => return false,
        };
        for sc in &sel.classes {
            if !has_class(class_str, sc) {
                return false;
            }
        }
    }

    // Attribute selector check.
    for attr_sel in &sel.attrs {
        let node_attr = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, &attr_sel.name));
        match attr_sel.op {
            AttrOp::Exists => {
                if node_attr.is_none() { return false; }
            }
            AttrOp::Exact => {
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) if eq_ignore_ascii_case(&a.value, v) => {}
                    _ => return false,
                }
            }
            AttrOp::Contains => {
                // [attr~=val]: word in space-separated list
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) if has_class(&a.value, v) => {}
                    _ => return false,
                }
            }
            AttrOp::Prefix => {
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) => {
                        if !starts_with_ignore_case(&a.value, v) { return false; }
                    }
                    _ => return false,
                }
            }
            AttrOp::Suffix => {
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) => {
                        if !ends_with_ignore_case(&a.value, v) { return false; }
                    }
                    _ => return false,
                }
            }
            AttrOp::Substring => {
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) => {
                        if !contains_ignore_case(&a.value, v) { return false; }
                    }
                    _ => return false,
                }
            }
            AttrOp::DashMatch => {
                // [attr|=val]: exact or starts with val-
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) => {
                        if !eq_ignore_ascii_case(&a.value, v)
                            && !starts_with_ignore_case(&a.value, &{
                                let mut s = v.clone();
                                s.push('-');
                                s
                            })
                        {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        }
    }

    // Pseudo-class check.
    for pc in &sel.pseudo_classes {
        if !pseudo_class_matches(pc, dom, node_id) {
            return false;
        }
    }

    true
}

fn pseudo_class_matches(pc: &PseudoClass, dom: &Dom, node_id: NodeId) -> bool {
    match pc {
        PseudoClass::Root => {
            // Root is the <html> element (no parent or parent is document root)
            dom.nodes[node_id].parent.is_none()
                || dom.nodes[node_id].parent == Some(0)
        }
        PseudoClass::FirstChild => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                children.iter()
                    .find(|&&c| matches!(dom.nodes[c].node_type, NodeType::Element { .. }))
                    == Some(&node_id)
            } else {
                false
            }
        }
        PseudoClass::LastChild => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                children.iter().rev()
                    .find(|&&c| matches!(dom.nodes[c].node_type, NodeType::Element { .. }))
                    == Some(&node_id)
            } else {
                false
            }
        }
        PseudoClass::NthChild(n) => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                let mut count = 0i32;
                for &c in children {
                    if matches!(dom.nodes[c].node_type, NodeType::Element { .. }) {
                        count += 1;
                        if c == node_id {
                            return count == *n;
                        }
                    }
                }
            }
            false
        }
        PseudoClass::NthLastChild(n) => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                let mut count = 0i32;
                for &c in children.iter().rev() {
                    if matches!(dom.nodes[c].node_type, NodeType::Element { .. }) {
                        count += 1;
                        if c == node_id {
                            return count == *n;
                        }
                    }
                }
            }
            false
        }
        PseudoClass::FirstOfType => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let my_tag = dom.tag(node_id);
                let children = &dom.nodes[pid].children;
                for &c in children {
                    if dom.tag(c) == my_tag {
                        return c == node_id;
                    }
                }
            }
            false
        }
        PseudoClass::LastOfType => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let my_tag = dom.tag(node_id);
                let children = &dom.nodes[pid].children;
                for &c in children.iter().rev() {
                    if dom.tag(c) == my_tag {
                        return c == node_id;
                    }
                }
            }
            false
        }
        PseudoClass::Empty => {
            dom.nodes[node_id].children.is_empty()
        }
        PseudoClass::Not(selectors) => {
            // :not(a, b, c) — matches if NONE of the listed selectors match.
            !selectors.iter().any(|sel| simple_matches(sel, dom, node_id))
        }
        PseudoClass::Checked | PseudoClass::Disabled | PseudoClass::Enabled => {
            // Check for corresponding HTML attributes
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                match pc {
                    PseudoClass::Checked => attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "checked")),
                    PseudoClass::Disabled => attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "disabled")),
                    PseudoClass::Enabled => !attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "disabled")),
                    _ => false,
                }
            } else {
                false
            }
        }
        // :is() — matches if any selector in the list matches.
        PseudoClass::Is(selectors) | PseudoClass::Where(selectors) => {
            selectors.iter().any(|sel| simple_selector_matches(sel, dom, node_id))
        }
        // :has() — matches if any descendant matches the inner selector.
        // For performance, only check direct children (shallow :has).
        PseudoClass::Has(sel) => {
            let children = &dom.nodes[node_id].children;
            children.iter().any(|&c| simple_selector_matches(sel, dom, c))
        }
        // :focus-visible, :focus-within — stateful, not applicable in static rendering.
        PseudoClass::FocusVisible | PseudoClass::FocusWithin => false,
        // :placeholder-shown — check if input has no value.
        PseudoClass::PlaceholderShown => {
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                let has_value = attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "value") && !a.value.is_empty());
                !has_value
            } else { false }
        }
        // Stateful pseudo-classes (hover, active, focus, visited) are not
        // applicable in static rendering; always return false.
        PseudoClass::Hover | PseudoClass::Active | PseudoClass::Focus | PseudoClass::Visited => false,
    }
}

/// Check if a SimpleSelector matches a node (used for :is/:where/:has).
fn simple_selector_matches(sel: &SimpleSelector, dom: &Dom, node_id: NodeId) -> bool {
    if let Some(tag) = sel.tag {
        if dom.tag(node_id) != Some(tag) { return false; }
    }
    if let Some(ref id) = sel.id {
        if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
            let has_id = attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "id") && eq_ignore_ascii_case(&a.value, id));
            if !has_id { return false; }
        } else { return false; }
    }
    for cls in &sel.classes {
        if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
            let class_attr = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "class"));
            let has_class = class_attr.map_or(false, |a| {
                a.value.split_whitespace().any(|c| eq_ignore_ascii_case(c, cls))
            });
            if !has_class { return false; }
        } else { return false; }
    }
    true
}

fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() { return false; }
    eq_ignore_ascii_case(&haystack[..needle.len()], needle)
}

fn ends_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() { return false; }
    eq_ignore_ascii_case(&haystack[haystack.len() - needle.len()..], needle)
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() { return true; }
    if haystack.len() < needle.len() { return false; }
    for i in 0..=(haystack.len() - needle.len()) {
        if eq_ignore_ascii_case(&haystack[i..i + needle.len()], needle) {
            return true;
        }
    }
    false
}

/// Check if `class_str` (space-separated class list) contains `needle`
/// (case-insensitive).
fn has_class(class_str: &str, needle: &str) -> bool {
    for tok in class_str.split(|c: char| c == ' ' || c == '\t' || c == '\n') {
        if eq_ignore_ascii_case(tok, needle) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Resolve styles for entire DOM
// ---------------------------------------------------------------------------

/// Compute the final resolved style for every node in the DOM.
/// Returns a `Vec<ComputedStyle>` indexed by `NodeId`.
pub fn resolve_styles(
    dom: &Dom,
    stylesheets: &[&Stylesheet],
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<(usize, Vec<Declaration>)>,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    // Store viewport dimensions for resolve_length() to resolve vh/vw units.
    unsafe {
        VIEWPORT_W = viewport_width;
        VIEWPORT_H = viewport_height;
    }

    let count = dom.nodes.len();
    crate::debug_surf!("[style] resolve_styles: {} nodes, {} stylesheets", count, stylesheets.len());
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!("[style]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());

    let mut styles: Vec<ComputedStyle> = Vec::with_capacity(count);
    let root_font_size: i32 = 16;

    // ── Pre-collect all applicable CSS rules ONCE (node-independent). ──
    let mut all_rules: Vec<(&Rule, usize)> = Vec::new();
    let mut order = 0usize;
    for sheet in stylesheets {
        for rule in &sheet.rules {
            all_rules.push((rule, order));
            order += 1;
        }
        for mr in &sheet.media_rules {
            if crate::css::evaluate_media_query(&mr.query, viewport_width, viewport_height) {
                for rule in &mr.rules {
                    all_rules.push((rule, order));
                    order += 1;
                }
            }
        }
    }
    crate::debug_surf!("[style] collected {} applicable rules (once)", all_rules.len());

    // Build rule index for O(1) tag/id/class lookup (avoids O(nodes × rules) brute force).
    let rule_index = RuleIndex::build(&all_rules);
    crate::debug_surf!("[style] rule index: {} wildcard, {} id-buckets, {} class-buckets",
        rule_index.wildcard.len(), rule_index.by_id.len(), rule_index.by_class.len());

    // Reusable scratch buffers for per-node matching (avoids repeated alloc/free).
    let mut matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(64);
    let mut candidates: Vec<usize> = Vec::with_capacity(128);
    let mut seen_bitset: Vec<u64> = Vec::with_capacity((all_rules.len() + 63) / 64);

    // Separate storage for custom properties (--name: value).
    // Only nodes that DEFINE custom properties have non-empty entries.
    // var() references are resolved on-demand by walking the DOM parent chain,
    // eliminating the per-node clone that caused heap-stack collision on large
    // pages (~54 MiB for chip.de's 6228 nodes).
    let mut custom_props: Vec<Vec<(String, String)>> = vec![Vec::new(); count];

    for id in 0..count {
        #[cfg(feature = "debug_surf")]
        {
            if id < 5 || id % 1000 == 0 {
                crate::debug_surf!("[style] node {}/{} RSP=0x{:X} heap=0x{:X}",
                    id, count, crate::debug_rsp(), crate::debug_heap_pos());
            }
        }

        let node = &dom.nodes[id];
        let parent_fs = node.parent.map_or(16, |pid| {
            if pid < id { styles[pid].font_size } else { 16 }
        });

        // Phase 1: Start from UA defaults (elements) or initial values (text).
        let (mut style, mut set_flags) = match &node.node_type {
            NodeType::Element { tag, .. } => ua_style_and_flags(*tag),
            NodeType::Text(_) => {
                let mut s = default_style();
                s.display = Display::Inline;
                (s, 0u16)
            }
        };

        // UA override: <input type="hidden"> → display:none (per HTML spec).
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            if *tag == Tag::Input {
                if attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "type")
                    && eq_ignore_ascii_case(&a.value, "hidden"))
                {
                    style.display = Display::None;
                }
            }
        }

        // Phase 1b: Presentational hints from HTML attributes (specificity 0,
        // per HTML spec §15.3.3). Applied after UA styles but before author rules.
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            // `align` attribute on div, main, nav, header, footer, section,
            // article, aside, hgroup, address, center, p, h1-h6, blockquote,
            // figure, figcaption, details, summary, dialog, search.
            // Maps to text-align (HTML spec §15.3.3).
            let supports_align = matches!(tag,
                Tag::Div | Tag::Main | Tag::Nav | Tag::Header | Tag::Footer
                | Tag::Section | Tag::Article | Tag::Aside | Tag::Hgroup
                | Tag::Address | Tag::Center | Tag::P
                | Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6
                | Tag::Blockquote | Tag::Figure | Tag::Figcaption
                | Tag::Details | Tag::Summary | Tag::Dialog | Tag::Search
            );
            if supports_align {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "align") {
                        let val = a.value.trim();
                        let align = if val.eq_ignore_ascii_case("left") {
                            Some(TextAlignVal::Left)
                        } else if val.eq_ignore_ascii_case("right") {
                            Some(TextAlignVal::Right)
                        } else if val.eq_ignore_ascii_case("center") {
                            Some(TextAlignVal::Center)
                        } else if val.eq_ignore_ascii_case("justify") {
                            Some(TextAlignVal::Justify)
                        } else {
                            None
                        };
                        if let Some(ta) = align {
                            style.text_align = ta;
                            set_flags |= SET_TEXT_ALIGN;
                        }
                        break;
                    }
                }
            }
        }

        // Phase 2 + 3: Apply author rules and inline styles.
        // Custom property declarations are stored in custom_props[id].
        // var() references are resolved by walking the parent chain.
        if matches!(node.node_type, NodeType::Element { .. }) {
            let (ancestors_cp, current_and_rest) = custom_props.split_at_mut(id);
            let node_cp = &mut current_and_rest[0];

            set_flags |= apply_author_rules(
                &mut style, dom, id, &all_rules, &rule_index,
                &mut candidates, &mut seen_bitset, &mut matches,
                parent_fs, root_font_size, node_cp, ancestors_cp,
            );

            // Phase 3: Apply inline styles (highest specificity).
            // Uses a cache to avoid re-parsing style="..." on every relayout.
            if let NodeType::Element { attrs, .. } = &node.node_type {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "style") {
                        // Look up cached declarations for this node, or parse and cache.
                        let cached_idx = inline_style_cache.iter().position(|(nid, _)| *nid == id);
                        let inline_decls: &[Declaration] = if let Some(ci) = cached_idx {
                            &inline_style_cache[ci].1
                        } else {
                            let parsed = crate::css::parse_inline_style(&a.value);
                            inline_style_cache.push((id, parsed));
                            &inline_style_cache.last().unwrap().1
                        };

                        for decl in inline_decls {
                            if let Property::CustomProperty(ref name) = decl.property {
                                if let CssValue::Keyword(ref val) = decl.value {
                                    store_custom_prop(node_cp, name, val);
                                }
                            } else if let CssValue::Var(_, _) = &decl.value {
                                let resolved = resolve_var_in_decl(
                                    decl, dom, id, node_cp, ancestors_cp,
                                );
                                set_flags |= decl_set_flag(&resolved.property);
                                apply_declaration(
                                    &mut style, &resolved, parent_fs, root_font_size,
                                );
                            } else if has_nested_var(decl) {
                                let resolved = resolve_nested_var_decl(
                                    decl, dom, id, node_cp, ancestors_cp,
                                );
                                set_flags |= decl_set_flag(&resolved.property);
                                apply_declaration(
                                    &mut style, &resolved, parent_fs, root_font_size,
                                );
                            } else {
                                set_flags |= decl_set_flag(&decl.property);
                                apply_declaration(
                                    &mut style, decl, parent_fs, root_font_size,
                                );
                            }
                        }
                        break;
                    }
                }
            }
        }

        // (Phase 3b removed: custom properties are resolved on-demand via
        // parent chain walk, eliminating the per-node clone that caused
        // heap-stack collision on large pages.)

        // Phase 4: Inherit inheritable properties NOT explicitly set.
        if let Some(pid) = node.parent {
            if pid < id {
                inherit_unset(&mut style, &styles[pid], set_flags);
            }
        }

        // Phase 5: Resolve `li` list_style from parent (ol -> decimal).
        if let NodeType::Element { tag: Tag::Li, .. } = &node.node_type {
            if set_flags & SET_LIST_STYLE != 0 && style.list_style == ListStyle::Disc {
                if let Some(pid) = node.parent {
                    if dom.tag(pid) == Some(Tag::Ol) {
                        style.list_style = ListStyle::Decimal;
                    }
                }
            }
        }

        // Phase 6: Resolve auto line_height.
        if style.line_height == 0 {
            style.line_height = (style.font_size * 6 + 2) / 5;
        }

        styles.push(style);
    }

    crate::debug_surf!("[style] resolve_styles done: {} styles", styles.len());
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!("[style]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());

    // ── Phase 7: Resolve ::before/::after pseudo-element styles ──
    let mut pseudo = PseudoStyles::empty(count);
    for id in 0..count {
        let node = &dom.nodes[id];
        if !matches!(node.node_type, NodeType::Element { .. }) {
            continue;
        }
        // Check all rules for pseudo-element selectors targeting this node.
        for &(rule, _order) in &all_rules {
            for sel in &rule.selectors {
                let pe = sel.pseudo_element();
                if pe.is_none() { continue; }
                // Check if the base selector (without pseudo-element) matches this node.
                if !selector_matches_base(sel, dom, id) {
                    continue;
                }
                let pe = pe.unwrap();
                let slot = match pe {
                    PseudoElement::Before => &mut pseudo.before[id],
                    PseudoElement::After => &mut pseudo.after[id],
                };
                // Create or update the pseudo-element style.
                // Start from parent style (inherit), then apply rule declarations.
                if slot.is_none() {
                    let mut ps = styles[id].clone();
                    ps.content = None;
                    ps.content_url = None;
                    // Reset non-inherited properties to defaults.
                    ps.background_color = 0;
                    ps.border_width = 0;
                    ps.padding_top = 0;
                    ps.padding_right = 0;
                    ps.padding_bottom = 0;
                    ps.padding_left = 0;
                    ps.margin_top = 0;
                    ps.margin_right = 0;
                    ps.margin_bottom = 0;
                    ps.margin_left = 0;
                    ps.width = None;
                    ps.height = None;
                    ps.display = Display::Inline;
                    *slot = Some(ps);
                }
                let ps = slot.as_mut().unwrap();
                let parent_fs = styles[id].font_size;
                let root_fs = 16;
                for decl in &rule.declarations {
                    apply_declaration(ps, decl, parent_fs, root_fs);
                }
            }
        }
        // Resolve line_height for pseudo styles.
        if let Some(ref mut ps) = pseudo.before[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            // Only keep if content is set and non-empty.
            if ps.content.is_none() {
                pseudo.before[id] = None;
            }
        }
        if let Some(ref mut ps) = pseudo.after[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            if ps.content.is_none() {
                pseudo.after[id] = None;
            }
        }
    }

    (styles, pseudo)
}

fn apply_author_rules(
    style: &mut ComputedStyle,
    dom: &Dom,
    node_id: NodeId,
    all_rules: &[(&Rule, usize)],
    rule_index: &RuleIndex,
    candidates: &mut Vec<usize>,
    seen_bitset: &mut Vec<u64>,
    matches: &mut Vec<((u32, u32, u32), usize)>,
    parent_fs: i32,
    root_fs: i32,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
) -> u16 {
    // Reuse the caller's matches buffer (avoids alloc/free per node).
    matches.clear();

    // Use the rule index to get only candidate rules for this node's tag/id/classes.
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (*tag, attrs),
        _ => return 0,
    };
    let id_attr = attrs.iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "id"))
        .map(|a| a.value.as_str());
    let class_attr = attrs.iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
        .map(|a| a.value.as_str());

    rule_index.candidates(tag, id_attr, class_attr, candidates, seen_bitset);

    for &idx in candidates.iter() {
        let (rule, _order) = all_rules[idx];
        for sel in &rule.selectors {
            if selector_matches(sel, dom, node_id) {
                matches.push((sel.specificity(), idx));
                break;
            }
        }
    }

    // Sort by specificity (ascending); equal specificity keeps source order.
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut set_flags: u16 = 0;

    // Phase 1: Apply normal (non-!important) declarations.
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        for decl in &rule.declarations {
            if !decl.important {
                if let Property::CustomProperty(ref name) = decl.property {
                    if let CssValue::Keyword(ref val) = decl.value {
                        store_custom_prop(node_cp, name, val);
                    }
                } else if let CssValue::Var(_, _) = &decl.value {
                    let resolved = resolve_var_in_decl(decl, dom, node_id, node_cp, ancestors_cp);
                    set_flags |= decl_set_flag(&resolved.property);
                    apply_declaration(style, &resolved, parent_fs, root_fs);
                } else if has_nested_var(decl) {
                    let resolved = resolve_nested_var_decl(decl, dom, node_id, node_cp, ancestors_cp);
                    set_flags |= decl_set_flag(&resolved.property);
                    apply_declaration(style, &resolved, parent_fs, root_fs);
                } else {
                    set_flags |= decl_set_flag(&decl.property);
                    apply_declaration(style, decl, parent_fs, root_fs);
                }
            }
        }
    }

    // Phase 2: Apply !important declarations (override normal ones).
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        for decl in &rule.declarations {
            if decl.important {
                if let Property::CustomProperty(ref name) = decl.property {
                    if let CssValue::Keyword(ref val) = decl.value {
                        store_custom_prop(node_cp, name, val);
                    }
                } else if let CssValue::Var(_, _) = &decl.value {
                    let resolved = resolve_var_in_decl(decl, dom, node_id, node_cp, ancestors_cp);
                    set_flags |= decl_set_flag(&resolved.property);
                    apply_declaration(style, &resolved, parent_fs, root_fs);
                } else if has_nested_var(decl) {
                    let resolved = resolve_nested_var_decl(decl, dom, node_id, node_cp, ancestors_cp);
                    set_flags |= decl_set_flag(&resolved.property);
                    apply_declaration(style, &resolved, parent_fs, root_fs);
                } else {
                    set_flags |= decl_set_flag(&decl.property);
                    apply_declaration(style, decl, parent_fs, root_fs);
                }
            }
        }
    }

    set_flags
}

/// Store a custom property in a node's custom property list.
fn store_custom_prop(cp: &mut Vec<(String, String)>, name: &str, val: &str) {
    if let Some(existing) = cp.iter_mut().find(|(k, _)| k == name) {
        existing.1.clear();
        existing.1.push_str(val);
    } else {
        cp.push((String::from(name), String::from(val)));
    }
}

/// Look up a custom property by walking the DOM parent chain.
///
/// Checks the current node's own custom properties first, then walks up
/// the ancestor chain. Returns the raw value string if found.
fn lookup_custom_property<'a>(
    name: &str,
    node_cp: &'a [(String, String)],
    dom: &Dom,
    node_id: NodeId,
    ancestors_cp: &'a [Vec<(String, String)>],
) -> Option<&'a str> {
    // Check this node's own custom properties first.
    if let Some((_, val)) = node_cp.iter().find(|(k, _)| k == name) {
        return Some(val.as_str());
    }
    // Walk up the parent chain.
    let mut cur = dom.nodes[node_id].parent;
    while let Some(pid) = cur {
        if pid < ancestors_cp.len() {
            if let Some((_, val)) = ancestors_cp[pid].iter().find(|(k, _)| k == name) {
                return Some(val.as_str());
            }
            cur = dom.nodes[pid].parent;
        } else {
            break;
        }
    }
    None
}

/// Resolve var() references by walking the DOM parent chain.
fn resolve_var_in_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if let CssValue::Var(ref name, ref fallback) = decl.value {
        // Look up custom property via parent chain walk.
        if let Some(val) = lookup_custom_property(name, node_cp, dom, node_id, ancestors_cp) {
            // Re-parse the raw value string as the target property.
            let resolved = crate::css::parse_value(&decl.property, val);
            return Declaration {
                property: decl.property.clone(),
                value: resolved,
                important: decl.important,
            };
        }
        // Use fallback if available.
        if let Some(fb) = fallback {
            return Declaration {
                property: decl.property.clone(),
                value: (**fb).clone(),
                important: decl.important,
            };
        }
        // No value found — return as-is (will be treated as unknown).
        return decl.clone();
    }
    decl.clone()
}

/// Check if a declaration has nested var() inside a function value (e.g. rgb(R G B/var(--x,1))).
fn has_nested_var(decl: &Declaration) -> bool {
    if let CssValue::Keyword(ref s) = decl.value {
        s.contains("var(")
    } else {
        false
    }
}

/// Resolve nested var() references within a value string, e.g.
/// "rgb(31 30 28/var(--tw-bg-opacity,1))" → "rgb(31 30 28/1)"
fn resolve_nested_vars(
    value: &str,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> String {
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"var(" {
            // Find matching closing paren, respecting nesting
            let start = i + 4;
            let mut depth: u32 = 1;
            let mut end = start;
            while end < bytes.len() && depth > 0 {
                if bytes[end] == b'(' { depth += 1; }
                if bytes[end] == b')' { depth -= 1; }
                if depth > 0 { end += 1; }
            }
            let inner = &value[start..end]; // content between var( and )
            // Split on first comma for fallback
            let (var_name, fallback) = if let Some(comma) = inner.find(',') {
                (inner[..comma].trim(), Some(inner[comma + 1..].trim()))
            } else {
                (inner.trim(), None)
            };
            // Look up the variable
            if let Some(val) = lookup_custom_property(var_name, node_cp, dom, node_id, ancestors_cp) {
                result.push_str(val);
            } else if let Some(fb) = fallback {
                // Recursively resolve vars in fallback too
                let resolved_fb = resolve_nested_vars(fb, dom, node_id, node_cp, ancestors_cp);
                result.push_str(&resolved_fb);
            } else {
                // No value, no fallback — keep original
                result.push_str(&value[i..end + 1]);
            }
            i = end + 1; // skip past closing )
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Resolve a declaration that has nested var() in its Keyword value.
fn resolve_nested_var_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if let CssValue::Keyword(ref s) = decl.value {
        let resolved_str = resolve_nested_vars(s, dom, node_id, node_cp, ancestors_cp);
        let resolved = crate::css::parse_value(&decl.property, &resolved_str);
        Declaration {
            property: decl.property.clone(),
            value: resolved,
            important: decl.important,
        }
    } else {
        decl.clone()
    }
}

// ---------------------------------------------------------------------------
// Inheritance (only unset inheritable properties)
// ---------------------------------------------------------------------------

fn inherit_unset(child: &mut ComputedStyle, parent: &ComputedStyle, set: u16) {
    if set & SET_COLOR == 0      { child.color = parent.color; }
    if set & SET_FONT_SIZE == 0  { child.font_size = parent.font_size; }
    if set & SET_FONT_WEIGHT == 0 { child.font_weight = parent.font_weight; }
    if set & SET_FONT_STYLE == 0 { child.font_style = parent.font_style; }
    if set & SET_TEXT_ALIGN == 0 { child.text_align = parent.text_align; }
    if set & SET_LINE_HEIGHT == 0 { child.line_height = parent.line_height; }
    if set & SET_WHITE_SPACE == 0 { child.white_space = parent.white_space; }
    if set & SET_LIST_STYLE == 0 { child.list_style = parent.list_style; }
    if set & SET_LIST_STYLE_POS == 0 { child.list_style_position = parent.list_style_position; }
    if set & SET_TEXT_DECO == 0  { child.text_decoration = parent.text_decoration; }
    if set & SET_VISIBILITY == 0 { child.visibility = parent.visibility; }
    if set & SET_TEXT_TRANSFORM == 0 { child.text_transform = parent.text_transform; }
    if set & SET_LETTER_SPACING == 0 { child.letter_spacing = parent.letter_spacing; }
    if set & SET_WORD_SPACING == 0 { child.word_spacing = parent.word_spacing; }
    if set & SET_WORD_BREAK == 0 { child.word_break = parent.word_break; }
    if set & SET_OVERFLOW_WRAP == 0 { child.overflow_wrap = parent.overflow_wrap; }
}

/// Map a CSS property to the inheritable-set bitflag (0 if not inheritable).
fn decl_set_flag(prop: &Property) -> u16 {
    match prop {
        Property::Color => SET_COLOR,
        Property::FontSize => SET_FONT_SIZE,
        Property::FontWeight => SET_FONT_WEIGHT,
        Property::FontStyle => SET_FONT_STYLE,
        Property::TextAlign => SET_TEXT_ALIGN,
        Property::LineHeight => SET_LINE_HEIGHT,
        Property::WhiteSpace => SET_WHITE_SPACE,
        Property::ListStyleType => SET_LIST_STYLE,
        Property::ListStylePosition => SET_LIST_STYLE_POS,
        Property::TextDecoration => SET_TEXT_DECO,
        Property::Visibility => SET_VISIBILITY,
        Property::TextTransform => SET_TEXT_TRANSFORM,
        Property::LetterSpacing => SET_LETTER_SPACING,
        Property::WordSpacing => SET_WORD_SPACING,
        Property::WordBreak => SET_WORD_BREAK,
        Property::OverflowWrap => SET_OVERFLOW_WRAP,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Declaration application
// ---------------------------------------------------------------------------

/// Resolve a CSS length value to pixels.
///
/// `CssValue::Length` stores fixed-point * 100: "16px" -> Length(1600, Px),
/// "1.5em" -> Length(150, Em), "2rem" -> Length(200, Rem).
///
/// Conversion formulas (v = stored value):
///   Px:  pixels = v / 100
///   Em:  pixels = v * parent_fs / 100
///   Rem: pixels = v * root_fs / 100
///   Pt:  pixels = v * 4 / 300   (1pt ~= 1.333px)
fn resolve_length(val: &CssValue, parent_fs: i32, root_fs: i32) -> Option<i32> {
    match val {
        CssValue::Length(v, Unit::Px) => Some(v / 100),
        CssValue::Length(v, Unit::Em) => Some(v * parent_fs / 100),
        CssValue::Length(v, Unit::Rem) => Some(v * root_fs / 100),
        CssValue::Length(v, Unit::Pt) => Some(v * 4 / 300),
        CssValue::Length(_, Unit::Percent) => Option::None,
        // fr units are meaningful only inside a grid container; cannot resolve here.
        CssValue::Length(_, Unit::Fr) => Option::None,
        // Viewport units: 1vw = 1% of viewport width, etc.
        CssValue::Length(v, Unit::Vw) => {
            let vw = unsafe { VIEWPORT_W };
            Some((*v as i64 * vw as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vh) => {
            let vh = unsafe { VIEWPORT_H };
            Some((*v as i64 * vh as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vmin) => {
            let dim = unsafe { VIEWPORT_W.min(VIEWPORT_H) };
            Some((*v as i64 * dim as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vmax) => {
            let dim = unsafe { VIEWPORT_W.max(VIEWPORT_H) };
            Some((*v as i64 * dim as i64 / 10000) as i32)
        }
        CssValue::Number(v) => Some(v / 100),
        CssValue::Percentage(_) => Option::None,
        CssValue::Calc(px, _pct) => {
            // For margin/padding/etc (non-width/height), evaluate calc as best we can.
            // The px component is always resolved; the pct component is lost here.
            Some(px / 100)
        }
        _ => Option::None,
    }
}

/// Apply a single CSS declaration to a computed style.
/// Parse a CSS length value from a transform function argument.
/// Supports: px, em, rem, %, and bare numbers (treated as px).
fn parse_transform_length(s: &str, parent_fs: i32) -> i32 {
    let s = s.trim();
    if s.ends_with("px") {
        let num = &s[..s.len() - 2];
        parse_simple_float(num)
    } else if s.ends_with("em") {
        let num = &s[..s.len() - 2];
        let v = parse_simple_float(num);
        v * parent_fs / 100
    } else if s.ends_with("rem") {
        let num = &s[..s.len() - 3];
        let v = parse_simple_float(num);
        v * 16 / 100
    } else if s.ends_with('%') {
        // Percentage in translate is relative to the element's own size,
        // which we don't know here. Store as-is and resolve at layout time.
        // For now, treat as 0 (can't resolve without element dimensions).
        0
    } else {
        // Bare number — treat as px.
        parse_simple_float(s)
    }
}

/// Parse a simple float like "10", "-5.5", "0" to an integer.
fn parse_simple_float(s: &str) -> i32 {
    let s = s.trim();
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };
    let mut int_part = 0i32;
    let mut frac = 0i32;
    let mut in_frac = false;
    let mut frac_mul = 10;
    for &b in s.as_bytes() {
        if b == b'.' {
            in_frac = true;
        } else if b.is_ascii_digit() {
            if in_frac {
                if frac_mul <= 100 {
                    frac += (b - b'0') as i32 * (100 / frac_mul);
                    frac_mul *= 10;
                }
            } else {
                int_part = int_part * 10 + (b - b'0') as i32;
            }
        }
    }
    let result = int_part;
    if neg { -result } else { result }
}

pub fn apply_declaration(
    style: &mut ComputedStyle,
    decl: &Declaration,
    parent_fs: i32,
    root_fs: i32,
) {
    match decl.property {
        Property::Display => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.display = match kw.as_str() {
                    "block" => Display::Block,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "list-item" => Display::ListItem,
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "grid" => Display::Grid,
                    "inline-grid" => Display::InlineGrid,
                    "flow-root" => Display::FlowRoot,
                    "none" => Display::None,
                    "contents" => Display::Contents,
                    _ => style.display,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.display = Display::None;
            }
        }
        Property::Color => {
            if let CssValue::Color(c) = decl.value { style.color = c; }
        }
        Property::BackgroundColor | Property::Background => {
            match decl.value {
                CssValue::Color(c) => { style.background_color = c; }
                CssValue::None => { style.background_color = 0x00000000; }
                CssValue::CurrentColor => {
                    // currentColor → use this element's computed color property.
                    style.background_color = if style.color != 0 { style.color } else { 0xFF000000 };
                }
                _ => {}
            }
        }
        Property::FontSize => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                if px > 0 { style.font_size = px; }
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_size = match kw.as_str() {
                    "xx-small" => 9,
                    "x-small"  => 10,
                    "small"    => 13,
                    "medium"   => 16,
                    "large"    => 18,
                    "x-large"  => 24,
                    "xx-large" => 32,
                    "smaller"  => (parent_fs * 5 + 3) / 6, // ~0.833x
                    "larger"   => (parent_fs * 6 + 2) / 5, // ~1.2x
                    _ => style.font_size,
                };
            }
        }
        Property::FontWeight => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_weight = match kw.as_str() {
                    "bold" | "bolder" => FontWeight::Bold,
                    "normal" | "lighter" => FontWeight::Normal,
                    _ => style.font_weight,
                };
            }
            if let CssValue::Number(v) = decl.value {
                style.font_weight = if v / 100 >= 700 {
                    FontWeight::Bold
                } else {
                    FontWeight::Normal
                };
            }
        }
        Property::FontStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_style = match kw.as_str() {
                    "italic" | "oblique" => FontStyleVal::Italic,
                    _ => FontStyleVal::Normal,
                };
            }
        }
        Property::TextAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_align = match kw.as_str() {
                    "center" => TextAlignVal::Center,
                    "right" => TextAlignVal::Right,
                    "justify" => TextAlignVal::Justify,
                    _ => TextAlignVal::Left,
                };
            }
        }
        Property::TextDecoration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_decoration = match kw.as_str() {
                    "underline" => TextDeco::Underline,
                    "line-through" => TextDeco::LineThrough,
                    "overline" => TextDeco::Overline,
                    "none" => TextDeco::None,
                    _ => style.text_decoration,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.text_decoration = TextDeco::None;
            }
        }
        Property::LineHeight => {
            // line-height: <number> means multiple of font_size (not pixels).
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100, e.g. "1.5" -> 150
                style.line_height = (style.font_size * v) / 100;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.line_height = px;
            }
        }
        Property::Width => {
            // Clear all width variants first.
            style.width_max_content = false;
            style.width_min_content = false;
            style.width_fit_content = false;
            match decl.value {
                CssValue::Auto => { style.width = Option::None; style.width_pct = Option::None; style.width_calc = Option::None; }
                CssValue::Percentage(v) => { style.width_pct = Some(v); style.width = Option::None; style.width_calc = Option::None; }
                CssValue::Calc(px, pct) => { style.width_calc = Some((px, pct)); style.width = Option::None; style.width_pct = Option::None; }
                CssValue::Keyword(ref kw) => {
                    match kw.as_str() {
                        "max-content" | "-webkit-max-content" | "-moz-max-content" => {
                            style.width_max_content = true;
                            style.width = Option::None; style.width_pct = Option::None; style.width_calc = Option::None;
                        }
                        "min-content" | "-webkit-min-content" | "-moz-min-content" => {
                            style.width_min_content = true;
                            style.width = Option::None; style.width_pct = Option::None; style.width_calc = Option::None;
                        }
                        "fit-content" | "-webkit-fit-content" | "-moz-fit-content" => {
                            style.width_fit_content = true;
                            style.width = Option::None; style.width_pct = Option::None; style.width_calc = Option::None;
                        }
                        _ => {
                            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                                style.width = Some(px);
                                style.width_pct = Option::None;
                                style.width_calc = Option::None;
                            }
                        }
                    }
                }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.width = Some(px);
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                }
            }
        }
        Property::Height => {
            match decl.value {
                CssValue::Auto => { style.height = Option::None; style.height_pct = Option::None; style.height_calc = Option::None; }
                CssValue::Percentage(v) => { style.height_pct = Some(v); style.height = Option::None; style.height_calc = Option::None; }
                CssValue::Calc(px, pct) => { style.height_calc = Some((px, pct)); style.height = Option::None; style.height_pct = Option::None; }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.height = Some(px);
                        style.height_pct = Option::None;
                        style.height_calc = Option::None;
                    }
                }
            }
        }
        Property::MaxWidth => {
            match decl.value {
                CssValue::None => style.max_width = Option::None,
                CssValue::Percentage(v) => {
                    // Store percentage as negative marker; layout resolves against container.
                    style.max_width = Some(-(v.max(1)));
                }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.max_width = Some(px);
                    }
                }
            }
        }
        Property::MinWidth => {
            if let CssValue::Percentage(v) = decl.value {
                style.min_width = -(v.max(1));
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_width = px;
            }
        }
        Property::MaxHeight => {
            match decl.value {
                CssValue::None => style.max_height = Option::None,
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.max_height = Some(px);
                    }
                }
            }
        }
        Property::MinHeight => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_height = px;
            }
        }
        // Margin properties — track `auto` for centering.
        Property::Margin => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px; style.margin_right = px;
                style.margin_bottom = px; style.margin_left = px;
                style.margin_left_auto = false; style.margin_right_auto = false;
            }
        }
        Property::MarginTop => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
            }
        }
        Property::MarginRight => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_right_auto = true;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_right = px;
                style.margin_right_auto = false;
            }
        }
        Property::MarginBottom => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_bottom = px;
            }
        }
        Property::MarginLeft => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
                style.margin_left_auto = false;
            }
        }
        // Shorthand padding.
        Property::Padding => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px; style.padding_right = px;
                style.padding_bottom = px; style.padding_left = px;
            }
        }
        Property::PaddingTop => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
            }
        }
        Property::PaddingRight => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_right = px;
            }
        }
        Property::PaddingBottom => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_bottom = px;
            }
        }
        Property::PaddingLeft => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_left = px;
            }
        }
        Property::BorderWidth => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px; style.border_right.width = px;
                style.border_bottom.width = px; style.border_left.width = px;
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                let w = match kw.as_str() {
                    "thin" => 1, "medium" => 3, "thick" => 5,
                    _ => style.border_width,
                };
                style.border_width = w;
                style.border_top.width = w; style.border_right.width = w;
                style.border_bottom.width = w; style.border_left.width = w;
            }
        }
        Property::BorderColor => {
            let c = match decl.value {
                CssValue::Color(c) => Some(c),
                CssValue::CurrentColor => Some(if style.color != 0 { style.color } else { 0xFF000000 }),
                _ => None,
            };
            if let Some(c) = c {
                style.border_color = c;
                style.border_top.color = c; style.border_right.color = c;
                style.border_bottom.color = c; style.border_left.color = c;
            }
        }
        Property::BorderStyle => {
            let sv = resolve_border_style_val(&decl.value);
            style.border_top.style = sv; style.border_right.style = sv;
            style.border_bottom.style = sv; style.border_left.style = sv;
        }
        Property::BorderRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_radius = px;
                style.border_top_left_radius = px; style.border_top_right_radius = px;
                style.border_bottom_right_radius = px; style.border_bottom_left_radius = px;
            }
        }
        // Shorthand border: just pick up width and color from the value.
        Property::Border | Property::BorderTop | Property::BorderRight
        | Property::BorderBottom | Property::BorderLeft => {
            if let CssValue::Color(c) = decl.value {
                style.border_color = c;
                style.border_top.color = c; style.border_right.color = c;
                style.border_bottom.color = c; style.border_left.color = c;
            }
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px; style.border_right.width = px;
                style.border_bottom.width = px; style.border_left.width = px;
            }
        }
        Property::ListStyleType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style = match kw.as_str() {
                    "disc" => ListStyle::Disc,
                    "circle" => ListStyle::Circle,
                    "square" => ListStyle::Square,
                    "decimal" | "decimal-leading-zero" => ListStyle::Decimal,
                    "none" => ListStyle::None,
                    "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                    "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
                    "lower-roman" => ListStyle::LowerRoman,
                    "upper-roman" => ListStyle::UpperRoman,
                    _ => style.list_style,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.list_style = ListStyle::None;
            }
        }
        Property::ListStylePosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style_position = match kw.as_str() {
                    "inside" => ListStylePosition::Inside,
                    _ => ListStylePosition::Outside,
                };
            }
        }
        Property::WhiteSpace => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.white_space = match kw.as_str() {
                    "pre" => WhiteSpace::Pre,
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    _ => WhiteSpace::Normal,
                };
            }
        }
        Property::Position => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.position = match kw.as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => style.position,
                };
            }
        }
        Property::Top => {
            if matches!(decl.value, CssValue::Auto) {
                style.top = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.top = Some(px);
            }
        }
        Property::Right => {
            if matches!(decl.value, CssValue::Auto) {
                style.right_offset = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.right_offset = Some(px);
            }
        }
        Property::Bottom => {
            if matches!(decl.value, CssValue::Auto) {
                style.bottom_offset = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.bottom_offset = Some(px);
            }
        }
        Property::Left => {
            if matches!(decl.value, CssValue::Auto) {
                style.left_offset = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.left_offset = Some(px);
            }
        }
        Property::ZIndex => {
            match decl.value {
                CssValue::Number(v) => {
                    style.z_index = v / 100;
                    style.z_index_auto = false;
                }
                CssValue::Auto | CssValue::Inherit => {
                    style.z_index = 0;
                    style.z_index_auto = true;
                }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.z_index = px;
                        style.z_index_auto = false;
                    }
                }
            }
        }
        Property::FlexDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_direction = match kw.as_str() {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => style.flex_direction,
                };
            }
        }
        Property::FlexWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_wrap = match kw.as_str() {
                    "nowrap" => FlexWrap::Nowrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => style.flex_wrap,
                };
            }
        }
        Property::JustifyContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_content = match kw.as_str() {
                    "flex-start" | "start" => JustifyContent::FlexStart,
                    "flex-end" | "end" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => style.justify_content,
                };
            }
        }
        Property::AlignItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.align_items = match kw.as_str() {
                    "flex-start" | "start" => AlignItems::FlexStart,
                    "flex-end" | "end" => AlignItems::FlexEnd,
                    "center" => AlignItems::Center,
                    "stretch" => AlignItems::Stretch,
                    "baseline" => AlignItems::Baseline,
                    _ => style.align_items,
                };
            }
        }
        Property::AlignSelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.align_self = match kw.as_str() {
                    "auto" => Option::None,
                    "flex-start" | "start" => Some(AlignItems::FlexStart),
                    "flex-end" | "end" => Some(AlignItems::FlexEnd),
                    "center" => Some(AlignItems::Center),
                    "stretch" => Some(AlignItems::Stretch),
                    "baseline" => Some(AlignItems::Baseline),
                    _ => style.align_self,
                };
            }
        }
        Property::FlexGrow => {
            if let CssValue::Number(v) = decl.value {
                style.flex_grow = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_grow = px * 100;
            }
        }
        Property::FlexShrink => {
            if let CssValue::Number(v) = decl.value {
                style.flex_shrink = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_shrink = px * 100;
            }
        }
        Property::FlexBasis => {
            if matches!(decl.value, CssValue::Auto) {
                style.flex_basis = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_basis = Some(px);
            }
        }
        Property::RowGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.row_gap = px;
            }
        }
        Property::ColumnGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.column_gap = px;
            }
        }
        Property::Order => {
            if let CssValue::Number(v) = decl.value {
                style.order = v / 100;
            }
        }
        Property::BoxSizing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.box_sizing = match kw.as_str() {
                    "border-box" => BoxSizing::BorderBox,
                    "content-box" => BoxSizing::ContentBox,
                    _ => style.box_sizing,
                };
            }
        }
        Property::Float => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.float = match kw.as_str() {
                    "left" => FloatVal::Left,
                    "right" => FloatVal::Right,
                    "none" => FloatVal::None,
                    _ => style.float,
                };
            }
            if matches!(decl.value, CssValue::None) { style.float = FloatVal::None; }
        }
        Property::Clear => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.clear = match kw.as_str() {
                    "left" => ClearVal::Left,
                    "right" => ClearVal::Right,
                    "both" => ClearVal::Both,
                    "none" => ClearVal::None,
                    _ => style.clear,
                };
            }
            if matches!(decl.value, CssValue::None) { style.clear = ClearVal::None; }
        }
        Property::Opacity => {
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100: "0.5" → 50, "1" → 100
                style.opacity = ((v * 255) / 100).max(0).min(255);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.opacity = (px * 255).max(0).min(255);
            }
        }
        Property::Visibility => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.visibility = match kw.as_str() {
                    "visible" => Visibility::Visible,
                    "hidden" => Visibility::Hidden,
                    "collapse" => Visibility::Collapse,
                    _ => style.visibility,
                };
            }
        }
        Property::TextTransform => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_transform = match kw.as_str() {
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    "none" => TextTransform::None,
                    _ => style.text_transform,
                };
            }
            if matches!(decl.value, CssValue::None) { style.text_transform = TextTransform::None; }
        }
        Property::OverflowX => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_x = parse_overflow_keyword(kw);
            }
        }
        Property::OverflowY => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_y = parse_overflow_keyword(kw);
            }
        }
        // Transitions
        Property::Transition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.transitions = parse_transition_shorthand(kw);
            }
        }
        Property::TransitionProperty => {
            // Set property names on existing TransitionDef entries, or create one.
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style.transitions.resize_with(names.len().max(style.transitions.len()), || {
                    TransitionDef { property: String::new(), duration_ms: 0, timing: TimingFunction::Ease, delay_ms: 0 }
                });
                for (i, name) in names.iter().enumerate() {
                    if i < style.transitions.len() {
                        style.transitions[i].property = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::TransitionDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef { property: String::from("all"), duration_ms: ms, timing: TimingFunction::Ease, delay_ms: 0 });
                } else {
                    for t in &mut style.transitions { t.duration_ms = ms; }
                }
            }
        }
        Property::TransitionTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef { property: String::from("all"), duration_ms: 0, timing: tf, delay_ms: 0 });
                } else {
                    for t in &mut style.transitions { t.timing = tf; }
                }
            }
        }
        Property::TransitionDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef { property: String::from("all"), duration_ms: 0, timing: TimingFunction::Ease, delay_ms: ms });
                } else {
                    for t in &mut style.transitions { t.delay_ms = ms; }
                }
            }
        }
        // Animations
        Property::Animation => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.animations = parse_animation_shorthand(kw);
            }
        }
        Property::AnimationName => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style.animations.resize_with(names.len().max(style.animations.len()), || {
                    AnimationDef { name: String::new(), duration_ms: 0, timing: TimingFunction::Ease, delay_ms: 0, iteration_count: 1, alternate: false }
                });
                for (i, name) in names.iter().enumerate() {
                    if i < style.animations.len() {
                        style.animations[i].name = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::AnimationDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.animations.is_empty() {
                    style.animations.push(AnimationDef { name: String::new(), duration_ms: ms, timing: TimingFunction::Ease, delay_ms: 0, iteration_count: 1, alternate: false });
                } else {
                    for a in &mut style.animations { a.duration_ms = ms; }
                }
            }
        }
        Property::AnimationTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                for a in &mut style.animations { a.timing = tf; }
            }
        }
        Property::AnimationDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                for a in &mut style.animations { a.delay_ms = ms; }
            }
        }
        Property::AnimationIterationCount => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let count = if kw == "infinite" { 0 } else { kw.parse::<u32>().unwrap_or(1) };
                for a in &mut style.animations { a.iteration_count = count; }
            } else if let CssValue::Number(v) = decl.value {
                let count = (v / 100) as u32;
                for a in &mut style.animations { a.iteration_count = count; }
            }
        }
        Property::AnimationDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let alt = kw == "alternate" || kw == "alternate-reverse";
                for a in &mut style.animations { a.alternate = alt; }
            }
        }
        Property::AnimationFillMode | Property::AnimationPlayState => {}
        Property::TextIndent => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_indent = px;
            }
        }
        Property::VerticalAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.vertical_align = match kw.as_str() {
                    "baseline" => VerticalAlign::Baseline,
                    "top" => VerticalAlign::Top,
                    "middle" => VerticalAlign::Middle,
                    "bottom" => VerticalAlign::Bottom,
                    "text-top" => VerticalAlign::TextTop,
                    "text-bottom" => VerticalAlign::TextBottom,
                    "sub" => VerticalAlign::Sub,
                    "super" => VerticalAlign::Super,
                    _ => style.vertical_align,
                };
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.vertical_align = VerticalAlign::Length(px);
            }
        }
        Property::FontFamily => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_family = Some(kw.clone());
            }
        }
        Property::LetterSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.letter_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.letter_spacing = px;
            }
        }
        Property::WordSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.word_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.word_spacing = px;
            }
        }
        Property::WordBreak => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.word_break = match kw.as_str() {
                    "break-all" => WordBreak::BreakAll,
                    "keep-all" => WordBreak::KeepAll,
                    _ => WordBreak::Normal,
                };
            }
        }
        Property::OverflowWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_wrap = match kw.as_str() {
                    "break-word" => OverflowWrapVal::BreakWord,
                    "anywhere" => OverflowWrapVal::Anywhere,
                    _ => OverflowWrapVal::Normal,
                };
            }
        }
        Property::TextOverflow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_overflow = match kw.as_str() {
                    "ellipsis" => TextOverflowVal::Ellipsis,
                    _ => TextOverflowVal::Clip,
                };
            }
        }
        // Per-side border widths
        Property::BorderTopWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_top.width);
            style.border_width = style.border_top.width; // sync unified
        }
        Property::BorderRightWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_right.width);
        }
        Property::BorderBottomWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_bottom.width);
        }
        Property::BorderLeftWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_left.width);
        }
        // Per-side border colors
        Property::BorderTopColor => {
            if let CssValue::Color(c) = decl.value { style.border_top.color = c; }
        }
        Property::BorderRightColor => {
            if let CssValue::Color(c) = decl.value { style.border_right.color = c; }
        }
        Property::BorderBottomColor => {
            if let CssValue::Color(c) = decl.value { style.border_bottom.color = c; }
        }
        Property::BorderLeftColor => {
            if let CssValue::Color(c) = decl.value { style.border_left.color = c; }
        }
        // Per-side border styles
        Property::BorderTopStyle => {
            style.border_top.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderRightStyle => {
            style.border_right.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderBottomStyle => {
            style.border_bottom.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderLeftStyle => {
            style.border_left.style = resolve_border_style_val(&decl.value);
        }
        // Per-corner border radius
        Property::BorderTopLeftRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_top_left_radius = px;
            }
        }
        Property::BorderTopRightRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_top_right_radius = px;
            }
        }
        Property::BorderBottomRightRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_bottom_right_radius = px;
            }
        }
        Property::BorderBottomLeftRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_bottom_left_radius = px;
            }
        }
        // Outline
        Property::OutlineWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.outline_width);
        }
        Property::OutlineColor => {
            if let CssValue::Color(c) = decl.value { style.outline_color = c; }
        }
        Property::OutlineStyle => {
            style.outline_style = resolve_border_style_val(&decl.value);
        }
        Property::OutlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.outline_offset = px;
            }
        }
        // Shadows
        Property::BoxShadow => {
            if matches!(decl.value, CssValue::None) {
                style.box_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.box_shadows = parse_box_shadows(kw, parent_fs, root_fs);
            }
        }
        Property::TextShadow => {
            if matches!(decl.value, CssValue::None) {
                style.text_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.text_shadows = parse_text_shadows(kw, parent_fs, root_fs);
            }
        }
        // Background extensions
        Property::BackgroundImage => {
            if matches!(decl.value, CssValue::None) {
                style.background_image = BackgroundImageVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.background_image = parse_background_image_val(kw);
            }
        }
        Property::BackgroundSize => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_size = match kw.as_str() {
                    "cover" => BackgroundSizeVal::Cover,
                    "contain" => BackgroundSizeVal::Contain,
                    "auto" => BackgroundSizeVal::Auto,
                    _ => {
                        // Try "Wpx Hpx" or "W% H%"
                        let parts: Vec<&str> = kw.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            let h = parse_bg_size_dim(parts[1], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, h)
                        } else if parts.len() == 1 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, -1)
                        } else {
                            BackgroundSizeVal::Auto
                        }
                    }
                };
            }
            if matches!(decl.value, CssValue::Auto) {
                style.background_size = BackgroundSizeVal::Auto;
            }
        }
        Property::BackgroundRepeat => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_repeat = match kw.as_str() {
                    "repeat-x" => BackgroundRepeatVal::RepeatX,
                    "repeat-y" => BackgroundRepeatVal::RepeatY,
                    "no-repeat" => BackgroundRepeatVal::NoRepeat,
                    _ => BackgroundRepeatVal::Repeat,
                };
            }
        }
        Property::BackgroundPosition => {
            // Simplified: just parse keywords or lengths
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if !parts.is_empty() {
                    style.background_position_x = parse_bg_position_part(parts[0], parent_fs, root_fs);
                }
                if parts.len() >= 2 {
                    style.background_position_y = parse_bg_position_part(parts[1], parent_fs, root_fs);
                }
            }
        }
        // Content
        Property::Content => {
            if matches!(decl.value, CssValue::None) {
                style.content = Option::None;
                style.content_url = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                // Use the full content value parser for proper multi-value handling.
                let (text, url) = parse_content_value(kw.as_str());
                style.content = text;
                style.content_url = url;
            }
        }
        Property::ObjectFit => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.object_fit = match kw.as_str() {
                    "fill" => ObjectFit::Fill,
                    "contain" => ObjectFit::Contain,
                    "cover" => ObjectFit::Cover,
                    "none" => ObjectFit::None,
                    "scale-down" => ObjectFit::ScaleDown,
                    _ => style.object_fit,
                };
            }
        }
        Property::Transform => {
            // Parse transform functions: translate(x,y), translateX(x), translateY(y)
            if matches!(decl.value, CssValue::None) || matches!(decl.value, CssValue::Keyword(ref k) if k == "none") {
                style.transform_tx = 0;
                style.transform_ty = 0;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                let s = kw.as_str();
                let mut tx = 0i32;
                let mut ty = 0i32;
                let mut pos = 0usize;
                let bytes = s.as_bytes();
                while pos < bytes.len() {
                    // Skip whitespace
                    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') { pos += 1; }
                    if pos >= bytes.len() { break; }
                    // Read function name
                    let name_start = pos;
                    while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' { pos += 1; }
                    let fname = core::str::from_utf8(&bytes[name_start..pos]).unwrap_or("");
                    if pos < bytes.len() && bytes[pos] == b'(' {
                        pos += 1; // skip '('
                        // Read args until ')'
                        let args_start = pos;
                        while pos < bytes.len() && bytes[pos] != b')' { pos += 1; }
                        let args = core::str::from_utf8(&bytes[args_start..pos]).unwrap_or("");
                        if pos < bytes.len() { pos += 1; } // skip ')'
                        match fname {
                            "translateX" | "translatex" => {
                                tx += parse_transform_length(args.trim(), parent_fs);
                            }
                            "translateY" | "translatey" => {
                                ty += parse_transform_length(args.trim(), parent_fs);
                            }
                            "translate" => {
                                let parts: Vec<&str> = args.split(',').collect();
                                if !parts.is_empty() {
                                    tx += parse_transform_length(parts[0].trim(), parent_fs);
                                }
                                if parts.len() > 1 {
                                    ty += parse_transform_length(parts[1].trim(), parent_fs);
                                }
                            }
                            _ => {} // scale, rotate, etc. — not yet supported
                        }
                    } else {
                        break;
                    }
                }
                style.transform_tx = tx;
                style.transform_ty = ty;
            }
        }
        Property::TransformOrigin => {}
        Property::AlignContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.align_content = match kw.as_str() {
                    "flex-start" | "start" => AlignContent::FlexStart,
                    "flex-end" | "end" => AlignContent::FlexEnd,
                    "center" => AlignContent::Center,
                    "space-between" => AlignContent::SpaceBetween,
                    "space-around" => AlignContent::SpaceAround,
                    "space-evenly" => AlignContent::SpaceEvenly,
                    "stretch" => AlignContent::Stretch,
                    _ => style.align_content,
                };
            }
        }
        // Properties we parse but do not yet resolve:
        Property::BorderCollapse => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.border_collapse = kw == "collapse";
            }
        }
        Property::BorderSpacing => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_spacing = px;
            }
        }
        // Filter
        Property::Filter => {
            if matches!(decl.value, CssValue::None) {
                style.filter = FilterVal::none();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.filter = parse_filter_value(kw, parent_fs, root_fs);
            }
        }
        // Aspect ratio
        Property::AspectRatio => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "auto" {
                    style.aspect_ratio = 0;
                } else if let Some(pos) = kw.find('/') {
                    // "16 / 9" format
                    let w_str = kw[..pos].trim();
                    let h_str = kw[pos + 1..].trim();
                    if let (Some(w), Some(h)) = (try_parse_simple_float(w_str), try_parse_simple_float(h_str)) {
                        if h > 0 { style.aspect_ratio = w * 100 / h; }
                    }
                } else if let Some(v) = try_parse_simple_float(kw.trim()) {
                    style.aspect_ratio = v;
                }
            } else if let CssValue::Number(v) = decl.value {
                style.aspect_ratio = v;
            }
        }
        // Text decoration sub-properties
        Property::TextDecorationColor => {
            if let CssValue::Color(c) = decl.value { style.text_decoration_color = c; }
        }
        Property::TextDecorationStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_decoration_style = match kw.as_str() {
                    "solid" => TextDecorationStyle::Solid,
                    "double" => TextDecorationStyle::Double,
                    "dotted" => TextDecorationStyle::Dotted,
                    "dashed" => TextDecorationStyle::Dashed,
                    "wavy" => TextDecorationStyle::Wavy,
                    _ => style.text_decoration_style,
                };
            }
        }
        Property::TextDecorationThickness => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_decoration_thickness = px;
            }
        }
        Property::TextUnderlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_underline_offset = px;
            }
        }
        // Font variant
        Property::FontVariant => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_variant = match kw.as_str() {
                    "small-caps" => FontVariantVal::SmallCaps,
                    _ => FontVariantVal::Normal,
                };
            }
        }
        // Tab size
        Property::TabSize => {
            if let CssValue::Number(v) = decl.value {
                style.tab_size = (v / 100).max(1);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.tab_size = px.max(1);
            }
        }
        // Clip path
        Property::ClipPath => {
            if matches!(decl.value, CssValue::None) {
                style.clip_path = ClipPathVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_path = parse_clip_path_value(kw, parent_fs, root_fs);
            }
        }
        Property::Clip => {
            // `clip: rect(top, right, bottom, left)` for absolutely-positioned elements.
            // `clip: auto` clears the clip rect.
            if matches!(decl.value, CssValue::Auto) || matches!(decl.value, CssValue::None) {
                style.clip_rect = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_rect = parse_clip_rect(kw, parent_fs, root_fs);
            }
        }
        // CSS counters
        Property::CounterReset => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_reset = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_reset = Option::None;
            }
        }
        Property::CounterIncrement => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_increment = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_increment = Option::None;
            }
        }
        // Inset shorthand is expanded before reaching here.
        Property::Inset => {}
        Property::Overflow => {
            // `overflow` shorthand: one or two keywords.
            // One value → both axes. Two values → overflow-x overflow-y.
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if parts.len() == 1 {
                    let v = parse_overflow_keyword(parts[0]);
                    style.overflow_x = v;
                    style.overflow_y = v;
                } else if parts.len() >= 2 {
                    style.overflow_x = parse_overflow_keyword(parts[0]);
                    style.overflow_y = parse_overflow_keyword(parts[1]);
                }
            }
        }
        Property::BorderStyle
        | Property::Flex
        | Property::Gap | Property::Cursor
        | Property::TableLayout
        | Property::Outline => {}
        // Grid container properties
        Property::GridTemplateColumns => {
            style.grid_template_columns = decode_track_list(&decl.value);
        }
        Property::GridTemplateRows => {
            style.grid_template_rows = decode_track_list(&decl.value);
        }
        Property::GridTemplateAreas => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_template_areas = parse_grid_template_areas_value(kw);
            }
        }
        // GridTemplate shorthand is expanded before reaching here.
        Property::GridTemplate => {}
        Property::GridAutoColumns => {
            style.grid_auto_columns = decode_single_track(&decl.value);
        }
        Property::GridAutoRows => {
            style.grid_auto_rows = decode_single_track(&decl.value);
        }
        Property::GridAutoFlow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_auto_flow_column = kw.contains("column");
            }
        }
        Property::JustifyItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_items = parse_align_items_kw(kw);
            }
        }
        // Grid item placement
        Property::GridColumn => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_column_start = start;
                style.grid_column_end = end;
            }
        }
        Property::GridColumnStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_start = GridLine::Index(n);
            }
        }
        Property::GridColumnEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_end = GridLine::Index(n);
            }
        }
        Property::GridRow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_row_start = start;
                style.grid_row_end = end;
            }
        }
        Property::GridRowStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_start = GridLine::Index(n);
            }
        }
        Property::GridRowEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_end = GridLine::Index(n);
            }
        }
        Property::GridArea => {
            // CSS Grid §8.2: `grid-area: row-start / col-start / row-end / col-end`
            // If fewer than 4 values:
            //   1 value:  all four set to that value
            //   2 values: row-end = row-start, col-end = col-start
            //   3 values: col-end = col-start
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.splitn(4, '/').collect();
                let trimmed: Vec<&str> = parts.iter().map(|s| s.trim()).collect();
                let n = trimmed.len();
                let row_s = parse_grid_line(trimmed[0]);
                let col_s = if n >= 2 { parse_grid_line(trimmed[1]) } else { row_s.clone() };
                let row_e = if n >= 3 { parse_grid_line(trimmed[2]) } else { row_s.clone() };
                let col_e = if n >= 4 { parse_grid_line(trimmed[3]) } else { col_s.clone() };
                style.grid_row_start = row_s;
                style.grid_column_start = col_s;
                style.grid_row_end = row_e;
                style.grid_column_end = col_e;
            }
        }
        Property::CustomProperty(_) => {
            // Custom properties stored separately in resolve_styles; no-op here.
        }
        Property::MaskImage => {
            // Recognized for @supports evaluation but not visually applied.
        }
    }
}

// ---------------------------------------------------------------------------
// Grid helpers
// ---------------------------------------------------------------------------

/// Decode a `CssValue` into a list of `GridTrackSize` (for `grid-template-*`).
///
/// Single-token values such as `CssValue::Length(100, Unit::Fr)` are wrapped in
/// a one-element Vec; multi-token values arrive as `CssValue::Keyword`.
fn decode_track_list(val: &CssValue) -> Vec<GridTrackSize> {
    match val {
        CssValue::Keyword(kw) => parse_track_list(kw),
        CssValue::Auto => vec![GridTrackSize::Auto],
        CssValue::Length(v, Unit::Fr) => vec![GridTrackSize::Fr(*v)],
        CssValue::Length(v, Unit::Px) => vec![GridTrackSize::Px(v / 100)],
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => {
            vec![GridTrackSize::Percent(*v)]
        }
        _ => Vec::new(),
    }
}

/// Decode a `CssValue` into a single `GridTrackSize` (for `grid-auto-*`).
fn decode_single_track(val: &CssValue) -> GridTrackSize {
    match val {
        CssValue::Keyword(kw) => parse_single_track(kw),
        CssValue::Auto => GridTrackSize::Auto,
        CssValue::Length(v, Unit::Fr) => GridTrackSize::Fr(*v),
        CssValue::Length(v, Unit::Px) => GridTrackSize::Px(v / 100),
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => GridTrackSize::Percent(*v),
        _ => GridTrackSize::Auto,
    }
}

/// Parse a CSS track-list string such as `"100px 1fr auto"` or
/// `"repeat(3, 1fr)"` into a `Vec<GridTrackSize>`.
fn parse_track_list(s: &str) -> Vec<GridTrackSize> {
    let mut tracks = Vec::new();
    let s = s.trim();

    // Handle repeat(count, size) — supports numeric counts and auto-fill/auto-fit.
    if s.starts_with("repeat(") {
        let inner = s.trim_start_matches("repeat(").trim_end_matches(')');
        let mut parts = inner.splitn(2, ',');
        let count_str = parts.next().unwrap_or("1").trim();
        let size_str  = parts.next().unwrap_or("auto").trim();

        // Handle auto-fill / auto-fit keywords.
        if count_str == "auto-fill" || count_str == "auto-fit" {
            let min_px = parse_minmax_min(size_str);
            let track = if count_str == "auto-fill" {
                GridTrackSize::AutoFill { min_px }
            } else {
                GridTrackSize::AutoFit { min_px }
            };
            tracks.push(track);
            return tracks;
        }

        // Numeric repeat count.
        let count: usize = count_str.parse().unwrap_or(1).max(1);
        let track = parse_single_track(size_str);
        for _ in 0..count {
            tracks.push(track.clone());
        }
        return tracks;
    }

    // Space-separated list of track sizes (respecting parentheses).
    let tokens = split_whitespace_respecting_parens(s);
    for token in &tokens {
        tracks.push(parse_single_track(token));
    }
    tracks
}

/// Split a string on whitespace, but keep parenthesized groups together.
/// E.g. "12.25rem minmax(0, 1fr)" → ["12.25rem", "minmax(0, 1fr)"]
fn split_whitespace_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;
    // Skip leading whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
    start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                // Skip whitespace.
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
                start = i;
                continue;
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

/// Extract the minimum pixel value from `minmax(300px, 1fr)` or similar.
/// Falls back to 0 if the syntax is not recognized.
fn parse_minmax_min(s: &str) -> i32 {
    let s = s.trim();
    if s.starts_with("minmax(") {
        let inner = s.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, _max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            if let Some(px_val) = min_str.strip_suffix("px") {
                return px_val.trim().parse::<f32>().unwrap_or(0.0) as i32;
            }
            if let Some(pct_val) = min_str.strip_suffix('%') {
                // Store percentage as negative to distinguish from px.
                return -(pct_val.trim().parse::<f32>().unwrap_or(0.0) as i32);
            }
        }
    }
    // Not minmax(), try as a plain track size.
    match parse_single_track(s) {
        GridTrackSize::Px(px) => px,
        _ => 0,
    }
}

/// Parse a single track size token (`"100px"`, `"1fr"`, `"50%"`, `"auto"`,
/// `"minmax(200px, 1fr)"`).
pub(crate) fn parse_single_track(token: &str) -> GridTrackSize {
    let token = token.trim();
    if token == "auto" || token.is_empty() {
        return GridTrackSize::Auto;
    }
    // Handle minmax(min, max).
    if token.starts_with("minmax(") {
        let inner = token.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            let max_str = max_str.trim();
            // Parse min component → pixel value (0 for min-content/auto).
            let min_px = if min_str == "0" { 0 }
                else if min_str == "min-content" || min_str == "max-content" || min_str == "auto" { 0 }
                else if let Some(v) = min_str.strip_suffix("px") { v.parse::<f32>().map(|f| f as i32).unwrap_or(0) }
                else if let Some(v) = min_str.strip_suffix("rem") { v.parse::<f32>().map(|f| (f * 16.0) as i32).unwrap_or(0) }
                else { 0 };
            // Parse max component.
            if let Some(fr_v) = max_str.strip_suffix("fr") {
                let fr = fr_v.parse::<f32>().map(|f| (f * 100.0) as i32).unwrap_or(100);
                return GridTrackSize::Minmax { min_px, max_px: fr, max_is_fr: true };
            }
            // Non-fr max: treat as a track size with a minimum floor.
            let max_track = parse_single_track(max_str);
            return match max_track {
                GridTrackSize::Px(px) => GridTrackSize::Minmax { min_px, max_px: px, max_is_fr: false },
                GridTrackSize::Auto | GridTrackSize::MaxContent => GridTrackSize::Minmax { min_px, max_px: -1, max_is_fr: false },
                other => other,
            };
        }
        return GridTrackSize::Auto;
    }
    if let Some(fr_val) = token.strip_suffix("fr") {
        if let Ok(v) = fr_val.parse::<f32>() {
            return GridTrackSize::Fr((v * 100.0) as i32);
        }
    }
    if let Some(pct_val) = token.strip_suffix('%') {
        if let Ok(v) = pct_val.parse::<f32>() {
            return GridTrackSize::Percent((v * 100.0) as i32);
        }
    }
    if let Some(px_val) = token.strip_suffix("px") {
        if let Ok(v) = px_val.parse::<f32>() {
            return GridTrackSize::Px(v as i32);
        }
    }
    if let Some(rem_val) = token.strip_suffix("rem") {
        if let Ok(v) = rem_val.parse::<f32>() {
            // 1rem = 16px (root font-size default).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    if let Some(em_val) = token.strip_suffix("em") {
        if let Ok(v) = em_val.parse::<f32>() {
            // 1em ≈ 16px (approximation — grid tracks don't have font context).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    // Handle fit-content(value): min(max-content, max(min-content, value))
    // Approximated as Minmax { min_px: 0, max_px: value, max_is_fr: false }.
    if token.starts_with("fit-content(") && token.ends_with(')') {
        let inner = &token["fit-content(".len()..token.len() - 1];
        let max_px = if let Some(v) = inner.trim().strip_suffix("px") {
            v.parse::<f32>().unwrap_or(0.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("rem") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("em") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else {
            0
        };
        return GridTrackSize::Minmax { min_px: 0, max_px, max_is_fr: false };
    }
    // Handle min-content / max-content keywords.
    if token == "min-content" { return GridTrackSize::MinContent; }
    if token == "max-content" { return GridTrackSize::MaxContent; }
    GridTrackSize::Auto
}

/// Parse a single `GridLine` from a string token (`"auto"`, `"2"`, `"span 3"`, `"areaName"`).
fn parse_grid_line(s: &str) -> GridLine {
    let s = s.trim();
    if s.is_empty() || s == "auto" { return GridLine::Auto; }
    if let Some(rest) = s.strip_prefix("span ") {
        if let Ok(n) = rest.trim().parse::<i32>() {
            return GridLine::Span(n.max(1));
        }
    }
    if let Ok(n) = s.parse::<i32>() {
        return GridLine::Index(n);
    }
    // Named grid area — store the name for resolution at layout time.
    if s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return GridLine::Named(String::from(s));
    }
    GridLine::Auto
}

/// Parse `"start / end"` shorthand into a pair of `GridLine` values.
fn parse_grid_line_pair(s: &str) -> (GridLine, GridLine) {
    let mut it = s.splitn(2, '/');
    let start = parse_grid_line(it.next().unwrap_or("auto"));
    let end   = parse_grid_line(it.next().unwrap_or("auto"));
    (start, end)
}

/// Extract an integer from a `CssValue::Number` (fixed-point ×100).
fn try_integer(val: &CssValue) -> Option<i32> {
    if let CssValue::Number(v) = val {
        return Some(v / 100);
    }
    None
}

/// Parse an `align-items` / `justify-items` keyword into `AlignItems`.
fn parse_align_items_kw(kw: &str) -> AlignItems {
    match kw {
        "flex-start" | "start" => AlignItems::FlexStart,
        "flex-end" | "end"     => AlignItems::FlexEnd,
        "center"               => AlignItems::Center,
        "baseline"             => AlignItems::Baseline,
        _                      => AlignItems::Stretch,
    }
}

fn parse_overflow_keyword(kw: &str) -> OverflowVal {
    match kw {
        "visible" => OverflowVal::Visible,
        "hidden" => OverflowVal::Hidden,
        "scroll" => OverflowVal::Scroll,
        "auto" => OverflowVal::Auto,
        _ => OverflowVal::Visible,
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Transition / Animation helpers
// ---------------------------------------------------------------------------

/// Parse a CSS timing-function keyword.
pub(crate) fn parse_timing_function(s: &str) -> TimingFunction {
    match s.trim() {
        "linear"      => TimingFunction::Linear,
        "ease-in"     => TimingFunction::EaseIn,
        "ease-out"    => TimingFunction::EaseOut,
        "ease-in-out" => TimingFunction::EaseInOut,
        "step-start"  => TimingFunction::StepStart,
        "step-end"    => TimingFunction::StepEnd,
        _             => TimingFunction::Ease,
    }
}

/// Apply a timing function: maps progress `t ∈ [0,1]` to `[0,1]`.
/// Input and output are multiplied by 1000 (fixed-point) to avoid floats.
pub(crate) fn apply_timing(timing: TimingFunction, t: i32) -> i32 {
    // t is in [0, 1000].
    match timing {
        TimingFunction::Linear => t,
        TimingFunction::StepStart => if t > 0 { 1000 } else { 0 },
        TimingFunction::StepEnd => if t >= 1000 { 1000 } else { 0 },
        // Cubic bezier approximations (sufficient for browser rendering).
        TimingFunction::EaseIn => {
            // cubic-bezier(0.42, 0, 1, 1) ≈ t³
            let f = t as i64;
            ((f * f * f) / (1_000_000)) as i32
        }
        TimingFunction::EaseOut => {
            // cubic-bezier(0, 0, 0.58, 1) ≈ 1 - (1-t)³
            let inv = (1000 - t) as i64;
            (1000 - (inv * inv * inv / 1_000_000)) as i32
        }
        // Ease and EaseInOut use the same cheap approximation: smoothstep.
        TimingFunction::Ease | TimingFunction::EaseInOut => {
            // smoothstep: 3t² - 2t³
            let f = t as i64;
            ((3 * f * f - 2 * f * f * f / 1000) / 1000) as i32
        }
    }
}

/// Parse a CSS time value (`"0.3s"`, `"300ms"`) to milliseconds.
fn parse_time_ms(s: &str) -> u32 {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("ms") {
        return v.trim().parse::<f32>().map(|f| f as u32).unwrap_or(0);
    }
    if let Some(v) = s.strip_suffix('s') {
        return v.trim().parse::<f32>().map(|f| (f * 1000.0) as u32).unwrap_or(0);
    }
    // Pure number — assume seconds if ≤ 10, milliseconds otherwise.
    if let Ok(v) = s.parse::<f32>() {
        return if v <= 10.0 { (v * 1000.0) as u32 } else { v as u32 };
    }
    0
}

/// Parse a `transition` shorthand: `property duration timing delay`.
///
/// Comma-separated layers are each parsed into a `TransitionDef`.
fn parse_transition_shorthand(s: &str) -> Vec<TransitionDef> {
    let mut defs = Vec::new();
    for layer in s.split(',') {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() { continue; }
        let mut def = TransitionDef {
            property: String::from("all"),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 {
                    def.duration_ms = ms;
                } else {
                    def.delay_ms = ms;
                }
                time_count += 1;
            } else if matches!(*tok, "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end") {
                def.timing = parse_timing_function(tok);
            } else if *tok != "none" {
                def.property = tok.to_ascii_lowercase();
            }
        }
        defs.push(def);
    }
    defs
}

/// Parse an `animation` shorthand: `name duration timing delay iterations direction fill-mode`.
///
/// Comma-separated layers each become an `AnimationDef`.
fn parse_animation_shorthand(s: &str) -> Vec<AnimationDef> {
    let mut defs = Vec::new();
    for layer in s.split(',') {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() { continue; }
        let mut def = AnimationDef {
            name: String::new(),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
            iteration_count: 1,
            alternate: false,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 { def.duration_ms = ms; } else { def.delay_ms = ms; }
                time_count += 1;
            } else if matches!(*tok, "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end") {
                def.timing = parse_timing_function(tok);
            } else if *tok == "infinite" {
                def.iteration_count = 0;
            } else if *tok == "alternate" || *tok == "alternate-reverse" {
                def.alternate = true;
            } else if matches!(*tok, "none" | "normal" | "reverse" | "both" | "forwards" | "backwards" | "running" | "paused") {
                // Ignore direction/fill-mode/play-state keywords — not yet tracked.
            } else if let Ok(n) = tok.parse::<u32>() {
                def.iteration_count = n;
            } else if !tok.is_empty() && def.name.is_empty() {
                def.name = tok.to_ascii_lowercase();
            }
        }
        if !def.name.is_empty() {
            defs.push(def);
        }
    }
    defs
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    for i in 0..ab.len() {
        let ca = if ab[i] >= b'A' && ab[i] <= b'Z' { ab[i] + 32 } else { ab[i] };
        let cb = if bb[i] >= b'A' && bb[i] <= b'Z' { bb[i] + 32 } else { bb[i] };
        if ca != cb { return false; }
    }
    true
}

// ---------------------------------------------------------------------------
// Border helpers
// ---------------------------------------------------------------------------

fn resolve_border_width(val: &CssValue, parent_fs: i32, root_fs: i32, out: &mut i32) {
    if let Some(px) = resolve_length(val, parent_fs, root_fs) {
        *out = px;
    }
    if let CssValue::Keyword(ref kw) = *val {
        *out = match kw.as_str() {
            "thin" => 1, "medium" => 3, "thick" => 5,
            _ => *out,
        };
    }
}

fn resolve_border_style_val(val: &CssValue) -> BorderStyleVal {
    if matches!(*val, CssValue::None) { return BorderStyleVal::None; }
    if let CssValue::Keyword(ref kw) = *val {
        match kw.as_str() {
            "solid" => BorderStyleVal::Solid,
            "dashed" => BorderStyleVal::Dashed,
            "dotted" => BorderStyleVal::Dotted,
            "double" => BorderStyleVal::Double,
            "groove" => BorderStyleVal::Groove,
            "ridge" => BorderStyleVal::Ridge,
            "inset" => BorderStyleVal::Inset,
            "outset" => BorderStyleVal::Outset,
            "hidden" => BorderStyleVal::Hidden,
            "none" => BorderStyleVal::None,
            _ => BorderStyleVal::None,
        }
    } else {
        BorderStyleVal::None
    }
}

// ---------------------------------------------------------------------------
// Shadow parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse `box-shadow` value: `offset-x offset-y [blur [spread]] color [inset], ...`
fn parse_box_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<BoxShadowVal> {
    let mut shadows = Vec::new();
    for layer in s.split(',') {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" { continue; }
        let mut inset = false;
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        // Tokenize respecting parentheses (for rgb()/rgba())
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if lower == "inset" {
                inset = true;
            } else if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if lengths.len() >= 2 {
            shadows.push(BoxShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                spread: if lengths.len() >= 4 { lengths[3] } else { 0 },
                color,
                inset,
            });
        }
    }
    shadows
}

/// Parse `text-shadow` value: `offset-x offset-y [blur] color, ...`
fn parse_text_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<TextShadowVal> {
    let mut shadows = Vec::new();
    for layer in s.split(',') {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" { continue; }
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if lengths.len() >= 2 {
            shadows.push(TextShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                color,
            });
        }
    }
    shadows
}

/// Tokenize a CSS value string, keeping parenthesized groups (like `rgb(...)`) as one token.
fn tokenize_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
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
// Background image parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse `background-image` value: `url(...)` or `linear-gradient(...)`.
fn parse_background_image_val(s: &str) -> BackgroundImageVal {
    let lower = s.trim().to_ascii_lowercase();
    if lower == "none" {
        return BackgroundImageVal::None;
    }
    if lower.starts_with("url(") {
        let inner = lower.trim_start_matches("url(").trim_end_matches(')');
        let inner = inner.trim_matches('"').trim_matches('\'');
        return BackgroundImageVal::Url(String::from(inner));
    }
    if lower.starts_with("linear-gradient(") {
        let inner = lower.trim_start_matches("linear-gradient(").trim_end_matches(')');
        return parse_linear_gradient(inner);
    }
    BackgroundImageVal::None
}

/// Parse the interior of `linear-gradient(...)`.
fn parse_linear_gradient(inner: &str) -> BackgroundImageVal {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.is_empty() {
        return BackgroundImageVal::None;
    }

    let mut angle_deg: i32 = 180; // default top-to-bottom
    let mut stops = Vec::new();
    let mut start_idx = 0;

    // Check if first part is an angle or direction
    let first = parts[0].trim();
    if first.ends_with("deg") {
        if let Ok(a) = first.trim_end_matches("deg").trim().parse::<f32>() {
            angle_deg = a as i32;
        }
        start_idx = 1;
    } else if first.starts_with("to ") {
        angle_deg = match first {
            "to top" => 0,
            "to right" => 90,
            "to bottom" => 180,
            "to left" => 270,
            "to top right" | "to right top" => 45,
            "to bottom right" | "to right bottom" => 135,
            "to bottom left" | "to left bottom" => 225,
            "to top left" | "to left top" => 315,
            _ => 180,
        };
        start_idx = 1;
    }

    for i in start_idx..parts.len() {
        let part = parts[i].trim();
        // Try to parse "color position" or just "color"
        let tokens: Vec<&str> = part.split_whitespace().collect();
        let color_str = if tokens.len() >= 1 { tokens[0] } else { part };
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))
            .unwrap_or(0xFF000000);
        let position = if tokens.len() >= 2 {
            parse_gradient_position(tokens[1])
        } else {
            -1 // auto
        };
        stops.push(GradientStop { color, position });
    }

    // Auto-distribute positions for stops with position == -1
    if !stops.is_empty() {
        let len = stops.len();
        if stops[0].position < 0 { stops[0].position = 0; }
        if len > 1 && stops[len - 1].position < 0 { stops[len - 1].position = 10000; }
        // Interpolate auto positions
        let mut i = 1;
        while i < len - 1 {
            if stops[i].position < 0 {
                // Find next non-auto
                let mut j = i + 1;
                while j < len && stops[j].position < 0 { j += 1; }
                if j < len {
                    let start_pos = stops[i - 1].position;
                    let end_pos = stops[j].position;
                    let span = j - i + 1;
                    for k in i..j {
                        stops[k].position = start_pos + (end_pos - start_pos) * (k - i + 1) as i32 / span as i32;
                    }
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
    }

    BackgroundImageVal::LinearGradient { angle_deg, stops }
}

fn parse_gradient_position(s: &str) -> i32 {
    if s.ends_with('%') {
        if let Ok(v) = s.trim_end_matches('%').parse::<f32>() {
            return (v * 100.0) as i32;
        }
    }
    -1
}

fn split_comma_respecting_parens(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => { if depth > 0 { depth -= 1; } }
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn parse_bg_size_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    let s = s.trim();
    if s == "auto" { return -1; }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    -1
}

fn parse_bg_position_part(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    match s {
        "left" | "top" => 0,
        "center" => 5000, // 50% * 100
        "right" | "bottom" => 10000, // 100% * 100
        _ => {
            if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    return px;
                }
            }
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Filter parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse a CSS `filter` value like `blur(5px) grayscale(50%) brightness(120%)`.
fn parse_filter_value(s: &str, parent_fs: i32, root_fs: i32) -> FilterVal {
    let mut f = FilterVal::none();
    let s = s.trim();
    if s == "none" { return f; }

    // Tokenize function calls like "blur(5px)" "grayscale(50%)"
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') { pos += 1; }
        if pos >= bytes.len() { break; }

        // Read function name
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' { pos += 1; }
        let name = &s[name_start..pos];
        if pos >= bytes.len() || bytes[pos] != b'(' { break; }
        pos += 1; // skip '('

        // Read argument until ')'
        let arg_start = pos;
        while pos < bytes.len() && bytes[pos] != b')' { pos += 1; }
        let arg = &s[arg_start..pos];
        if pos < bytes.len() { pos += 1; } // skip ')'

        let arg = arg.trim();
        match name {
            "blur" => {
                if let Some(dim) = crate::css::try_parse_dimension_pub(arg) {
                    if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                        f.blur_px = px.max(0);
                    }
                }
            }
            "brightness" => { f.brightness = parse_filter_pct(arg); }
            "contrast" => { f.contrast = parse_filter_pct(arg); }
            "grayscale" => { f.grayscale = parse_filter_pct(arg); }
            "saturate" => { f.saturate = parse_filter_pct(arg); }
            "sepia" => { f.sepia = parse_filter_pct(arg); }
            "opacity" => { f.opacity = parse_filter_pct(arg); }
            "invert" => { f.invert = parse_filter_pct(arg); }
            "hue-rotate" => {
                let deg_str = arg.trim_end_matches("deg").trim();
                if let Ok(v) = deg_str.parse::<i32>() {
                    f.hue_rotate = v;
                }
            }
            _ => {} // drop-shadow, url() — not supported
        }
    }
    f
}

/// Parse a filter function argument as percentage (100% = 10000).
fn parse_filter_pct(s: &str) -> i32 {
    let s = s.trim();
    if s.ends_with('%') {
        let num = &s[..s.len() - 1];
        if let Ok(v) = num.parse::<i32>() {
            return v * 100;
        }
    }
    // Try as decimal (0.5 = 5000, 1.0 = 10000)
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let CssValue::Number(v) = dim {
            return v * 100; // v is already *100
        }
    }
    10000
}

/// Parse a simple float/int string to fixed-point * 100 (returns Option).
fn try_parse_simple_float(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        match dim {
            CssValue::Number(v) => return Some(v),
            CssValue::Length(v, _) => return Some(v),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Clip-path parsing
// ---------------------------------------------------------------------------

/// Parse `clip-path: circle(...)` or `clip-path: inset(...)`.
/// Parse `clip: rect(top, right, bottom, left)` into [top, right, bottom, left] in px.
/// Also accepts space-separated values (legacy syntax).
fn parse_clip_rect(s: &str, parent_fs: i32, root_fs: i32) -> Option<[i32; 4]> {
    let s = s.trim();
    // Must start with "rect("
    let inner = s.strip_prefix("rect(")?.trim_end_matches(')').trim();
    // Values can be comma- or space-separated.
    let parts: Vec<&str> = if inner.contains(',') {
        inner.split(',').map(|p| p.trim()).collect()
    } else {
        inner.split_whitespace().collect()
    };
    if parts.len() < 4 { return None; }
    let mut vals = [0i32; 4];
    for (i, p) in parts[..4].iter().enumerate() {
        vals[i] = if *p == "auto" {
            0
        } else {
            let cv = crate::css::parse_value(&crate::css::Property::Top, p);
            resolve_length(&cv, parent_fs, root_fs).unwrap_or(0)
        };
    }
    Some(vals)
}

/// Parse a CSS `content` property value.
///
/// Handles:
/// - Quoted strings: `"text"` or `'text'`
/// - `none` / `normal` → (None, None)
/// - `counter(name)` / `counter(name, style)` → encoded as `\x01COUNTER:name\x01` in text
/// - `counters(name, sep)` → encoded as `\x01COUNTER:name\x01`
/// - `url("...")` → (Some(""), Some(url))
/// - Multi-value: `"(" counter(n) ")"` → concatenated result
/// - Icon/unicode: `"\e900"` → kept as-is (Unicode escape)
///
/// Returns `(text_content, url_content)`.
pub(crate) fn parse_content_value(raw: &str) -> (Option<String>, Option<String>) {
    let s = raw.trim();
    if s.is_empty() { return (None, None); }

    let lower = s.to_ascii_lowercase();
    if lower == "none" || lower == "normal" || lower == "no-open-quote" || lower == "no-close-quote" {
        return (None, None);
    }

    // Pure url(...) without any surrounding text
    if lower.starts_with("url(") && !lower.contains('"') && !lower.contains('\'') || lower.starts_with("url(\"") || lower.starts_with("url('") {
        // Check if the whole value is url(...)
        let trimmed = s.trim_end_matches(')').trim();
        if trimmed.starts_with("url(") || trimmed.to_ascii_lowercase().starts_with("url(") {
            let url = extract_css_url(s);
            return (Some(String::new()), Some(url));
        }
    }

    // Multi-value parser: iterate over tokens
    let mut result = String::new();
    let mut url_found: Option<String> = None;
    let bytes = s.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() { break; }

        if bytes[pos] == b'"' || bytes[pos] == b'\'' {
            // Quoted string: collect content between quotes
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote {
                pos += 1;
            }
            let text = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            // Unescape CSS unicode escapes like \e900
            result.push_str(&unescape_css_string(text));
            if pos < bytes.len() { pos += 1; } // skip closing quote
        } else if rest_starts_with_ci(bytes, pos, b"counter(") {
            pos += 8;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"counters(") {
            pos += 9;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"url(") {
            // url(...) inside multi-value content
            pos += 4;
            // Skip past closing paren
            let mut depth = 1usize;
            let url_start = pos;
            while pos < bytes.len() && depth > 0 {
                if bytes[pos] == b'(' { depth += 1; }
                else if bytes[pos] == b')' { depth -= 1; }
                if depth > 0 { pos += 1; }
            }
            let url_raw = core::str::from_utf8(&bytes[url_start..pos]).unwrap_or("");
            let url = url_raw.trim().trim_matches('"').trim_matches('\'');
            url_found = Some(String::from(url));
            if pos < bytes.len() { pos += 1; }
        } else if rest_starts_with_ci(bytes, pos, b"open-quote") {
            result.push('\u{201C}');
            pos += 10;
        } else if rest_starts_with_ci(bytes, pos, b"close-quote") {
            result.push('\u{201D}');
            pos += 11;
        } else if rest_starts_with_ci(bytes, pos, b"attr(") {
            // attr(name) — skip for now
            pos += 5;
            while pos < bytes.len() && bytes[pos] != b')' { pos += 1; }
            if pos < bytes.len() { pos += 1; }
        } else {
            // Unknown token — skip to next whitespace or quote
            while pos < bytes.len()
                && bytes[pos] != b' ' && bytes[pos] != b'\t'
                && bytes[pos] != b'"' && bytes[pos] != b'\''
            {
                pos += 1;
            }
        }
    }

    if result.is_empty() && url_found.is_none() {
        // Nothing useful parsed — treat the raw value as a plain text string
        // (handles icon font chars stored as unquoted keywords)
        let stripped = s.trim_matches('"').trim_matches('\'');
        if stripped == "none" || stripped == "normal" {
            return (None, None);
        }
        if stripped.is_empty() {
            return (Some(String::new()), None);
        }
        return (Some(String::from(stripped)), None);
    }

    let text = if result.is_empty() { Some(String::new()) } else { Some(result) };
    (text, url_found)
}

/// Unescape CSS string escapes: `\e900` → U+E900, `\n` → newline, etc.
fn unescape_css_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            // Hex escape: up to 6 hex digits
            if bytes[i].is_ascii_hexdigit() {
                let start = i;
                let mut hex_end = i;
                while hex_end < bytes.len() && hex_end - start < 6 && bytes[hex_end].is_ascii_hexdigit() {
                    hex_end += 1;
                }
                let hex_str = core::str::from_utf8(&bytes[start..hex_end]).unwrap_or("0");
                if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
                i = hex_end;
                // Skip optional single whitespace after hex escape
                if i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\t') {
                    i += 1;
                }
            } else {
                // Simple escape: \n, \t, \", \\, etc.
                let c = match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b => b as char,
                };
                out.push(c);
                i += 1;
            }
        } else {
            // Pass through non-escape bytes as UTF-8.
            // Collect a run of non-backslash bytes and decode them.
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' { i += 1; }
            if let Ok(s) = core::str::from_utf8(&bytes[start..i]) {
                out.push_str(s);
            } else {
                // Fallback: push individual ASCII chars
                for b in &bytes[start..i] {
                    if *b < 128 { out.push(*b as char); }
                }
            }
        }
    }
    out
}

/// Check if `bytes[pos..]` starts with `prefix` (case-insensitive ASCII).
fn rest_starts_with_ci(bytes: &[u8], pos: usize, prefix: &[u8]) -> bool {
    if pos + prefix.len() > bytes.len() { return false; }
    for (i, &pb) in prefix.iter().enumerate() {
        let b = bytes[pos + i];
        let bl = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        let pl = if pb >= b'A' && pb <= b'Z' { pb + 32 } else { pb };
        if bl != pl { return false; }
    }
    true
}

/// Read a counter name from bytes starting at `pos` (inside counter(...) after the `(`).
/// Returns (name, new_pos) where new_pos is after the closing `)`.
fn read_counter_name(bytes: &[u8], mut pos: usize) -> (String, usize) {
    // Skip whitespace
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') { pos += 1; }
    let start = pos;
    // Read until comma or closing paren
    while pos < bytes.len() && bytes[pos] != b',' && bytes[pos] != b')' { pos += 1; }
    let name = core::str::from_utf8(&bytes[start..pos]).unwrap_or("").trim().to_ascii_lowercase();
    // Skip past closing paren (and anything between comma and paren)
    let mut depth = 1i32;
    while pos < bytes.len() && depth > 0 {
        if bytes[pos] == b'(' { depth += 1; }
        else if bytes[pos] == b')' { depth -= 1; }
        pos += 1;
    }
    (name, pos)
}

/// Extract the URL from `url("...")` or `url(...)`.
fn extract_css_url(s: &str) -> String {
    let s = s.trim();
    let inner = if let Some(rest) = s.strip_prefix("url(") {
        rest.trim_end_matches(')').trim()
    } else if let Some(rest) = s.to_ascii_lowercase().strip_prefix("url(").map(|_| &s[4..]) {
        rest.trim_end_matches(')').trim()
    } else {
        s
    };
    String::from(inner.trim_matches('"').trim_matches('\''))
}

fn parse_clip_path_value(s: &str, parent_fs: i32, root_fs: i32) -> ClipPathVal {
    let s = s.trim();
    if s == "none" { return ClipPathVal::None; }

    if s.starts_with("circle(") {
        let inner = s.trim_start_matches("circle(").trim_end_matches(')').trim();
        // "50px at 100px 100px" or "50%" or "50px"
        let parts: Vec<&str> = inner.split_whitespace().collect();
        let radius = if !parts.is_empty() {
            resolve_clip_dim(parts[0], parent_fs, root_fs)
        } else { 50 };
        let (cx, cy) = if parts.len() >= 4 && parts[1] == "at" {
            (resolve_clip_dim(parts[2], parent_fs, root_fs),
             resolve_clip_dim(parts[3], parent_fs, root_fs))
        } else { (50, 50) }; // default: center (percentage-like)
        return ClipPathVal::Circle { radius, cx, cy };
    }

    if s.starts_with("inset(") {
        let inner = s.trim_start_matches("inset(").trim_end_matches(')').trim();
        // Split on "round" for optional border-radius
        let (dims_str, radius) = if let Some(round_pos) = inner.find("round") {
            let r_str = inner[round_pos + 5..].trim();
            let r = resolve_clip_dim(r_str, parent_fs, root_fs);
            (&inner[..round_pos], r)
        } else {
            (inner, 0)
        };
        let parts: Vec<&str> = dims_str.split_whitespace().collect();
        let (t, r, b, l) = match parts.len() {
            1 => {
                let v = resolve_clip_dim(parts[0], parent_fs, root_fs);
                (v, v, v, v)
            }
            2 => {
                let tb = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                (tb, lr, tb, lr)
            }
            3 => {
                let t = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                let b = resolve_clip_dim(parts[2], parent_fs, root_fs);
                (t, lr, b, lr)
            }
            _ => {
                (resolve_clip_dim(parts[0], parent_fs, root_fs),
                 resolve_clip_dim(parts[1], parent_fs, root_fs),
                 resolve_clip_dim(parts[2], parent_fs, root_fs),
                 resolve_clip_dim(parts[3], parent_fs, root_fs))
            }
        };
        return ClipPathVal::Inset { top: t, right: r, bottom: b, left: l, radius };
    }

    ClipPathVal::None
}

fn resolve_clip_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Grid template areas parsing
// ---------------------------------------------------------------------------

/// Parse `grid-template-areas` value into named grid areas.
/// Example: `'header header' 'sidebar content' 'footer footer'`
/// Returns a list of GridArea with 1-based line numbers.
fn parse_grid_template_areas_value(s: &str) -> Vec<GridArea> {
    let mut areas: Vec<GridArea> = Vec::new();
    let mut row: i32 = 1;

    // Extract each quoted row string.
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Find start of quoted string.
        if bytes[pos] == b'\'' || bytes[pos] == b'"' {
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote { pos += 1; }
            let row_str = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            if pos < bytes.len() { pos += 1; } // skip closing quote

            // Parse cells in this row.
            let cells: Vec<&str> = row_str.split_whitespace().collect();
            for (col_idx, &name) in cells.iter().enumerate() {
                if name == "." { continue; } // empty cell
                let col = col_idx as i32 + 1; // 1-based

                // Check if this area already exists — extend it.
                if let Some(existing) = areas.iter_mut().find(|a| a.name == name) {
                    // Extend the area to cover this cell.
                    if row + 1 > existing.row_end { existing.row_end = row + 1; }
                    if col + 1 > existing.col_end { existing.col_end = col + 1; }
                    if row < existing.row_start { existing.row_start = row; }
                    if col < existing.col_start { existing.col_start = col; }
                } else {
                    areas.push(GridArea {
                        name: String::from(name),
                        row_start: row,
                        col_start: col,
                        row_end: row + 1,
                        col_end: col + 1,
                    });
                }
            }
            row += 1;
        } else {
            pos += 1;
        }
    }
    areas
}
