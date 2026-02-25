//! Table layout: implements CSS table layout for `<table>`, `<tr>`, `<td>`, `<th>`.
//!
//! Supports:
//! - Automatic column width distribution
//! - `colspan` attribute
//! - `cellpadding` / `cellspacing` attributes
//! - `width` attribute on `<table>`, `<td>`, `<th>`
//! - `align` attribute on `<td>`, `<th>`, `<table>`
//! - `<thead>`, `<tbody>`, `<tfoot>` section grouping
//! - `<caption>` element
//! - `border` attribute
//! - `valign` attribute

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{ComputedStyle, Display, TextAlignVal};
use crate::ImageCache;

use super::{
    LayoutBox, BoxType, Edges,
    layout_children,
    font_size_px, is_bold, edges_from,
};
use super::block::build_block;

/// Build a table layout box for a `<table>` element.
pub fn layout_table(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
) -> LayoutBox {
    let style = &styles[node_id];
    let tag = dom.tag(node_id);

    let mut bx = LayoutBox::new(Some(node_id), BoxType::Block);
    bx.color = style.color;
    bx.bg_color = style.background_color;
    bx.border_width = style.border_width;
    bx.border_color = style.border_color;
    bx.font_size = font_size_px(style);
    bx.bold = is_bold(style);
    bx.text_align = style.text_align;
    bx.margin = edges_from(
        style.margin_top, style.margin_right,
        style.margin_bottom, style.margin_left,
    );
    bx.padding = edges_from(
        style.padding_top, style.padding_right,
        style.padding_bottom, style.padding_left,
    );

    // Parse table attributes.
    let cellspacing = parse_int_attr(dom, node_id, "cellspacing").unwrap_or(2);
    let cellpadding = parse_int_attr(dom, node_id, "cellpadding").unwrap_or(1);
    let table_border = parse_int_attr(dom, node_id, "border").unwrap_or(0);

    // Resolve table width.
    let table_width = if let Some(w) = style.width {
        w
    } else if let Some(pct) = style.width_pct {
        (available_width as i64 * pct as i64 / 10000) as i32
    } else if let Some((px100, pct100)) = style.width_calc {
        px100 / 100 + (available_width as i64 * pct100 as i64 / 10000) as i32
    } else {
        // Tables without explicit width use available width.
        available_width - bx.margin.left - bx.margin.right
    };

    bx.width = table_width;

    // Handle margin:auto centering for tables.
    if style.margin_left_auto && style.margin_right_auto {
        let remaining = available_width - bx.width;
        if remaining > 0 {
            bx.margin.left = remaining / 2;
            bx.margin.right = remaining - bx.margin.left;
        }
    }

    // Collect rows from the table's children.
    // Rows can be direct children or inside <thead>/<tbody>/<tfoot>.
    let mut rows: Vec<NodeId> = Vec::new();
    let mut caption_id: Option<NodeId> = None;

    for &child_id in &dom.get(node_id).children {
        let child_tag = dom.tag(child_id);
        match child_tag {
            Some(Tag::Tr) => rows.push(child_id),
            Some(Tag::Thead) | Some(Tag::Tbody) | Some(Tag::Tfoot) => {
                for &grand in &dom.get(child_id).children {
                    if dom.tag(grand) == Some(Tag::Tr) {
                        rows.push(grand);
                    }
                }
            }
            Some(Tag::Caption) => {
                caption_id = Some(child_id);
            }
            Some(Tag::Colgroup) | Some(Tag::Col) => {
                // TODO: column width hints
            }
            _ => {}
        }
    }

    // Determine number of columns by scanning all rows.
    let mut max_cols: usize = 0;
    for &row_id in &rows {
        let mut cols_in_row = 0usize;
        for &cell_id in &dom.get(row_id).children {
            let ct = dom.tag(cell_id);
            if ct == Some(Tag::Td) || ct == Some(Tag::Th) {
                let colspan = parse_int_attr(dom, cell_id, "colspan").unwrap_or(1).max(1) as usize;
                cols_in_row += colspan;
            }
        }
        if cols_in_row > max_cols {
            max_cols = cols_in_row;
        }
    }

    if max_cols == 0 {
        bx.height = 0;
        return bx;
    }

    // Calculate content width available for cells.
    let content_width = table_width - bx.padding.left - bx.padding.right
        - cellspacing * (max_cols as i32 + 1);

    // Phase 1: Compute minimum/preferred column widths.
    let mut col_min_widths = vec![0i32; max_cols];
    let mut col_pref_widths = vec![0i32; max_cols];
    let mut col_fixed_widths = vec![0i32; max_cols]; // explicit width from attrs
    let mut col_has_fixed = vec![false; max_cols];

    for &row_id in &rows {
        let mut col_idx = 0usize;
        for &cell_id in &dom.get(row_id).children {
            let ct = dom.tag(cell_id);
            if ct != Some(Tag::Td) && ct != Some(Tag::Th) {
                continue;
            }
            if col_idx >= max_cols { break; }

            let colspan = parse_int_attr(dom, cell_id, "colspan").unwrap_or(1).max(1) as usize;

            // Check for explicit width on cell.
            let cell_style = &styles[cell_id];
            let explicit_w = if let Some(w) = cell_style.width {
                Some(w)
            } else if let Some(pct) = cell_style.width_pct {
                Some((content_width as i64 * pct as i64 / 10000) as i32)
            } else if let Some((px100, pct100)) = cell_style.width_calc {
                Some(px100 / 100 + (content_width as i64 * pct100 as i64 / 10000) as i32)
            } else {
                parse_int_attr(dom, cell_id, "width")
            };

            // Layout cell to determine minimum content width.
            let cell_pad = cellpadding * 2;
            let cell_border = if table_border > 0 { 2 } else { 0 };
            let cell_overhead = cell_pad + cell_border;

            // Try laying out with generous width to get preferred width.
            let test_w = content_width;
            let cell_box = layout_cell(dom, styles, cell_id, test_w, cellpadding, table_border, images, viewport_w);
            let pref_w = cell_content_width(&cell_box) + cell_overhead;

            if colspan == 1 {
                if let Some(ew) = explicit_w {
                    let ew = ew.max(0);
                    col_fixed_widths[col_idx] = ew.max(col_fixed_widths[col_idx]);
                    col_has_fixed[col_idx] = true;
                }
                col_min_widths[col_idx] = col_min_widths[col_idx].max(30); // minimum reasonable
                col_pref_widths[col_idx] = col_pref_widths[col_idx].max(pref_w);
            }
            // For colspan > 1, we distribute width later.

            col_idx += colspan;
        }
    }

    // Phase 2: Distribute column widths.
    let mut col_widths = vec![0i32; max_cols];
    let mut total_fixed = 0i32;
    let mut num_flex = 0usize;

    for i in 0..max_cols {
        if col_has_fixed[i] {
            col_widths[i] = col_fixed_widths[i];
            total_fixed += col_fixed_widths[i];
        } else {
            num_flex += 1;
        }
    }

    let remaining = content_width - total_fixed;
    if num_flex > 0 && remaining > 0 {
        // Distribute remaining space proportionally to preferred widths.
        let total_pref: i32 = (0..max_cols)
            .filter(|&i| !col_has_fixed[i])
            .map(|i| col_pref_widths[i].max(30))
            .sum();

        if total_pref > 0 {
            for i in 0..max_cols {
                if !col_has_fixed[i] {
                    let pref = col_pref_widths[i].max(30);
                    col_widths[i] = (remaining as i64 * pref as i64 / total_pref as i64) as i32;
                }
            }
        } else {
            let per_col = remaining / num_flex as i32;
            for i in 0..max_cols {
                if !col_has_fixed[i] {
                    col_widths[i] = per_col;
                }
            }
        }
    } else if num_flex > 0 {
        // All remaining space used by fixed, flex columns get minimum.
        for i in 0..max_cols {
            if !col_has_fixed[i] {
                col_widths[i] = col_min_widths[i].max(30);
            }
        }
    }

    // Ensure minimum width per column.
    for i in 0..max_cols {
        if col_widths[i] < 10 {
            col_widths[i] = 10;
        }
    }

    // Phase 3: Layout each row.
    let mut cursor_y = bx.padding.top;

    // Layout caption if present.
    if let Some(cap_id) = caption_id {
        let cap_box = build_block(dom, styles, cap_id, table_width - bx.padding.left - bx.padding.right, images, viewport_w);
        let mut placed = cap_box;
        placed.x = bx.padding.left;
        placed.y = cursor_y;
        cursor_y += placed.height + placed.margin.top + placed.margin.bottom;
        bx.children.push(placed);
    }

    cursor_y += cellspacing;

    for &row_id in &rows {
        let row_style = &styles[row_id];
        let mut row_height = 0i32;
        let mut cell_boxes: Vec<(LayoutBox, usize, usize)> = Vec::new(); // (box, col_start, colspan)

        let mut col_idx = 0usize;
        for &cell_id in &dom.get(row_id).children {
            let ct = dom.tag(cell_id);
            if ct != Some(Tag::Td) && ct != Some(Tag::Th) {
                continue;
            }
            if col_idx >= max_cols { break; }

            let colspan = parse_int_attr(dom, cell_id, "colspan").unwrap_or(1).max(1) as usize;
            let colspan = colspan.min(max_cols - col_idx);

            // Calculate cell width (sum of spanned columns + spacing between them).
            let mut cell_w = 0i32;
            for c in col_idx..col_idx + colspan {
                cell_w += col_widths[c];
                if c > col_idx {
                    cell_w += cellspacing;
                }
            }

            // Layout cell content.
            let cell_box = layout_cell(dom, styles, cell_id, cell_w, cellpadding, table_border, images, viewport_w);
            let ch = cell_box.height;
            if ch > row_height { row_height = ch; }

            cell_boxes.push((cell_box, col_idx, colspan));
            col_idx += colspan;
        }

        // Position cells in the row.
        let row_y = cursor_y;
        for (mut cell_box, col_start, _colspan) in cell_boxes {
            let mut cell_x = bx.padding.left + cellspacing;
            for c in 0..col_start {
                cell_x += col_widths[c] + cellspacing;
            }

            cell_box.x = cell_x;
            cell_box.y = row_y;
            // Stretch cell height to match row height.
            cell_box.height = row_height;

            // Apply row background color if cell has none.
            if cell_box.bg_color == 0 && row_style.background_color != 0 {
                cell_box.bg_color = row_style.background_color;
            }

            bx.children.push(cell_box);
        }

        cursor_y += row_height + cellspacing;
    }

    bx.height = cursor_y + bx.padding.bottom;
    bx
}

