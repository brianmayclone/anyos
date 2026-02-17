//! Layout engine for the Surf web browser.
//!
//! Takes a DOM tree (`Dom`) and per-node computed styles (`ComputedStyle`)
//! and produces a tree of `LayoutBox`es with absolute positions and sizes.
//! Text is line-broken with word wrapping using the uisys font measurement
//! API.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{ComputedStyle, Display, FontWeight, FontStyleVal, TextAlignVal, WhiteSpace, ListStyle, TextDeco};

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Anonymous,
    LineBox,
}

#[derive(Clone, Copy, Default)]
pub struct Edges {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

// ---------------------------------------------------------------------------
// Font measurement helper
// ---------------------------------------------------------------------------

fn measure_text(text: &str, font_size: i32, bold: bool) -> (i32, i32) {
    let font_id: u16 = if bold { 1 } else { 0 };
    let (w, h) = uisys_client::font_measure(font_id, font_size as u16, text);
    (w as i32, h as i32)
}

// ---------------------------------------------------------------------------
// Layout box constructors
// ---------------------------------------------------------------------------

impl LayoutBox {
    fn new(node_id: Option<NodeId>, box_type: BoxType) -> Self {
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
        }
    }

    fn new_text(text: String, font_size: i32, bold: bool, italic: bool, color: u32) -> Self {
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
// Style extraction helpers
// ---------------------------------------------------------------------------

/// Extract the resolved font size from computed style in pixels.
fn font_size_px(style: &ComputedStyle) -> i32 {
    let s = style.font_size;
    if s <= 0 { 16 } else { s }
}

/// Check if a node is bold.
fn is_bold(style: &ComputedStyle) -> bool {
    matches!(style.font_weight, FontWeight::Bold)
}

/// Check if a node is italic.
fn is_italic(style: &ComputedStyle) -> bool {
    matches!(style.font_style, FontStyleVal::Italic)
}

/// Resolve edges (margin or padding) from the computed style.
fn edges_from(top: i32, right: i32, bottom: i32, left: i32) -> Edges {
    Edges { top, right, bottom, left }
}

/// Get the link URL if this node is an `<a>` with an href attribute.
fn link_href(dom: &Dom, node_id: NodeId) -> Option<String> {
    if dom.tag(node_id) == Some(Tag::A) {
        dom.attr(node_id, "href").map(|s| String::from(s))
    } else {
        None
    }
}

/// Walk up the DOM to find the closest ancestor `<a>` href.
fn inherited_link(dom: &Dom, node_id: NodeId) -> Option<String> {
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if let Some(href) = link_href(dom, id) {
            return Some(href);
        }
        cur = dom.get(id).parent;
    }
    None
}

/// Determine the list marker string for an `<li>` element.
fn list_marker_for(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> Option<String> {
    if dom.tag(node_id) != Some(Tag::Li) {
        return None;
    }
    match style.list_style {
        ListStyle::Disc => Some(String::from("\u{2022} ")),
        ListStyle::Circle => Some(String::from("\u{25E6} ")),
        ListStyle::Square => Some(String::from("\u{25AA} ")),
        ListStyle::Decimal => {
            // Count preceding <li> siblings to determine the number.
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

/// Simple decimal formatting without core::fmt (no_std friendly).
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

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Build a layout tree from the DOM and computed styles.
///
/// `styles` is indexed by `NodeId` and must have the same length as
/// `dom.nodes`.
pub fn layout(dom: &Dom, styles: &[ComputedStyle], viewport_width: i32) -> LayoutBox {
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
    let height = layout_children(dom, styles, &child_ids, content_width, &mut root, body_id);

    root.height = height + root.padding.top + root.padding.bottom;
    root
}

// ---------------------------------------------------------------------------
// Block layout
// ---------------------------------------------------------------------------

/// Layout a list of child node IDs within the given available width.
/// Appends resulting `LayoutBox`es to `parent.children` and returns the total
/// height consumed.
fn layout_children(
    dom: &Dom,
    styles: &[ComputedStyle],
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    _parent_node: NodeId,
) -> i32 {
    let mut cursor_y: i32 = parent.padding.top;
    let mut prev_margin_bottom: i32 = 0;

    // Separate children into block vs inline runs.
    let mut i = 0;
    while i < child_ids.len() {
        let cid = child_ids[i];
        let style = &styles[cid];

        // Skip display:none
        if style.display == Display::None {
            i += 1;
            continue;
        }

        let is_block = is_block_level(dom, cid, style);

        if is_block {
            // Lay out as a block box.
            let child_box = build_block(dom, styles, cid, available_width);

            // Margin collapsing: use max of previous bottom and this top.
            let collapsed = if prev_margin_bottom > child_box.margin.top {
                prev_margin_bottom
            } else {
                child_box.margin.top
            };
            if cursor_y == parent.padding.top {
                // First child -- apply top margin directly.
                cursor_y += child_box.margin.top;
            } else {
                cursor_y += collapsed - prev_margin_bottom;
            }

            let mut placed = child_box;
            placed.x = parent.padding.left + placed.margin.left;
            placed.y = cursor_y;
            cursor_y += placed.height + placed.margin.bottom;
            prev_margin_bottom = placed.margin.bottom;

            parent.children.push(placed);
            i += 1;
        } else {
            // Collect a run of inline children.
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
            let line_boxes = layout_inline_content(
                dom, styles, &inline_ids, available_width, parent.padding.left,
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

    cursor_y
}

/// Determine whether a node should generate a block-level box.
fn is_block_level(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> bool {
    if style.display == Display::Block {
        return true;
    }
    // Treat <br> and <hr> as block-level for layout purposes.
    if let Some(tag) = dom.tag(node_id) {
        if tag == Tag::Hr {
            return true;
        }
        if tag.is_block() && style.display != Display::Inline {
            return true;
        }
    }
    false
}

/// Build a block-level layout box for a single DOM node.
fn build_block(dom: &Dom, styles: &[ComputedStyle], node_id: NodeId, available_width: i32) -> LayoutBox {
    let style = &styles[node_id];
    let tag = dom.tag(node_id);

    let mut bx = LayoutBox::new(Some(node_id), BoxType::Block);
    bx.color = style.color;
    bx.bg_color = style.background_color;
    bx.border_width = style.border_width;
    bx.border_color = style.border_color;
    bx.border_radius = style.border_radius;
    bx.font_size = font_size_px(style);
    bx.bold = is_bold(style);
    bx.italic = is_italic(style);
    bx.text_decoration = style.text_decoration;
    bx.text_align = style.text_align;
    bx.link_url = link_href(dom, node_id);
    bx.list_marker = list_marker_for(dom, node_id, style);
    bx.margin = edges_from(
        style.margin_top, style.margin_right,
        style.margin_bottom, style.margin_left,
    );
    bx.padding = edges_from(
        style.padding_top, style.padding_right,
        style.padding_bottom, style.padding_left,
    );

    // Determine own width.
    let border2 = bx.border_width * 2;
    let outer_margin = bx.margin.left + bx.margin.right;
    let max_content_w = available_width - outer_margin - border2;

    if let Some(w) = style.width {
        if w > 0 {
            bx.width = w.min(max_content_w);
        } else {
            bx.width = max_content_w;
        }
    } else {
        bx.width = max_content_w;
    }

    // Handle <hr> specifically.
    if tag == Some(Tag::Hr) {
        bx.is_hr = true;
        bx.height = 1 + bx.padding.top + bx.padding.bottom + border2;
        if bx.margin.top == 0 && bx.margin.bottom == 0 {
            bx.margin.top = 8;
            bx.margin.bottom = 8;
        }
        return bx;
    }

    // Handle <img> as block/inline-block replaced element.
    if tag == Some(Tag::Img) {
        let (iw, ih) = image_dimensions(dom, node_id, bx.width);
        bx.image_src = dom.attr(node_id, "src").map(|s| String::from(s));
        bx.image_width = Some(iw);
        bx.image_height = Some(ih);
        bx.height = ih + bx.padding.top + bx.padding.bottom + border2;
        bx.width = iw + bx.padding.left + bx.padding.right + border2;
        return bx;
    }

    // Lay out children inside this block.
    let inner_w = bx.width - bx.padding.left - bx.padding.right;
    let children: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    let content_h = layout_children(dom, styles, &children, inner_w, &mut bx, node_id);

    // Set height.
    if let Some(h) = style.height {
        bx.height = h + border2;
    } else {
        bx.height = content_h + bx.padding.bottom + border2;
    }

    bx
}

// ---------------------------------------------------------------------------
// Inline layout / line breaking
// ---------------------------------------------------------------------------

/// Represents a single inline fragment before line-breaking.
struct InlineFragment {
    /// Width in pixels.
    width: i32,
    /// Height in pixels.
    height: i32,
    /// The layout box for this fragment (text run, image, etc.).
    layout_box: LayoutBox,
    /// True if this fragment forces a line break after it.
    breaks_after: bool,
}

/// Lay out a run of inline child nodes, performing word wrapping.
/// Returns a list of line boxes positioned at x = `start_x`.
fn layout_inline_content(
    dom: &Dom,
    styles: &[ComputedStyle],
    child_ids: &[NodeId],
    available_width: i32,
    start_x: i32,
) -> Vec<LayoutBox> {
    // 1. Flatten all inline children into fragments.
    let mut fragments: Vec<InlineFragment> = Vec::new();
    for &cid in child_ids {
        let style = &styles[cid];
        if style.display == Display::None {
            continue;
        }
        collect_inline_fragments(dom, styles, cid, &mut fragments);
    }

    // 2. Break fragments into lines.
    let mut lines: Vec<LayoutBox> = Vec::new();
    let mut line = LayoutBox::new(None, BoxType::LineBox);
    line.x = start_x;
    line.width = available_width;
    let mut line_x: i32 = 0;
    let mut line_h: i32 = 0;

    for frag in fragments {
        let fw = frag.width;
        let fh = frag.height;

        // Check if we need to wrap.
        if line_x > 0 && line_x + fw > available_width && !line.children.is_empty() {
            // Finish current line.
            line.height = line_h;
            lines.push(line);
            line = LayoutBox::new(None, BoxType::LineBox);
            line.x = start_x;
            line.width = available_width;
            line_x = 0;
            line_h = 0;
        }

        let mut child = frag.layout_box;
        child.x = start_x + line_x;
        child.y = 0; // will be set relative to line box later
        child.width = fw;
        child.height = fh;

        line_x += fw;
        if fh > line_h {
            line_h = fh;
        }

        line.children.push(child);

        if frag.breaks_after {
            line.height = if line_h > 0 { line_h } else { 16 }; // default line height for empty lines
            lines.push(line);
            line = LayoutBox::new(None, BoxType::LineBox);
            line.x = start_x;
            line.width = available_width;
            line_x = 0;
            line_h = 0;
        }
    }

    // Flush last line.
    if !line.children.is_empty() {
        line.height = line_h;
        lines.push(line);
    }

    // 3. Vertically stack lines, baseline-align children inside each.
    let mut _total_y = 0;
    for ln in &mut lines {
        let lh = ln.height;
        for child in &mut ln.children {
            // Align to bottom of line box (simplified baseline alignment).
            child.y = lh - child.height;
        }
    }

    lines
}

/// Recursively collect inline fragments from a node and its inline children.
fn collect_inline_fragments(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    out: &mut Vec<InlineFragment>,
) {
    let node = dom.get(node_id);
    let style = &styles[node_id];

    match &node.node_type {
        NodeType::Text(text) => {
            let fs = font_size_px(style);
            let bold = is_bold(style);
            let italic = is_italic(style);
            let color = style.color;
            let link = inherited_link(dom, node_id);
            let deco = style.text_decoration;

            if style.white_space == WhiteSpace::Pre {
                // Preserve whitespace: break on newlines.
                emit_preformatted_fragments(text, fs, bold, italic, color, link, deco, out);
            } else {
                // Normal flow: word-wrap.
                emit_word_fragments(text, fs, bold, italic, color, link, deco, out);
            }
        }
        NodeType::Element { tag, .. } => {
            // Handle <br>
            if *tag == Tag::Br {
                let mut brk = LayoutBox::new(Some(node_id), BoxType::Inline);
                brk.font_size = font_size_px(style);
                out.push(InlineFragment {
                    width: 0,
                    height: 0,
                    layout_box: brk,
                    breaks_after: true,
                });
                return;
            }

            // Handle inline <img>
            if *tag == Tag::Img {
                let (iw, ih) = image_dimensions(dom, node_id, 300);
                let mut img = LayoutBox::new(Some(node_id), BoxType::Inline);
                img.image_src = dom.attr(node_id, "src").map(|s| String::from(s));
                img.image_width = Some(iw);
                img.image_height = Some(ih);
                img.width = iw;
                img.height = ih;
                out.push(InlineFragment {
                    width: iw,
                    height: ih,
                    layout_box: img,
                    breaks_after: false,
                });
                return;
            }

            // Recurse into inline children.
            let children: Vec<NodeId> = node.children.iter().copied().collect();
            for &cid in &children {
                let cs = &styles[cid];
                if cs.display == Display::None {
                    continue;
                }
                collect_inline_fragments(dom, styles, cid, out);
            }
        }
    }
}

/// Emit word fragments for normal text (collapse whitespace, break on words).
fn emit_word_fragments(
    text: &str,
    font_size: i32,
    bold: bool,
    italic: bool,
    color: u32,
    link: Option<String>,
    deco: TextDeco,
    out: &mut Vec<InlineFragment>,
) {
    let trimmed = text.as_bytes();
    if trimmed.is_empty() {
        return;
    }

    // Split into words on whitespace boundaries, preserving a single space
    // between words.
    let mut i = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    // Leading space.
    let has_leading_space = len > 0 && is_ascii_ws(bytes[0]);

    // Collect words.
    let mut words: Vec<&str> = Vec::new();
    while i < len {
        // Skip whitespace.
        while i < len && is_ascii_ws(bytes[i]) {
            i += 1;
        }
        if i >= len {
            break;
        }
        let start = i;
        while i < len && !is_ascii_ws(bytes[i]) {
            i += 1;
        }
        if let Ok(word) = core::str::from_utf8(&bytes[start..i]) {
            words.push(word);
        }
    }

    // Trailing space.
    let has_trailing_space = len > 1 && is_ascii_ws(bytes[len - 1]);

    // Emit space fragment before first word if there was leading whitespace.
    if has_leading_space && !words.is_empty() {
        let (sw, sh) = measure_text(" ", font_size, bold);
        let mut space_box = LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
        space_box.link_url = link.clone();
        space_box.text_decoration = deco;
        out.push(InlineFragment {
            width: sw,
            height: sh,
            layout_box: space_box,
            breaks_after: false,
        });
    }

    for (wi, word) in words.iter().enumerate() {
        let (ww, wh) = measure_text(word, font_size, bold);
        let mut wbox = LayoutBox::new_text(String::from(*word), font_size, bold, italic, color);
        wbox.link_url = link.clone();
        wbox.text_decoration = deco;
        out.push(InlineFragment {
            width: ww,
            height: wh,
            layout_box: wbox,
            breaks_after: false,
        });

        // Inter-word space (or trailing space after last word).
        let need_space = wi + 1 < words.len() || has_trailing_space;
        if need_space {
            let (sw, sh) = measure_text(" ", font_size, bold);
            let mut sbox = LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
            sbox.link_url = link.clone();
            sbox.text_decoration = deco;
            out.push(InlineFragment {
                width: sw,
                height: sh,
                layout_box: sbox,
                breaks_after: false,
            });
        }
    }
}

/// Emit fragments for preformatted text (preserve whitespace, break on \n).
fn emit_preformatted_fragments(
    text: &str,
    font_size: i32,
    bold: bool,
    italic: bool,
    color: u32,
    link: Option<String>,
    deco: TextDeco,
    out: &mut Vec<InlineFragment>,
) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find end of this segment (up to newline or end).
        let start = i;
        while i < len && bytes[i] != b'\n' {
            i += 1;
        }

        if start < i {
            if let Ok(seg) = core::str::from_utf8(&bytes[start..i]) {
                let (sw, sh) = measure_text(seg, font_size, bold);
                let mut sbox = LayoutBox::new_text(String::from(seg), font_size, bold, italic, color);
                sbox.link_url = link.clone();
                sbox.text_decoration = deco;
                out.push(InlineFragment {
                    width: sw,
                    height: sh,
                    layout_box: sbox,
                    breaks_after: false,
                });
            }
        }

        if i < len && bytes[i] == b'\n' {
            // Emit a line break.
            let brk = LayoutBox::new(None, BoxType::Inline);
            out.push(InlineFragment {
                width: 0,
                height: if font_size > 0 { font_size } else { 16 },
                layout_box: brk,
                breaks_after: true,
            });
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Image dimension helpers
// ---------------------------------------------------------------------------

/// Parse image width/height attributes or fall back to defaults.
fn image_dimensions(dom: &Dom, node_id: NodeId, max_width: i32) -> (i32, i32) {
    let w = dom.attr(node_id, "width").and_then(parse_attr_int).unwrap_or(300);
    let h = dom.attr(node_id, "height").and_then(parse_attr_int).unwrap_or(150);

    // Clamp to available width, preserving aspect ratio.
    if w > max_width && w > 0 {
        let scaled_h = (h as i64 * max_width as i64 / w as i64) as i32;
        (max_width, scaled_h.max(1))
    } else {
        (w, h)
    }
}

fn parse_attr_int(s: &str) -> Option<i32> {
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

fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}
