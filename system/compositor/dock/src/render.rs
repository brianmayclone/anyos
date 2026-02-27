//! Dock rendering, hit testing, magnification, and position-aware layout.

use alloc::vec;
use alloc::vec::Vec;

use crate::framebuffer::Framebuffer;
use crate::settings::{DockSettings, POS_BOTTOM, POS_LEFT};
use crate::theme::*;
use crate::types::DockItem;

/// Drag-and-drop state passed to the renderer.
pub struct DragInfo {
    pub source_idx: usize,
    pub mouse_x: i32,
    pub mouse_y: i32,
}

/// Render state passed from main loop to renderer each frame.
pub struct RenderState<'a> {
    pub hover_idx: Option<usize>,
    pub bounce_items: &'a [(usize, u32)],
    pub now: u32,
    pub drag: Option<DragInfo>,
    /// Mouse position along the dock axis (X for bottom, Y for left/right).
    pub mouse_along: i32,
    /// Magnification progress 0..1000 (animated ramp-in/out).
    pub mag_progress: i32,
    pub settings: &'a DockSettings,
}

/// Compute bounce Y-offset for an icon being launched.
///
/// 3 bounces over 2000ms with decreasing amplitude (16, 10, 5 pixels).
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

/// Compute magnified sizes for all icons using a bell-curve distance falloff.
///
/// Returns a Vec of rendered icon sizes (one per item).
fn compute_magnified_sizes(
    item_count: usize,
    geom: &DockGeometry,
    settings: &DockSettings,
    mouse_along: i32,
    mag_progress: i32,
    dock_origin: i32,
) -> Vec<u32> {
    let icon_size = geom.icon_size;

    if !settings.magnification || mag_progress <= 0 || item_count == 0 {
        return vec![icon_size; item_count];
    }

    let stride = (icon_size + geom.icon_spacing) as i32;
    let max_range = stride * 3; // falloff over ~3 icon widths
    let max_extra = settings.mag_size as i32 - icon_size as i32;

    let mut sizes = Vec::with_capacity(item_count);
    for i in 0..item_count {
        // Center position of this icon along the dock axis
        let center = dock_origin + geom.h_padding as i32
            + i as i32 * stride
            + icon_size as i32 / 2;

        let distance = (center - mouse_along).abs();

        let extra = if distance >= max_range || max_extra <= 0 {
            0i32
        } else {
            // Quadratic falloff: f(t) = (1 - t)^2
            let t = distance * 1000 / max_range; // 0..1000
            let inv = 1000 - t;
            let factor = inv * inv / 1000; // 0..1000
            factor * max_extra / 1000
        };

        // Scale extra by magnification animation progress
        let scaled_extra = extra * mag_progress / 1000;
        sizes.push((icon_size as i32 + scaled_extra) as u32);
    }

    sizes
}

/// Draw a soft shadow around a rounded rectangle (4 steps for performance).
fn draw_shadow(fb: &mut Framebuffer, x: i32, y: i32, w: u32, h: u32, r: i32, offset_y: i32, spread: i32, max_alpha: u32) {
    // 4 evenly spaced rings instead of `spread` (typically 12) — 3× faster
    let steps: i32 = 4;
    for s in 1..=steps {
        let i = s * spread / steps;
        let alpha = max_alpha * (spread - i) as u32 / spread as u32;
        if alpha == 0 { continue; }
        let color = alpha << 24;
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

/// Choose the best icon source for rendering at a given draw size.
/// Uses icon_hires when available and draw_size > base icon_size.
fn pick_icon<'a>(item: &'a DockItem, draw_size: u32, base_size: u32) -> Option<&'a crate::types::Icon> {
    if draw_size > base_size {
        item.icon_hires.as_ref().or(item.icon.as_ref())
    } else {
        item.icon.as_ref()
    }
}

// ── Bottom dock rendering ───────────────────────────────────────────────────

pub fn render_dock(fb: &mut Framebuffer, items: &[DockItem], screen_w: u32, screen_h: u32, rs: &RenderState) {
    fb.clear();

    let item_count = items.len();
    if item_count == 0 {
        return;
    }

    let geom = geometry();

    match geom.position {
        POS_LEFT => render_vertical(fb, items, screen_w, screen_h, rs, true),
        POS_BOTTOM => render_horizontal(fb, items, screen_w, rs),
        _ => render_vertical(fb, items, screen_w, screen_h, rs, false), // POS_RIGHT
    }
}

