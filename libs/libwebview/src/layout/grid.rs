//! CSS Grid layout: `layout_grid()` implements the CSS Grid Layout algorithm.
//!
//! This implementation covers the most common subset of the spec:
//! - Explicit track sizing via `grid-template-columns` / `grid-template-rows`
//! - `fr`, `px`, `%`, `rem`/`em` units and `auto` / `min-content` / `max-content`
//! - `minmax(min, max)` with fr, px, or auto max (§7.2)
//! - `fit-content(value)` approximated as `minmax(0, value)` (§7.2)
//! - `repeat(N, ...)`, `repeat(auto-fill, ...)`, `repeat(auto-fit, ...)` (§7.1)
//! - `grid-template-areas` → named area placement (§7.3, case-sensitive)
//! - `grid-area: areaName` → resolved against template areas at layout time
//! - Named `GridLine::Named` resolved to 1-based indices via `resolve_named_area`
//! - `grid-template` shorthand: simple (rows / cols) and interleaved (areas + row sizes / cols)
//! - Explicit item placement with `grid-column-start/end` and `grid-row-start/end`
//! - Auto-placement (row-major scanning, left-to-right, top-to-bottom) (§8)
//! - `column-gap` / `row-gap` between tracks
//! - `justify-items` / `align-items` within each cell
//!
//! Known limitations (future work):
//! - Named grid lines (`[line-name]` syntax) — only area names via Named(String) are resolved
//! - Subgrid still lacks full CSS Grid Level 2 sizing and placement behavior
//! - `grid-auto-flow: column` dense packing is not implemented

use alloc::vec;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId};
use crate::style::{
    AlignContent, AlignItems, BoxSizing, ComputedStyle, Display, GridArea, GridLine,
    GridTrackSize, JustifyContent, Position, PseudoStyles,
};
use crate::ImageCache;

use super::block::build_block;
use super::{apply_transform_translation, LayoutBox};

// ────────────────────────────────────────────────────────────
// Public entry-point
// ────────────────────────────────────────────────────────────

