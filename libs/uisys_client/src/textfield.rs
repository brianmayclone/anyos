use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn textfield(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, placeholder: &str, cursor: u32, focused: bool) {
    let mut tbuf = [0u8; 256];
    let tlen = nul_copy(text, &mut tbuf);
    let mut pbuf = [0u8; 128];
    let plen = nul_copy(placeholder, &mut pbuf);
    let flags = if focused { 1u32 } else { 0 };
    (exports().textfield_render)(win, x, y, w, h, tbuf.as_ptr(), tlen, pbuf.as_ptr(), plen, cursor, flags);
}

pub fn textfield_ex(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: &str, placeholder: &str,
    cursor: u32, focused: bool, password: bool,
    sel_start: u32, sel_end: u32,
) {
    let mut tbuf = [0u8; 256];
    let tlen = nul_copy(text, &mut tbuf);
    let mut pbuf = [0u8; 128];
    let plen = nul_copy(placeholder, &mut pbuf);
    let mut flags = if focused { 1u32 } else { 0 };
    if password { flags |= 2; }
    (exports().textfield_render_ex)(win, x, y, w, h, tbuf.as_ptr(), tlen, pbuf.as_ptr(), plen, cursor, flags, sel_start, sel_end);
}

pub fn textfield_password(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, placeholder: &str, cursor: u32, focused: bool) {
    let mut tbuf = [0u8; 256];
    let tlen = nul_copy(text, &mut tbuf);
    let mut pbuf = [0u8; 128];
    let plen = nul_copy(placeholder, &mut pbuf);
    let flags = (if focused { 1u32 } else { 0 }) | 2;
    (exports().textfield_render)(win, x, y, w, h, tbuf.as_ptr(), tlen, pbuf.as_ptr(), plen, cursor, flags);
}

pub fn textfield_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().textfield_hit_test)(x, y, w, h, mx, my) != 0
}

// ── Stateful component ──

pub struct UiTextField {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    buf: [u8; 256],
    len: usize,
    pub cursor: u32,
    /// Selection anchor (where the selection started).
    /// When sel_anchor == cursor, there is no selection.
    pub sel_anchor: u32,
    pub focused: bool,
    pub password: bool,
    /// True while the mouse button is held down (for drag selection).
    mouse_pressed: bool,
    /// Uptime tick of the last mouse-down (for double-click detection).
    last_click_tick: u32,
}

