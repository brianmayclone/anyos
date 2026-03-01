use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

const TAB_PAD_X: i32 = 12;
const CLOSE_BTN_SIZE: i32 = 14;
const CLOSE_BTN_PAD: i32 = 6;
const TAB_HEIGHT: i32 = 28;
const TAB_FONT_SIZE: u16 = 12;
const TAB_GAP: i32 = 1;


pub struct TabBar {
    pub(crate) text_base: TextControlBase,
    /// Cached tab labels parsed from pipe-separated text.
    labels: Vec<Vec<u8>>,
    /// Which tab is hovered (-1 = none).
    hover_tab: i32,
    /// Whether the close button on the hovered tab is hovered.
    close_hovered: bool,
}

impl TabBar {
    pub fn new(text_base: TextControlBase) -> Self {
        let mut tb = Self {
            text_base,
            labels: Vec::new(),
            hover_tab: -1,
            close_hovered: false,
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
    }

    /// Compute the width of a tab given its label.
    fn tab_width(&self, label: &[u8]) -> i32 {
        let (tw, _) = crate::draw::text_size_at(label, TAB_FONT_SIZE);
        TAB_PAD_X + tw as i32 + CLOSE_BTN_PAD + CLOSE_BTN_SIZE + TAB_PAD_X
    }

    /// Compute tab X positions and widths. Returns vec of (x, w) for each tab.
    fn tab_rects(&self) -> Vec<(i32, i32)> {
        let mut rects = Vec::new();
        let mut cx = 0i32;
        for label in &self.labels {
            let w = self.tab_width(label);
            rects.push((cx, w));
            cx += w + TAB_GAP;
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
        for (i, &(tx, tw)) in rects.iter().enumerate() {
            if lx >= tx && lx < tx + tw {
                // Check if on close button (right side of tab)
                let close_x = tx + tw - TAB_PAD_X - CLOSE_BTN_SIZE;
                let close_y = (TAB_HEIGHT - CLOSE_BTN_SIZE) / 2;
                if lx >= close_x && lx < close_x + CLOSE_BTN_SIZE
                    && ly >= close_y && ly < close_y + CLOSE_BTN_SIZE
                {
                    return (i as i32, true);
                }
                return (i as i32, false);
            }
        }
        (-1, false)
    }
}

impl Control for TabBar {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::TabBar }

    fn set_text(&mut self, t: &[u8]) {
        self.text_base.set_text(t);
        self.parse_labels();
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let active = b.state as usize;
        let tc = crate::theme::colors();

        // Background
        crate::draw::fill_rect(surface, x, y, w, h, tc.window_bg);

        // Scaled constants for rendering
        let tab_pad_x = crate::theme::scale_i32(TAB_PAD_X);
        let close_btn_size = crate::theme::scale_i32(CLOSE_BTN_SIZE);
        let close_btn_pad = crate::theme::scale_i32(CLOSE_BTN_PAD);
        let tab_gap = crate::theme::scale_i32(TAB_GAP);
        let tab_font = crate::draw::scale_font(TAB_FONT_SIZE);
        let close_font = crate::draw::scale_font(10);
        let close_corner = crate::theme::scale(3);

        // Compute scaled tab rects inline (physical pixels)
        let mut cx = 0i32;
        for (i, label) in self.labels.iter().enumerate() {
            let (tw_text, _) = crate::draw::text_size_at(label, tab_font);
            let tab_w = tab_pad_x + tw_text as i32 + close_btn_pad + close_btn_size + tab_pad_x;
            let tab_x = x + cx;
            let is_active = i == active;
            let is_hovered = self.hover_tab == i as i32;

            // Tab background
            let bg = if is_active {
                tc.window_bg
            } else if is_hovered {
                tc.tab_hover_bg
            } else {
                tc.tab_inactive_bg
            };
            crate::draw::fill_rect(surface, tab_x, y, tab_w as u32, h, bg);

            // Active indicator (bottom border)
            if is_active {
                let indicator_h = crate::theme::scale(2);
                crate::draw::fill_rect(surface, tab_x, y + h as i32 - indicator_h as i32, tab_w as u32, indicator_h, tc.tab_border_active);
            }

            // Tab label text
            let text_color = if is_active { tc.text } else { tc.text_secondary };
            let text_x = tab_x + tab_pad_x;
            let text_y = y + (h as i32 - tab_font as i32) / 2;
            crate::draw::draw_text_sized(surface, text_x, text_y, text_color, label, tab_font);

            // Close button
            let close_x = tab_x + tab_w - tab_pad_x - close_btn_size;
            let close_y = y + (h as i32 - close_btn_size) / 2;

            let show_close = is_active || is_hovered;
            if show_close {
                let close_hover = is_hovered && self.close_hovered;
                if close_hover {
                    crate::draw::fill_rounded_rect(surface, close_x, close_y,
                        close_btn_size as u32, close_btn_size as u32, close_corner, tc.input_border);
                }
                let fg = if close_hover { tc.text } else { tc.text_secondary };
                let cx_text = close_x + (close_btn_size - crate::theme::scale_i32(6)) / 2;
                let cy_text = close_y + (close_btn_size - close_font as i32) / 2;
                crate::draw::draw_text_sized(surface, cx_text, cy_text, fg, b"x", close_font);
            }

            cx += tab_w + tab_gap;
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        let (tab, is_close) = self.hit_tab(lx, ly);
        if tab < 0 {
            return EventResponse::IGNORED;
        }
        self.text_base.base.state = tab as u32;
        if is_close {
            // Close button clicked → fire submit
            EventResponse::SUBMIT
        } else {
            // Tab label clicked → fire change
            EventResponse::CHANGED
        }
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        let (tab, is_close) = self.hit_tab(lx, ly);
        let changed = tab != self.hover_tab || is_close != self.close_hovered;
        self.hover_tab = tab;
        self.close_hovered = is_close;
        if changed {
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_leave(&mut self) {
        self.hover_tab = -1;
        self.close_hovered = false;
        self.text_base.base.mark_dirty();
    }
}
