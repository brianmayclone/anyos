//! Window management — WindowInfo, HitTest, create/destroy/focus/render, chrome.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

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

// ── Dimensions ─────────────────────────────────────────────────────────────

pub const MENUBAR_HEIGHT: u32 = 24;
pub const TITLE_BAR_HEIGHT: u32 = 28;

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

// ── Hit Test ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum HitTest {
    None,
    TitleBar,
    CloseButton,
    MinButton,
    MaxButton,
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

    /// Full window width (same as content for borderless).
    pub fn full_width(&self) -> u32 {
        self.content_width
    }

    /// Full window height (content + title bar for decorated windows).
    pub fn full_height(&self) -> u32 {
        if self.is_borderless() {
            self.content_height
        } else {
            self.content_height + TITLE_BAR_HEIGHT
        }
    }

    /// Hit test a point (in screen coordinates) against the window.
    pub fn hit_test(&self, px: i32, py: i32) -> HitTest {
        let wx = px - self.x;
        let wy = py - self.y;
        let fw = self.full_width() as i32;
        let fh = self.full_height() as i32;

        if wx < 0 || wy < 0 || wx >= fw || wy >= fh {
            return HitTest::None;
        }

        if self.is_borderless() {
            return HitTest::Content;
        }

        // Resize edges (4px border)
        if self.is_resizable() && !self.maximized {
            let edge = 4;
            let top = wy < edge;
            let bottom = wy >= fh - edge;
            let left = wx < edge;
            let right = wx >= fw - edge;

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

        // Title bar
        if wy < TITLE_BAR_HEIGHT as i32 {
            let btn_y = TITLE_BTN_Y as i32;
            let btn_r = (TITLE_BTN_SIZE / 2) as i32;
            if wy >= btn_y && wy < btn_y + TITLE_BTN_SIZE as i32 {
                // Close button at x=8
                let cx = 8 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::CloseButton;
                }
                // Minimize at x=28
                let cx = 28 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::MinButton;
                }
                // Maximize at x=48
                let cx = 48 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::MaxButton;
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
            content_h + TITLE_BAR_HEIGHT
        };

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        let opaque = borderless && !force_shadow;
        let layer_id = self.compositor.add_layer(x, y, content_w, full_h, opaque);

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
        };

        self.windows.push(win);
        self.focus_window(id);

        id
    }

    /// Destroy a window.
    pub fn destroy_window(&mut self, id: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            let layer_id = self.windows[idx].layer_id;
            self.compositor.remove_layer(layer_id);
            self.windows.remove(idx);

            self.menu_bar.remove_menu(id);

            if self.focused_window == Some(id) {
                self.focused_window = None;
                self.compositor.set_focused_layer(None);
                if let Some(last) = self.windows.last() {
                    let next_id = last.id;
                    self.focus_window(next_id);
                } else {
                    if self.menu_bar.on_focus_change(None) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
            }
        }
    }

    /// Destroy all windows owned by a given thread (process exit cleanup).
    pub fn on_process_exit(&mut self, tid: u32) {
        let window_ids: Vec<u32> = self.windows.iter()
            .filter(|w| w.owner_tid == tid)
            .map(|w| w.id)
            .collect();
        for id in window_ids {
            self.destroy_window(id);
        }
        self.app_subs.retain(|(t, _)| *t != tid);
    }

    /// Called when system theme changes — re-render all window chrome and menubar.
    pub fn on_theme_change(&mut self) {
        self.draw_menubar();

        let win_ids: Vec<u32> = self.windows.iter()
            .filter(|w| !w.is_borderless())
            .map(|w| w.id)
            .collect();
        for id in win_ids {
            self.render_window(id);
        }

        self.compositor.damage_all();
    }

    /// Focus a window (bring to front and set focused style).
    pub fn focus_window(&mut self, id: u32) {
        if let Some(old_id) = self.focused_window {
            if old_id != id {
                if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                    self.windows[idx].focused = false;
                    let win_id = self.windows[idx].id;
                    self.render_titlebar(win_id);
                }
            }
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            self.windows[idx].focused = true;
            self.focused_window = Some(id);
            let layer_id = self.windows[idx].layer_id;
            self.compositor.set_focused_layer(Some(layer_id));
            self.compositor.raise_layer(layer_id);

            let win = self.windows.remove(idx);
            self.windows.push(win);

            self.ensure_top_layers();
            self.render_window(id);

            if self.menu_bar.on_focus_change(Some(id)) {
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }
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
                }
            }
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            self.windows[idx].focused = true;
            self.focused_window = Some(id);
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
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }
        }
    }

    /// Re-raise always-on-top windows and the menubar.
    pub(crate) fn ensure_top_layers(&mut self) {
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
        // Stack-copy title to avoid heap allocation (title.clone())
        let mut title_buf = [0u8; 128];
        let title_len = self.windows[win_idx].title.len().min(128);
        title_buf[..title_len].copy_from_slice(&self.windows[win_idx].title.as_bytes()[..title_len]);
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
                    pixels, stride, full_h, 0, 0, cw, full_h, 8, color_window_bg(),
                );

                draw_rounded_rect_outline(
                    pixels, stride, full_h, 0, 0, cw, full_h, 8, color_window_border(),
                );

                let (tb_top, tb_bot) = if focused {
                    (color_titlebar_focused_top(), color_titlebar_focused_bottom())
                } else {
                    (color_titlebar_unfocused_top(), color_titlebar_unfocused_bottom())
                };
                fill_rounded_rect_top_gradient(pixels, stride, 0, 0, cw, TITLE_BAR_HEIGHT, 8, tb_top, tb_bot);

                let border_y = TITLE_BAR_HEIGHT - 1;
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
                    [color_btn_unfocused(), color_btn_unfocused(), color_btn_unfocused()]
                };
                for (i, &base) in base_colors.iter().enumerate() {
                    if btn_hidden[i] { continue; }
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
                    let cx = 8 + i as i32 * TITLE_BTN_SPACING as i32 + TITLE_BTN_SIZE as i32 / 2;
                    let cy = TITLE_BTN_Y as i32 + TITLE_BTN_SIZE as i32 / 2;
                    fill_circle(pixels, stride, full_h, cx, cy, (TITLE_BTN_SIZE / 2) as i32, color);
                }

                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, title_str);
                let tx = (cw as i32 - tw as i32) / 2;
                let ty = ((TITLE_BAR_HEIGHT as i32 - th as i32) / 2).max(0);
                anyos_std::ui::window::font_render_buf(
                    FONT_ID, FONT_SIZE, pixels, stride, full_h, tx, ty,
                    color_titlebar_text(), title_str,
                );
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
        // Stack-copy title to avoid heap allocation (title.clone())
        let mut title_buf = [0u8; 128];
        let title_len = self.windows[win_idx].title.len().min(128);
        title_buf[..title_len].copy_from_slice(&self.windows[win_idx].title.as_bytes()[..title_len]);
        let title_str = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("");

        if self.windows[win_idx].is_borderless() {
            return;
        }

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;

            let (tb_top, tb_bot) = if focused {
                (color_titlebar_focused_top(), color_titlebar_focused_bottom())
            } else {
                (color_titlebar_unfocused_top(), color_titlebar_unfocused_bottom())
            };
            fill_rounded_rect_top_gradient(pixels, stride, 0, 0, cw, TITLE_BAR_HEIGHT, 8, tb_top, tb_bot);

            let border_y = TITLE_BAR_HEIGHT - 1;
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
                [color_btn_unfocused(), color_btn_unfocused(), color_btn_unfocused()]
            };
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
                let cx = 8 + i as i32 * TITLE_BTN_SPACING as i32 + TITLE_BTN_SIZE as i32 / 2;
                let cy = TITLE_BTN_Y as i32 + TITLE_BTN_SIZE as i32 / 2;
                fill_circle(pixels, stride, full_h, cx, cy, (TITLE_BTN_SIZE / 2) as i32, color);
            }

            let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, title_str);
            let tx = (cw as i32 - tw as i32) / 2;
            let ty = ((TITLE_BAR_HEIGHT as i32 - th as i32) / 2).max(0);
            anyos_std::ui::window::font_render_buf(
                FONT_ID, FONT_SIZE, pixels, stride, full_h, tx, ty,
                color_titlebar_text(), title_str,
            );
        }

        self.compositor.mark_layer_dirty(layer_id);
    }

    /// Toggle window maximize/restore.
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
                }
            } else {
                let x = self.windows[idx].x;
                let y = self.windows[idx].y;
                let cw = self.windows[idx].content_width;
                let ch = self.windows[idx].content_height;
                self.windows[idx].saved_bounds = Some((x, y, cw, ch));
                self.windows[idx].maximized = true;

                let new_x = 0i32;
                let new_y = MENUBAR_HEIGHT as i32 + 1;
                let new_w = self.screen_width;
                let new_ch = self.screen_height - MENUBAR_HEIGHT - 1 - TITLE_BAR_HEIGHT;

                let layer_id = self.windows[idx].layer_id;
                self.windows[idx].x = new_x;
                self.windows[idx].y = new_y;
                self.windows[idx].content_width = new_w;
                self.windows[idx].content_height = new_ch;

                let full_h = self.windows[idx].full_height();
                self.compositor.move_layer(layer_id, new_x, new_y);
                self.compositor.resize_layer(layer_id, new_w, full_h);
                self.render_window(win_id);
            }
        }
    }

    // ── IPC Window Operations ──────────────────────────────────────────

    /// Compute the next auto-placement position (cascading).
    fn next_auto_position(&mut self, win_w: u32, win_h: u32) -> (i32, i32) {
        let x = self.cascade_x;
        let y = self.cascade_y;

        // Advance cascade for next window
        self.cascade_x += 30;
        self.cascade_y += 30;

        // Wrap horizontally
        if self.cascade_x + win_w as i32 > self.screen_width as i32 - 60 {
            self.cascade_x = 120;
            self.cascade_y += 30;
        }
        // Wrap vertically
        if self.cascade_y + win_h as i32 > self.screen_height as i32 - 80 {
            self.cascade_x = 120;
            self.cascade_y = MENUBAR_HEIGHT as i32 + 50;
        }

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
            content_h + TITLE_BAR_HEIGHT
        };

        // Determine position: explicit or auto-placed
        let (x, y) = if raw_x == crate::ipc_protocol::CW_USEDEFAULT
            || raw_y == crate::ipc_protocol::CW_USEDEFAULT
        {
            self.next_auto_position(content_w, full_h)
        } else {
            (raw_x as i32, raw_y as i32)
        };

        let layer_id = self.compositor.add_layer(
            x,
            y,
            content_w,
            full_h,
            false,
        );

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        if !borderless || force_shadow {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.has_shadow = true;
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
        };

        self.windows.push(win);
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
        // Determine position: explicit or auto-placed
        let (x, y) = if raw_x == crate::ipc_protocol::CW_USEDEFAULT
            || raw_y == crate::ipc_protocol::CW_USEDEFAULT
        {
            self.next_auto_position(content_w, content_h)
        } else {
            (raw_x as i32, raw_y as i32)
        };

        // VRAM windows are always borderless + opaque (no title bar chrome)
        let layer_id = self.compositor.add_vram_layer(
            x,
            y,
            content_w,
            content_h,
        )?;

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
        };

        self.windows.push(win);
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
            content_h + TITLE_BAR_HEIGHT
        };

        // Determine position: explicit or auto-placed
        let (x, y) = if raw_x == crate::ipc_protocol::CW_USEDEFAULT
            || raw_y == crate::ipc_protocol::CW_USEDEFAULT
        {
            self.next_auto_position(content_w, full_h)
        } else {
            (raw_x as i32, raw_y as i32)
        };

        let layer_id = self.compositor.add_layer_with_pixels(
            x,
            y,
            content_w,
            full_h,
            false,
            pre_pixels,
        );

        let force_shadow = flags & WIN_FLAG_SHADOW != 0;
        if !borderless || force_shadow {
            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                layer.has_shadow = true;
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
        };

        self.windows.push(win);
        self.focus_window_no_render(id);

        id
    }

    /// Copy SHM content into the window layer's content area.
    /// For VRAM-direct windows (shm_ptr is null), just marks the layer dirty.
    /// If `dirty_rect` is Some, only copies that region from SHM (partial present).
    pub fn present_ipc_window(&mut self, window_id: u32, dirty_rect: Option<crate::compositor::Rect>) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        let layer_id = self.windows[win_idx].layer_id;

        // VRAM-direct windows: no pixel copy needed, just mark dirty + damage
        if let Some(layer) = self.compositor.get_layer(layer_id) {
            if layer.is_vram {
                let bounds = layer.damage_bounds();
                self.compositor.mark_layer_dirty(layer_id);
                self.compositor.add_damage(bounds);
                return;
            }
        }

        let shm_ptr = self.windows[win_idx].shm_ptr;
        if shm_ptr.is_null() {
            return;
        }

        let cw = self.windows[win_idx].content_width;
        let ch = self.windows[win_idx].content_height;
        let borderless = self.windows[win_idx].is_borderless();
        let content_y = if borderless { 0 } else { TITLE_BAR_HEIGHT };
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
            let rw = dr.width.min(shm_w.saturating_sub(rx)).min(cw.saturating_sub(rx));
            let rh = dr.height.min(shm_h.saturating_sub(ry)).min(ch.saturating_sub(ry));
            if rw == 0 || rh == 0 {
                return;
            }
            (rx, ry, rw, rh)
        } else {
            (0, 0, shm_w.min(cw), shm_h.min(ch))
        };

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
                        pixels[dst_off + dst_col as usize] =
                            src_slice[src_row_off + src_x];
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
}