fn render_horizontal(fb: &mut Framebuffer, items: &[DockItem], screen_width: u32, rs: &RenderState) {
    let geom = geometry();
    let item_count = items.len();
    let icon_size = geom.icon_size;
    let spacing = geom.icon_spacing;

    // Pill height is CONSTANT — never changes with magnification
    let pill_h = geom.dock_height; // icon_size + 16

    // Compute magnified sizes for layout
    let base_total_w = item_count as u32 * icon_size
        + (item_count as u32 - 1) * spacing
        + geom.h_padding * 2;
    let base_dock_x = (screen_width as i32 - base_total_w as i32) / 2;

    let sizes = if rs.drag.is_some() {
        vec![icon_size; item_count]
    } else {
        compute_magnified_sizes(item_count, geom, rs.settings, rs.mouse_along, rs.mag_progress, base_dock_x)
    };

    // Total width uses magnified sizes + spacing between each pair
    let total_icon_w: u32 = sizes.iter().sum();
    let total_w = total_icon_w
        + if item_count > 1 { (item_count as u32 - 1) * spacing } else { 0 }
        + geom.h_padding * 2;

    let dock_x = (screen_width as i32 - total_w as i32) / 2;
    let dock_y = geom.margin as i32;

    // Shadow + pill background (constant height)
    draw_shadow(fb, dock_x, dock_y, total_w, pill_h, geom.border_radius, 4, 12, 40);
    fb.fill_rounded_rect(dock_x, dock_y, total_w, pill_h, geom.border_radius, dock_bg());

    // Top highlight line
    let hl_inset = geom.border_radius as u32;
    if total_w > hl_inset * 2 {
        fb.fill_rect(
            dock_x + geom.border_radius,
            dock_y,
            total_w - hl_inset * 2,
            1,
            COLOR_HIGHLIGHT,
        );
    }

    let hz = anyos_std::sys::tick_hz().max(1);

    // Baseline: bottom of icons aligned to pill bottom - 8px
    let icon_baseline_y = dock_y + pill_h as i32 - 8;

    // Compute drag layout if active
    let drag_layout = rs.drag.as_ref().map(|d| {
        let drop_target = compute_drop_index_h(d.mouse_x, screen_width, items, d.source_idx, icon_size, spacing, geom.h_padding);
        let insert_at = if drop_target > d.source_idx { drop_target - 1 } else { drop_target };
        (d.source_idx, insert_at)
    });

    let mut dragged_icon_info: Option<(i32, &crate::types::Icon)> = None;
    let base_stride = (icon_size + spacing) as i32;

    if let Some((source_idx, gap_slot)) = drag_layout {
        // Drag mode: draw N-1 items with a gap (no magnification)
        let mut ix = dock_x + geom.h_padding as i32;
        let mut slot = 0usize;
        let mut gap_done = false;

        for (i, item) in items.iter().enumerate() {
            if i == source_idx {
                if let Some(ref icon) = item.icon {
                    let base_icon_y = icon_baseline_y - icon_size as i32;
                    dragged_icon_info = Some((base_icon_y, icon));
                }
                continue;
            }

            if !gap_done && slot == gap_slot {
                ix += base_stride;
                slot += 1;
                gap_done = true;
            }

            let base_icon_y = icon_baseline_y - icon_size as i32;
            let bounce_y = get_bounce_offset(i, rs.bounce_items, rs.now, hz);
            let icon_y = base_icon_y - bounce_y;

            if let Some(ref icon) = item.icon {
                fb.blit_icon(icon, ix, icon_y);
            } else {
                fb.fill_rounded_rect(ix, icon_y, icon_size, icon_size, 10, 0xFF3C3C41);
            }

            if item.running {
                let dot_x = ix + icon_size as i32 / 2;
                let dot_y = icon_baseline_y + 5;
                fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
            }

            ix += base_stride;
            slot += 1;
        }

        // Draw drag ghost
        if let Some(ref drag) = rs.drag {
            if let Some((base_y, icon)) = dragged_icon_info {
                let ghost_x = drag.mouse_x - icon_size as i32 / 2;
                blit_icon_ghost(fb, icon, ghost_x, base_y);
            }
        }
    } else {
        // Normal mode with magnification
        // Each icon gets its own width slot; advance by draw_size + spacing
        let mut ix = dock_x + geom.h_padding as i32;

        for (i, item) in items.iter().enumerate() {
            let draw_size = sizes[i];

            // Icon grows UPWARD: bottom edge pinned to baseline
            let icon_y_base = icon_baseline_y - draw_size as i32;
            let bounce_y = get_bounce_offset(i, rs.bounce_items, rs.now, hz);
            let icon_y = icon_y_base - bounce_y;

            // Icon is centered within its draw_size slot
            let icon_x = ix;

            if let Some(icon) = pick_icon(item, draw_size, icon_size) {
                if draw_size != icon.width {
                    fb.blit_icon_scaled(icon, icon_x, icon_y, draw_size, draw_size);
                } else {
                    fb.blit_icon(icon, icon_x, icon_y);
                }
            } else {
                fb.fill_rounded_rect(icon_x, icon_y, draw_size, draw_size, 10, 0xFF3C3C41);
            }

            // Running indicator dot (at fixed baseline)
            if item.running {
                let dot_x = ix + draw_size as i32 / 2;
                let dot_y = icon_baseline_y + 5;
                fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
            }

            // Advance by THIS icon's magnified width + spacing
            ix += draw_size as i32 + spacing as i32;
        }
    }

    // Tooltip for hovered item (not during drag)
    if rs.drag.is_none() {
        if let Some(idx) = rs.hover_idx {
            if let Some(item) = items.get(idx) {
                let name = &item.name;
                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, name);
                let pill_w = tw + TOOLTIP_PAD * 2;
                let pill_tooltip_h = th + 8;

                // Compute center X of this icon by summing widths of preceding icons
                let mut cx = dock_x + geom.h_padding as i32;
                for j in 0..idx {
                    cx += sizes[j] as i32 + spacing as i32;
                }
                cx += sizes[idx] as i32 / 2;

                // Position tooltip above the magnified icon's top edge (constant gap)
                let icon_top_y = icon_baseline_y - sizes[idx] as i32;
                let pill_x = cx - pill_w as i32 / 2;
                let pill_y = icon_top_y - pill_tooltip_h as i32 - 4;

                fb.fill_rounded_rect(pill_x, pill_y, pill_w, pill_tooltip_h, 6, tooltip_bg());

                let text_x = pill_x + TOOLTIP_PAD as i32;
                let text_y = pill_y + ((pill_tooltip_h as i32 - th as i32) / 2);
                fb.draw_text(text_x, text_y, name, tooltip_text());
            }
        }
    }
}

