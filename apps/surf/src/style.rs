//! Style resolution: takes a DOM tree + CSS stylesheets and computes
//! the final `ComputedStyle` for every node.
//!
//! Cascade order: initial values -> UA defaults -> author rules (by
//! specificity) -> inline styles.  Inheritable properties that are not
//! explicitly set by any declaration are inherited from the parent node.

use alloc::vec::Vec;

use crate::css::{CssValue, Declaration, Property, Selector, SimpleSelector, Stylesheet, Unit};
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
    None,
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
    pub list_style: ListStyle,
    pub white_space: WhiteSpace,
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

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

/// Reasonable defaults: dark-theme light text, transparent background.
pub fn default_style() -> ComputedStyle {
    ComputedStyle {
        display: Display::Block,
        color: 0xFFE6E6E6,
        background_color: 0,
        font_size: 16,
        font_weight: FontWeight::Normal,
        font_style: FontStyleVal::Normal,
        text_align: TextAlignVal::Left,
        text_decoration: TextDeco::None,
        line_height: 0,
        margin_top: 0,  margin_right: 0,  margin_bottom: 0,  margin_left: 0,
        padding_top: 0, padding_right: 0, padding_bottom: 0, padding_left: 0,
        border_width: 0,
        border_color: 0xFF808080,
        border_radius: 0,
        width: Option::None,
        height: Option::None,
        max_width: Option::None,
        list_style: ListStyle::None,
        white_space: WhiteSpace::Normal,
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
        Tag::Head | Tag::Title | Tag::Meta | Tag::Link | Tag::Style | Tag::Script => {
            s.display = Display::None;
        }
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
    match selector {
        Selector::Universal => {
            // Universal matches any element (not text nodes).
            matches!(dom.nodes[node_id].node_type, NodeType::Element { .. })
        }
        Selector::Simple(simple) => simple_matches(simple, dom, node_id),
        Selector::Descendant(ancestor_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id) {
                return false;
            }
            // Walk up ancestors looking for a match of the outer selector.
            let mut cur = dom.nodes[node_id].parent;
            while let Some(pid) = cur {
                if selector_matches(ancestor_sel, dom, pid) {
                    return true;
                }
                cur = dom.nodes[pid].parent;
            }
            false
        }
    }
}

fn simple_matches(sel: &SimpleSelector, dom: &Dom, node_id: NodeId) -> bool {
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

    true
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
pub fn resolve_styles(dom: &Dom, stylesheets: &[Stylesheet]) -> Vec<ComputedStyle> {
    let count = dom.nodes.len();
    let mut styles: Vec<ComputedStyle> = Vec::with_capacity(count);

    let root_font_size: i32 = 16;

    for id in 0..count {
        let node = &dom.nodes[id];
        let parent_fs = node.parent.map_or(16, |pid| styles[pid].font_size);

        // Phase 1: Start from UA defaults (elements) or initial values (text).
        let (mut style, mut set_flags) = match &node.node_type {
            NodeType::Element { tag, .. } => ua_style_and_flags(*tag),
            NodeType::Text(_) => (default_style(), 0u16),
        };

        // Phase 2: Apply author stylesheet rules sorted by specificity.
        if matches!(node.node_type, NodeType::Element { .. }) {
            set_flags |= apply_author_rules(
                &mut style, dom, id, stylesheets, parent_fs, root_font_size,
            );

            // Phase 3: Apply inline styles (highest specificity).
            let inline_decls = get_inline_decls(dom, id);
            for decl in &inline_decls {
                set_flags |= decl_set_flag(decl.property);
                apply_declaration(&mut style, decl, parent_fs, root_font_size);
            }
        }

        // Phase 4: Inherit inheritable properties NOT explicitly set.
        if let Some(pid) = node.parent {
            inherit_unset(&mut style, &styles[pid], set_flags);
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
            // Approximate 1.2x: (font_size * 6 + 2) / 5
            style.line_height = (style.font_size * 6 + 2) / 5;
        }

        styles.push(style);
    }

    styles
}

/// Parse inline `style="..."` attribute into declarations.
fn get_inline_decls(dom: &Dom, node_id: NodeId) -> Vec<Declaration> {
    if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
        for a in attrs {
            if eq_ignore_ascii_case(&a.name, "style") {
                return crate::css::parse_inline_style(&a.value);
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Author rule application
// ---------------------------------------------------------------------------

/// Matched rule entry: (specificity, sheet_index, rule_index).
struct MatchEntry {
    spec: (u32, u32, u32),
    sheet_idx: usize,
    rule_idx: usize,
}

fn apply_author_rules(
    style: &mut ComputedStyle,
    dom: &Dom,
    node_id: NodeId,
    stylesheets: &[Stylesheet],
    parent_fs: i32,
    root_fs: i32,
) -> u16 {
    let mut matches: Vec<MatchEntry> = Vec::new();

    for (si, sheet) in stylesheets.iter().enumerate() {
        for (ri, rule) in sheet.rules.iter().enumerate() {
            for sel in &rule.selectors {
                if selector_matches(sel, dom, node_id) {
                    matches.push(MatchEntry {
                        spec: sel.specificity(),
                        sheet_idx: si,
                        rule_idx: ri,
                    });
                    break;
                }
            }
        }
    }

    // Sort by specificity (ascending); equal specificity keeps source order.
    matches.sort_by(|a, b| a.spec.cmp(&b.spec));

    let mut set_flags: u16 = 0;

    for m in &matches {
        let rule = &stylesheets[m.sheet_idx].rules[m.rule_idx];
        for decl in &rule.declarations {
            set_flags |= decl_set_flag(decl.property);
            apply_declaration(style, decl, parent_fs, root_fs);
        }
    }

    set_flags
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
}

/// Map a CSS property to the inheritable-set bitflag (0 if not inheritable).
fn decl_set_flag(prop: Property) -> u16 {
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
        CssValue::Number(v) => Some(v / 100),
        CssValue::Percentage(_) => Option::None,
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
            if let CssValue::Color(c) = decl.value { style.background_color = c; }
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
                CssValue::Auto => style.width = Option::None,
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.width = Some(px);
                    }
                }
            }
        }
        Property::Height => {
            match decl.value {
                CssValue::Auto => style.height = Option::None,
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.height = Some(px);
                    }
                }
            }
        }
        Property::MaxWidth => {
            match decl.value {
                CssValue::None => style.max_width = Option::None,
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.max_width = Some(px);
                    }
                }
            }
        }
        // Shorthand margin: apply a single value to all four sides.
        Property::Margin => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px; style.margin_right = px;
                style.margin_bottom = px; style.margin_left = px;
            }
        }
        Property::MarginTop => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
            }
        }
        Property::MarginRight => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_right = px;
            }
        }
        Property::MarginBottom => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_bottom = px;
            }
        }
        Property::MarginLeft => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
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
        // Properties we parse but do not use in ComputedStyle yet.
        Property::TextIndent | Property::VerticalAlign | Property::MinWidth
        | Property::MinHeight | Property::MaxHeight | Property::BorderStyle
        | Property::Overflow => {}
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

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
