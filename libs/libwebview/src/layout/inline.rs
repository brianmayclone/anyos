//! Inline layout: line-breaking, word wrapping, and inline element fragments.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{ComputedStyle, Display, WhiteSpace, TextDeco, TextTransform, TextAlignVal};
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
    child_ids: &[NodeId],
    available_width: i32,
    start_x: i32,
    images: &ImageCache,
    text_align: TextAlignVal,
) -> Vec<LayoutBox> {
    // 1. Flatten all inline children into fragments.
    let mut fragments: Vec<InlineFragment> = Vec::new();
    for &cid in child_ids {
        let style = &styles[cid];
        if style.display == Display::None {
            continue;
        }
        collect_inline_fragments(dom, styles, cid, &mut fragments, available_width, images);
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
        child.y = 0;
        child.width = fw;
        child.height = fh;

        line_x += fw;
        if fh > line_h {
            line_h = fh;
        }

        line.children.push(child);

        if frag.breaks_after {
            line.height = if line_h > 0 { line_h } else { 16 };
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

    // 3. Apply text-align: shift children within each line box.
    if text_align != TextAlignVal::Left {
        for ln in &mut lines {
            // Calculate used width of content in this line.
            let used: i32 = ln.children.last()
                .map(|c| (c.x - start_x) + c.width)
                .unwrap_or(0);
            let free = available_width - used;
            if free > 0 {
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

    // 4. Baseline-align children inside each line.
    for ln in &mut lines {
        let lh = ln.height;
        for child in &mut ln.children {
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
    available_width: i32,
    images: &ImageCache,
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

            // Apply text-transform
            let transformed = if style.text_transform != TextTransform::None {
                apply_text_transform(text, style.text_transform)
            } else {
                String::from(text.as_str())
            };

            if style.white_space == WhiteSpace::Pre || style.white_space == WhiteSpace::PreWrap {
                emit_preformatted_fragments(&transformed, fs, bold, italic, color, link, deco, out);
            } else if style.white_space == WhiteSpace::Nowrap {
                // Nowrap: emit as single fragment (no word breaking)
                emit_nowrap_fragments(&transformed, fs, bold, italic, color, link, deco, out);
            } else {
                emit_word_fragments(&transformed, fs, bold, italic, color, link, deco, out);
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
                emit_button_fragment(dom, node_id, out);
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

            // Recurse into inline children.
            let children: Vec<NodeId> = node.children.iter().copied().collect();
            for &cid in &children {
                let cs = &styles[cid];
                if cs.display == Display::None {
                    continue;
                }
                collect_inline_fragments(dom, styles, cid, out, available_width, images);
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
    _styles: &[ComputedStyle],
    node_id: NodeId,
    out: &mut Vec<InlineFragment>,
) {
    let input_type = dom.attr(node_id, "type").unwrap_or("text");
    let mut lower_buf = [0u8; 16];
    let lower = ascii_lower_str(input_type, &mut lower_buf);

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
            out.push(InlineFragment { width: w, height: 28, layout_box: btn, breaks_after: false });
        }
        "password" => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Password);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            out.push(InlineFragment { width: w, height: 28, layout_box: tf, breaks_after: false });
        }
        _ => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::TextInput);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            out.push(InlineFragment { width: w, height: 28, layout_box: tf, breaks_after: false });
        }
    }
}

/// Emit a `<button>` form field fragment.
fn emit_button_fragment(
    dom: &Dom,
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
        let mut wbox = LayoutBox::new_text(String::from(*word), font_size, bold, italic, color);
        wbox.link_url = link.clone();
        wbox.text_decoration = deco;
        out.push(InlineFragment {
            width: ww,
            height: wh,
            layout_box: wbox,
            breaks_after: false,
        });

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
