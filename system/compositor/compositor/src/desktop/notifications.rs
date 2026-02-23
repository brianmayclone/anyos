//! Notification system — macOS-style toast banners with slide-in/out animation.
//!
//! The compositor manages notification banners as owned layers: no SHM, no
//! separate daemon.  Apps send `CMD_SHOW_NOTIFICATION` with title/message via
//! SHM; the compositor renders the banner, animates slide-in, and auto-dismisses
//! after a configurable timeout.

use alloc::string::String;
use alloc::vec::Vec;

use crate::compositor::{Compositor, Rect};

use super::drawing::{fill_rect, fill_rounded_rect, draw_rounded_rect_outline};
use super::theme;
use super::window::MENUBAR_HEIGHT;

// ── Layout Constants ──────────────────────────────────────────────────────

/// Banner width in pixels.
const NOTIF_W: u32 = 320;
/// Banner height in pixels.
const NOTIF_H: u32 = 72;
/// Corner radius.
const NOTIF_RADIUS: u32 = 12;
/// Right margin from screen edge.
const MARGIN_RIGHT: i32 = 12;
/// Top margin below menubar.
const MARGIN_TOP: i32 = MENUBAR_HEIGHT as i32 + 12;
/// Vertical gap between stacked banners.
const STACK_GAP: i32 = 8;
/// Maximum visible notifications (excess auto-dismiss oldest).
const MAX_VISIBLE: usize = 4;

/// Auto-dismiss timeout when none specified (ms).
const DEFAULT_TIMEOUT_MS: u32 = 5000;
/// Slide-in animation duration (ms).
const SLIDE_IN_MS: u32 = 250;
/// Slide-out (dismiss) animation duration (ms).
const SLIDE_OUT_MS: u32 = 200;

/// Font IDs (match compositor theme).
const FONT_ID: u16 = 0;
const FONT_ID_BOLD: u16 = 1;
const FONT_SIZE: u16 = 13;
const FONT_SIZE_SMALL: u16 = 11;

/// Animation ID encoding: `id * 2` = X-position, `id * 2 + 1` = Y-position.
fn anim_id_x(notif_id: u32) -> u32 { notif_id.wrapping_mul(2) }
fn anim_id_y(notif_id: u32) -> u32 { notif_id.wrapping_mul(2).wrapping_add(1) }

// ── Notification ──────────────────────────────────────────────────────────

/// A single notification banner.
pub(crate) struct Notification {
    /// Unique notification ID.
    pub id: u32,
    /// Compositor layer ID for this banner.
    pub layer_id: u32,
    /// TID of the app that sent this notification.
    pub sender_tid: u32,
    /// Title text (copied from SHM).
    pub title: String,
    /// Message text (copied from SHM).
    pub message: String,
    /// Optional 16x16 ARGB icon pixels.
    pub icon: Option<[u32; 256]>,
    /// Uptime tick when notification was created.
    pub created_tick: u32,
    /// Auto-dismiss timeout in ticks.
    pub timeout_ticks: u32,
    /// Target Y position in the notification stack.
    pub target_y: i32,
    /// Whether dismiss animation is currently running.
    pub dismissing: bool,
}

// ── NotificationManager ──────────────────────────────────────────────────

/// Manages all active notification banners.
pub(crate) struct NotificationManager {
    /// Active notifications (oldest first).
    pub notifications: Vec<Notification>,
    /// Next notification ID (monotonically increasing).
    next_id: u32,
    /// Animation set (separate from button anims to avoid ID collisions).
    anims: anyos_std::anim::AnimSet,
}

impl NotificationManager {
    /// Create a new empty notification manager.
    pub fn new() -> Self {
        NotificationManager {
            notifications: Vec::with_capacity(8),
            next_id: 1,
            anims: anyos_std::anim::AnimSet::new(),
        }
    }

