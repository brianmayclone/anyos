//! Window management — WindowInfo, HitTest, create/destroy/focus/render, chrome.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::println;
use anyos_std::process;

use crate::compositor::Rect;

use super::drawing::*;
use super::theme::*;
use super::Desktop;

// ── Window Flags ───────────────────────────────────────────────────────────

pub const WIN_FLAG_BORDERLESS: u32 = 0x01;
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;
pub const WIN_FLAG_NO_CLOSE: u32 = 0x08;
pub const WIN_FLAG_NO_MINIMIZE: u32 = 0x10;
pub const WIN_FLAG_NO_MAXIMIZE: u32 = 0x20;
pub const WIN_FLAG_SHADOW: u32 = 0x40;
pub const WIN_FLAG_SCALE_CONTENT: u32 = 0x80;
pub const WIN_FLAG_NO_MOVE: u32 = 0x100;
/// DPI-aware: the app renders at physical resolution (libanyui windows).
/// The compositor will not upscale the window's content.
pub const WIN_FLAG_DPI_AWARE: u32 = 0x200;
/// App supports fullscreen mode (set via CMD_SET_FULLSCREEN_CAP).
pub const WIN_FLAG_FULLSCREEN_CAPABLE: u32 = 0x400;
/// Borderless window input follows the surface alpha channel.
/// Fully transparent pixels are treated as click-through.
pub const WIN_FLAG_ALPHA_HIT_TEST: u32 = 0x800;

// ── Dimensions ─────────────────────────────────────────────────────────────

/// Menubar height in physical pixels (DPI-scaled).
#[inline(always)]
pub fn menubar_height() -> u32 {
    crate::desktop::theme::scale(24)
}

/// Title bar height in physical pixels (DPI-scaled).
#[inline(always)]
pub fn title_bar_height() -> u32 {
    crate::desktop::theme::scale(28)
}

/// Minimum padding (px) between traffic-light buttons / window edge and the title text (DPI-scaled).
#[inline(always)]
fn title_padding() -> i32 {
    crate::desktop::theme::scale_i32(8)
}

/// Right-edge of the last traffic-light button (DPI-scaled).
#[inline(always)]
fn title_buttons_right() -> i32 {
    crate::desktop::theme::scale_i32(8) + 2 * title_btn_spacing() as i32 + title_btn_size() as i32
}

/// X position of shortcut button left edge (after traffic light buttons, DPI-scaled).
#[inline(always)]
fn shortcut_btn_x() -> i32 {
    title_buttons_right() + crate::desktop::theme::scale_i32(6)
}

/// Width of the shortcut button in physical pixels (DPI-scaled).
#[inline(always)]
fn shortcut_btn_w() -> u32 {
    crate::desktop::theme::scale(32)
}

/// Height of the shortcut button in physical pixels (DPI-scaled).
#[inline(always)]
fn shortcut_btn_h() -> u32 {
    crate::desktop::theme::scale(16)
}

/// Y position of the shortcut button (vertically centered in title bar, DPI-scaled).
#[inline(always)]
fn shortcut_btn_y() -> i32 {
    (title_bar_height() as i32 - shortcut_btn_h() as i32) / 2
}

/// Right edge of shortcut button (used as left_bound for title text).
#[inline(always)]
fn shortcut_btn_right() -> i32 {
    shortcut_btn_x() + shortcut_btn_w() as i32
}

// ── Monitor-Send buttons (multi-monitor) ─────────────────────────────
//
// On a multi-monitor setup the title bar exposes one small button per
// non-current output. Clicking such a button moves the window to that
// output, preserving the relative position inside the source output's
// rectangle when it fits, otherwise clamping into the target. The
// buttons live on the right edge, stacked right-to-left in output id
// order.

/// Width of one monitor-send button (DPI-scaled).
#[inline(always)]
pub(crate) fn monitor_btn_w() -> u32 {
    crate::desktop::theme::scale(28)
}

/// Height of one monitor-send button (DPI-scaled).
#[inline(always)]
pub(crate) fn monitor_btn_h() -> u32 {
    crate::desktop::theme::scale(16)
}

/// Vertical position of the monitor button strip inside the title bar.
#[inline(always)]
pub(crate) fn monitor_btn_y() -> i32 {
    (title_bar_height() as i32 - monitor_btn_h() as i32) / 2
}

/// Horizontal gap between adjacent monitor buttons.
#[inline(always)]
pub(crate) fn monitor_btn_gap() -> i32 {
    crate::desktop::theme::scale_i32(4)
}

/// Right-edge inset where the right-most monitor button ends.
#[inline(always)]
pub(crate) fn monitor_btn_right_inset() -> i32 {
    crate::desktop::theme::scale_i32(8)
}

/// Compute the X position of the monitor button at slot `slot` (0 =
/// rightmost), given the window's full width.
#[inline(always)]
pub(crate) fn monitor_btn_x_at(window_full_w: u32, slot: u32) -> i32 {
    let w = monitor_btn_w() as i32;
    let gap = monitor_btn_gap();
    window_full_w as i32 - monitor_btn_right_inset() - (slot as i32 + 1) * w - (slot as i32) * gap
}

/// Render one monitor-send button at the given X. The button shows a
/// stylised display icon plus the target output id as a single digit.
fn render_monitor_btn(
    pixels: &mut [u32],
    stride: u32,
    full_h: u32,
    btn_x: i32,
    target_id: u8,
    focused: bool,
) {
    let y = monitor_btn_y();
    let w = monitor_btn_w();
    let h = monitor_btn_h();
    let bg = if focused {
        if super::theme::is_light() {
            0x30000000
        } else {
            0x30FFFFFF
        }
    } else {
        if super::theme::is_light() {
            0x18000000
        } else {
            0x18FFFFFF
        }
    };
    let r = crate::desktop::theme::scale(4);
    fill_rounded_rect(pixels, stride, full_h, btn_x, y, w, h, r, bg);

    // Tiny monitor glyph: 8x6 rounded rect on the left, base bar
    // beneath. Drawn directly because anyui icon assets aren't
    // available from inside the compositor's title-bar fast path.
    let glyph_w = crate::desktop::theme::scale(10) as i32;
    let glyph_h = crate::desktop::theme::scale(6) as i32;
    let glyph_x = btn_x + crate::desktop::theme::scale_i32(4);
    let glyph_y = y + (h as i32 - glyph_h - crate::desktop::theme::scale_i32(2)) / 2;
    let stroke = color_titlebar_text();
    // Top + bottom of monitor frame.
    fill_rect(
        pixels,
        stride,
        full_h,
        glyph_x,
        glyph_y,
        glyph_w as u32,
        1,
        stroke,
    );
    fill_rect(
        pixels,
        stride,
        full_h,
        glyph_x,
        glyph_y + glyph_h - 1,
        glyph_w as u32,
        1,
        stroke,
    );
    // Left + right side.
    fill_rect(
        pixels,
        stride,
        full_h,
        glyph_x,
        glyph_y,
        1,
        glyph_h as u32,
        stroke,
    );
    fill_rect(
        pixels,
        stride,
        full_h,
        glyph_x + glyph_w - 1,
        glyph_y,
        1,
        glyph_h as u32,
        stroke,
    );
    // Stand pixel.
    fill_rect(
        pixels,
        stride,
        full_h,
        glyph_x + glyph_w / 2 - 1,
        glyph_y + glyph_h,
        2,
        1,
        stroke,
    );

    // Digit (target output id 0..9).
    let mut buf = [0u8; 2];
    let label = if target_id < 10 {
        buf[0] = b'0' + target_id;
        core::str::from_utf8(&buf[..1]).unwrap_or("")
    } else {
        // 10-15 fall back to a generic glyph — we still have unique
        // ids so callers can disambiguate by hit-test slot.
        buf[0] = b'+';
        core::str::from_utf8(&buf[..1]).unwrap_or("")
    };
    let fs = crate::desktop::theme::scale_font(10);
    let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, label);
    let tx = btn_x + (w as i32) - tw as i32 - crate::desktop::theme::scale_i32(4);
    let ty = y + (h as i32 - th as i32) / 2;
    anyos_std::ui::window::font_render_buf(
        FONT_ID, fs, pixels, stride, full_h, tx, ty, stroke, label,
    );
}

/// Render the shortcut button ("F1".."F12") into a pixel buffer at the title bar.
fn render_shortcut_btn(pixels: &mut [u32], stride: u32, full_h: u32, slot: u8, focused: bool) {
    if slot == 0 {
        return;
    }
    let x = shortcut_btn_x();
    let y = shortcut_btn_y();
    let w = shortcut_btn_w();
    let h = shortcut_btn_h();
    // Background: subtle rounded rect
    let bg = if focused {
        if super::theme::is_light() {
            0x30000000
        } else {
            0x30FFFFFF
        }
    } else {
        if super::theme::is_light() {
            0x18000000
        } else {
            0x18FFFFFF
        }
    };
    let r = crate::desktop::theme::scale(4);
    fill_rounded_rect(pixels, stride, full_h, x, y, w, h, r, bg);
    // Label text
    let mut label_buf = [0u8; 4];
    let label = match slot {
        1..=9 => {
            label_buf[0] = b'F';
            label_buf[1] = b'0' + slot;
            core::str::from_utf8(&label_buf[..2]).unwrap_or("")
        }
        10 => {
            label_buf[0] = b'F';
            label_buf[1] = b'1';
            label_buf[2] = b'0';
            core::str::from_utf8(&label_buf[..3]).unwrap_or("")
        }
        11 => {
            label_buf[0] = b'F';
            label_buf[1] = b'1';
            label_buf[2] = b'1';
            core::str::from_utf8(&label_buf[..3]).unwrap_or("")
        }
        12 => {
            label_buf[0] = b'F';
            label_buf[1] = b'1';
            label_buf[2] = b'2';
            core::str::from_utf8(&label_buf[..3]).unwrap_or("")
        }
        _ => return,
    };
    let fs = crate::desktop::theme::scale_font(10);
    let text_color = color_titlebar_text();
    let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, label);
    let tx = x + (w as i32 - tw as i32) / 2;
    let ty = y + (h as i32 - th as i32) / 2;
    anyos_std::ui::window::font_render_buf(
        FONT_ID, fs, pixels, stride, full_h, tx, ty, text_color, label,
    );
}

/// Truncate a title string so that it fits within `max_width` pixels.
/// If the full title fits, returns it unchanged.
/// Otherwise appends "..." and shortens until it fits.
/// Returns the displayable slice length (byte offset, always on a char boundary).
fn title_display_len(title: &str, max_width: u32) -> usize {
    let fs = scaled_font_size();
    let (tw, _) = anyos_std::ui::window::font_measure(FONT_ID, fs, title);
    if tw <= max_width {
        return title.len();
    }
    // Measure ellipsis width once
    let (ew, _) = anyos_std::ui::window::font_measure(FONT_ID, fs, "...");
    if max_width <= ew {
        return 0;
    }
    let target = max_width - ew;
    // Walk backwards through chars to find the longest prefix that fits
    let mut best = 0usize;
    for (i, _) in title.char_indices() {
        let (w, _) = anyos_std::ui::window::font_measure(FONT_ID, fs, &title[..i]);
        if w > target {
            break;
        }
        best = i;
    }
    best
}

// ── Event Types ────────────────────────────────────────────────────────────

pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;
pub const EVENT_MOUSE_SCROLL: u32 = 7;
pub const EVENT_WINDOW_CLOSE: u32 = 8;
pub const EVENT_MENU_ITEM: u32 = 9;
pub const EVENT_STATUS_ICON_CLICK: u32 = 10;
pub const EVENT_FOCUS_LOST: u32 = 11;
pub const EVENT_FULLSCREEN_ENTER: u32 = 12;
pub const EVENT_FULLSCREEN_EXIT: u32 = 13;
// Cross-window drag-and-drop (compositor-internal codes — translated to
// proto::EVT_DRAG_* in drain_ipc_events).
pub const EVENT_DRAG_ENTER: u32 = 14;
pub const EVENT_DRAG_OVER: u32 = 15;
pub const EVENT_DRAG_LEAVE: u32 = 16;
pub const EVENT_DROP: u32 = 17;
pub const EVENT_DRAG_FEEDBACK: u32 = 18;
pub const EVENT_DRAG_END: u32 = 19;
pub const EVENT_WINDOW_MOVED: u32 = 20;

// ── Hit Test ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum HitTest {
    None,
    TitleBar,
    CloseButton,
    MinButton,
    MaxButton,
    ShortcutButton,
    /// Click on the "send window to monitor N" button (multi-monitor).
    /// Payload is the target output id; the compositor calls
    /// `move_window_to_output(window, output_id)` on click.
    MonitorButton(u8),
    Content,
    ResizeTop,
    ResizeBottom,
    ResizeLeft,
    ResizeRight,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
    ResizeBottomRight,
}

