use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn searchfield(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, cursor: u32, focused: bool) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().searchfield_render)(win, x, y, w, h, buf.as_ptr(), len, cursor, focused as u32);
}

pub fn searchfield_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().searchfield_hit_test)(x, y, w, h, mx, my) != 0
}

// ── Stateful component ──

pub struct UiSearchField {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    buf: [u8; 256],
    len: usize,
    pub cursor: u32,
    pub focused: bool,
}

impl UiSearchField {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiSearchField { x, y, w, h, buf: [0; 256], len: 0, cursor: 0, focused: false }
    }

    pub fn text(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    pub fn set_text(&mut self, s: &str) {
        let n = s.len().min(255);
        self.buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        self.len = n;
        self.cursor = n as u32;
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
    }

    pub fn render(&self, win: u32) {
        searchfield(win, self.x, self.y, self.w, self.h, self.text(), self.cursor, self.focused);
    }

    /// Returns `true` if search text changed.
    pub fn handle_event(&mut self, event: &UiEvent) -> bool {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            self.focused = searchfield_hit_test(self.x, self.y, self.w, self.h, mx, my);
            return false;
        }

        if event.is_key_down() && self.focused {
            let key = event.key_code();
            let ch = event.char_val();

            match key {
                KEY_BACKSPACE => {
                    if self.cursor > 0 && self.len > 0 {
                        let pos = self.cursor as usize;
                        let mut i = pos - 1;
                        while i + 1 < self.len {
                            self.buf[i] = self.buf[i + 1];
                            i += 1;
                        }
                        self.len -= 1;
                        self.cursor -= 1;
                        return true;
                    }
                }
                KEY_DELETE => {
                    let pos = self.cursor as usize;
                    if pos < self.len {
                        let mut i = pos;
                        while i + 1 < self.len {
                            self.buf[i] = self.buf[i + 1];
                            i += 1;
                        }
                        self.len -= 1;
                        return true;
                    }
                }
                KEY_LEFT => {
                    if self.cursor > 0 { self.cursor -= 1; }
                }
                KEY_RIGHT => {
                    if (self.cursor as usize) < self.len { self.cursor += 1; }
                }
                KEY_HOME => { self.cursor = 0; }
                KEY_END => { self.cursor = self.len as u32; }
                _ => {
                    if ch >= 0x20 && ch <= 0x7E && self.len < 255 {
                        let pos = self.cursor as usize;
                        let mut i = self.len;
                        while i > pos {
                            self.buf[i] = self.buf[i - 1];
                            i -= 1;
                        }
                        self.buf[pos] = ch as u8;
                        self.len += 1;
                        self.cursor += 1;
                        return true;
                    }
                }
            }
        }
        false
    }
}
