//! Dock rendering, hit testing, and animation helpers.

use anyos_std::anim::AnimSet;
use libcompositor_client::WindowHandle;

use crate::framebuffer::Framebuffer;
use crate::theme::*;
use crate::types::DockItem;

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

pub struct RenderState<'a> {
    pub hover_idx: Option<usize>,
    pub anims: &'a AnimSet,
    pub bounce_items: &'a [(usize, u32)],
    pub now: u32,
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
    uisys_client::draw_shadow_rounded_rect_buf(
        &mut fb.pixels, fb.width, fb.height,
        dock_x, dock_y, total_width, DOCK_HEIGHT, DOCK_BORDER_RADIUS,
        0, 4, 12, 40,
    );

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

    for (i, item) in items.iter().enumerate() {
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

        ix += (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
    }

    // Tooltip for hovered item
    if let Some(idx) = rs.hover_idx {
        if let Some(item) = items.get(idx) {
            let name = &item.name;
            let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, name);
            let pill_w = tw + TOOLTIP_PAD * 2;
            let pill_h = th + 8;

            let item_stride = (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
            let icon_center_x = dock_x + DOCK_H_PADDING as i32
                + idx as i32 * item_stride
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

/// Copy local framebuffer pixels into the SHM window surface.
pub fn blit_to_surface(fb: &Framebuffer, win: &WindowHandle) {
    let count = (fb.width * fb.height) as usize;
    let surface = unsafe { core::slice::from_raw_parts_mut(win.surface(), count) };
    let copy_len = count.min(fb.pixels.len()).min(surface.len());
    surface[..copy_len].copy_from_slice(&fb.pixels[..copy_len]);
}