/// Lay out `child_ids` as a grid container inside `parent` and return total
/// height consumed by the grid (not including the parent's own padding/border).
pub fn layout_grid(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
    inherited_subgrid_cols: Option<&[i32]>,
    inherited_subgrid_col_gap: Option<i32>,
    inherited_subgrid_rows: Option<&[i32]>,
    inherited_subgrid_row_gap: Option<i32>,
) -> i32 {
    let parent_idx = parent.node_id.unwrap_or(0);
    let parent_style = &styles[parent_idx];

    let uses_subgrid_cols = matches!(
        parent_style.grid_template_columns.as_slice(),
        [GridTrackSize::Subgrid]
    );
    let col_gap = if uses_subgrid_cols {
        inherited_subgrid_col_gap.unwrap_or(parent_style.column_gap)
    } else {
        parent_style.column_gap
    };
    let uses_subgrid_rows = matches!(
        parent_style.grid_template_rows.as_slice(),
        [GridTrackSize::Subgrid]
    );
    let row_gap = if uses_subgrid_rows {
        inherited_subgrid_row_gap.unwrap_or(parent_style.row_gap)
    } else {
        parent_style.row_gap
    };
    let container_align = parent_style.align_items;
    let align_content = parent_style.align_content;
    let justify_content = parent_style.justify_content;
    let justify_items = parent_style.justify_items;

    // ── 1. Resolve column track sizes ──────────────────────────────────────
    // Expand auto-fill / auto-fit into concrete 1fr tracks based on the
    // available container width and the minimum item width.
    let resolved_templates: Vec<GridTrackSize>;
    let col_templates: &[GridTrackSize] = {
        let src = &parent_style.grid_template_columns;
        if uses_subgrid_cols {
            &[]
        } else if src.len() == 1 {
            match src[0] {
                GridTrackSize::AutoFill { min_px } | GridTrackSize::AutoFit { min_px } => {
                    let min_px = min_px.max(1);
                    let num_cols =
                        ((available_width + col_gap) / (min_px + col_gap)).max(1) as usize;
                    resolved_templates = vec![GridTrackSize::Fr(100); num_cols];
                    &resolved_templates
                }
                _ => src.as_slice(),
            }
        } else {
            src.as_slice()
        }
    };
    let auto_col = &parent_style.grid_auto_columns;

    // ── 2. Collect visible, non-absolutely-positioned children ────────────
    // Per CSS Grid §4: whitespace-only text nodes do not generate grid items.
    let mut items: Vec<GridItem> = child_ids
        .iter()
        .filter_map(|&cid| {
            let st = &styles[cid];
            if st.display == Display::None {
                return None;
            }
            if matches!(st.position, Position::Absolute | Position::Fixed) {
                return None;
            }
            // Skip whitespace-only text nodes (CSS Grid §4) and SVG raw text.
            if let crate::dom::NodeType::Text(ref t) = dom.get(cid).node_type {
                if t.bytes()
                    .all(|b| b == b' ' || b == b'\n' || b == b'\r' || b == b'\t')
                {
                    return None;
                }
                if super::is_inside_svg(dom, cid) {
                    return None;
                }
            }
            Some(GridItem {
                node_id: cid,
                col_start: st.grid_column_start.clone(),
                col_end: st.grid_column_end.clone(),
                row_start: st.grid_row_start.clone(),
                row_end: st.grid_row_end.clone(),
                placed_col: 0,
                placed_row: 0,
                span_cols: 1,
                span_rows: 1,
                layout: None,
            })
        })
        .collect();

    if items.is_empty() {
        // No in-flow children, but still need to handle absolutely-positioned
        // children which form their containing block from this grid container.
        // Compute the grid's intrinsic content height from grid-template-rows
        // (even without items, the explicit grid defines a content area).
        // Then clamp by max-height (parent.height was already set by build_block
        // before layout_grid was called).
        let template_h: i32 = parent_style
            .grid_template_rows
            .iter()
            .map(|t| match t {
                GridTrackSize::Px(px) => *px,
                GridTrackSize::Minmax { min_px, .. } => *min_px,
                _ => 0,
            })
            .sum();
        let row_gap_total = if parent_style.grid_template_rows.len() > 1 {
            row_gap * (parent_style.grid_template_rows.len() as i32 - 1)
        } else {
            0
        };
        let computed_h = parent_style.height.unwrap_or(template_h + row_gap_total);
        // Clamp by max-height if set (CSS Grid container intrinsic sizing).
        let h = if let Some(max_h) = parent_style.max_height {
            computed_h.min(max_h)
        } else {
            computed_h
        };
        layout_grid_abs_children(
            dom,
            styles,
            pseudo,
            child_ids,
            parent,
            images,
            viewport_w,
            available_width,
            h,
        );
        return 0;
    }

    // ── 3. Resolve named grid areas ───────────────────────────────────────
    // If the parent has `grid-template-areas`, resolve `GridLine::Named` and
    // `grid-area: areaName` to explicit line indices.
    let template_areas = &parent_style.grid_template_areas;
    if !template_areas.is_empty() {
        for item in &mut items {
            resolve_named_area(
                &mut item.col_start,
                &mut item.col_end,
                &mut item.row_start,
                &mut item.row_end,
                template_areas,
            );
        }
    }

    // ── 4. Determine number of explicit columns ──────────────────────────
    // The explicit grid has as many columns as `grid-template-columns` defines
    // (minimum 1).  Items that exceed the explicit grid extend it implicitly.
    // If grid-template-areas defines more columns, use that.
    let areas_max_col = template_areas.iter().map(|a| a.col_end).max().unwrap_or(0) as usize;
    let areas_max_row = template_areas.iter().map(|a| a.row_end).max().unwrap_or(0) as usize;
    let explicit_cols = inherited_subgrid_cols
        .map(|cols| cols.len().max(1))
        .unwrap_or_else(|| col_templates.len().max(1))
        .max(areas_max_col.saturating_sub(1));
    let explicit_rows = inherited_subgrid_rows
        .map(|rows| rows.len().max(1))
        .unwrap_or_else(|| row_templates_len(parent_style).max(1))
        .max(areas_max_row.saturating_sub(1));

    // ── 5. Auto-place all items ──────────────────────────────────────────
    auto_place(
        &mut items,
        explicit_cols,
        if uses_subgrid_cols {
            Some(explicit_cols)
        } else {
            None
        },
        if uses_subgrid_rows {
            Some(explicit_rows)
        } else {
            None
        },
    );

    // Total column count needed (explicit + implicit).
    let total_cols = if uses_subgrid_cols {
        inherited_subgrid_cols
            .map(|cols| cols.len().max(1))
            .unwrap_or(explicit_cols)
    } else {
        items
            .iter()
            .map(|it| it.placed_col + it.span_cols)
            .max()
            .unwrap_or(1)
    };

    // ── 5. Resolve column pixel widths ────────────────────────────────────
    let col_widths = if uses_subgrid_cols {
        inherited_subgrid_cols
            .map(|cols| cols.to_vec())
            .unwrap_or_else(|| vec![available_width.max(0)])
    } else {
        resolve_col_widths(
            col_templates,
            auto_col,
            total_cols,
            available_width,
            col_gap,
        )
    };

    // ── 6. Total row count ────────────────────────────────────────────────
    let total_rows = if uses_subgrid_rows {
        inherited_subgrid_rows
            .map(|rows| rows.len().max(1))
            .unwrap_or(1)
    } else {
        items
            .iter()
            .map(|it| it.placed_row + it.span_rows)
            .max()
            .unwrap_or(1)
    };

    let row_templates = &parent_style.grid_template_rows;
    let auto_row = &parent_style.grid_auto_rows;

    // ── 7. Measure each item at its column span width ─────────────────────
    for item in &mut items {
        let col_w = span_width(&col_widths, item.placed_col, item.span_cols, col_gap);
        let item_style = &styles[item.node_id];
        let effective_justify = item_style.justify_self.unwrap_or(justify_items);
        let use_fit_content_width = effective_justify != AlignItems::Stretch
            && item_style.width.is_none()
            && item_style.width_pct.is_none()
            && item_style.width_calc.is_none()
            && !item_style.width_max_content
            && !item_style.width_min_content
            && !item_style.width_fit_content;
        let item_avail = if use_fit_content_width {
            super::shrink_to_fit_width(
                dom,
                styles,
                pseudo,
                item.node_id,
                col_w,
                images,
                viewport_w,
            )
        } else {
            col_w
        };
        let mut bx = build_block(
            dom,
            styles,
            pseudo,
            item.node_id,
            item_avail,
            images,
            viewport_w,
            0,
        );
        relayout_subgrid_columns_if_needed(
            dom,
            styles,
            pseudo,
            &mut bx,
            images,
            viewport_w,
            &col_widths[item.placed_col..(item.placed_col + item.span_cols).min(col_widths.len())],
            col_gap,
        );
        item.layout = Some(bx);
    }

    // ── 8. Resolve row heights ────────────────────────────────────────────
    let mut row_heights = if uses_subgrid_rows {
        inherited_subgrid_rows
            .map(|rows| rows.to_vec())
            .unwrap_or_else(|| vec![0; total_rows])
    } else {
        resolve_row_heights(row_templates, auto_row, total_rows, &items)
    };

    for item in &mut items {
        if let Some(ref mut bx) = item.layout {
            relayout_subgrid_rows_if_needed(
                dom,
                styles,
                pseudo,
                bx,
                images,
                viewport_w,
                &col_widths
                    [item.placed_col..(item.placed_col + item.span_cols).min(col_widths.len())],
                col_gap,
                &row_heights
                    [item.placed_row..(item.placed_row + item.span_rows).min(row_heights.len())],
                row_gap,
            );
        }
    }

    // ── 9. Position every item ────────────────────────────────────────────
    let base_grid_w = tracks_total(&col_widths, col_gap);
    let available_grid_w = available_width.max(0);
    let (content_x_offset, extra_col_gap) = distribute_grid_content_inline(
        justify_content,
        available_grid_w - base_grid_w,
        col_widths.len(),
    );

    let base_grid_h = tracks_total(&row_heights, row_gap);
    let available_grid_h = definite_grid_content_height(parent_style, parent.height, base_grid_h);
    expand_fr_rows(
        &mut row_heights,
        row_templates,
        auto_row,
        available_grid_h,
        row_gap,
    );
    let base_grid_h = tracks_total(&row_heights, row_gap);
    let (content_y_offset, extra_row_gap) = distribute_grid_content_block(
        align_content,
        available_grid_h - base_grid_h,
        row_heights.len(),
    );
    if align_content == AlignContent::Stretch && base_grid_h < available_grid_h {
        stretch_auto_rows(&mut row_heights, row_templates, auto_row, available_grid_h - base_grid_h);
    }
    if justify_content == JustifyContent::FlexStart && base_grid_w < available_grid_w {
        // Nothing to do; keep the historical behavior for the default.
    }

    let row_offsets: Vec<i32> = {
        let mut offsets = Vec::with_capacity(total_rows);
        let mut y = content_y_offset;
        for r in 0..total_rows {
            offsets.push(y);
            y += row_heights[r] + if r + 1 < total_rows { row_gap + extra_row_gap } else { 0 };
        }
        offsets
    };
    // Recompute total height from row_offsets.
    let cursor_y = if !row_offsets.is_empty() {
        let last_row = total_rows - 1;
        row_offsets[last_row] + row_heights[last_row]
    } else {
        0
    };

    for item in &mut items {
        let x = content_x_offset + col_offset(&col_widths, item.placed_col, col_gap + extra_col_gap);
        let y = row_offsets[item.placed_row];
        let cell_w = span_width(&col_widths, item.placed_col, item.span_cols, col_gap + extra_col_gap);
        let cell_h = span_height(&row_heights, item.placed_row, item.span_rows, row_gap + extra_row_gap);

        if let Some(mut bx) = item.layout.take() {
            let item_w = bx.width;
            let item_h = bx.height;
            let item_style = &styles[item.node_id];
            let effective_justify = item_style.justify_self.unwrap_or(justify_items);
            let effective_align = item_style.align_self.unwrap_or(container_align);

            // Horizontal alignment (justify-self falling back to justify-items).
            let x_offset = item_axis_offset_with_auto_margins(
                effective_justify,
                item_w,
                cell_w,
                item_style.margin_left_auto,
                item_style.margin_right_auto,
            );
            // Vertical alignment (align-self falling back to align-items).
            let y_offset = item_axis_offset_with_auto_margins(
                effective_align,
                item_h,
                cell_h,
                item_style.margin_top_auto,
                item_style.margin_bottom_auto,
            );

            // Position the item box at its grid cell offset.
            // Do NOT use translate_box (recursive) — flatten() accumulates
            // parent offsets naturally, so only the item's own x/y needs to
            // be set.  translate_box would shift descendants twice.
            bx.x = x + x_offset;
            bx.y = y + y_offset;

            parent.children.push(bx);
        }
    }

    // ── 6. Handle absolutely-positioned grid children ─────────────────────
    layout_grid_abs_children(
        dom,
        styles,
        pseudo,
        child_ids,
        parent,
        images,
        viewport_w,
        available_width,
        cursor_y,
    );

    cursor_y
}

