//! Block-level layout: `build_block()` builds a block box for a single DOM node.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, Tag};
use crate::style::{
    resolve_inset, resolve_margins, AlignContent, BoxSizing, ComputedStyle, Direction, Display,
    FontStyleVal, FontWeight, ListStylePosition, OverflowVal, Position, PseudoStyles, TextDeco,
    Visibility,
};
use crate::ImageCache;

use super::flex::layout_flex;
use super::grid::layout_grid;
use super::{
    apply_transform_translation, edges_from, font_size_px, image_dimensions, is_bold, is_italic,
    layout_children_ex_with_budget, link_href, list_marker_for, BoxType, FormFieldKind, LayoutBox,
};

/// Build a block-level layout box for a single DOM node.
///
/// `viewport_w` is the full viewport width, passed down to child layout calls
/// so that `position:fixed` descendants can be positioned correctly.
pub fn build_block(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
    // Definite height of the containing block (0 = no definite height / auto).
    // Used to resolve percentage heights. Per CSS spec, if the parent has no
    // definite height, `height: X%` computes to `auto`.
    parent_height: i32,
) -> LayoutBox {
    build_block_with_budget(
        dom,
        styles,
        pseudo,
        node_id,
        available_width,
        images,
        viewport_w,
        parent_height,
        0,
        None,
    )
}

pub fn build_block_with_budget(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
    parent_height: i32,
    abs_y: i32,
    layout_budget_bottom: Option<i32>,
) -> LayoutBox {
    build_block_internal(
        dom,
        styles,
        pseudo,
        node_id,
        available_width,
        images,
        viewport_w,
        parent_height,
        abs_y,
        layout_budget_bottom,
        None,
    )
}

pub(super) fn build_block_with_forced_outer_height(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
    parent_height: i32,
    forced_outer_height: i32,
) -> LayoutBox {
    build_block_internal(
        dom,
        styles,
        pseudo,
        node_id,
        available_width,
        images,
        viewport_w,
        parent_height,
        0,
        None,
        Some(forced_outer_height),
    )
}

