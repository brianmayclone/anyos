//! Table layout: implements CSS table layout for `<table>`, `<tr>`, `<td>`, `<th>`.
//!
//! Supports:
//! - `table-layout: auto` — column widths sized to content (CSS Table §17.5.2.2)
//! - `table-layout: fixed` — column widths from first row only (CSS Table §17.5.2.1)
//! - `colspan` attribute
//! - `cellpadding` / `cellspacing` attributes
//! - `width` attribute on `<table>`, `<td>`, `<th>`
//! - `align` attribute on `<td>`, `<th>`, `<table>`
//! - `vertical-align: top/middle/bottom/baseline` on `<td>`, `<th>` (CSS + HTML `valign`)
//! - `<thead>`, `<tbody>`, `<tfoot>` section grouping
//! - `<caption>` element
//! - `border` attribute

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{
    ComputedStyle, Display, PseudoStyles, TextAlignVal, VerticalAlign, resolve_inset,
    resolve_margins,
};
use crate::ImageCache;

use super::block::build_block;
use super::flex;
use super::{edges_from, font_size_px, is_bold, layout_children, BoxType, Edges, LayoutBox};

/// Build a table layout box for a `<table>` element.
pub fn layout_table(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
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
    bx.background_clip = style.background_clip;
    bx.background_position_x = style.background_position_x;
    bx.background_position_y = style.background_position_y;
    bx.appearance_none = style.appearance == crate::style::AppearanceVal::None;
    bx.border_width = style.border_width;
    bx.border_color = style.border_color;
    bx.font_size = font_size_px(style);
    bx.bold = is_bold(style);
    bx.text_align = style.text_align;
    let (margin_top, margin_right, margin_bottom, margin_left) =
        resolve_margins(style, available_width);
    bx.margin = edges_from(margin_top, margin_right, margin_bottom, margin_left);
    bx.padding = edges_from(
        style.padding_top,
        style.padding_right,
        style.padding_bottom,
        style.padding_left,
    );
    let table_border_left = style.border_left.width.max(bx.border_width);
    let table_border_right = style.border_right.width.max(bx.border_width);
    let table_border_top = style.border_top.width.max(bx.border_width);
    let table_border_bottom = style.border_bottom.width.max(bx.border_width);

    // Parse table attributes, respecting CSS border-collapse / border-spacing.
    let css_spacing_x = style.border_spacing_x;
    let css_spacing_y = style.border_spacing_y;
    let is_collapsed = style.border_collapse;
    let cellspacing_x = if is_collapsed {
        0 // border-collapse: collapse → no spacing between cells
    } else if css_spacing_x > 0 || css_spacing_y > 0 {
        css_spacing_x // CSS border-spacing overrides HTML attribute
    } else {
        parse_int_attr(dom, node_id, "cellspacing").unwrap_or(2)
    };
    let cellspacing_y = if is_collapsed {
        0
    } else if css_spacing_x > 0 || css_spacing_y > 0 {
        css_spacing_y
    } else {
        parse_int_attr(dom, node_id, "cellspacing").unwrap_or(2)
    };
    let cellpadding = parse_int_attr(dom, node_id, "cellpadding").unwrap_or(1);
    let table_border = parse_int_attr(dom, node_id, "border").unwrap_or(0);

    // Resolve table width.
    let has_explicit_table_width =
        style.width.is_some() || style.width_pct.is_some() || style.width_calc.is_some();
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
    let table_definite_height = style.height.unwrap_or(0);

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

    // ── Build a grid with proper rowspan + colspan tracking ─────────────────
    // CSS Table §17.4: each cell occupies a rectangular set of grid slots.
    // We track which (row, col) slots are occupied and resolve the column
    // index for each cell accordingly.
    //
    // grid_cells: Vec<(row, col, colspan, rowspan, node_id)>
    struct CellPos {
        row: usize,
        col: usize,
        colspan: usize,
        rowspan: usize,
        node_id: NodeId,
    }

    // occupied[row][col] = true if that grid slot is taken
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let ensure_grid = |occupied: &mut Vec<Vec<bool>>, rows_needed: usize, cols_needed: usize| {
        while occupied.len() < rows_needed {
            occupied.push(Vec::new());
        }
        for row in occupied.iter_mut() {
            while row.len() < cols_needed {
                row.push(false);
            }
        }
    };

    let mut grid_cells: Vec<CellPos> = Vec::new();
    let mut max_cols: usize = 1;

    for (row_num, &row_id) in rows.iter().enumerate() {
        let mut col_idx = 0usize;
        for &cell_id in &dom.get(row_id).children {
            let ct = dom.tag(cell_id);
            if ct != Some(Tag::Td) && ct != Some(Tag::Th) {
                continue;
            }

            let colspan = parse_int_attr(dom, cell_id, "colspan").unwrap_or(1).max(1) as usize;
            let rowspan = parse_int_attr(dom, cell_id, "rowspan").unwrap_or(1).max(1) as usize;

            // Advance col_idx past occupied slots (from previous rows' rowspans).
            ensure_grid(&mut occupied, row_num + rowspan, col_idx + colspan + 10);
            while col_idx < occupied[row_num].len() && occupied[row_num][col_idx] {
                col_idx += 1;
            }

            // Mark all slots this cell occupies.
            ensure_grid(&mut occupied, row_num + rowspan, col_idx + colspan);
            for r in row_num..row_num + rowspan {
                for c in col_idx..col_idx + colspan {
                    if r < occupied.len() && c < occupied[r].len() {
                        occupied[r][c] = true;
                    }
                }
            }

            let end_col = col_idx + colspan;
            if end_col > max_cols {
                max_cols = end_col;
            }

            grid_cells.push(CellPos {
                row: row_num,
                col: col_idx,
                colspan,
                rowspan,
                node_id: cell_id,
            });
            col_idx += colspan;
        }
    }

    if max_cols == 0 {
        bx.height = 0;
        return bx;
    }

    // Calculate content width available for cells.
    let content_width = (
        table_width
            - table_border_left
            - table_border_right
            - bx.padding.left
            - bx.padding.right
            - cellspacing_x * (max_cols as i32 + 1)
    )
    .max(0);

    // Phase 1: Compute column widths.
    //
    // `table-layout: fixed` (CSS Table §17.5.2.1):
    //   Column widths are determined by explicit widths on cells in the FIRST ROW only.
    //   Remaining space is distributed evenly among unsized columns.
    //   Cells in subsequent rows do not affect column widths (faster rendering).
    //
    // `table-layout: auto` (CSS Table §17.5.2.2, default):
    //   All cells are measured; columns sized to content.
    let table_layout_fixed = style.table_layout_fixed;

    let mut col_min_widths = vec![0i32; max_cols];
    let mut col_pref_widths = vec![0i32; max_cols];
    let mut col_fixed_widths = vec![0i32; max_cols];
    let mut col_has_fixed = vec![false; max_cols];

    // Determine which rows to scan for column sizing.
    // For `fixed`, only consider cells in the first row (row 0).
    let first_row_only = table_layout_fixed;

    for cp in &grid_cells {
        let col_idx = cp.col;
        let colspan = cp.colspan;
        let cell_id = cp.node_id;
        if col_idx >= max_cols {
            continue;
        }
        if first_row_only && cp.row != 0 {
            continue;
        }

        let cell_style = &styles[cell_id];
        let cell_padding_h = cell_style.padding_left.max(cellpadding)
            + cell_style.padding_right.max(cellpadding);
        let cell_border_h = if is_collapsed {
            let bw = if cell_style.border_top.width > 0 || cell_style.border_left.width > 0 {
                1i32
            } else if table_border > 0 || cell_style.border_width > 0 {
                1i32
            } else {
                0i32
            };
            bw * 2
        } else if table_border > 0 {
            2
        } else {
            cell_style.border_width * 2
        };
        let cell_overhead = cell_padding_h + cell_border_h;

        // Check for explicit CSS or HTML width on cell.
        let explicit_w = if let Some(w) = cell_style.width {
            Some(w)
        } else if let Some(pct) = cell_style.width_pct {
            Some((content_width as i64 * pct as i64 / 10000) as i32)
        } else {
            parse_int_attr(dom, cell_id, "width")
        };

        if table_layout_fixed {
            // Fixed layout: use explicit widths only — no content measurement.
            if colspan == 1 {
                if let Some(ew) = explicit_w {
                    let ew = ew.max(0);
                    col_fixed_widths[col_idx] = ew.max(col_fixed_widths[col_idx]);
                    col_has_fixed[col_idx] = true;
                }
            } else if let Some(ew) = explicit_w {
                let per_col = (ew / colspan as i32).max(0);
                for c in col_idx..col_idx + colspan {
                    if c < max_cols && !col_has_fixed[c] {
                        col_fixed_widths[c] = per_col.max(col_fixed_widths[c]);
                        col_has_fixed[c] = true;
                    }
                }
            }
        } else {
            // Auto layout: measure content for preferred/minimum widths.
            // Max-content width for this cell (natural width without line-breaking).
            let has_children = !dom.get(cell_id).children.is_empty();
            let pref_w = if has_children {
                flex::measure_max_content(dom, styles, pseudo, cell_id, images, viewport_w)
                    + cell_overhead
            } else {
                cell_overhead
            };

            if colspan == 1 {
                if let Some(ew) = explicit_w {
                    let ew = ew.max(0);
                    col_fixed_widths[col_idx] = ew.max(col_fixed_widths[col_idx]);
                    col_has_fixed[col_idx] = true;
                }
                let min_w = if has_children {
                    super::intrinsic_min_width(dom, styles, pseudo, cell_id, images, viewport_w)
                        + cell_overhead
                } else {
                    cell_overhead
                };
                col_min_widths[col_idx] = col_min_widths[col_idx].max(min_w.max(10));
                col_pref_widths[col_idx] =
                    col_pref_widths[col_idx].max(pref_w.max(col_min_widths[col_idx]));
            } else {
                // Distribute explicit width of multi-column cells across the spanned columns.
                if let Some(ew) = explicit_w {
                    let per_col = ew / colspan as i32;
                    for c in col_idx..col_idx + colspan {
                        if c < max_cols && !col_has_fixed[c] {
                            col_fixed_widths[c] = per_col.max(col_fixed_widths[c]);
                            col_has_fixed[c] = true;
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Distribute column widths (CSS Table §17.5.2 simplified).
    //
    // Step 1: Fixed columns get their explicit widths.
    // Step 2: If sum(pref) <= remaining → each flex column gets pref.
    // Step 3: Otherwise → distribute remaining proportionally to pref.
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

    let remaining = (content_width - total_fixed).max(0);
    if num_flex > 0 {
        let total_pref: i32 = (0..max_cols)
            .filter(|&i| !col_has_fixed[i])
            .map(|i| col_pref_widths[i].max(col_min_widths[i]).max(10))
            .sum::<i32>()
            .max(1);

        for i in 0..max_cols {
            if !col_has_fixed[i] {
                let pref = col_pref_widths[i].max(col_min_widths[i]).max(10);
                col_widths[i] = if total_pref <= remaining {
                    pref
                } else {
                    (remaining as i64 * pref as i64 / total_pref as i64) as i32
                };
                col_widths[i] = col_widths[i].max(col_min_widths[i]).max(10);
            }
        }
    }

    // Enforce minimum width per column.
    for i in 0..max_cols {
        if col_widths[i] < 10 {
            col_widths[i] = 10;
        }
    }

    if !has_explicit_table_width {
        let used_width = bx.padding.left
            + table_border_left
            + table_border_right
            + bx.padding.right
            + cellspacing_x * (max_cols as i32 + 1)
            + col_widths.iter().copied().sum::<i32>();
        bx.width = used_width.min(table_width).max(0);
    }

    // Phase 3: Layout each row.
    let mut cursor_y = table_border_top + bx.padding.top;

    // Layout caption if present.
    if let Some(cap_id) = caption_id {
        let cap_box = build_block(
            dom,
            styles,
            pseudo,
            cap_id,
            table_width - table_border_left - table_border_right - bx.padding.left - bx.padding.right,
            images,
            viewport_w,
            0,
        );
        let mut placed = cap_box;
        placed.x = table_border_left + bx.padding.left;
        placed.y = cursor_y;
        cursor_y += placed.height + placed.margin.top + placed.margin.bottom;
        bx.children.push(placed);
    }

    cursor_y += cellspacing_y;

    // Build per-row cell lists from grid_cells (only first-row of multi-row spans get laid out).
    let num_rows = rows.len();
    for row_num in 0..num_rows {
        let row_id = rows[row_num];
        let row_style = &styles[row_id];
        let section_style = dom
            .get(row_id)
            .parent
            .and_then(|pid| styles.get(pid))
            .filter(|section_style| {
                matches!(
                    dom.tag(dom.get(row_id).parent.unwrap_or(node_id)),
                    Some(Tag::Thead | Tag::Tbody | Tag::Tfoot)
                )
            });
        let mut row_height = 0i32;
        // (box, col_start, colspan, content_height) — content_height before row-stretch.
        let mut cell_boxes: Vec<(LayoutBox, usize, usize, i32)> = Vec::new();
        let is_last_row = row_num + 1 == num_rows;

        for cp in grid_cells.iter().filter(|cp| cp.row == row_num) {
            let col_idx = cp.col;
            let colspan = cp.colspan;
            let cell_id = cp.node_id;
            if col_idx >= max_cols {
                continue;
            }
            let colspan = colspan.min(max_cols - col_idx);
            let is_last_col = col_idx + colspan >= max_cols;

            // Calculate cell width (sum of spanned columns + spacing between them).
            let mut cell_w = 0i32;
            for c in col_idx..col_idx + colspan {
                cell_w += col_widths[c];
                if c > col_idx {
                    cell_w += cellspacing_x;
                }
            }

            // Layout cell content.
            let cell_box = layout_cell(
                dom,
                styles,
                pseudo,
                cell_id,
                cell_w,
                cellpadding,
                table_border,
                is_collapsed,
                is_last_col,
                is_last_row,
                images,
                viewport_w,
            );
            let ch = cell_box.height;
            if ch > row_height {
                row_height = ch;
            }

            cell_boxes.push((cell_box, col_idx, colspan, ch));
        }

        // Position cells in the row.
        let row_y = cursor_y;
        let row_dx = {
            let left = resolve_inset(row_style.left_offset, row_style.left_calc, table_width, true);
            let right =
                resolve_inset(row_style.right_offset, row_style.right_calc, table_width, true);
            left.unwrap_or_else(|| right.map(|v| -v).unwrap_or(0))
        };
        let row_dy = {
            let top = resolve_inset(
                row_style.top,
                row_style.top_calc,
                table_definite_height,
                table_definite_height > 0,
            );
            let bottom = resolve_inset(
                row_style.bottom_offset,
                row_style.bottom_calc,
                table_definite_height,
                table_definite_height > 0,
            );
            top.unwrap_or_else(|| bottom.map(|v| -v).unwrap_or(0))
        };
        let section_dx = if let Some(section_style) = section_style {
            let left =
                resolve_inset(section_style.left_offset, section_style.left_calc, table_width, true);
            let right = resolve_inset(
                section_style.right_offset,
                section_style.right_calc,
                table_width,
                true,
            );
            left.unwrap_or_else(|| right.map(|v| -v).unwrap_or(0))
        } else {
            0
        };
        let section_dy = if let Some(section_style) = section_style {
            let top = resolve_inset(
                section_style.top,
                section_style.top_calc,
                table_definite_height,
                table_definite_height > 0,
            );
            let bottom = resolve_inset(
                section_style.bottom_offset,
                section_style.bottom_calc,
                table_definite_height,
                table_definite_height > 0,
            );
            top.unwrap_or_else(|| bottom.map(|v| -v).unwrap_or(0))
        } else {
            0
        };
        for (mut cell_box, col_start, _colspan, content_h) in cell_boxes {
            let mut cell_x = table_border_left + bx.padding.left + cellspacing_x;
            for c in 0..col_start {
                cell_x += col_widths[c] + cellspacing_x;
            }

            cell_box.x = cell_x + row_dx + section_dx;
            cell_box.y = row_y + row_dy + section_dy;
            // Stretch cell height to match row height.
            cell_box.height = row_height;

            // Apply vertical alignment within the cell (CSS Table §17.5.3).
            // Reads the CSS `vertical-align` or HTML `valign` attribute.
            let cell_id = cell_box.node_id.unwrap_or(0);
            let css_valign = styles[cell_id].vertical_align;
            let html_valign = dom.attr(cell_id, "valign").unwrap_or("");
            let y_offset: i32 = match css_valign {
                VerticalAlign::Middle => (row_height - content_h).max(0) / 2,
                VerticalAlign::Bottom | VerticalAlign::TextBottom => {
                    (row_height - content_h).max(0)
                }
                VerticalAlign::Baseline | VerticalAlign::Top | VerticalAlign::TextTop => 0,
                _ => match html_valign {
                    "middle" => (row_height - content_h).max(0) / 2,
                    "bottom" => (row_height - content_h).max(0),
                    _ => 0,
                },
            };
            if y_offset > 0 {
                for child in &mut cell_box.children {
                    child.y += y_offset;
                }
            }

            // Apply row background color if cell has none.
            if cell_box.bg_color == 0 && row_style.background_color != 0 {
                cell_box.bg_color = row_style.background_color;
            }

            bx.children.push(cell_box);
        }

        cursor_y += row_height + cellspacing_y;
    }

    bx.height = cursor_y + bx.padding.bottom + table_border_bottom;
    bx
}

/// Layout a single table cell's content.
///
/// `is_collapsed`: border-collapse:collapse mode — use per-side 1px borders so
///   adjacent cells share a single pixel (no double-border artifact).
/// `is_last_col` / `is_last_row`: whether this cell is on the right/bottom edge
///   of the table, needed to draw the closing border in collapsed mode.
fn layout_cell(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    cell_id: NodeId,
    cell_width: i32,
    cellpadding: i32,
    table_border: i32,
    is_collapsed: bool,
    is_last_col: bool,
    is_last_row: bool,
    images: &ImageCache,
    viewport_w: i32,
) -> LayoutBox {
    let style = &styles[cell_id];

    let mut bx = LayoutBox::new(Some(cell_id), BoxType::Block);
    bx.color = style.color;
    bx.bg_color = style.background_color;
    bx.background_clip = style.background_clip;
    bx.background_position_x = style.background_position_x;
    bx.background_position_y = style.background_position_y;
    bx.appearance_none = style.appearance == crate::style::AppearanceVal::None;
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

    // Determine border color (CSS > table attribute > default).
    let border_color = if style.border_color != 0 {
        style.border_color
    } else if table_border > 0 {
        0xFF999999
    } else {
        style.border_color
    };

    let (h_border, v_border) = if is_collapsed {
        // border-collapse: collapse — each cell draws its top+left edge only.
        // The last column also draws the right edge; the last row draws the bottom.
        // This yields exactly 1px between every pair of adjacent cells.
        let bw = if style.border_top.width > 0 || style.border_left.width > 0 {
            1i32
        } else if table_border > 0 || style.border_width > 0 {
            1i32
        } else {
            0i32
        };
        let bc = if border_color != 0 {
            border_color
        } else {
            0xFFA2A9B1
        };

        bx.border_width = 0; // uniform border suppressed; use per-side
        bx.border_top_width = bw;
        bx.border_left_width = bw;
        bx.border_right_width = if is_last_col { bw } else { 0 };
        bx.border_bottom_width = if is_last_row { bw } else { 0 };
        bx.border_top_color = bc;
        bx.border_left_color = bc;
        bx.border_right_color = bc;
        bx.border_bottom_color = bc;
        // Use CSS per-side styles if provided, otherwise solid.
        bx.border_top_style = if style.border_top.style != crate::style::BorderStyleVal::None {
            style.border_top.style
        } else {
            crate::style::BorderStyleVal::Solid
        };
        bx.border_left_style = bx.border_top_style;
        bx.border_right_style = bx.border_top_style;
        bx.border_bottom_style = bx.border_top_style;
        // For size calculations: h_border = top + bottom, v_border = left + right.
        let right_b = if is_last_col { bw } else { 0 };
        let bottom_b = if is_last_row { bw } else { 0 };
        (bw + bottom_b, bw + right_b)
    } else {
        let cell_border = if table_border > 0 {
            1
        } else {
            style.border_width
        };
        bx.border_width = cell_border;
        bx.border_color = border_color;
        (cell_border * 2, cell_border * 2)
    };

    let inner_w = (cell_width - bx.padding.left - bx.padding.right - v_border).max(0);

    let child_ids: Vec<NodeId> = dom.get(cell_id).children.iter().copied().collect();
    let laid_out_height = layout_children(
        dom,
        styles,
        pseudo,
        &child_ids,
        inner_w,
        &mut bx,
        cell_id,
        images,
        viewport_w,
        0,
        0,
        0,
        None,
    );
    let content_start_y = bx.border_width + bx.padding.top;
    let content_height = laid_out_height.saturating_sub(content_start_y);

    bx.height = content_height + bx.padding.top + bx.padding.bottom + h_border;
    bx
}

/// Estimate the intrinsic content width of a laid-out cell.
///
/// Block children fill available width, so we recursively find the actual
/// content extent (text widths, image widths) instead of using the filled
/// box widths.  This prevents all columns from getting equal preferred
/// widths when the table is laid out at a generous test width.
fn cell_content_width(bx: &LayoutBox) -> i32 {
    intrinsic_content_width(bx)
}

/// Recursively compute the intrinsic content width of a layout box.
/// Bottoms out at text nodes and images (which have intrinsic widths).
/// For block containers without explicit width, takes the max of children.
/// For inline/flex children placed side by side, uses their x positions.
fn intrinsic_content_width(bx: &LayoutBox) -> i32 {
    // Text and images have intrinsic widths.
    if bx.text.is_some() || bx.image_src.is_some() {
        return bx.width;
    }
    if bx.children.is_empty() {
        return bx.padding.left + bx.padding.right + bx.border_width * 2;
    }
    // Skip out-of-flow children; use x position for side-by-side layout.
    let right_edge = bx
        .children
        .iter()
        .filter(|c| !c.is_out_of_flow)
        .map(|c| {
            let cw = intrinsic_content_width(c);
            c.x - bx.x + cw + c.margin.right
        })
        .max()
        .unwrap_or(0);
    // Include the box's own padding/border.
    let own_extra = bx.padding.left + bx.padding.right + bx.border_width * 2;
    right_edge.max(own_extra)
}

/// Parse an integer attribute from the DOM (e.g., cellpadding="5", width="200").
fn parse_int_attr(dom: &Dom, node_id: NodeId, attr_name: &str) -> Option<i32> {
    let val = dom.attr(node_id, attr_name)?;
    let s = val.trim().trim_end_matches("px");
    parse_simple_int(s)
}

fn parse_simple_int(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    let negative = bytes[0] == b'-';
    if negative {
        i = 1;
    }
    let mut val: i32 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < b'0' || b > b'9' {
            break;
        }
        val = val * 10 + (b - b'0') as i32;
        i += 1;
    }
    if i == 0 || (negative && i == 1) {
        return None;
    }
    Some(if negative { -val } else { val })
}
