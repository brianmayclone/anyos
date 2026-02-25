//! Block-level layout: `build_block()` builds a block box for a single DOM node.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, Tag};
use crate::style::{
    ComputedStyle, Display, BoxSizing, OverflowVal, Visibility, Position,
};
use crate::ImageCache;

use super::{
    LayoutBox, BoxType,
    font_size_px, is_bold, is_italic, edges_from,
    link_href, list_marker_for, image_dimensions,
    layout_children,
};
use super::flex::layout_flex;

/// Build a block-level layout box for a single DOM node.
pub fn build_block(dom: &Dom, styles: &[ComputedStyle], node_id: NodeId, available_width: i32, images: &ImageCache) -> LayoutBox {
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
    bx.overflow_hidden = matches!(style.overflow_x, OverflowVal::Hidden)
        || matches!(style.overflow_y, OverflowVal::Hidden);
    bx.visibility_hidden = matches!(style.visibility, Visibility::Hidden | Visibility::Collapse);
    bx.opacity = style.opacity;
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
        bx.height = ih + bx.padding.top + bx.padding.bottom + border2;
        bx.width = iw + bx.padding.left + bx.padding.right + border2;
        return bx;
    }

    // Inner (content) width for child layout.
    let inner_w = bx.width - bx.padding.left - bx.padding.right - border2;
    let inner_w = inner_w.max(0);

    // Lay out children — dispatch to flex or block flow.
    let children: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    let content_h = if matches!(style.display, Display::Flex | Display::InlineFlex) {
        layout_flex(dom, styles, &children, inner_w, &mut bx, images)
    } else {
        layout_children(dom, styles, &children, inner_w, &mut bx, node_id, images)
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
    } else {
        bx.height = content_h + bx.padding.bottom + border2;
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

    bx
}