fn build_block_internal(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
    parent_height: i32,
    abs_y: i32,
    layout_budget_bottom: Option<i32>,
    forced_outer_height: Option<i32>,
) -> LayoutBox {
    let style = &styles[node_id];
    let tag = dom.tag(node_id);

    let mut bx = LayoutBox::new(Some(node_id), BoxType::Block);
    bx.color = style.color;
    bx.bg_color = if style.background_color_is_current {
        style.color
    } else {
        style.background_color
    };
    bx.accent_color = style.accent_color;
    bx.uses_dark_color_scheme = style.color_scheme == crate::style::ColorSchemeVal::Dark;
    bx.appearance_none = style.appearance == crate::style::AppearanceVal::None;
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
    bx.list_marker_inside = style.list_style_position == ListStylePosition::Inside;
    // Overflow clipping: `hidden` and `scroll` always clip.
    // `auto` clips too (a scrollbar would hide overflow; since we don't render
    // scrollbars, we clip to prevent content from overlapping other boxes).
    bx.overflow_hidden = !matches!(style.overflow_x, OverflowVal::Visible)
        || !matches!(style.overflow_y, OverflowVal::Visible);
    bx.visibility_hidden = matches!(style.visibility, Visibility::Hidden | Visibility::Collapse);
    bx.opacity = style.opacity;
    bx.backdrop_filter_blur = style.backdrop_filter.blur_px;
    bx.is_positioned = style.position != Position::Static;
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
    bx.text_overflow_ellipsis =
        matches!(style.text_overflow, crate::style::TextOverflowVal::Ellipsis);
    // Background image
    bx.background_image = style.background_image.clone();
    bx.mask_image = style.mask_image.clone();
    bx.background_size = style.background_size;
    bx.background_repeat = style.background_repeat;
    bx.background_clip = style.background_clip;
    bx.background_position_x = style.background_position_x;
    bx.background_position_y = style.background_position_y;
    bx.mask_size = style.mask_size;
    bx.mask_repeat = style.mask_repeat;
    bx.mask_clip = style.mask_clip;
    bx.mask_origin = style.mask_origin;
    bx.mask_position_x = style.mask_position_x;
    bx.mask_position_x_is_percent = style.mask_position_x_is_percent;
    bx.mask_position_y = style.mask_position_y;
    bx.mask_position_y_is_percent = style.mask_position_y_is_percent;
    // Letter spacing
    bx.letter_spacing = style.letter_spacing;
    // Z-index — only applies to positioned elements (CSS2 §9.9.1).
    // Elements with `position: static` ignore z-index.
    if style.position != Position::Static {
        bx.z_index = style.z_index;
        bx.z_index_auto = style.z_index_auto;
    }
    // Stacking context creation (CSS2 §9.9.1, CSS3 Compositing):
    // - Positioned elements with explicit z-index (not auto) — includes z-index: 0
    // - Elements with opacity < 1
    // - Elements with CSS transform
    bx.creates_stacking_context = (style.position != Position::Static && !style.z_index_auto)
        || style.opacity < 255
        || style.transform_tx != 0
        || style.transform_ty != 0
        || style.transform_tx_pct != 0
        || style.transform_ty_pct != 0
        || style.transform_sx != 1000
        || style.transform_sy != 1000
        || style.transform_rotate != 0;
    // Per-side border styles
    bx.border_top_style = style.border_top.style;
    bx.border_right_style = style.border_right.style;
    bx.border_bottom_style = style.border_bottom.style;
    bx.border_left_style = style.border_left.style;
    // Filter & clip-path
    bx.filter = style.filter.clone();
    bx.clip_path = style.clip_path.clone();
    bx.clip_rect = style.clip_rect;
    // Text decoration sub-properties
    bx.text_decoration_color = style.text_decoration_color;
    bx.text_decoration_style = style.text_decoration_style;
    bx.text_decoration_thickness = style.text_decoration_thickness;
    bx.text_underline_offset = style.text_underline_offset;
    let (margin_top, margin_right, margin_bottom, margin_left) =
        resolve_margins(style, available_width);
    bx.margin = edges_from(margin_top, margin_right, margin_bottom, margin_left);
    let padding_pct_basis = if style.writing_mode.is_vertical() && parent_height > 0 {
        parent_height
    } else {
        available_width
    }
    .max(0);
    let resolve_padding = |px: i32, pct: Option<i32>| -> i32 {
        pct.map(|v| (padding_pct_basis as i64 * v as i64 / 10000) as i32)
            .unwrap_or(px)
    };
    bx.padding = edges_from(
        resolve_padding(style.padding_top, style.padding_top_pct),
        resolve_padding(style.padding_right, style.padding_right_pct),
        resolve_padding(style.padding_bottom, style.padding_bottom_pct),
        resolve_padding(style.padding_left, style.padding_left_pct),
    );

    // ---- Width resolution ----
    let horizontal_border = bx.border_left_width + bx.border_right_width;
    let vertical_border = bx.border_top_width + bx.border_bottom_width;
    let is_border_box = matches!(style.box_sizing, BoxSizing::BorderBox);
    let vertical_non_content = bx.padding.top + bx.padding.bottom + vertical_border;
    let definite_h_for_aspect = if let Some(h) = style.height {
        Some(h)
    } else if let Some(pct) = style.height_pct {
        if pct > 0 && parent_height > 0 {
            Some((parent_height as i64 * pct as i64 / 10000) as i32)
        } else {
            None
        }
    } else if let Some((px100, pct100)) = style.height_calc {
        let px_part = px100 / 100;
        let pct_part = if parent_height > 0 {
            (parent_height as i64 * pct100 as i64 / 10000) as i32
        } else {
            0
        };
        Some(px_part + pct_part)
    } else {
        None
    };

    // max-content / min-content / fit-content: measure intrinsic width.
    let intrinsic_w: Option<i32> = if style.width_max_content {
        let content_w = super::intrinsic_width(
            dom,
            styles,
            pseudo,
            node_id,
            available_width,
            images,
            viewport_w,
        );
        let pad_border = bx.padding.left + bx.padding.right + horizontal_border;
        Some(if is_border_box {
            content_w + pad_border
        } else {
            content_w + pad_border
        })
    } else if style.width_min_content {
        // min-content: use the minimum (longest unbreakable word).
        let content_w =
            super::intrinsic_min_width(dom, styles, pseudo, node_id, images, viewport_w);
        let pad_border = bx.padding.left + bx.padding.right + horizontal_border;
        Some(content_w + pad_border)
    } else if style.width_fit_content {
        // fit-content: min(max-content, max(min-content, available)).
        let max_w = super::intrinsic_width(
            dom,
            styles,
            pseudo,
            node_id,
            available_width,
            images,
            viewport_w,
        );
        let min_w = super::intrinsic_min_width(dom, styles, pseudo, node_id, images, viewport_w);
        let avail = available_width - bx.margin.left - bx.margin.right;
        let pad_border = bx.padding.left + bx.padding.right + horizontal_border;
        Some(max_w.min(avail - pad_border).max(min_w) + pad_border)
    } else {
        None
    };

    // Resolve explicit width (px, percentage, or calc).
    let explicit_w = if let Some(w) = intrinsic_w {
        Some(w)
    } else if let Some(w) = style.width {
        Some(w)
    } else if let Some(pct) = style.width_pct {
        Some((available_width as i64 * pct as i64 / 10000) as i32)
    } else if let Some((px100, pct100)) = style.width_calc {
        // calc(): px component (fixed-100) + pct component (fixed-100) of container width.
        let px_part = px100 / 100;
        let pct_part = (available_width as i64 * pct100 as i64 / 10000) as i32;
        Some(px_part + pct_part)
    } else if style.aspect_ratio > 0 {
        definite_h_for_aspect.map(|h| {
            let content_h = if is_border_box {
                (h - bx.padding.top - bx.padding.bottom - vertical_border).max(0)
            } else {
                h.max(0)
            };
            let content_w = content_h * style.aspect_ratio / 100;
            content_w + bx.padding.left + bx.padding.right + horizontal_border
        })
    } else {
        None
    };

    // Compute outer-box width.
    if let Some(w) = explicit_w {
        // Intrinsic widths are already full outer widths (content + padding + border).
        if intrinsic_w.is_some() {
            bx.width = w.max(0);
        } else if w >= 0 {
            if is_border_box {
                bx.width = w;
            } else {
                bx.width = w + bx.padding.left + bx.padding.right + horizontal_border;
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
    let resolve_min_max_calc = |calc: (i32, i32)| -> i32 {
        calc.0 / 100 + (available_width as i64 * calc.1 as i64 / 10000) as i32
    };
    if let Some(mw) = style
        .max_width
        .or_else(|| style.max_width_calc.map(resolve_min_max_calc))
    {
        let max = resolve_min_max(mw);
        let max_outer = if is_border_box {
            max
        } else {
            max + bx.padding.left + bx.padding.right + horizontal_border
        };
        if bx.width > max_outer {
            bx.width = max_outer;
        }
    }
    let min_width_val = if let Some(calc) = style.min_width_calc {
        resolve_min_max_calc(calc)
    } else {
        style.min_width
    };
    if min_width_val > 0 || min_width_val < 0 {
        let min = resolve_min_max(min_width_val);
        let min_outer = if is_border_box {
            min
        } else {
            min + bx.padding.left + bx.padding.right + horizontal_border
        };
        if bx.width < min_outer {
            bx.width = min_outer;
        }
    }

    // Clamp to available space.
    let max_allowed = available_width - bx.margin.left - bx.margin.right;
    let preserve_explicit_width = explicit_w.is_some();
    if bx.width > max_allowed && max_allowed > 0 && !preserve_explicit_width {
        bx.width = max_allowed;
    }

    // Handle margin:auto for in-flow boxes. Absolutely/fixed positioned boxes
    // resolve auto margins later together with inset constraints.
    if !matches!(style.position, Position::Absolute | Position::Fixed) {
        let remaining = available_width - bx.width - bx.margin.left - bx.margin.right;
        if style.margin_left_auto && style.margin_right_auto {
            if remaining >= 0 {
                bx.margin.left = remaining / 2;
                bx.margin.right = remaining - bx.margin.left;
            } else if matches!(style.direction, Direction::Rtl) {
                // CSS2.1 §10.3.3: in over-constrained RTL blocks, keep the
                // inline-end auto margin at 0 and push the inline-start side.
                bx.margin.right = 0;
                bx.margin.left = remaining;
            } else {
                // CSS2.1 §10.3.3: in over-constrained LTR blocks, keep the
                // inline-start auto margin at 0 and let the end side go negative.
                bx.margin.left = 0;
                bx.margin.right = remaining;
            }
        } else if style.margin_left_auto {
            if remaining >= 0 {
                bx.margin.left = remaining;
            } else if matches!(style.direction, Direction::Rtl) {
                bx.margin.left = remaining;
                bx.margin.right = 0;
            } else {
                bx.margin.left = 0;
                bx.margin.right += remaining;
            }
        } else if style.margin_right_auto {
            if remaining >= 0 {
                bx.margin.right = remaining;
            } else if matches!(style.direction, Direction::Rtl) {
                bx.margin.right = 0;
                bx.margin.left += remaining;
            } else {
                bx.margin.right = remaining;
            }
        }
    }

    // Handle <hr> specifically.
    if tag == Some(Tag::Hr) {
        bx.is_hr = true;
        bx.height = 1 + bx.padding.top + bx.padding.bottom + vertical_border;
        if bx.margin.top == 0 && bx.margin.bottom == 0 {
            bx.margin.top = 8;
            bx.margin.bottom = 8;
        }
        return bx;
    }

    // Handle <img> as block/inline-block replaced element.
    if tag == Some(Tag::Img) || dom.has_tag_name(node_id, "a-img") {
        let (natural_w, natural_h) = image_dimensions(dom, node_id, available_width, images);
        let mut content_w = natural_w.max(1);
        let mut content_h = natural_h.max(1);
        let horizontal_non_content = bx.padding.left + bx.padding.right + horizontal_border;
        let vertical_non_content = bx.padding.top + bx.padding.bottom + vertical_border;

        let resolve_specified_width = |w: i32| {
            if is_border_box {
                (w - horizontal_non_content).max(0)
            } else {
                w.max(0)
            }
        };
        let resolve_specified_height = |h: i32| {
            if is_border_box {
                (h - vertical_non_content).max(0)
            } else {
                h.max(0)
            }
        };

        let specified_w = style
            .width
            .map(resolve_specified_width)
            .or_else(|| {
                style.width_pct.map(|pct| {
                    let border_box = (available_width as i64 * pct as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            })
            .or_else(|| {
                style.width_calc.map(|(px100, pct100)| {
                    let border_box =
                        px100 / 100 + (available_width as i64 * pct100 as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            });
        let specified_h = style
            .height
            .map(resolve_specified_height)
            .or_else(|| {
                if parent_height > 0 {
                    style.height_pct.map(|pct| {
                        let border_box = (parent_height as i64 * pct as i64 / 10000) as i32;
                        resolve_specified_height(border_box)
                    })
                } else {
                    None
                }
            })
            .or_else(|| {
                if parent_height > 0 {
                    style.height_calc.map(|(px100, pct100)| {
                        let border_box =
                            px100 / 100 + (parent_height as i64 * pct100 as i64 / 10000) as i32;
                        resolve_specified_height(border_box)
                    })
                } else {
                    None
                }
            });
        let resolved_max_h = style.max_height.map(resolve_specified_height).or_else(|| {
            if parent_height > 0 {
                style.max_height_calc.map(|(px100, pct100)| {
                    let border_box =
                        px100 / 100 + (parent_height as i64 * pct100 as i64 / 10000) as i32;
                    resolve_specified_height(border_box)
                })
            } else {
                style
                    .max_height_calc
                    .map(|(px100, _)| resolve_specified_height(px100 / 100))
            }
        });
        let resolved_max_w = style
            .max_width
            .map(|value| {
                let border_box = if value < 0 {
                    (available_width.max(0) as i64 * (-value) as i64 / 10000) as i32
                } else {
                    value
                };
                resolve_specified_width(border_box)
            })
            .or_else(|| {
                style.max_width_calc.map(|(px100, pct100)| {
                    let border_box = px100 / 100
                        + (available_width.max(0) as i64 * pct100 as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            });

        match (specified_w, specified_h) {
            (Some(w), Some(h)) => {
                content_w = w.max(1);
                content_h = h.max(1);
            }
            (Some(w), None) => {
                content_w = w.max(1);
                if natural_w > 0 {
                    content_h =
                        ((natural_h as i64 * content_w as i64) / natural_w as i64).max(1) as i32;
                }
            }
            (None, Some(h)) => {
                content_h = h.max(1);
                if natural_h > 0 {
                    content_w =
                        ((natural_w as i64 * content_h as i64) / natural_h as i64).max(1) as i32;
                }
            }
            (None, None) => {}
        }

        if let Some(max_h) = resolved_max_h {
            if content_h > max_h.max(0) {
                content_h = max_h.max(0);
                if natural_h > 0 {
                    content_w =
                        ((natural_w as i64 * content_h as i64) / natural_h as i64).max(1) as i32;
                }
            }
        }
        if let Some(max_w) = resolved_max_w {
            if content_w > max_w.max(0) {
                content_w = max_w.max(0);
                if natural_w > 0 {
                    content_h =
                        ((natural_h as i64 * content_w as i64) / natural_w as i64).max(1) as i32;
                }
            }
        }

        bx.image_src = dom.image_url(node_id);
        bx.image_width = Some(content_w);
        bx.image_height = Some(content_h);
        bx.object_fit = style.object_fit;
        bx.object_position_x = style.object_position_x;
        bx.object_position_x_is_percent = style.object_position_x_is_percent;
        bx.object_position_y = style.object_position_y;
        bx.object_position_y_is_percent = style.object_position_y_is_percent;
        bx.height = content_h + bx.padding.top + bx.padding.bottom + vertical_border;
        bx.width = content_w + bx.padding.left + bx.padding.right + horizontal_border;
        return bx;
    }

    // Handle inline <svg> as a replaced element: rasterised by surf into the
    // image cache under the synthetic key "__svg_<node_id>__".
    if tag == Some(Tag::Svg) {
        let key = super::svg_inline_key(node_id);
        let (w, h) = super::svg_intrinsic_dimensions(dom, images, node_id);
        let mut content_w = w.min(available_width.max(1));
        let mut content_h = if w > 0 && content_w < w {
            ((h as i64 * content_w as i64) / w as i64).max(1) as i32
        } else {
            h.max(1)
        };
        let horizontal_non_content = bx.padding.left + bx.padding.right + horizontal_border;
        let vertical_non_content = bx.padding.top + bx.padding.bottom + vertical_border;
        let resolve_specified_width = |value: i32| {
            if is_border_box {
                (value - horizontal_non_content).max(0)
            } else {
                value.max(0)
            }
        };
        let resolve_specified_height = |value: i32| {
            if is_border_box {
                (value - vertical_non_content).max(0)
            } else {
                value.max(0)
            }
        };
        let specified_w = style
            .width
            .map(resolve_specified_width)
            .or_else(|| {
                style.width_pct.map(|pct| {
                    let border_box = (available_width.max(0) as i64 * pct as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            })
            .or_else(|| {
                style.width_calc.map(|(px100, pct100)| {
                    let border_box = px100 / 100
                        + (available_width.max(0) as i64 * pct100 as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            });
        let specified_h = style.height.map(resolve_specified_height).or_else(|| {
            style
                .height_calc
                .map(|(px100, _)| resolve_specified_height(px100 / 100))
        });
        match (specified_w, specified_h) {
            (Some(spec_w), Some(spec_h)) => {
                content_w = spec_w.max(1);
                content_h = spec_h.max(1);
            }
            (Some(spec_w), None) => {
                content_w = spec_w.max(1);
                if w > 0 {
                    content_h = ((h as i64 * content_w as i64) / w as i64).max(1) as i32;
                }
            }
            (None, Some(spec_h)) => {
                content_h = spec_h.max(1);
                if h > 0 {
                    content_w = ((w as i64 * content_h as i64) / h as i64).max(1) as i32;
                }
            }
            (None, None) => {}
        }
        let resolved_max_h = style.max_height.map(resolve_specified_height).or_else(|| {
            style
                .max_height_calc
                .map(|(px100, _)| resolve_specified_height(px100 / 100))
        });
        let resolved_max_w = style
            .max_width
            .map(|value| {
                let border_box = if value < 0 {
                    (available_width.max(0) as i64 * (-value) as i64 / 10000) as i32
                } else {
                    value
                };
                resolve_specified_width(border_box)
            })
            .or_else(|| {
                style.max_width_calc.map(|(px100, pct100)| {
                    let border_box = px100 / 100
                        + (available_width.max(0) as i64 * pct100 as i64 / 10000) as i32;
                    resolve_specified_width(border_box)
                })
            });
        if let Some(max_h) = resolved_max_h {
            if content_h > max_h.max(0) {
                content_h = max_h.max(0);
                if h > 0 {
                    content_w = ((w as i64 * content_h as i64) / h as i64).max(1) as i32;
                }
            }
        }
        if let Some(max_w) = resolved_max_w {
            if content_w > max_w.max(0) {
                content_w = max_w.max(0);
                if w > 0 {
                    content_h = ((h as i64 * content_w as i64) / w as i64).max(1) as i32;
                }
            }
        }
        bx.image_src = Some(key);
        bx.image_width = Some(content_w);
        bx.image_height = Some(content_h);
        bx.object_fit = style.object_fit;
        bx.object_position_x = style.object_position_x;
        bx.object_position_x_is_percent = style.object_position_x_is_percent;
        bx.object_position_y = style.object_position_y;
        bx.object_position_y_is_percent = style.object_position_y_is_percent;
        bx.height = content_h + bx.padding.top + bx.padding.bottom + vertical_border;
        bx.width = content_w + bx.padding.left + bx.padding.right + horizontal_border;
        return bx;
    }

    // Handle replaced/form elements as flex items or block-level boxes.
    // These have intrinsic sizes that build_block wouldn't otherwise know about.
    if tag == Some(Tag::Input) {
        let input_type = dom.attr(node_id, "type").unwrap_or("text");
        let input_type_lower = input_type.to_ascii_lowercase();
        let input_type = input_type_lower.as_str();
        if input_type == "hidden" {
            bx.width = 0;
            bx.height = 0;
            return bx;
        }
        match input_type {
            "checkbox" => {
                let sz = if let Some(h) = style.height { h } else { 16 };
                bx.width = if let Some(w) = style.width { w } else { sz };
                bx.height = sz + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(FormFieldKind::Checkbox);
                bx.form_checked = dom.attr(node_id, "checked").is_some();
                bx.form_disabled = dom.attr(node_id, "disabled").is_some();
            }
            "radio" => {
                let sz = if let Some(h) = style.height { h } else { 16 };
                bx.width = if let Some(w) = style.width { w } else { sz };
                bx.height = sz + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(FormFieldKind::Radio);
                bx.form_checked = dom.attr(node_id, "checked").is_some();
                bx.form_disabled = dom.attr(node_id, "disabled").is_some();
            }
            "submit" | "button" => {
                let input_h = if let Some(h) = style.height { h } else { 30 };
                bx.height = input_h + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(if input_type == "button" {
                    FormFieldKind::ButtonEl
                } else {
                    FormFieldKind::Submit
                });
                bx.form_value = dom.attr(node_id, "value").map(|s| String::from(s));
                bx.text = bx
                    .form_value
                    .clone()
                    .or_else(|| Some(String::from("Submit")));
            }
            "reset" => {
                let input_h = if let Some(h) = style.height { h } else { 30 };
                bx.height = input_h + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(FormFieldKind::Reset);
                bx.form_value = dom.attr(node_id, "value").map(|s| String::from(s));
                bx.text = bx
                    .form_value
                    .clone()
                    .or_else(|| Some(String::from("Reset")));
            }
            "password" => {
                let input_h = if let Some(h) = style.height { h } else { 28 };
                bx.height = input_h + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(FormFieldKind::Password);
                bx.form_placeholder = dom.attr(node_id, "placeholder").map(|s| String::from(s));
            }
            "range" => {
                let input_h = if let Some(h) = style.height { h } else { 28 };
                bx.width = if let Some(w) = style.width { w } else { 200 };
                bx.height = input_h + bx.padding.top + bx.padding.bottom + vertical_border;
                bx.form_field = Some(FormFieldKind::Range);
                bx.form_disabled = dom.attr(node_id, "disabled").is_some();
                // Compute percentage and encode as 0..1000 in form_value.
                let min_v: f32 = dom
                    .attr(node_id, "min")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let max_v: f32 = dom
                    .attr(node_id, "max")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(100.0);
                let cur_v: f32 = dom
                    .attr(node_id, "value")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(50.0);
                let pct = if max_v > min_v {
                    ((cur_v - min_v) / (max_v - min_v)).min(1.0).max(0.0)
                } else {
                    0.5
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
                bx.form_value = Some(val_str);
            }
            _ => {
                let input_h = if let Some(h) = style.height { h } else { 28 };
                bx.height = input_h + bx.padding.top + bx.padding.bottom + vertical_border;
                let kind = match input_type {
                    "number" => FormFieldKind::Number,
                    "color" => FormFieldKind::Color,
                    "file" => FormFieldKind::File,
                    "date" => FormFieldKind::Date,
                    "time" => FormFieldKind::Time,
                    "datetime-local" => FormFieldKind::DatetimeLocal,
                    "month" => FormFieldKind::Month,
                    "week" => FormFieldKind::Week,
                    _ => FormFieldKind::TextInput,
                };
                bx.form_field = Some(kind);
                bx.form_is_search = input_type == "search";
                bx.form_placeholder = dom.attr(node_id, "placeholder").map(|s| String::from(s));
                bx.form_value = dom.attr(node_id, "value").map(|s| String::from(s));
                bx.form_disabled = dom.attr(node_id, "disabled").is_some();
                bx.form_readonly = dom.attr(node_id, "readonly").is_some();
                bx.form_required = dom.attr(node_id, "required").is_some();
            }
        }
        return bx;
    }
    if tag == Some(Tag::Button) && button_uses_native_control(dom, node_id) {
        let btn_h = if let Some(h) = style.height { h } else { 45 };
        bx.height = btn_h + bx.padding.top + bx.padding.bottom + vertical_border;
        // Extract button text from children.
        let text = dom.visible_text_content(node_id);
        let label = text.trim();
        if !label.is_empty() {
            bx.text = Some(String::from(label));
        }
        let btn_type = dom.attr(node_id, "type").unwrap_or("submit");
        bx.form_field = Some(match btn_type {
            "button" => FormFieldKind::ButtonEl,
            "reset" => FormFieldKind::Reset,
            _ => FormFieldKind::Submit,
        });
        return bx;
    }
    if tag == Some(Tag::Textarea) {
        let cols = dom
            .attr(node_id, "cols")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(20);
        let rows = dom
            .attr(node_id, "rows")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(2);
        let ta_w = if let Some(w) = style.width {
            w
        } else {
            (cols * 8).max(80)
        };
        let ta_h = if let Some(h) = style.height {
            h
        } else {
            (rows * 18).max(28)
        };
        bx.width = ta_w + bx.padding.left + bx.padding.right + horizontal_border;
        bx.height = ta_h + bx.padding.top + bx.padding.bottom + vertical_border;
        bx.form_field = Some(FormFieldKind::Textarea);
        return bx;
    }

    // list-style-position: inside — reserve MARKER_W px at the start of the
    // content area so that inline children begin after the marker.
    // The renderer compensates by drawing the marker at padding.left - MARKER_W.
    const MARKER_W: i32 = 20;
    if bx.list_marker_inside && bx.list_marker.is_some() {
        bx.padding.left += MARKER_W;
    }

    // Inner (content) width for child layout.
    let inner_w = bx.width - bx.padding.left - bx.padding.right - horizontal_border;
    let inner_w = inner_w.max(0);
    let resolve_height_calc = |calc: (i32, i32)| -> i32 {
        calc.0 / 100 + (parent_height.max(0) as i64 * calc.1 as i64 / 10000) as i32
    };
    let explicit_outer_height_hint = if let Some(h) = forced_outer_height {
        Some(h.max(0))
    } else if let Some(h) = style.height {
        Some(if is_border_box {
            h
        } else {
            h + vertical_non_content
        })
    } else if let Some(pct) = style.height_pct {
        if pct > 0 && parent_height > 0 {
            let resolved_h = (parent_height as i64 * pct as i64 / 10000) as i32;
            Some(if is_border_box {
                resolved_h
            } else {
                resolved_h + vertical_non_content
            })
        } else {
            None
        }
    } else if let Some(calc) = style.height_calc {
        let resolved_h = resolve_height_calc(calc);
        Some(if is_border_box {
            resolved_h
        } else {
            resolved_h + vertical_non_content
        })
    } else if style.aspect_ratio > 0 && inner_w > 0 {
        // CSS Sizing 4 §2.4: when the box has a definite inline size and
        // `aspect-ratio` is set, its block size is computed from the ratio.
        // This makes the height definite for the purpose of resolving
        // percentage heights on descendants (and abs-positioned descendants
        // resolving their containing block height against the padding box).
        let ar_h = inner_w * 100 / style.aspect_ratio;
        Some(ar_h + vertical_non_content)
    } else {
        None
    };
    let definite_parent_content_h = explicit_outer_height_hint.map(|mut outer_h| {
        if let Some(mh) = style
            .max_height
            .or_else(|| style.max_height_calc.map(resolve_height_calc))
        {
            let max_outer = if is_border_box {
                mh
            } else {
                mh + vertical_non_content
            };
            if outer_h > max_outer {
                outer_h = max_outer;
            }
        }
        let min_height_val = if let Some(calc) = style.min_height_calc {
            resolve_height_calc(calc)
        } else {
            style.min_height
        };
        if min_height_val > 0 {
            let min_outer = if is_border_box {
                min_height_val
            } else {
                min_height_val + vertical_non_content
            };
            if outer_h < min_outer {
                outer_h = min_outer;
            }
        }
        if is_border_box {
            (outer_h - vertical_non_content).max(0)
        } else {
            outer_h.max(0)
        }
    });
    // Lay out children — dispatch to flex, grid, or block flow.
    // Inject ::before / ::after block-level pseudo-element boxes.
    // Inline pseudo-elements are injected into the inline run by layout_children / layout_inline_content.
    let children: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();

    // Determine which pseudo-elements exist and whether they are block-level.
    let before_is_block = node_id < pseudo.before.len()
        && pseudo.before[node_id]
            .as_ref()
            .map(|ps| is_block_pseudo(ps))
            .unwrap_or(false);
    let after_is_block = node_id < pseudo.after.len()
        && pseudo.after[node_id]
            .as_ref()
            .map(|ps| is_block_pseudo(ps))
            .unwrap_or(false);

    let content_h = if matches!(style.display, Display::Flex | Display::InlineFlex) {
        // Flex containers: inject block pseudo-elements as first/last flex children.
        if before_is_block {
            if let Some(pb) = build_pseudo_element_box(
                pseudo.before[node_id].as_ref().unwrap(),
                inner_w,
                images,
                viewport_w,
            ) {
                bx.children.push(pb);
            }
        }
        let fh = layout_flex(
            dom,
            styles,
            pseudo,
            &children,
            inner_w,
            parent_height,
            &mut bx,
            images,
            viewport_w,
            definite_parent_content_h,
        );
        if after_is_block {
            if let Some(pb) = build_pseudo_element_box(
                pseudo.after[node_id].as_ref().unwrap(),
                inner_w,
                images,
                viewport_w,
            ) {
                bx.children.push(pb);
            }
        }
        fh
    } else if matches!(style.display, Display::Grid | Display::InlineGrid) {
        layout_grid(
            dom, styles, pseudo, &children, inner_w, &mut bx, images, viewport_w, None, None, None,
            None,
        )
    } else {
        // Block containers: block-level pseudo-elements go into flow via layout_children_ex.
        // This ensures ::before and ::after are properly placed within the normal flow,
        // accounting for their heights in the cursor_y progression.
        let before_box = if before_is_block {
            build_pseudo_element_box(
                pseudo.before[node_id].as_ref().unwrap(),
                inner_w,
                images,
                viewport_w,
            )
        } else {
            None
        };
        let after_box = if after_is_block {
            build_pseudo_element_box(
                pseudo.after[node_id].as_ref().unwrap(),
                inner_w,
                images,
                viewport_w,
            )
        } else {
            None
        };

        let ch = layout_children_ex_with_budget(
            dom,
            styles,
            pseudo,
            &children,
            inner_w,
            &mut bx,
            node_id,
            images,
            viewport_w,
            parent_height,
            definite_parent_content_h.unwrap_or(0),
            before_box,
            after_box,
            abs_y,
            layout_budget_bottom,
        );

        // ── Parent-child top margin collapse (CSS §8.3.1) ──────────────────────
        // If this block has no border-top and no padding-top and is not a BFC
        // (overflow: visible), its first in-flow child's top margin "escapes"
        // upward and collapses with the parent's own margin-top.
        // We implement this by:
        //   1. Finding the first non-OOF child's top margin (= its y offset from 0).
        //   2. Collapsing it into bx.margin.top.
        //   3. Shifting all children y by -first_margin so the first child is flush.
        // CSS §10.6.7: A block that establishes a new BFC does NOT collapse
        // margins with its children. BFC is established by:
        // - overflow != visible (CSS2 §9.4.1)
        // - display: flow-root / inline-block / flex / inline-flex / grid / inline-grid
        // - floated elements (float != none)
        // - absolutely/fixed positioned elements
        let is_bfc = !matches!(style.overflow_x, OverflowVal::Visible)
            || !matches!(style.overflow_y, OverflowVal::Visible)
            || matches!(
                style.display,
                Display::InlineBlock
                    | Display::FlowRoot
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid
            )
            || style.float != crate::style::FloatVal::None
            || matches!(style.position, Position::Absolute | Position::Fixed)
            || (!style.align_content_is_normal
                && !matches!(
                    style.display,
                    Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
                ));
        if !is_bfc && bx.border_width == 0 && bx.padding.top == 0 && style.border_top.width == 0 {
            // Find first in-flow child — its y == its margin.top (since cursor_y was 0).
            if let Some(first_child) = bx.children.iter().find(|c| !c.is_out_of_flow) {
                let first_margin = first_child.y; // y == 0 + margin.top at layout start
                if first_margin > 0 {
                    // Collapse: add first child's margin into parent's own margin.
                    if first_margin > bx.margin.top {
                        bx.margin.top = first_margin;
                    }
                    // Shift all children up by first_margin so child starts at y=0.
                    for child in &mut bx.children {
                        child.y -= first_margin;
                    }
                    // Reduce the content height accordingly.
                    ch - first_margin
                } else {
                    ch
                }
            } else {
                ch
            }
        } else {
            ch
        }
    };

    // ---- Height resolution ----
    let explicit_h = if let Some(h) = style.height {
        Some(h)
    } else if let Some(pct) = style.height_pct {
        // Percentage heights require a definite parent height (CSS spec §10.5).
        // If parent_height == 0 (no definite height), treat as `auto`.
        if pct > 0 && parent_height > 0 {
            Some((parent_height as i64 * pct as i64 / 10000) as i32)
        } else {
            None
        }
    } else if let Some((px100, pct100)) = style.height_calc {
        let px_part = px100 / 100;
        let pct_part = if parent_height > 0 {
            (parent_height as i64 * pct100 as i64 / 10000) as i32
        } else {
            0
        };
        Some(px_part + pct_part)
    } else if matches!(style.position, Position::Absolute | Position::Fixed)
        && style.top.is_some()
        && style.bottom_offset.is_some()
        && parent_height > 0
    {
        // CSS §10.6.4: absolute/fixed with both top and bottom and height:auto
        // → height = cb_height - top - bottom.
        let t = style.top.unwrap_or(0);
        let b = style.bottom_offset.unwrap_or(0);
        let h = (parent_height - t - b).max(0);
        if h > 0 {
            Some(h)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(h) = forced_outer_height {
        bx.height = h.max(0);
    } else if let Some(h) = explicit_h {
        if is_border_box {
            bx.height = h;
        } else {
            bx.height = h + bx.padding.top + bx.padding.bottom + vertical_border;
        }
    } else if style.aspect_ratio > 0 && bx.width > 0 {
        // aspect-ratio: width / height — compute height from width.
        // aspect_ratio is stored as (w/h) * 100, so height = width * 100 / aspect_ratio.
        let content_w = bx.width - bx.padding.left - bx.padding.right - horizontal_border;
        let ar_h = content_w * 100 / style.aspect_ratio;
        bx.height = ar_h + bx.padding.top + bx.padding.bottom + vertical_border;
    } else {
        // content_h from layout_children already includes border_width (top) + padding.top.
        // Add padding.bottom + border_width (bottom) to get the full outer height.
        bx.height = content_h + bx.padding.bottom + bx.border_width;
    }

    // Apply min-height / max-height.
    if let Some(mh) = style
        .max_height
        .or_else(|| style.max_height_calc.map(resolve_height_calc))
    {
        let max_h = if is_border_box {
            mh
        } else {
            mh + bx.padding.top + bx.padding.bottom + vertical_border
        };
        if bx.height > max_h {
            bx.height = max_h;
        }
    }
    let min_height_val = if let Some(calc) = style.min_height_calc {
        resolve_height_calc(calc)
    } else {
        style.min_height
    };
    if min_height_val > 0 {
        let min_h = if is_border_box {
            min_height_val
        } else {
            min_height_val + bx.padding.top + bx.padding.bottom + vertical_border
        };
        if bx.height < min_h {
            bx.height = min_h;
        }
    }

    if !style.align_content_is_normal
        && !matches!(
            style.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        )
    {
        apply_block_align_content(&mut bx, style, vertical_border);
    }

    if matches!(
        style.display,
        Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
    ) {
        append_out_of_flow_children(
            dom, styles, pseudo, node_id, inner_w, &mut bx, images, viewport_w,
        );
    }

    // Apply position:relative offset (does not affect child layout).
    if style.position == Position::Relative {
        let top = resolve_inset(style.top, style.top_calc, parent_height, parent_height > 0);
        let left = resolve_inset(style.left_offset, style.left_calc, available_width, true);
        let bottom = resolve_inset(
            style.bottom_offset,
            style.bottom_calc,
            parent_height,
            parent_height > 0,
        );
        let right = resolve_inset(style.right_offset, style.right_calc, available_width, true);
        let dy = top.unwrap_or_else(|| bottom.map(|v| -v).unwrap_or(0));
        let dx = left.unwrap_or_else(|| right.map(|v| -v).unwrap_or(0));
        bx.y += dy;
        bx.x += dx;
    }

    // position:sticky — behaves like position:relative in layout (in-flow),
    // but the renderer clamps the element to sticky_top when scrolled past.
    if style.position == Position::Sticky {
        bx.is_sticky = true;
        bx.sticky_top = style.top.unwrap_or(0);
        // Stay in normal flow (no is_fixed, no absolute repositioning).
    }

    // Apply CSS transform: translate offsets.
    apply_transform_translation(&mut bx, style);

    // Apply CSS transform: scale and rotate.
    bx.transform_tx_pct = style.transform_tx_pct;
    bx.transform_ty_pct = style.transform_ty_pct;
    bx.transform_sx = style.transform_sx;
    bx.transform_sy = style.transform_sy;
    bx.transform_origin_x = style.transform_origin_x;
    bx.transform_origin_x_is_percent = style.transform_origin_x_is_percent;
    bx.transform_origin_y = style.transform_origin_y;
    bx.transform_origin_y_is_percent = style.transform_origin_y_is_percent;
    bx.transform_rotate = style.transform_rotate;
    bx
}

fn append_out_of_flow_children(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
) {
    let child_ids: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    let parent_style = &styles[node_id];
    for &abs_id in &child_ids {
        let abs_style = &styles[abs_id];
        if abs_style.display == Display::None {
            continue;
        }
        if !matches!(abs_style.position, Position::Absolute | Position::Fixed) {
            continue;
        }

        // Absolute descendants are positioned relative to the parent's
        // padding box, whose origin is directly after the border.
        let content_x = parent.border_width;
        let content_y = parent.border_width;
        let content_w =
            (parent.width - parent.padding.left - parent.padding.right - parent.border_width * 2)
                .max(0);
        let content_h =
            (parent.height - parent.padding.top - parent.padding.bottom - parent.border_width * 2)
                .max(0);

        let mut abs_box = build_block(
            dom, styles, pseudo, abs_id, content_w, images, viewport_w, content_h,
        );

        let mut static_x = content_x;
        let mut static_y = content_y;

        match parent_style.display {
            Display::Flex | Display::InlineFlex => {
                if matches!(
                    parent_style.flex_direction,
                    crate::style::FlexDirection::Row | crate::style::FlexDirection::RowReverse
                ) {
                    static_y += match abs_style.align_self.unwrap_or(parent_style.align_items) {
                        crate::style::AlignItems::Center => (content_h - abs_box.height).max(0) / 2,
                        crate::style::AlignItems::FlexEnd => (content_h - abs_box.height).max(0),
                        _ => 0,
                    };
                } else {
                    static_x += match abs_style.align_self.unwrap_or(parent_style.align_items) {
                        crate::style::AlignItems::Center => (content_w - abs_box.width).max(0) / 2,
                        crate::style::AlignItems::FlexEnd => (content_w - abs_box.width).max(0),
                        _ => 0,
                    };
                }
            }
            Display::Grid | Display::InlineGrid => {
                static_x += match abs_style.justify_self.unwrap_or(parent_style.justify_items) {
                    crate::style::AlignItems::Center => (content_w - abs_box.width).max(0) / 2,
                    crate::style::AlignItems::FlexEnd => (content_w - abs_box.width).max(0),
                    _ => 0,
                };
                static_y += match abs_style.align_self.unwrap_or(parent_style.align_items) {
                    crate::style::AlignItems::Center => (content_h - abs_box.height).max(0) / 2,
                    crate::style::AlignItems::FlexEnd => (content_h - abs_box.height).max(0),
                    _ => 0,
                };
            }
            _ => {}
        }

        let abs_left = resolve_inset(
            abs_style.left_offset,
            abs_style.left_calc,
            content_w,
            true,
        );
        let abs_top = resolve_inset(abs_style.top, abs_style.top_calc, content_h, content_h > 0);
        let abs_right = resolve_inset(
            abs_style.right_offset,
            abs_style.right_calc,
            content_w,
            true,
        );
        let abs_bottom = resolve_inset(
            abs_style.bottom_offset,
            abs_style.bottom_calc,
            content_h,
            content_h > 0,
        );

        abs_box.x = content_x + abs_left.unwrap_or(0) + abs_box.margin.left;
        abs_box.y = content_y + abs_top.unwrap_or(0) + abs_box.margin.top;

        if abs_left.is_none() {
            if let Some(r) = abs_right {
                abs_box.x = content_x + content_w - r - abs_box.width - abs_box.margin.right;
            } else {
                abs_box.x = static_x;
            }
        }
        if abs_top.is_none() {
            if let Some(b) = abs_bottom {
                abs_box.y = content_y + content_h - b - abs_box.height - abs_box.margin.bottom;
            } else {
                abs_box.y = static_y;
            }
        }

        apply_transform_translation(&mut abs_box, abs_style);
        abs_box.is_fixed = abs_style.position == Position::Fixed;
        abs_box.is_out_of_flow = true;
        abs_box.is_positioned = true;
        abs_box.static_position_x = Some(static_x);
        abs_box.static_position_y = Some(static_y);
        abs_box.static_position_width = Some(content_w);
        abs_box.static_position_height = Some(content_h);

        let abs_order = child_ids
            .iter()
            .position(|&child_id| child_id == abs_id)
            .unwrap_or(child_ids.len());
        let insert_at = parent
            .children
            .iter()
            .position(|child| {
                child
                    .node_id
                    .and_then(|child_id| child_ids.iter().position(|&id| id == child_id))
                    .map(|child_order| child_order > abs_order)
                    .unwrap_or(false)
            })
            .unwrap_or(parent.children.len());
        parent.children.insert(insert_at, abs_box);
    }
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

fn apply_block_align_content(bx: &mut LayoutBox, style: &ComputedStyle, vertical_border: i32) {
    let flow_indices: Vec<usize> = bx
        .children
        .iter()
        .enumerate()
        .filter_map(|(idx, child)| (!child.is_out_of_flow).then_some(idx))
        .collect();
    if flow_indices.is_empty() {
        return;
    }

    let group_top = flow_indices
        .iter()
        .map(|&idx| bx.children[idx].y - bx.children[idx].margin.top)
        .min()
        .unwrap_or(0);
    let group_bottom = flow_indices
        .iter()
        .map(|&idx| {
            let child = &bx.children[idx];
            child.y + child.height + child.margin.bottom
        })
        .max()
        .unwrap_or(group_top);
    let group_h = (group_bottom - group_top).max(0);
    let content_top = bx.border_width + bx.padding.top;
    let content_h = (bx.height - bx.padding.top - bx.padding.bottom - vertical_border).max(0);
    let free = content_h - group_h;
    let unsafe_align = !matches!(style.overflow_y, OverflowVal::Visible);
    let offset = match style.align_content {
        AlignContent::FlexEnd => {
            if unsafe_align {
                free
            } else {
                free.max(0)
            }
        }
        AlignContent::Center => {
            if unsafe_align {
                free / 2
            } else {
                free.max(0) / 2
            }
        }
        AlignContent::SpaceAround | AlignContent::SpaceEvenly => {
            if unsafe_align {
                free / 2
            } else {
                free.max(0) / 2
            }
        }
        _ => 0,
    } + (content_top - group_top);

    if offset == 0 {
        return;
    }
    for child in &mut bx.children {
        if !child.is_out_of_flow {
            child.y += offset;
        }
    }
}

// ---------------------------------------------------------------------------
// Pseudo-element helpers
// ---------------------------------------------------------------------------

/// Returns true if the pseudo-element's display property is block-level.
/// Inline pseudo-elements are injected into the inline flow by layout_children/layout_inline_content.
fn is_block_pseudo(ps: &ComputedStyle) -> bool {
    matches!(
        ps.display,
        Display::Block
            | Display::FlowRoot
            | Display::InlineBlock
            | Display::Flex
            | Display::InlineFlex
            | Display::Grid
            | Display::InlineGrid
    )
}

/// Build a LayoutBox for a `::before` or `::after` pseudo-element.
///
/// Handles all display modes:
/// - `display: block` / `display: inline-block`: creates a block box with
///   background, border, padding and optional text child.
/// - `display: inline` (default): creates an inline text box.
/// - `content: url(...)`: creates an image box.
/// - `content: ""` (empty) with block display + visual properties: creates a
///   dimensioned block (e.g., decorative lines / separators).
///
/// Returns `None` if the pseudo-element has no visible output.
pub(super) fn build_pseudo_element_box(
    ps: &ComputedStyle,
    available_w: i32,
    images: &crate::ImageCache,
    viewport_w: i32,
) -> Option<LayoutBox> {
    let content_text = ps.content.as_deref().unwrap_or("");
    let has_text = !content_text.is_empty();
    let is_block = matches!(
        ps.display,
        Display::Block
            | Display::FlowRoot
            | Display::InlineBlock
            | Display::Flex
            | Display::InlineFlex
            | Display::Grid
            | Display::InlineGrid
    );

    // URL content: render as image box
    if let Some(ref url) = ps.content_url {
        if !url.is_empty() {
            let mut ib = LayoutBox::new(None, BoxType::Inline);
            ib.image_src = Some(url.clone());
            // Estimate dimensions from cache; default to icon size if not found
            let sz = images
                .get_ref(url.as_str())
                .map(|e| (e.width.min(65535) as i32, e.height.min(65535) as i32))
                .unwrap_or((16, 16));
            ib.width = sz.0.min(available_w);
            ib.height = sz.1;
            ib.bg_color = ps.background_color;
            return Some(ib);
        }
    }

    let fs = if ps.font_size > 0 { ps.font_size } else { 16 };
    let bold = matches!(ps.font_weight, FontWeight::Bold);
    let italic = matches!(ps.font_style, FontStyleVal::Italic);

    if is_block {
        // Build a block-level pseudo-element box.
        let mut pb = LayoutBox::new(None, BoxType::Block);
        pb.color = ps.color;
        pb.bg_color = if ps.background_color_is_current {
            ps.color
        } else {
            ps.background_color
        };
        pb.font_size = fs;
        pb.bold = bold;
        pb.italic = italic;
        pb.appearance_none = ps.appearance == crate::style::AppearanceVal::None;
        pb.text_decoration = ps.text_decoration;
        pb.border_width = ps.border_width;
        pb.border_color = ps.border_color;
        pb.border_top_width = ps.border_top.width;
        pb.border_right_width = ps.border_right.width;
        pb.border_bottom_width = ps.border_bottom.width;
        pb.border_left_width = ps.border_left.width;
        pb.border_top_color = ps.border_top.color;
        pb.border_right_color = ps.border_right.color;
        pb.border_bottom_color = ps.border_bottom.color;
        pb.border_left_color = ps.border_left.color;
        pb.border_top_style = ps.border_top.style;
        pb.border_right_style = ps.border_right.style;
        pb.border_bottom_style = ps.border_bottom.style;
        pb.border_left_style = ps.border_left.style;
        pb.border_top_left_radius = ps.border_top_left_radius;
        pb.border_top_right_radius = ps.border_top_right_radius;
        pb.border_bottom_right_radius = ps.border_bottom_right_radius;
        pb.border_bottom_left_radius = ps.border_bottom_left_radius;
        pb.border_radius = ps.border_radius;
        pb.padding = super::edges_from(
            ps.padding_top,
            ps.padding_right,
            ps.padding_bottom,
            ps.padding_left,
        );
        let (pmargin_top, pmargin_right, pmargin_bottom, pmargin_left) =
            resolve_margins(ps, available_w);
        pb.margin = super::edges_from(pmargin_top, pmargin_right, pmargin_bottom, pmargin_left);
        pb.background_image = ps.background_image.clone();
        pb.mask_image = ps.mask_image.clone();
        pb.background_size = ps.background_size;
        pb.background_repeat = ps.background_repeat;
        pb.background_clip = ps.background_clip;
        pb.background_position_x = ps.background_position_x;
        pb.background_position_y = ps.background_position_y;
        pb.mask_size = ps.mask_size;
        pb.mask_repeat = ps.mask_repeat;
        pb.mask_clip = ps.mask_clip;
        pb.mask_origin = ps.mask_origin;
        pb.mask_position_x = ps.mask_position_x;
        pb.mask_position_x_is_percent = ps.mask_position_x_is_percent;
        pb.mask_position_y = ps.mask_position_y;
        pb.mask_position_y_is_percent = ps.mask_position_y_is_percent;
        pb.opacity = ps.opacity;
        pb.z_index = ps.z_index;
        pb.z_index_auto = ps.z_index_auto;
        pb.letter_spacing = ps.letter_spacing;
        pb.text_align = ps.text_align;

        // Determine box width
        let border2 = pb.border_top_width + pb.border_bottom_width;
        let pad_h =
            pb.padding.left + pb.padding.right + pb.border_left_width + pb.border_right_width;
        let inner_for_text = (available_w - pad_h).max(0);
        if let Some(w) = ps.width {
            pb.width = w;
        } else if ps.width_pct.is_some() {
            let pct = ps.width_pct.unwrap_or(0);
            pb.width = (available_w as i64 * pct as i64 / 10000) as i32;
        } else {
            pb.width = available_w;
        }

        // Determine box height
        if let Some(h) = ps.height {
            pb.height = h;
        } else if has_text {
            pb.height = pb.padding.top + pb.padding.bottom + fs + 4;
        } else {
            pb.height = pb.padding.top + pb.padding.bottom + border2;
        }
        // Apply min/max height
        if let Some(mh) = ps.max_height {
            if pb.height > mh {
                pb.height = mh;
            }
        } else if let Some((px100, pct100)) = ps.max_height_calc {
            let _ = pct100;
            let mh = px100 / 100;
            if pb.height > mh {
                pb.height = mh;
            }
        }
        let min_h = if let Some((px100, pct100)) = ps.min_height_calc {
            let _ = pct100;
            px100 / 100
        } else {
            ps.min_height
        };
        if min_h > 0 && pb.height < min_h {
            pb.height = min_h;
        }

        // Add text content as inline child
        if has_text {
            let mut tb =
                LayoutBox::new_text(String::from(content_text), fs, bold, italic, ps.color);
            tb.custom_font_id = ps
                .font_family
                .as_ref()
                .and_then(|family| crate::lookup_web_font(family))
                .unwrap_or(0);
            tb.bg_color = 0;
            tb.text_decoration = ps.text_decoration;
            tb.x = pb.padding.left + pb.border_left_width;
            tb.y = pb.padding.top + pb.border_top_width;
            // Estimate text width/height
            let (tw, th) = super::measure_text(content_text, fs, tb.custom_font_id, bold, italic);
            tb.width = tw.min(inner_for_text);
            tb.height = th.max(fs + 2);
            pb.height = pb
                .height
                .max(tb.y + tb.height + pb.padding.bottom + pb.border_bottom_width);
            pb.children.push(tb);
        }

        // Skip completely empty boxes with no visual properties
        let has_visual = pb.bg_color != 0
            || pb.border_top_width > 0
            || pb.border_right_width > 0
            || pb.border_bottom_width > 0
            || pb.border_left_width > 0
            || pb.border_width > 0
            || !matches!(pb.background_image, crate::style::BackgroundImageVal::None)
            || pb.height > 0;
        if !has_visual && !has_text {
            return None;
        }

        Some(pb)
    } else {
        // Inline pseudo-element: create a text box if there is text.
        if !has_text {
            return None;
        }
        let mut tb = LayoutBox::new_text(String::from(content_text), fs, bold, italic, ps.color);
        tb.custom_font_id = ps
            .font_family
            .as_ref()
            .and_then(|family| crate::lookup_web_font(family))
            .unwrap_or(0);
        tb.bg_color = ps.background_color;
        tb.text_decoration = ps.text_decoration;
        tb.letter_spacing = ps.letter_spacing;
        Some(tb)
    }
}
