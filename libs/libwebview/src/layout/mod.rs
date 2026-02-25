//! Layout engine for the Surf web browser.
//!
//! Takes a DOM tree (`Dom`) and per-node computed styles (`ComputedStyle`)
//! and produces a tree of `LayoutBox`es with absolute positions and sizes.
//!
//! Sub-modules:
//!   - `block`: Block-level layout (`build_block`)
//!   - `flex`: Flexbox layout (`layout_flex`)
//!   - `inline`: Inline/text layout, form element fragments
//!   - `form`: Form field position collection

pub mod block;
pub mod flex;
pub mod inline;
pub mod form;
pub mod table;

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, Tag};
use crate::style::{
    ComputedStyle, Display, FontWeight, FontStyleVal, TextAlignVal,
    ListStyle, TextDeco, TextTransform, FloatVal, Position, ClearVal,
};
use crate::ImageCache;

// Re-export sub-module public items.
pub use form::{FormFieldPos, collect_form_positions};
use block::build_block;
use inline::layout_inline_content;

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

pub struct LayoutBox {
    pub node_id: Option<NodeId>,
    pub box_type: BoxType,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub margin: Edges,
    pub padding: Edges,
    pub border_width: i32,
    pub children: Vec<LayoutBox>,
    /// Text content for text runs.
    pub text: Option<String>,
    pub font_size: i32,
    pub bold: bool,
    pub italic: bool,
    pub color: u32,
    pub bg_color: u32,
    pub border_color: u32,
    pub border_radius: i32,
    pub text_decoration: TextDeco,
    pub text_align: TextAlignVal,
    pub link_url: Option<String>,
    pub list_marker: Option<String>,
    pub is_hr: bool,
    /// Image source URL for `<img>` elements.
    pub image_src: Option<String>,
    pub image_width: Option<i32>,
    pub image_height: Option<i32>,
    /// Form field kind (for `<input>`, `<button>`, `<textarea>`, `<select>`).
    pub form_field: Option<FormFieldKind>,
    /// Placeholder text for form text inputs.
    pub form_placeholder: Option<String>,
    /// Default value for form text inputs.
    pub form_value: Option<String>,
    /// If true, children that extend outside this box should be clipped.
    pub overflow_hidden: bool,
    /// If true, this box is invisible but still takes up space.
    pub visibility_hidden: bool,
    /// Opacity: 0..255 (255 = fully opaque).
    pub opacity: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Anonymous,
    LineBox,
}

/// Kind of HTML form field for interactive rendering.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormFieldKind {
    TextInput,
    Password,
    Submit,
    Checkbox,
    Radio,
    Hidden,
    ButtonEl,
    Textarea,
}

#[derive(Clone, Copy, Default)]
pub struct Edges {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

// ---------------------------------------------------------------------------
// Layout box constructors (pub(super) for sub-modules)
// ---------------------------------------------------------------------------

impl LayoutBox {
    pub(super) fn new(node_id: Option<NodeId>, box_type: BoxType) -> Self {
        LayoutBox {
            node_id,
            box_type,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            margin: Edges::default(),
            padding: Edges::default(),
            border_width: 0,
            children: Vec::new(),
            text: None,
            font_size: 16,
            bold: false,
            italic: false,
            color: 0xFF000000,
            bg_color: 0,
            border_color: 0,
            border_radius: 0,
            text_decoration: TextDeco::None,
            text_align: TextAlignVal::Left,
            link_url: None,
            list_marker: None,
            is_hr: false,
            image_src: None,
            image_width: None,
            image_height: None,
            form_field: None,
            form_placeholder: None,
            form_value: None,
            overflow_hidden: false,
            visibility_hidden: false,
            opacity: 255,
        }
    }

