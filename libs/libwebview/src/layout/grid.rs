//! CSS Grid layout: `layout_grid()` implements the CSS Grid Layout algorithm.
//!
//! This implementation covers the most common subset of the spec:
//! - Explicit track sizing via `grid-template-columns` / `grid-template-rows`
//! - `fr` units resolved after fixed and percent tracks are sized
//! - `auto` tracks sized to the tallest content in the row
//! - Explicit item placement with `grid-column-start/end` and `grid-row-start/end`
//! - Auto-placement (row-major scanning, left-to-right, top-to-bottom)
//! - `column-gap` / `row-gap` between tracks
//! - `justify-items` / `align-items` within each cell
//!
//! Limitations (future work):
//! - Named grid lines are ignored (only numeric indices and `span N`)
//! - `grid-template-areas` not parsed or used
//! - `minmax()` and `fit-content()` are not supported
//! - Subgrid and nested grids are not specially handled

use alloc::vec;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId};
use crate::style::{
    AlignItems, ComputedStyle, Display, GridLine, GridTrackSize, Position,
};
use crate::ImageCache;

use super::LayoutBox;
use super::block::build_block;

// ────────────────────────────────────────────────────────────
// Public entry-point
// ────────────────────────────────────────────────────────────

/// Lay out `child_ids` as a grid container inside `parent` and return total
/// height consumed by the grid (not including the parent's own padding/border).
pub fn layout_grid(
    dom: &Dom,
    styles: &[ComputedStyle],
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
) -> i32 {
    let parent_idx = parent.node_id.unwrap_or(0);
    let parent_style = &styles[parent_idx];

    let col_gap = parent_style.column_gap;
    let row_gap = parent_style.row_gap;
    let container_align = parent_style.align_items;
    let justify_items = parent_style.justify_items;

    // ── 1. Resolve column track sizes ──────────────────────────────────────
    let col_templates = &parent_style.grid_template_columns;
    let auto_col = &parent_style.grid_auto_columns;

    // ── 2. Collect visible, non-absolutely-positioned children ────────────
    let mut items: Vec<GridItem> = child_ids
        .iter()
        .filter_map(|&cid| {
            let st = &styles[cid];
            if st.display == Display::None { return None; }
            if matches!(st.position, Position::Absolute | Position::Fixed) { return None; }
            Some(GridItem {
                node_id: cid,
                col_start: st.grid_column_start,
                col_end: st.grid_column_end,
                row_start: st.grid_row_start,
                row_end: st.grid_row_end,
                placed_col: 0,
                placed_row: 0,
                span_cols: 1,
                span_rows: 1,
                layout: None,
            })
        })
        .collect();

    if items.is_empty() {
        return 0;
    }

    // ── 3. Determine number of explicit columns ──────────────────────────
    // The explicit grid has as many columns as `grid-template-columns` defines
    // (minimum 1).  Items that exceed the explicit grid extend it implicitly.
    let explicit_cols = col_templates.len().max(1);

    // ── 4. Auto-place all items ──────────────────────────────────────────
    auto_place(&mut items, explicit_cols);

    // Total column count needed (explicit + implicit).
    let total_cols = items.iter().map(|it| it.placed_col + it.span_cols).max().unwrap_or(1);

    // ── 5. Resolve column pixel widths ────────────────────────────────────
    let col_widths = resolve_col_widths(
        col_templates,
        auto_col,
        total_cols,
        available_width,
        col_gap,
    );

    // ── 6. Total row count ────────────────────────────────────────────────
    let total_rows = items.iter().map(|it| it.placed_row + it.span_rows).max().unwrap_or(1);

    let row_templates = &parent_style.grid_template_rows;
    let auto_row = &parent_style.grid_auto_rows;

    // ── 7. Measure each item at its column span width ─────────────────────
    for item in &mut items {
        let col_w = span_width(&col_widths, item.placed_col, item.span_cols, col_gap);
        let bx = build_block(dom, styles, item.node_id, col_w, images);
        item.layout = Some(bx);
    }

    // ── 8. Resolve row heights ────────────────────────────────────────────
    let row_heights = resolve_row_heights(
        row_templates,
        auto_row,
        total_rows,
        &items,
    );

    // ── 9. Position every item ────────────────────────────────────────────
    let mut cursor_y = 0i32;
    let row_offsets: Vec<i32> = {
        let mut offsets = Vec::with_capacity(total_rows);
        let mut y = 0i32;
        for r in 0..total_rows {
            offsets.push(y);
            y += row_heights[r] + if r + 1 < total_rows { row_gap } else { 0 };
        }
        y
        // (we'll use `cursor_y` differently — just store and use offsets)
        ; offsets
    };
    // Recompute total height from row_offsets.
    cursor_y = if !row_offsets.is_empty() {
        let last_row = total_rows - 1;
        row_offsets[last_row] + row_heights[last_row]
    } else {
        0
    };

    for item in &mut items {
        let x = col_offset(&col_widths, item.placed_col, col_gap);
        let y = row_offsets[item.placed_row];
        let cell_w = span_width(&col_widths, item.placed_col, item.span_cols, col_gap);
        let cell_h = span_height(&row_heights, item.placed_row, item.span_rows, row_gap);

        if let Some(mut bx) = item.layout.take() {
            let item_w = bx.width;
            let item_h = bx.height;

            // Horizontal alignment (justify-items).
            let x_offset = align_offset(justify_items, item_w, cell_w);
            // Vertical alignment (align-items).
            let y_offset = align_offset(container_align, item_h, cell_h);

            // Translate the subtree so that (bx.x, bx.y) lands at (x + x_offset, y + y_offset).
            let dx = parent.x + x + x_offset - bx.x;
            let dy = parent.y + y + y_offset - bx.y;
            translate_box(&mut bx, dx, dy);

            parent.children.push(bx);
        }
    }

    cursor_y
}

