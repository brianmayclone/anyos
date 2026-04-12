//! Inline layout: line-breaking, word wrapping, and inline element fragments.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{
    ComputedStyle, Display, Position, PseudoStyles, TextAlignVal, TextDeco, TextTransform,
    VerticalAlign, WhiteSpace, resolve_inset,
};
use crate::ImageCache;

use super::{
    apply_text_transform, ascii_lower_str, font_size_px, image_dimensions, inherited_link,
    is_ascii_ws, is_bold, is_italic, measure_text, parse_attr_int, size_attr_width, BoxType,
    FormFieldKind, LayoutBox,
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
        dom,
        styles,
        pseudo,
        child_ids,
        available_width,
        start_x,
        images,
        text_align,
        line_height,
        viewport_w,
        None,
        None,
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
        } else {
            0
        }
    } else {
        0
    };

    // 1. Flatten all inline children into fragments.
    let mut fragments: Vec<InlineFragment> = Vec::new();

    // Inject parent's ::before pseudo-element as first fragment (inline display only).
    if let Some(bps) = before_ps {
        if let Some(ref text) = bps.content {
            if !text.is_empty() {
                let fs = if bps.font_size > 0 { bps.font_size } else { 16 };
                let bold = matches!(bps.font_weight, crate::style::FontWeight::Bold);
                let italic = matches!(bps.font_style, crate::style::FontStyleVal::Italic);
                let custom_font_id = bps
                    .font_family
                    .as_ref()
                    .and_then(|family| crate::lookup_web_font(family))
                    .unwrap_or(0);
                let (tw, th) = measure_text(text, fs, custom_font_id, bold, italic);
                let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, bps.color);
                tb.custom_font_id = custom_font_id;
                tb.bg_color = bps.background_color;
                tb.text_decoration = bps.text_decoration;
                tb.letter_spacing = bps.letter_spacing;
                fragments.push(InlineFragment {
                    width: tw,
                    height: th,
                    layout_box: tb,
                    breaks_after: false,
                });
            }
        }
    }

    for &cid in child_ids {
        let style = &styles[cid];
        if style.display == Display::None {
            continue;
        }
        collect_inline_fragments(
            dom,
            styles,
            pseudo,
            cid,
            &mut fragments,
            available_width,
            images,
            0,
            viewport_w,
        );
    }

    // Inject parent's ::after pseudo-element as last fragment (inline display only).
    if let Some(aps) = after_ps {
        if let Some(ref text) = aps.content {
            if !text.is_empty() {
                let fs = if aps.font_size > 0 { aps.font_size } else { 16 };
                let bold = matches!(aps.font_weight, crate::style::FontWeight::Bold);
                let italic = matches!(aps.font_style, crate::style::FontStyleVal::Italic);
                let custom_font_id = aps
                    .font_family
                    .as_ref()
                    .and_then(|family| crate::lookup_web_font(family))
                    .unwrap_or(0);
                let (tw, th) = measure_text(text, fs, custom_font_id, bold, italic);
                let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, aps.color);
                tb.custom_font_id = custom_font_id;
                tb.bg_color = aps.background_color;
                tb.text_decoration = aps.text_decoration;
                tb.letter_spacing = aps.letter_spacing;
                fragments.push(InlineFragment {
                    width: tw,
                    height: th,
                    layout_box: tb,
                    breaks_after: false,
                });
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
        let is_oof_block_like =
            frag.layout_box.is_out_of_flow && !matches!(frag.layout_box.box_type, BoxType::Inline);
        let is_collapsible_space = matches!(frag.layout_box.text.as_deref(), Some(" "));
        let _cur_avail = if lines.is_empty() {
            first_line_width
        } else {
            available_width
        };

        if is_oof_block_like && !line.children.is_empty() {
            line.height = if line_h > 0 {
                line_h.max(line_height)
            } else {
                0
            };
            lines.push(line);
            line = LayoutBox::new(None, BoxType::LineBox);
            line.x = start_x;
            line.width = available_width;
            line_x = 0;
            line_h = 0;
        }

        // Collapsed whitespace must not generate visible advance at the start
        // of a line. This keeps line layout aligned with CSS whitespace
        // collapsing and prevents shrink-to-fit width mismatches.
        if is_collapsible_space && line.children.is_empty() {
            continue;
        }

        // Check if we need to wrap.
        if line_x > 0
            && line_x + fw
                > (if lines.is_empty() {
                    available_width
                } else {
                    available_width
                })
            && !line.children.is_empty()
        {
            line.height = if line_h > 0 {
                line_h.max(line_height)
            } else {
                0
            };
            lines.push(line);
            line = LayoutBox::new(None, BoxType::LineBox);
            line.x = start_x;
            line.width = available_width;
            line_x = 0; // No text-indent on subsequent lines
            line_h = 0;
        }

        let mut child = frag.layout_box;
        child.x = start_x + line_x + child.x;
        child.y = child.y;
        if !child.is_out_of_flow {
            child.width = fw;
            child.height = fh;
        }
        if child.is_out_of_flow {
            child.static_position_x = Some(child.x);
            child.static_position_y = Some(child.y);
        }

        line_x += fw;
        if fh > line_h {
            line_h = fh;
        }

        line.children.push(child);

        if frag.breaks_after {
            // Use line_h when > 0; do NOT apply a minimum height for lines that
            // consist entirely of zero-height content (e.g. a collapsed
            // overflow:hidden block).  Applying line_height.max(16) here would
            // give phantom height to every collapsed dropdown in the page.
            line.height = if line_h > 0 {
                line_h.max(line_height)
            } else {
                0
            };
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
        // Same rule as in breaks_after: zero-height-only content produces a 0-height
        // line box (e.g. a trailing whitespace-only text node inside an inline element).
        line.height = if line_h > 0 {
            line_h.max(line_height)
        } else {
            0
        };
        lines.push(line);
    }

    // 3. Apply text-align: shift children within each line box.
    let line_count = lines.len();
    if text_align != TextAlignVal::Left {
        for (line_idx, ln) in lines.iter_mut().enumerate() {
            // Calculate used width of content in this line.
            let used: i32 = ln
                .children
                .last()
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

    if matches!(style.position, Position::Absolute | Position::Fixed) {
        let mut oof_box = if matches!(
            style.display,
            Display::Block
                | Display::FlowRoot
                | Display::Flex
                | Display::Grid
                | Display::ListItem
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::InlineGrid
        ) {
            super::block::build_block(dom, styles, pseudo, node_id, available_width, images, viewport_w, 0)
        } else {
            let mut bx = LayoutBox::new(Some(node_id), BoxType::Inline);
            bx.width = 0;
            bx.height = 0;
            bx
        };
        oof_box.is_out_of_flow = true;
        oof_box.is_fixed = style.position == Position::Fixed;
        out.push(InlineFragment {
            width: 0,
            height: 0,
            layout_box: oof_box,
            breaks_after: false,
        });
        return;
    }

    match &node.node_type {
        NodeType::Text(text) => {
            // SVG raw text children must never be rendered as visible text.
            // The HTML parser stores the SVG inner markup (<path>, <circle>, etc.)
            // as a Text node child of the <svg> element.  Walk the full ancestor
            // chain because SVG elements like <g>, <defs> etc. become Tag::Unknown
            // and wrap the actual text nodes.
            if super::is_inside_svg(dom, node_id) {
                return;
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

            let custom_font_id = style
                .font_family
                .as_ref()
                .and_then(|family| crate::lookup_web_font(family))
                .unwrap_or(0);
            let start_idx = out.len();
            if style.white_space == WhiteSpace::Pre || style.white_space == WhiteSpace::PreWrap {
                emit_preformatted_fragments(
                    &transformed,
                    fs,
                    custom_font_id,
                    bold,
                    italic,
                    color,
                    link,
                    deco,
                    out,
                );
            } else if style.white_space == WhiteSpace::Nowrap {
                emit_nowrap_fragments(
                    &transformed,
                    fs,
                    custom_font_id,
                    bold,
                    italic,
                    color,
                    link,
                    deco,
                    out,
                );
            } else {
                emit_word_fragments(
                    &transformed,
                    fs,
                    custom_font_id,
                    bold,
                    italic,
                    color,
                    link,
                    deco,
                    style.letter_spacing,
                    style.word_spacing,
                    out,
                );
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
            if custom_font_id != 0 {
                for frag in &mut out[start_idx..] {
                    frag.layout_box.custom_font_id = custom_font_id;
                }
            }
        }
        NodeType::Element { tag, .. } => {
            let fragment_start = out.len();
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
            if *tag == Tag::Img || dom.has_tag_name(node_id, "a-img") {
                let (iw, ih) = image_dimensions(dom, node_id, available_width, images);
                let mut img = LayoutBox::new(Some(node_id), BoxType::Inline);
                img.image_src = dom.image_url(node_id);
                img.image_width = Some(iw);
                img.image_height = Some(ih);
                img.object_fit = style.object_fit;
                img.object_position_x = style.object_position_x;
                img.object_position_x_is_percent = style.object_position_x_is_percent;
                img.object_position_y = style.object_position_y;
                img.object_position_y_is_percent = style.object_position_y_is_percent;
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
                let natural = images
                    .get_ref(&key)
                    .map(|e| (e.width.min(65535) as i32, e.height.min(65535) as i32));
                let iw = dom
                    .attr(node_id, "width")
                    .and_then(parse_attr_int)
                    .or(natural.map(|(w, _)| w))
                    .unwrap_or(100);
                let ih = dom
                    .attr(node_id, "height")
                    .and_then(parse_attr_int)
                    .or(natural.map(|(_, h)| h))
                    .unwrap_or(100);
                let iw = iw.min(available_width.max(1));
                let mut img = LayoutBox::new(Some(node_id), BoxType::Inline);
                img.image_src = Some(key);
                img.image_width = Some(iw);
                img.image_height = Some(ih);
                img.object_fit = style.object_fit;
                img.object_position_x = style.object_position_x;
                img.object_position_x_is_percent = style.object_position_x_is_percent;
                img.object_position_y = style.object_position_y;
                img.object_position_y_is_percent = style.object_position_y_is_percent;
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

            // Render only plain-text buttons as native form controls.
            // Rich buttons with SVG/text children should keep their DOM content.
            if *tag == Tag::Button && button_uses_native_control(dom, node_id) {
                emit_button_fragment(dom, styles, node_id, out);
                return;
            }

            // Handle <textarea>
            if *tag == Tag::Textarea {
                let cols = dom
                    .attr(node_id, "cols")
                    .and_then(parse_attr_int)
                    .unwrap_or(20);
                let rows = dom
                    .attr(node_id, "rows")
                    .and_then(parse_attr_int)
                    .unwrap_or(2);
                let w = (cols * 8).max(80).min(600);
                let h = (rows * 18).max(28).min(400);
                let mut ta = LayoutBox::new(Some(node_id), BoxType::Inline);
                ta.form_field = Some(FormFieldKind::Textarea);
                if node_id < styles.len() {
                    ta.bg_color = styles[node_id].background_color;
                    ta.color = styles[node_id].color;
                    ta.accent_color = styles[node_id].accent_color;
                    ta.uses_dark_color_scheme =
                        styles[node_id].color_scheme == crate::style::ColorSchemeVal::Dark;
                }
                out.push(InlineFragment {
                    width: w,
                    height: h,
                    layout_box: ta,
                    breaks_after: false,
                });
                return;
            }

            // Handle <select>
            if *tag == Tag::Select {
                // Collect all <option> items with their values and optgroup labels.
                // DropDown widget uses pipe-separated items.
                let mut labels = String::new();
                let mut values = String::new();
                let mut selected_idx: i32 = -1;
                let mut opt_count: i32 = 0;
                let mut first_enabled_idx: i32 = -1;
                let children = &dom.get(node_id).children.clone();
                for &cid in children {
                    if dom.tag(cid) == Some(Tag::Optgroup) {
                        // Optgroup: add a disabled separator label (prefixed with "─ ")
                        let group_label = dom.attr(cid, "label").unwrap_or("");
                        if !labels.is_empty() {
                            labels.push('|');
                            values.push('|');
                        }
                        labels.push_str("\u{2500} ");
                        labels.push_str(group_label);
                        values.push_str("__optgroup__");
                        opt_count += 1;
                        // Process children of optgroup
                        let group_children = &dom.get(cid).children.clone();
                        for &gcid in group_children {
                            if dom.tag(gcid) == Some(Tag::Option) {
                                let txt = dom.text_content(gcid);
                                let txt = txt.trim();
                                let val = dom.attr(gcid, "value").unwrap_or(txt);
                                if !labels.is_empty() {
                                    labels.push('|');
                                    values.push('|');
                                }
                                // Indent options within optgroup
                                labels.push_str("  ");
                                labels.push_str(txt);
                                values.push_str(val);
                                if dom.attr(gcid, "disabled").is_none() && first_enabled_idx < 0 {
                                    first_enabled_idx = opt_count;
                                }
                                if dom.attr(gcid, "selected").is_some() && selected_idx < 0 {
                                    selected_idx = opt_count;
                                }
                                opt_count += 1;
                            }
                        }
                    } else if dom.tag(cid) == Some(Tag::Option) {
                        let txt = dom.text_content(cid);
                        let txt = txt.trim();
                        let val = dom.attr(cid, "value").unwrap_or(txt);
                        if !labels.is_empty() {
                            labels.push('|');
                            values.push('|');
                        }
                        labels.push_str(txt);
                        values.push_str(val);
                        if dom.attr(cid, "disabled").is_none() && first_enabled_idx < 0 {
                            first_enabled_idx = opt_count;
                        }
                        if dom.attr(cid, "selected").is_some() && selected_idx < 0 {
                            selected_idx = opt_count;
                        }
                        opt_count += 1;
                    }
                }
                // Default to first enabled option if none explicitly selected.
                if selected_idx < 0 {
                    selected_idx = if first_enabled_idx >= 0 {
                        first_enabled_idx
                    } else {
                        0
                    };
                }

                // Determine display text for sizing.
                let selected_text = if selected_idx >= 0 {
                    labels
                        .split('|')
                        .nth(selected_idx as usize)
                        .unwrap_or("\u{00a0}")
                } else {
                    "\u{00a0}"
                };

                let is_multiple = dom.attr(node_id, "multiple").is_some();
                let size_attr: u32 = dom
                    .attr(node_id, "size")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(if is_multiple { 4 } else { 0 });
                let disabled = dom.attr(node_id, "disabled").is_some();
                let required = dom.attr(node_id, "required").is_some();

                let fs = font_size_px(style);
                let bold = is_bold(style);
                let italic = is_italic(style);
                let custom_font_id = style
                    .font_family
                    .as_ref()
                    .and_then(|family| crate::lookup_web_font(family))
                    .unwrap_or(0);
                let (tw, _) = measure_text(selected_text, fs, custom_font_id, bold, italic);
                // Width: max of all option widths + padding for arrow
                let mut max_w = tw;
                for opt_label in labels.split('|') {
                    let (ow, _) = measure_text(opt_label, fs, custom_font_id, bold, italic);
                    if ow > max_w {
                        max_w = ow;
                    }
                }
                let w = (max_w + 36).max(80).min(400);
                let h = if size_attr > 1 {
                    // Listbox mode: show `size` rows.
                    (size_attr as i32 * (fs + 4)).max(28)
                } else {
                    28
                };

                let mut sel = LayoutBox::new(Some(node_id), BoxType::Inline);
                sel.form_field = Some(FormFieldKind::Select);
                sel.text = Some(String::from(selected_text));
                sel.font_size = fs;
                sel.bold = bold;
                sel.form_options = Some(labels);
                sel.form_option_values = Some(values);
                sel.form_selected_index = selected_idx;
                sel.form_multiple = is_multiple;
                sel.form_size = size_attr;
                sel.form_disabled = disabled;
                sel.form_required = required;
                if node_id < styles.len() {
                    sel.bg_color = styles[node_id].background_color;
                    sel.color = styles[node_id].color;
                    sel.accent_color = styles[node_id].accent_color;
                    sel.uses_dark_color_scheme =
                        styles[node_id].color_scheme == crate::style::ColorSchemeVal::Dark;
                }
                out.push(InlineFragment {
                    width: w,
                    height: h,
                    layout_box: sel,
                    breaks_after: false,
                });
                return;
            }

            // Handle <progress>
            if *tag == Tag::Progress {
                let max_val: f32 = dom
                    .attr(node_id, "max")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                let cur_val: f32 = dom
                    .attr(node_id, "value")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let pct = if max_val > 0.0 {
                    (cur_val / max_val).min(1.0).max(0.0)
                } else {
                    0.0
                };

                let w = 200;
                let h = 20;

                // Store the percentage in form_value as an integer 0..1000
                // (fixed-point with 1 decimal, avoids format! / float-to-string).
                let pct_i = (pct * 1000.0) as i32;
                let mut pb = LayoutBox::new(Some(node_id), BoxType::Inline);
                pb.form_field = Some(FormFieldKind::Progress);
                pb.width = w;
                pb.height = h;
                // Encode pct in form_value as "NNN" (0..1000).
                let mut val_str = String::new();
                let digits = [
                    (b'0' + (pct_i / 100 % 10) as u8) as char,
                    (b'0' + (pct_i / 10 % 10) as u8) as char,
                    (b'0' + (pct_i % 10) as u8) as char,
                ];
                for &d in &digits {
                    val_str.push(d);
                }
                if pct_i >= 1000 {
                    val_str.clear();
                    val_str.push('X');
                } // 100%
                pb.form_value = Some(val_str);
                if node_id < styles.len() {
                    pb.bg_color = styles[node_id].background_color;
                    pb.color = styles[node_id].color;
                    pb.accent_color = styles[node_id].accent_color;
                    pb.uses_dark_color_scheme =
                        styles[node_id].color_scheme == crate::style::ColorSchemeVal::Dark;
                }
                out.push(InlineFragment {
                    width: w,
                    height: h,
                    layout_box: pb,
                    breaks_after: false,
                });
                return;
            }

            // Handle <meter> (HTML §4.10.16)
            if *tag == Tag::Meter {
                let min_val: f32 = dom
                    .attr(node_id, "min")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let max_val: f32 = dom
                    .attr(node_id, "max")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                let cur_val: f32 = dom
                    .attr(node_id, "value")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let range = max_val - min_val;
                let pct = if range > 0.0 {
                    ((cur_val - min_val) / range).min(1.0).max(0.0)
                } else {
                    0.0
                };
                let pct_i = (pct * 1000.0) as i32;
                let mut val_str = String::new();
                if pct_i >= 1000 {
                    val_str.push('X');
                } else {
                    val_str.push((b'0' + (pct_i / 100 % 10) as u8) as char);
                    val_str.push((b'0' + (pct_i / 10 % 10) as u8) as char);
                    val_str.push((b'0' + (pct_i % 10) as u8) as char);
                }
                let mut mb = LayoutBox::new(Some(node_id), BoxType::Inline);
                mb.form_field = Some(FormFieldKind::Meter);
                mb.width = 200;
                mb.height = 20;
                mb.form_value = Some(val_str);
                if node_id < styles.len() {
                    mb.bg_color = styles[node_id].background_color;
                    mb.color = styles[node_id].color;
                    mb.accent_color = styles[node_id].accent_color;
                    mb.uses_dark_color_scheme =
                        styles[node_id].color_scheme == crate::style::ColorSchemeVal::Dark;
                }
                out.push(InlineFragment {
                    width: 200,
                    height: 20,
                    layout_box: mb,
                    breaks_after: false,
                });
                return;
            }

            // CSS 2.1 §9.2.1.1: Block-level elements inside inline formatting context.
            // When a block-level box appears inside an inline context, it breaks the
            // inline formatting and is laid out as a block box on its own "line".
            // We treat it like an inline-block that fills available width and
            // forces a line break after.
            if matches!(
                style.display,
                Display::Block
                    | Display::FlowRoot
                    | Display::Flex
                    | Display::Grid
                    | Display::ListItem
            ) {
                use super::block::build_block;
                let mut block_box = build_block(
                    dom,
                    styles,
                    pseudo,
                    node_id,
                    available_width,
                    images,
                    viewport_w,
                    0,
                );
                let w = block_box.width + block_box.margin.left + block_box.margin.right;
                let h = block_box.height + block_box.margin.top + block_box.margin.bottom;
                // Skip empty blocks (no content, no padding/border) to avoid
                // spurious line breaks from empty containers like <ul></ul>.
                if h <= 0 && block_box.children.is_empty() {
                    return;
                }
                block_box.box_type = BoxType::InlineBlock;
                out.push(InlineFragment {
                    width: w,
                    height: h,
                    layout_box: block_box,
                    breaks_after: true,
                });
                return;
            }

            // Handle display: inline-block / inline-flex / inline-grid — lay out as block, emit as inline fragment.
            if matches!(style.display, Display::InlineBlock | Display::InlineFlex | Display::InlineGrid) {
                use super::block::build_block;
                // Shrink-to-fit: if no explicit width, use max-content so the box is only as
                // wide as its content (CSS §10.3.9 "Inline replaced elements, block-level
                // replaced elements in normal flow, and inline-block elements").
                let stf_w = if style.width.is_some() || style.width_pct.is_some() {
                    available_width // explicit width → honour it
                } else {
                    super::flex::measure_max_content(
                        dom, styles, pseudo, node_id, images, viewport_w,
                    )
                    .min(available_width)
                    .max(1)
                };
                let mut block_box =
                    build_block(dom, styles, pseudo, node_id, stf_w, images, viewport_w, 0);
                block_box.box_type = BoxType::InlineBlock;
                let w = block_box.width + block_box.margin.left + block_box.margin.right;
                let h = block_box.height + block_box.margin.top + block_box.margin.bottom;
                out.push(InlineFragment {
                    width: w,
                    height: h,
                    layout_box: block_box,
                    breaks_after: false,
                });
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
                out.push(InlineFragment {
                    width: left_space,
                    height: 0,
                    layout_box: spacer,
                    breaks_after: false,
                });
            }

            // Inject ::before pseudo-element content.
            if node_id < pseudo.before.len() {
                if let Some(ref ps) = pseudo.before[node_id] {
                    if let Some(ref text) = ps.content {
                        if !text.is_empty() {
                            let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                            let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                            let italic =
                                matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                            let custom_font_id = ps
                                .font_family
                                .as_ref()
                                .and_then(|family| crate::lookup_web_font(family))
                                .unwrap_or(0);
                            let (tw, th) = measure_text(text, fs, custom_font_id, bold, italic);
                            let mut tb =
                                LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                            tb.custom_font_id = custom_font_id;
                            tb.bg_color = ps.background_color;
                            tb.text_decoration = ps.text_decoration;
                            out.push(InlineFragment {
                                width: tw,
                                height: th,
                                layout_box: tb,
                                breaks_after: false,
                            });
                        }
                    }
                }
            }

            let children: Vec<NodeId> = node.children.iter().copied().collect();
            let child_bg = if style.background_color != 0 {
                style.background_color
            } else {
                inherited_bg
            };

            // CSS 2.1 §9.2.1.1: When an inline element contains block-level
            // children, whitespace-only text nodes between blocks are stripped
            // (they do not generate anonymous inline boxes).
            let has_block_child = children.iter().any(|&cid| {
                let cs = &styles[cid];
                cs.display != Display::None
                    && matches!(
                        cs.display,
                        Display::Block
                            | Display::FlowRoot
                            | Display::Flex
                            | Display::Grid
                            | Display::ListItem
                    )
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
                collect_inline_fragments(
                    dom,
                    styles,
                    pseudo,
                    cid,
                    out,
                    available_width,
                    images,
                    child_bg,
                    viewport_w,
                );
            }

            // Inject ::after pseudo-element content.
            if node_id < pseudo.after.len() {
                if let Some(ref ps) = pseudo.after[node_id] {
                    if let Some(ref text) = ps.content {
                        if !text.is_empty() {
                            let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                            let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                            let italic =
                                matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                            let custom_font_id = ps
                                .font_family
                                .as_ref()
                                .and_then(|family| crate::lookup_web_font(family))
                                .unwrap_or(0);
                            let (tw, th) = measure_text(text, fs, custom_font_id, bold, italic);
                            let mut tb =
                                LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                            tb.custom_font_id = custom_font_id;
                            tb.bg_color = ps.background_color;
                            tb.text_decoration = ps.text_decoration;
                            out.push(InlineFragment {
                                width: tw,
                                height: th,
                                layout_box: tb,
                                breaks_after: false,
                            });
                        }
                    }
                }
            }

            // Right padding + margin → insert spacer.
            let right_space = pr + mr;
            if right_space > 0 {
                let spacer = LayoutBox::new(None, BoxType::Inline);
                out.push(InlineFragment {
                    width: right_space,
                    height: 0,
                    layout_box: spacer,
                    breaks_after: false,
                });
            }

            if style.position == Position::Relative {
                let dx = {
                    let left = resolve_inset(style.left_offset, style.left_calc, available_width, true);
                    let right =
                        resolve_inset(style.right_offset, style.right_calc, available_width, true);
                    left.unwrap_or_else(|| right.map(|v| -v).unwrap_or(0))
                };
                let dy = {
                    let top = resolve_inset(style.top, style.top_calc, available_width, true);
                    let bottom =
                        resolve_inset(style.bottom_offset, style.bottom_calc, available_width, true);
                    top.unwrap_or_else(|| bottom.map(|v| -v).unwrap_or(0))
                };
                if dx != 0 || dy != 0 {
                    for frag in out.iter_mut().skip(fragment_start) {
                        frag.layout_box.x += dx;
                        frag.layout_box.y += dy;
                    }
                }
            }
        }
    }
}

/// Emit fragments for nowrap text (no line breaking within words or between them).
fn emit_nowrap_fragments(
    text: &str,
    font_size: i32,
    custom_font_id: u32,
    bold: bool,
    italic: bool,
    color: u32,
    link: Option<String>,
    deco: TextDeco,
    out: &mut Vec<InlineFragment>,
) {
    let collapsed = collapse_whitespace(text);
    if collapsed.is_empty() {
        return;
    }
    let (w, h) = measure_text(&collapsed, font_size, custom_font_id, bold, italic);
    let mut wbox = LayoutBox::new_text(collapsed, font_size, bold, italic, color);
    wbox.custom_font_id = custom_font_id;
    wbox.link_url = link;
    wbox.text_decoration = deco;
    out.push(InlineFragment {
        width: w,
        height: h,
        layout_box: wbox,
        breaks_after: false,
    });
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
    let (css_bg, css_fg, css_accent, uses_dark_color_scheme) = if node_id < styles.len() {
        (
            styles[node_id].background_color,
            styles[node_id].color,
            styles[node_id].accent_color,
            styles[node_id].color_scheme == crate::style::ColorSchemeVal::Dark,
        )
    } else {
        (0, 0, 0, false)
    };

    match lower {
        "hidden" => {
            // Hidden inputs have no visual representation but must be tracked
            // for form submission. Create a zero-size layout box.
            let mut hid = LayoutBox::new(Some(node_id), BoxType::Inline);
            hid.form_field = Some(FormFieldKind::Hidden);
            hid.form_value = dom.attr(node_id, "value").map(String::from);
            hid.accent_color = css_accent;
            hid.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 0,
                height: 0,
                layout_box: hid,
                breaks_after: false,
            });
            return;
        }
        "checkbox" => {
            let mut cb = LayoutBox::new(Some(node_id), BoxType::Inline);
            cb.form_field = Some(FormFieldKind::Checkbox);
            cb.form_checked = dom.attr(node_id, "checked").is_some();
            cb.form_disabled = dom.attr(node_id, "disabled").is_some();
            cb.accent_color = css_accent;
            cb.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 20,
                height: 20,
                layout_box: cb,
                breaks_after: false,
            });
        }
        "radio" => {
            let mut rb = LayoutBox::new(Some(node_id), BoxType::Inline);
            rb.form_field = Some(FormFieldKind::Radio);
            rb.form_checked = dom.attr(node_id, "checked").is_some();
            rb.form_disabled = dom.attr(node_id, "disabled").is_some();
            rb.accent_color = css_accent;
            rb.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 20,
                height: 20,
                layout_box: rb,
                breaks_after: false,
            });
        }
        "submit" | "button" => {
            let label = dom.attr(node_id, "value").unwrap_or("Submit");
            let (bw, _) = measure_text(label, 14, 0, false, false);
            let w = (bw + 24).max(60);
            let mut btn = LayoutBox::new(Some(node_id), BoxType::Inline);
            btn.form_field = Some(FormFieldKind::Submit);
            btn.text = Some(String::from(label));
            btn.bg_color = css_bg;
            btn.color = css_fg;
            btn.accent_color = css_accent;
            btn.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: btn,
                breaks_after: false,
            });
        }
        "reset" => {
            let label = dom.attr(node_id, "value").unwrap_or("Reset");
            let (bw, _) = measure_text(label, 14, 0, false, false);
            let w = (bw + 24).max(60);
            let mut btn = LayoutBox::new(Some(node_id), BoxType::Inline);
            btn.form_field = Some(FormFieldKind::Reset);
            btn.text = Some(String::from(label));
            btn.bg_color = css_bg;
            btn.color = css_fg;
            btn.accent_color = css_accent;
            btn.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: btn,
                breaks_after: false,
            });
        }
        "password" => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Password);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "range" => {
            // HTML5 range slider.
            let min_val: f32 = dom
                .attr(node_id, "min")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0);
            let max_val: f32 = dom
                .attr(node_id, "max")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(100.0);
            let cur_val: f32 = dom
                .attr(node_id, "value")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(50.0);
            let pct = if max_val > min_val {
                ((cur_val - min_val) / (max_val - min_val))
                    .min(1.0)
                    .max(0.0)
            } else {
                0.5
            };

            let w = 200;
            let h = 28;
            // Encode percentage as integer 0..1000 in form_value.
            let pct_i = (pct * 1000.0) as i32;
            let mut val_str = String::new();
            let digits = [
                (b'0' + (pct_i / 100 % 10) as u8) as char,
                (b'0' + (pct_i / 10 % 10) as u8) as char,
                (b'0' + (pct_i % 10) as u8) as char,
            ];
            for &d in &digits {
                val_str.push(d);
            }
            if pct_i >= 1000 {
                val_str.clear();
                val_str.push('X');
            } // 100%
            let mut rng = LayoutBox::new(Some(node_id), BoxType::Inline);
            rng.form_field = Some(FormFieldKind::Range);
            rng.form_value = Some(val_str);
            rng.bg_color = css_bg;
            rng.form_disabled = dom.attr(node_id, "disabled").is_some();
            rng.accent_color = css_accent;
            rng.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: h,
                layout_box: rng,
                breaks_after: false,
            });
        }
        "number" => {
            let w = size_attr_width(dom, node_id, 150);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Number);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_min = dom.attr(node_id, "min").map(String::from);
            tf.form_max = dom.attr(node_id, "max").map(String::from);
            tf.form_step = dom.attr(node_id, "step").map(String::from);
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_readonly = dom.attr(node_id, "readonly").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "color" => {
            let mut cb = LayoutBox::new(Some(node_id), BoxType::Inline);
            cb.form_field = Some(FormFieldKind::Color);
            cb.form_value = dom.attr(node_id, "value").map(String::from);
            cb.form_disabled = dom.attr(node_id, "disabled").is_some();
            cb.bg_color = css_bg;
            cb.accent_color = css_accent;
            cb.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 44,
                height: 28,
                layout_box: cb,
                breaks_after: false,
            });
        }
        "file" => {
            let label = "Choose File";
            let (bw, _) = measure_text(label, 14, 0, false, false);
            let w = (bw + 24).max(120);
            let mut fb = LayoutBox::new(Some(node_id), BoxType::Inline);
            fb.form_field = Some(FormFieldKind::File);
            fb.text = Some(String::from(label));
            fb.form_disabled = dom.attr(node_id, "disabled").is_some();
            fb.bg_color = css_bg;
            fb.color = css_fg;
            fb.accent_color = css_accent;
            fb.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: fb,
                breaks_after: false,
            });
        }
        "date" => {
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Date);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_placeholder = Some(String::from("yyyy-mm-dd"));
            tf.form_min = dom.attr(node_id, "min").map(String::from);
            tf.form_max = dom.attr(node_id, "max").map(String::from);
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 160,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "time" => {
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Time);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_placeholder = Some(String::from("hh:mm"));
            tf.form_min = dom.attr(node_id, "min").map(String::from);
            tf.form_max = dom.attr(node_id, "max").map(String::from);
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 120,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "datetime-local" => {
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::DatetimeLocal);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_placeholder = Some(String::from("yyyy-mm-ddThh:mm"));
            tf.form_min = dom.attr(node_id, "min").map(String::from);
            tf.form_max = dom.attr(node_id, "max").map(String::from);
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 230,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "month" => {
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Month);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_placeholder = Some(String::from("yyyy-mm"));
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 140,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        "week" => {
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::Week);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_placeholder = Some(String::from("yyyy-Www"));
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: 140,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
        _ => {
            let w = size_attr_width(dom, node_id, 200);
            let mut tf = LayoutBox::new(Some(node_id), BoxType::Inline);
            tf.form_field = Some(FormFieldKind::TextInput);
            tf.form_placeholder = dom.attr(node_id, "placeholder").map(String::from);
            tf.form_value = dom.attr(node_id, "value").map(String::from);
            tf.form_disabled = dom.attr(node_id, "disabled").is_some();
            tf.form_readonly = dom.attr(node_id, "readonly").is_some();
            tf.form_required = dom.attr(node_id, "required").is_some();
            tf.form_pattern = dom.attr(node_id, "pattern").map(String::from);
            tf.form_maxlength = dom
                .attr(node_id, "maxlength")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(-1);
            tf.form_minlength = dom
                .attr(node_id, "minlength")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(-1);
            tf.form_datalist = collect_datalist_suggestions(dom, node_id);
            tf.bg_color = css_bg;
            tf.color = css_fg;
            tf.accent_color = css_accent;
            tf.uses_dark_color_scheme = uses_dark_color_scheme;
            out.push(InlineFragment {
                width: w,
                height: 28,
                layout_box: tf,
                breaks_after: false,
            });
        }
    }
}

