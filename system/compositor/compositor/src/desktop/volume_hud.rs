//! Volume HUD — macOS-style centered-bottom overlay for volume changes.
//!
//! Displays a compact rounded rect with a speaker icon, volume bar, and
//! percentage text.  Appears on multimedia key presses, auto-dismisses
//! after 2 seconds.  Reuses a single compositor layer (no per-frame alloc).

use crate::compositor::{Compositor, Rect};

use super::drawing::{fill_rect, fill_rounded_rect, draw_rounded_rect_outline};
use super::theme;
use super::window::MENUBAR_HEIGHT;

// ── Layout Constants ──────────────────────────────────────────────────────

const HUD_W: u32 = 200;
const HUD_H: u32 = 48;
const RADIUS: u32 = 16;
const MARGIN_BOTTOM: i32 = 40;

/// Auto-dismiss timeout in ticks (converted from ms at runtime).
const DISMISS_MS: u32 = 2000;
/// Slide-in animation duration (ms).
const SLIDE_IN_MS: u32 = 150;
/// Slide-out (dismiss) animation duration (ms).
const SLIDE_OUT_MS: u32 = 150;

/// Volume bar dimensions.
const BAR_X: i32 = 40;
const BAR_Y: i32 = 21;
const BAR_W: u32 = 108;
const BAR_H: u32 = 6;

/// Font settings.
const FONT_ID: u16 = 0;
const FONT_SIZE: u16 = 11;

/// Animation ID constants (unique, won't collide with notification/button anims).
const ANIM_ID_Y: u32 = 0xFFFE_0000;

// ── VolumeHud ─────────────────────────────────────────────────────────────

/// Volume HUD overlay state.
pub(crate) struct VolumeHud {
    /// Compositor layer ID (None = not yet created).
    layer_id: Option<u32>,
    /// Whether the HUD is currently visible.
    visible: bool,
    /// Uptime tick when `show()` was last called.
    last_show_tick: u32,
    /// Dismiss timeout in ticks (computed from DISMISS_MS).
    dismiss_ticks: u32,
    /// Volume before mute (for toggle restore).
    pub saved_volume: u8,
    /// Animation set for slide-in/out.
    anims: anyos_std::anim::AnimSet,
    /// Whether the dismiss animation is running.
    dismissing: bool,
}

impl VolumeHud {
    /// Create a new (inactive) volume HUD.
    pub fn new() -> Self {
        let hz = anyos_std::sys::tick_hz().max(1);
        let dismiss_ticks = (DISMISS_MS as u64 * hz as u64 / 1000) as u32;
        VolumeHud {
            layer_id: None,
            visible: false,
            last_show_tick: 0,
            dismiss_ticks,
            saved_volume: 50,
            anims: anyos_std::anim::AnimSet::new(),
            dismissing: false,
        }
    }

    /// Show (or update) the volume HUD with the given volume level.
    pub fn show(
        &mut self,
        compositor: &mut Compositor,
        screen_width: u32,
        screen_height: u32,
        volume: u8,
        muted: bool,
        menubar_layer_id: u32,
    ) {
        let cx = (screen_width as i32 - HUD_W as i32) / 2;
        let target_y = screen_height as i32 - HUD_H as i32 - MARGIN_BOTTOM;
        let off_screen_y = screen_height as i32 + 10;

        let layer_id = match self.layer_id {
            Some(id) => {
                // Reuse existing layer — reposition and make visible
                compositor.set_layer_visible(id, true);
                id
            }
            None => {
                // First show: create layer off-screen (will slide in)
                let id = compositor.add_layer(cx, off_screen_y, HUD_W, HUD_H, false);
                if let Some(layer) = compositor.get_layer_mut(id) {
                    layer.has_shadow = true;
                }
                self.layer_id = Some(id);
                id
            }
        };

        // Render content
        if let Some(pixels) = compositor.layer_pixels(layer_id) {
            render_hud(pixels, HUD_W, HUD_H, volume, muted);
        }
        compositor.mark_layer_dirty(layer_id);

        // Position (X is always centered, Y animates)
        compositor.move_layer(layer_id, cx, off_screen_y);

        // Raise above windows, menubar above HUD
        compositor.raise_layer(layer_id);
        compositor.raise_layer(menubar_layer_id);

        // Slide-in animation (Y: off-screen → target_y)
        if !self.visible {
            self.anims.start(
                ANIM_ID_Y,
                off_screen_y * 1000,
                target_y * 1000,
                SLIDE_IN_MS,
                anyos_std::anim::Easing::EaseOut,
            );
        } else {
            // Already visible: snap to position, reset timer
            compositor.move_layer(layer_id, cx, target_y);
            self.anims.remove(ANIM_ID_Y);
        }

        self.visible = true;
        self.dismissing = false;
        self.last_show_tick = anyos_std::sys::uptime();

        // Damage region
        compositor.add_damage(Rect::new(
            cx - 12,
            target_y - 12,
            HUD_W + 24,
            HUD_H + 24 + MARGIN_BOTTOM as u32,
        ));
    }

