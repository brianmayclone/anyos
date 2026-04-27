//! Flexbox layout: `layout_flex()` implements the CSS Flexible Box Layout.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{
    AlignContent, AlignItems, ComputedStyle, Display, FlexDirection, FlexWrap, JustifyContent,
    Position, PseudoStyles,
};
use crate::ImageCache;

use super::block::{build_block, build_block_with_forced_outer_height};
use super::LayoutBox;

struct FlexItem {
    node_id: NodeId,
    grow: i32,
    shrink: i32,
    order: i32,
    main_base: i32,
    cross_base: i32,
    layout: Option<LayoutBox>,
    /// True when the item has `flex-basis: auto`, `width: auto`, and its content
    /// (or a descendant) uses a percentage main-axis size (e.g. a table with
    /// `width: 100%`). Per the CSS Flexbox interop quirk (Mozilla bug 1469649),
    /// such items need a "definite post-flexing main size" so the percentage can
    /// resolve. We achieve this by treating them as `flex-grow: 1` when no other
    /// item in the line has explicit grow.
    needs_definite_main: bool,
}

struct FlexLine {
    start: usize,
    end: usize,
    total_main: i32,
    cross_size: i32, // resolved cross size of this line
}

fn round_div_i64(num: i64, den: i64) -> i32 {
    if den == 0 {
        return 0;
    }
    if num >= 0 {
        ((num + den / 2) / den) as i32
    } else {
        ((num - den / 2) / den) as i32
    }
}

fn justify_offset_before_item(
    justify: JustifyContent,
    idx: usize,
    count: usize,
    remaining: i32,
) -> i32 {
    let idx = idx as i64;
    let count = count as i64;
    let remaining = remaining.max(0) as i64;
    match justify {
        JustifyContent::FlexStart => 0,
        JustifyContent::FlexEnd => remaining as i32,
        JustifyContent::Center => round_div_i64(remaining, 2),
        JustifyContent::SpaceBetween => {
            if count > 1 {
                round_div_i64(idx * remaining, count - 1)
            } else {
                0
            }
        }
        JustifyContent::SpaceAround => {
            if count > 0 {
                round_div_i64((2 * idx + 1) * remaining, 2 * count)
            } else {
                0
            }
        }
        JustifyContent::SpaceEvenly => round_div_i64((idx + 1) * remaining, count + 1),
    }
}

/// Resolve the effective align-items for a child (considering align-self).
fn resolve_align(container_align: AlignItems, child_style: &ComputedStyle) -> AlignItems {
    child_style.align_self.unwrap_or(container_align)
}

fn flex_item_baseline(style: &ComputedStyle, bx: &LayoutBox, is_row: bool) -> i32 {
    let font_baseline = style.font_size.max(1) * 4 / 5;
    if is_row {
        font_baseline.max(0)
    } else {
        let size = bx.width.max(0);
        font_baseline.max(0).min(size)
    }
}

/// Check whether a DOM subtree contains a descendant with a percentage main-axis
/// size (`width: %` for row direction, `height: %` for column).
/// Used to detect flex items that need a "definite post-flexing main size"
/// (CSS Flexbox interop quirk for percentage-sized table descendants).
fn has_percent_main_descendant(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    is_row: bool,
) -> bool {
    // Check this node's children (we don't check the node itself — the caller
    // already verified the flex item has auto main-axis size).
    for &cid in &dom.get(node_id).children {
        let cst = &styles[cid];
        let has_pct = if is_row {
            cst.width_pct.is_some()
        } else {
            cst.height_pct.is_some()
        };
        if has_pct {
            return true;
        }
        // Recurse into descendants.
        if has_percent_main_descendant(dom, styles, cid, is_row) {
            return true;
        }
    }
    false
}

