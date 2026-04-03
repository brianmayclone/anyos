//! Inline layout: line-breaking, word wrapping, and inline element fragments.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{ComputedStyle, Display, Position, WhiteSpace, TextDeco, TextTransform, TextAlignVal, VerticalAlign, PseudoStyles};
use crate::ImageCache;

use super::{
    LayoutBox, BoxType, FormFieldKind,
    font_size_px, is_bold, is_italic, inherited_link,
    image_dimensions, measure_text, parse_attr_int,
    is_ascii_ws, ascii_lower_str, size_attr_width,
    apply_text_transform,
};

/// Represents a single inline fragment before line-breaking.
struct InlineFragment {
    width: i32,
    height: i32,
    layout_box: LayoutBox,
    breaks_after: bool,
}

/// Lay out a run of inline child nodes, performing word wrapping.
/// Returns a list of line boxes positioned at x = `start_x`.
pub fn layout_inline_content(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    start_x: i32,
    images: &ImageCache,
    text_align: TextAlignVal,
    line_height: i32,
    viewport_w: i32,
) -> Vec<LayoutBox> {
    layout_inline_content_with_pseudo(
        dom, styles, pseudo, child_ids, available_width, start_x, images,
        text_align, line_height, viewport_w, None, None,
    )
}

/// Like `layout_inline_content` but also accepts inline before/after pseudo-element styles
/// from the block parent (these are not reachable through child_ids).
pub fn layout_inline_content_with_pseudo(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    start_x: i32,
    images: &ImageCache,
    text_align: TextAlignVal,
    line_height: i32,
    viewport_w: i32,
    before_ps: Option<&ComputedStyle>,
    after_ps: Option<&ComputedStyle>,
) -> Vec<LayoutBox> {
    // Determine text_indent from the parent style of the first child.
    let text_indent = if let Some(&first_cid) = child_ids.first() {
        let dom_node = dom.get(first_cid);
        if let Some(pid) = dom_node.parent {
            styles[pid].text_indent
        } else { 0 }
    } else { 0 };

    // 1. Flatten all inline children into fragments.
    let mut fragments: Vec<InlineFragment> = Vec::new();

    // Inject parent's ::before pseudo-element as first fragment (inline display only).
    if let Some(bps) = before_ps {
        if let Some(ref text) = bps.content {
            if !text.is_empty() {
                let fs = if bps.font_size > 0 { bps.font_size } else { 16 };
                let bold = matches!(bps.font_weight, crate::style::FontWeight::Bold);
                let italic = matches!(bps.font_style, crate::style::FontStyleVal::Italic);
                let (tw, th) = measure_text(text, fs, bold);
                let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, bps.color);
                tb.bg_color = bps.background_color;
                tb.text_decoration = bps.text_decoration;
                tb.letter_spacing = bps.letter_spacing;
                fragments.push(InlineFragment { width: tw, height: th, layout_box: tb, breaks_after: false });
            }
        }
    }

    for &cid in child_ids {
        let style = &styles[cid];
        if style.display == Display::None {
            continue;
        }
        collect_inline_fragments(dom, styles, pseudo, cid, &mut fragments, available_width, images, 0, viewport_w);
    }

    // Inject parent's ::after pseudo-element as last fragment (inline display only).
    if let Some(aps) = after_ps {
        if let Some(ref text) = aps.content {
            if !text.is_empty() {
                let fs = if aps.font_size > 0 { aps.font_size } else { 16 };
                let bold = matches!(aps.font_weight, crate::style::FontWeight::Bold);
                let italic = matches!(aps.font_style, crate::style::FontStyleVal::Italic);
                let (tw, th) = measure_text(text, fs, bold);
                let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, aps.color);
                tb.bg_color = aps.background_color;
                tb.text_decoration = aps.text_decoration;
                tb.letter_spacing = aps.letter_spacing;
                fragments.push(InlineFragment { width: tw, height: th, layout_box: tb, breaks_after: false });
            }
        }
    }

    // 2. Break fragments into lines.
    let mut lines: Vec<LayoutBox> = Vec::new();
    let mut line = LayoutBox::new(None, BoxType::LineBox);
    line.x = start_x;
    line.width = available_width;
    // Apply text-indent to the first line
    let mut line_x: i32 = if text_indent > 0 { text_indent } else { 0 };
    let mut line_h: i32 = 0;
    let first_line_width = available_width - line_x;

    for frag in fragments {
        let fw = frag.width;
        let fh = frag.height;
        let _cur_avail = if lines.is_empty() { first_line_width } else { available_width };

        // Check if we need to wrap.
        if line_x > 0 && line_x + fw > (if lines.is_empty() { available_width } else { available_width }) && !line.children.is_empty() {
            line.height = line_h.max(line_height);
            lines.push(line);
            line = LayoutBox::new(None, BoxType::LineBox);
            line.x = start_x;
            line.width = available_width;
            line_x = 0; // No text-indent on subsequent lines
            line_h = 0;
        }

        let mut child = frag.layout_box;
        child.x = start_x + line_x;
        child.y = 0;
        child.width = fw;
        child.height = fh;

        line_x += fw;
        if fh > line_h {
            line_h = fh;
        }

        line.children.push(child);

        if frag.breaks_after {
            line.height = if line_h > 0 { line_h.max(line_height) } else { line_height.max(16) };
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
        line.height = line_h.max(line_height);
        lines.push(line);
    }

    // 3. Apply text-align: shift children within each line box.
    let line_count = lines.len();
    if text_align != TextAlignVal::Left {
        for (line_idx, ln) in lines.iter_mut().enumerate() {
            // Calculate used width of content in this line.
            let used: i32 = ln.children.last()
                .map(|c| (c.x - start_x) + c.width)
                .unwrap_or(0);
            let free = available_width - used;
            if free > 0 {
                match text_align {
                    TextAlignVal::Justify => {
                        // Justify: distribute extra space between words.
                        // Don't justify the last line.
                        if line_idx < line_count - 1 && ln.children.len() > 1 {
                            let gaps = (ln.children.len() - 1) as i32;
                            if gaps > 0 {
                                let extra_per_gap = free / gaps;
                                let mut remainder = free % gaps;
                                let mut cumulative = 0i32;
                                for (ci, child) in ln.children.iter_mut().enumerate() {
                                    if ci > 0 {
                                        cumulative += extra_per_gap;
                                        if remainder > 0 {
                                            cumulative += 1;
                                            remainder -= 1;
                                        }
                                    }
                                    child.x += cumulative;
                                }
                            }
                        }
                    }
                    _ => {
                        let shift = match text_align {
                            TextAlignVal::Center => free / 2,
                            TextAlignVal::Right => free,
                            _ => 0,
                        };
                        if shift > 0 {
                            for child in &mut ln.children {
                                child.x += shift;
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Vertical-align children inside each line.
    for ln in &mut lines {
        let lh = ln.height;
        for child in &mut ln.children {
            // Default: align to bottom (baseline approximation).
            let base_y = lh - child.height;
            child.y = base_y;

            // Apply vertical-align from the node's style if available.
            if let Some(nid) = child.node_id {
                if nid < styles.len() {
                    let va = &styles[nid].vertical_align;
                    child.y = match va {
                        VerticalAlign::Baseline => base_y,
                        VerticalAlign::Top => 0,
                        VerticalAlign::Middle => (lh - child.height) / 2,
                        VerticalAlign::Bottom => lh - child.height,
                        VerticalAlign::TextTop => 0,
                        VerticalAlign::TextBottom => lh - child.height,
                        VerticalAlign::Sub => base_y + child.height / 4,
                        VerticalAlign::Super => base_y - child.height / 4,
                        VerticalAlign::Length(offset) => base_y - *offset,
                    };
                }
            }
        }
    }

    lines
}

/// Recursively collect inline fragments from a node and its inline children.
fn collect_inline_fragments(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    out: &mut Vec<InlineFragment>,
    available_width: i32,
    images: &ImageCache,
    inherited_bg: u32,
    viewport_w: i32,
) {
    let node = dom.get(node_id);
    let style = &styles[node_id];

    // Absolutely/fixed-positioned elements are removed from inline flow.
    // They are handled as deferred boxes in layout_children instead.
    if matches!(style.position, Position::Absolute | Position::Fixed) {
        return;
    }

    match &node.node_type {
        NodeType::Text(text) => {
            // SVG raw text children must never be rendered as visible text.
            // The HTML parser stores the SVG inner markup (<path>, <circle>, etc.)
            // as a Text node child of the <svg> element.  Normally the SVG early-
            // return prevents this node from being visited, but CSS display:contents
            // or other promotion paths could expose it.
            if let Some(pid) = node.parent {
                if dom.tag(pid) == Some(Tag::Svg) {
                    return;
                }
            }

            let fs = font_size_px(style);
            let bold = is_bold(style);
            let italic = is_italic(style);
            let color = style.color;
            let link = inherited_link(dom, node_id);
            let deco = style.text_decoration;

            // Apply text-transform
            let transformed = if style.text_transform != TextTransform::None {
                apply_text_transform(text, style.text_transform)
            } else {
                String::from(text.as_str())
            };

            let start_idx = out.len();
            if style.white_space == WhiteSpace::Pre || style.white_space == WhiteSpace::PreWrap {
                emit_preformatted_fragments(&transformed, fs, bold, italic, color, link, deco, out);
            } else if style.white_space == WhiteSpace::Nowrap {
                emit_nowrap_fragments(&transformed, fs, bold, italic, color, link, deco, out);
            } else {
                emit_word_fragments(&transformed, fs, bold, italic, color, link, deco,
                    style.letter_spacing, style.word_spacing, out);
            }
            // Propagate inherited background color to newly emitted text fragments.
            if inherited_bg != 0 {
                for frag in &mut out[start_idx..] {
                    if frag.layout_box.bg_color == 0 {
                        frag.layout_box.bg_color = inherited_bg;
                    }
                }
            }
            // Resolve web font ID from font-family.
            if let Some(ref family) = style.font_family {
                if let Some(wf_id) = crate::lookup_web_font(family) {
                    for frag in &mut out[start_idx..] {
                        frag.layout_box.custom_font_id = wf_id;
                    }
                }
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

            // Handle inline <img> — use available_width instead of hardcoded 300
            if *tag == Tag::Img {
                let (iw, ih) = image_dimensions(dom, node_id, available_width, images);
                let mut img = LayoutBox::new(Some(node_id), BoxType::Inline);
                img.image_src = dom.attr(node_id, "src").map(|s| String::from(s));
                img.image_width = Some(iw);
                img.image_height = Some(ih);
                img.object_fit = style.object_fit;
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

            // Handle inline <svg> as replaced element.
            if *tag == Tag::Svg {
                let key = super::svg_inline_key(node_id);
                let natural = images.get_ref(&key).map(|e| {
                    (e.width.min(65535) as i32, e.height.min(65535) as i32)
                });
                let iw = dom.attr(node_id, "width").and_then(parse_attr_int)
                    .or(natural.map(|(w, _)| w)).unwrap_or(100);
                let ih = dom.attr(node_id, "height").and_then(parse_attr_int)
                    .or(natural.map(|(_, h)| h)).unwrap_or(100);
                let iw = iw.min(available_width.max(1));
                let mut img = LayoutBox::new(Some(node_id), BoxType::Inline);
                img.image_src = Some(key);
                img.image_width = Some(iw);
                img.image_height = Some(ih);
                img.object_fit = style.object_fit;
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

            // Handle <input>
            if *tag == Tag::Input {
                emit_input_fragment(dom, styles, node_id, out);
                return;
            }

            // Handle <button>
            if *tag == Tag::Button {
                emit_button_fragment(dom, styles, node_id, out);
                return;
            }

            // Handle <textarea>
            if *tag == Tag::Textarea {
                let cols = dom.attr(node_id, "cols").and_then(parse_attr_int).unwrap_or(20);
                let rows = dom.attr(node_id, "rows").and_then(parse_attr_int).unwrap_or(2);
                let w = (cols * 8).max(80).min(600);
                let h = (rows * 18).max(28).min(400);
                let mut ta = LayoutBox::new(Some(node_id), BoxType::Inline);
                ta.form_field = Some(FormFieldKind::Textarea);
                out.push(InlineFragment { width: w, height: h, layout_box: ta, breaks_after: false });
                return;
            }

            // Handle <select>
            if *tag == Tag::Select {
                let w = 150;
                let mut sel = LayoutBox::new(Some(node_id), BoxType::Inline);
                sel.form_field = Some(FormFieldKind::TextInput);
                out.push(InlineFragment { width: w, height: 28, layout_box: sel, breaks_after: false });
                return;
            }

            // CSS 2.1 §9.2.1.1: Block-level elements inside inline formatting context.
            // When a block-level box appears inside an inline context, it breaks the
            // inline formatting and is laid out as a block box on its own "line".
            // We treat it like an inline-block that fills available width and
            // forces a line break after.
            if matches!(style.display, Display::Block | Display::FlowRoot
                | Display::Flex | Display::Grid | Display::ListItem)
            {
                use super::block::build_block;
                let mut block_box = build_block(dom, styles, pseudo, node_id, available_width, images, viewport_w);
                let w = block_box.width + block_box.margin.left + block_box.margin.right;
                let h = block_box.height + block_box.margin.top + block_box.margin.bottom;
                // Skip empty blocks (no content, no padding/border) to avoid
                // spurious line breaks from empty containers like <ul></ul>.
                if h <= 0 && block_box.children.is_empty() {
                    return;
                }
                block_box.box_type = BoxType::InlineBlock;
                out.push(InlineFragment { width: w, height: h, layout_box: block_box, breaks_after: true });
                return;
            }

            // Handle display: inline-block / inline-flex — lay out as block, emit as inline fragment.
            if matches!(style.display, Display::InlineBlock | Display::InlineFlex) {
                use super::block::build_block;
                // Shrink-to-fit: if no explicit width, use max-content so the box is only as
                // wide as its content (CSS §10.3.9 "Inline replaced elements, block-level
                // replaced elements in normal flow, and inline-block elements").
                let stf_w = if style.width.is_some() || style.width_pct.is_some() {
                    available_width  // explicit width → honour it
                } else {
                    super::flex::measure_max_content(dom, styles, pseudo, node_id, images, viewport_w)
                        .min(available_width).max(1)
                };
                let mut block_box = build_block(dom, styles, pseudo, node_id, stf_w, images, viewport_w);
                block_box.box_type = BoxType::InlineBlock;
                let w = block_box.width + block_box.margin.left + block_box.margin.right;
                let h = block_box.height + block_box.margin.top + block_box.margin.bottom;
                out.push(InlineFragment { width: w, height: h, layout_box: block_box, breaks_after: false });
                return;
            }

            // Recurse into inline children, applying inline margin/padding.
            let ml = style.margin_left.max(0);
            let mr = style.margin_right.max(0);
            let pl = style.padding_left.max(0);
            let pr = style.padding_right.max(0);

            // Left margin + padding → insert spacer.
            let left_space = ml + pl;
            if left_space > 0 {
                let spacer = LayoutBox::new(None, BoxType::Inline);
                out.push(InlineFragment { width: left_space, height: 0, layout_box: spacer, breaks_after: false });
            }

            // Inject ::before pseudo-element content.
            if node_id < pseudo.before.len() {
                if let Some(ref ps) = pseudo.before[node_id] {
                    if let Some(ref text) = ps.content {
                        if !text.is_empty() {
                            let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                            let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                            let italic = matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                            let (tw, th) = measure_text(text, fs, bold);
                            let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                            tb.bg_color = ps.background_color;
                            tb.text_decoration = ps.text_decoration;
                            out.push(InlineFragment { width: tw, height: th, layout_box: tb, breaks_after: false });
                        }
                    }
                }
            }

            let children: Vec<NodeId> = node.children.iter().copied().collect();
            let child_bg = if style.background_color != 0 { style.background_color } else { inherited_bg };

            // CSS 2.1 §9.2.1.1: When an inline element contains block-level
            // children, whitespace-only text nodes between blocks are stripped
            // (they do not generate anonymous inline boxes).
            let has_block_child = children.iter().any(|&cid| {
                let cs = &styles[cid];
                cs.display != Display::None && matches!(cs.display,
                    Display::Block | Display::FlowRoot | Display::Flex
                    | Display::Grid | Display::ListItem)
            });

            for &cid in &children {
                let cs = &styles[cid];
                if cs.display == Display::None {
                    continue;
                }
                // Skip whitespace-only text between block siblings.
                if has_block_child {
                    if let NodeType::Text(ref t) = dom.get(cid).node_type {
                        if t.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
                            continue;
                        }
                    }
                }
                collect_inline_fragments(dom, styles, pseudo, cid, out, available_width, images, child_bg, viewport_w);
            }

            // Inject ::after pseudo-element content.
            if node_id < pseudo.after.len() {
                if let Some(ref ps) = pseudo.after[node_id] {
                    if let Some(ref text) = ps.content {
                        if !text.is_empty() {
                            let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                            let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                            let italic = matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                            let (tw, th) = measure_text(text, fs, bold);
                            let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                            tb.bg_color = ps.background_color;
                            tb.text_decoration = ps.text_decoration;
                            out.push(InlineFragment { width: tw, height: th, layout_box: tb, breaks_after: false });
                        }
                    }
                }
            }

            // Right padding + margin → insert spacer.
            let right_space = pr + mr;
            if right_space > 0 {
                let spacer = LayoutBox::new(None, BoxType::Inline);
                out.push(InlineFragment { width: right_space, height: 0, layout_box: spacer, breaks_after: false });
            }
        }
    }
}

/// Emit fragments for nowrap text (no line breaking within words or between them).
fn emit_nowrap_fragments(
    text: &str,
    font_size: i32,
    bold: bool,
    italic: bool,
    color: u32,
    link: Option<String>,
    deco: TextDeco,
    out: &mut Vec<InlineFragment>,
) {
    let collapsed = collapse_whitespace(text);
    if collapsed.is_empty() { return; }
    let (w, h) = measure_text(&collapsed, font_size, bold);
    let mut wbox = LayoutBox::new_text(collapsed, font_size, bold, italic, color);
    wbox.link_url = link;
    wbox.text_decoration = deco;
    out.push(InlineFragment { width: w, height: h, layout_box: wbox, breaks_after: false });
}

/// Collapse whitespace sequences to single spaces.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Emit an `<input>` form field fragment.
fn emit_input_fragment(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    out: &mut Vec<InlineFragment>,
) {
    let input_type = dom.attr(node_id, "type").unwrap_or("text");
    let mut lower_buf = [0u8; 16];
    let lower = ascii_lower_str(input_type, &mut lower_buf);

    // Propagate CSS-declared background and text colors to the fragment so the
    // renderer can apply them to the native widget instead of its theme default.
    let (css_bg, css_fg) = if node_id < styles.len() {
        (styles[node_id].background_color, styles[node_id].color)
    } else {
        (0, 0)
    };

    match lower {
        "hidden" => {
            // Hidden inputs have no visual representation but must be tracked
            // for form submission. Create a zero-size layout box.
            let mut hid = LayoutBox::new(Some(node_id), BoxType::Inline);
            hid.form_field = Some(FormFieldKind::Hidden);
            hid.form_value = dom.attr(node_id, "value").map(String::from);
            out.push(InlineFragment { width: 0, height: 0, layout_box: hid, breaks_after: false });
            return;
        }
        "checkbox" => {
            let mut cb = LayoutBox::new(Some(node_id), BoxType::Inline);
            cb.form_field = Some(FormFieldKind::Checkbox);
            out.push(InlineFragment { width: 20, height: 20, layout_box: cb, breaks_after: false });
        }
        "radio" => {
            let mut rb = LayoutBox::new(Some(node_id), BoxType::Inline);
            rb.form_field = Some(FormFieldKind::Radio);
            out.push(InlineFragment { width: 20, height: 20, layout_box: rb, breaks_after: false });
        }
        "submit" | "button" | "reset" => {
            let label = dom.attr(node_id, "value").unwrap_or("Submit");
            let (bw, _) = measure_text(label, 14, false);
            let w = (bw + 24).max(60);
            let mut btn = LayoutBox::new(Some(node_id), BoxType::Inline);
            btn.form_field = Some(FormFieldKind::Submit);
            btn.text = Some(String::from(label));
            btn.bg_color = css_bg;
            btn.color = css_fg;
            out.push(InlineFragment { width: w, height: 28, layout_box: btn, breaks_after: false });
        }
        "password" => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Password);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.bg_color = css_bg;
            tf.color = css_fg;
            out.push(InlineFragment { width: w, height: 28, layout_box: tf, breaks_after: false });
        }
        _ => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::TextInput);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.bg_color = css_bg;
            tf.color = css_fg;
            out.push(InlineFragment { width: w, height: 28, layout_box: tf, breaks_after: false });
        }
    }
}

/// Emit a `<button>` form field fragment.
fn emit_button_fragment(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    out: &mut Vec<InlineFragment>,
) {
    let text = dom.text_content(node_id);
    let label = text.trim();
    let label = if label.is_empty() { "Button" } else { label };
    let (bw, _) = measure_text(label, 14, false);
    let w = (bw + 24).max(60);
    let btn_type = dom.attr(node_id, "type").unwrap_or("submit");
    let kind = if btn_type == "submit" { FormFieldKind::Submit } else { FormFieldKind::ButtonEl };
    let mut btn = LayoutBox::new(Some(node_id), BoxType::Inline);
    btn.form_field = Some(kind);
    btn.text = Some(String::from(label));
    // Apply CSS colors so the renderer can style the button widget.
    if node_id < styles.len() {
        btn.bg_color = styles[node_id].background_color;
        btn.color = styles[node_id].color;
    }
    out.push(InlineFragment { width: w, height: 28, layout_box: btn, breaks_after: false });
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
    letter_spacing: i32,
    word_spacing: i32,
    out: &mut Vec<InlineFragment>,
) {
    let trimmed = text.as_bytes();
    if trimmed.is_empty() {
        return;
    }

    let mut i = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    let has_leading_space = len > 0 && is_ascii_ws(bytes[0]);

    let mut words: Vec<&str> = Vec::new();
    while i < len {
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

    let has_trailing_space = len > 1 && is_ascii_ws(bytes[len - 1]);

    if words.is_empty() {
        if has_leading_space {
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
        return;
    }

    if has_leading_space {
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
        // Apply letter-spacing: add extra pixels per character.
        let letter_extra = letter_spacing * (word.len().max(1) as i32 - 1).max(0);
        let mut wbox = LayoutBox::new_text(String::from(*word), font_size, bold, italic, color);
        wbox.link_url = link.clone();
        wbox.text_decoration = deco;
        out.push(InlineFragment {
            width: ww + letter_extra,
            height: wh,
            layout_box: wbox,
            breaks_after: false,
        });

        let need_space = wi + 1 < words.len() || has_trailing_space;
        if need_space {
            let (sw, sh) = measure_text(" ", font_size, bold);
            // Apply word-spacing: add extra pixels to space between words.
            let mut sbox = LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
            sbox.link_url = link.clone();
            sbox.text_decoration = deco;
            out.push(InlineFragment {
                width: sw + word_spacing,
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