    /// Tick the HUD: update animations, auto-dismiss.
    /// Returns `true` if the HUD is still active (caller should keep compositing).
    pub fn tick(&mut self, compositor: &mut Compositor) -> bool {
        if !self.visible {
            return false;
        }

        let layer_id = match self.layer_id {
            Some(id) => id,
            None => return false,
        };

        let now = anyos_std::sys::uptime();

        // Update slide animation
        if let Some(val) = self.anims.value(ANIM_ID_Y, now) {
            let py = val / 1000;
            // Recover cx from layer position (keep X stable)
            if let Some(layer) = compositor.get_layer(layer_id) {
                let cx = layer.x;
                compositor.move_layer(layer_id, cx, py);
            }
        }

        // Check auto-dismiss timeout
        if !self.dismissing {
            let elapsed = now.wrapping_sub(self.last_show_tick);
            if elapsed >= self.dismiss_ticks {
                self.start_dismiss(compositor);
            }
        }

        // Check if dismiss animation is complete
        if self.dismissing && !self.anims.is_active(ANIM_ID_Y, now) {
            // Animation done — hide layer
            compositor.set_layer_visible(layer_id, false);
            self.visible = false;
            self.dismissing = false;
            self.anims.remove(ANIM_ID_Y);
            return false;
        }

        self.anims.remove_done(now);
        true
    }

    /// Start the dismiss (slide-out) animation.
    fn start_dismiss(&mut self, compositor: &mut Compositor) {
        if self.dismissing { return; }
        self.dismissing = true;

        let layer_id = match self.layer_id {
            Some(id) => id,
            None => return,
        };

        // Get current Y position from layer
        let (current_y, screen_h) = if let Some(layer) = compositor.get_layer(layer_id) {
            (layer.y, 0i32) // we need screen height, estimate from position
        } else {
            return;
        };

        // Slide down off-screen
        let off_screen_y = current_y + HUD_H as i32 + MARGIN_BOTTOM + 10;
        self.anims.start(
            ANIM_ID_Y,
            current_y * 1000,
            off_screen_y * 1000,
            SLIDE_OUT_MS,
            anyos_std::anim::Easing::EaseIn,
        );
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// Render the volume HUD content into a pixel buffer.
fn render_hud(pixels: &mut [u32], w: u32, h: u32, volume: u8, muted: bool) {
    // Clear to transparent
    for p in pixels.iter_mut() {
        *p = 0;
    }

    // Background: rounded rect
    fill_rounded_rect(pixels, w, h, 0, 0, w, h, RADIUS, theme::color_hud_bg());

    // 1px border outline
    draw_rounded_rect_outline(pixels, w, h, 0, 0, w, h, RADIUS, 0x30FFFFFF);

    // Speaker icon (simplified 10x10, left side)
    let icon_x: i32 = 14;
    let icon_y: i32 = 14;
    let icon_color = if muted { 0xFF666666 } else { theme::color_hud_text() };
    draw_speaker_mini(pixels, w, h, icon_x, icon_y, icon_color, volume, muted);

    // Volume bar background (full width)
    fill_rect(pixels, w, h, BAR_X, BAR_Y, BAR_W, BAR_H, theme::color_hud_bar_bg());

    // Volume bar filled portion
    if !muted && volume > 0 {
        let filled_w = (BAR_W as u32 * volume as u32 / 100).max(1);
        fill_rect(pixels, w, h, BAR_X, BAR_Y, filled_w, BAR_H, theme::color_hud_bar());
    }

    // Percentage text (right side)
    let mut buf = [0u8; 5];
    let text = fmt_percent(&mut buf, volume, muted);
    let text_x = BAR_X + BAR_W as i32 + 8;
    let text_y = (h as i32 - 11) / 2;
    anyos_std::ui::window::font_render_buf(
        FONT_ID, FONT_SIZE, pixels, w, h,
        text_x, text_y, theme::color_hud_text(), text,
    );
}

/// Draw a minimal speaker icon at (x, y), approximately 10x10 pixels.
fn draw_speaker_mini(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    volume: u8,
    muted: bool,
) {
    let s = stride as i32;
    let bh = buf_h as i32;

    // Helper to set a pixel with bounds checking
    let mut px = |px: i32, py: i32, c: u32| {
        if px >= 0 && py >= 0 && px < s && py < bh {
            let idx = (py * s + px) as usize;
            if idx < pixels.len() {
                pixels[idx] = c;
            }
        }
    };

    // Speaker body (rect 0..3 x 3..7)
    for dy in 3..7 {
        for dx in 0..3 {
            px(x + dx, y + dy, color);
        }
    }
    // Speaker cone (triangle 3..6)
    for dy in 1..9 {
        let half = if dy < 5 { 5 - dy } else { dy - 4 };
        let xend = (3 + half).min(7);
        for dx in 3..xend {
            px(x + dx, y + dy, color);
        }
    }

    if muted {
        // X mark
        for i in 0..4 {
            px(x + 8 + i, y + 2 + i, 0xFFFF3B30);
            px(x + 8 + i, y + 7 - i, 0xFFFF3B30);
        }
    } else {
        // Sound waves based on volume
        if volume > 0 {
            for dy in 3..7 {
                px(x + 8, y + dy, color);
            }
        }
        if volume > 50 {
            for dy in 2..8 {
                px(x + 10, y + dy, color);
            }
        }
    }
}

/// Format volume as "XX%" or "Mute" string.
fn fmt_percent<'a>(buf: &'a mut [u8; 5], vol: u8, muted: bool) -> &'a str {
    if muted {
        return "Mute";
    }
    let mut pos = 0;
    if vol >= 100 {
        buf[pos] = b'1'; pos += 1;
        buf[pos] = b'0'; pos += 1;
        buf[pos] = b'0'; pos += 1;
    } else if vol >= 10 {
        buf[pos] = b'0' + vol / 10; pos += 1;
        buf[pos] = b'0' + vol % 10; pos += 1;
    } else {
        buf[pos] = b'0' + vol; pos += 1;
    }
    buf[pos] = b'%'; pos += 1;
    core::str::from_utf8(&buf[..pos]).unwrap_or("?")
}