    /// Show a new notification banner.
    ///
    /// Creates an owned compositor layer, renders the banner content, and
    /// starts the slide-in animation.  Returns the notification ID.
    pub fn show(
        &mut self,
        compositor: &mut Compositor,
        screen_width: u32,
        menubar_layer_id: u32,
        sender_tid: u32,
        title: &str,
        message: &str,
        icon: Option<[u32; 256]>,
        timeout_ms: u32,
    ) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        // Evict oldest if at capacity
        if self.notifications.len() >= MAX_VISIBLE {
            self.start_dismiss(0, screen_width);
        }

        // Compute target position in stack
        let slot = self.notifications.iter().filter(|n| !n.dismissing).count() as i32;
        let target_y = MARGIN_TOP + slot * (NOTIF_H as i32 + STACK_GAP);
        let target_x = screen_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;

        // Create layer off-screen (right edge) for slide-in
        let start_x = screen_width as i32 + 10;
        let layer_id = compositor.add_layer(start_x, target_y, NOTIF_W, NOTIF_H, false);
        if let Some(layer) = compositor.get_layer_mut(layer_id) {
            layer.has_shadow = true;
        }

        // Render the notification content into the layer
        if let Some(pixels) = compositor.layer_pixels(layer_id) {
            render_notification(pixels, NOTIF_W, NOTIF_H, title, message, icon.as_ref());
        }
        compositor.mark_layer_dirty(layer_id);

        // Raise notification above windows but below menubar
        compositor.raise_layer(layer_id);
        compositor.raise_layer(menubar_layer_id);

        // Start slide-in animation (X: off-screen → target_x)
        self.anims.start(
            anim_id_x(id),
            start_x * 1000,      // fixed-point
            target_x * 1000,
            SLIDE_IN_MS,
            anyos_std::anim::Easing::EaseOut,
        );

        // Convert timeout_ms to ticks
        let hz = anyos_std::sys::tick_hz().max(1);
        let timeout = if timeout_ms == 0 { DEFAULT_TIMEOUT_MS } else { timeout_ms };
        let timeout_ticks = (timeout as u64 * hz as u64 / 1000) as u32;

        let notif = Notification {
            id,
            layer_id,
            sender_tid,
            title: String::from(title),
            message: String::from(message),
            icon,
            created_tick: anyos_std::sys::uptime(),
            timeout_ticks,
            target_y,
            dismissing: false,
        };

        self.notifications.push(notif);

        // Damage the region
        compositor.add_damage(Rect::new(
            target_x - 12,
            target_y - 4,
            NOTIF_W + 24,
            NOTIF_H + 16,
        ));