    pub(super) fn new_text(text: String, font_size: i32, bold: bool, italic: bool, color: u32) -> Self {
        let mut b = LayoutBox::new(None, BoxType::Inline);
        b.text = Some(text);
        b.font_size = font_size;
        b.bold = bold;
        b.italic = italic;
        b.color = color;
        b
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (pub(super) for sub-modules)
// ---------------------------------------------------------------------------

pub(super) fn measure_text(text: &str, font_size: i32, bold: bool) -> (i32, i32) {
    let font_id: u16 = if bold { 1 } else { 0 };
    let (w, h) = libanyui_client::measure_text(text, font_id, font_size as u16);
    (w as i32, h as i32)
}

pub(super) fn font_size_px(style: &ComputedStyle) -> i32 {
    let s = style.font_size;
    if s <= 0 { 16 } else { s }
}

pub(super) fn is_bold(style: &ComputedStyle) -> bool {
    matches!(style.font_weight, FontWeight::Bold)
}

pub(super) fn is_italic(style: &ComputedStyle) -> bool {
    matches!(style.font_style, FontStyleVal::Italic)
}

pub(super) fn edges_from(top: i32, right: i32, bottom: i32, left: i32) -> Edges {
    Edges { top, right, bottom, left }
}

pub(super) fn link_href(dom: &Dom, node_id: NodeId) -> Option<String> {
    if dom.tag(node_id) == Some(Tag::A) {
        dom.attr(node_id, "href").map(|s| String::from(s))
    } else {
        None
    }
}

pub(super) fn inherited_link(dom: &Dom, node_id: NodeId) -> Option<String> {
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if let Some(href) = link_href(dom, id) {
            return Some(href);
        }
        cur = dom.get(id).parent;
    }
    None
}

pub(super) fn list_marker_for(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> Option<String> {
    if dom.tag(node_id) != Some(Tag::Li) {
        return None;
    }
    match style.list_style {
        ListStyle::Disc => Some(String::from("\u{2022} ")),
        ListStyle::Circle => Some(String::from("\u{25E6} ")),
        ListStyle::Square => Some(String::from("\u{25AA} ")),
        ListStyle::Decimal => {
            let parent = dom.get(node_id).parent?;
            let siblings = &dom.get(parent).children;
            let mut idx = 1;
            for &sib in siblings {
                if sib == node_id {
                    break;
                }
                if dom.tag(sib) == Some(Tag::Li) {
                    idx += 1;
                }
            }
            let mut s = String::new();
            format_decimal(&mut s, idx);
            s.push_str(". ");
            Some(s)
        }
        ListStyle::None => None,
    }
}

fn format_decimal(out: &mut String, mut n: u32) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(buf[i] as char);
    }
}

pub(super) fn image_dimensions(dom: &Dom, node_id: NodeId, max_width: i32, images: &ImageCache) -> (i32, i32) {
    // Get natural dimensions from image cache (actual decoded image size).
    let src = dom.attr(node_id, "src");
    let natural = src.and_then(|s| images.get(s)).map(|e| (e.width as i32, e.height as i32));

    // HTML attributes override natural size; fall back to natural; then 300x150.
    let w = dom.attr(node_id, "width").and_then(parse_attr_int)
        .or(natural.map(|(w, _)| w))
        .unwrap_or(300);
    let h = dom.attr(node_id, "height").and_then(parse_attr_int)
        .or(natural.map(|(_, h)| h))
        .unwrap_or(150);

    // Scale down proportionally if wider than container.
    if w > max_width && max_width > 0 && w > 0 {
        let scaled_h = (h as i64 * max_width as i64 / w as i64) as i32;
        (max_width, scaled_h.max(1))
    } else {
        (w, h)
    }
}

pub(super) fn parse_attr_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut val: i32 = 0;
    for &b in bytes {
        if b.is_ascii_digit() {
            val = val * 10 + (b - b'0') as i32;
        } else {
            break;
        }
    }
    if val > 0 { Some(val) } else { None }
}

pub(super) fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