fn relayout_subgrid_columns_if_needed(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    bx: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
    inherited_cols: &[i32],
    inherited_col_gap: i32,
) {
    let Some(node_id) = bx.node_id else {
        return;
    };
    let style = &styles[node_id];
    if !matches!(style.display, Display::Grid | Display::InlineGrid) {
        return;
    }
    if !matches!(
        style.grid_template_columns.as_slice(),
        [GridTrackSize::Subgrid]
    ) {
        return;
    }
    if inherited_cols.is_empty() {
        return;
    }

    let child_ids: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    let border2 = bx.border_width * 2;
    let inner_w = inherited_cols.iter().sum::<i32>()
        + inherited_col_gap * (inherited_cols.len().saturating_sub(1) as i32);
    bx.children.clear();
    let content_h = layout_grid(
        dom,
        styles,
        pseudo,
        &child_ids,
        inner_w.max(0),
        bx,
        images,
        viewport_w,
        Some(inherited_cols),
        Some(inherited_col_gap),
        None,
        None,
    );

    if style.height.is_none()
        && style.height_pct.is_none()
        && style.height_calc.is_none()
        && style.aspect_ratio <= 0
    {
        bx.height = content_h + bx.padding.bottom + bx.border_width;
        let is_border_box = matches!(style.box_sizing, BoxSizing::BorderBox);
        if let Some(max_h) = style.max_height {
            let max_outer = if is_border_box {
                max_h
            } else {
                max_h + bx.padding.top + bx.padding.bottom + border2
            };
            if bx.height > max_outer {
                bx.height = max_outer;
            }
        }
        if style.min_height > 0 {
            let min_outer = if is_border_box {
                style.min_height
            } else {
                style.min_height + bx.padding.top + bx.padding.bottom + border2
            };
            if bx.height < min_outer {
                bx.height = min_outer;
            }
        }
    }
}