fn render_vertical(fb: &mut Framebuffer, items: &[DockItem], _screen_w: u32, screen_h: u32, rs: &RenderState, is_left: bool) {
    let geom = geometry();
    let item_count = items.len();
    let icon_size = geom.icon_size;
    let spacing = geom.icon_spacing;

    // Pill width is CONSTANT — never changes with magnification
    let pill_w = geom.dock_height; // icon_size + 16

    // Compute magnified sizes
    let base_total_h = item_count as u32 * icon_size
        + (item_count as u32 - 1) * spacing
        + geom.h_padding * 2;
    let base_dock_y = (screen_h as i32 - base_total_h as i32) / 2;

    let sizes = if rs.drag.is_some() {
        vec![icon_size; item_count]
    } else {
        compute_magnified_sizes(item_count, geom, rs.settings, rs.mouse_along, rs.mag_progress, base_dock_y)
    };

    // Total height uses magnified sizes + spacing
    let total_icon_h: u32 = sizes.iter().sum();
    let total_h = total_icon_h
        + if item_count > 1 { (item_count as u32 - 1) * spacing } else { 0 }
        + geom.h_padding * 2;

    let dock_y = (screen_h as i32 - total_h as i32) / 2;
    // Pill flush with screen edge; tooltip space is on the inner side
    let dock_x = if is_left {
        0
    } else {
        fb.width as i32 - pill_w as i32
    };

    // Shadow + pill background (constant width)
    draw_shadow(fb, dock_x, dock_y, pill_w, total_h, geom.border_radius, 0, 12, 40);
    fb.fill_rounded_rect(dock_x, dock_y, pill_w, total_h, geom.border_radius, dock_bg());

    // Side highlight line
    let hl_inset = geom.border_radius as u32;
    if total_h > hl_inset * 2 {
        if is_left {
            fb.fill_rect(dock_x + pill_w as i32 - 1, dock_y + geom.border_radius, 1, total_h - hl_inset * 2, COLOR_HIGHLIGHT);
        } else {
            fb.fill_rect(dock_x, dock_y + geom.border_radius, 1, total_h - hl_inset * 2, COLOR_HIGHLIGHT);
        }
    }

    let hz = anyos_std::sys::tick_hz().max(1);

    // Baseline for icons: for left dock icons grow rightward, for right they grow leftward
    // Icon center X is fixed at pill center
    let icon_baseline_x_left = dock_x + 8; // 8px from left edge of pill
    let icon_baseline_x_right = dock_x + pill_w as i32 - 8; // 8px from right edge

    let mut iy = dock_y + geom.h_padding as i32;

    for (i, item) in items.iter().enumerate() {
        let draw_size = sizes[i];

        // Icon centered horizontally in the pill
        let icon_x = if is_left {
            icon_baseline_x_left
        } else {
            icon_baseline_x_right - draw_size as i32
        };
        let icon_y = iy;

        let bounce_x = get_bounce_offset(i, rs.bounce_items, rs.now, hz);
        let icon_x_final = if is_left { icon_x + bounce_x } else { icon_x - bounce_x };

        if let Some(icon) = pick_icon(item, draw_size, icon_size) {
            if draw_size != icon.width {
                fb.blit_icon_scaled(icon, icon_x_final, icon_y, draw_size, draw_size);
            } else {
                fb.blit_icon(icon, icon_x_final, icon_y);
            }
        } else {
            fb.fill_rounded_rect(icon_x_final, icon_y, draw_size, draw_size, 10, 0xFF3C3C41);
        }

        // Running indicator dot
        if item.running {
            let dot_y = iy + draw_size as i32 / 2;
            if is_left {
                let dot_x = dock_x + pill_w as i32 - 3;
                fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
            } else {
                let dot_x = dock_x + 3;
                fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
            }
        }

        // Advance by THIS icon's magnified height + spacing
        iy += draw_size as i32 + spacing as i32;
    }

    // Tooltip for hovered item
    if rs.drag.is_none() {
        if let Some(idx) = rs.hover_idx {
            if let Some(item) = items.get(idx) {
                let name = &item.name;
                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, name);
                let pill_tw = tw + TOOLTIP_PAD * 2;
                let pill_th = th + 8;

                // Compute center Y of this icon by summing heights of preceding icons
                let mut cy = dock_y + geom.h_padding as i32;
                for j in 0..idx {
                    cy += sizes[j] as i32 + spacing as i32;
                }
                cy += sizes[idx] as i32 / 2;

                let pill_ty = cy - pill_th as i32 / 2;

                // Position tooltip at constant gap from the magnified icon's outer edge
                let pill_tx = if is_left {
                    let icon_right = icon_baseline_x_left + sizes[idx] as i32;
                    icon_right + 4
                } else {
                    let icon_left = icon_baseline_x_right - sizes[idx] as i32;
                    icon_left - pill_tw as i32 - 4
                };

                fb.fill_rounded_rect(pill_tx, pill_ty, pill_tw, pill_th, 6, tooltip_bg());
                let text_x = pill_tx + TOOLTIP_PAD as i32;
                let text_y = pill_ty + ((pill_th as i32 - th as i32) / 2);
                fb.draw_text(text_x, text_y, name, tooltip_text());
            }
        }
    }
}