/// Layout a single table cell's content.
fn layout_cell(
    dom: &Dom,
    styles: &[ComputedStyle],
    cell_id: NodeId,
    cell_width: i32,
    cellpadding: i32,
    table_border: i32,
    images: &ImageCache,
    viewport_w: i32,
) -> LayoutBox {
    let style = &styles[cell_id];
    let cell_border = if table_border > 0 { 1 } else { style.border_width };

    let mut bx = LayoutBox::new(Some(cell_id), BoxType::Block);
    bx.color = style.color;
    bx.bg_color = style.background_color;
    bx.border_width = cell_border;
    bx.border_color = if table_border > 0 && style.border_color == 0 {
        0xFF999999
    } else {
        style.border_color
    };
    bx.font_size = font_size_px(style);
    bx.bold = is_bold(style);
    bx.text_align = style.text_align;
    bx.width = cell_width;
    bx.padding = Edges {
        top: style.padding_top.max(cellpadding),
        right: style.padding_right.max(cellpadding),
        bottom: style.padding_bottom.max(cellpadding),
        left: style.padding_left.max(cellpadding),
    };

    let inner_w = cell_width - bx.padding.left - bx.padding.right - cell_border * 2;
    let inner_w = inner_w.max(0);

    let child_ids: Vec<NodeId> = dom.get(cell_id).children.iter().copied().collect();
    let height = layout_children(dom, styles, &child_ids, inner_w, &mut bx, cell_id, images, viewport_w);

    bx.height = height + bx.padding.top + bx.padding.bottom + cell_border * 2;
    bx
}

/// Estimate the content width of a laid-out cell.
fn cell_content_width(bx: &LayoutBox) -> i32 {
    let mut max_w = 0i32;
    for child in &bx.children {
        let cw = child.x + child.width;
        if cw > max_w { max_w = cw; }
    }
    max_w
}

/// Parse an integer attribute from the DOM (e.g., cellpadding="5", width="200").
fn parse_int_attr(dom: &Dom, node_id: NodeId, attr_name: &str) -> Option<i32> {
    let val = dom.attr(node_id, attr_name)?;
    let s = val.trim().trim_end_matches("px");
    parse_simple_int(s)
}

fn parse_simple_int(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }
    let mut i = 0;
    let negative = bytes[0] == b'-';
    if negative { i = 1; }
    let mut val: i32 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < b'0' || b > b'9' { break; }
        val = val * 10 + (b - b'0') as i32;
        i += 1;
    }
    if i == 0 || (negative && i == 1) { return None; }
    Some(if negative { -val } else { val })
}