fn relayout_subgrid_rows_if_needed(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    bx: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
    inherited_cols: &[i32],
    inherited_col_gap: i32,
    inherited_rows: &[i32],
    inherited_row_gap: i32,
) {
    let Some(node_id) = bx.node_id else {
        return;
    };
    let style = &styles[node_id];
    if !matches!(style.display, Display::Grid | Display::InlineGrid) {
        return;
    }
    if !matches!(
        style.grid_template_rows.as_slice(),
        [GridTrackSize::Subgrid]
    ) {
        return;
    }
    if inherited_rows.is_empty() {
        return;
    }

    let child_ids: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    let border2 = bx.border_width * 2;
    let inner_w = (bx.width - bx.padding.left - bx.padding.right - border2).max(0);
    bx.children.clear();
    let content_h = layout_grid(
        dom,
        styles,
        pseudo,
        &child_ids,
        inner_w,
        bx,
        images,
        viewport_w,
        if matches!(
            style.grid_template_columns.as_slice(),
            [GridTrackSize::Subgrid]
        ) {
            Some(inherited_cols)
        } else {
            None
        },
        if matches!(
            style.grid_template_columns.as_slice(),
            [GridTrackSize::Subgrid]
        ) {
            Some(inherited_col_gap)
        } else {
            None
        },
        Some(inherited_rows),
        Some(inherited_row_gap),
    );

    if style.height.is_none()
        && style.height_pct.is_none()
        && style.height_calc.is_none()
        && style.aspect_ratio <= 0
    {
        bx.height = content_h + bx.padding.bottom + bx.border_width;
        let is_border_box = matches!(style.box_sizing, BoxSizing::BorderBox);
        if let Some(max_h) = style.max_height {
            let max_outer = if is_border_box {
                max_h
            } else {
                max_h + bx.padding.top + bx.padding.bottom + border2
            };
            if bx.height > max_outer {
                bx.height = max_outer;
            }
        }
        if style.min_height > 0 {
            let min_outer = if is_border_box {
                style.min_height
            } else {
                style.min_height + bx.padding.top + bx.padding.bottom + border2
            };
            if bx.height < min_outer {
                bx.height = min_outer;
            }
        }
    }
}

/// Lay out absolutely-positioned children of a grid container.
/// Per CSS Grid §9: abs children have the grid container's content area as
/// their containing block (when the container is positioned).
fn layout_grid_abs_children(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    parent: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
    available_width: i32,
    grid_content_h: i32,
) {
    let bw = parent.border_width;
    // For width: prefer parent.width if set, else available_width.
    let parent_w = if parent.width > 0 {
        parent.width
    } else {
        available_width
    };
    let cb_w = (parent_w - parent.padding.left - parent.padding.right - 2 * bw).max(0);
    let cb_h = grid_content_h.max(0);
    let content_x = bw + parent.padding.left;
    let content_y = bw + parent.padding.top;

    for &cid in child_ids {
        let st = &styles[cid];
        if !matches!(st.position, Position::Absolute | Position::Fixed) {
            continue;
        }
        if st.display == Display::None {
            continue;
        }

        let sizing_width = if st.left_offset.is_some()
            && st.right_offset.is_some()
            && st.width.is_none()
            && st.width_pct.is_none()
        {
            (cb_w - st.left_offset.unwrap_or(0) - st.right_offset.unwrap_or(0)).max(0)
        } else {
            cb_w
        };

        let mut abs_box = build_block(
            dom,
            styles,
            pseudo,
            cid,
            sizing_width,
            images,
            viewport_w,
            cb_h,
        );

        let l = st.left_offset.unwrap_or(0);
        let t = st.top.unwrap_or(0);
        abs_box.x = content_x + l + abs_box.margin.left;
        abs_box.y = content_y + t + abs_box.margin.top;

        if st.left_offset.is_none() {
            if let Some(r) = st.right_offset {
                abs_box.x = content_x + cb_w - r - abs_box.width - abs_box.margin.right;
            }
        }
        if st.top.is_none() {
            if let Some(b) = st.bottom_offset {
                abs_box.y = content_y + cb_h - b - abs_box.height - abs_box.margin.bottom;
            }
        }

        apply_transform_translation(&mut abs_box, st);
        abs_box.is_out_of_flow = true;
        abs_box.is_positioned = true;
        parent.children.push(abs_box);
    }
}

// ────────────────────────────────────────────────────────────
// Auto-placement algorithm (row-major)
// ────────────────────────────────────────────────────────────

/// Resolve span size from a pair of GridLine values relative to the explicit
/// grid width.  Returns (start_or_none, span_size).
fn resolve_span(start: &GridLine, end: &GridLine, _explicit: usize) -> (Option<usize>, usize) {
    match (start, end) {
        (GridLine::Index(s), GridLine::Index(e)) => {
            let s = (*s - 1).max(0) as usize;
            let span = ((*e - 1).max(0) as usize).saturating_sub(s).max(1);
            (Some(s), span)
        }
        (GridLine::Index(s), GridLine::Span(n)) => {
            let s = (*s - 1).max(0) as usize;
            (Some(s), (*n).max(1) as usize)
        }
        (GridLine::Index(s), GridLine::Auto) => {
            let s = (*s - 1).max(0) as usize;
            (Some(s), 1)
        }
        (GridLine::Auto, GridLine::Index(e)) => {
            let span = 1usize;
            let _ = span;
            (None, ((*e - 1).max(1)) as usize)
        }
        (GridLine::Auto, GridLine::Span(n)) => (None, (*n).max(1) as usize),
        (GridLine::Span(n), _) => (None, (*n).max(1) as usize),
        // Named lines that weren't resolved — treat as auto.
        _ => (None, 1),
    }
}

