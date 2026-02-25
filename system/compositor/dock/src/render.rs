//! Dock rendering, hit testing, and animation helpers.

use anyos_std::anim::AnimSet;

use crate::framebuffer::Framebuffer;
use crate::theme::*;
use crate::types::DockItem;

/// Drag-and-drop state passed to the renderer.
pub struct DragInfo {
    pub source_idx: usize,
    pub mouse_x: i32,
}

/// Compute bounce Y-offset for an icon being launched.
///
/// 3 bounces over 2000ms with decreasing amplitude (16, 10, 5 pixels).
/// Uses a parabolic sine-approximation for smooth, natural motion.
fn bounce_offset(elapsed_ms: u32) -> i32 {
    if elapsed_ms >= 2000 {
        return 0;
    }
    let bounce_dur = 667u32;
    let bounce_idx = (elapsed_ms / bounce_dur).min(2);
    let t_in_bounce = elapsed_ms - bounce_idx * bounce_dur;
    let peak: i32 = match bounce_idx {
        0 => 16,
        1 => 10,
        _ => 5,
    };
    let t = t_in_bounce as i64;
    let d = bounce_dur as i64;
    let sine_approx = (4 * t * (d - t) * 1000 / (d * d)) as i32;
    peak * sine_approx / 1000
}

/// Draw a soft shadow around a rounded rectangle using concentric expanded rects.
fn draw_shadow(fb: &mut Framebuffer, x: i32, y: i32, w: u32, h: u32, r: i32, offset_y: i32, spread: i32, max_alpha: u32) {
    for i in 1..=spread {
        let alpha = max_alpha * (spread - i) as u32 / spread as u32;
        if alpha == 0 { continue; }
        let color = alpha << 24; // black with decreasing alpha
        fb.fill_rounded_rect(
            x - i,
            y - i + offset_y,
            w + 2 * i as u32,
            h + 2 * i as u32,
            r + i,
            color,
        );
    }
}