// ── Pre-render Chrome (called OUTSIDE lock) ────────────────────────────────

/// Pre-render window chrome (title bar, buttons, body) into a pixel buffer.
pub fn pre_render_chrome(
    pixels: &mut [u32],
    stride: u32,
    full_h: u32,
    title: &str,
    focused: bool,
) {
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

    fill_rounded_rect(pixels, stride, full_h, 0, 0, stride, full_h, 8, color_window_bg());
    draw_rounded_rect_outline(pixels, stride, full_h, 0, 0, stride, full_h, 8, color_window_border());

    let (tb_top, tb_bot) = if focused {
        (color_titlebar_focused_top(), color_titlebar_focused_bottom())
    } else {
        (color_titlebar_unfocused_top(), color_titlebar_unfocused_bottom())
    };
    fill_rounded_rect_top_gradient(pixels, stride, 0, 0, stride, TITLE_BAR_HEIGHT, 8, tb_top, tb_bot);

    let border_y = TITLE_BAR_HEIGHT - 1;
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
        [color_btn_unfocused(), color_btn_unfocused(), color_btn_unfocused()]
    };
    for (i, &color) in base_colors.iter().enumerate() {
        if btn_hidden[i] { continue; }
        let cx = 8 + i as i32 * TITLE_BTN_SPACING as i32 + TITLE_BTN_SIZE as i32 / 2;
        let cy = TITLE_BTN_Y as i32 + TITLE_BTN_SIZE as i32 / 2;
        fill_circle(pixels, stride, full_h, cx, cy, (TITLE_BTN_SIZE / 2) as i32, color);
    }

    let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, title);
    let tx = (stride as i32 - tw as i32) / 2;
    let ty = ((TITLE_BAR_HEIGHT as i32 - th as i32) / 2).max(0);
    anyos_std::ui::window::font_render_buf(
        FONT_ID, FONT_SIZE, pixels, stride, full_h, tx, ty,
        color_titlebar_text(), title,
    );
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