/// Compute the max-content width of a DOM node by recursively measuring
/// its content without building a full layout.
///
/// For flex items without explicit width (Case E), `build_block` fills
/// available width for block children, so we can't use the built layout
/// box widths.  Instead, measure the natural content extent directly.
pub(super) fn measure_max_content(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    images: &ImageCache,
    viewport_w: i32,
) -> i32 {
    let st = &styles[node_id];

    // Explicit width → use it.
    if let Some(w) = st.width {
        if w > 0 {
            return w;
        }
    }

    // Absolute/fixed → 0 (out of flow).
    if matches!(st.position, Position::Absolute | Position::Fixed) {
        return 0;
    }
    if st.display == Display::None {
        return 0;
    }

    // CSS Sizing §5.1: If box has a definite size in block axis and an aspect
    // ratio, the inline size is computed from the block size × aspect ratio.
    // aspect_ratio is stored as (w/h) * 100.
    if st.aspect_ratio > 0 {
        if let Some(h) = st.height {
            return (h * st.aspect_ratio / 100).max(0);
        }
    }

    if let Some(w) = super::intrinsic_form_control_width(dom, styles, node_id, Some(viewport_w)) {
        return w;
    }

    let pad_border = st.padding_left
        + st.padding_right
        + st.border_width * 2
        + st.border_left.width
        + st.border_right.width;

    // Text node → measure text width.
    if let crate::dom::NodeType::Text(ref text) = dom.nodes[node_id].node_type {
        if super::is_inside_svg(dom, node_id) {
            return 0;
        }
        let measured = super::trim_leading_ascii_ws(text);
        if measured.trim().is_empty() {
            return 0;
        }
        let fs = st.font_size.max(1);
        let bold = matches!(st.font_weight, crate::style::FontWeight::Bold);
        let italic = matches!(st.font_style, crate::style::FontStyleVal::Italic);
        let custom_font_id = st
            .font_family
            .as_ref()
            .and_then(|family| crate::lookup_web_font(family))
            .unwrap_or(0);
        let tw = super::measure_collapsed_text_width(
            measured,
            fs,
            custom_font_id,
            bold,
            italic,
            st.letter_spacing,
            st.word_spacing,
        );
        return tw;
    }

    // Image → use image dimensions or CSS width.
    if dom.tag(node_id) == Some(Tag::Img) || dom.has_tag_name(node_id, "a-img") {
        if let Some(src) = dom.image_url(node_id) {
            if let Some(info) = images.get_ref(&src) {
                let w = dom
                    .attr(node_id, "width")
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(info.width as i32);
                return w + pad_border;
            }
        }
        return pad_border;
    }

    // Inline <svg> → use rasterised dimensions, but honor definite CSS sizing
    // before using it as a flex max-content contribution. A common pattern is
    // `height: 24px; width: auto`, where the natural viewBox width would wildly
    // overstate the flex base size.
    if dom.tag(node_id) == Some(Tag::Svg) {
        let (w, _) = super::svg_intrinsic_dimensions(dom, images, node_id);
        let (_, h) = super::svg_intrinsic_dimensions(dom, images, node_id);
        let content_w = if let Some(css_w) = st.width {
            css_w.max(0)
        } else if let Some(pct) = st.width_pct {
            (viewport_w.max(0) as i64 * pct as i64 / 10000) as i32
        } else if let Some((px100, pct100)) = st.width_calc {
            px100 / 100 + (viewport_w.max(0) as i64 * pct100 as i64 / 10000) as i32
        } else if let Some(css_h) = st.height {
            if h > 0 {
                ((w as i64 * css_h.max(0) as i64) / h as i64).max(0) as i32
            } else {
                w
            }
        } else if let Some((px100, _)) = st.height_calc {
            if h > 0 {
                ((w as i64 * (px100 / 100).max(0) as i64) / h as i64).max(0) as i32
            } else {
                w
            }
        } else {
            w
        };
        return content_w + pad_border;
    }

    let children: Vec<usize> = dom.get(node_id).children.iter().copied().collect();
    let is_flex = matches!(st.display, Display::Flex | Display::InlineFlex);
    let is_row = is_flex
        && matches!(
            st.flex_direction,
            crate::style::FlexDirection::Row | crate::style::FlexDirection::RowReverse
        );

    // Inline formatting context → max-content is the unwrapped line width.
    if !is_flex && children_form_inline_run(dom, styles, &children) {
        let mut total = 0i32;
        for &cid in &children {
            let cst = &styles[cid];
            if cst.display == Display::None {
                continue;
            }
            if matches!(cst.position, Position::Absolute | Position::Fixed) {
                continue;
            }
            if let crate::dom::NodeType::Text(ref text) = dom.nodes[cid].node_type {
                if text.trim().is_empty() {
                    continue;
                }
            }
            total += measure_max_content(dom, styles, pseudo, cid, images, viewport_w)
                + cst.margin_left
                + cst.margin_right;
        }
        return total + pad_border;
    }

    // Flex container → sum of children's max-content widths + gaps.
    if is_row {
        let gap = st.column_gap;
        let mut total = 0i32;
        let mut count = 0;
        for &cid in &children {
            let cst = &styles[cid];
            if cst.display == Display::None {
                continue;
            }
            if matches!(cst.position, Position::Absolute | Position::Fixed) {
                continue;
            }
            let cw = measure_max_content(dom, styles, pseudo, cid, images, viewport_w);
            if cw > 0 {
                if count > 0 {
                    total += gap;
                }
                total += cw + cst.margin_left + cst.margin_right;
                count += 1;
            }
        }
        return total + pad_border;
    }

    // Block/column container → max of children's max-content widths.
    let mut max_w = 0i32;
    for &cid in &children {
        let cst = &styles[cid];
        if cst.display == Display::None {
            continue;
        }
        if matches!(cst.position, Position::Absolute | Position::Fixed) {
            continue;
        }
        let cw = measure_max_content(dom, styles, pseudo, cid, images, viewport_w)
            + cst.margin_left
            + cst.margin_right;
        if cw > max_w {
            max_w = cw;
        }
    }
    max_w + pad_border
}