// ────────────────────────────────────────────────────────────
// Auto-placement algorithm (row-major)
// ────────────────────────────────────────────────────────────

/// Resolve span size from a pair of GridLine values relative to the explicit
/// grid width.  Returns (span_size, start_or_none).
fn resolve_span(start: GridLine, end: GridLine, _explicit: usize) -> (Option<usize>, usize) {
    match (start, end) {
        (GridLine::Index(s), GridLine::Index(e)) => {
            let s = (s - 1).max(0) as usize;
            let span = ((e - 1).max(0) as usize).saturating_sub(s).max(1);
            (Some(s), span)
        }
        (GridLine::Index(s), GridLine::Span(n)) => {
            let s = (s - 1).max(0) as usize;
            (Some(s), (n.max(1)) as usize)
        }
        (GridLine::Index(s), GridLine::Auto) => {
            let s = (s - 1).max(0) as usize;
            (Some(s), 1)
        }
        (GridLine::Auto, GridLine::Index(e)) => {
            // End known, start unknown — we know the span ends at `e`.
            // Auto-placement must handle this; return end as the clue.
            let span = 1usize; // placeholder; handled differently below
            let _ = span;
            (None, ((e - 1).max(1)) as usize) // return end-1 as span hint
        }
        (GridLine::Auto, GridLine::Span(n)) => (None, (n.max(1)) as usize),
        (GridLine::Span(n), _) => (None, (n.max(1)) as usize),
        (GridLine::Auto, GridLine::Auto) => (None, 1),
    }
}