/// Collect pipe-separated suggestions from a `<datalist>` referenced by the `list` attribute.
/// Returns None if no datalist is found, or Some(pipe_separated_options).
fn collect_datalist_suggestions(dom: &Dom, input_node_id: NodeId) -> Option<String> {
    let list_id = dom.attr(input_node_id, "list")?;
    if list_id.is_empty() {
        return None;
    }
    // Find the <datalist> element with matching id.
    for i in 0..dom.nodes.len() {
        if dom.tag(i) == Some(Tag::Datalist) {
            if dom.attr(i, "id") == Some(list_id) {
                let mut suggestions = String::new();
                let children = &dom.get(i).children;
                for &cid in children {
                    if dom.tag(cid) == Some(Tag::Option) {
                        let val = dom.attr(cid, "value").unwrap_or("");
                        let label = dom.attr(cid, "label").unwrap_or(val);
                        let text = if !val.is_empty() { val } else { label };
                        if !text.is_empty() {
                            if !suggestions.is_empty() {
                                suggestions.push('|');
                            }
                            suggestions.push_str(text);
                        }
                    }
                }
                if !suggestions.is_empty() {
                    return Some(suggestions);
                }
                return None;
            }
        }
    }
    None
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
    let style = &styles[node_id];
    let custom_font_id = style
        .font_family
        .as_ref()
        .and_then(|family| crate::lookup_web_font(family))
        .unwrap_or(0);
    let (bw, _) = measure_text(
        label,
        font_size_px(style),
        custom_font_id,
        is_bold(style),
        is_italic(style),
    );
    let w = (bw + 24).max(60);
    let btn_type = dom.attr(node_id, "type").unwrap_or("submit");
    let kind = match btn_type {
        "submit" => FormFieldKind::Submit,
        "reset" => FormFieldKind::Reset,
        _ => FormFieldKind::ButtonEl,
    };
    let mut btn = LayoutBox::new(Some(node_id), BoxType::Inline);
    btn.form_field = Some(kind);
    btn.text = Some(String::from(label));
    // Apply CSS colors so the renderer can style the button widget.
    if node_id < styles.len() {
        btn.bg_color = styles[node_id].background_color;
        btn.color = styles[node_id].color;
    }
    out.push(InlineFragment {
        width: w,
        height: 28,
        layout_box: btn,
        breaks_after: false,
    });
}