fn children_form_inline_run(dom: &Dom, styles: &[ComputedStyle], children: &[NodeId]) -> bool {
    let mut saw_inline = false;
    for &cid in children {
        let st = &styles[cid];
        if st.display == Display::None {
            continue;
        }
        if matches!(st.position, Position::Absolute | Position::Fixed) {
            continue;
        }
        match &dom.get(cid).node_type {
            NodeType::Text(text) => {
                if !text.trim().is_empty() {
                    saw_inline = true;
                }
            }
            NodeType::Element { .. } => match st.display {
                Display::Inline
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::InlineGrid
                | Display::Contents => {
                    saw_inline = true;
                }
                _ => return false,
            },
        }
    }
    saw_inline
}

/// Lay out children as a flex container and return the total height consumed.
pub fn layout_flex(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    parent_height: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
    container_height_hint: Option<i32>,
) -> i32 {
    let parent_style_idx = parent.node_id.unwrap_or(0);
    let parent_style = &styles[parent_style_idx];
    let direction = parent_style.flex_direction;
    let wrap = parent_style.flex_wrap;
    let justify = parent_style.justify_content;
    let align = parent_style.align_items;
    let align_content = parent_style.align_content;
    let row_gap = parent_style.row_gap;
    let col_gap = parent_style.column_gap;

    let is_row = matches!(direction, FlexDirection::Row | FlexDirection::RowReverse);
    let is_reverse = matches!(
        direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    );
    // §9.2 Step 1: Determine available main space.
    // For row flex: available_width is the container's content width.
    // For column flex: resolve height against a definite containing-block height.
    let definite_container_height = if let Some(h) = container_height_hint {
        Some(h)
    } else if let Some(h) = parent_style.height {
        Some(h)
    } else if let Some(pct) = parent_style.height_pct {
        if pct > 0 && parent_height > 0 {
            Some((parent_height as i64 * pct as i64 / 10000) as i32)
        } else {
            None
        }
    } else if let Some((px100, pct100)) = parent_style.height_calc {
        let px_part = px100 / 100;
        let pct_part = if parent_height > 0 {
            (parent_height as i64 * pct100 as i64 / 10000) as i32
        } else {
            0
        };
        Some(px_part + pct_part)
    } else if matches!(parent_style.position, Position::Absolute | Position::Fixed)
        && parent_style.top.is_some()
        && parent_style.bottom_offset.is_some()
        && parent_height > 0
    {
        // CSS §10.6.4: absolute with top+bottom and height:auto → cb_height - top - bottom.
        let t = parent_style.top.unwrap_or(0);
        let b = parent_style.bottom_offset.unwrap_or(0);
        let h = (parent_height - t - b).max(0);
        if h > 0 {
            Some(h)
        } else {
            None
        }
    } else {
        None
    };

    // `min-height` still matters for free-space distribution, but it does not
    // make percentage heights on children definite on its own.
    let main_size = if is_row {
        available_width
    } else if let Some(h) = definite_container_height {
        h
    } else if parent_style.min_height > 0 {
        parent_style.min_height
    } else {
        0
    };

    // Collect visible flex items.
    // Per spec §4: each in-flow child becomes a flex item.
    // Skip hidden inputs (they generate no box).
    let mut items: Vec<FlexItem> = Vec::new();
    for &cid in child_ids {
        let st = &styles[cid];
        if st.display == Display::None {
            continue;
        }
        if matches!(st.position, Position::Absolute | Position::Fixed) {
            continue;
        }
        // CSS Flexbox §4: text nodes containing only whitespace do NOT generate
        // anonymous flex items (they are treated as `display: none`).
        if let NodeType::Text(ref s) = dom.get(cid).node_type {
            if s.chars().all(|c| c.is_whitespace()) {
                continue;
            }
        }
        // Skip <input type="hidden"> — generates no box.
        if dom.tag(cid) == Some(Tag::Input) {
            if dom.attr(cid, "type").unwrap_or("") == "hidden" {
                continue;
            }
        }
        // Detect "needs definite main size" quirk: flex item with auto basis,
        // auto main-axis size, no explicit grow, but contains a descendant with
        // a percentage main-axis size. Per CSS Flexbox interop, such items
        // should be sized so the percentage can resolve (Mozilla bug 1469649).
        let needs_definite_main = st.flex_basis.is_none()
            && st.flex_basis_pct.is_none()
            && (if is_row {
                st.width.is_none() && st.width_pct.is_none() && st.width_calc.is_none()
            } else {
                st.height.is_none() && st.height_pct.is_none() && st.height_calc.is_none()
            })
            && st.flex_grow == 0
            && has_percent_main_descendant(dom, styles, cid, is_row);

        items.push(FlexItem {
            node_id: cid,
            grow: st.flex_grow,
            shrink: st.flex_shrink,
            order: st.order,
            main_base: 0,
            cross_base: 0,
            layout: None,
            needs_definite_main,
        });
    }

    // Sort by order.
    items.sort_by(|a, b| a.order.cmp(&b.order));

    // Phase 1: Determine the flex base size of each item (CSS Flexbox §9.2 Step 2).
    //
    // Per spec: flex-basis:auto + width:auto → used flex basis = max-content size.
    // max-content = the width when text never wraps (natural content width).
    // We approximate this by laying out with a large available width, then
    // using the actual content extent as the base size.
    //
    // Text nodes (anonymous flex items) are measured directly via text measurement.
    let parent_font_size = parent_style.font_size.max(1);
    let parent_bold = matches!(parent_style.font_weight, crate::style::FontWeight::Bold);
    let parent_color = parent_style.color;
    let parent_custom_font_id = parent_style
        .font_family
        .as_ref()
        .and_then(|family| crate::lookup_web_font(family))
        .unwrap_or(0);

    for item in &mut items {
        // Handle text nodes as anonymous flex items (CSS Flexbox §4).
        if let NodeType::Text(ref text) = dom.nodes[item.node_id].node_type {
            if super::is_inside_svg(dom, item.node_id) {
                item.main_base = 0;
                item.cross_base = 0;
                continue;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                item.main_base = 0;
                item.cross_base = 0;
                continue;
            }
            let (tw, th) = super::measure_text(
                trimmed,
                parent_font_size,
                parent_custom_font_id,
                parent_bold,
                false,
            );
            let mut text_box = super::LayoutBox::new_text(
                String::from(trimmed),
                parent_font_size,
                parent_bold,
                false,
                parent_color,
            );
            text_box.custom_font_id = parent_custom_font_id;
            let measured_h = th.max(parent_font_size);
            text_box.width = tw;
            text_box.height = measured_h;
            if is_row {
                item.main_base = tw;
                item.cross_base = measured_h;
            } else {
                item.main_base = measured_h;
                item.cross_base = tw;
            }
            item.layout = Some(text_box);
            continue;
        }

        let st = &styles[item.node_id];
        let main_margins = if is_row {
            st.margin_left + st.margin_right
        } else {
            st.margin_top + st.margin_bottom
        };
        if let Some(basis) = st.flex_basis {
            // Case A: definite flex-basis (absolute length).
            item.main_base = basis + main_margins;
        } else if let Some(pct) = st.flex_basis_pct {
            // Case A: definite flex-basis (percentage of container main size).
            // If container main size is indefinite, fall through to auto handling below.
            let container_main = if is_row {
                available_width
            } else {
                definite_container_height.unwrap_or(-1)
            };
            if container_main > 0 {
                item.main_base = (container_main as i64 * pct as i64 / 10000) as i32 + main_margins;
            } else {
                // Indefinite container — fall back to max-content
                if is_row {
                    let mc_w =
                        measure_max_content(dom, styles, pseudo, item.node_id, images, viewport_w);
                    item.main_base =
                        mc_w.max(1).min(available_width.max(1)) + st.margin_left + st.margin_right;
                } else {
                    let child_box = build_block(
                        dom,
                        styles,
                        pseudo,
                        item.node_id,
                        available_width,
                        images,
                        viewport_w,
                        0,
                    );
                    item.main_base =
                        child_box.height + child_box.margin.top + child_box.margin.bottom;
                    item.cross_base =
                        child_box.width + child_box.margin.left + child_box.margin.right;
                    item.layout = Some(child_box);
                }
            }
        } else if is_row {
            if let Some(w) = st.width {
                // Case A: definite width.
                item.main_base = w + main_margins;
            } else if let Some(pct) = st.width_pct {
                item.main_base =
                    (available_width as i64 * pct as i64 / 10000) as i32 + main_margins;
            } else if let Some((px100, pct100)) = st.width_calc {
                item.main_base = px100 / 100
                    + (available_width as i64 * pct100 as i64 / 10000) as i32
                    + main_margins;
            } else {
                // Case E: flex-basis:auto + width:auto → max-content size.
                // Measure the natural content width by recursively walking the
                // DOM (not via build_block, which fills available_width for
                // block children).
                let mc_w =
                    measure_max_content(dom, styles, pseudo, item.node_id, images, viewport_w);
                let base_w = mc_w.max(1).min(available_width);
                item.main_base = base_w + st.margin_left + st.margin_right;
                // Cross size: build at the resolved width to get the height.
                let child_box = build_block(
                    dom,
                    styles,
                    pseudo,
                    item.node_id,
                    base_w,
                    images,
                    viewport_w,
                    definite_container_height.unwrap_or(0),
                );
                item.cross_base = child_box.height + child_box.margin.top + child_box.margin.bottom;
                // Don't cache the layout — it was done at max-content width,
                // Phase 3 will re-layout at the resolved width.
            }
        } else {
            if let Some(h) = st.height {
                item.main_base = h + main_margins;
            } else {
                let child_box = build_block(
                    dom,
                    styles,
                    pseudo,
                    item.node_id,
                    available_width,
                    images,
                    viewport_w,
                    definite_container_height.unwrap_or(0),
                );
                item.main_base = child_box.height + child_box.margin.top + child_box.margin.bottom;
                item.cross_base = child_box.width + child_box.margin.left + child_box.margin.right;
                item.layout = Some(child_box);
            }
        }
    }

    // Phase 2: Break into flex lines (if wrapping).
    let gap = if is_row { col_gap } else { row_gap };
    let mut lines: Vec<FlexLine> = Vec::new();
    let mut line_start = 0;
    let mut line_main = 0i32;

    for i in 0..items.len() {
        let item_main = items[i].main_base;
        let with_gap = if line_start < i { gap } else { 0 };
        let new_main = line_main + item_main + with_gap;

        if wrap != FlexWrap::Nowrap && line_start < i && main_size > 0 && new_main > main_size {
            lines.push(FlexLine {
                start: line_start,
                end: i,
                total_main: line_main,
                cross_size: 0,
            });
            line_start = i;
            line_main = item_main;
        } else {
            line_main = new_main;
        }
    }
    if line_start < items.len() {
        lines.push(FlexLine {
            start: line_start,
            end: items.len(),
            total_main: line_main,
            cross_size: 0,
        });
    }

    // Phase 3: Resolve flexible lengths and position items.
    let cross_gap = if is_row { row_gap } else { col_gap };
    let bw = parent.border_width;
    let mut cross_cursor: i32 = bw + parent.padding.top;
    // For flex-col: track the max vertical (main-axis) extent across all lines.
    // cross_cursor tracks X position for flex-col, NOT height.
    // The container height = bw + padding.top + max_col_main_extent.
    let mut max_col_main: i32 = 0;

    // CSS §9.4: For single-line flex containers with a definite cross-axis size,
    // the line cross size IS the container's inner cross size.
    let single_line = lines.len() == 1 && wrap == FlexWrap::Nowrap;
    let definite_cross: Option<i32> = if is_row {
        // For row flex, cross axis is vertical. Use the resolved definite container height.
        definite_container_height
            .map(|h| h - parent.padding.top - parent.padding.bottom - 2 * bw)
            .filter(|&h| h > 0)
    } else {
        Some(available_width - parent.padding.left - parent.padding.right - 2 * bw)
            .filter(|&w| w > 0)
    };

    for line in &mut lines {
        let count = line.end - line.start;
        if count == 0 {
            continue;
        }

        // Distribute free space along main axis.
        let total_gaps = gap * (count as i32 - 1).max(0);
        let free_space = if main_size > 0 {
            main_size - line.total_main - total_gaps
        } else {
            0
        };

        let total_grow: i32 = items[line.start..line.end].iter().map(|it| it.grow).sum();
        let total_shrink: i32 = items[line.start..line.end].iter().map(|it| it.shrink).sum();

        // CSS Flexbox interop quirk: when no item has explicit grow but some
        // items contain percentage-sized descendants (and thus need a "definite
        // post-flexing main size"), distribute free space equally among them.
        // See Mozilla bug 1469649 — required for tests like
        // fixed-table-layout-with-percentage-width-in-flex-item.html.
        let definite_count = items[line.start..line.end]
            .iter()
            .filter(|it| it.needs_definite_main)
            .count() as i32;
        let auto_grow_each = if total_grow == 0 && free_space > 0 && definite_count > 0 {
            free_space / definite_count
        } else {
            0
        };

        // Compute final main sizes.
        let mut main_sizes: Vec<i32> = Vec::with_capacity(count);
        for i in line.start..line.end {
            let base = items[i].main_base;
            let final_size = if auto_grow_each > 0 && items[i].needs_definite_main {
                base + auto_grow_each
            } else if free_space > 0 && total_grow > 0 {
                base + (free_space as i64 * items[i].grow as i64 / total_grow as i64) as i32
            } else if free_space < 0 && total_shrink > 0 {
                (base + (free_space as i64 * items[i].shrink as i64 / total_shrink as i64) as i32)
                    .max(0)
            } else {
                base
            };
            // §9.7: Clamp to min/max constraints.
            // Note: main_base is content-box in most code paths (only the
            // auto/max-content case adds margins), so clamp without margins.
            let st = &styles[items[i].node_id];
            let clamped = if is_row {
                let mut s = final_size;
                if st.min_width > 0 {
                    s = s.max(st.min_width);
                }
                if let Some(mw) = st.max_width {
                    s = s.min(mw);
                }
                s
            } else {
                let mut s = final_size;
                if st.min_height > 0 {
                    s = s.max(st.min_height);
                }
                if let Some(mh) = st.max_height {
                    s = s.min(mh);
                }
                s
            };
            main_sizes.push(clamped);
        }

        // Re-layout items with resolved sizes.
        let mut cross_max: i32 = if single_line {
            definite_cross.unwrap_or(0)
        } else {
            0
        };
        for (idx, i) in (line.start..line.end).enumerate() {
            let item_main = main_sizes[idx];
            let st = &styles[items[i].node_id];
            // For row flex, child_avail is the resolved main size minus margins.
            let item_margins = if is_row {
                st.margin_left + st.margin_right
            } else {
                st.margin_top + st.margin_bottom
            };
            let child_avail = if is_row {
                (item_main - item_margins).max(0)
            } else {
                available_width
            };
            let forced_child_outer_height = if is_row {
                None
            } else {
                Some((item_main - item_margins).max(0))
            };

            let mut child_box = if let Some(existing) = items[i].layout.take() {
                let existing_main = if is_row {
                    existing.width + existing.margin.left + existing.margin.right
                } else {
                    existing.height + existing.margin.top + existing.margin.bottom
                };
                // Re-layout if the resolved size differs from the initial measurement.
                // This is common when flex-grow/shrink changed the size from the
                // max-content base, or when Phase 1 didn't cache a layout.
                if existing_main != item_main {
                    if let Some(forced_h) = forced_child_outer_height {
                        build_block_with_forced_outer_height(
                            dom,
                            styles,
                            pseudo,
                            items[i].node_id,
                            child_avail,
                            images,
                            viewport_w,
                            definite_container_height.unwrap_or(0),
                            forced_h,
                        )
                    } else {
                        build_block(
                            dom,
                            styles,
                            pseudo,
                            items[i].node_id,
                            child_avail,
                            images,
                            viewport_w,
                            definite_container_height.unwrap_or(0),
                        )
                    }
                } else {
                    existing
                }
            } else {
                // No cached layout (e.g. row-flex items measured at max-content width).
                if let Some(forced_h) = forced_child_outer_height {
                    build_block_with_forced_outer_height(
                        dom,
                        styles,
                        pseudo,
                        items[i].node_id,
                        child_avail,
                        images,
                        viewport_w,
                        definite_container_height.unwrap_or(0),
                        forced_h,
                    )
                } else {
                    build_block(
                        dom,
                        styles,
                        pseudo,
                        items[i].node_id,
                        child_avail,
                        images,
                        viewport_w,
                        definite_container_height.unwrap_or(0),
                    )
                }
            };

            // §8.1: Auto margins on flex items are treated as 0 during layout.
            // build_block may have set auto margins for block-centering — reset them.
            // The flex positioning code will distribute auto margins later.
            let st = &styles[items[i].node_id];
            if !is_row && !matches!(dom.get(items[i].node_id).node_type, NodeType::Text(_)) {
                let item_align = resolve_align(align, st);
                let auto_cross_size =
                    st.width.is_none() && st.width_pct.is_none() && st.width_calc.is_none();
                if auto_cross_size && !matches!(item_align, AlignItems::Stretch) {
                    let fit_w = measure_max_content(
                        dom,
                        styles,
                        pseudo,
                        items[i].node_id,
                        images,
                        viewport_w,
                    )
                    .max(0);
                    child_box.width = fit_w.min(available_width.max(0));
                }
            }
            if is_row {
                // Cross axis = vertical for row flex.
                // (vertical auto margins are rare, skip for now)
            } else {
                // Cross axis = horizontal for column flex.
                if st.margin_left_auto {
                    child_box.margin.left = 0;
                }
                if st.margin_right_auto {
                    child_box.margin.right = 0;
                }
            }

            // Force the resolved width for items that grew or shrank.
            if is_row {
                let target_w = (item_main - child_box.margin.left - child_box.margin.right).max(0);
                if child_box.width != target_w {
                    child_box.width = target_w;
                    // Recompute height from aspect-ratio when width changes.
                    let st = &styles[items[i].node_id];
                    if st.aspect_ratio > 0 {
                        let border2 = child_box.border_width * 2;
                        let content_w =
                            (target_w - child_box.padding.left - child_box.padding.right - border2)
                                .max(0);
                        let ar_h = content_w * 100 / st.aspect_ratio;
                        child_box.height =
                            ar_h + child_box.padding.top + child_box.padding.bottom + border2;
                    }
                }
            }

            let item_cross = if is_row {
                child_box.height + child_box.margin.top + child_box.margin.bottom
            } else {
                child_box.width + child_box.margin.left + child_box.margin.right
            };

            if item_cross > cross_max {
                cross_max = item_cross;
            }
            items[i].layout = Some(child_box);
        }

        let line_baseline = if is_row {
            let mut max_baseline = 0;
            for i in line.start..line.end {
                let item_node = items[i].node_id;
                if !matches!(
                    resolve_align(align, &styles[item_node]),
                    AlignItems::Baseline
                ) {
                    continue;
                }
                if let Some(child_box) = items[i].layout.as_ref() {
                    let baseline = child_box.margin.top
                        + flex_item_baseline(&styles[item_node], child_box, true);
                    if baseline > max_baseline {
                        max_baseline = baseline;
                    }
                }
            }
            max_baseline
        } else {
            0
        };

        // Position items along main axis.
        let used_main: i32 = main_sizes.iter().sum::<i32>() + total_gaps;
        // If main_size is 0 (content-sized container), use the actual content size.
        let effective_main = if main_size > 0 { main_size } else { used_main };
        let remaining = effective_main - used_main;

        let mut running_main = 0i32;

        for (idx, i) in (line.start..line.end).enumerate() {
            let item_node = items[i].node_id;
            let item_align = resolve_align(align, &styles[item_node]);
            let no_explicit_h = styles[item_node].height.is_none();
            let no_explicit_w = styles[item_node].width.is_none();
            let item_main = main_sizes[idx];

            let child_box = items[i].layout.as_mut().unwrap();
            let lead_offset = justify_offset_before_item(justify, idx, count, remaining);

            if is_row {
                if is_reverse {
                    let x_pos = effective_main - lead_offset - running_main - item_main;
                    child_box.x = bw + parent.padding.left + x_pos + child_box.margin.left;
                } else {
                    child_box.x = bw
                        + parent.padding.left
                        + lead_offset
                        + running_main
                        + child_box.margin.left;
                }

                let item_h = child_box.height + child_box.margin.top + child_box.margin.bottom;
                let cross_offset = match item_align {
                    AlignItems::FlexStart => 0,
                    AlignItems::FlexEnd => (cross_max - item_h).max(0),
                    AlignItems::Center => (cross_max - item_h).max(0) / 2,
                    AlignItems::Stretch => {
                        if no_explicit_h {
                            let stretched_h =
                                cross_max - child_box.margin.top - child_box.margin.bottom;
                            if styles[item_node].writing_mode.is_vertical() {
                                let relaid = build_block_with_forced_outer_height(
                                    dom,
                                    styles,
                                    pseudo,
                                    item_node,
                                    stretched_h.max(child_box.width),
                                    images,
                                    viewport_w,
                                    definite_container_height.unwrap_or(0),
                                    stretched_h,
                                );
                                *child_box = relaid;
                            }
                            child_box.height = stretched_h;
                        }
                        0
                    }
                    AlignItems::Baseline => {
                        let baseline = child_box.margin.top
                            + flex_item_baseline(&styles[item_node], child_box, true);
                        (line_baseline - baseline).max(0)
                    }
                };
                child_box.y = cross_cursor + cross_offset + child_box.margin.top;
            } else {
                if is_reverse {
                    let y_pos = effective_main - lead_offset - running_main - item_main;
                    child_box.y = cross_cursor + y_pos + child_box.margin.top;
                } else {
                    child_box.y = cross_cursor + lead_offset + running_main + child_box.margin.top;
                }

                let item_w = child_box.width + child_box.margin.left + child_box.margin.right;
                // §8.1: auto margins on cross axis take priority over align-self.
                let st = &styles[item_node];
                let has_auto_margin_lr = st.margin_left_auto || st.margin_right_auto;
                let cross_offset = if has_auto_margin_lr {
                    let remaining = (available_width - item_w).max(0);
                    if st.margin_left_auto && st.margin_right_auto {
                        child_box.margin.left = remaining / 2;
                        child_box.margin.right = remaining - child_box.margin.left;
                        0
                    } else if st.margin_left_auto {
                        child_box.margin.left = remaining;
                        0
                    } else {
                        child_box.margin.right = remaining;
                        0
                    }
                } else {
                    match item_align {
                        AlignItems::FlexStart => 0,
                        AlignItems::FlexEnd => (available_width - item_w).max(0),
                        AlignItems::Center => (available_width - item_w).max(0) / 2,
                        AlignItems::Stretch => {
                            if no_explicit_w {
                                child_box.width = available_width
                                    - child_box.margin.left
                                    - child_box.margin.right;
                            }
                            0
                        }
                        AlignItems::Baseline => 0,
                    }
                }; // close match + else
                child_box.x = bw + parent.padding.left + cross_offset + child_box.margin.left;
            }

            running_main += item_main + gap;
        }

        // Move items into parent.
        for i in line.start..line.end {
            if let Some(child_box) = items[i].layout.take() {
                parent.children.push(child_box);
            }
        }

        line.cross_size = cross_max;
        cross_cursor += cross_max + cross_gap;
        // For flex-col: track max height (used_main) across all flex columns.
        if !is_row {
            max_col_main = max_col_main.max(used_main);
        }
    }

    // Apply align-content: redistribute cross-axis space between flex lines.
    // Only meaningful when wrapping AND there's a definite container height
    // with extra space to distribute. If height is auto, lines keep natural sizes.
    if lines.len() > 1 && is_row && parent_style.height.is_some() {
        let total_lines_cross: i32 = lines.iter().map(|l| l.cross_size).sum();
        let total_gaps_cross = cross_gap * (lines.len() as i32 - 1).max(0);
        let total_cross_used = total_lines_cross + total_gaps_cross;
        let container_cross = parent_style.height.unwrap();
        let content_cross = container_cross - parent.padding.top - parent.padding.bottom - 2 * bw;
        let free = content_cross - total_cross_used;

        if free > 0 {
            let line_count = lines.len() as i32;

            // Compute per-line offsets based on align-content.
            let mut line_offsets: Vec<i32> = Vec::with_capacity(lines.len());
            match align_content {
                AlignContent::FlexStart => {
                    for _ in 0..lines.len() {
                        line_offsets.push(0);
                    }
                }
                AlignContent::FlexEnd => {
                    let shift = free.max(0);
                    for _ in 0..lines.len() {
                        line_offsets.push(shift);
                    }
                }
                AlignContent::Center => {
                    let shift = free.max(0) / 2;
                    for _ in 0..lines.len() {
                        line_offsets.push(shift);
                    }
                }
                AlignContent::SpaceBetween => {
                    let gap_extra = if line_count > 1 {
                        free.max(0) / (line_count - 1)
                    } else {
                        0
                    };
                    for li in 0..lines.len() {
                        line_offsets.push(gap_extra * li as i32);
                    }
                }
                AlignContent::SpaceAround => {
                    let per = free.max(0) / line_count.max(1);
                    for li in 0..lines.len() {
                        line_offsets.push(per / 2 + per * li as i32);
                    }
                }
                AlignContent::SpaceEvenly => {
                    let per = free.max(0) / (line_count + 1).max(1);
                    for li in 0..lines.len() {
                        line_offsets.push(per * (li as i32 + 1));
                    }
                }
                AlignContent::Stretch => {
                    // Distribute extra space equally to each line.
                    let extra_per_line = if line_count > 0 {
                        free.max(0) / line_count
                    } else {
                        0
                    };
                    // Each line grows, so subsequent lines shift by accumulated growth.
                    for li in 0..lines.len() {
                        line_offsets.push(extra_per_line * li as i32);
                    }
                    // Also grow each line's items to the new cross size,
                    // BUT only items WITHOUT a definite height (per CSS spec).
                    if extra_per_line > 0 {
                        let mut child_idx = 0;
                        for li in 0..lines.len() {
                            let item_count = lines[li].end - lines[li].start;
                            let new_cross = lines[li].cross_size + extra_per_line;
                            for ii in lines[li].start..lines[li].end {
                                let item_node = items[ii].node_id;
                                let item_st = &styles[item_node];
                                // Skip items with definite height — they keep their size.
                                if item_st.height.is_none() && child_idx < parent.children.len() {
                                    let child = &mut parent.children[child_idx];
                                    let item_h =
                                        child.height + child.margin.top + child.margin.bottom;
                                    if item_h < new_cross {
                                        child.height =
                                            new_cross - child.margin.top - child.margin.bottom;
                                    }
                                }
                                child_idx += 1;
                            }
                        }
                    }
                }
            }

            // Apply the computed offsets to children, grouped by line.
            let mut child_idx = 0;
            for (li, line) in lines.iter().enumerate() {
                let item_count = line.end - line.start;
                let offset = if li < line_offsets.len() {
                    line_offsets[li]
                } else {
                    0
                };
                if offset != 0 {
                    for _ in 0..item_count {
                        if child_idx < parent.children.len() {
                            parent.children[child_idx].y += offset;
                        }
                        child_idx += 1;
                    }
                } else {
                    child_idx += item_count;
                }
            }
        }
    }

    let result = if is_row {
        cross_cursor
    } else {
        parent
            .children
            .iter()
            .map(|child| child.y + child.height + child.margin.bottom)
            .max()
            .unwrap_or(bw + parent.padding.top)
    };
    result
}