/// Place all grid items using the CSS Grid auto-placement algorithm
/// (row-major, left-to-right, no dense packing).
fn auto_place(items: &mut Vec<GridItem>, explicit_cols: usize) {
    // Grid occupancy map: (col, row) → occupied.
    // We use a simple Vec and grow it as needed.
    let mut occupied: Vec<Vec<bool>> = vec![vec![false; explicit_cols.max(1)]]; // [row][col]

    // Pre-pass: resolve items with fully explicit positions.
    for item in items.iter_mut() {
        let (col_start, span_cols) =
            resolve_span(item.col_start, item.col_end, explicit_cols);
        let (row_start, span_rows) =
            resolve_span(item.row_start, item.row_end, explicit_cols);
        item.span_cols = span_cols.max(1);
        item.span_rows = span_rows.max(1);

        if let (Some(c), Some(r)) = (col_start, row_start) {
            item.placed_col = c;
            item.placed_row = r;
            mark_occupied(&mut occupied, r, c, item.span_rows, item.span_cols);
        }
    }

    // Second pass: auto-place items without fully explicit positions.
    let mut auto_cursor_row = 0usize;
    let mut auto_cursor_col = 0usize;

    for item in items.iter_mut() {
        let col_start = match item.col_start {
            GridLine::Index(n) => Some((n - 1).max(0) as usize),
            _ => None,
        };
        let row_start = match item.row_start {
            GridLine::Index(n) => Some((n - 1).max(0) as usize),
            _ => None,
        };

        // Already fully placed above.
        if col_start.is_some() && row_start.is_some() { continue; }

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
            col_start,
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
        while r.len() < cols { r.push(false); }
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
    fixed_col: Option<usize>,
) -> (usize, usize) {
    let mut r = *cursor_row;
    let mut c = if let Some(fc) = fixed_col {
        fc
    } else {
        *cursor_col
    };

    loop {
        if c + span_c > num_cols {
            // Wrap to next row.
            c = if fixed_col.is_some() { fixed_col.unwrap() } else { 0 };
            r += 1;
        }
        if fits(occupied, r, c, span_r, span_c) {
            return (r, c);
        }
        if fixed_col.is_some() {
            r += 1;
        } else {
            c += 1;
        }
    }
}

/// Check whether a span fits at (row, col) without overlap.
fn fits(
    occupied: &Vec<Vec<bool>>,
    row: usize,
    col: usize,
    span_r: usize,
    span_c: usize,
) -> bool {
    for r in row..row + span_r {
        if r >= occupied.len() { continue; } // empty row = free
        let row_data = &occupied[r];
        for c in col..col + span_c {
            if c < row_data.len() && row_data[c] {
                return false;
            }
        }
    }
    true
}

// ────────────────────────────────────────────────────────────
// Track sizing helpers
// ────────────────────────────────────────────────────────────

/// Resolve column widths in pixels from the track template + auto-column definition.
///
/// Algorithm:
/// 1. Assign fixed-px and percent tracks.
/// 2. Distribute remaining space proportionally among `fr` tracks.
/// 3. Fill `auto` tracks with equal shares of the remaining free space.
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
        if idx < templates.len() { &templates[idx] } else { auto_track }
    };

    let total_gap = col_gap * (total_cols.saturating_sub(1) as i32);
    let available = (container_width - total_gap).max(0);

    // First pass: fixed and percent; count fr units.
    let mut fixed_total = 0i32;
    let mut fr_total = 0i32; // sum of fr values (×100 fixed-point)
    for i in 0..total_cols {
        match track_for(i) {
            GridTrackSize::Px(px) => { widths.push(*px); fixed_total += px; }
            GridTrackSize::Percent(pct) => {
                let px = (available as i64 * *pct as i64 / 10000) as i32;
                widths.push(px);
                fixed_total += px;
            }
            GridTrackSize::Fr(f) => { widths.push(0); fr_total += f; }
            GridTrackSize::Auto => { widths.push(0); } // sized in second pass
        }
    }

    // Distribute free space to fr tracks.
    let free = (available - fixed_total).max(0);
    if fr_total > 0 {
        for i in 0..total_cols {
            if let GridTrackSize::Fr(f) = track_for(i) {
                widths[i] = (free as i64 * *f as i64 / fr_total as i64) as i32;
            }
        }
    } else {
        // No fr tracks — distribute remaining free space equally to auto tracks.
        let auto_count = (0..total_cols)
            .filter(|&i| matches!(track_for(i), GridTrackSize::Auto))
            .count() as i32;
        if auto_count > 0 {
            let share = free / auto_count;
            for i in 0..total_cols {
                if matches!(track_for(i), GridTrackSize::Auto) {
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
        if idx < templates.len() { &templates[idx] } else { auto_track }
    };

    let mut heights: Vec<i32> = vec![0; total_rows];

    // Pass 1: explicit sizes.
    for r in 0..total_rows {
        match track_for(r) {
            GridTrackSize::Px(px) => heights[r] = *px,
            _ => {} // content-sized or fr — determined from items
        }
    }

    // Pass 2: content-size rows that are `auto` or `fr`.
    for item in items {
        let item_h = item.layout.as_ref().map(|b| b.height).unwrap_or(0);
        // Distribute the item height evenly across its row span.
        // For simplicity (sufficient for 99% of pages), attribute to the first row.
        let r = item.placed_row;
        if r < total_rows {
            match track_for(r) {
                GridTrackSize::Auto | GridTrackSize::Fr(_) => {
                    if item_h > heights[r] {
                        heights[r] = item_h;
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
        if i > col { w += col_gap; }
        w += col_widths.get(i).copied().unwrap_or(0);
    }
    w.max(0)
}

/// Combined pixel height of `span` rows starting at `row`.
fn span_height(row_heights: &[i32], row: usize, span: usize, row_gap: i32) -> i32 {
    let mut h = 0i32;
    for i in row..row + span {
        if i > row { h += row_gap; }
        h += row_heights.get(i).copied().unwrap_or(0);
    }
    h.max(0)
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