/// Blit an icon with alpha blending at reduced opacity (for drag ghost).
fn blit_icon_ghost(fb: &mut Framebuffer, icon: &crate::types::Icon, x: i32, y: i32) {
    let stride = fb.width as i32;
    for row in 0..icon.height as i32 {
        let dy = y + row;
        if dy < 0 || dy >= fb.height as i32 { continue; }
        for col in 0..icon.width as i32 {
            let dx = x + col;
            if dx < 0 || dx >= stride { continue; }
            let src = icon.pixels[(row as u32 * icon.width + col as u32) as usize];
            let src_a = (src >> 24) & 0xFF;
            if src_a == 0 { continue; }
            // 50% opacity
            let ghost_a = src_a / 2;
            let dst_idx = (dy * stride + dx) as usize;
            let dst = fb.pixels[dst_idx];
            let inv_a = 255 - ghost_a;
            let r = ((((src >> 16) & 0xFF) * ghost_a + ((dst >> 16) & 0xFF) * inv_a) / 255) & 0xFF;
            let g = ((((src >> 8) & 0xFF) * ghost_a + ((dst >> 8) & 0xFF) * inv_a) / 255) & 0xFF;
            let b = (((src & 0xFF) * ghost_a + (dst & 0xFF) * inv_a) / 255) & 0xFF;
            fb.pixels[dst_idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }
}

/// Draw a single dock item at the given X position.
fn draw_item(fb: &mut Framebuffer, item: &DockItem, i: usize, ix: i32, base_icon_y: i32, hz: u32, rs: &RenderState) {
    let extra = rs.anims.value_or(100 + i as u32, rs.now, 0).max(0) / 1000;
    let extra_u = extra as u32;
    let draw_w = DOCK_ICON_SIZE + extra_u;
    let draw_h = DOCK_ICON_SIZE + extra_u;
    let offset_x = -(extra / 2);
    let offset_y = -(extra / 2);

    let bounce_y = rs.bounce_items.iter()
        .find(|(idx, _)| *idx == i)
        .map(|(_, start)| {
            let elapsed_ms = rs.now.wrapping_sub(*start) * 1000 / hz;
            bounce_offset(elapsed_ms)
        })
        .unwrap_or(0);

    let icon_x = ix + offset_x;
    let icon_y = base_icon_y + offset_y - bounce_y;

    if let Some(ref icon) = item.icon {
        if extra_u > 0 {
            fb.blit_icon_scaled(icon, icon_x, icon_y, draw_w, draw_h);
        } else {
            fb.blit_icon(icon, icon_x, icon_y);
        }
    } else {
        fb.fill_rounded_rect(icon_x, icon_y, draw_w, draw_h, 10, 0xFF3C3C41);
    }

    // Running indicator dot
    if item.running {
        let dot_x = ix + DOCK_ICON_SIZE as i32 / 2;
        let dot_y = base_icon_y + DOCK_ICON_SIZE as i32 + 5;
        fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
    }
}

pub struct RenderState<'a> {
    pub hover_idx: Option<usize>,
    pub anims: &'a AnimSet,
    pub bounce_items: &'a [(usize, u32)],
    pub now: u32,
    pub drag: Option<DragInfo>,
}

pub fn render_dock(fb: &mut Framebuffer, items: &[DockItem], screen_width: u32, rs: &RenderState) {
    fb.clear();

    let item_count = items.len() as u32;
    if item_count == 0 {
        return;
    }

    let total_width = item_count * DOCK_ICON_SIZE
        + (item_count - 1) * DOCK_ICON_SPACING
        + DOCK_H_PADDING * 2;

    let dock_x = (screen_width as i32 - total_width as i32) / 2;
    let dock_y = DOCK_MARGIN as i32;

    // Soft shadow around the pill
    draw_shadow(fb, dock_x, dock_y, total_width, DOCK_HEIGHT, DOCK_BORDER_RADIUS, 4, 12, 40);

    // Glass pill background
    fb.fill_rounded_rect(dock_x, dock_y, total_width, DOCK_HEIGHT, DOCK_BORDER_RADIUS, dock_bg());

    // Top highlight line for depth
    fb.fill_rect(
        dock_x + DOCK_BORDER_RADIUS,
        dock_y,
        total_width - DOCK_BORDER_RADIUS as u32 * 2,
        1,
        COLOR_HIGHLIGHT,
    );

    // Draw each item
    let base_icon_y = dock_y + ((DOCK_HEIGHT as i32 - DOCK_ICON_SIZE as i32) / 2) - 2;
    let mut ix = dock_x + DOCK_H_PADDING as i32;
    let hz = anyos_std::sys::tick_hz().max(1);
    let item_stride_px = (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;

    // Compute drag layout: gap slot position where the dragged icon will land
    let drag_layout = rs.drag.as_ref().map(|d| {
        let drop_target = compute_drop_index(d.mouse_x, screen_width, items, d.source_idx);
        let insert_at = if drop_target > d.source_idx {
            drop_target - 1
        } else {
            drop_target
        };
        (d.source_idx, insert_at)
    });

    let mut dragged_icon_info: Option<(i32, &crate::types::Icon)> = None;

    if let Some((source_idx, gap_slot)) = drag_layout {
        // ── Drag mode: draw N-1 items into N slots, leaving a gap ──
        let mut slot = 0usize; // visual slot counter
        let mut gap_done = false;

        for (i, item) in items.iter().enumerate() {
            if i == source_idx {
                // Save dragged icon for ghost rendering
                if let Some(ref icon) = item.icon {
                    dragged_icon_info = Some((base_icon_y, icon));
                }
                continue;
            }

            // Insert gap before this item if we've reached the gap slot
            if !gap_done && slot == gap_slot {
                ix += item_stride_px; // empty slot
                slot += 1;
                gap_done = true;
            }

            draw_item(fb, item, i, ix, base_icon_y, hz, rs);
            ix += item_stride_px;
            slot += 1;
        }
        // If gap is at the very end (after all remaining items), nothing extra needed
    } else {
        // ── Normal mode: no drag ──
        for (i, item) in items.iter().enumerate() {
            draw_item(fb, item, i, ix, base_icon_y, hz, rs);
            ix += item_stride_px;
        }
    }

    // Draw drag ghost icon at mouse position
    if let Some(ref drag) = rs.drag {
        if let Some((base_y, icon)) = dragged_icon_info {
            let ghost_x = drag.mouse_x - DOCK_ICON_SIZE as i32 / 2;
            blit_icon_ghost(fb, icon, ghost_x, base_y);
        }
    }

    // Tooltip for hovered item (not during drag)
    if rs.drag.is_none() {
        if let Some(idx) = rs.hover_idx {
            if let Some(item) = items.get(idx) {
                let name = &item.name;
                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, name);
                let pill_w = tw + TOOLTIP_PAD * 2;
                let pill_h = th + 8;

                let icon_center_x = dock_x + DOCK_H_PADDING as i32
                    + idx as i32 * item_stride_px
                    + DOCK_ICON_SIZE as i32 / 2;
                let pill_x = icon_center_x - pill_w as i32 / 2;
                let pill_y = dock_y - pill_h as i32 - 4;

                fb.fill_rounded_rect(pill_x, pill_y, pill_w, pill_h, 6, tooltip_bg());

                let text_x = pill_x + TOOLTIP_PAD as i32;
                let text_y = pill_y + ((pill_h as i32 - th as i32) / 2);
                fb.draw_text(text_x, text_y, name, tooltip_text());
            }
        }
    }
}

/// Hit-test a local coordinate against dock items. Returns item index if hit.
pub fn dock_hit_test(x: i32, y: i32, screen_width: u32, items: &[DockItem]) -> Option<usize> {
    let item_count = items.len() as u32;
    if item_count == 0 {
        return None;
    }

    let total_width = item_count * DOCK_ICON_SIZE
        + (item_count - 1) * DOCK_ICON_SPACING
        + DOCK_H_PADDING * 2;

    let dock_x = (screen_width as i32 - total_width as i32) / 2;
    let dock_y = DOCK_MARGIN as i32;

    if x < dock_x || x >= dock_x + total_width as i32 {
        return None;
    }
    if y < dock_y || y >= dock_y + DOCK_HEIGHT as i32 {
        return None;
    }

    let local_x = x - dock_x - DOCK_H_PADDING as i32;
    if local_x < 0 {
        return None;
    }

    let item_stride = DOCK_ICON_SIZE + DOCK_ICON_SPACING;
    let idx = local_x as u32 / item_stride;
    if idx < item_count {
        Some(idx as usize)
    } else {
        None
    }
}

/// Compute where a dragged item would be inserted based on mouse X position.
fn compute_drop_index(mouse_x: i32, screen_width: u32, items: &[DockItem], _source_idx: usize) -> usize {
    let item_count = items.len() as u32;
    if item_count == 0 { return 0; }

    let total_width = item_count * DOCK_ICON_SIZE
        + (item_count - 1) * DOCK_ICON_SPACING
        + DOCK_H_PADDING * 2;
    let dock_x = (screen_width as i32 - total_width as i32) / 2;

    let local_x = mouse_x - dock_x - DOCK_H_PADDING as i32;
    if local_x < 0 { return 0; }

    let item_stride = (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
    // Calculate position based on center of each icon slot
    let slot = (local_x + item_stride / 2) / item_stride;
    (slot as usize).min(items.len())
}

/// Public wrapper for computing drop index (used by main.rs).
pub fn drag_drop_index(mouse_x: i32, screen_width: u32, items: &[DockItem], source_idx: usize) -> usize {
    compute_drop_index(mouse_x, screen_width, items, source_idx)
}