impl UiTextField {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiTextField {
            x, y, w, h,
            buf: [0; 256], len: 0,
            cursor: 0, sel_anchor: 0,
            focused: false, password: false,
            mouse_pressed: false,
            last_click_tick: 0,
        }
    }

    pub fn text(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    pub fn set_text(&mut self, s: &str) {
        let n = s.len().min(255);
        self.buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        self.len = n;
        self.cursor = n as u32;
        self.sel_anchor = self.cursor;
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
        self.sel_anchor = 0;
    }

    /// Returns true if text is selected.
    pub fn has_selection(&self) -> bool {
        self.sel_anchor != self.cursor
    }

    /// Returns (start, end) of selection in sorted order.
    pub fn selection_range(&self) -> (usize, usize) {
        let a = self.sel_anchor as usize;
        let b = self.cursor as usize;
        if a <= b { (a, b) } else { (b, a) }
    }

    /// Select all text.
    pub fn select_all(&mut self) {
        self.sel_anchor = 0;
        self.cursor = self.len as u32;
    }

    /// Get selected text.
    pub fn selected_text(&self) -> &str {
        if !self.has_selection() { return ""; }
        let (start, end) = self.selection_range();
        let start = start.min(self.len);
        let end = end.min(self.len);
        unsafe { core::str::from_utf8_unchecked(&self.buf[start..end]) }
    }

    pub fn render(&self, win: u32, placeholder: &str) {
        textfield_ex(
            win, self.x, self.y, self.w, self.h,
            self.text(), placeholder,
            self.cursor, self.focused, self.password,
            self.sel_anchor, self.cursor,
        );
    }

    /// Returns true if `c` is a word delimiter for double-click selection.
    fn is_word_delimiter(c: u8) -> bool {
        matches!(c,
            b' ' | b'\t' | b'\n' | b'\r'
            | b'.' | b',' | b'/' | b';' | b':' | b'!' | b'?'
            | b'@' | b'#' | b'$' | b'%' | b'^' | b'&' | b'*'
            | b'(' | b')' | b'-' | b'=' | b'+' | b'[' | b']'
            | b'{' | b'}' | b'|' | b'\\' | b'<' | b'>' | b'"'
            | b'\'' | b'~' | b'`'
        )
    }

    /// Select the word at `pos`, expanding outward to word delimiters.
    fn select_word_at(&mut self, pos: usize) {
        if self.len == 0 { return; }
        let pos = pos.min(self.len.saturating_sub(1));

        // If on a delimiter, select just that character
        if Self::is_word_delimiter(self.buf[pos]) {
            self.sel_anchor = pos as u32;
            self.cursor = (pos + 1).min(self.len) as u32;
            return;
        }

        // Find word start: scan left until delimiter or start of text
        let mut start = pos;
        while start > 0 && !Self::is_word_delimiter(self.buf[start - 1]) {
            start -= 1;
        }

        // Find word end: scan right until delimiter or end of text
        let mut end = pos;
        while end < self.len && !Self::is_word_delimiter(self.buf[end]) {
            end += 1;
        }

        self.sel_anchor = start as u32;
        self.cursor = end as u32;
    }

    /// Delete the currently selected text. Returns true if something was deleted.
    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() { return false; }
        let (start, end) = self.selection_range();
        let start = start.min(self.len);
        let end = end.min(self.len);
        if start == end { return false; }

        // Shift bytes left
        let mut dst = start;
        let mut src = end;
        while src < self.len {
            self.buf[dst] = self.buf[src];
            dst += 1;
            src += 1;
        }
        self.len = dst;
        self.cursor = start as u32;
        self.sel_anchor = self.cursor;
        true
    }

    /// Returns `true` if text content changed.
    /// Focus changes are reflected in `self.focused` but don't return true.
    pub fn handle_event(&mut self, event: &UiEvent) -> bool {
        // ── Mouse down: position cursor, detect double-click ──
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            self.focused = textfield_hit_test(self.x, self.y, self.w, self.h, mx, my);
            if self.focused {
                let pos = (exports().textfield_cursor_from_click)(self.x, self.len as u32, mx);
                let pos = pos.min(self.len as u32);

                // Double-click detection (~400ms threshold at 100 Hz PIT)
                let now = anyos_std::sys::uptime();
                let is_double = now.wrapping_sub(self.last_click_tick) < 40;
                self.last_click_tick = now;

                if is_double {
                    // Select word at cursor position
                    self.select_word_at(pos as usize);
                } else {
                    // Single click — position cursor, optionally extend selection
                    let shift = (event.modifiers() & 1) != 0;
                    self.cursor = pos;
                    if !shift {
                        self.sel_anchor = self.cursor;
                    }
                }
                self.mouse_pressed = true;
            } else {
                self.mouse_pressed = false;
            }
            return false;
        }

        // ── Mouse move: drag selection ──
        if event.event_type == EVENT_MOUSE_MOVE && self.mouse_pressed && self.focused {
            let (mx, _my) = event.mouse_pos();
            let pos = (exports().textfield_cursor_from_click)(self.x, self.len as u32, mx);
            self.cursor = pos.min(self.len as u32);
            // sel_anchor stays at drag origin → selection grows/shrinks
            return false;
        }

        // ── Mouse up: end drag ──
        if event.is_mouse_up() {
            self.mouse_pressed = false;
            return false;
        }

        if event.is_key_down() && self.focused {
            let key = event.key_code();
            let ch = event.char_val();
            let mods = event.modifiers();
            let shift = (mods & 1) != 0;  // bit 0 = shift
            let ctrl = (mods & 2) != 0;   // bit 1 = ctrl

            // Ctrl+A: select all
            if ctrl && (ch == b'a' as u32 || ch == b'A' as u32) {
                self.select_all();
                return false;
            }

            match key {
                KEY_BACKSPACE => {
                    if self.has_selection() {
                        return self.delete_selection();
                    }
                    if self.cursor > 0 && self.len > 0 {
                        let pos = self.cursor as usize;
                        let mut i = pos - 1;
                        while i + 1 < self.len {
                            self.buf[i] = self.buf[i + 1];
                            i += 1;
                        }
                        self.len -= 1;
                        self.cursor -= 1;
                        self.sel_anchor = self.cursor;
                        return true;
                    }
                }
                KEY_DELETE => {
                    if self.has_selection() {
                        return self.delete_selection();
                    }
                    let pos = self.cursor as usize;
                    if pos < self.len {
                        let mut i = pos;
                        while i + 1 < self.len {
                            self.buf[i] = self.buf[i + 1];
                            i += 1;
                        }
                        self.len -= 1;
                        self.sel_anchor = self.cursor;
                        return true;
                    }
                }
                KEY_LEFT => {
                    if !shift && self.has_selection() {
                        // Collapse selection to left edge
                        let (start, _end) = self.selection_range();
                        self.cursor = start as u32;
                        self.sel_anchor = self.cursor;
                    } else if self.cursor > 0 {
                        self.cursor -= 1;
                        if !shift { self.sel_anchor = self.cursor; }
                    }
                }
                KEY_RIGHT => {
                    if !shift && self.has_selection() {
                        // Collapse selection to right edge
                        let (_start, end) = self.selection_range();
                        self.cursor = end as u32;
                        self.sel_anchor = self.cursor;
                    } else if (self.cursor as usize) < self.len {
                        self.cursor += 1;
                        if !shift { self.sel_anchor = self.cursor; }
                    }
                }
                KEY_HOME => {
                    self.cursor = 0;
                    if !shift { self.sel_anchor = self.cursor; }
                }
                KEY_END => {
                    self.cursor = self.len as u32;
                    if !shift { self.sel_anchor = self.cursor; }
                }
                _ => {
                    if ch >= 0x20 && ch <= 0x7E && self.len < 255 {
                        // Delete selection first, then insert
                        self.delete_selection();

                        let pos = self.cursor as usize;
                        if self.len < 255 {
                            let mut i = self.len;
                            while i > pos {
                                self.buf[i] = self.buf[i - 1];
                                i -= 1;
                            }
                            self.buf[pos] = ch as u8;
                            self.len += 1;
                            self.cursor += 1;
                            self.sel_anchor = self.cursor;
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}
