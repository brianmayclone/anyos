//! Flexbox layout: `layout_flex()` implements the CSS Flexible Box Layout.

use alloc::vec::Vec;

use crate::dom::{Dom, NodeId};
use crate::style::{
    ComputedStyle, Display, Position, FlexDirection, FlexWrap,
    JustifyContent, AlignItems,
};
use crate::ImageCache;

use super::LayoutBox;
use super::block::build_block;

struct FlexItem {
    node_id: NodeId,
    grow: i32,
    shrink: i32,
    order: i32,
    main_base: i32,
    cross_base: i32,
    layout: Option<LayoutBox>,
}

struct FlexLine {
    start: usize,
    end: usize,
    total_main: i32,
}

/// Resolve the effective align-items for a child (considering align-self).
fn resolve_align(container_align: AlignItems, child_style: &ComputedStyle) -> AlignItems {
    child_style.align_self.unwrap_or(container_align)
}

/// Lay out children as a flex container and return the total height consumed.
pub fn layout_flex(
    dom: &Dom,
    styles: &[ComputedStyle],
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
) -> i32 {
    let parent_style_idx = parent.node_id.unwrap_or(0);
    let parent_style = &styles[parent_style_idx];
    let direction = parent_style.flex_direction;
    let wrap = parent_style.flex_wrap;
    let justify = parent_style.justify_content;
    let align = parent_style.align_items;
    let row_gap = parent_style.row_gap;
    let col_gap = parent_style.column_gap;

    let is_row = matches!(direction, FlexDirection::Row | FlexDirection::RowReverse);
    let is_reverse = matches!(direction, FlexDirection::RowReverse | FlexDirection::ColumnReverse);
    let main_size = if is_row { available_width } else { i32::MAX / 2 };

    // Collect visible flex items.
    let mut items: Vec<FlexItem> = Vec::new();
    for &cid in child_ids {
        let st = &styles[cid];
        if st.display == Display::None { continue; }
        if matches!(st.position, Position::Absolute | Position::Fixed) { continue; }
        items.push(FlexItem {
            node_id: cid,
            grow: st.flex_grow,
            shrink: st.flex_shrink,
            order: st.order,
            main_base: 0,
            cross_base: 0,
            layout: None,
        });
    }

    // Sort by order.
    items.sort_by(|a, b| a.order.cmp(&b.order));

    // Phase 1: Measure each item to get its base size.
    for item in &mut items {
        let st = &styles[item.node_id];
        if let Some(basis) = st.flex_basis {
            item.main_base = basis;
        } else if is_row {
            if let Some(w) = st.width {
                item.main_base = w;
            } else if let Some(pct) = st.width_pct {
                item.main_base = (available_width as i64 * pct as i64 / 10000) as i32;
            } else if let Some((px100, pct100)) = st.width_calc {
                item.main_base = px100 / 100 + (available_width as i64 * pct100 as i64 / 10000) as i32;
            } else {
                let child_box = build_block(dom, styles, item.node_id, available_width, images);
                item.main_base = child_box.width + child_box.margin.left + child_box.margin.right;
                item.cross_base = child_box.height + child_box.margin.top + child_box.margin.bottom;
                item.layout = Some(child_box);
            }
        } else {
            if let Some(h) = st.height {
                item.main_base = h;
            } else {
                let child_box = build_block(dom, styles, item.node_id, available_width, images);
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

        if wrap != FlexWrap::Nowrap && line_start < i && new_main > main_size {
            lines.push(FlexLine { start: line_start, end: i, total_main: line_main });
            line_start = i;
            line_main = item_main;
        } else {
            line_main = new_main;
        }
    }
    if line_start < items.len() {
        lines.push(FlexLine { start: line_start, end: items.len(), total_main: line_main });
    }

    // Phase 3: Resolve flexible lengths and position items.
    let cross_gap = if is_row { row_gap } else { col_gap };
    let mut cross_cursor: i32 = parent.padding.top;

    for line in &lines {
        let count = line.end - line.start;
        if count == 0 { continue; }

        // Distribute free space along main axis.
        let total_gaps = gap * (count as i32 - 1).max(0);
        let free_space = main_size - line.total_main;

        let total_grow: i32 = items[line.start..line.end].iter()
            .map(|it| it.grow).sum();
        let total_shrink: i32 = items[line.start..line.end].iter()
            .map(|it| it.shrink).sum();

        // Compute final main sizes.
        let mut main_sizes: Vec<i32> = Vec::with_capacity(count);
        for i in line.start..line.end {
            let base = items[i].main_base;
            let final_size = if free_space > 0 && total_grow > 0 {
                base + (free_space as i64 * items[i].grow as i64 / total_grow as i64) as i32
            } else if free_space < 0 && total_shrink > 0 {
                (base + (free_space as i64 * items[i].shrink as i64 / total_shrink as i64) as i32).max(0)
            } else {
                base
            };
            main_sizes.push(final_size);
        }

        // Re-layout items with resolved sizes.
        let mut cross_max: i32 = 0;
        for (idx, i) in (line.start..line.end).enumerate() {
            let item_main = main_sizes[idx];
            let child_avail = if is_row { item_main } else { available_width };
            let mut child_box = if let Some(existing) = items[i].layout.take() {
                if is_row && (existing.width + existing.margin.left + existing.margin.right) != item_main {
                    build_block(dom, styles, items[i].node_id, child_avail, images)
                } else {
                    existing
                }
            } else {
                build_block(dom, styles, items[i].node_id, child_avail, images)
            };

            if is_row && total_grow > 0 && items[i].grow > 0 {
                child_box.width = item_main - child_box.margin.left - child_box.margin.right;
            }

            let item_cross = if is_row {
                child_box.height + child_box.margin.top + child_box.margin.bottom
            } else {
                child_box.width + child_box.margin.left + child_box.margin.right
            };

            if item_cross > cross_max { cross_max = item_cross; }
            items[i].layout = Some(child_box);
        }

        // Position items along main axis.
        let used_main: i32 = main_sizes.iter().sum::<i32>() + total_gaps;
        let remaining = main_size - used_main;

        let (mut main_cursor, main_gap_extra) = match justify {
            JustifyContent::FlexStart => (0, 0),
            JustifyContent::FlexEnd => (remaining.max(0), 0),
            JustifyContent::Center => (remaining.max(0) / 2, 0),
            JustifyContent::SpaceBetween => {
                if count > 1 {
                    (0, remaining.max(0) / (count as i32 - 1))
                } else {
                    (0, 0)
                }
            }
            JustifyContent::SpaceAround => {
                let space = remaining.max(0) / (count as i32 * 2).max(1);
                (space, space * 2)
            }
            JustifyContent::SpaceEvenly => {
                let space = remaining.max(0) / (count as i32 + 1).max(1);
                (space, space)
            }
        };

        if is_reverse {
            main_cursor = main_size;
        }

        for (idx, i) in (line.start..line.end).enumerate() {
            let item_node = items[i].node_id;
            let item_align = resolve_align(align, &styles[item_node]);
            let no_explicit_h = styles[item_node].height.is_none();
            let no_explicit_w = styles[item_node].width.is_none();
            let item_main = main_sizes[idx];

            let child_box = items[i].layout.as_mut().unwrap();

            if is_row {
                if is_reverse {
                    main_cursor -= item_main;
                    child_box.x = parent.padding.left + main_cursor + child_box.margin.left;
                    if idx > 0 { main_cursor -= gap + main_gap_extra; }
                } else {
                    child_box.x = parent.padding.left + main_cursor + child_box.margin.left;
                    main_cursor += item_main + gap;
                    if idx < count - 1 { main_cursor += main_gap_extra; }
                }

                let item_h = child_box.height + child_box.margin.top + child_box.margin.bottom;
                let cross_offset = match item_align {
                    AlignItems::FlexStart => 0,
                    AlignItems::FlexEnd => (cross_max - item_h).max(0),
                    AlignItems::Center => (cross_max - item_h).max(0) / 2,
                    AlignItems::Stretch => {
                        if no_explicit_h {
                            child_box.height = cross_max - child_box.margin.top - child_box.margin.bottom;
                        }
                        0
                    }
                    AlignItems::Baseline => 0,
                };
                child_box.y = cross_cursor + cross_offset + child_box.margin.top;
            } else {
                if is_reverse {
                    main_cursor -= item_main;
                    child_box.y = cross_cursor + main_cursor + child_box.margin.top;
                    if idx > 0 { main_cursor -= gap + main_gap_extra; }
                } else {
                    child_box.y = cross_cursor + main_cursor + child_box.margin.top;
                    main_cursor += item_main + gap;
                    if idx < count - 1 { main_cursor += main_gap_extra; }
                }

                let item_w = child_box.width + child_box.margin.left + child_box.margin.right;
                let cross_offset = match item_align {
                    AlignItems::FlexStart => 0,
                    AlignItems::FlexEnd => (available_width - item_w).max(0),
                    AlignItems::Center => (available_width - item_w).max(0) / 2,
                    AlignItems::Stretch => {
                        if no_explicit_w {
                            child_box.width = available_width - child_box.margin.left - child_box.margin.right;
                        }
                        0
                    }
                    AlignItems::Baseline => 0,
                };
                child_box.x = parent.padding.left + cross_offset + child_box.margin.left;
            }
        }

        // Move items into parent.
        for i in line.start..line.end {
            if let Some(child_box) = items[i].layout.take() {
                parent.children.push(child_box);
            }
        }

        cross_cursor += cross_max + cross_gap;
    }

    cross_cursor
}