/// Place all grid items using the CSS Grid auto-placement algorithm
/// (row-major, left-to-right, no dense packing).
/// Resolve `GridLine::Named` values against the template areas.
/// If all four lines of an item are Named with the same name, look up the area
/// and set explicit col/row start/end.
fn resolve_named_area(
    col_start: &mut GridLine,
    col_end: &mut GridLine,
    row_start: &mut GridLine,
    row_end: &mut GridLine,
    areas: &[GridArea],
) {
    // Case 1: grid-area: areaName → all four are Named("areaName")
    // Per CSS Grid spec §7.3: <custom-ident> values are case-sensitive.
    if let GridLine::Named(ref name) = row_start {
        if let Some(area) = areas.iter().find(|a| a.name == *name) {
            *row_start = GridLine::Index(area.row_start);
            *row_end = GridLine::Index(area.row_end);
            // Also set col if not already explicit.
            if matches!(col_start, GridLine::Auto | GridLine::Named(_)) {
                *col_start = GridLine::Index(area.col_start);
            }
            if matches!(col_end, GridLine::Auto | GridLine::Named(_)) {
                *col_end = GridLine::Index(area.col_end);
            }
            return;
        }
    }
    // Case 2: Individual named lines — case-sensitive per CSS Grid §7.3.
    if let GridLine::Named(ref name) = col_start {
        if let Some(area) = areas.iter().find(|a| a.name == *name) {
            *col_start = GridLine::Index(area.col_start);
        }
    }
    if let GridLine::Named(ref name) = col_end {
        if let Some(area) = areas.iter().find(|a| a.name == *name) {
            *col_end = GridLine::Index(area.col_end);
        }
    }
    if let GridLine::Named(ref name) = row_start {
        if let Some(area) = areas.iter().find(|a| a.name == *name) {
            *row_start = GridLine::Index(area.row_start);
        }
    }
    if let GridLine::Named(ref name) = row_end {
        if let Some(area) = areas.iter().find(|a| a.name == *name) {
            *row_end = GridLine::Index(area.row_end);
        }
    }
}

fn auto_place(
    items: &mut Vec<GridItem>,
    explicit_cols: usize,
    max_cols: Option<usize>,
    max_rows: Option<usize>,
) {
    // Grid occupancy map: (col, row) → occupied.
    // We use a simple Vec and grow it as needed.
    let mut occupied: Vec<Vec<bool>> = vec![vec![false; explicit_cols.max(1)]]; // [row][col]

    // Pre-pass: resolve items with fully explicit positions.
    for item in items.iter_mut() {
        let (col_start, span_cols) = resolve_span(&item.col_start, &item.col_end, explicit_cols);
        let (row_start, span_rows) = resolve_span(&item.row_start, &item.row_end, explicit_cols);
        item.span_cols = clamp_span(span_cols.max(1), max_cols);
        item.span_rows = clamp_span(span_rows.max(1), max_rows);

        if let (Some(c), Some(r)) = (col_start, row_start) {
            item.placed_col = clamp_start(c, item.span_cols, max_cols);
            item.placed_row = clamp_start(r, item.span_rows, max_rows);
            mark_occupied(
                &mut occupied,
                item.placed_row,
                item.placed_col,
                item.span_rows,
                item.span_cols,
            );
        }
    }

    // Second pass: auto-place items without fully explicit positions.
    let mut auto_cursor_row = 0usize;
    let mut auto_cursor_col = 0usize;

    for item in items.iter_mut() {
        let col_start = match item.col_start {
            GridLine::Index(n) => Some(clamp_start(
                (n - 1).max(0) as usize,
                item.span_cols,
                max_cols,
            )),
            _ => None,
        };
        let row_start = match item.row_start {
            GridLine::Index(n) => Some(clamp_start(
                (n - 1).max(0) as usize,
                item.span_rows,
                max_rows,
            )),
            _ => None,
        };

        // Already fully placed above.
        if col_start.is_some() && row_start.is_some() {
            continue;
        }

        let span_c = item.span_cols;
        let span_r = item.span_rows;
        let num_cols = explicit_cols.max(1);

        // Find a slot.
        let (placed_r, placed_c) = find_slot(
            &occupied,
            &mut auto_cursor_row,
            &mut auto_cursor_col,
            span_r,
            span_c,
            num_cols,
            max_rows,
            col_start,
            row_start,
        );
        item.placed_row = placed_r;
        item.placed_col = placed_c;
        mark_occupied(&mut occupied, placed_r, placed_c, span_r, span_c);
        // Advance cursor past this item.
        auto_cursor_col = placed_c + span_c;
        if auto_cursor_col >= num_cols {
            auto_cursor_col = 0;
            auto_cursor_row = placed_r + 1;
        }
    }
}

/// Grow occupied grid if necessary and mark cells as used.
fn ensure_rows(occupied: &mut Vec<Vec<bool>>, row: usize, cols: usize) {
    while occupied.len() <= row {
        occupied.push(vec![false; cols]);
    }
    // Widen existing rows if the grid grew.
    for r in occupied.iter_mut() {
        while r.len() < cols {
            r.push(false);
        }
    }
}

