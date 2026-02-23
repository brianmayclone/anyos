use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

const TAB_PAD_X: i32 = 12;
const CLOSE_BTN_SIZE: i32 = 14;
const CLOSE_BTN_PAD: i32 = 6;
const TAB_HEIGHT: i32 = 28;
const TAB_FONT_SIZE: u16 = 12;
const TAB_GAP: i32 = 1;

// Colors
const BG_COLOR: u32 = 0xFF1E1E1E;
const TAB_INACTIVE: u32 = 0xFF2D2D2D;
const TAB_ACTIVE: u32 = 0xFF1E1E1E;
const TAB_HOVER: u32 = 0xFF383838;
const TEXT_INACTIVE: u32 = 0xFF969696;
const TEXT_ACTIVE: u32 = 0xFFFFFFFF;
const CLOSE_NORMAL: u32 = 0xFF808080;
const CLOSE_HOVER_BG: u32 = 0xFF505050;
const CLOSE_HOVER_FG: u32 = 0xFFFFFFFF;
const BORDER_ACTIVE: u32 = 0xFF007ACC;

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
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let active = self.text_base.base.state as usize;

        // Background
        crate::draw::fill_rect(surface, x, y, w, h, BG_COLOR);

        let rects = self.tab_rects();

        for (i, label) in self.labels.iter().enumerate() {
            if i >= rects.len() { break; }
            let (tx, tw) = rects[i];
            let tab_x = x + tx;
            let is_active = i == active;
            let is_hovered = self.hover_tab == i as i32;

            // Tab background
            let bg = if is_active {
                TAB_ACTIVE
            } else if is_hovered {
                TAB_HOVER
            } else {
                TAB_INACTIVE
            };
            crate::draw::fill_rect(surface, tab_x, y, tw as u32, h, bg);

            // Active indicator (bottom border)
            if is_active {
                crate::draw::fill_rect(surface, tab_x, y + h as i32 - 2, tw as u32, 2, BORDER_ACTIVE);
            }

            // Tab label text
            let text_color = if is_active { TEXT_ACTIVE } else { TEXT_INACTIVE };
            let text_x = tab_x + TAB_PAD_X;
            let text_y = y + (h as i32 - TAB_FONT_SIZE as i32) / 2;
            crate::draw::draw_text_sized(surface, text_x, text_y, text_color, label, TAB_FONT_SIZE);

            // Close button "×"
            let close_x = tab_x + tw - TAB_PAD_X - CLOSE_BTN_SIZE;
            let close_y = y + (h as i32 - CLOSE_BTN_SIZE) / 2;

            let show_close = is_active || is_hovered;
            if show_close {
                let close_hover = is_hovered && self.close_hovered;
                if close_hover {
                    crate::draw::fill_rounded_rect(surface, close_x, close_y,
                        CLOSE_BTN_SIZE as u32, CLOSE_BTN_SIZE as u32, 3, CLOSE_HOVER_BG);
                }
                let fg = if close_hover { CLOSE_HOVER_FG } else { CLOSE_NORMAL };
                // Draw "x" centered in the close button area
                let cx = close_x + (CLOSE_BTN_SIZE - 6) / 2;
                let cy = close_y + (CLOSE_BTN_SIZE - 10) / 2;
                crate::draw::draw_text_sized(surface, cx, cy, fg, b"x", 10);
            }
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