fn button_uses_native_control(dom: &Dom, node_id: NodeId) -> bool {
    let children = &dom.get(node_id).children;
    if children.is_empty() {
        return true;
    }
    let mut saw_nonempty_text = false;
    for &cid in children {
        match &dom.get(cid).node_type {
            crate::dom::NodeType::Text(t) => {
                if !t.trim().is_empty() {
                    saw_nonempty_text = true;
                }
            }
            crate::dom::NodeType::Element { .. } => return false,
        }
    }
    saw_nonempty_text
}

/// Emit word fragments for normal text (collapse whitespace, break on words).
fn emit_word_fragments(
    text: &str,
    font_size: i32,
    custom_font_id: u32,
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
        // Whitespace-only text node (e.g. "\n  " between block siblings).
        // Per CSS §9.2.1 and white-space:normal collapsing rules, such nodes
        // collapse to a single space CHARACTER but must NOT contribute any line
        // height — otherwise every indentation newline between block children of
        // an inline element (like <a-menu>) would add ~19 px of phantom height.
        // We emit a zero-height space so the word-spacing gap is preserved without
        // affecting the line box height calculation.
        if has_leading_space {
            let (sw, _sh) = measure_text(" ", font_size, custom_font_id, bold, italic);
            let mut space_box =
                LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
            space_box.custom_font_id = custom_font_id;
            space_box.link_url = link.clone();
            space_box.text_decoration = deco;
            out.push(InlineFragment {
                width: sw,
                height: 0, // No height: whitespace-only nodes must not set line height
                layout_box: space_box,
                breaks_after: false,
            });
        }
        return;
    }

    if has_leading_space {
        let (sw, sh) = measure_text(" ", font_size, custom_font_id, bold, italic);
        let mut space_box = LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
        space_box.custom_font_id = custom_font_id;
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
        let (ww, wh) = measure_text(word, font_size, custom_font_id, bold, italic);
        // Apply letter-spacing: add extra pixels per character.
        let letter_extra = letter_spacing * (word.len().max(1) as i32 - 1).max(0);
        let mut wbox = LayoutBox::new_text(String::from(*word), font_size, bold, italic, color);
        wbox.custom_font_id = custom_font_id;
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
            let (sw, sh) = measure_text(" ", font_size, custom_font_id, bold, italic);
            // Apply word-spacing: add extra pixels to space between words.
            let mut sbox = LayoutBox::new_text(String::from(" "), font_size, bold, italic, color);
            sbox.custom_font_id = custom_font_id;
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
    custom_font_id: u32,
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
                let (sw, sh) = measure_text(seg, font_size, custom_font_id, bold, italic);
                let mut sbox =
                    LayoutBox::new_text(String::from(seg), font_size, bold, italic, color);
                sbox.custom_font_id = custom_font_id;
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