pub(crate) fn is_resize_edge(ht: HitTest) -> bool {
    matches!(
        ht,
        HitTest::ResizeTop
            | HitTest::ResizeBottom
            | HitTest::ResizeLeft
            | HitTest::ResizeRight
            | HitTest::ResizeTopLeft
            | HitTest::ResizeTopRight
            | HitTest::ResizeBottomLeft
            | HitTest::ResizeBottomRight
    )
}

// ── Interaction State ──────────────────────────────────────────────────────

pub(crate) struct DragState {
    pub window_id: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    /// True once the mouse has actually moved during the drag.
    /// Edge-snapping only activates when this is true (prevents snap on click-release).
    pub moved: bool,
}

pub(crate) struct ResizeState {
    pub window_id: u32,
    pub start_mouse_x: i32,
    pub start_mouse_y: i32,
    pub start_x: i32,
    pub start_y: i32,
    pub start_w: u32,
    pub start_h: u32,
    pub edge: HitTest,
}

// ── Window Info ────────────────────────────────────────────────────────────

pub struct WindowInfo {
    pub id: u32,
    pub layer_id: u32,
    pub title: String,
    /// Position of the full window (including title bar).
    pub x: i32,
    pub y: i32,
    /// Content area dimensions (excluding title bar).
    pub content_width: u32,
    pub content_height: u32,
    pub flags: u32,
    pub owner_tid: u32,
    /// Event queue for this window.
    pub events: VecDeque<[u32; 5]>,
    /// Whether this window has the focused title bar style.
    pub focused: bool,
    /// Saved bounds for maximize toggle.
    pub saved_bounds: Option<(i32, i32, u32, u32)>,
    /// Whether the window is maximized.
    pub maximized: bool,
    /// SHM region ID (0 = local/compositor-owned window).
    pub shm_id: u32,
    /// SHM pixel pointer (null = local window).
    pub shm_ptr: *mut u32,
    /// SHM buffer dimensions (may lag behind content_width/height during resize).
    pub shm_width: u32,
    pub shm_height: u32,
    /// Set true on CMD_PRESENT, cleared after compose emits EVT_FRAME_ACK.
    pub needs_frame_ack: bool,
    /// If this window is a modal child, the compositor window ID of its owner.
    /// 0 = no owner (normal window). Set via CMD_SET_MODAL_OWNER.
    pub modal_owner: u32,
    /// Whether this window is currently in fullscreen mode.
    pub fullscreen: bool,
    /// Whether this window supports fullscreen (registered via CMD_SET_FULLSCREEN_CAP).
    pub fullscreen_capable: bool,
    /// Saved window bounds before entering fullscreen: (x, y, width, height).
    pub saved_bounds_fs: Option<(i32, i32, u32, u32)>,
    /// Original flags before fullscreen (to restore borderless state etc.).
    pub saved_flags_fs: u32,
    /// Assigned F-key shortcut slot (0 = none, 1–12 = F1–F12).
    pub shortcut_slot: u8,
}

impl WindowInfo {
    pub fn is_borderless(&self) -> bool {
        self.flags & WIN_FLAG_BORDERLESS != 0
    }

    pub fn is_resizable(&self) -> bool {
        self.flags & WIN_FLAG_NOT_RESIZABLE == 0
    }

    pub fn is_always_on_top(&self) -> bool {
        self.flags & WIN_FLAG_ALWAYS_ON_TOP != 0
    }

    fn uses_alpha_hit_test(&self) -> bool {
        self.flags & WIN_FLAG_ALPHA_HIT_TEST != 0
    }

    fn alpha_accepts_hit(&self, wx: i32, wy: i32) -> bool {
        if !self.uses_alpha_hit_test() {
            return true;
        }
        if self.shm_ptr.is_null() || wx < 0 || wy < 0 {
            return false;
        }

        let x = wx as u32;
        let y = wy as u32;
        if x >= self.shm_width || y >= self.shm_height {
            return false;
        }

        let idx = y as usize * self.shm_width as usize + x as usize;
        let pixel = unsafe { *self.shm_ptr.add(idx) };
        (pixel >> 24) != 0
    }

    /// Full window width (same as content for borderless).
    pub fn full_width(&self) -> u32 {
        self.content_width
    }

    /// Full window height (content + title bar for decorated windows).
    pub fn full_height(&self) -> u32 {
        if self.is_borderless() {
            self.content_height
        } else {
            self.content_height + title_bar_height()
        }
    }

    /// Hit test a point (in screen coordinates) against the window.
    pub fn hit_test(&self, px: i32, py: i32) -> HitTest {
        let wx = px - self.x;
        let wy = py - self.y;
        let fw = self.full_width() as i32;
        let fh = self.full_height() as i32;

        // Resize zones extend outside the window bounds for easier targeting (DPI-scaled).
        // Check the outer margin first, before the normal bounds check.
        let outer = crate::desktop::theme::scale_i32(2);
        if self.is_resizable() && !self.maximized && !self.is_borderless() {
            if wx >= -outer && wy >= -outer && wx < fw + outer && wy < fh + outer {
                let inner = crate::desktop::theme::scale_i32(6); // inner border from edge
                let top = wy < inner;
                let bottom = wy >= fh - inner;
                let left = wx < inner;
                let right = wx >= fw - inner;

                // Corner zones (inner+outer = 8px grab area on each axis)
                if top && left {
                    return HitTest::ResizeTopLeft;
                }
                if top && right {
                    return HitTest::ResizeTopRight;
                }
                if bottom && left {
                    return HitTest::ResizeBottomLeft;
                }
                if bottom && right {
                    return HitTest::ResizeBottomRight;
                }

                // Edge zones (only if cursor is outside window or within inner border)
                if wx < 0 || wy < 0 || wx >= fw || wy >= fh {
                    // Outside bounds — must be a resize edge
                    if top || wy < 0 {
                        return HitTest::ResizeTop;
                    }
                    if bottom || wy >= fh {
                        return HitTest::ResizeBottom;
                    }
                    if left || wx < 0 {
                        return HitTest::ResizeLeft;
                    }
                    if right || wx >= fw {
                        return HitTest::ResizeRight;
                    }
                }

                // Inside bounds — check inner border
                if top {
                    return HitTest::ResizeTop;
                }
                if bottom {
                    return HitTest::ResizeBottom;
                }
                if left {
                    return HitTest::ResizeLeft;
                }
                if right {
                    return HitTest::ResizeRight;
                }
            }
        }

        if wx < 0 || wy < 0 || wx >= fw || wy >= fh {
            return HitTest::None;
        }

        if self.is_borderless() {
            return if self.alpha_accepts_hit(wx, wy) {
                HitTest::Content
            } else {
                HitTest::None
            };
        }

        // Title bar
        let tb_h = title_bar_height() as i32;
        if wy < tb_h {
            let btn_y_pos = title_btn_y() as i32;
            let btn_sz = title_btn_size() as i32;
            let btn_r = btn_sz / 2;
            let btn_sp = title_btn_spacing() as i32;
            if wy >= btn_y_pos && wy < btn_y_pos + btn_sz {
                // Close button
                let cx = crate::desktop::theme::scale_i32(8) + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y_pos - btn_r).abs() <= btn_r {
                    return HitTest::CloseButton;
                }
                // Minimize button
                let cx = crate::desktop::theme::scale_i32(8) + btn_sp + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y_pos - btn_r).abs() <= btn_r {
                    return HitTest::MinButton;
                }
                // Maximize button
                let cx = crate::desktop::theme::scale_i32(8) + 2 * btn_sp + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y_pos - btn_r).abs() <= btn_r {
                    return HitTest::MaxButton;
                }
            }
            // Shortcut button (only if this window has a slot assigned)
            if self.shortcut_slot > 0 {
                let sbx = shortcut_btn_x();
                let sby = shortcut_btn_y();
                let sbw = shortcut_btn_w() as i32;
                let sbh = shortcut_btn_h() as i32;
                if wx >= sbx && wx < sbx + sbw && wy >= sby && wy < sby + sbh {
                    return HitTest::ShortcutButton;
                }
            }
            return HitTest::TitleBar;
        }

        HitTest::Content
    }
}

// ── Resize Computation ─────────────────────────────────────────────────────

pub(crate) fn compute_resize(
    edge: HitTest,
    start_x: i32,
    start_y: i32,
    start_w: u32,
    start_h: u32,
    dx: i32,
    dy: i32,
) -> (i32, i32, u32, u32) {
    let min_w: u32 = 100;
    let min_h: u32 = 60;
    let mut x = start_x;
    let mut y = start_y;
    let mut w = start_w;
    let mut h = start_h;

    match edge {
        HitTest::ResizeRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
        }
        HitTest::ResizeBottom => {
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
        }
        HitTest::ResizeTop => {
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        HitTest::ResizeBottomRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeBottomLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeTopRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        HitTest::ResizeTopLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        _ => {}
    }

    (x, y, w, h)
}

// ── Desktop Window Management ──────────────────────────────────────────────

