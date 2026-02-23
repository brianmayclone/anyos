use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct TextArea {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
    pub(crate) scroll_y: i32,
}

impl TextArea {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base, cursor_pos: 0, focused: false, scroll_y: 0 }
    }

    /// Count newlines in text to determine total line count.
    fn line_count(&self) -> usize {
        if self.text_base.text.is_empty() { return 1; }
        let mut count = 1usize;
        for &b in &self.text_base.text {
            if b == b'\n' { count += 1; }
        }
        count
    }

    /// Approximate line height from font size.
    fn line_height(&self) -> i32 {
        self.text_base.text_style.font_size as i32 + 4
    }

    /// Total content height in pixels.
    fn content_height(&self) -> i32 {
        self.line_count() as i32 * self.line_height()
    }

    /// Maximum scroll offset.
    fn max_scroll(&self) -> i32 {
        let ch = self.content_height();
        let vh = self.text_base.base.h as i32 - 12; // 6px padding top+bottom
        (ch - vh).max(0)
    }

    /// Clamp scroll_y to valid range.
    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll());
    }

    /// Auto-scroll to bottom (for output append use case).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_y = self.max_scroll();
    }
}

impl Control for TextArea {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::TextArea }

    fn set_text(&mut self, t: &[u8]) {
        self.text_base.set_text(t);
        self.scroll_to_bottom();
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let bg = if self.text_base.base.color != 0 {
            self.text_base.base.color
        } else {
            crate::theme::colors().input_bg
        };
        let tc = crate::theme::colors();

        // Background
        crate::draw::fill_rect(surface, x, y, w, h, bg);

        // Clip text to control bounds
        let clipped = surface.with_clip(x, y, w, h);
        let text_color = if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text
        };

        let lh = self.line_height();
        let pad_x = 8;
        let pad_y = 6;
        let font_id = self.text_base.text_style.font_id;
        let font_size = self.text_base.text_style.font_size;
        let text = &self.text_base.text;

        // Render visible lines only
        if !text.is_empty() {
            let viewport_h = h as i32 - pad_y * 2;
            let first_vis = (self.scroll_y / lh).max(0) as usize;
            let last_vis = ((self.scroll_y + viewport_h) / lh + 1) as usize;

            let mut line_idx = 0usize;
            let mut line_start = 0usize;

            for i in 0..=text.len() {
                let is_end = i == text.len() || text[i] == b'\n';
                if is_end {
                    if line_idx >= first_vis && line_idx <= last_vis {
                        let line_y = y + pad_y + (line_idx as i32) * lh - self.scroll_y;
                        let line_data = &text[line_start..i];
                        if !line_data.is_empty() {
                            crate::draw::draw_text_ex(
                                &clipped, x + pad_x, line_y, text_color,
                                line_data, font_id, font_size,
                            );
                        }
                    }
                    if line_idx > last_vis { break; }
                    line_idx += 1;
                    line_start = i + 1;
                }
            }
        }

        // Cursor
        if self.focused {
            let cpos = self.cursor_pos.min(text.len());
            let mut cur_line = 0usize;
            let mut col_start = 0usize;
            for i in 0..cpos {
                if text[i] == b'\n' {
                    cur_line += 1;
                    col_start = i + 1;
                }
            }
            let col_slice = &text[col_start..cpos];
            let cx_offset = crate::draw::text_width_n(col_slice, col_slice.len()) as i32;
            let cy = y + pad_y + (cur_line as i32) * lh - self.scroll_y;
            crate::draw::fill_rect(&clipped, x + pad_x + cx_offset, cy, 2, font_size as u32, tc.accent);
        }

        // Scrollbar
        let content_h = self.content_height();
        let view_h = h as i32 - 4;
        if content_h > view_h && view_h > 4 {
            let bar_w = 6u32;
            let bar_x = x + w as i32 - bar_w as i32 - 2;
            let track_y = y + 2;
            let track_h = view_h;
            crate::draw::fill_rect(&clipped, bar_x, track_y, bar_w, track_h as u32, tc.scrollbar_track);
            let thumb_h = ((view_h as i64 * track_h as i64) / content_h as i64).max(20) as i32;
            let max_scroll = self.max_scroll();
            let scroll_frac = if max_scroll > 0 {
                (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else { 0 };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            crate::draw::fill_rounded_rect(&clipped, bar_x, thumb_y, bar_w, thumb_h as u32, 3, tc.scrollbar);
        }
    }

    fn is_interactive(&self) -> bool { true }
    fn accepts_focus(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.cursor_pos = self.text_base.text.len();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse {
        if char_code >= 0x20 && char_code < 0x7F {
            let ch = char_code as u8;
            if self.cursor_pos > self.text_base.text.len() {
                self.cursor_pos = self.text_base.text.len();
            }
            self.text_base.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if char_code == 0x0A || char_code == 0x0D {
            if self.cursor_pos > self.text_base.text.len() {
                self.cursor_pos = self.text_base.text.len();
            }
            self.text_base.text.insert(self.cursor_pos, b'\n');
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if keycode == 0x0E || char_code == 0x08 {
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == 0x4B {
            if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            EventResponse::CONSUMED
        } else if keycode == 0x4D {
            if self.cursor_pos < self.text_base.text.len() { self.cursor_pos += 1; }
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let lh = self.line_height();
        self.scroll_y = (self.scroll_y - delta * lh).clamp(0, self.max_scroll());
        self.text_base.base.dirty = true;
        EventResponse::CONSUMED
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.text_base.base.focused = true;
        self.text_base.base.dirty = true;
        self.cursor_pos = self.text_base.text.len();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.text_base.base.focused = false;
        self.text_base.base.dirty = true;
    }
}