// ── Hit testing ──────────────────────────────────────────────────────────────

/// Hit-test a local coordinate against dock items. Returns item index if hit.
pub fn dock_hit_test(x: i32, y: i32, screen_w: u32, screen_h: u32, items: &[DockItem], settings: &DockSettings, mouse_along: i32, mag_progress: i32) -> Option<usize> {
    let geom = geometry();
    let item_count = items.len() as u32;
    if item_count == 0 {
        return None;
    }

    match geom.position {
        POS_LEFT => hit_test_vertical(x, y, screen_h, items, geom, true),
        POS_BOTTOM => hit_test_horizontal(x, y, screen_w, items, geom),
        _ => hit_test_vertical(x, y, screen_h, items, geom, false),
    }
}

fn hit_test_horizontal(x: i32, y: i32, screen_width: u32, items: &[DockItem], geom: &DockGeometry) -> Option<usize> {
    let item_count = items.len() as u32;
    let total_width = item_count * geom.icon_size
        + (item_count - 1) * geom.icon_spacing
        + geom.h_padding * 2;

    let dock_x = (screen_width as i32 - total_width as i32) / 2;
    let dock_y = geom.margin as i32;

    if x < dock_x || x >= dock_x + total_width as i32 {
        return None;
    }
    if y < dock_y || y >= dock_y + geom.dock_height as i32 {
        return None;
    }

    let local_x = x - dock_x - geom.h_padding as i32;
    if local_x < 0 {
        return None;
    }

    let item_stride = geom.icon_size + geom.icon_spacing;
    let idx = local_x as u32 / item_stride;
    if idx < item_count {
        Some(idx as usize)
    } else {
        None
    }
}

