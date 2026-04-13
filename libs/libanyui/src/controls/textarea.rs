use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};

pub struct TextArea {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    sel_anchor: usize,
    dragging: bool,
    pub(crate) read_only: bool,
    pub(crate) focused: bool,
    pub(crate) scroll_y: i32,
    /// Maximum text length in bytes (0 = unlimited).
    pub(crate) max_length: usize,
}

impl TextArea {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            cursor_pos: 0,
            sel_anchor: 0,
            dragging: false,
            read_only: false,
            focused: false,
            scroll_y: 0,
            max_length: 0,
        }
    }

    /// Count newlines in text to determine total line count.
    fn line_count(&self) -> usize {
        if self.text_base.text.is_empty() {
            return 1;
        }
        let mut count = 1usize;
        for &b in &self.text_base.text {
            if b == b'\n' {
                count += 1;
            }
        }
        count
    }

    /// Approximate line height from font size.
    fn line_height(&self) -> i32 {
        self.text_base.text_style.font_size as i32 + 4
    }

    /// Total content height in pixels (saturating to avoid overflow).
    fn content_height(&self) -> i32 {
        (self.line_count() as i32).saturating_mul(self.line_height())
    }

    /// Maximum scroll offset.
    fn max_scroll(&self) -> i32 {
        let ch = self.content_height();
        let vh = self.text_base.base.h as i32 - 12; // 6px padding top+bottom
        (ch - vh).max(0)
    }

    /// Auto-scroll to bottom (for output append use case).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_y = self.max_scroll();
    }

    pub(crate) fn set_cursor_pos(&mut self, pos: usize) {
        self.cursor_pos = pos.min(self.text_base.text.len());
        self.sel_anchor = self.cursor_pos;
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
    }

    pub(crate) fn cursor_pos(&self) -> usize {
        self.cursor_pos.min(self.text_base.text.len())
    }

    pub(crate) fn set_selection(&mut self, start: usize, end: usize) {
        let len = self.text_base.text.len();
        self.sel_anchor = start.min(len);
        self.cursor_pos = end.min(len);
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
    }

    pub(crate) fn selection(&self) -> (usize, usize) {
        self.selection_range()
    }

    fn selection_range(&self) -> (usize, usize) {
        if self.cursor_pos <= self.sel_anchor {
            (self.cursor_pos, self.sel_anchor)
        } else {
            (self.sel_anchor, self.cursor_pos)
        }
    }

    fn has_selection(&self) -> bool {
        self.cursor_pos != self.sel_anchor
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        self.text_base.text.drain(start..end);
        self.cursor_pos = start;
        self.sel_anchor = start;
        self.ensure_cursor_visible();
        true
    }

    fn selected_bytes(&self) -> &[u8] {
        if !self.has_selection() {
            return &[];
        }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        &self.text_base.text[start..end]
    }

    fn pos_to_line_col(&self, pos: usize) -> (usize, usize) {
        let text = &self.text_base.text;
        let pos = pos.min(text.len());
        let mut line = 0usize;
        let mut col_start = 0usize;
        for i in 0..pos {
            if text[i] == b'\n' {
                line += 1;
                col_start = i + 1;
            }
        }
        (line, pos.saturating_sub(col_start))
    }

    fn line_col_to_pos(&self, line: usize, col: usize) -> usize {
        let text = &self.text_base.text;
        let mut cur_line = 0usize;
        let mut line_start = 0usize;
        let mut i = 0usize;
        while i < text.len() {
            if cur_line == line {
                break;
            }
            if text[i] == b'\n' {
                cur_line += 1;
                line_start = i + 1;
            }
            i += 1;
        }
        if cur_line != line {
            return text.len();
        }
        let mut line_end = line_start;
        while line_end < text.len() && text[line_end] != b'\n' {
            line_end += 1;
        }
        line_start + col.min(line_end.saturating_sub(line_start))
    }

    fn line_bounds(&self, line: usize) -> (usize, usize) {
        let text = &self.text_base.text;
        let mut cur_line = 0usize;
        let mut start = 0usize;
        let mut i = 0usize;
        while i < text.len() {
            if cur_line == line {
                break;
            }
            if text[i] == b'\n' {
                cur_line += 1;
                start = i + 1;
            }
            i += 1;
        }
        if cur_line != line {
            return (text.len(), text.len());
        }
        let mut end = start;
        while end < text.len() && text[end] != b'\n' {
            end += 1;
        }
        (start, end)
    }

    fn pixel_to_pos(&self, lx: i32, ly: i32) -> usize {
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let lh = font_size as i32 + crate::theme::scale_i32(4);
        let pad_x = crate::theme::scale_i32(8);
        let pad_y = crate::theme::scale_i32(6);
        let rel_y = (ly - pad_y + crate::theme::scale_i32(self.scroll_y)).max(0);
        let line = (rel_y / lh).max(0) as usize;
        let line = line.min(self.line_count().saturating_sub(1));
        let (start, end) = self.line_bounds(line);
        let line_data = &self.text_base.text[start..end];
        let rel_x = (lx - pad_x).max(0);
        start + crate::draw::text_hit_test(line_data, rel_x, font_size)
    }

    fn ensure_cursor_visible(&mut self) {
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let lh = font_size as i32 + crate::theme::scale_i32(4);
        let pad_y = crate::theme::scale_i32(6);
        let (line, _) = self.pos_to_line_col(self.cursor_pos);
        let cursor_y = line as i32 * lh;
        let view_h = self.text_base.base.h as i32 - pad_y * 2;
        if cursor_y < self.scroll_y {
            self.scroll_y = cursor_y;
        } else if cursor_y + lh > self.scroll_y + view_h {
            self.scroll_y = cursor_y + lh - view_h;
        }
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll());
    }

    fn word_left(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let text = &self.text_base.text;
        let mut i = pos - 1;
        while i > 0 && !is_word_char(text[i]) {
            i -= 1;
        }
        while i > 0 && is_word_char(text[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self, pos: usize) -> usize {
        let text = &self.text_base.text;
        let len = text.len();
        if pos >= len {
            return len;
        }
        let mut i = pos;
        while i < len && is_word_char(text[i]) {
            i += 1;
        }
        while i < len && !is_word_char(text[i]) {
            i += 1;
        }
        i
    }

    pub(crate) fn select_all(&mut self) {
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Control for TextArea {
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
        ControlKind::TextArea
    }

    fn set_text(&mut self, t: &[u8]) {
        self.text_base.set_text(t);
        self.scroll_to_bottom();
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let bg = if b.color != 0 {
            b.color
        } else {
            crate::theme::colors().input_bg
        };
        let tc = crate::theme::colors();
        let corner = crate::theme::input_corner();
        let palette =
            crate::controls::chrome::flat_field_palette(bg, b.hovered, self.focused, b.disabled);

        if self.focused && !b.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        // Clip text to control bounds (physical)
        let clipped = surface.with_clip(x + 2, y + 2, w.saturating_sub(4), h.saturating_sub(4));
        let text_color = if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text
        };

        let font_id = self.text_base.text_style.font_id;
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let lh = font_size as i32 + crate::theme::scale_i32(4);
        let pad_x = crate::theme::scale_i32(8);
        let pad_y = crate::theme::scale_i32(6);
        let scaled_scroll_y = crate::theme::scale_i32(self.scroll_y);
        let text = &self.text_base.text;

        let selection = if self.has_selection() {
            Some(self.selection_range())
        } else {
            None
        };

        // Render visible lines only
        if !text.is_empty() {
            let viewport_h = h as i32 - pad_y * 2;
            let first_vis = (scaled_scroll_y / lh).max(0) as usize;
            let last_vis = ((scaled_scroll_y + viewport_h) / lh + 1) as usize;

            let mut line_idx = 0usize;
            let mut line_start = 0usize;

            for i in 0..=text.len() {
                let is_end = i == text.len() || text[i] == b'\n';
                if is_end {
                    if line_idx >= first_vis && line_idx <= last_vis {
                        let line_y = y + pad_y + (line_idx as i32) * lh - scaled_scroll_y;
                        let line_data = &text[line_start..i];
                        if let Some((sel_start, sel_end)) = selection {
                            let draw_start = sel_start.max(line_start).min(i);
                            let draw_end = sel_end.max(line_start).min(i);
                            if draw_start < draw_end || (line_idx > first_vis && sel_start < line_start && sel_end > i) {
                                let start_col = draw_start.saturating_sub(line_start);
                                let end_col = draw_end.saturating_sub(line_start);
                                let start_px = crate::draw::text_width_n_at(
                                    line_data,
                                    start_col.min(line_data.len()),
                                    font_size,
                                ) as i32;
                                let end_px = crate::draw::text_width_n_at(
                                    line_data,
                                    end_col.min(line_data.len()),
                                    font_size,
                                ) as i32;
                                let sel_x = x + pad_x + start_px;
                                let sel_w = (end_px - start_px).max(0) as u32;
                                if sel_w > 0 {
                                    crate::draw::fill_rect(
                                        &clipped,
                                        sel_x,
                                        line_y,
                                        sel_w,
                                        font_size as u32,
                                        tc.accent & 0x60FFFFFF,
                                    );
                                }
                            }
                        }
                        if !line_data.is_empty() {
                            crate::draw::draw_text_ex(
                                &clipped,
                                x + pad_x,
                                line_y,
                                text_color,
                                line_data,
                                font_id,
                                font_size,
                            );
                        }
                    }
                    if line_idx > last_vis {
                        break;
                    }
                    line_idx += 1;
                    line_start = i + 1;
                }
            }
        }

        // Cursor
        if self.focused {
            let cpos = self.cursor_pos.min(text.len());
            let (cur_line, cur_col) = self.pos_to_line_col(cpos);
            let (line_start, line_end) = self.line_bounds(cur_line);
            let col_slice = &text[line_start..(line_start + cur_col).min(line_end)];
            let cx_offset =
                crate::draw::text_width_n_at(col_slice, col_slice.len(), font_size) as i32;
            let cy = y + pad_y + (cur_line as i32) * lh - scaled_scroll_y;
            let cursor_w = crate::theme::scale(2);
            crate::draw::fill_rect(
                &clipped,
                x + pad_x + cx_offset,
                cy,
                cursor_w,
                font_size as u32,
                tc.accent,
            );
        }

        // Scrollbar
        let content_h = crate::theme::scale_i32(self.content_height());
        let view_h = h as i32 - crate::theme::scale_i32(4);
        if content_h > view_h && view_h > 4 {
            let bar_w = crate::theme::scale(6);
            let bar_x = x + w as i32 - bar_w as i32 - crate::theme::scale_i32(2);
            let track_y = y + crate::theme::scale_i32(2);
            let track_h = view_h;
            crate::controls::chrome::draw_surface(
                &clipped,
                bar_x,
                track_y,
                bar_w,
                track_h as u32,
                bar_w / 2,
                crate::controls::chrome::flat_field_palette(
                    tc.scrollbar_track,
                    false,
                    false,
                    false,
                ),
            );
            let thumb_h = ((view_h as i64 * track_h as i64) / content_h as i64)
                .max(crate::theme::scale_i32(20) as i64) as i32;
            let max_scroll = crate::theme::scale_i32(self.max_scroll());
            let scroll_frac = if max_scroll > 0 {
                (scaled_scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else {
                0
            };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            let thumb_r = crate::theme::scale(3);
            crate::controls::chrome::draw_surface(
                &clipped,
                bar_x,
                thumb_y,
                bar_w,
                thumb_h as u32,
                thumb_r,
                crate::controls::chrome::neutral_palette(true, false, false),
            );
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }
    fn accepts_focus(&self) -> bool {
        true
    }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        let pos = self.pixel_to_pos(lx, ly);
        self.cursor_pos = pos;
        self.sel_anchor = pos;
        self.dragging = true;
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        if !self.dragging {
            return EventResponse::IGNORED;
        }
        self.cursor_pos = self.pixel_to_pos(lx, ly);
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.dragging = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_double_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        let pos = self.pixel_to_pos(lx, ly);
        let text = &self.text_base.text;
        if text.is_empty() {
            return EventResponse::CONSUMED;
        }
        let pos = pos.min(text.len().saturating_sub(1));
        let mut start = pos;
        while start > 0 && is_word_char(text[start - 1]) {
            start -= 1;
        }
        let mut end = pos;
        while end < text.len() && is_word_char(text[end]) {
            end += 1;
        }
        if start == end && pos < text.len() {
            end = pos + 1;
            start = pos;
        }
        self.sel_anchor = start;
        self.cursor_pos = end;
        self.ensure_cursor_visible();
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_triple_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.select_all();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        use crate::control::*;
        let shift = modifiers & MOD_SHIFT != 0;
        let ctrl = modifiers & MOD_CTRL != 0;

        if ctrl && (char_code == b'a' as u32 || char_code == b'A' as u32) {
            self.select_all();
            return EventResponse::CONSUMED;
        }

        if ctrl && (char_code == b'c' as u32 || char_code == b'C' as u32) {
            if self.has_selection() {
                let bytes = self.selected_bytes().to_vec();
                crate::compositor::clipboard_set(&bytes);
            }
            return EventResponse::CONSUMED;
        }

        if ctrl && (char_code == b'x' as u32 || char_code == b'X' as u32) {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if self.has_selection() {
                let bytes = self.selected_bytes().to_vec();
                crate::compositor::clipboard_set(&bytes);
                self.delete_selection();
                self.text_base.base.mark_dirty();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }

        if ctrl && (char_code == b'v' as u32 || char_code == b'V' as u32) {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if let Some(clip) = crate::compositor::clipboard_get() {
                let filtered: alloc::vec::Vec<u8> = clip
                    .into_iter()
                    .filter(|&b| b == b'\n' || b >= 0x20 || b >= 0x80)
                    .collect();
                if !filtered.is_empty() {
                    self.delete_selection();
                    let pos = self.cursor_pos.min(self.text_base.text.len());
                    let avail = if self.max_length > 0 {
                        self.max_length.saturating_sub(self.text_base.text.len())
                    } else {
                        filtered.len()
                    };
                    let to_insert = filtered.len().min(avail);
                    if to_insert > 0 {
                        for (i, &b) in filtered[..to_insert].iter().enumerate() {
                            self.text_base.text.insert(pos + i, b);
                        }
                        self.cursor_pos = pos + to_insert;
                        self.sel_anchor = self.cursor_pos;
                        self.ensure_cursor_visible();
                        self.text_base.base.mark_dirty();
                        return EventResponse::CHANGED;
                    }
                }
            }
            return EventResponse::CONSUMED;
        }

        if shift && matches!(keycode, KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN | KEY_HOME | KEY_END)
        {
            if !self.has_selection() {
                self.sel_anchor = self.cursor_pos;
            }
            match keycode {
                KEY_LEFT => {
                    if ctrl {
                        self.cursor_pos = self.word_left(self.cursor_pos);
                    } else if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                    }
                }
                KEY_RIGHT => {
                    if ctrl {
                        self.cursor_pos = self.word_right(self.cursor_pos);
                    } else if self.cursor_pos < self.text_base.text.len() {
                        self.cursor_pos += 1;
                    }
                }
                KEY_UP => {
                    let (line, col) = self.pos_to_line_col(self.cursor_pos);
                    if line > 0 {
                        self.cursor_pos = self.line_col_to_pos(line - 1, col);
                    }
                }
                KEY_DOWN => {
                    let (line, col) = self.pos_to_line_col(self.cursor_pos);
                    if line + 1 < self.line_count() {
                        self.cursor_pos = self.line_col_to_pos(line + 1, col);
                    }
                }
                KEY_HOME => {
                    let (line, _) = self.pos_to_line_col(self.cursor_pos);
                    let (start, _) = self.line_bounds(line);
                    self.cursor_pos = start;
                }
                KEY_END => {
                    let (line, _) = self.pos_to_line_col(self.cursor_pos);
                    let (_, end) = self.line_bounds(line);
                    self.cursor_pos = end;
                }
                _ => {}
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            return EventResponse::CONSUMED;
        }

        if char_code >= 0x20 && char_code < 0x7F {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if self.max_length > 0
                && !self.has_selection()
                && self.text_base.text.len() >= self.max_length
            {
                return EventResponse::CONSUMED;
            }
            self.delete_selection();
            let ch = char_code as u8;
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, ch);
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            return EventResponse::CHANGED;
        } else if keycode == KEY_ENTER {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if self.max_length > 0
                && !self.has_selection()
                && self.text_base.text.len() >= self.max_length
            {
                return EventResponse::CONSUMED;
            }
            self.delete_selection();
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, b'\n');
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            EventResponse::CHANGED
        } else if keycode == KEY_BACKSPACE {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if self.has_selection() {
                self.delete_selection();
                self.text_base.base.mark_dirty();
                return EventResponse::CHANGED;
            }
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                self.sel_anchor = self.cursor_pos;
                self.ensure_cursor_visible();
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == KEY_DELETE {
            if self.read_only {
                return EventResponse::CONSUMED;
            }
            if self.has_selection() {
                self.delete_selection();
                self.text_base.base.mark_dirty();
                return EventResponse::CHANGED;
            }
            if self.cursor_pos < self.text_base.text.len() {
                self.text_base.text.remove(self.cursor_pos);
                self.sel_anchor = self.cursor_pos;
                self.ensure_cursor_visible();
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == KEY_LEFT {
            if ctrl {
                self.cursor_pos = self.word_left(self.cursor_pos);
            } else if !shift && self.has_selection() {
                let (start, _) = self.selection_range();
                self.cursor_pos = start;
            } else if self.cursor_pos > 0 {
                self.cursor_pos -= 1;
            }
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else if keycode == KEY_RIGHT {
            if ctrl {
                self.cursor_pos = self.word_right(self.cursor_pos);
            } else if !shift && self.has_selection() {
                let (_, end) = self.selection_range();
                self.cursor_pos = end;
            } else if self.cursor_pos < self.text_base.text.len() {
                self.cursor_pos += 1;
            }
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else if keycode == KEY_UP {
            let (line, col) = self.pos_to_line_col(self.cursor_pos);
            if line > 0 {
                self.cursor_pos = self.line_col_to_pos(line - 1, col);
            }
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else if keycode == KEY_DOWN {
            let (line, col) = self.pos_to_line_col(self.cursor_pos);
            if line + 1 < self.line_count() {
                self.cursor_pos = self.line_col_to_pos(line + 1, col);
            }
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else if keycode == KEY_HOME {
            let (line, _) = self.pos_to_line_col(self.cursor_pos);
            let (start, _) = self.line_bounds(line);
            self.cursor_pos = start;
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else if keycode == KEY_END {
            let (line, _) = self.pos_to_line_col(self.cursor_pos);
            let (_, end) = self.line_bounds(line);
            self.cursor_pos = end;
            if !shift {
                self.sel_anchor = self.cursor_pos;
            }
            self.ensure_cursor_visible();
            self.text_base.base.mark_dirty();
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let lh = self.line_height();
        self.scroll_y = (self.scroll_y - delta * lh).clamp(0, self.max_scroll());
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.text_base.base.focused = true;
        self.text_base.base.mark_dirty();
        self.ensure_cursor_visible();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.text_base.base.focused = false;
        self.dragging = false;
        self.sel_anchor = self.cursor_pos;
        self.text_base.base.mark_dirty();
    }
}
