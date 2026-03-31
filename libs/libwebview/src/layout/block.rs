//! Block-level layout: `build_block()` builds a block box for a single DOM node.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, Tag};
use crate::style::{
    ComputedStyle, Display, BoxSizing, OverflowVal, Visibility, Position,
    PseudoStyles,
};
use crate::ImageCache;

use super::{
    LayoutBox, BoxType, FormFieldKind,
    font_size_px, is_bold, is_italic, edges_from,
    link_href, list_marker_for, image_dimensions, parse_attr_int,
    layout_children,
};
use super::flex::layout_flex;
use super::grid::layout_grid;

/// Build a block-level layout box for a single DOM node.
///
/// `viewport_w` is the full viewport width, passed down to child layout calls
/// so that `position:fixed` descendants can be positioned correctly.
pub fn build_block(dom: &Dom, styles: &[ComputedStyle], pseudo: &PseudoStyles, node_id: NodeId, available_width: i32, images: &ImageCache, viewport_w: i32) -> LayoutBox {
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
    // Resolve web font ID from font-family.
    if let Some(ref family) = style.font_family {
        if let Some(wf_id) = crate::lookup_web_font(family) {
            bx.custom_font_id = wf_id;
        }
    }
    bx.text_align = style.text_align;
    bx.link_url = link_href(dom, node_id);
    bx.list_marker = list_marker_for(dom, node_id, style);
    bx.overflow_hidden = matches!(style.overflow_x, OverflowVal::Hidden)
        || matches!(style.overflow_y, OverflowVal::Hidden);
    bx.visibility_hidden = matches!(style.visibility, Visibility::Hidden | Visibility::Collapse);
    bx.opacity = style.opacity;
    // Per-side borders (litehtml-style)
    bx.border_top_width = style.border_top.width;
    bx.border_right_width = style.border_right.width;
    bx.border_bottom_width = style.border_bottom.width;
    bx.border_left_width = style.border_left.width;
    bx.border_top_color = style.border_top.color;
    bx.border_right_color = style.border_right.color;
    bx.border_bottom_color = style.border_bottom.color;
    bx.border_left_color = style.border_left.color;
    bx.border_top_left_radius = style.border_top_left_radius;
    bx.border_top_right_radius = style.border_top_right_radius;
    bx.border_bottom_right_radius = style.border_bottom_right_radius;
    bx.border_bottom_left_radius = style.border_bottom_left_radius;
    // Outline
    bx.outline_width = style.outline_width;
    bx.outline_color = style.outline_color;
    bx.outline_offset = style.outline_offset;
    // Shadows
    bx.box_shadows = style.box_shadows.clone();
    bx.text_shadows = style.text_shadows.clone();
    // Text overflow
    bx.text_overflow_ellipsis = matches!(style.text_overflow, crate::style::TextOverflowVal::Ellipsis);
    // Background image
    bx.background_image = style.background_image.clone();
    bx.background_size = style.background_size;
    bx.background_repeat = style.background_repeat;
    // Letter spacing
    bx.letter_spacing = style.letter_spacing;
    // Z-index
    bx.z_index = style.z_index;
    // Per-side border styles
    bx.border_top_style = style.border_top.style;
    bx.border_right_style = style.border_right.style;
    bx.border_bottom_style = style.border_bottom.style;
    bx.border_left_style = style.border_left.style;
    // Filter & clip-path
    bx.filter = style.filter.clone();
    bx.clip_path = style.clip_path.clone();
    // Text decoration sub-properties
    bx.text_decoration_color = style.text_decoration_color;
    bx.text_decoration_style = style.text_decoration_style;
    bx.text_decoration_thickness = style.text_decoration_thickness;
    bx.text_underline_offset = style.text_underline_offset;
    bx.margin = edges_from(
        style.margin_top, style.margin_right,
        style.margin_bottom, style.margin_left,
    );
    bx.padding = edges_from(
        style.padding_top, style.padding_right,
        style.padding_bottom, style.padding_left,
    );

    // ---- Width resolution ----
    let border2 = bx.border_width * 2;
    let is_border_box = matches!(style.box_sizing, BoxSizing::BorderBox);

    // Resolve explicit width (px, percentage, or calc).
    let explicit_w = if let Some(w) = style.width {
        Some(w)
    } else if let Some(pct) = style.width_pct {
        Some((available_width as i64 * pct as i64 / 10000) as i32)
    } else if let Some((px100, pct100)) = style.width_calc {
        // calc(): px component (fixed-100) + pct component (fixed-100) of container width.
        let px_part = px100 / 100;
        let pct_part = (available_width as i64 * pct100 as i64 / 10000) as i32;
        Some(px_part + pct_part)
    } else {
        None
    };

    // Compute outer-box width.
    if let Some(w) = explicit_w {
        if w > 0 {
            if is_border_box {
                bx.width = w;
            } else {
                bx.width = w + bx.padding.left + bx.padding.right + border2;
            }
        } else {
            bx.width = available_width - bx.margin.left - bx.margin.right;
        }
    } else {
        bx.width = available_width - bx.margin.left - bx.margin.right;
    }

    // Apply min-width / max-width.
    let resolve_min_max = |val: i32| -> i32 {
        if val < 0 {
            (available_width as i64 * (-val) as i64 / 10000) as i32
        } else {
            val
        }
    };
    if let Some(mw) = style.max_width {
        let max = resolve_min_max(mw);
        let max_outer = if is_border_box { max } else { max + bx.padding.left + bx.padding.right + border2 };
        if bx.width > max_outer { bx.width = max_outer; }
    }
    if style.min_width > 0 || style.min_width < 0 {
        let min = resolve_min_max(style.min_width);
        let min_outer = if is_border_box { min } else { min + bx.padding.left + bx.padding.right + border2 };
        if bx.width < min_outer { bx.width = min_outer; }
    }

    // Clamp to available space.
    let max_allowed = available_width - bx.margin.left - bx.margin.right;
    if bx.width > max_allowed && max_allowed > 0 {
        bx.width = max_allowed;
    }

    // Handle margin:auto centering.
    if style.margin_left_auto && style.margin_right_auto {
        let remaining = available_width - bx.width;
        if remaining > 0 {
            bx.margin.left = remaining / 2;
            bx.margin.right = remaining - bx.margin.left;
        }
    } else if style.margin_left_auto {
        let remaining = available_width - bx.width - bx.margin.right;
        if remaining > 0 { bx.margin.left = remaining; }
    } else if style.margin_right_auto {
        let remaining = available_width - bx.width - bx.margin.left;
        if remaining > 0 { bx.margin.right = remaining; }
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
        let (iw, ih) = image_dimensions(dom, node_id, bx.width, images);
        bx.image_src = dom.attr(node_id, "src").map(|s| String::from(s));
        bx.image_width = Some(iw);
        bx.image_height = Some(ih);
        bx.object_fit = style.object_fit;
        bx.height = ih + bx.padding.top + bx.padding.bottom + border2;
        bx.width = iw + bx.padding.left + bx.padding.right + border2;
        return bx;
    }

    // Handle inline <svg> as a replaced element: rasterised by surf into the
    // image cache under the synthetic key "__svg_<node_id>__".
    if tag == Some(Tag::Svg) {
        let key = super::svg_inline_key(node_id);
        let natural = images.get_ref(&key).map(|e| {
            (e.width.min(65535) as i32, e.height.min(65535) as i32)
        });
        let w = dom.attr(node_id, "width").and_then(parse_attr_int)
            .or(natural.map(|(w, _)| w)).unwrap_or(100);
        let h = dom.attr(node_id, "height").and_then(parse_attr_int)
            .or(natural.map(|(_, h)| h)).unwrap_or(100);
        let w = w.min(bx.width.max(1));
        bx.image_src = Some(key);
        bx.image_width = Some(w);
        bx.image_height = Some(h);
        bx.object_fit = style.object_fit;
        bx.height = h + bx.padding.top + bx.padding.bottom + border2;
        bx.width = w + bx.padding.left + bx.padding.right + border2;
        return bx;
    }

    // Handle replaced/form elements as flex items or block-level boxes.
    // These have intrinsic sizes that build_block wouldn't otherwise know about.
    if tag == Some(Tag::Input) {
        let input_type = dom.attr(node_id, "type").unwrap_or("text");
        let is_hidden = input_type == "hidden";
        if is_hidden {
            bx.width = 0;
            bx.height = 0;
            return bx;
        }
        let input_h = if let Some(h) = style.height { h } else { 45 };
        bx.height = input_h + bx.padding.top + bx.padding.bottom + border2;
        bx.form_field = Some(FormFieldKind::TextInput);
        bx.form_placeholder = dom.attr(node_id, "placeholder").map(|s| String::from(s));
        bx.form_value = dom.attr(node_id, "value").map(|s| String::from(s));
        return bx;
    }
    if tag == Some(Tag::Button) {
        let btn_h = if let Some(h) = style.height { h } else { 45 };
        bx.height = btn_h + bx.padding.top + bx.padding.bottom + border2;
        // Extract button text from children.
        let children = &dom.get(node_id).children;
        for &cid in children {
            if let crate::dom::NodeType::Text(ref t) = dom.get(cid).node_type {
                bx.text = Some(String::from(t.as_str()));
                break;
            }
        }
        bx.form_field = Some(FormFieldKind::Submit);
        return bx;
    }
    if tag == Some(Tag::Textarea) {
        let cols = dom.attr(node_id, "cols").and_then(|s| s.parse::<i32>().ok()).unwrap_or(20);
        let rows = dom.attr(node_id, "rows").and_then(|s| s.parse::<i32>().ok()).unwrap_or(2);
        let ta_w = if let Some(w) = style.width { w } else { (cols * 8).max(80) };
        let ta_h = if let Some(h) = style.height { h } else { (rows * 18).max(28) };
        bx.width = ta_w + bx.padding.left + bx.padding.right + border2;
        bx.height = ta_h + bx.padding.top + bx.padding.bottom + border2;
        bx.form_field = Some(FormFieldKind::Textarea);
        return bx;
    }

    // Inner (content) width for child layout.
    let inner_w = bx.width - bx.padding.left - bx.padding.right - border2;
    let inner_w = inner_w.max(0);

    // Lay out children — dispatch to flex, grid, or block flow.
    // Inject ::before content as first inline child and ::after as last.
    let children: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();

    let has_before = node_id < pseudo.before.len() && pseudo.before[node_id].is_some();
    let has_after = node_id < pseudo.after.len() && pseudo.after[node_id].is_some();

    let content_h = if matches!(style.display, Display::Flex | Display::InlineFlex) {
        layout_flex(dom, styles, pseudo, &children, inner_w, &mut bx, images, viewport_w)
    } else if matches!(style.display, Display::Grid | Display::InlineGrid) {
        layout_grid(dom, styles, pseudo, &children, inner_w, &mut bx, images, viewport_w)
    } else {
        // Prepend ::before pseudo-element content.
        if has_before {
            let ps = pseudo.before[node_id].as_ref().unwrap();
            if let Some(ref text) = ps.content {
                if !text.is_empty() {
                    let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                    let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                    let italic = matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                    let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                    tb.bg_color = ps.background_color;
                    tb.text_decoration = ps.text_decoration;
                    bx.children.push(tb);
                }
            }
        }

        let h = layout_children(dom, styles, pseudo, &children, inner_w, &mut bx, node_id, images, viewport_w);

        // Append ::after pseudo-element content.
        if has_after {
            let ps = pseudo.after[node_id].as_ref().unwrap();
            if let Some(ref text) = ps.content {
                if !text.is_empty() {
                    let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
                    let bold = matches!(ps.font_weight, crate::style::FontWeight::Bold);
                    let italic = matches!(ps.font_style, crate::style::FontStyleVal::Italic);
                    let mut tb = LayoutBox::new_text(text.clone(), fs, bold, italic, ps.color);
                    tb.bg_color = ps.background_color;
                    tb.text_decoration = ps.text_decoration;
                    bx.children.push(tb);
                }
            }
        }

        h
    };

    // ---- Height resolution ----
    let explicit_h = if let Some(h) = style.height {
        Some(h)
    } else if let Some(pct) = style.height_pct {
        // Percentage heights require a definite parent height.
        // For now, compute against viewport height (approximated as available_width
        // since we don't track parent heights separately). This is imperfect but
        // handles common cases like `height: 100%` on body children.
        if pct > 0 {
            Some((available_width as i64 * pct as i64 / 10000) as i32)
        } else {
            None
        }
    } else if let Some((px100, pct100)) = style.height_calc {
        let px_part = px100 / 100;
        let pct_part = (available_width as i64 * pct100 as i64 / 10000) as i32;
        Some(px_part + pct_part)
    } else {
        None
    };

    if let Some(h) = explicit_h {
        if is_border_box {
            bx.height = h;
        } else {
            bx.height = h + bx.padding.top + bx.padding.bottom + border2;
        }
    } else if style.aspect_ratio > 0 && bx.width > 0 {
        // aspect-ratio: width / height — compute height from width.
        // aspect_ratio is stored as (w/h) * 100, so height = width * 100 / aspect_ratio.
        let content_w = bx.width - bx.padding.left - bx.padding.right - border2;
        let ar_h = content_w * 100 / style.aspect_ratio;
        bx.height = ar_h + bx.padding.top + bx.padding.bottom + border2;
    } else {
        // content_h from layout_children already includes border_width (top) + padding.top.
        // Add padding.bottom + border_width (bottom) to get the full outer height.
        bx.height = content_h + bx.padding.bottom + bx.border_width;
    }

    // Apply min-height / max-height.
    if let Some(mh) = style.max_height {
        let max_h = if is_border_box { mh } else { mh + bx.padding.top + bx.padding.bottom + border2 };
        if bx.height > max_h { bx.height = max_h; }
    }
    if style.min_height > 0 {
        let min_h = if is_border_box { style.min_height } else {
            style.min_height + bx.padding.top + bx.padding.bottom + border2
        };
        if bx.height < min_h { bx.height = min_h; }
    }

    // Apply position:relative offset (does not affect child layout).
    if style.position == Position::Relative {
        if let Some(t) = style.top { bx.y += t; }
        if let Some(l) = style.left_offset { bx.x += l; }
        if style.top.is_none() {
            if let Some(b) = style.bottom_offset { bx.y -= b; }
        }
        if style.left_offset.is_none() {
            if let Some(r) = style.right_offset { bx.x -= r; }
        }
    }

    // position:sticky — in this simplified implementation, elements with
    // both `position:sticky` and a `top` value are treated as `position:fixed`.
    if style.position == Position::Sticky && style.top.is_some() {
        bx.is_sticky = true;
        bx.sticky_top = style.top.unwrap_or(0);
        bx.is_fixed = true;
        bx.y = style.top.unwrap_or(0);
    }

    // Apply CSS transform: translate offsets.
    if style.transform_tx != 0 || style.transform_ty != 0 {
        bx.x += style.transform_tx;
        bx.y += style.transform_ty;
    }

    bx
}