impl Desktop {
    /// Create a new window.
    pub fn create_window(
        &mut self,
        title: &str,
        x: i32,
        y: i32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        owner_tid: u32,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let borderless = flags & WIN_FLAG_BORDERLESS != 0;
        let full_h = if borderless {
            content_h
        } else {
            content_h + title_bar_height()
        };

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        // Client windows are never assumed opaque — borderless windows (dock, overlays)
        // may have transparent pixels, and the compositor cannot know at creation time.
        // Only compositor-internal layers (background, menubar) should be explicitly opaque.
        let layer_id = self.compositor.add_layer(x, y, content_w, full_h, false);

        if !borderless || force_shadow {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.has_shadow = true;
            }
        }

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from(title),
            x,
            y,
            content_width: content_w,
            content_height: content_h,
            flags,
            owner_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id: 0,
            shm_ptr: core::ptr::null_mut(),
            shm_width: 0,
            shm_height: 0,
            needs_frame_ack: false,
            modal_owner: 0,
            fullscreen: false,
            fullscreen_capable: false,
            saved_bounds_fs: None,
            saved_flags_fs: 0,
            shortcut_slot: 0,
        };

        self.windows.push(win);
        self.assign_fkey_slot(id);
        self.focus_window(id);

        id
    }

    // ── F-Key Shortcut Slot Management ──────────────────────────────

    /// Assign the next free F-key shortcut slot (F1..F12) to a window.
    /// Only non-borderless IPC windows (owner_tid != 0) get a slot.
    pub(crate) fn assign_fkey_slot(&mut self, win_id: u32) {
        let idx = match self.windows.iter().position(|w| w.id == win_id) {
            Some(i) => i,
            None => return,
        };
        // Only decorated (non-borderless) IPC windows get shortcuts.
        // Skip modal windows (dialogs) — they belong to their owner.
        if self.windows[idx].is_borderless() || self.windows[idx].owner_tid == 0 {
            return;
        }
        if self.windows[idx].modal_owner != 0 {
            return;
        }
        // Skip dialog-like windows (not resizable + no minimize + no maximize)
        let f = self.windows[idx].flags;
        if f & WIN_FLAG_NOT_RESIZABLE != 0
            && f & WIN_FLAG_NO_MINIMIZE != 0
            && f & WIN_FLAG_NO_MAXIMIZE != 0
        {
            return;
        }
        // Find first free slot
        for slot in 0..12u8 {
            if self.fkey_slots[slot as usize] == 0 {
                self.fkey_slots[slot as usize] = win_id;
                self.windows[idx].shortcut_slot = slot + 1; // 1-based
                return;
            }
        }
        // All 12 slots occupied — no shortcut assigned
    }

    /// Release the F-key slot for a window and reassign it to a window without one.
    pub(crate) fn release_fkey_slot(&mut self, win_id: u32) {
        // Find and clear the slot
        let mut freed_slot: Option<u8> = None;
        for slot in 0..12u8 {
            if self.fkey_slots[slot as usize] == win_id {
                self.fkey_slots[slot as usize] = 0;
                freed_slot = Some(slot);
                break;
            }
        }
        // Clear on the window itself (in case it's still in the list)
        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
            self.windows[idx].shortcut_slot = 0;
        }
        // Reassign the freed slot to the first unassigned non-borderless window
        // (exclude the window being destroyed — it's still in the list at this point)
        if let Some(slot) = freed_slot {
            let candidate = self
                .windows
                .iter()
                .find(|w| {
                    w.id != win_id && w.shortcut_slot == 0 && !w.is_borderless() && w.owner_tid != 0
                })
                .map(|w| w.id);
            if let Some(cand_id) = candidate {
                self.fkey_slots[slot as usize] = cand_id;
                if let Some(idx) = self.windows.iter().position(|w| w.id == cand_id) {
                    self.windows[idx].shortcut_slot = slot + 1;
                    self.render_titlebar(cand_id);
                }
            }
        }
    }

    /// Destroy a window.
    pub fn destroy_window(&mut self, id: u32) {
        self.release_fkey_slot(id);
        // Cancel any drag rooted at this window before tearing it down.
        self.drag_cancel_for_window(id);
        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            // Clear modal_owner on any windows that reference the destroyed window.
            for w in &mut self.windows {
                if w.modal_owner == id {
                    w.modal_owner = 0;
                }
            }
            let layer_id = self.windows[idx].layer_id;
            self.compositor.remove_layer(layer_id);
            self.windows.remove(idx);

            self.menu_bar.remove_menu(id);

            if self.focused_window == Some(id) {
                self.focused_window = None;
                self.app_cursor = None;
                self.compositor.set_focused_layer(None);
                if let Some(last) = self.windows.last() {
                    let next_id = last.id;
                    self.focus_window(next_id);
                } else {
                    if self.menu_bar.on_focus_change(None) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0,
                            0,
                            self.screen_width,
                            menubar_height() + 1,
                        ));
                    }
                }
            }
            // Refresh shortcut overlay if it's open
            if self.shortcut_overlay_visible {
                self.render_shortcut_overlay();
            }
        }
    }

    /// Destroy all windows owned by a given thread (process exit cleanup).
    pub fn on_process_exit(&mut self, tid: u32) {
        // If the exiting process owns the fullscreen window, exit fullscreen first
        if let Some(fs_id) = self.fullscreen_window {
            if self
                .windows
                .iter()
                .any(|w| w.id == fs_id && w.owner_tid == tid)
            {
                self.exit_fullscreen();
            }
        }
        // Tear down any drag whose source belongs to the exiting process,
        // so the target doesn't keep dangling DRAG_ENTER state.
        self.drag_cancel_for_tid(tid);

        let window_ids: Vec<u32> = self
            .windows
            .iter()
            .filter(|w| w.owner_tid == tid)
            .map(|w| w.id)
            .collect();
        for id in window_ids {
            self.destroy_window(id);
        }
        self.app_subs.retain(|(t, _)| *t != tid);
    }

    /// Safety net for crashed apps: remove windows/subscriptions whose owner TID no longer exists.
    pub fn reap_exited_processes(&mut self) -> Vec<u32> {
        let mut candidate_tids: Vec<u32> = Vec::new();
        for win in &self.windows {
            if win.owner_tid != 0 && !candidate_tids.contains(&win.owner_tid) {
                candidate_tids.push(win.owner_tid);
            }
        }
        for &(tid, _) in &self.app_subs {
            if tid != 0 && !candidate_tids.contains(&tid) {
                candidate_tids.push(tid);
            }
        }

        let mut exited_tids: Vec<u32> = Vec::new();
        for tid in candidate_tids {
            let status = process::try_waitpid(tid);
            if status != process::STILL_RUNNING && status != process::STOPPED {
                exited_tids.push(tid);
            }
        }

        for &tid in &exited_tids {
            println!("compositor: reaping dead app tid={}", tid);
            self.on_process_exit(tid);
        }

        exited_tids
    }

    /// Called when system theme changes — re-render all window chrome and menubar.
    pub fn on_theme_change(&mut self) {
        self.draw_menubar();

        let win_ids: Vec<u32> = self
            .windows
            .iter()
            .filter(|w| !w.is_borderless())
            .map(|w| w.id)
            .collect();
        for id in win_ids {
            self.render_window(id);
        }

        self.compositor.damage_all();
    }

    /// Focus a window (bring to front and set focused style).
    /// If the target window has a modal child, redirect focus to the modal instead.
    pub fn focus_window(&mut self, id: u32) {
        // Check if this window has a modal child — if so, redirect focus to the modal.
        // Walk the chain to find the topmost modal descendant.
        let mut target_id = id;
        for _ in 0..16 {
            // prevent infinite loops
            if let Some(modal_child_id) = self
                .windows
                .iter()
                .find(|w| w.modal_owner == target_id)
                .map(|w| w.id)
            {
                target_id = modal_child_id;
            } else {
                break;
            }
        }

        if let Some(old_id) = self.focused_window {
            if old_id != target_id {
                if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                    self.windows[idx].focused = false;
                    let win_id = self.windows[idx].id;
                    self.render_titlebar(win_id);
                    self.push_event(win_id, [EVENT_FOCUS_LOST, 0, 0, 0, 0]);
                }
                // Drop the previous app's cursor override on focus change.
                self.app_cursor = None;
            }
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == target_id) {
            self.windows[idx].focused = true;
            self.focused_window = Some(target_id);
            let owner_tid = self.windows[idx].owner_tid;
            let layer_id = self.windows[idx].layer_id;
            self.compositor.set_focused_layer(Some(layer_id));
            self.compositor.raise_layer(layer_id);

            let win = self.windows.remove(idx);
            self.windows.push(win);

            self.ensure_top_layers();
            self.render_window(target_id);

            if self.menu_bar.on_focus_change(Some(target_id)) {
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
                ));
            }

            // Broadcast focus change for Shell
            self.emit_focus_changed(owner_tid, target_id);
        }
    }

    /// Focus a window without re-rendering it (used when chrome was pre-rendered).
    pub(crate) fn focus_window_no_render(&mut self, id: u32) {
        if let Some(old_id) = self.focused_window {
            if old_id != id {
                if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                    self.windows[idx].focused = false;
                    let win_id = self.windows[idx].id;
                    self.render_titlebar(win_id);
                    self.push_event(win_id, [EVENT_FOCUS_LOST, 0, 0, 0, 0]);
                }
                self.app_cursor = None;
            }
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            self.windows[idx].focused = true;
            self.focused_window = Some(id);
            let owner_tid = self.windows[idx].owner_tid;
            let layer_id = self.windows[idx].layer_id;
            self.compositor.set_focused_layer(Some(layer_id));
            self.compositor.raise_layer(layer_id);

            let win = self.windows.remove(idx);
            self.windows.push(win);

            self.ensure_top_layers();
            self.compositor.mark_layer_dirty(layer_id);

            if self.menu_bar.on_focus_change(Some(id)) {
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
                ));
            }

            self.emit_focus_changed(owner_tid, id);
        }
    }

    /// Re-raise always-on-top windows, modal children, and the menubar.
    /// Order: modal children first, then always-on-top windows (popups),
    /// so that dropdown/context-menu popups appear above modal dialogs.
    pub(crate) fn ensure_top_layers(&mut self) {
        // Raise modal children above their owners (iterate in order so chains
        // A→B→C raise correctly: B above A, then C above B).
        for i in 0..self.windows.len() {
            if self.windows[i].modal_owner != 0 {
                self.compositor.raise_layer(self.windows[i].layer_id);
            }
        }
        // Raise always-on-top windows above modals (includes dropdown/context
        // menu popups which are borderless + always-on-top).
        for win in &self.windows {
            if win.is_always_on_top() {
                self.compositor.raise_layer(win.layer_id);
            }
        }
        self.compositor.raise_layer(self.menubar_layer_id);
    }

    /// Get a window's event queue.
    pub fn poll_event(&mut self, window_id: u32) -> Option<[u32; 5]> {
        self.windows
            .iter_mut()
            .find(|w| w.id == window_id)
            .and_then(|w| w.events.pop_front())
    }

    /// Push an event to a window's queue.
    pub(crate) fn push_event(&mut self, window_id: u32, event: [u32; 5]) {
        if let Some(win) = self.windows.iter_mut().find(|w| w.id == window_id) {
            if win.events.len() < 256 {
                win.events.push_back(event);
            }
        }
    }

    // ── Window Rendering ───────────────────────────────────────────────

    /// Render a window's surface (decorations + content area).
    pub(crate) fn render_window(&mut self, window_id: u32) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        let layer_id = self.windows[win_idx].layer_id;
        let cw = self.windows[win_idx].content_width;
        let _ch = self.windows[win_idx].content_height;
        let borderless = self.windows[win_idx].is_borderless();
        let focused = self.windows[win_idx].focused;
        let full_h = self.windows[win_idx].full_height();
        let shortcut_slot = self.windows[win_idx].shortcut_slot;
        // Pre-compute the list of "send to monitor" target output ids.
        // Done before layer_pixels() borrows self.compositor mutably.
        let other_outputs: alloc::vec::Vec<u8> = self.other_outputs_for_window(window_id);
        // Stack-copy title to avoid heap allocation (title.clone())
        let mut title_buf = [0u8; 256];
        let title_len = self.windows[win_idx].title.len().min(256);
        title_buf[..title_len]
            .copy_from_slice(&self.windows[win_idx].title.as_bytes()[..title_len]);
        let title_str = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("");

        // Borderless IPC windows: no chrome — restore SHM content directly.
        if borderless && self.windows[win_idx].owner_tid != 0 {
            if !self.windows[win_idx].shm_ptr.is_null() {
                self.present_ipc_window(window_id, None);
            }
            return;
        }

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;

            if borderless {
                for p in pixels.iter_mut() {
                    *p = 0x00000000;
                }
            } else {
                for p in pixels.iter_mut() {
                    *p = 0x00000000;
                }

                fill_rounded_rect(
                    pixels,
                    stride,
                    full_h,
                    0,
                    0,
                    cw,
                    full_h,
                    8,
                    color_window_bg(),
                );

                draw_rounded_rect_outline(
                    pixels,
                    stride,
                    full_h,
                    0,
                    0,
                    cw,
                    full_h,
                    8,
                    color_window_border(),
                );

                let (tb_top, tb_bot) = if focused {
                    (
                        color_titlebar_focused_top(),
                        color_titlebar_focused_bottom(),
                    )
                } else {
                    (
                        color_titlebar_unfocused_top(),
                        color_titlebar_unfocused_bottom(),
                    )
                };
                let tb_h = title_bar_height();
                fill_rounded_rect_top_gradient(pixels, stride, 0, 0, cw, tb_h, 8, tb_top, tb_bot);

                let border_y = tb_h - 1;
                for x in 0..cw {
                    let idx = (border_y * stride + x) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = color_window_border();
                    }
                }

                // Traffic light buttons with animated hover/press blend
                let now = anyos_std::sys::uptime();
                let win_flags = self.windows[win_idx].flags;
                let btn_hidden = [
                    win_flags & WIN_FLAG_NO_CLOSE != 0,
                    win_flags & WIN_FLAG_NO_MINIMIZE != 0,
                    win_flags & WIN_FLAG_NO_MAXIMIZE != 0,
                ];
                let base_colors: [u32; 3] = if focused {
                    [COLOR_CLOSE_BTN, COLOR_MIN_BTN, COLOR_MAX_BTN]
                } else {
                    [
                        color_btn_unfocused(),
                        color_btn_unfocused(),
                        color_btn_unfocused(),
                    ]
                };
                let btn_sz = title_btn_size();
                let btn_sp = title_btn_spacing();
                let btn_y_pos = title_btn_y();
                let btn_left = crate::desktop::theme::scale_i32(8);
                for (i, &base) in base_colors.iter().enumerate() {
                    if btn_hidden[i] {
                        continue;
                    }
                    let aid = button_anim_id(window_id, i as u8);
                    let color = if let Some(t) = self.btn_anims.value(aid, now) {
                        let target = if self.btn_pressed == Some((window_id, i as u8)) {
                            button_press_color(i as u8)
                        } else {
                            button_hover_color(i as u8)
                        };
                        anyos_std::anim::color_blend(base, target, t as u32)
                    } else {
                        base
                    };
                    let cx = btn_left + i as i32 * btn_sp as i32 + btn_sz as i32 / 2;
                    let cy = btn_y_pos as i32 + btn_sz as i32 / 2;
                    fill_circle(pixels, stride, full_h, cx, cy, (btn_sz / 2) as i32, color);
                }

                // Shortcut button (F1..F12 badge)
                render_shortcut_btn(pixels, stride, full_h, shortcut_slot, focused);

                // Per-output "send to monitor" buttons on the right.
                // Drawn right-to-left so output id order reads naturally.
                for (slot, &target_id) in other_outputs.iter().enumerate() {
                    let bx = monitor_btn_x_at(cw, slot as u32);
                    render_monitor_btn(pixels, stride, full_h, bx, target_id, focused);
                }

                // Available width for title: between buttons (left) and window edge (right), with padding
                let left_bound = if shortcut_slot > 0 {
                    shortcut_btn_right() + title_padding()
                } else {
                    title_buttons_right() + title_padding()
                };
                let max_title_w = if (cw as i32) > left_bound + title_padding() {
                    (cw as i32 - left_bound - title_padding()) as u32
                } else {
                    0
                };

                let trunc_len = title_display_len(title_str, max_title_w);
                let mut display_buf = [0u8; 260];
                let display_str = if trunc_len < title_str.len() && trunc_len > 0 {
                    let total = trunc_len + 3;
                    display_buf[..trunc_len].copy_from_slice(&title_str.as_bytes()[..trunc_len]);
                    display_buf[trunc_len..trunc_len + 3].copy_from_slice(b"...");
                    core::str::from_utf8(&display_buf[..total]).unwrap_or(title_str)
                } else if trunc_len == 0 {
                    ""
                } else {
                    title_str
                };

                if !display_str.is_empty() {
                    let fs = scaled_font_size();
                    let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, display_str);
                    // Center within available area (right of buttons)
                    let center_area = cw as i32;
                    let mut tx = (center_area - tw as i32) / 2;
                    // Ensure title doesn't overlap buttons
                    if tx < left_bound {
                        tx = left_bound;
                    }
                    let ty = ((tb_h as i32 - th as i32) / 2).max(0);
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID,
                        fs,
                        pixels,
                        stride,
                        full_h,
                        tx,
                        ty,
                        color_titlebar_text(),
                        display_str,
                    );
                }
            }
        }

        self.compositor.mark_layer_dirty(layer_id);

        if self.windows[win_idx].owner_tid != 0 && !self.windows[win_idx].shm_ptr.is_null() {
            self.present_ipc_window(window_id, None);
        }
    }

    /// Lightweight title-bar-only repaint for focus/unfocus changes.
    pub(crate) fn render_titlebar(&mut self, window_id: u32) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        let layer_id = self.windows[win_idx].layer_id;
        let cw = self.windows[win_idx].content_width;
        let focused = self.windows[win_idx].focused;
        let full_h = self.windows[win_idx].full_height();
        let shortcut_slot = self.windows[win_idx].shortcut_slot;
        let other_outputs: alloc::vec::Vec<u8> = self.other_outputs_for_window(window_id);
        // Stack-copy title to avoid heap allocation (title.clone())
        let mut title_buf = [0u8; 256];
        let title_len = self.windows[win_idx].title.len().min(256);
        title_buf[..title_len]
            .copy_from_slice(&self.windows[win_idx].title.as_bytes()[..title_len]);
        let title_str = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("");

        if self.windows[win_idx].is_borderless() {
            return;
        }

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;
            let tb_h = title_bar_height();

            let (tb_top, tb_bot) = if focused {
                (
                    color_titlebar_focused_top(),
                    color_titlebar_focused_bottom(),
                )
            } else {
                (
                    color_titlebar_unfocused_top(),
                    color_titlebar_unfocused_bottom(),
                )
            };
            fill_rounded_rect_top_gradient(pixels, stride, 0, 0, cw, tb_h, 8, tb_top, tb_bot);

            let border_y = tb_h - 1;
            for x in 0..cw {
                let idx = (border_y * stride + x) as usize;
                if idx < pixels.len() {
                    pixels[idx] = color_window_border();
                }
            }

            // Traffic light buttons
            let now = anyos_std::sys::uptime();
            let base_colors: [u32; 3] = if focused {
                [COLOR_CLOSE_BTN, COLOR_MIN_BTN, COLOR_MAX_BTN]
            } else {
                [
                    color_btn_unfocused(),
                    color_btn_unfocused(),
                    color_btn_unfocused(),
                ]
            };
            let btn_sz = title_btn_size();
            let btn_sp = title_btn_spacing();
            let btn_y_pos = title_btn_y();
            let btn_left = crate::desktop::theme::scale_i32(8);
            for (i, &base) in base_colors.iter().enumerate() {
                let aid = button_anim_id(window_id, i as u8);
                let color = if let Some(t) = self.btn_anims.value(aid, now) {
                    let target = if self.btn_pressed == Some((window_id, i as u8)) {
                        button_press_color(i as u8)
                    } else {
                        button_hover_color(i as u8)
                    };
                    anyos_std::anim::color_blend(base, target, t as u32)
                } else {
                    base
                };
                let cx = btn_left + i as i32 * btn_sp as i32 + btn_sz as i32 / 2;
                let cy = btn_y_pos as i32 + btn_sz as i32 / 2;
                fill_circle(pixels, stride, full_h, cx, cy, (btn_sz / 2) as i32, color);
            }

            // Shortcut button (F1..F12 badge)
            render_shortcut_btn(pixels, stride, full_h, shortcut_slot, focused);

            // Per-output "send to monitor" buttons on the right.
            for (slot, &target_id) in other_outputs.iter().enumerate() {
                let bx = monitor_btn_x_at(cw, slot as u32);
                render_monitor_btn(pixels, stride, full_h, bx, target_id, focused);
            }

            let left_bound = if shortcut_slot > 0 {
                shortcut_btn_right() + title_padding()
            } else {
                title_buttons_right() + title_padding()
            };
            let max_title_w = if (cw as i32) > left_bound + title_padding() {
                (cw as i32 - left_bound - title_padding()) as u32
            } else {
                0
            };

            let trunc_len = title_display_len(title_str, max_title_w);
            let mut display_buf = [0u8; 260];
            let display_str = if trunc_len < title_str.len() && trunc_len > 0 {
                let total = trunc_len + 3;
                display_buf[..trunc_len].copy_from_slice(&title_str.as_bytes()[..trunc_len]);
                display_buf[trunc_len..trunc_len + 3].copy_from_slice(b"...");
                core::str::from_utf8(&display_buf[..total]).unwrap_or(title_str)
            } else if trunc_len == 0 {
                ""
            } else {
                title_str
            };

            if !display_str.is_empty() {
                let fs = scaled_font_size();
                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, display_str);
                let center_area = cw as i32;
                let mut tx = (center_area - tw as i32) / 2;
                if tx < left_bound {
                    tx = left_bound;
                }
                let ty = ((tb_h as i32 - th as i32) / 2).max(0);
                anyos_std::ui::window::font_render_buf(
                    FONT_ID,
                    fs,
                    pixels,
                    stride,
                    full_h,
                    tx,
                    ty,
                    color_titlebar_text(),
                    display_str,
                );
            }
        }

        self.compositor.mark_layer_dirty(layer_id);
    }

    /// Toggle window maximize/restore.
    /// Refresh the kernel-reported output set and reflow any windows
    /// that were on outputs which just disappeared.
    ///
    /// Called from the management thread on display events
    /// (HotplugChanged / LayoutApplied / PreferredModeChanged). For
    /// every window currently sitting on a vanished output we move
    /// it onto the primary, preserving its inset relative to its old
    /// output's top-left so the user finds it roughly where it was —
    /// just on a different physical screen. Same clamp/shrink rules
    /// as the manual "send to monitor" button path so a window from
    /// a 4K secondary that got unplugged still fits on the primary.
    pub(crate) fn refresh_outputs_and_reflow(&mut self) {
        let vanished = self.compositor.refresh_outputs();
        if vanished.is_empty() {
            return;
        }
        // Snapshot the windows that need to move (by id) before
        // mutating, so the iteration doesn't see a Vec mid-edit.
        let mut targets: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
        for w in &self.windows {
            // For a vanished output we know the old virtual_x /
            // virtual_y was somewhere in that output's old rect —
            // but we don't keep the old rects after refresh_outputs
            // removed them. Use a "any window outside the union of
            // current outputs" heuristic: those are the ones that
            // need rescuing.
            let cx = w.x + (w.content_width as i32 / 2);
            let cy = w.y;
            let on_an_output = self.compositor.outputs.iter().any(|o| {
                cx >= o.virtual_x
                    && cy >= o.virtual_y
                    && cx < o.virtual_x + o.fb_width as i32
                    && cy < o.virtual_y + o.fb_height as i32
            });
            if !on_an_output {
                targets.push(w.id);
            }
        }
        if targets.is_empty() {
            return;
        }
        let primary_id = self.compositor.outputs[0].id as u8;
        for win_id in targets {
            self.move_window_to_output(win_id, primary_id);
        }
        let _ = vanished;
    }

    /// Output ids that should appear as "send to monitor" buttons in
    /// the title bar of `window_id`. The window's current output is
    /// excluded; remaining outputs are returned in id order, capped to
    /// 8 entries (we only have rendering room for that many buttons).
    pub(crate) fn other_outputs_for_window(&self, window_id: u32) -> alloc::vec::Vec<u8> {
        let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        if self.compositor.outputs.len() < 2 {
            return out;
        }
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return out,
        };
        let w = &self.windows[win_idx];
        // Use the same titlebar-centre heuristic the maximize path uses
        // so the "current output" matches what the user perceives.
        let cx = w.x + (w.content_width as i32 / 2);
        let cy = w.y;
        let current_id = self.compositor.output_at(cx, cy).id;
        for o in &self.compositor.outputs {
            if o.id == current_id {
                continue;
            }
            if out.len() >= 8 {
                break;
            }
            out.push(o.id as u8);
        }
        out
    }

    /// Move the window onto a different output, preserving its relative
    /// position inside the source output's rect when possible.
    /// Falls back to clamping into the target output's bounds when the
    /// translated rectangle would go off-screen on the new output (e.g.
    /// the source was a 4K monitor and the target is a 1280×800 panel).
    /// No-op when the window is already on the target output, or when
    /// the target id doesn't match any active output.
    pub(crate) fn move_window_to_output(&mut self, window_id: u32, target_output_id: u8) {
        // Resolve the target output rect.
        let target = match self
            .compositor
            .outputs
            .iter()
            .find(|o| o.id as u8 == target_output_id)
        {
            Some(o) => (o.virtual_x, o.virtual_y, o.fb_width, o.fb_height),
            None => return,
        };
        let (tx, ty, tw, th) = target;

        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };
        let win_x = self.windows[win_idx].x;
        let win_y = self.windows[win_idx].y;
        let cw = self.windows[win_idx].content_width as i32;
        let full_h = self.windows[win_idx].full_height() as i32;
        let layer_id = self.windows[win_idx].layer_id;

        // Source output (the one whose rect contains the titlebar centre).
        let cx = win_x + cw / 2;
        let cy = win_y;
        let (sx, sy, sw, sh) = {
            let so = self.compositor.output_at(cx, cy);
            (so.virtual_x, so.virtual_y, so.fb_width, so.fb_height)
        };
        if (sx, sy) == (tx, ty) {
            return; // already on target
        }

        // Local position inside the source output, then translate into
        // target output. Clamp so the window stays at least mostly
        // visible on the target if the target is smaller than the source.
        let local_x = win_x - sx;
        let local_y = win_y - sy;
        let mut new_x = tx + local_x;
        let mut new_y = ty + local_y;

        // Clamp X: keep the window's right edge inside target right edge
        // and the left edge non-negative-relative-to-target.
        let right_edge = tx + tw as i32;
        let bottom_edge = ty + th as i32;
        if new_x + cw > right_edge {
            new_x = right_edge - cw;
        }
        if new_x < tx {
            new_x = tx;
        }
        if new_y + full_h > bottom_edge {
            new_y = bottom_edge - full_h;
        }
        // Keep the title bar visible (don't move it under the menubar
        // on the primary; on secondaries there is no menubar so we
        // just clamp to the output top).
        let min_y = if tx == 0 && ty == 0 {
            menubar_height() as i32 + 1
        } else {
            ty
        };
        if new_y < min_y {
            new_y = min_y;
        }

        // If the window is wider/taller than the target output, shrink
        // it to fit (preserve content aspect approximately by capping
        // each dimension). This keeps the visible-after-move invariant
        // even on extreme size deltas.
        let mut new_cw = cw as u32;
        let mut new_ch = self.windows[win_idx].content_height;
        let max_w = tw;
        let max_h = th.saturating_sub(if tx == 0 && ty == 0 {
            menubar_height() + title_bar_height() + 1
        } else {
            title_bar_height()
        });
        let resized = new_cw > max_w || new_ch > max_h;
        if new_cw > max_w {
            new_cw = max_w;
        }
        if new_ch > max_h {
            new_ch = max_h;
        }

        // If the window was maximized, leave that state intact but
        // re-apply the maximize against the new output's bounds. The
        // simplest path is to drop the maximize flag; users can hit
        // the maximize button again on the new monitor if they want.
        if self.windows[win_idx].maximized {
            self.windows[win_idx].maximized = false;
            self.windows[win_idx].saved_bounds = None;
        }

        self.windows[win_idx].x = new_x;
        self.windows[win_idx].y = new_y;
        self.windows[win_idx].content_width = new_cw;
        self.windows[win_idx].content_height = new_ch;

        let new_full_h = self.windows[win_idx].full_height();
        self.compositor.move_layer(layer_id, new_x, new_y);
        if resized {
            self.compositor.resize_layer(layer_id, new_cw, new_full_h);
            self.push_event(window_id, [EVENT_RESIZE, new_cw, new_ch, 0, 0]);
        }
        // Notify the app so libanyui can refresh its cached window-frame
        // position (otherwise Window::get_position keeps returning the
        // initial spawn coords forever after a cross-monitor move).
        self.push_event(
            window_id,
            [EVENT_WINDOW_MOVED, new_x as u32, new_y as u32, 0, 0],
        );
        self.render_window(window_id);
    }

    pub(crate) fn toggle_maximize(&mut self, win_id: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
            if self.windows[idx].maximized {
                if let Some((sx, sy, sw, sh)) = self.windows[idx].saved_bounds {
                    let layer_id = self.windows[idx].layer_id;
                    self.windows[idx].x = sx;
                    self.windows[idx].y = sy;
                    self.windows[idx].content_width = sw;
                    self.windows[idx].content_height = sh;
                    self.windows[idx].maximized = false;

                    let full_h = self.windows[idx].full_height();
                    self.compositor.move_layer(layer_id, sx, sy);
                    self.compositor.resize_layer(layer_id, sw, full_h);
                    self.render_window(win_id);
                    self.push_event(win_id, [EVENT_RESIZE, sw, sh, 0, 0]);
                    self.push_event(
                        win_id,
                        [EVENT_WINDOW_MOVED, sx as u32, sy as u32, 0, 0],
                    );
                }
            } else {
                let x = self.windows[idx].x;
                let y = self.windows[idx].y;
                let cw = self.windows[idx].content_width;
                let ch = self.windows[idx].content_height;
                self.windows[idx].saved_bounds = Some((x, y, cw, ch));
                self.windows[idx].maximized = true;

                // Multi-monitor: maximize on the output under the window's
                // titlebar centre (matching wlroots / Windows / macOS
                // semantics). For pure single-output setups output_at()
                // always returns the primary, so this collapses to the
                // legacy 0,0,screen_width,screen_height path.
                let titlebar_cx = x + (cw as i32 / 2);
                let titlebar_cy = y;
                let (out_x, out_y, out_w, out_h) = {
                    let o = self.compositor.output_at(titlebar_cx, titlebar_cy);
                    (o.virtual_x, o.virtual_y, o.fb_width, o.fb_height)
                };
                // Menubar lives on the primary output only — secondary
                // outputs use their full height for the maximized window.
                let mb = if out_x == 0 && out_y == 0 {
                    menubar_height() as i32 + 1
                } else {
                    0
                };
                let new_x = out_x;
                let new_y = out_y + mb;
                let new_w = out_w;
                let new_ch = out_h - mb as u32 - title_bar_height();

                let layer_id = self.windows[idx].layer_id;
                self.windows[idx].x = new_x;
                self.windows[idx].y = new_y;
                self.windows[idx].content_width = new_w;
                self.windows[idx].content_height = new_ch;

                let full_h = self.windows[idx].full_height();
                self.compositor.move_layer(layer_id, new_x, new_y);
                self.compositor.resize_layer(layer_id, new_w, full_h);
                self.render_window(win_id);
                self.push_event(win_id, [EVENT_RESIZE, new_w, new_ch, 0, 0]);
                self.push_event(
                    win_id,
                    [EVENT_WINDOW_MOVED, new_x as u32, new_y as u32, 0, 0],
                );
            }
        }
    }

    /// Tile all visible windows evenly across the desktop.
    pub(crate) fn tile_all_windows(&mut self) {
        // Collect IDs of visible, framed windows (not minimized, not borderless)
        // Borderless windows (dock, toolbar, notifications, overlays) are never rearranged.
        let mut visible_ids: Vec<u32> = Vec::new();
        for win in &self.windows {
            if win.x >= 0 && !win.is_borderless() {
                visible_ids.push(win.id);
            }
        }
        let count = visible_ids.len();
        if count == 0 {
            return;
        }

        let area_x = 0i32;
        let area_y = menubar_height() as i32 + 1;
        let area_w = self.screen_width;
        let area_h = self.screen_height - menubar_height() - 1;

        // Compute grid: cols x rows
        let cols = if count <= 1 {
            1u32
        } else if count <= 2 {
            2
        } else if count <= 4 {
            2
        } else if count <= 6 {
            3
        } else if count <= 9 {
            3
        } else {
            4
        };
        let rows = ((count as u32) + cols - 1) / cols;

        let cell_w = area_w / cols;
        let cell_h = area_h / rows;

        for (i, &win_id) in visible_ids.iter().enumerate() {
            let col = (i as u32) % cols;
            let row = (i as u32) / cols;

            let wx = area_x + (col * cell_w) as i32;
            let wy = area_y + (row * cell_h) as i32;

            if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                let borderless = self.windows[idx].is_borderless();
                let tb = if borderless { 0 } else { title_bar_height() };
                let content_w = cell_w;
                let content_h = cell_h.saturating_sub(tb);

                self.windows[idx].x = wx;
                self.windows[idx].y = wy;
                self.windows[idx].content_width = content_w;
                self.windows[idx].content_height = content_h;
                self.windows[idx].maximized = false;
                self.windows[idx].saved_bounds = None;

                let layer_id = self.windows[idx].layer_id;
                let full_h = self.windows[idx].full_height();
                self.compositor.move_layer(layer_id, wx, wy);
                self.compositor.resize_layer(layer_id, content_w, full_h);
                // Re-render chrome and blit existing SHM content into new layer size
                self.render_window(win_id);
                // Notify app so it re-renders at the new size
                self.push_event(win_id, [EVENT_RESIZE, content_w, content_h, 0, 0]);
                self.push_event(
                    win_id,
                    [EVENT_WINDOW_MOVED, wx as u32, wy as u32, 0, 0],
                );
            }
        }
        // Mark entire screen dirty so old window positions are repainted
        self.compositor.damage_all();
    }

    /// Toggle "show desktop": minimize all windows, or restore them.
    pub(crate) fn toggle_show_desktop(&mut self) {
        // Check if any visible framed window exists
        let has_visible = self
            .windows
            .iter()
            .any(|w| w.x >= 0 && !w.is_borderless() && w.owner_tid != 0);

        if has_visible {
            // Minimize all visible framed windows
            let ids: Vec<u32> = self
                .windows
                .iter()
                .filter(|w| w.x >= 0 && !w.is_borderless() && w.owner_tid != 0)
                .map(|w| w.id)
                .collect();
            for id in ids {
                self.minimize_window(id);
            }
        } else {
            // Restore: un-minimize all windows that have saved bounds
            let ids: Vec<u32> = self
                .windows
                .iter()
                .filter(|w| w.x < -9000 && w.saved_bounds.is_some() && w.owner_tid != 0)
                .map(|w| w.id)
                .collect();
            for id in ids {
                if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
                    if let Some((sx, sy, _sw, _sh)) = self.windows[idx].saved_bounds.take() {
                        self.windows[idx].x = sx;
                        self.windows[idx].y = sy;
                        let layer_id = self.windows[idx].layer_id;
                        self.compositor.move_layer(layer_id, sx, sy);
                        self.push_event(
                            id,
                            [EVENT_WINDOW_MOVED, sx as u32, sy as u32, 0, 0],
                        );
                    }
                }
            }
            self.compositor.damage_all();
        }
    }

    /// Snap a window to the left or right half of the screen.
    /// `edge`: 0 = left, 1 = right, 2 = top (full width top half), 3 = bottom (full width bottom half).
    pub(crate) fn snap_window_to_half(&mut self, win_id: u32, edge: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
            let area_y = menubar_height() as i32 + 1;
            let area_h = self.screen_height - menubar_height() - 1;
            let area_w = self.screen_width;

            // Save original bounds for restore
            if self.windows[idx].saved_bounds.is_none() {
                self.windows[idx].saved_bounds = Some((
                    self.windows[idx].x,
                    self.windows[idx].y,
                    self.windows[idx].content_width,
                    self.windows[idx].content_height,
                ));
            }

            let borderless = self.windows[idx].is_borderless();
            let tb = if borderless { 0 } else { title_bar_height() };

            let (wx, wy, cw, ch) = match edge {
                0 => (0i32, area_y, area_w / 2, area_h.saturating_sub(tb)), // left
                1 => (
                    (area_w / 2) as i32,
                    area_y,
                    area_w / 2,
                    area_h.saturating_sub(tb),
                ), // right
                2 => (0i32, area_y, area_w, (area_h / 2).saturating_sub(tb)), // top
                _ => (
                    0i32,
                    area_y + (area_h / 2) as i32,
                    area_w,
                    (area_h / 2).saturating_sub(tb),
                ), // bottom
            };

            self.windows[idx].x = wx;
            self.windows[idx].y = wy;
            self.windows[idx].content_width = cw;
            self.windows[idx].content_height = ch;
            self.windows[idx].maximized = false;

            let layer_id = self.windows[idx].layer_id;
            let full_h = self.windows[idx].full_height();
            self.compositor.move_layer(layer_id, wx, wy);
            self.compositor.resize_layer(layer_id, cw, full_h);
            // Re-render chrome and blit existing SHM content into new layer size
            self.render_window(win_id);
            // Notify app so it re-renders at the new size
            self.push_event(win_id, [EVENT_RESIZE, cw, ch, 0, 0]);
            self.push_event(
                win_id,
                [EVENT_WINDOW_MOVED, wx as u32, wy as u32, 0, 0],
            );
            // Mark entire screen dirty so old window position is repainted
            self.compositor.damage_all();
        }
    }

    /// Minimize a window (move off-screen and save bounds for restore).
    pub(crate) fn minimize_window(&mut self, win_id: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
            if self.windows[idx].x >= 0 && self.windows[idx].saved_bounds.is_none() {
                self.windows[idx].saved_bounds = Some((
                    self.windows[idx].x,
                    self.windows[idx].y,
                    self.windows[idx].content_width,
                    self.windows[idx].full_height(),
                ));
            }
            let layer_id = self.windows[idx].layer_id;
            self.compositor.move_layer(layer_id, -10000, -10000);
            // Focus next visible window
            if let Some(next_id) = self
                .windows
                .iter()
                .rev()
                .find(|w| w.id != win_id && w.x >= 0)
                .map(|w| w.id)
            {
                self.focus_window(next_id);
            }
        }
    }

    // ── IPC Window Operations ──────────────────────────────────────────

    /// Resolve the initial window position.
    /// Missing coordinates are centered on the usable desktop area per axis.
    fn resolve_initial_window_position(
        &self,
        raw_x: u16,
        raw_y: u16,
        win_w: u32,
        win_h: u32,
        clamp_explicit_y: bool,
    ) -> (i32, i32) {
        let min_y = menubar_height() as i32 + 1;
        let area_w = self.screen_width as i32;
        let area_h = (self.screen_height as i32 - min_y).max(0);

        let centered_x = ((area_w - win_w as i32) / 2).max(0);
        let centered_y = min_y + ((area_h - win_h as i32) / 2).max(0);

        let x = if raw_x == crate::ipc_protocol::CW_USEDEFAULT {
            centered_x
        } else {
            raw_x as i32
        };
        let y = if raw_y == crate::ipc_protocol::CW_USEDEFAULT {
            centered_y
        } else if clamp_explicit_y {
            (raw_y as i32).max(min_y)
        } else {
            raw_y as i32
        };

        (x, y)
    }

    /// Create a window backed by a shared memory region.
    /// `raw_x` / `raw_y`: pixel coordinates, or `CW_USEDEFAULT` (0xFFFF) for auto-placement.
    pub fn create_ipc_window(
        &mut self,
        app_tid: u32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        shm_id: u32,
        shm_ptr: *mut u32,
        raw_x: u16,
        raw_y: u16,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let borderless = flags & WIN_FLAG_BORDERLESS != 0;
        let full_h = if borderless {
            content_h
        } else {
            content_h + title_bar_height()
        };

        let (x, y) = self.resolve_initial_window_position(raw_x, raw_y, content_w, full_h, true);

        let layer_id = self.compositor.add_layer(x, y, content_w, full_h, false);

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        if !borderless || force_shadow {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.has_shadow = true;
            }
        }

        // DPI-aware windows render at physical resolution — no compositor upscaling.
        if flags & WIN_FLAG_DPI_AWARE != 0 {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.dpi_aware = true;
            }
        }

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from("Window"),
            x,
            y,
            content_width: content_w,
            content_height: content_h,
            flags,
            owner_tid: app_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id,
            shm_ptr,
            shm_width: content_w,
            shm_height: content_h,
            needs_frame_ack: false,
            modal_owner: 0,
            fullscreen: false,
            fullscreen_capable: false,
            saved_bounds_fs: None,
            saved_flags_fs: 0,
            shortcut_slot: 0,
        };

        self.windows.push(win);
        self.assign_fkey_slot(id);
        self.focus_window(id);

        id
    }

    /// Create a VRAM-direct window (app renders directly to off-screen VRAM).
    /// Returns `Some([RESP_VRAM_WINDOW_CREATED, win_id, stride_pixels, tid, surface_va])`
    /// on success, or `None` if VRAM allocation fails.
    pub fn create_vram_window(
        &mut self,
        app_tid: u32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        raw_x: u16,
        raw_y: u16,
    ) -> Option<[u32; 5]> {
        let (x, y) =
            self.resolve_initial_window_position(raw_x, raw_y, content_w, content_h, false);

        // VRAM windows are always borderless + opaque (no title bar chrome)
        let layer_id = self.compositor.add_vram_layer(x, y, content_w, content_h)?;

        // Get the VRAM allocation info for mapping
        let stride_pixels = self.compositor.fb_pitch / 4;
        let alloc_info = self.compositor.vram_allocator.as_ref()?.get(layer_id)?;
        let map_offset = alloc_info.offset;
        let map_size = alloc_info.size;

        // Map VRAM into the app's address space via kernel syscall.
        // Returns the user VA where the surface is mapped (e.g. 0x18000000).
        let surface_va = anyos_std::ipc::vram_map(app_tid, map_offset, map_size);
        if surface_va == 0 || surface_va == u32::MAX {
            // VRAM mapping failed — remove the layer and free allocation
            self.compositor.remove_layer(layer_id);
            return None;
        }

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        if force_shadow {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.has_shadow = true;
            }
        }

        let id = self.next_window_id;
        self.next_window_id += 1;

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from("Window"),
            x,
            y,
            content_width: content_w,
            content_height: content_h,
            flags: flags | WIN_FLAG_BORDERLESS, // VRAM windows are always borderless
            owner_tid: app_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id: 0,
            shm_ptr: core::ptr::null_mut(),
            shm_width: content_w,
            shm_height: content_h,
            needs_frame_ack: false,
            modal_owner: 0,
            fullscreen: false,
            fullscreen_capable: false,
            saved_bounds_fs: None,
            saved_flags_fs: 0,
            shortcut_slot: 0,
        };

        self.windows.push(win);
        self.assign_fkey_slot(id);
        self.focus_window(id);

        Some([
            crate::ipc_protocol::RESP_VRAM_WINDOW_CREATED,
            id,
            stride_pixels,
            app_tid,
            surface_va,
        ])
    }

    /// Create an IPC window using pre-rendered pixels (fast path).
    pub fn create_ipc_window_fast(
        &mut self,
        app_tid: u32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        shm_id: u32,
        shm_ptr: *mut u32,
        pre_pixels: Vec<u32>,
        raw_x: u16,
        raw_y: u16,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let borderless = flags & WIN_FLAG_BORDERLESS != 0;
        let full_h = if borderless {
            content_h
        } else {
            content_h + title_bar_height()
        };

        let (x, y) = self.resolve_initial_window_position(raw_x, raw_y, content_w, full_h, true);

        let layer_id = self
            .compositor
            .add_layer_with_pixels(x, y, content_w, full_h, false, pre_pixels);

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
            // Hide the layer until the first CMD_PRESENT arrives with real pixels.
            // This prevents a transparent / garbage frame from being composited
            // before the app has rendered anything (fixes intermittent missing dock).
            layer.visible = false;

            if !borderless || force_shadow {
                layer.has_shadow = true;
            }

            // DPI-aware windows render at physical resolution — no compositor upscaling.
            if flags & WIN_FLAG_DPI_AWARE != 0 {
                layer.dpi_aware = true;
            }
        }

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from("Window"),
            x,
            y,
            content_width: content_w,
            content_height: content_h,
            flags,
            owner_tid: app_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id,
            shm_ptr,
            shm_width: content_w,
            shm_height: content_h,
            needs_frame_ack: false,
            modal_owner: 0,
            fullscreen: false,
            fullscreen_capable: false,
            saved_bounds_fs: None,
            saved_flags_fs: 0,
            shortcut_slot: 0,
        };

        self.windows.push(win);
        self.assign_fkey_slot(id);
        self.focus_window_no_render(id);
        println!(
            "compositor: create_window tid={} win={} size={}x{} flags={:#x} shm={}",
            app_tid, id, content_w, content_h, flags, shm_id
        );

        id
    }

    /// Copy SHM content into the window layer's content area.
    /// For VRAM-direct windows (shm_ptr is null), just marks the layer dirty.
    /// If `dirty_rect` is Some, only copies that region from SHM (partial present).
    pub fn present_ipc_window(
        &mut self,
        window_id: u32,
        dirty_rect: Option<crate::compositor::Rect>,
    ) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        // Mark for frame ACK — render thread will send EVT_FRAME_ACK after compositing
        self.windows[win_idx].needs_frame_ack = true;

        let layer_id = self.windows[win_idx].layer_id;

        // Make the layer visible on its first CMD_PRESENT regardless of path.
        // The layer starts hidden (visible=false) to avoid compositing transparent
        // pixels before the app renders its first frame.  Any CMD_PRESENT call is
        // the app saying "I have real pixels" — honour that immediately so the
        // window is never stuck invisible (e.g. in verbose mode where the first
        // present_rect might be clipped to zero by a race in the dirty-rect path).
        let was_hidden = if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
            let hidden = !layer.visible;
            layer.visible = true;
            hidden
        } else {
            false
        };

        // VRAM-direct windows: no pixel copy needed, just mark dirty + damage
        if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
            if layer.is_vram {
                let bounds = layer.damage_bounds();
                self.compositor.mark_layer_dirty(layer_id);
                self.compositor.add_damage(bounds);
                return;
            }
        }

        let shm_ptr = self.windows[win_idx].shm_ptr;
        if shm_ptr.is_null() {
            // Window not ready for pixel copy, but if it just became visible
            // we still need damage so the render thread composites it.
            if was_hidden {
                if let Some(layer) = self.compositor.get_layer(layer_id) {
                    self.compositor.add_damage(layer.damage_bounds());
                }
            }
            return;
        }

        let cw = self.windows[win_idx].content_width;
        let ch = self.windows[win_idx].content_height;
        let borderless = self.windows[win_idx].is_borderless();
        let content_y = if borderless { 0 } else { title_bar_height() };
        let scale_content = self.windows[win_idx].flags & WIN_FLAG_SCALE_CONTENT != 0;

        let shm_w = self.windows[win_idx].shm_width;
        let shm_h = self.windows[win_idx].shm_height;
        if shm_w == 0 || shm_h == 0 {
            return;
        }

        let needs_scale = scale_content && (shm_w != cw || shm_h != ch);

        // Compute copy bounds — either the dirty rect or the full content area
        let (copy_x, copy_y, copy_w, copy_h) = if let Some(ref dr) = dirty_rect {
            let rx = (dr.x.max(0) as u32).min(shm_w);
            let ry = (dr.y.max(0) as u32).min(shm_h);
            let rw = dr
                .width
                .min(shm_w.saturating_sub(rx))
                .min(cw.saturating_sub(rx));
            let rh = dr
                .height
                .min(shm_h.saturating_sub(ry))
                .min(ch.saturating_sub(ry));
            if rw == 0 || rh == 0 {
                return;
            }
            (rx, ry, rw, rh)
        } else {
            (0, 0, shm_w.min(cw), shm_h.min(ch))
        };

        // Make the layer visible on its first CMD_PRESENT.
        // It starts hidden (visible=false) to avoid compositing transparent pixels
        // before the app has rendered its first frame.
        if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
            layer.visible = true;
        }

        if was_hidden {
            println!(
                "compositor: first_present tid={} win={} size={}x{}",
                self.windows[win_idx].owner_tid,
                window_id,
                self.windows[win_idx].content_width,
                self.windows[win_idx].content_height
            );
        }

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;
            let src_count = (shm_w * shm_h) as usize;
            let src_slice = unsafe { core::slice::from_raw_parts(shm_ptr, src_count) };

            if needs_scale && dirty_rect.is_none() {
                // Scaled path using fixed-point stepping (no per-pixel division)
                let x_step = ((shm_w as u32) << 16) / cw.max(1);
                let y_step = ((shm_h as u32) << 16) / ch.max(1);
                let mut src_y_fp: u32 = 0;

                for dst_row in 0..ch {
                    let src_y = (src_y_fp >> 16).min(shm_h - 1);
                    let src_row_off = (src_y * shm_w) as usize;
                    let dst_off = ((content_y + dst_row) * stride) as usize;
                    let mut src_x_fp: u32 = 0;

                    // Content area is always opaque — no alpha check needed
                    for dst_col in 0..cw {
                        let src_x = (src_x_fp >> 16).min(shm_w - 1) as usize;
                        pixels[dst_off + dst_col as usize] = src_slice[src_row_off + src_x];
                        src_x_fp += x_step;
                    }

                    src_y_fp += y_step;
                }
            } else {
                // Non-scaled path with dirty rect support
                // Content area is always opaque — alpha transparency for
                // window chrome corners is handled during compositing.
                for row in 0..copy_h {
                    let src_off = ((copy_y + row) * shm_w + copy_x) as usize;
                    let dst_off = ((content_y + copy_y + row) * stride + copy_x) as usize;
                    let w = copy_w as usize;
                    let src_end = (src_off + w).min(src_slice.len());
                    let dst_end = (dst_off + w).min(pixels.len());
                    let safe_w = (src_end - src_off).min(dst_end - dst_off);
                    if safe_w > 0 {
                        pixels[dst_off..dst_off + safe_w]
                            .copy_from_slice(&src_slice[src_off..src_off + safe_w]);
                    }
                }
            }
        }

        self.compositor.mark_layer_dirty(layer_id);

        // Damage only the dirty region (offset by layer position + content_y)
        if let Some(ref dr) = dirty_rect {
            if let Some(layer) = self.compositor.get_layer(layer_id) {
                let lx = layer.x;
                let ly = layer.y;
                let screen_rect = crate::compositor::Rect::new(
                    lx + dr.x.max(0),
                    ly + content_y as i32 + dr.y.max(0),
                    dr.width,
                    dr.height,
                );
                self.compositor.add_damage(screen_rect);
            }
        } else if let Some(layer) = self.compositor.get_layer(layer_id) {
            let bounds = layer.damage_bounds();
            self.compositor.add_damage(bounds);
        }
    }

    // ── Shortcut Manager Overlay ──────────────────────────────────────

    /// Toggle the shortcut manager overlay on/off.
    pub(crate) fn toggle_shortcut_overlay(&mut self) {
        if self.shortcut_overlay_visible {
            self.close_shortcut_overlay();
        } else {
            self.open_shortcut_overlay();
        }
    }

    /// Open the shortcut manager overlay (centered on screen).
    fn open_shortcut_overlay(&mut self) {
        if self.shortcut_overlay_visible {
            return;
        }

        // Overlay size: 4 columns × 3 rows of shortcut cards
        let card_w = crate::desktop::theme::scale(160);
        let card_h = crate::desktop::theme::scale(100);
        let gap = crate::desktop::theme::scale(12);
        let padding = crate::desktop::theme::scale(20);
        let title_h = crate::desktop::theme::scale(36);
        let cols = 4u32;
        let rows = 3u32;
        let overlay_w = padding * 2 + cols * card_w + (cols - 1) * gap;
        let overlay_h = padding + title_h + rows * card_h + (rows - 1) * gap + padding;

        let ox = (self.screen_width as i32 - overlay_w as i32) / 2;
        let oy = (self.screen_height as i32 - overlay_h as i32) / 2;

        // Create or reuse the overlay layer
        if self.shortcut_overlay_layer != 0 {
            self.compositor.remove_layer(self.shortcut_overlay_layer);
        }
        let layer_id = self
            .compositor
            .add_layer(ox, oy, overlay_w, overlay_h, false);
        self.shortcut_overlay_layer = layer_id;
        self.shortcut_overlay_visible = true;

        // Select first occupied slot
        self.shortcut_overlay_selection = -1;
        for i in 0..12 {
            if self.fkey_slots[i] != 0 {
                self.shortcut_overlay_selection = i as i32;
                break;
            }
        }

        // Raise above everything
        self.compositor.raise_layer(layer_id);
        self.compositor.raise_layer(self.menubar_layer_id);

        self.render_shortcut_overlay();
    }

    /// Select next slot in the overlay (Tab).
    pub(crate) fn shortcut_overlay_select_next(&mut self) {
        if !self.shortcut_overlay_visible {
            return;
        }
        let mut sel = self.shortcut_overlay_selection;
        // Find next occupied slot (wrap around)
        for _ in 0..12 {
            sel = (sel + 1) % 12;
            if self.fkey_slots[sel as usize] != 0 {
                self.shortcut_overlay_selection = sel;
                self.render_shortcut_overlay();
                return;
            }
        }
        // No occupied slots — select first anyway
        self.shortcut_overlay_selection = 0;
        self.render_shortcut_overlay();
    }

    /// Select previous slot in the overlay (Shift+Tab).
    pub(crate) fn shortcut_overlay_select_prev(&mut self) {
        if !self.shortcut_overlay_visible {
            return;
        }
        let mut sel = self.shortcut_overlay_selection;
        for _ in 0..12 {
            sel = if sel <= 0 { 11 } else { sel - 1 };
            if self.fkey_slots[sel as usize] != 0 {
                self.shortcut_overlay_selection = sel;
                self.render_shortcut_overlay();
                return;
            }
        }
        self.shortcut_overlay_selection = 0;
        self.render_shortcut_overlay();
    }

    /// Close the shortcut manager overlay.
    pub(crate) fn close_shortcut_overlay(&mut self) {
        if !self.shortcut_overlay_visible {
            return;
        }
        if self.shortcut_overlay_layer != 0 {
            let lid = self.shortcut_overlay_layer;
            if let Some(layer) = self.compositor.get_layer(lid) {
                let bounds = layer.damage_bounds();
                self.compositor.add_damage(bounds);
            }
            self.compositor.remove_layer(lid);
            self.shortcut_overlay_layer = 0;
        }
        self.shortcut_overlay_visible = false;
    }

    /// Check if a screen point is inside the shortcut overlay bounds.
    pub(crate) fn is_point_in_shortcut_overlay(&self, mx: i32, my: i32) -> bool {
        let layer_id = self.shortcut_overlay_layer;
        if layer_id == 0 {
            return false;
        }
        if let Some(layer) = self.compositor.get_layer(layer_id) {
            let lx = mx - layer.x;
            let ly = my - layer.y;
            lx >= 0 && ly >= 0 && (lx as u32) < layer.width && (ly as u32) < layer.height
        } else {
            false
        }
    }

    /// Hit test the close button on a card. Returns Some(slot_index) if an X button was clicked.
    pub(crate) fn hit_test_shortcut_overlay_close(&self, mx: i32, my: i32) -> Option<usize> {
        let layer_id = self.shortcut_overlay_layer;
        if layer_id == 0 {
            return None;
        }
        let layer = self.compositor.get_layer(layer_id)?;
        let lx = mx - layer.x;
        let ly = my - layer.y;

        let card_w = crate::desktop::theme::scale(160);
        let card_h = crate::desktop::theme::scale(100);
        let gap = crate::desktop::theme::scale(12);
        let padding = crate::desktop::theme::scale(20);
        let title_h = crate::desktop::theme::scale(36);
        let cols = 4u32;
        let xbtn_sz = crate::desktop::theme::scale(18) as i32;

        for slot in 0..12usize {
            if self.fkey_slots[slot] == 0 {
                continue;
            }
            let col = (slot as u32) % cols;
            let row = (slot as u32) / cols;
            let cx = padding as i32 + (col * (card_w + gap)) as i32;
            let cy = (padding + title_h) as i32 + (row * (card_h + gap)) as i32;
            let xbtn_x = cx + card_w as i32 - xbtn_sz - crate::desktop::theme::scale_i32(4);
            let xbtn_y = cy + crate::desktop::theme::scale_i32(4);
            if lx >= xbtn_x && lx < xbtn_x + xbtn_sz && ly >= xbtn_y && ly < xbtn_y + xbtn_sz {
                return Some(slot);
            }
        }
        None
    }

    /// Hit test the shortcut overlay. Returns Some(slot_index) if a card was clicked.
    pub(crate) fn hit_test_shortcut_overlay(&self, mx: i32, my: i32) -> Option<usize> {
        let layer_id = self.shortcut_overlay_layer;
        if layer_id == 0 {
            return None;
        }
        let layer = self.compositor.get_layer(layer_id)?;
        let ox = layer.x;
        let oy = layer.y;
        let lx = mx - ox;
        let ly = my - oy;
        if lx < 0 || ly < 0 {
            return None;
        }

        let card_w = crate::desktop::theme::scale(160);
        let card_h = crate::desktop::theme::scale(100);
        let gap = crate::desktop::theme::scale(12);
        let padding = crate::desktop::theme::scale(20);
        let title_h = crate::desktop::theme::scale(36);
        let cols = 4u32;

        let overlay_w = padding * 2 + cols * card_w + (cols - 1) * gap;
        let rows = 3u32;
        let overlay_h = padding + title_h + rows * card_h + (rows - 1) * gap + padding;
        if lx as u32 >= overlay_w || ly as u32 >= overlay_h {
            return None;
        }

        // Check each card
        for slot in 0..12usize {
            let col = (slot as u32) % cols;
            let row = (slot as u32) / cols;
            let cx = padding as i32 + (col * (card_w + gap)) as i32;
            let cy = (padding + title_h) as i32 + (row * (card_h + gap)) as i32;
            if lx >= cx && lx < cx + card_w as i32 && ly >= cy && ly < cy + card_h as i32 {
                return Some(slot);
            }
        }
        // Clicked inside overlay but not on a card — still inside overlay
        // Return None to indicate "not a slot" but the caller should check bounds separately
        None
    }

    /// Render the shortcut overlay contents.
    pub(crate) fn render_shortcut_overlay(&mut self) {
        let layer_id = self.shortcut_overlay_layer;
        if layer_id == 0 {
            return;
        }

        let card_w = crate::desktop::theme::scale(160);
        let card_h = crate::desktop::theme::scale(100);
        let gap = crate::desktop::theme::scale(12);
        let padding = crate::desktop::theme::scale(20);
        let title_h = crate::desktop::theme::scale(36);
        let cols = 4u32;
        let rows = 3u32;
        let overlay_w = padding * 2 + cols * card_w + (cols - 1) * gap;
        let overlay_h = padding + title_h + rows * card_h + (rows - 1) * gap + padding;

        // Pre-compute multi-monitor data BEFORE layer_pixels takes a
        // mutable borrow on self.compositor.
        let multi_monitor = self.compositor.outputs.len() >= 2;
        let mut slot_output_id_pre: [u8; 12] = [0; 12];
        if multi_monitor {
            for slot in 0..12usize {
                let wid = self.fkey_slots[slot];
                if wid == 0 {
                    continue;
                }
                if let Some(win) = self.windows.iter().find(|w| w.id == wid) {
                    let cx_w = win.x + (win.content_width as i32 / 2);
                    let cy_w = win.y;
                    slot_output_id_pre[slot] = self.compositor.output_at(cx_w, cy_w).id as u8;
                }
            }
        }

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = overlay_w;

            // Background: dark semi-transparent rounded rect
            for p in pixels.iter_mut() {
                *p = 0x00000000;
            }
            let bg_color = if super::theme::is_light() {
                0xE8F0F0F5
            } else {
                0xE8202025
            };
            let border_color = if super::theme::is_light() {
                0xFFD1D1D6
            } else {
                0xFF4A4A4E
            };
            fill_rounded_rect(
                pixels, stride, overlay_h, 0, 0, overlay_w, overlay_h, 12, bg_color,
            );
            draw_rounded_rect_outline(
                pixels,
                stride,
                overlay_h,
                0,
                0,
                overlay_w,
                overlay_h,
                12,
                border_color,
            );

            // Title
            let title_text = "Fenster-Shortcuts (Strg+F1..F12)";
            let fs = scaled_font_size();
            let text_color = color_titlebar_text();
            let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, title_text);
            let tx = (overlay_w as i32 - tw as i32) / 2;
            let ty = (padding as i32 + (title_h as i32 - th as i32) / 2).max(padding as i32);
            anyos_std::ui::window::font_render_buf(
                FONT_ID, fs, pixels, stride, overlay_h, tx, ty, text_color, title_text,
            );

            // Collect window titles for each slot (stack-copy to avoid borrow issues)
            let mut slot_titles: [[u8; 64]; 12] = [[0u8; 64]; 12];
            let mut slot_title_lens: [usize; 12] = [0; 12];
            let mut slot_has_window: [bool; 12] = [false; 12];

            // We need to read SHM pointers for thumbnails
            let mut slot_shm_info: [(u32, u32, *const u32); 12] = [(0, 0, core::ptr::null()); 12];
            // Multi-monitor: per-slot, which output the window currently
            // sits on. Pre-computed above to avoid borrow conflict.
            let slot_output_id = slot_output_id_pre;

            for slot in 0..12usize {
                let wid = self.fkey_slots[slot];
                if wid == 0 {
                    continue;
                }
                if let Some(win) = self.windows.iter().find(|w| w.id == wid) {
                    slot_has_window[slot] = true;
                    let tlen = win.title.len().min(63);
                    slot_titles[slot][..tlen].copy_from_slice(&win.title.as_bytes()[..tlen]);
                    slot_title_lens[slot] = tlen;
                    if !win.shm_ptr.is_null() && win.shm_width > 0 && win.shm_height > 0 {
                        slot_shm_info[slot] =
                            (win.shm_width, win.shm_height, win.shm_ptr as *const u32);
                    }
                }
            }

            // Draw 12 cards in 4×3 grid
            let card_bg = if super::theme::is_light() {
                0xFFE8E8EC
            } else {
                0xFF2C2C30
            };
            let card_active_bg = if super::theme::is_light() {
                0xFFD0D0D8
            } else {
                0xFF383840
            };
            let card_selected_bg = if super::theme::is_light() {
                0xFFC0D0F0
            } else {
                0xFF2A3A60
            };
            let card_selected_border = if super::theme::is_light() {
                0xFF007AFF
            } else {
                0xFF0A84FF
            };
            let selection = self.shortcut_overlay_selection;
            let card_border = if super::theme::is_light() {
                0xFFC0C0C8
            } else {
                0xFF505058
            };
            let label_fs = crate::desktop::theme::scale_font(10);
            let title_fs = crate::desktop::theme::scale_font(9);
            let badge_h = crate::desktop::theme::scale(20);
            let thumb_top = badge_h + crate::desktop::theme::scale(4);
            let thumb_area_h = card_h.saturating_sub(thumb_top + crate::desktop::theme::scale(4));

            for slot in 0..12usize {
                let col = (slot as u32) % cols;
                let row = (slot as u32) / cols;
                let cx = padding as i32 + (col * (card_w + gap)) as i32;
                let cy = (padding + title_h) as i32 + (row * (card_h + gap)) as i32;

                let is_selected = selection == slot as i32;
                let bg = if is_selected {
                    card_selected_bg
                } else if slot_has_window[slot] {
                    card_active_bg
                } else {
                    card_bg
                };
                let border = if is_selected {
                    card_selected_border
                } else {
                    card_border
                };
                fill_rounded_rect(pixels, stride, overlay_h, cx, cy, card_w, card_h, 6, bg);
                draw_rounded_rect_outline(
                    pixels, stride, overlay_h, cx, cy, card_w, card_h, 6, border,
                );
                // Double border for selected card
                if is_selected {
                    draw_rounded_rect_outline(
                        pixels,
                        stride,
                        overlay_h,
                        cx + 1,
                        cy + 1,
                        card_w - 2,
                        card_h - 2,
                        5,
                        border,
                    );
                }

                // F-key label badge
                let mut lbl = [0u8; 4];
                let lbl_str = if slot < 9 {
                    lbl[0] = b'F';
                    lbl[1] = b'1' + slot as u8;
                    core::str::from_utf8(&lbl[..2]).unwrap_or("")
                } else {
                    lbl[0] = b'F';
                    lbl[1] = b'1';
                    lbl[2] = b'0' + (slot as u8 - 9);
                    core::str::from_utf8(&lbl[..3]).unwrap_or("")
                };
                let badge_color = if super::theme::is_light() {
                    0xFF007AFF
                } else {
                    0xFF0A84FF
                };
                let badge_w = crate::desktop::theme::scale(32);
                let bx = cx + crate::desktop::theme::scale_i32(6);
                let by = cy + crate::desktop::theme::scale_i32(4);
                fill_rounded_rect(
                    pixels,
                    stride,
                    overlay_h,
                    bx,
                    by,
                    badge_w,
                    badge_h,
                    4,
                    badge_color,
                );
                let (lw, lh) = anyos_std::ui::window::font_measure(FONT_ID, label_fs, lbl_str);
                let ltx = bx + (badge_w as i32 - lw as i32) / 2;
                let lty = by + (badge_h as i32 - lh as i32) / 2;
                anyos_std::ui::window::font_render_buf(
                    FONT_ID, label_fs, pixels, stride, overlay_h, ltx, lty, 0xFFFFFFFF, lbl_str,
                );

                // Multi-monitor: per-card "M{output_id}" badge directly
                // after the F-key badge, with an output-specific colour
                // so users can tell at a glance which monitor a window
                // currently lives on. Right-click on the card cycles
                // the window through the other outputs (handled in
                // input.rs).
                let monitor_badge_w = if multi_monitor && slot_has_window[slot] {
                    let mb_w = crate::desktop::theme::scale(28);
                    let mb_x = bx + badge_w as i32 + crate::desktop::theme::scale_i32(4);
                    // Distinct colour per output id for quick scanning.
                    let oid = slot_output_id[slot];
                    let mb_bg = match oid {
                        0 => 0xFF0A84FF, // primary — blue
                        1 => 0xFF34C759, // 2nd — green
                        2 => 0xFFFF9500, // 3rd — orange
                        3 => 0xFFAF52DE, // 4th — purple
                        _ => 0xFF8E8E93, // 5th+ — neutral grey
                    };
                    fill_rounded_rect(pixels, stride, overlay_h, mb_x, by, mb_w, badge_h, 4, mb_bg);
                    let mut mlbl = [0u8; 4];
                    mlbl[0] = b'M';
                    if oid < 10 {
                        mlbl[1] = b'0' + oid;
                    } else {
                        mlbl[1] = b'0' + (oid % 10);
                    }
                    let mlbl_str = core::str::from_utf8(&mlbl[..2]).unwrap_or("");
                    let (mlw, mlh) =
                        anyos_std::ui::window::font_measure(FONT_ID, label_fs, mlbl_str);
                    let mltx = mb_x + (mb_w as i32 - mlw as i32) / 2;
                    let mlty = by + (badge_h as i32 - mlh as i32) / 2;
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID, label_fs, pixels, stride, overlay_h, mltx, mlty, 0xFFFFFFFF,
                        mlbl_str,
                    );
                    mb_w + crate::desktop::theme::scale(4)
                } else {
                    0
                };

                // Close button (X) — top-right of card, only for occupied slots
                if slot_has_window[slot] {
                    let xbtn_sz = crate::desktop::theme::scale(18);
                    let xbtn_x =
                        cx + card_w as i32 - xbtn_sz as i32 - crate::desktop::theme::scale_i32(4);
                    let xbtn_y = cy + crate::desktop::theme::scale_i32(4);
                    let xbtn_bg = if super::theme::is_light() {
                        0x40000000
                    } else {
                        0x40FFFFFF
                    };
                    fill_rounded_rect(
                        pixels, stride, overlay_h, xbtn_x, xbtn_y, xbtn_sz, xbtn_sz, 4, xbtn_bg,
                    );
                    // Draw "×" character
                    let xfs = crate::desktop::theme::scale_font(11);
                    let xstr = "\u{00D7}"; // ×
                    let (xw, xh) = anyos_std::ui::window::font_measure(FONT_ID, xfs, xstr);
                    let xtx = xbtn_x + (xbtn_sz as i32 - xw as i32) / 2;
                    let xty = xbtn_y + (xbtn_sz as i32 - xh as i32) / 2;
                    let xcolor = if super::theme::is_light() {
                        0xFF666666
                    } else {
                        0xFFAAAAAA
                    };
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID, xfs, pixels, stride, overlay_h, xtx, xty, xcolor, xstr,
                    );
                }

                // Window title (next to badge — shifted by monitor
                // badge width when present)
                if slot_has_window[slot] {
                    let tstr = core::str::from_utf8(&slot_titles[slot][..slot_title_lens[slot]])
                        .unwrap_or("");
                    let max_tw = card_w.saturating_sub(
                        badge_w + monitor_badge_w + crate::desktop::theme::scale(18),
                    );
                    let tlen = title_display_len(tstr, max_tw);
                    let display = if tlen < tstr.len() && tlen > 0 {
                        &tstr[..tlen]
                    } else {
                        tstr
                    };
                    if !display.is_empty() {
                        let ttx = bx
                            + badge_w as i32
                            + monitor_badge_w as i32
                            + crate::desktop::theme::scale_i32(6);
                        let (_, tth) =
                            anyos_std::ui::window::font_measure(FONT_ID, title_fs, display);
                        let tty = by + (badge_h as i32 - tth as i32) / 2;
                        anyos_std::ui::window::font_render_buf(
                            FONT_ID, title_fs, pixels, stride, overlay_h, ttx, tty, text_color,
                            display,
                        );
                    }

                    // Thumbnail: scale down the SHM content
                    let (sw, sh, sptr) = slot_shm_info[slot];
                    if !sptr.is_null() && sw > 0 && sh > 0 {
                        let thumb_w = card_w.saturating_sub(crate::desktop::theme::scale(12));
                        // Maintain aspect ratio
                        let scale_x = (thumb_w * 1000) / sw.max(1);
                        let scale_y = (thumb_area_h * 1000) / sh.max(1);
                        let s = scale_x.min(scale_y);
                        let tw_px = (sw * s / 1000).min(thumb_w);
                        let th_px = (sh * s / 1000).min(thumb_area_h);
                        if tw_px > 0 && th_px > 0 {
                            let tx = cx + (card_w as i32 - tw_px as i32) / 2;
                            let ty = cy + thumb_top as i32;
                            // Simple nearest-neighbor scale blit
                            let src =
                                unsafe { core::slice::from_raw_parts(sptr, (sw * sh) as usize) };
                            for dy in 0..th_px {
                                let src_y = (dy * sh / th_px).min(sh - 1);
                                for dx in 0..tw_px {
                                    let src_x = (dx * sw / tw_px).min(sw - 1);
                                    let px_x = tx + dx as i32;
                                    let px_y = ty + dy as i32;
                                    if px_x >= 0 && px_y >= 0 {
                                        let di = px_y as u32 * stride + px_x as u32;
                                        let si = src_y * sw + src_x;
                                        if (di as usize) < pixels.len() && (si as usize) < src.len()
                                        {
                                            pixels[di as usize] = src[si as usize] | 0xFF000000;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Empty slot: show "—"
                    let empty = "\u{2014}"; // em dash
                    let (ew, eh) = anyos_std::ui::window::font_measure(FONT_ID, fs, empty);
                    let ex = cx + (card_w as i32 - ew as i32) / 2;
                    let ey = cy + (card_h as i32 - eh as i32) / 2 + badge_h as i32 / 2;
                    let dim_color = if super::theme::is_light() {
                        0xFF999999
                    } else {
                        0xFF666666
                    };
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID, fs, pixels, stride, overlay_h, ex, ey, dim_color, empty,
                    );
                }
            }
        }

        self.compositor.mark_layer_dirty(layer_id);
        if let Some(layer) = self.compositor.get_layer(layer_id) {
            let bounds = layer.damage_bounds();
            self.compositor.add_damage(bounds);
        }
    }
}

// ── Pre-render Chrome (called OUTSIDE lock) ────────────────────────────────

/// Pre-render window chrome (title bar, buttons, body) into a pixel buffer.
pub fn pre_render_chrome(pixels: &mut [u32], stride: u32, full_h: u32, title: &str, focused: bool) {
    pre_render_chrome_ex(pixels, stride, full_h, title, focused, 0);
}

/// Pre-render window chrome with flags (hides disabled buttons).
pub fn pre_render_chrome_ex(
    pixels: &mut [u32],
    stride: u32,
    full_h: u32,
    title: &str,
    focused: bool,
    flags: u32,
) {
    for p in pixels.iter_mut() {
        *p = 0x00000000;
    }

    fill_rounded_rect(
        pixels,
        stride,
        full_h,
        0,
        0,
        stride,
        full_h,
        8,
        color_window_bg(),
    );
    draw_rounded_rect_outline(
        pixels,
        stride,
        full_h,
        0,
        0,
        stride,
        full_h,
        8,
        color_window_border(),
    );

    let tb_h = title_bar_height();
    let (tb_top, tb_bot) = if focused {
        (
            color_titlebar_focused_top(),
            color_titlebar_focused_bottom(),
        )
    } else {
        (
            color_titlebar_unfocused_top(),
            color_titlebar_unfocused_bottom(),
        )
    };
    fill_rounded_rect_top_gradient(pixels, stride, 0, 0, stride, tb_h, 8, tb_top, tb_bot);

    let border_y = tb_h - 1;
    for x in 0..stride {
        let idx = (border_y * stride + x) as usize;
        if idx < pixels.len() {
            pixels[idx] = color_window_border();
        }
    }

    let btn_hidden = [
        flags & WIN_FLAG_NO_CLOSE != 0,
        flags & WIN_FLAG_NO_MINIMIZE != 0,
        flags & WIN_FLAG_NO_MAXIMIZE != 0,
    ];
    let base_colors: [u32; 3] = if focused {
        [COLOR_CLOSE_BTN, COLOR_MIN_BTN, COLOR_MAX_BTN]
    } else {
        [
            color_btn_unfocused(),
            color_btn_unfocused(),
            color_btn_unfocused(),
        ]
    };
    let btn_sz = title_btn_size();
    let btn_sp = title_btn_spacing();
    let btn_y_pos = title_btn_y();
    let btn_left = crate::desktop::theme::scale_i32(8);
    for (i, &color) in base_colors.iter().enumerate() {
        if btn_hidden[i] {
            continue;
        }
        let cx = btn_left + i as i32 * btn_sp as i32 + btn_sz as i32 / 2;
        let cy = btn_y_pos as i32 + btn_sz as i32 / 2;
        fill_circle(pixels, stride, full_h, cx, cy, (btn_sz / 2) as i32, color);
    }

    let cw = stride;
    let left_bound = title_buttons_right() + title_padding();
    let max_title_w = if (cw as i32) > left_bound + title_padding() {
        (cw as i32 - left_bound - title_padding()) as u32
    } else {
        0
    };

    let trunc_len = title_display_len(title, max_title_w);
    let mut display_buf = [0u8; 260];
    let display_str = if trunc_len < title.len() && trunc_len > 0 {
        let total = trunc_len + 3;
        display_buf[..trunc_len].copy_from_slice(&title.as_bytes()[..trunc_len]);
        display_buf[trunc_len..trunc_len + 3].copy_from_slice(b"...");
        core::str::from_utf8(&display_buf[..total]).unwrap_or(title)
    } else if trunc_len == 0 {
        ""
    } else {
        title
    };

    if !display_str.is_empty() {
        let fs = scaled_font_size();
        let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, display_str);
        let mut tx = (cw as i32 - tw as i32) / 2;
        if tx < left_bound {
            tx = left_bound;
        }
        let ty = ((tb_h as i32 - th as i32) / 2).max(0);
        anyos_std::ui::window::font_render_buf(
            FONT_ID,
            fs,
            pixels,
            stride,
            full_h,
            tx,
            ty,
            color_titlebar_text(),
            display_str,
        );
    }
}

/// Copy SHM content into a pre-rendered pixel buffer at the content area offset.
pub fn copy_shm_to_pixels(
    pixels: &mut [u32],
    stride: u32,
    content_y: u32,
    shm_ptr: *const u32,
    shm_w: u32,
    shm_h: u32,
) {
    if shm_ptr.is_null() || shm_w == 0 || shm_h == 0 {
        return;
    }
    let src_count = (shm_w * shm_h) as usize;
    let src_slice = unsafe { core::slice::from_raw_parts(shm_ptr, src_count) };
    let copy_w = shm_w.min(stride) as usize;
    for row in 0..shm_h {
        let src_off = (row * shm_w) as usize;
        let dst_off = ((content_y + row) * stride) as usize;
        let src_end = (src_off + copy_w).min(src_slice.len());
        let dst_end = (dst_off + copy_w).min(pixels.len());
        let safe_w = (src_end - src_off).min(dst_end - dst_off);
        if safe_w > 0 {
            pixels[dst_off..dst_off + safe_w]
                .copy_from_slice(&src_slice[src_off..src_off + safe_w]);
        }
    }
}