fn mark_occupied(
    occupied: &mut Vec<Vec<bool>>,
    row: usize,
    col: usize,
    span_r: usize,
    span_c: usize,
) {
    let cols = occupied.first().map(|r| r.len()).unwrap_or(1);
    let max_col = col + span_c;
    ensure_rows(occupied, row + span_r - 1, max_col.max(cols));
    for r in row..row + span_r {
        for c in col..col + span_c {
            if c < occupied[r].len() {
                occupied[r][c] = true;
            }
        }
    }
}

/// Find the next available slot for an item with given span,
/// scanning row-major from the current cursor.
fn find_slot(
    occupied: &Vec<Vec<bool>>,
    cursor_row: &mut usize,
    cursor_col: &mut usize,
    span_r: usize,
    span_c: usize,
    num_cols: usize,
    max_rows: Option<usize>,
    fixed_col: Option<usize>,
    fixed_row: Option<usize>,
) -> (usize, usize) {
    let mut r = fixed_row.unwrap_or(*cursor_row);
    let mut c = if let Some(fc) = fixed_col {
        fc
    } else {
        *cursor_col
    };

    loop {
        if c + span_c > num_cols {
            // Wrap to next row.
            c = if fixed_col.is_some() {
                fixed_col.unwrap()
            } else {
                0
            };
            if fixed_row.is_some() {
                break;
            }
            r += 1;
        }
        if let Some(limit_rows) = max_rows {
            if r + span_r > limit_rows {
                break;
            }
        }
        if fits(occupied, r, c, span_r, span_c) {
            return (r, c);
        }
        if fixed_row.is_some() {
            c += 1;
        } else if fixed_col.is_some() {
            r += 1;
        } else {
            c += 1;
        }
    }

    (
        clamp_start(fixed_row.unwrap_or(*cursor_row), span_r, max_rows),
        clamp_start(fixed_col.unwrap_or(*cursor_col), span_c, Some(num_cols)),
    )
}

/// Check whether a span fits at (row, col) without overlap.
fn fits(occupied: &Vec<Vec<bool>>, row: usize, col: usize, span_r: usize, span_c: usize) -> bool {
    for r in row..row + span_r {
        if r >= occupied.len() {
            continue;
        } // empty row = free
        let row_data = &occupied[r];
        for c in col..col + span_c {
            if c < row_data.len() && row_data[c] {
                return false;
            }
        }
    }
    true
}

fn clamp_span(span: usize, limit: Option<usize>) -> usize {
    if let Some(limit) = limit {
        span.min(limit.max(1)).max(1)
    } else {
        span.max(1)
    }
}

fn clamp_start(start: usize, span: usize, limit: Option<usize>) -> usize {
    if let Some(limit) = limit {
        let limit = limit.max(1);
        let max_start = limit.saturating_sub(span.min(limit));
        start.min(max_start)
    } else {
        start
    }
}

fn row_templates_len(style: &ComputedStyle) -> usize {
    style.grid_template_rows.len()
}

// ────────────────────────────────────────────────────────────
// Track sizing helpers
// ────────────────────────────────────────────────────────────

/// Resolve column widths in pixels from the track template + auto-column definition.
///
/// Algorithm:
/// 1. Assign fixed-px and percent tracks.
/// 2. Distribute remaining space proportionally among `fr` tracks (incl. Minmax with fr max).
/// 3. Fill `auto` / `MinContent` / `MaxContent` tracks with equal shares of remaining free space.
/// 4. Apply min_px floors from Minmax tracks.
fn resolve_col_widths(
    templates: &[GridTrackSize],
    auto_track: &GridTrackSize,
    total_cols: usize,
    container_width: i32,
    col_gap: i32,
) -> Vec<i32> {
    let mut widths: Vec<i32> = Vec::with_capacity(total_cols);

    // Extend template with auto_track for implicit columns.
    let track_for = |idx: usize| -> &GridTrackSize {
        if idx < templates.len() {
            &templates[idx]
        } else {
            auto_track
        }
    };

    let total_gap = col_gap * (total_cols.saturating_sub(1) as i32);
    let available = (container_width - total_gap).max(0);

    // First pass: fixed and percent; count fr units.
    let mut fixed_total = 0i32;
    let mut fr_total = 0i32; // sum of fr values (×100 fixed-point)
    for i in 0..total_cols {
        match track_for(i) {
            GridTrackSize::Px(px) => {
                widths.push(*px);
                fixed_total += px;
            }
            GridTrackSize::Percent(pct) => {
                let px = (available as i64 * *pct as i64 / 10000) as i32;
                widths.push(px);
                fixed_total += px;
            }
            GridTrackSize::Fr(f) => {
                widths.push(0);
                fr_total += f;
            }
            GridTrackSize::Minmax {
                min_px,
                max_px,
                max_is_fr,
            } => {
                if *max_is_fr {
                    // Behaves like fr(max_px) with a minimum floor of min_px.
                    widths.push(0);
                    fr_total += max_px;
                } else if *max_px < 0 {
                    // minmax(N, auto) — start at min_px, may grow with free space.
                    widths.push(*min_px);
                    fixed_total += min_px;
                } else {
                    // minmax(min, max_px) — use max_px as fixed size, floor at min_px.
                    let px = (*max_px).max(*min_px);
                    widths.push(px);
                    fixed_total += px;
                }
            }
            GridTrackSize::Auto
            | GridTrackSize::MinContent
            | GridTrackSize::MaxContent
            | GridTrackSize::AutoFill { .. }
            | GridTrackSize::AutoFit { .. }
            | GridTrackSize::Subgrid => {
                widths.push(0);
            }
        }
    }

    // Distribute free space to fr tracks.
    let free = (available - fixed_total).max(0);
    if fr_total > 0 {
        for i in 0..total_cols {
            match track_for(i) {
                GridTrackSize::Fr(f) => {
                    widths[i] = (free as i64 * *f as i64 / fr_total as i64) as i32;
                }
                GridTrackSize::Minmax {
                    min_px,
                    max_px,
                    max_is_fr: true,
                } => {
                    let fr_share = (free as i64 * *max_px as i64 / fr_total as i64) as i32;
                    widths[i] = fr_share.max(*min_px);
                }
                _ => {}
            }
        }
    } else {
        // No fr tracks — distribute remaining free space equally to auto tracks.
        let auto_count = (0..total_cols)
            .filter(|&i| {
                matches!(
                    track_for(i),
                    GridTrackSize::Auto
                        | GridTrackSize::MinContent
                        | GridTrackSize::MaxContent
                        | GridTrackSize::Subgrid
                )
            })
            .count() as i32;
        if auto_count > 0 {
            let share = free / auto_count;
            for i in 0..total_cols {
                if matches!(
                    track_for(i),
                    GridTrackSize::Auto
                        | GridTrackSize::MinContent
                        | GridTrackSize::MaxContent
                        | GridTrackSize::Subgrid
                ) {
                    widths[i] = share;
                }
            }
        }
    }

    widths
}

