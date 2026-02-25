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
    AttrOp, CssValue, Declaration, PseudoClass, Property, Rule, Selector, SimpleSelector,
    Stylesheet, Unit,
};
use crate::dom::{Dom, NodeId, NodeType, Tag};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
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
    None,
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
}

/// Resolved line address for `grid-column-start/end` etc.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GridLine {
    /// Automatic placement.
    Auto,
    /// Explicit 1-based line number (may be negative for lines from the end).
    Index(i32),
    /// `span N` — spans N tracks.
    Span(i32),
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
pub enum TextDeco { None, Underline, LineThrough }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListStyle { None, Disc, Circle, Square, Decimal }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace { Normal, Pre, Nowrap, PreWrap }

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
    pub white_space: WhiteSpace,
    // Positioning
    pub position: Position,
    pub top: Option<i32>,
    pub right_offset: Option<i32>,
    pub bottom_offset: Option<i32>,
    pub left_offset: Option<i32>,
    pub z_index: i32,
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
    pub order: i32,
    // Grid container
    /// Track sizes for `grid-template-columns` (empty = no explicit columns).
    pub grid_template_columns: Vec<GridTrackSize>,
    /// Track sizes for `grid-template-rows` (empty = no explicit rows).
    pub grid_template_rows: Vec<GridTrackSize>,
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
    // Transitions
    pub transitions: Vec<TransitionDef>,
    // Animations
    pub animations: Vec<AnimationDef>,
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
        white_space: WhiteSpace::Normal,
        // Positioning
        position: Position::Static,
        top: Option::None,
        right_offset: Option::None,
        bottom_offset: Option::None,
        left_offset: Option::None,
        z_index: 0,
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
        order: 0,
        // Grid container
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
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
        // Percentages
        width_pct: Option::None,
        height_pct: Option::None,
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
        }
        Tag::Ol => {
            s.margin_top = 16; s.margin_bottom = 16; s.padding_left = 40;
        }
        Tag::Li => {
            s.display = Display::ListItem;
            s.list_style = ListStyle::Disc;
            flags |= SET_LIST_STYLE;
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
// Selector matching
// ---------------------------------------------------------------------------

/// Check if a CSS selector matches a DOM element node.
fn selector_matches(selector: &Selector, dom: &Dom, node_id: NodeId) -> bool {
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
                if selector_matches(ancestor_sel, dom, pid) {
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
                selector_matches(parent_sel, dom, pid)
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
                selector_matches(prev_sel, dom, sib)
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
                if selector_matches(prev_sel, dom, sid) {
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
        PseudoClass::Not(inner) => {
            !simple_matches(inner, dom, node_id)
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
        // Stateful pseudo-classes (hover, active, focus, visited) are not
        // applicable in static rendering; always return false.
        PseudoClass::Hover | PseudoClass::Active | PseudoClass::Focus | PseudoClass::Visited => false,
    }
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
pub fn resolve_styles(dom: &Dom, stylesheets: &[&Stylesheet], viewport_width: i32, viewport_height: i32) -> Vec<ComputedStyle> {
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

    // Reusable scratch buffer for per-node matching (avoids repeated alloc/free).
    let mut matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(64);

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
            NodeType::Text(_) => (default_style(), 0u16),
        };

        // Phase 2 + 3: Apply author rules and inline styles.
        // Custom property declarations are stored in custom_props[id].
        // var() references are resolved by walking the parent chain.
        if matches!(node.node_type, NodeType::Element { .. }) {
            let (ancestors_cp, current_and_rest) = custom_props.split_at_mut(id);
            let node_cp = &mut current_and_rest[0];

            set_flags |= apply_author_rules(
                &mut style, dom, id, &all_rules, &mut matches,
                parent_fs, root_font_size, node_cp, ancestors_cp,
            );

            // Phase 3: Apply inline styles (highest specificity).
            if let NodeType::Element { attrs, .. } = &node.node_type {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "style") {
                        let inline_decls = crate::css::parse_inline_style(&a.value);
                        for decl in &inline_decls {
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
    styles
}

fn apply_author_rules(
    style: &mut ComputedStyle,
    dom: &Dom,
    node_id: NodeId,
    all_rules: &[(&Rule, usize)],
    matches: &mut Vec<((u32, u32, u32), usize)>,
    parent_fs: i32,
    root_fs: i32,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
) -> u16 {
    // Reuse the caller's matches buffer (avoids alloc/free per node).
    matches.clear();

    for (idx, (rule, _order)) in all_rules.iter().enumerate() {
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
    if set & SET_TEXT_DECO == 0  { child.text_decoration = parent.text_decoration; }
    if set & SET_VISIBILITY == 0 { child.visibility = parent.visibility; }
    if set & SET_TEXT_TRANSFORM == 0 { child.text_transform = parent.text_transform; }
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
        Property::TextDecoration => SET_TEXT_DECO,
        Property::Visibility => SET_VISIBILITY,
        Property::TextTransform => SET_TEXT_TRANSFORM,
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
                    "none" => Display::None,
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
            match decl.value {
                CssValue::Auto => { style.width = Option::None; style.width_pct = Option::None; style.width_calc = Option::None; }
                CssValue::Percentage(v) => { style.width_pct = Some(v); style.width = Option::None; style.width_calc = Option::None; }
                CssValue::Calc(px, pct) => { style.width_calc = Some((px, pct)); style.width = Option::None; style.width_pct = Option::None; }
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
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                style.border_width = match kw.as_str() {
                    "thin" => 1, "medium" => 3, "thick" => 5,
                    _ => style.border_width,
                };
            }
        }
        Property::BorderColor => {
            if let CssValue::Color(c) = decl.value { style.border_color = c; }
        }
        Property::BorderRadius => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_radius = px;
            }
        }
        // Shorthand border: just pick up width and color from the value.
        Property::Border | Property::BorderTop | Property::BorderRight
        | Property::BorderBottom | Property::BorderLeft => {
            if let CssValue::Color(c) = decl.value { style.border_color = c; }
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
            }
        }
        Property::ListStyleType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style = match kw.as_str() {
                    "disc" => ListStyle::Disc,
                    "circle" => ListStyle::Circle,
                    "square" => ListStyle::Square,
                    "decimal" => ListStyle::Decimal,
                    "none" => ListStyle::None,
                    _ => style.list_style,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.list_style = ListStyle::None;
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
            if let CssValue::Number(v) = decl.value {
                style.z_index = v / 100;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.z_index = px;
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
        // Properties we parse but do not yet resolve:
        Property::TextIndent | Property::VerticalAlign
        | Property::BorderStyle | Property::Overflow
        | Property::AlignContent | Property::Flex
        | Property::Gap | Property::Cursor
        | Property::BorderCollapse | Property::BorderSpacing | Property::TableLayout => {}
        // Grid container properties
        Property::GridTemplateColumns => {
            style.grid_template_columns = decode_track_list(&decl.value);
        }
        Property::GridTemplateRows => {
            style.grid_template_rows = decode_track_list(&decl.value);
        }
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
            // `grid-area: row-start / col-start / row-end / col-end`
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.splitn(4, '/').collect();
                let trimmed: Vec<&str> = parts.iter().map(|s| s.trim()).collect();
                if trimmed.len() >= 1 { style.grid_row_start = parse_grid_line(trimmed[0]); }
                if trimmed.len() >= 2 { style.grid_column_start = parse_grid_line(trimmed[1]); }
                if trimmed.len() >= 3 { style.grid_row_end = parse_grid_line(trimmed[2]); }
                if trimmed.len() >= 4 { style.grid_column_end = parse_grid_line(trimmed[3]); }
            }
        }
        Property::CustomProperty(_) => {
            // Custom properties stored separately in resolve_styles; no-op here.
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

    // Handle repeat(count, size) — only uniform repeats supported.
    if s.starts_with("repeat(") {
        let inner = s.trim_start_matches("repeat(").trim_end_matches(')');
        let mut parts = inner.splitn(2, ',');
        let count_str = parts.next().unwrap_or("1").trim();
        let size_str  = parts.next().unwrap_or("auto").trim();
        let count: usize = count_str.parse().unwrap_or(1).max(1);
        let track = parse_single_track(size_str);
        for _ in 0..count {
            tracks.push(track.clone());
        }
        return tracks;
    }

    // Space-separated list of track sizes.
    for token in s.split_whitespace() {
        tracks.push(parse_single_track(token));
    }
    tracks
}

/// Parse a single track size token (`"100px"`, `"1fr"`, `"50%"`, `"auto"`).
pub(crate) fn parse_single_track(token: &str) -> GridTrackSize {
    let token = token.trim();
    if token == "auto" || token.is_empty() {
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
    GridTrackSize::Auto
}

/// Parse a single `GridLine` from a string token (`"auto"`, `"2"`, `"span 3"`).
fn parse_grid_line(s: &str) -> GridLine {
    let s = s.trim();
    if s == "auto" { return GridLine::Auto; }
    if let Some(rest) = s.strip_prefix("span ") {
        if let Ok(n) = rest.trim().parse::<i32>() {
            return GridLine::Span(n.max(1));
        }
    }
    if let Ok(n) = s.parse::<i32>() {
        return GridLine::Index(n);
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
