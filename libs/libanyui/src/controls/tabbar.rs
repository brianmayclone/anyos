use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};
use alloc::vec::Vec;

const TAB_PAD_X: i32 = 14;
const CLOSE_BTN_SIZE: i32 = 16;
const CLOSE_BTN_PAD: i32 = 8;
const TAB_HEIGHT: i32 = 28;
const TAB_FONT_SIZE: u16 = 12;
const TAB_GAP: i32 = 4;
const TAB_TOP_MARGIN: i32 = 4;
const MAX_TAB_WIDTH: i32 = 200;
const MIN_TAB_WIDTH: i32 = 80;
/// Size of the navigation / "+" buttons on the right side.
const NAV_BTN_SIZE: i32 = 24;

pub struct TabBar {
    pub(crate) text_base: TextControlBase,
    /// Cached tab labels parsed from pipe-separated text.
    labels: Vec<Vec<u8>>,
    /// Which tab is hovered (-1 = none).
    hover_tab: i32,
    /// Whether the close button on the hovered tab is hovered.
    close_hovered: bool,
    /// First visible tab index when in overflow mode.
    scroll_offset: usize,
    /// Hover state for right-side buttons: -1=none, 0=scroll-left, 1=scroll-right, 2=plus
    nav_hover: i32,
    /// Whether the "+" (new-tab) button is shown.
    pub(crate) show_plus: bool,
}

impl TabBar {
    pub fn new(text_base: TextControlBase) -> Self {
        let mut tb = Self {
            text_base,
            labels: Vec::new(),
            hover_tab: -1,
            close_hovered: false,
            scroll_offset: 0,
            nav_hover: -1,
            show_plus: false,
        };
        tb.parse_labels();
        tb
    }

    /// Parse pipe-separated labels from text_base.text.
    fn parse_labels(&mut self) {
        self.labels.clear();
        if self.text_base.text.is_empty() {
            return;
        }
        let text = &self.text_base.text;
        let mut start = 0;
        for i in 0..text.len() {
            if text[i] == b'|' {
                self.labels.push(text[start..i].to_vec());
                start = i + 1;
            }
        }
        self.labels.push(text[start..].to_vec());
        // Clamp scroll offset
        if self.scroll_offset >= self.labels.len() {
            self.scroll_offset = 0;
        }
    }

    /// Compute overflow layout info (unscaled logical coords).
    /// Returns (has_overflow, show_left, show_right, visible_count, tab_width, nav_start_x).
    fn overflow_info(&self) -> (bool, bool, bool, usize, i32, i32) {
        let n = self.labels.len();
        if n == 0 {
            return (false, false, false, 0, 0, 0);
        }
        let total_w = self.text_base.base.w as i32;
        let plus_reserve = if self.show_plus {
            NAV_BTN_SIZE + TAB_GAP
        } else {
            0
        };
        let tab_area_full = total_w - plus_reserve;
        let total_gaps = TAB_GAP * (n as i32 + 1);
        let avail = tab_area_full - total_gaps;
        let natural_w = (avail / n as i32).min(MAX_TAB_WIDTH);

        if natural_w >= MIN_TAB_WIDTH {
            // No overflow — all tabs fit
            let nav_start = TAB_GAP + n as i32 * (natural_w + TAB_GAP);
            return (false, false, false, n, natural_w, nav_start);
        }

        // Overflow mode: both arrows on the right
        let arrow_reserve = (NAV_BTN_SIZE + TAB_GAP) * 2; // ◀ ▶
        let tab_area = total_w - arrow_reserve - plus_reserve;
        let visible = ((tab_area - TAB_GAP) / (MIN_TAB_WIDTH + TAB_GAP)).max(1) as usize;
        let visible = visible.min(n - self.scroll_offset);
        let show_left = self.scroll_offset > 0;
        let show_right = self.scroll_offset + visible < n;

        // Distribute available width equally among visible tabs
        let tab_gaps = TAB_GAP * (visible as i32 + 1);
        let per_tab = ((tab_area - tab_gaps) / visible as i32).min(MAX_TAB_WIDTH);

        let nav_start = total_w - arrow_reserve - plus_reserve;
        (true, show_left, show_right, visible, per_tab, nav_start)
    }