/// Resolve row heights: use the template where given, otherwise take the
/// maximum item height across all items in that row (content-sized).
fn resolve_row_heights(
    templates: &[GridTrackSize],
    auto_track: &GridTrackSize,
    total_rows: usize,
    items: &[GridItem],
) -> Vec<i32> {
    let track_for = |idx: usize| -> &GridTrackSize {
        if idx < templates.len() {
            &templates[idx]
        } else {
            auto_track
        }
    };

    let mut heights: Vec<i32> = vec![0; total_rows];

    // Pass 1: explicit sizes.
    for r in 0..total_rows {
        match track_for(r) {
            GridTrackSize::Px(px) => heights[r] = *px,
            GridTrackSize::Minmax {
                min_px,
                max_px,
                max_is_fr,
            } => {
                if !max_is_fr && *max_px >= 0 {
                    heights[r] = (*max_px).max(*min_px);
                } else {
                    // fr or auto max — enforce minimum, grow from content
                    heights[r] = *min_px;
                }
            }
            _ => {} // content-sized or fr — determined from items
        }
    }

    // Pass 2: content-size rows that are `auto`, `fr`, `MinContent`, or `MaxContent`.
    for item in items {
        let item_h = item.layout.as_ref().map(|b| b.height).unwrap_or(0);
        // Distribute the item height evenly across its row span.
        // For simplicity (sufficient for 99% of pages), attribute to the first row.
        let r = item.placed_row;
        if r < total_rows {
            match track_for(r) {
                GridTrackSize::Auto
                | GridTrackSize::Fr(_)
                | GridTrackSize::MinContent
                | GridTrackSize::MaxContent
                | GridTrackSize::Subgrid => {
                    if item_h > heights[r] {
                        heights[r] = item_h;
                    }
                }
                GridTrackSize::Minmax {
                    min_px,
                    max_is_fr: true,
                    ..
                }
                | GridTrackSize::Minmax {
                    min_px, max_px: -1, ..
                } => {
                    // fr or auto max — grow from content, respecting min floor
                    if item_h > heights[r] {
                        heights[r] = item_h.max(*min_px);
                    }
                }
                _ => {}
            }
        }
    }

    heights
}

// ────────────────────────────────────────────────────────────
// Geometry helpers
// ────────────────────────────────────────────────────────────

/// X offset of column `col` (0-based) within the grid.
fn col_offset(col_widths: &[i32], col: usize, col_gap: i32) -> i32 {
    let mut x = 0i32;
    for i in 0..col {
        x += col_widths.get(i).copied().unwrap_or(0) + col_gap;
    }
    x
}

/// Combined pixel width of `span` columns starting at `col`.
fn span_width(col_widths: &[i32], col: usize, span: usize, col_gap: i32) -> i32 {
    let mut w = 0i32;
    for i in col..col + span {
        if i > col {
            w += col_gap;
        }
        w += col_widths.get(i).copied().unwrap_or(0);
    }
    w.max(0)
}

/// Combined pixel height of `span` rows starting at `row`.
fn span_height(row_heights: &[i32], row: usize, span: usize, row_gap: i32) -> i32 {
    let mut h = 0i32;
    for i in row..row + span {
        if i > row {
            h += row_gap;
        }
        h += row_heights.get(i).copied().unwrap_or(0);
    }
    h.max(0)
}

fn tracks_total(tracks: &[i32], gap: i32) -> i32 {
    tracks.iter().copied().sum::<i32>() + gap * tracks.len().saturating_sub(1) as i32
}

fn definite_grid_content_height(style: &ComputedStyle, current_height: i32, content_height: i32) -> i32 {
    let mut h = current_height.max(content_height);
    if let Some(explicit) = style.height {
        h = h.max(explicit);
    } else if (style.height_pct.is_some() || style.height_calc.is_some()) && style.max_height.is_some() {
        // A grid container with a definite percentage/calc height may be clamped
        // by max-height after child layout. Use that clamped content height for
        // align-content before final box sizing, so tracks can be packed within
        // the same space the container will eventually occupy.
        h = h.max(style.max_height.unwrap_or(h));
    }
    h = h.max(style.min_height);
    if let Some(max_h) = style.max_height {
        h = h.min(max_h);
    }
    h.max(content_height)
}

