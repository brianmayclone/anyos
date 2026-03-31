//! Flexbox layout: `layout_flex()` implements the CSS Flexible Box Layout.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{
    ComputedStyle, Display, Position, FlexDirection, FlexWrap,
    JustifyContent, AlignItems, AlignContent, PseudoStyles,
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
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    images: &ImageCache,
    viewport_w: i32,
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
    let is_reverse = matches!(direction, FlexDirection::RowReverse | FlexDirection::ColumnReverse);
    // §9.2 Step 1: Determine available main space.
    // For row flex: available_width is the container's content width.
    // For column flex: use explicit height, then min-height, then 0 (content-driven).
    // Per spec: min-height provides a definite size for justify-content distribution.
    let main_size = if is_row {
        available_width
    } else {
        let explicit_h = parent_style.height.unwrap_or(0);
        let min_h = parent_style.min_height;
        let height_pct = parent_style.height_pct;
        if explicit_h > 0 {
            explicit_h
        } else if let Some(pct) = height_pct {
            // height as percentage of viewport (approximation).
            if pct > 0 { (available_width as i64 * pct as i64 / 10000) as i32 } else { 0 }
        } else if min_h > 0 {
            // min-height provides the definite size for centering.
            min_h
        } else {
            0 // content-driven
        }
    };

    // Collect visible flex items.
    // Per spec §4: each in-flow child becomes a flex item.
    // Skip hidden inputs (they generate no box).
    let mut items: Vec<FlexItem> = Vec::new();
    for &cid in child_ids {
        let st = &styles[cid];
        if st.display == Display::None { continue; }
        if matches!(st.position, Position::Absolute | Position::Fixed) { continue; }
        // Skip <input type="hidden"> — generates no box.
        if dom.tag(cid) == Some(Tag::Input) {
            if dom.attr(cid, "type").unwrap_or("") == "hidden" { continue; }
        }
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

    for item in &mut items {
        // Handle text nodes as anonymous flex items (CSS Flexbox §4).
        if let NodeType::Text(ref text) = dom.nodes[item.node_id].node_type {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                item.main_base = 0;
                item.cross_base = 0;
                continue;
            }
            let (tw, th) = super::measure_text(trimmed, parent_font_size, parent_bold);
            let mut text_box = super::LayoutBox::new_text(
                String::from(trimmed), parent_font_size, parent_bold, false, parent_color,
            );
            text_box.width = tw;
            text_box.height = th;
            if is_row {
                item.main_base = tw;
                item.cross_base = th;
            } else {
                item.main_base = th;
                item.cross_base = tw;
            }
            item.layout = Some(text_box);
            continue;
        }

        let st = &styles[item.node_id];
        if let Some(basis) = st.flex_basis {
            // Case A: definite flex-basis.
            item.main_base = basis;
        } else if is_row {
            if let Some(w) = st.width {
                // Case A: definite width.
                item.main_base = w;
            } else if let Some(pct) = st.width_pct {
                item.main_base = (available_width as i64 * pct as i64 / 10000) as i32;
            } else if let Some((px100, pct100)) = st.width_calc {
                item.main_base = px100 / 100 + (available_width as i64 * pct100 as i64 / 10000) as i32;
            } else {
                // Case E: flex-basis:auto + width:auto → max-content size.
                // Lay out with a large width to get the natural (max-content) size.
                let max_content_w = available_width * 4; // generous upper bound
                let child_box = build_block(dom, styles, pseudo, item.node_id,
                    max_content_w, images, viewport_w);
                // max-content = actual content extent (how wide the content wants to be).
                let content_w = child_box.children.iter()
                    .map(|c| c.x + c.width + c.margin.right)
                    .max()
                    .unwrap_or(0);
                let max_w = content_w + child_box.padding.left + child_box.padding.right
                    + child_box.border_width * 2;
                // Use max-content but cap at the container width (can't exceed parent).
                let base_w = max_w.max(1).min(available_width);
                item.main_base = base_w + child_box.margin.left + child_box.margin.right;
                item.cross_base = child_box.height + child_box.margin.top + child_box.margin.bottom;
                // Don't cache the layout — it was done at max-content width,
                // Phase 3 will re-layout at the resolved width.
            }
        } else {
            if let Some(h) = st.height {
                item.main_base = h;
            } else {
                let child_box = build_block(dom, styles, pseudo, item.node_id, available_width, images, viewport_w);
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
    let bw = parent.border_width;
    let mut cross_cursor: i32 = bw + parent.padding.top;

    for line in &lines {
        let count = line.end - line.start;
        if count == 0 { continue; }

        // Distribute free space along main axis.
        let total_gaps = gap * (count as i32 - 1).max(0);
        let free_space = main_size - line.total_main - total_gaps;

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
                    build_block(dom, styles, pseudo, items[i].node_id, child_avail, images, viewport_w)
                } else {
                    existing
                }
            } else {
                // No cached layout (e.g. row-flex items measured at max-content width).
                build_block(dom, styles, pseudo, items[i].node_id, child_avail, images, viewport_w)
            };

            // §8.1: Auto margins on flex items are treated as 0 during layout.
            // build_block may have set auto margins for block-centering — reset them.
            // The flex positioning code will distribute auto margins later.
            let st = &styles[items[i].node_id];
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
                }
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
        // If main_size is 0 (content-sized container), use the actual content size.
        let effective_main = if main_size > 0 { main_size } else { used_main };
        let remaining = effective_main - used_main;

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
                    child_box.x = bw + parent.padding.left + main_cursor + child_box.margin.left;
                    if idx > 0 { main_cursor -= gap + main_gap_extra; }
                } else {
                    child_box.x = bw + parent.padding.left + main_cursor + child_box.margin.left;
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
                } else { match item_align {
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
                } }; // close match + else
                child_box.x = bw + parent.padding.left + cross_offset + child_box.margin.left;
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

    // Apply align-content: redistribute cross-axis space between flex lines.
    // Only meaningful when wrapping and there's a definite container height.
    if lines.len() > 1 && align_content != AlignContent::Stretch {
        let total_cross = cross_cursor;
        // If the parent has an explicit height, use that for free space calculation.
        let container_cross = if let Some(h) = parent_style.height {
            h
        } else {
            total_cross // no free space if height is auto
        };
        let free = container_cross - total_cross;
        if free > 0 {
            let line_count = lines.len() as i32;
            let shift = match align_content {
                AlignContent::FlexEnd => free,
                AlignContent::Center => free / 2,
                AlignContent::SpaceBetween => 0, // handled below
                AlignContent::SpaceAround => 0,  // handled below
                AlignContent::SpaceEvenly => 0,  // handled below
                _ => 0,
            };
            // For space-between/around/evenly, calculate per-gap extra.
            let (initial_offset, gap_extra) = match align_content {
                AlignContent::SpaceBetween if line_count > 1 => {
                    (0, free / (line_count - 1))
                }
                AlignContent::SpaceAround if line_count > 0 => {
                    let per = free / line_count;
                    (per / 2, per)
                }
                AlignContent::SpaceEvenly if line_count > 0 => {
                    let per = free / (line_count + 1);
                    (per, per)
                }
                _ => (shift, 0),
            };
            // Apply offsets to all children.
            if initial_offset > 0 || gap_extra > 0 {
                let mut cumulative = initial_offset;
                let mut line_idx = 0;
                for child in &mut parent.children {
                    child.y += cumulative;
                    // Advance to next line's offset after each line boundary.
                    // This is approximate — we shift all children by increasing amounts.
                    line_idx += 1;
                    if gap_extra > 0 && line_idx > 0 {
                        cumulative = initial_offset + gap_extra * (line_idx as i32).min(line_count - 1);
                    }
                }
            }
        }
    }

    cross_cursor
}