    /// Compute tab rects (unscaled logical coords).
    fn tab_rects(&self) -> Vec<(i32, i32)> {
        let n = self.labels.len();
        if n == 0 {
            return Vec::new();
        }

        let (has_overflow, _, _, visible, per_tab, _) = self.overflow_info();

        if !has_overflow {
            let total_w = self.text_base.base.w as i32;
            let plus_reserve = if self.show_plus {
                NAV_BTN_SIZE + TAB_GAP
            } else {
                0
            };
            let tab_area = total_w - plus_reserve;
            let total_gaps = TAB_GAP * (n as i32 + 1);
            let avail = tab_area - total_gaps;
            let per_tab = (avail / n as i32).min(MAX_TAB_WIDTH);
            let mut rects = Vec::new();
            let mut cx = TAB_GAP;
            for _ in 0..n {
                rects.push((cx, per_tab));
                cx += per_tab + TAB_GAP;
            }
            return rects;
        }

        // Overflow mode — rects for visible tabs only
        let mut rects = Vec::new();
        let mut cx = TAB_GAP;
        for _ in 0..visible {
            rects.push((cx, per_tab));
            cx += per_tab + TAB_GAP;
        }
        rects
    }

    /// Find which tab (and if close button) is at the given local coordinates.
    /// Returns (tab_index or -1, is_close_btn).
    fn hit_tab(&self, lx: i32, ly: i32) -> (i32, bool) {
        if ly < 0 || ly >= TAB_HEIGHT {
            return (-1, false);
        }
        let rects = self.tab_rects();
        let (has_overflow, _, _, _, _, _) = self.overflow_info();
        let base = if has_overflow { self.scroll_offset } else { 0 };

        for (i, &(tx, tw)) in rects.iter().enumerate() {
            if lx >= tx && lx < tx + tw {
                let close_x = tx + tw - TAB_PAD_X - CLOSE_BTN_SIZE;
                let close_y = (TAB_HEIGHT - CLOSE_BTN_SIZE) / 2;
                if lx >= close_x
                    && lx < close_x + CLOSE_BTN_SIZE
                    && ly >= close_y
                    && ly < close_y + CLOSE_BTN_SIZE
                {
                    return ((base + i) as i32, true);
                }
                return ((base + i) as i32, false);
            }
        }
        (-1, false)
    }

    /// Check if click hits a right-side nav button.
    /// Returns: -1=miss, 0=scroll-left, 1=scroll-right, 2=plus
    fn hit_nav(&self, lx: i32, ly: i32) -> i32 {
        if ly < 0 || ly >= TAB_HEIGHT {
            return -1;
        }
        let total_w = self.text_base.base.w as i32;
        let (has_overflow, _, _, _, _, _) = self.overflow_info();

        // Buttons are arranged right-to-left from the right edge:
        // [...tabs...] [◀] [▶] [+]
        let mut rx = total_w; // current right edge

        if self.show_plus {
            rx -= NAV_BTN_SIZE + TAB_GAP;
            if lx >= rx && lx < rx + NAV_BTN_SIZE {
                return 2; // plus button
            }
        }

        if has_overflow {
            // Right arrow (▶)
            rx -= NAV_BTN_SIZE + TAB_GAP;
            if lx >= rx && lx < rx + NAV_BTN_SIZE {
                return 1;
            }
            // Left arrow (◀)
            rx -= NAV_BTN_SIZE + TAB_GAP;
            if lx >= rx && lx < rx + NAV_BTN_SIZE {
                return 0;
            }
        }

        -1
    }

    /// Ensure the active tab is visible by adjusting scroll_offset.
    fn ensure_active_visible(&mut self) {
        let active = self.text_base.base.state as usize;
        let n = self.labels.len();
        if n == 0 {
            return;
        }
        let (has_overflow, _, _, visible, _, _) = self.overflow_info();
        if !has_overflow {
            return;
        }

        if active < self.scroll_offset {
            self.scroll_offset = active;
        } else if active >= self.scroll_offset + visible {
            self.scroll_offset = active.saturating_sub(visible.saturating_sub(1));
        }
    }
}