        id
    }

    /// Start dismiss animation for a notification by index.
    fn start_dismiss(&mut self, idx: usize, screen_width: u32) {
        if idx >= self.notifications.len() { return; }
        if self.notifications[idx].dismissing { return; }

        self.notifications[idx].dismissing = true;
        let id = self.notifications[idx].id;
        let current_y = self.notifications[idx].target_y;

        // Slide-out to the right
        let target_x = screen_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;
        let off_screen_x = screen_width as i32 + 10;
        self.anims.start(
            anim_id_x(id),
            target_x * 1000,
            off_screen_x * 1000,
            SLIDE_OUT_MS,
            anyos_std::anim::Easing::EaseIn,
        );
    }

    /// Dismiss a notification by its ID.
    pub fn dismiss(&mut self, notif_id: u32, screen_width: u32) {
        if let Some(idx) = self.notifications.iter().position(|n| n.id == notif_id) {
            self.start_dismiss(idx, screen_width);
        }
    }

    /// Dismiss all notifications belonging to a specific TID.
    pub fn dismiss_for_tid(&mut self, tid: u32, screen_width: u32) {
        let ids: Vec<u32> = self.notifications.iter()
            .filter(|n| n.sender_tid == tid && !n.dismissing)
            .map(|n| n.id)
            .collect();
        for id in ids {
            self.dismiss(id, screen_width);
        }
    }

    /// Handle a click at (mx, my) in screen coordinates.
    ///
    /// Returns `Some(sender_tid)` if a notification was clicked (for
    /// `EVT_NOTIFICATION_CLICK`), `None` if no notification was hit.
    pub fn handle_click(
        &mut self,
        mx: i32,
        my: i32,
        screen_width: u32,
    ) -> Option<(u32, u32)> {
        let target_x = screen_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;

        for i in (0..self.notifications.len()).rev() {
            let n = &self.notifications[i];
            if n.dismissing { continue; }
            let nx = target_x;
            let ny = n.target_y;
            if mx >= nx && mx < nx + NOTIF_W as i32
                && my >= ny && my < ny + NOTIF_H as i32
            {
                let id = n.id;
                let sender_tid = n.sender_tid;
                self.start_dismiss(i, screen_width);
                return Some((id, sender_tid));
            }
        }
        None
    }

    /// Tick all notifications: update animations, auto-dismiss expired,
    /// remove completed dismiss animations.
    ///
    /// Returns `true` if any notification or animation is still active
    /// (caller should keep compositing).
    pub fn tick(
        &mut self,
        compositor: &mut Compositor,
        screen_width: u32,
    ) -> bool {
        if self.notifications.is_empty() {
            return false;
        }

        let now = anyos_std::sys::uptime();
        let target_x_base = screen_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;

        // Check auto-dismiss timeouts
        for i in 0..self.notifications.len() {
            if self.notifications[i].dismissing { continue; }
            let elapsed = now.wrapping_sub(self.notifications[i].created_tick);
            if elapsed >= self.notifications[i].timeout_ticks {
                self.start_dismiss(i, screen_width);
            }
        }

        // Update positions from animations
        for n in &self.notifications {
            let aid_x = anim_id_x(n.id);
            if let Some(val) = self.anims.value(aid_x, now) {
                let px = val / 1000;
                compositor.move_layer(n.layer_id, px, n.target_y);
            }
        }

        // Remove fully dismissed notifications (dismiss animation completed)
        let mut removed = false;
        let anims = &mut self.anims;
        self.notifications.retain(|n| {
            if n.dismissing {
                let aid_x = anim_id_x(n.id);
                if !anims.is_active(aid_x, now) {
                    // Animation done — remove layer
                    compositor.remove_layer(n.layer_id);
                    anims.remove(aid_x);
                    anims.remove(anim_id_y(n.id));
                    removed = true;
                    return false;
                }
            }
            true
        });

        // Reflow remaining notifications if any were removed
        if removed {
            self.reflow(compositor, screen_width);
        }

        // Clean up done animations
        self.anims.remove_done(now);

        !self.notifications.is_empty() || self.anims.has_active(now)
    }

    /// Reposition all non-dismissing notifications after a removal.
    fn reflow(&mut self, compositor: &mut Compositor, screen_width: u32) {
        let target_x = screen_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;
        let now = anyos_std::sys::uptime();
        let mut slot = 0i32;

        for n in &mut self.notifications {
            if n.dismissing { continue; }
            let new_y = MARGIN_TOP + slot * (NOTIF_H as i32 + STACK_GAP);
            if new_y != n.target_y {
                // Animate Y transition
                self.anims.start(
                    anim_id_y(n.id),
                    n.target_y * 1000,
                    new_y * 1000,
                    200,
                    anyos_std::anim::Easing::EaseOut,
                );
                n.target_y = new_y;
            }
            slot += 1;
        }

        // Update Y positions from reflow animations
        for n in &self.notifications {
            if n.dismissing { continue; }
            let aid_y = anim_id_y(n.id);
            if let Some(val) = self.anims.value(aid_y, now) {
                let py = val / 1000;
                let current_x = self.anims.value(anim_id_x(n.id), now)
                    .map(|v| v / 1000)
                    .unwrap_or(target_x);
                compositor.move_layer(n.layer_id, current_x, py);
            }
        }
    }

    /// Reposition all notifications after a screen resolution change.
    pub fn handle_resolution_change(
        &mut self,
        compositor: &mut Compositor,
        new_width: u32,
    ) {
        let target_x = new_width as i32 - NOTIF_W as i32 - MARGIN_RIGHT;
        for n in &mut self.notifications {
            if n.dismissing { continue; }
            compositor.move_layer(n.layer_id, target_x, n.target_y);
        }
    }

    /// Returns the number of active (non-dismissing) notifications.
    pub fn active_count(&self) -> usize {
        self.notifications.iter().filter(|n| !n.dismissing).count()
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// Render the notification banner content into a pixel buffer.
fn render_notification(
    pixels: &mut [u32],
    w: u32,
    h: u32,
    title: &str,
    message: &str,
    icon: Option<&[u32; 256]>,
) {
    // Clear to transparent
    for p in pixels.iter_mut() {
        *p = 0;
    }

    // Background: rounded rect with theme-aware colour
    fill_rounded_rect(pixels, w, h, 0, 0, w, h, NOTIF_RADIUS, theme::color_notif_bg());

    // 1px border outline
    draw_rounded_rect_outline(pixels, w, h, 0, 0, w, h, NOTIF_RADIUS, 0x30FFFFFF);

    // Content layout
    let pad_x: i32 = 12;
    let pad_y: i32 = 12;
    let text_x = if icon.is_some() { pad_x + 16 + 8 } else { pad_x };

    // Icon (16x16 ARGB, top-left corner)
    if let Some(icon_pixels) = icon {
        let ix = pad_x;
        let iy = pad_y;
        for dy in 0..16u32 {
            for dx in 0..16u32 {
                let src = icon_pixels[(dy * 16 + dx) as usize];
                if (src >> 24) == 0 { continue; } // skip transparent
                let dst_x = ix + dx as i32;
                let dst_y = iy + dy as i32;
                if dst_x >= 0 && dst_y >= 0
                    && (dst_x as u32) < w && (dst_y as u32) < h
                {
                    let idx = (dst_y as u32 * w + dst_x as u32) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = crate::compositor::alpha_blend(src, pixels[idx]);
                    }
                }
            }
        }
    }

    // Title (bold, 13px)
    let title_y = pad_y;
    let display_title = if title.len() > 40 { &title[..40] } else { title };
    anyos_std::ui::window::font_render_buf(
        FONT_ID_BOLD, FONT_SIZE, pixels, w, h,
        text_x, title_y, theme::color_notif_title(), display_title,
    );

    // "now" timestamp (right-aligned)
    let (tw, _) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE_SMALL, "now");
    let time_x = w as i32 - pad_x - tw as i32;
    anyos_std::ui::window::font_render_buf(
        FONT_ID, FONT_SIZE_SMALL, pixels, w, h,
        time_x, title_y + 2, theme::color_notif_time(), "now",
    );

    // Message (regular, 11px, up to 2 lines)
    let msg_y = title_y + 20;
    let display_msg = if message.len() > 80 { &message[..80] } else { message };

    // Simple line-break: find a split point near the midpoint if message is long
    let max_line_chars = 45;
    if display_msg.len() > max_line_chars {
        // Split at last space before max_line_chars
        let split = display_msg[..max_line_chars].rfind(' ')
            .unwrap_or(max_line_chars);
        let line1 = &display_msg[..split];
        let line2 = display_msg[split..].trim_start();
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE_SMALL, pixels, w, h,
            text_x, msg_y, theme::color_notif_message(), line1,
        );
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE_SMALL, pixels, w, h,
            text_x, msg_y + 16, theme::color_notif_message(), line2,
        );
    } else {
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE_SMALL, pixels, w, h,
            text_x, msg_y, theme::color_notif_message(), display_msg,
        );
    }
}