pub(super) fn ascii_lower_str<'a>(s: &str, buf: &'a mut [u8; 16]) -> &'a str {
    let len = s.len().min(16);
    for i in 0..len {
        let b = s.as_bytes()[i];
        buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

pub(super) fn size_attr_width(dom: &Dom, node_id: NodeId, default: i32) -> i32 {
    if let Some(size) = dom.attr(node_id, "size") {
        if let Some(s) = parse_attr_int(size) {
            return (s * 8).max(40).min(600);
        }
    }
    default
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Build a layout tree from the DOM and computed styles.
pub fn layout(dom: &Dom, styles: &[ComputedStyle], viewport_width: i32, images: &ImageCache) -> LayoutBox {
    crate::debug_surf!("[layout] layout start: {} nodes, viewport_width={}", dom.nodes.len(), viewport_width);
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!("[layout]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());

    let body_id = dom.find_body().unwrap_or(0);
    let style = &styles[body_id];

    let mut root = LayoutBox::new(Some(body_id), BoxType::Block);
    root.width = viewport_width;
    root.bg_color = style.background_color;
    root.color = style.color;
    root.padding = edges_from(
        style.padding_top, style.padding_right,
        style.padding_bottom, style.padding_left,
    );
    root.margin = edges_from(
        style.margin_top, style.margin_right,
        style.margin_bottom, style.margin_left,
    );

    let content_width = viewport_width - root.padding.left - root.padding.right
        - root.margin.left - root.margin.right;

    let children = &dom.get(body_id).children;
    let child_ids: Vec<NodeId> = children.iter().copied().collect();
    crate::debug_surf!("[layout] body has {} direct children, content_width={}", child_ids.len(), content_width);
    let height = layout_children(dom, styles, &child_ids, content_width, &mut root, body_id, images);

    root.height = height + root.padding.top + root.padding.bottom;
    crate::debug_surf!("[layout] layout done: root height={}", root.height);
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!("[layout]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());
    root
}

// ---------------------------------------------------------------------------
// Block flow orchestration
// ---------------------------------------------------------------------------

/// Layout a list of child node IDs within the given available width.
/// Appends resulting `LayoutBox`es to `parent.children` and returns the total
/// height consumed.
pub(super) fn layout_children(
    dom: &Dom,
    styles: &[ComputedStyle],
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    _parent_node: NodeId,
    images: &ImageCache,
) -> i32 {
    let mut cursor_y: i32 = parent.padding.top;
    let mut prev_margin_bottom: i32 = 0;
    let mut float_ctx = FloatContext::new(available_width);

    // Collect absolutely/fixed-positioned children to lay out after normal flow.
    let mut deferred_abs: Vec<NodeId> = Vec::new();

    let mut i = 0;
    while i < child_ids.len() {
        let cid = child_ids[i];
        let style = &styles[cid];

        if style.display == Display::None {
            i += 1;
            continue;
        }

        // Skip absolute/fixed from normal flow — position them after.
        if matches!(style.position, Position::Absolute | Position::Fixed) {
            deferred_abs.push(cid);
            i += 1;
            continue;
        }

        // Handle `clear` property — advance cursor past cleared floats.
        if style.clear != ClearVal::None {
            let clear_to = float_ctx.clear_y(style.clear);
            if clear_to > cursor_y {
                cursor_y = clear_to;
            }
        }

        let is_block = is_block_level(dom, cid, style);

        if is_block {
            let float_val = style.float;

            // ── Floated elements ──
            if float_val != FloatVal::None {
                let stf_width = shrink_to_fit_width(dom, styles, cid, available_width, images);
                let mut placed = if is_table_element(dom, cid) {
                    table::layout_table(dom, styles, cid, stf_width, images)
                } else {
                    build_block(dom, styles, cid, stf_width, images)
                };

                let total_w = placed.width + placed.margin.left + placed.margin.right;
                let total_h = placed.height + placed.margin.top + placed.margin.bottom;
                let place_y = float_ctx.find_y_for_float(float_val, total_w, total_h, cursor_y);

                let li = float_ctx.left_intrusion_at(place_y, total_h);
                let ri = float_ctx.right_intrusion_at(place_y, total_h);

                if float_val == FloatVal::Left {
                    placed.x = parent.padding.left + li + placed.margin.left;
                } else {
                    let right_edge = available_width - ri;
                    placed.x = parent.padding.left + right_edge - placed.width - placed.margin.right;
                }
                placed.y = place_y + placed.margin.top;

                float_ctx.add(PlacedFloat {
                    side: float_val,
                    x: placed.x - placed.margin.left,
                    y: place_y,
                    width: total_w,
                    height: total_h,
                });

                parent.children.push(placed);
                i += 1;
                continue;
            }

            // ── Normal block flow ──
            let li = float_ctx.left_intrusion_at(cursor_y, 1);
            let ri = float_ctx.right_intrusion_at(cursor_y, 1);
            let effective_avail = (available_width - li - ri).max(0);

            let child_box = if is_table_element(dom, cid) {
                table::layout_table(dom, styles, cid, effective_avail, images)
            } else {
                build_block(dom, styles, cid, effective_avail, images)
            };

            let collapsed = if prev_margin_bottom > child_box.margin.top {
                prev_margin_bottom
            } else {
                child_box.margin.top
            };
            if cursor_y == parent.padding.top {
                cursor_y += child_box.margin.top;
            } else {
                cursor_y += collapsed - prev_margin_bottom;
            }

            let mut placed = child_box;
            placed.x = parent.padding.left + placed.margin.left + li;

            // Center/right block alignment.
            let parent_align = styles[_parent_node].text_align;
            if parent_align == TextAlignVal::Center {
                let total_child_w = placed.width + placed.margin.left + placed.margin.right;
                if total_child_w < effective_avail {
                    placed.x = parent.padding.left + li + (effective_avail - total_child_w) / 2;
                }
            } else if parent_align == TextAlignVal::Right {
                let total_child_w = placed.width + placed.margin.left + placed.margin.right;
                if total_child_w < effective_avail {
                    placed.x = parent.padding.left + li + effective_avail - total_child_w;
                }
            }

            placed.y = cursor_y;
            cursor_y += placed.height + placed.margin.bottom;
            prev_margin_bottom = placed.margin.bottom;

            parent.children.push(placed);
            i += 1;
        } else {
            // ── Inline run ──
            let run_start = i;
            while i < child_ids.len() {
                let sid = child_ids[i];
                let ss = &styles[sid];
                if ss.display == Display::None {
                    i += 1;
                    continue;
                }
                if is_block_level(dom, sid, ss) {
                    break;
                }
                i += 1;
            }
            let inline_ids: Vec<NodeId> = child_ids[run_start..i].iter().copied().collect();

            // Query float intrusions for inline content.
            let li = float_ctx.left_intrusion_at(cursor_y, 1);
            let ri = float_ctx.right_intrusion_at(cursor_y, 1);
            let inline_avail = (available_width - li - ri).max(0);

            let parent_style = &styles[_parent_node];
            let parent_align = parent_style.text_align;
            let line_boxes = layout_inline_content(
                dom, styles, &inline_ids, inline_avail, parent.padding.left + li, images,
                parent_align, parent_style.line_height,
            );
            for lb in line_boxes {
                let h = lb.height;
                let mut placed = lb;
                placed.y = cursor_y;
                cursor_y += h;
                parent.children.push(placed);
            }
            prev_margin_bottom = 0;
        }
    }

    // Position absolutely/fixed elements out of flow.
    for &abs_id in &deferred_abs {
        let abs_style = &styles[abs_id];
        let mut abs_box = if is_table_element(dom, abs_id) {
            table::layout_table(dom, styles, abs_id, available_width, images)
        } else {
            build_block(dom, styles, abs_id, available_width, images)
        };

        // Position using top/left/right/bottom offsets relative to parent.
        let t = abs_style.top.unwrap_or(0);
        let l = abs_style.left_offset.unwrap_or(0);

        abs_box.x = parent.padding.left + l + abs_box.margin.left;
        abs_box.y = parent.padding.top + t + abs_box.margin.top;

        // right/bottom override if left/top not set.
        if abs_style.left_offset.is_none() {
            if let Some(r) = abs_style.right_offset {
                abs_box.x = available_width - r - abs_box.width - abs_box.margin.right;
            }
        }
        if abs_style.top.is_none() {
            if let Some(b) = abs_style.bottom_offset {
                // Position from bottom of current content area.
                abs_box.y = cursor_y - b - abs_box.height - abs_box.margin.bottom;
            }
        }

        parent.children.push(abs_box);
    }

    cursor_y
}

/// Apply text-transform to a string.
pub(super) fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => String::from(text),
        TextTransform::Uppercase => {
            let mut out = String::with_capacity(text.len());
            for ch in text.chars() {
                for c in ch.to_uppercase() { out.push(c); }
            }
            out
        }
        TextTransform::Lowercase => {
            let mut out = String::with_capacity(text.len());
            for ch in text.chars() {
                for c in ch.to_lowercase() { out.push(c); }
            }
            out
        }
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut prev_ws = true;
            for ch in text.chars() {
                if prev_ws && ch.is_alphabetic() {
                    for c in ch.to_uppercase() { out.push(c); }
                } else {
                    out.push(ch);
                }
                prev_ws = ch.is_whitespace();
            }
            out
        }
    }
}

/// Determine whether a node should generate a block-level box.
fn is_block_level(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> bool {
    if matches!(style.display, Display::Block | Display::Flex | Display::ListItem) {
        return true;
    }
    if let Some(tag) = dom.tag(node_id) {
        if tag == Tag::Hr || tag == Tag::Table {
            return true;
        }
        if tag.is_block() && style.display != Display::Inline && style.display != Display::InlineFlex && style.display != Display::InlineBlock {
            return true;
        }
    }
    false
}

fn is_table_element(dom: &Dom, node_id: NodeId) -> bool {
    matches!(dom.tag(node_id), Some(Tag::Table))
}

// ---------------------------------------------------------------------------
// Float context — tracks placed floats for correct flow-around behaviour.
// ---------------------------------------------------------------------------

struct PlacedFloat {
    side: FloatVal,  // Left or Right
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct FloatContext {
    floats: Vec<PlacedFloat>,
    container_width: i32,
}

impl FloatContext {
    fn new(container_width: i32) -> Self {
        FloatContext { floats: Vec::new(), container_width }
    }

    /// Total width consumed by left floats overlapping the given Y band.
    fn left_intrusion_at(&self, y: i32, h: i32) -> i32 {
        let mut max_right = 0i32;
        for f in &self.floats {
            if f.side == FloatVal::Left && f.y < y + h && f.y + f.height > y {
                let right = f.x + f.width;
                if right > max_right { max_right = right; }
            }
        }
        max_right
    }

    /// Total width consumed by right floats overlapping the given Y band.
    fn right_intrusion_at(&self, y: i32, h: i32) -> i32 {
        let mut max_left = self.container_width;
        for f in &self.floats {
            if f.side == FloatVal::Right && f.y < y + h && f.y + f.height > y {
                if f.x < max_left { max_left = f.x; }
            }
        }
        self.container_width - max_left
    }

    /// Available horizontal space at a given Y band.
    fn available_width_at(&self, y: i32, h: i32) -> i32 {
        let li = self.left_intrusion_at(y, h);
        let ri = self.right_intrusion_at(y, h);
        (self.container_width - li - ri).max(0)
    }

    /// Y position past which all floats matching `clear` are cleared.
    fn clear_y(&self, clear: ClearVal) -> i32 {
        let mut max_bottom = 0i32;
        for f in &self.floats {
            let dominated = match clear {
                ClearVal::Left => f.side == FloatVal::Left,
                ClearVal::Right => f.side == FloatVal::Right,
                ClearVal::Both => true,
                ClearVal::None => false,
            };
            if dominated {
                let bot = f.y + f.height;
                if bot > max_bottom { max_bottom = bot; }
            }
        }
        max_bottom
    }

    /// Find the Y position where a float of `width` can be placed.
    /// Scans downward from `start_y` in 1-px increments until there's room.
    fn find_y_for_float(&self, _side: FloatVal, width: i32, height: i32, start_y: i32) -> i32 {
        let mut y = start_y;
        loop {
            let li = self.left_intrusion_at(y, height);
            let ri = self.right_intrusion_at(y, height);
            let avail = self.container_width - li - ri;
            if avail >= width {
                return y;
            }
            y += 1;
            if y > start_y + 10000 { return y; } // safety cap
        }
    }

    fn add(&mut self, pf: PlacedFloat) {
        self.floats.push(pf);
    }
}

/// Compute shrink-to-fit width for a float element.
/// Uses the child's laid-out width if no explicit width was set.
fn shrink_to_fit_width(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    max_width: i32,
    images: &ImageCache,
) -> i32 {
    let style = &styles[node_id];
    // If explicit width is set, use it.
    if let Some(w) = style.width {
        if w > 0 { return w.min(max_width); }
    }
    // Otherwise, lay out with max_width and use the resulting content width.
    let trial = build_block(dom, styles, node_id, max_width, images);
    // Shrink-to-fit: use the content width (sum of children) capped at max_width.
    let content_w = trial.children.iter()
        .map(|c| c.x + c.width + c.margin.right)
        .max()
        .unwrap_or(0);
    let fit_w = content_w + trial.padding.left + trial.padding.right
        + trial.border_width * 2;
    fit_w.max(1).min(max_width)
}