impl Control for TabBar {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::TabBar
    }

    fn set_text(&mut self, t: &[u8]) {
        self.text_base.set_text(t);
        self.parse_labels();
        self.ensure_active_visible();
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = crate::control::prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let active = b.state as usize;
        let tc = crate::theme::colors();

        crate::draw::fill_rect(
            surface,
            x,
            y,
            w,
            h,
            crate::controls::chrome::blend(tc.window_bg, tc.tab_inactive_bg, 140),
        );

        // Scaled constants
        let tab_pad_x = crate::theme::scale_i32(TAB_PAD_X);
        let close_btn_size = crate::theme::scale_i32(CLOSE_BTN_SIZE);
        let close_btn_pad = crate::theme::scale_i32(CLOSE_BTN_PAD);
        let tab_gap = crate::theme::scale_i32(TAB_GAP);
        let tab_font = crate::draw::scale_font(TAB_FONT_SIZE);
        let close_font = crate::draw::scale_font(10);
        let close_corner = crate::theme::scale(4);
        let top_margin = crate::theme::scale_i32(TAB_TOP_MARGIN);
        let max_tab_w = crate::theme::scale_i32(MAX_TAB_WIDTH);
        let nav_btn_size = crate::theme::scale_i32(NAV_BTN_SIZE);
        let nav_font = crate::draw::scale_font(11);

        let (has_overflow, show_left, show_right, visible, _, _) = self.overflow_info();

        // Compute tab widths (scaled)
        let n = self.labels.len() as i32;
        let tab_h = (h as i32 - top_margin) as u32;

        // Right-side nav area (both arrows + plus button)
        let plus_reserve_s = if self.show_plus {
            nav_btn_size + tab_gap
        } else {
            0
        };
        let arrow_reserve_s = if has_overflow {
            (nav_btn_size + tab_gap) * 2
        } else {
            0
        };
        let nav_area_s = arrow_reserve_s + plus_reserve_s;

        let (per_tab_w, vis_start, vis_count) = if has_overflow {
            let tab_area = w as i32 - nav_area_s;
            let tab_gaps = tab_gap * (visible as i32 + 1);
            let pw = ((tab_area - tab_gaps) / visible as i32).min(max_tab_w);
            (pw, self.scroll_offset, visible)
        } else if n > 0 {
            let tab_area = w as i32 - plus_reserve_s;
            let total_gaps = tab_gap * (n + 1);
            let avail = tab_area - total_gaps;
            let pw = (avail / n).min(max_tab_w);
            (pw, 0, self.labels.len())
        } else {
            (0, 0, 0)
        };

        // Draw tabs
        let mut cx = tab_gap;
        for vi in 0..vis_count {
            let i = vis_start + vi;
            if i >= self.labels.len() {
                break;
            }
            let label = &self.labels[i];

            let tab_x = x + cx;
            let tab_y = y + top_margin;
            let tab_w = per_tab_w;
            let is_active = i == active;
            let is_hovered = self.hover_tab == i as i32;

            // Tab background — active tab uses accent color, fully rounded
            let pill_r = tab_h / 2; // Pill shape
            let palette = if is_active {
                crate::controls::chrome::accent_palette(
                    tc.tab_border_active,
                    is_hovered,
                    false,
                    false,
                )
            } else {
                crate::controls::chrome::neutral_palette(is_hovered, false, false)
            };
            crate::controls::chrome::draw_surface(
                surface,
                tab_x,
                tab_y,
                tab_w as u32,
                tab_h,
                pill_r,
                palette,
            );

            // Close button
            let close_x = tab_x + tab_w - tab_pad_x - close_btn_size;
            let close_y = tab_y + (tab_h as i32 - close_btn_size) / 2;
            let show_close = is_active || is_hovered;

            // Text area
            let text_area_left = tab_x + tab_pad_x;
            let text_area_right = if show_close {
                close_x - close_btn_pad
            } else {
                tab_x + tab_w - tab_pad_x
            };
            let text_area_w = (text_area_right - text_area_left).max(0);

            let (full_tw, _) = crate::draw::text_size_at(label, tab_font);
            let text_color = if is_active {
                0xFFFFFFFF
            } else {
                tc.text_secondary
            };
            let text_y = tab_y + (tab_h as i32 - tab_font as i32) / 2;

            if (full_tw as i32) <= text_area_w {
                let text_x = text_area_left + (text_area_w - full_tw as i32) / 2;
                crate::draw::draw_text_sized(surface, text_x, text_y, text_color, label, tab_font);
            } else {
                // Truncate with "..."
                let (dots_w, _) = crate::draw::text_size_at(b"...", tab_font);
                let max_text_w = (text_area_w - dots_w as i32).max(0) as u32;
                let mut fit_len = 0usize;
                for (idx, &byte) in label.iter().enumerate() {
                    if byte & 0xC0 == 0x80 {
                        continue;
                    }
                    let (cw, _) = crate::draw::text_size_at(&label[..idx + 1], tab_font);
                    if cw > max_text_w {
                        break;
                    }
                    fit_len = idx + 1;
                }
                while fit_len < label.len() && label[fit_len] & 0xC0 == 0x80 {
                    fit_len += 1;
                }
                let (trunc_w, _) = if fit_len > 0 {
                    crate::draw::text_size_at(&label[..fit_len], tab_font)
                } else {
                    (0, 0)
                };
                let total_trunc_w = trunc_w + dots_w;
                let text_x = text_area_left + (text_area_w - total_trunc_w as i32).max(0) / 2;
                if fit_len > 0 {
                    crate::draw::draw_text_sized(
                        surface,
                        text_x,
                        text_y,
                        text_color,
                        &label[..fit_len],
                        tab_font,
                    );
                }
                crate::draw::draw_text_sized(
                    surface,
                    text_x + trunc_w as i32,
                    text_y,
                    text_color,
                    b"...",
                    tab_font,
                );
            }

            // Draw close button
            if show_close {
                let close_hover = is_hovered && self.close_hovered;
                if close_hover {
                    crate::controls::chrome::draw_surface(
                        surface,
                        close_x,
                        close_y,
                        close_btn_size as u32,
                        close_btn_size as u32,
                        close_corner,
                        crate::controls::chrome::neutral_palette(true, false, false),
                    );
                }
                let fg = if close_hover {
                    tc.text
                } else {
                    tc.text_secondary
                };
                let cx_text = close_x + (close_btn_size - crate::theme::scale_i32(6)) / 2;
                let cy_text = close_y + (close_btn_size - close_font as i32) / 2;
                crate::draw::draw_text_sized(surface, cx_text, cy_text, fg, b"x", close_font);
            }

            cx += tab_w + tab_gap;
        }

        // ── Right-side navigation buttons ──────────────────────────────
        // Layout: [...tabs...] [◀] [▶] [+]  (all right-aligned)
        let mut btn_x = x + w as i32;

        // Plus button (rightmost)
        if self.show_plus {
            btn_x -= nav_btn_size + tab_gap;
            let btn_y = y + top_margin;
            let nav_pill = tab_h / 2;
            crate::controls::chrome::draw_surface(
                surface,
                btn_x,
                btn_y,
                nav_btn_size as u32,
                tab_h,
                nav_pill,
                crate::controls::chrome::neutral_palette(self.nav_hover == 2, false, false),
            );
            let (tw, _) = crate::draw::text_size_at(b"+", nav_font);
            let tx = btn_x + (nav_btn_size - tw as i32) / 2;
            let ty = btn_y + (tab_h as i32 - nav_font as i32) / 2;
            crate::draw::draw_text_sized(surface, tx, ty, tc.text_secondary, b"+", nav_font);
        }

        // Right arrow (▶)
        if has_overflow {
            btn_x -= nav_btn_size + tab_gap;
            let btn_y = y + top_margin;
            let enabled = show_right;
            let nav_pill = tab_h / 2;
            crate::controls::chrome::draw_surface(
                surface,
                btn_x,
                btn_y,
                nav_btn_size as u32,
                tab_h,
                nav_pill,
                crate::controls::chrome::neutral_palette(
                    self.nav_hover == 1 && enabled,
                    false,
                    false,
                ),
            );
            let arrow = b">";
            let fg = if enabled {
                tc.text_secondary
            } else {
                crate::controls::chrome::blend(tc.tab_inactive_bg, tc.text_secondary, 80)
            };
            let (aw, _) = crate::draw::text_size_at(arrow, nav_font);
            let ax2 = btn_x + (nav_btn_size - aw as i32) / 2;
            let ay2 = btn_y + (tab_h as i32 - nav_font as i32) / 2;
            crate::draw::draw_text_sized(surface, ax2, ay2, fg, arrow, nav_font);

            // Left arrow (◀)
            btn_x -= nav_btn_size + tab_gap;
            let btn_y = y + top_margin;
            let enabled = show_left;
            crate::controls::chrome::draw_surface(
                surface,
                btn_x,
                btn_y,
                nav_btn_size as u32,
                tab_h,
                nav_pill,
                crate::controls::chrome::neutral_palette(
                    self.nav_hover == 0 && enabled,
                    false,
                    false,
                ),
            );
            let arrow = b"<";
            let fg = if enabled {
                tc.text_secondary
            } else {
                crate::controls::chrome::blend(tc.tab_inactive_bg, tc.text_secondary, 80)
            };
            let (aw, _) = crate::draw::text_size_at(arrow, nav_font);
            let ax2 = btn_x + (nav_btn_size - aw as i32) / 2;
            let ay2 = btn_y + (tab_h as i32 - nav_font as i32) / 2;
            crate::draw::draw_text_sized(surface, ax2, ay2, fg, arrow, nav_font);
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Check right-side nav buttons first
        let nav = self.hit_nav(lx, ly);
        if nav == 0 {
            // Scroll left
            if self.scroll_offset > 0 {
                self.scroll_offset -= 1;
                self.text_base.base.mark_dirty();
            }
            return EventResponse::CONSUMED;
        } else if nav == 1 {
            // Scroll right
            let (_, _, _, visible, _, _) = self.overflow_info();
            if self.scroll_offset + visible < self.labels.len() {
                self.scroll_offset += 1;
                self.text_base.base.mark_dirty();
            }
            return EventResponse::CONSUMED;
        } else if nav == 2 {
            // Plus button — fire submit event for "new tab"
            return EventResponse::SUBMIT;
        }

        let (tab, is_close) = self.hit_tab(lx, ly);
        if tab < 0 {
            return EventResponse::IGNORED;
        }
        self.text_base.base.state = tab as u32;
        if is_close {
            EventResponse::SUBMIT
        } else {
            EventResponse::CHANGED
        }
    }

    fn handle_double_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Only allow double-click events on actual tabs (for rename etc.),
        // NOT on overflow arrows or the plus button.
        let nav = self.hit_nav(lx, ly);
        if nav >= 0 {
            return EventResponse::CONSUMED; // swallow — no EVENT_DOUBLE_CLICK fires
        }
        let (tab, _) = self.hit_tab(lx, ly);
        if tab < 0 {
            return EventResponse::CONSUMED; // empty area — swallow
        }
        // Double-click on a tab: let EVENT_DOUBLE_CLICK fire (IGNORED = not consumed)
        EventResponse::IGNORED
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        let nav = self.hit_nav(lx, ly);
        let (tab, is_close) = self.hit_tab(lx, ly);
        let changed =
            tab != self.hover_tab || is_close != self.close_hovered || nav != self.nav_hover;
        self.hover_tab = tab;
        self.close_hovered = is_close;
        self.nav_hover = nav;
        if changed {
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_leave(&mut self) {
        self.hover_tab = -1;
        self.close_hovered = false;
        self.nav_hover = -1;
        self.text_base.base.mark_dirty();
    }
}