fn distribute_grid_content_block(
    align: AlignContent,
    free_space: i32,
    track_count: usize,
) -> (i32, i32) {
    let free = free_space.max(0);
    match align {
        AlignContent::FlexEnd => (free, 0),
        AlignContent::Center => (free / 2, 0),
        AlignContent::SpaceBetween if track_count > 1 => {
            (0, free / (track_count.saturating_sub(1) as i32))
        }
        AlignContent::SpaceAround if track_count > 0 => {
            let gap = free / track_count as i32;
            (gap / 2, gap)
        }
        AlignContent::SpaceEvenly if track_count > 0 => {
            let gap = free / (track_count as i32 + 1);
            (gap, gap)
        }
        _ => (0, 0),
    }
}

fn distribute_grid_content_inline(
    justify: JustifyContent,
    free_space: i32,
    track_count: usize,
) -> (i32, i32) {
    let free = free_space.max(0);
    match justify {
        JustifyContent::FlexEnd => (free, 0),
        JustifyContent::Center => (free / 2, 0),
        JustifyContent::SpaceBetween if track_count > 1 => {
            (0, free / (track_count.saturating_sub(1) as i32))
        }
        JustifyContent::SpaceAround if track_count > 0 => {
            let gap = free / track_count as i32;
            (gap / 2, gap)
        }
        JustifyContent::SpaceEvenly if track_count > 0 => {
            let gap = free / (track_count as i32 + 1);
            (gap, gap)
        }
        _ => (0, 0),
    }
}

fn stretch_auto_rows(
    row_heights: &mut [i32],
    templates: &[GridTrackSize],
    auto_track: &GridTrackSize,
    free_space: i32,
) {
    let free = free_space.max(0);
    if free == 0 || row_heights.is_empty() {
        return;
    }
    let stretchable: Vec<usize> = (0..row_heights.len())
        .filter(|&idx| {
            let track = if idx < templates.len() {
                &templates[idx]
            } else {
                auto_track
            };
            matches!(
                track,
                GridTrackSize::Auto | GridTrackSize::Minmax { max_px: -1, .. }
            )
        })
        .collect();
    if stretchable.is_empty() {
        return;
    }
    let share = free / stretchable.len() as i32;
    let mut remainder = free - share * stretchable.len() as i32;
    for idx in stretchable {
        row_heights[idx] += share;
        if remainder > 0 {
            row_heights[idx] += 1;
            remainder -= 1;
        }
    }
}

fn expand_fr_rows(
    row_heights: &mut [i32],
    templates: &[GridTrackSize],
    auto_track: &GridTrackSize,
    container_height: i32,
    row_gap: i32,
) {
    if row_heights.is_empty() {
        return;
    }
    let total_gap = row_gap * row_heights.len().saturating_sub(1) as i32;
    let used = row_heights.iter().copied().sum::<i32>() + total_gap;
    let free = (container_height - used).max(0);
    if free == 0 {
        return;
    }

    let mut fr_total = 0i32;
    for idx in 0..row_heights.len() {
        let track = if idx < templates.len() {
            &templates[idx]
        } else {
            auto_track
        };
        match track {
            GridTrackSize::Fr(fr) => fr_total += *fr,
            GridTrackSize::Minmax {
                max_px,
                max_is_fr: true,
                ..
            } => fr_total += *max_px,
            _ => {}
        }
    }
    if fr_total <= 0 {
        return;
    }

    let mut distributed = 0i32;
    for idx in 0..row_heights.len() {
        let track = if idx < templates.len() {
            &templates[idx]
        } else {
            auto_track
        };
        let fr = match track {
            GridTrackSize::Fr(fr) => *fr,
            GridTrackSize::Minmax {
                max_px,
                max_is_fr: true,
                ..
            } => *max_px,
            _ => 0,
        };
        if fr <= 0 {
            continue;
        }
        let share = (free as i64 * fr as i64 / fr_total as i64) as i32;
        row_heights[idx] += share;
        distributed += share;
    }

    let mut remainder = free - distributed;
    for idx in 0..row_heights.len() {
        if remainder <= 0 {
            break;
        }
        let track = if idx < templates.len() {
            &templates[idx]
        } else {
            auto_track
        };
        if matches!(track, GridTrackSize::Fr(_) | GridTrackSize::Minmax { max_is_fr: true, .. }) {
            row_heights[idx] += 1;
            remainder -= 1;
        }
    }
}

fn item_axis_offset_with_auto_margins(
    align: AlignItems,
    item_size: i32,
    cell_size: i32,
    start_auto: bool,
    end_auto: bool,
) -> i32 {
    let free = (cell_size - item_size).max(0);
    if start_auto && end_auto {
        free / 2
    } else if start_auto {
        free
    } else {
        align_offset(align, item_size, cell_size)
    }
}

/// Compute the offset needed to align an item of `item_size` within `cell_size`
/// according to the given alignment.
fn align_offset(align: AlignItems, item_size: i32, cell_size: i32) -> i32 {
    match align {
        AlignItems::Center => (cell_size - item_size).max(0) / 2,
        AlignItems::FlexEnd => (cell_size - item_size).max(0),
        _ => 0, // FlexStart | Stretch | Baseline
    }
}

/// Recursively translate a `LayoutBox` and all its children by (dx, dy).
#[allow(dead_code)]
fn translate_box(bx: &mut LayoutBox, dx: i32, dy: i32) {
    bx.x += dx;
    bx.y += dy;
    for child in &mut bx.children {
        translate_box(child, dx, dy);
    }
}

// ────────────────────────────────────────────────────────────
// Internal data
// ────────────────────────────────────────────────────────────

struct GridItem {
    node_id: NodeId,
    col_start: GridLine,
    col_end: GridLine,
    row_start: GridLine,
    row_end: GridLine,
    placed_col: usize,
    placed_row: usize,
    span_cols: usize,
    span_rows: usize,
    layout: Option<LayoutBox>,
}