fn hit_test_vertical(x: i32, y: i32, screen_height: u32, items: &[DockItem], geom: &DockGeometry, is_left: bool) -> Option<usize> {
    let item_count = items.len() as u32;
    let total_h = item_count * geom.icon_size
        + (item_count - 1) * geom.icon_spacing
        + geom.h_padding * 2;
    let pill_w = geom.icon_size + 16;

    let dock_y = (screen_height as i32 - total_h as i32) / 2;
    // Pill flush with screen edge
    let dock_x = if is_left {
        0
    } else {
        // Right dock: pill at right edge of framebuffer
        let fb_w = geom.total_h as i32 + crate::TOOLTIP_EXTRA_W as i32;
        fb_w - pill_w as i32
    };

    if x < dock_x || x >= dock_x + pill_w as i32 {
        return None;
    }
    if y < dock_y || y >= dock_y + total_h as i32 {
        return None;
    }

    let local_y = y - dock_y - geom.h_padding as i32;
    if local_y < 0 {
        return None;
    }

    let item_stride = geom.icon_size + geom.icon_spacing;
    let idx = local_y as u32 / item_stride;
    if idx < item_count {
        Some(idx as usize)
    } else {
        None
    }
}

// ── Drag helpers ─────────────────────────────────────────────────────────────

fn compute_drop_index_h(mouse_x: i32, screen_width: u32, items: &[DockItem], _source_idx: usize, icon_size: u32, spacing: u32, h_padding: u32) -> usize {
    let item_count = items.len() as u32;
    if item_count == 0 { return 0; }

    let total_width = item_count * icon_size
        + (item_count - 1) * spacing
        + h_padding * 2;
    let dock_x = (screen_width as i32 - total_width as i32) / 2;

    let local_x = mouse_x - dock_x - h_padding as i32;
    if local_x < 0 { return 0; }

    let item_stride = (icon_size + spacing) as i32;
    let slot = (local_x + item_stride / 2) / item_stride;
    (slot as usize).min(items.len())
}

/// Public wrapper for computing drop index (used by main.rs).
pub fn drag_drop_index(mouse_x: i32, mouse_y: i32, screen_w: u32, screen_h: u32, items: &[DockItem], source_idx: usize) -> usize {
    let geom = geometry();
    match geom.position {
        POS_LEFT | 2 /* POS_RIGHT */ => {
            let item_count = items.len() as u32;
            if item_count == 0 { return 0; }
            let total_h = item_count * geom.icon_size + (item_count - 1) * geom.icon_spacing + geom.h_padding * 2;
            let dock_y = (screen_h as i32 - total_h as i32) / 2;
            let local_y = mouse_y - dock_y - geom.h_padding as i32;
            if local_y < 0 { return 0; }
            let stride = (geom.icon_size + geom.icon_spacing) as i32;
            let slot = (local_y + stride / 2) / stride;
            (slot as usize).min(items.len())
        }
        _ => compute_drop_index_h(mouse_x, screen_w, items, source_idx, geom.icon_size, geom.icon_spacing, geom.h_padding),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_bounce_offset(i: usize, bounce_items: &[(usize, u32)], now: u32, hz: u32) -> i32 {
    bounce_items.iter()
        .find(|(idx, _)| *idx == i)
        .map(|(_, start)| {
            let elapsed_ms = now.wrapping_sub(*start) * 1000 / hz;
            bounce_offset(elapsed_ms)
        })
        .unwrap_or(0)
}
