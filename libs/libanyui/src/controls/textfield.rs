use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct TextField {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
    pub(crate) password_mode: bool,
    pub(crate) placeholder: Vec<u8>,

    /// Optional prefix icon code (rendered at left edge).
    pub(crate) prefix_icon: Option<u32>,
    /// Optional postfix icon code (rendered at right edge).
    pub(crate) postfix_icon: Option<u32>,
    /// Width reserved for prefix area in pixels.
    pub(crate) prefix_width: u32,
    /// Width reserved for postfix area in pixels.
    pub(crate) postfix_width: u32,

}

impl TextField {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            cursor_pos: 0,
            focused: false,
            password_mode: false,
            placeholder: Vec::new(),
            prefix_icon: None,
            postfix_icon: None,
            prefix_width: 28,
            postfix_width: 28,
        }
    }

    /// Left edge of the text area (after prefix).
    fn text_area_left(&self) -> i32 {
        if self.prefix_icon.is_some() { self.prefix_width as i32 } else { 8 }
    }

    /// Right edge of the text area (before postfix), relative to control width.
    fn text_area_right(&self) -> i32 {
        let w = self.text_base.base.w as i32;
        if self.postfix_icon.is_some() { w - self.postfix_width as i32 } else { w - 8 }
    }
}

impl Control for TextField {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::TextField }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let corner = crate::theme::INPUT_CORNER;

        // Background
        let bg = if disabled { crate::theme::darken(tc.input_bg, 10) } else { tc.input_bg };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);

        // Border: focus (accent) > hover (lighter) > normal
        let border_color = if self.focused {
            tc.input_focus
        } else if hovered && !disabled {
            tc.accent
        } else {
            tc.input_border
        };
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, border_color);

        // Focus ring (2px glow around the field)
        if self.focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }

        // Prefix icon placeholder
        if let Some(_icon) = self.prefix_icon {
            crate::draw::fill_rounded_rect(surface, x + 8, y + (h as i32 - 12) / 2, 12, 12, 6, tc.text_secondary);
        }

        // Postfix icon placeholder
        if let Some(_icon) = self.postfix_icon {
            let px = x + w as i32 - self.postfix_width as i32 + 8;
            crate::draw::fill_rounded_rect(surface, px, y + (h as i32 - 12) / 2, 12, 12, 6, tc.text_secondary);
        }

        // Text area
        let text_x = x + self.text_area_left();
        let text_color = if disabled {
            tc.text_disabled
        } else {
            self.text_base.effective_text_color()
        };
        let font_size = self.text_base.text_style.font_size;

        if self.text_base.text.is_empty() && !self.placeholder.is_empty() {
            crate::draw::draw_text_sized(surface, text_x, y + 6, tc.text_secondary, &self.placeholder, font_size);
        } else if self.password_mode {
            let dot_count = self.text_base.text.len();
            let mut dots = [0u8; 128];
            let n = dot_count.min(128);
            for i in 0..n { dots[i] = b'*'; }
            crate::draw::draw_text_sized(surface, text_x, y + 6, text_color, &dots[..n], font_size);
        } else {
            crate::draw::draw_text_sized(surface, text_x, y + 6, text_color, &self.text_base.text, font_size);
        }

        // Cursor
        if self.focused {
            let cursor_text = self.cursor_pos.min(self.text_base.text.len());
            let cursor_x_offset = crate::draw::text_width_n(&self.text_base.text, cursor_text) as i32;
            let cx = text_x + cursor_x_offset;
            crate::draw::fill_rect(surface, cx, y + 4, 2, h - 8, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }
    fn accepts_focus(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.cursor_pos = self.text_base.text.len();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse {
        use crate::control::*;
        if char_code >= 0x20 && char_code < 0x7F {
            let ch = char_code as u8;
            if self.cursor_pos > self.text_base.text.len() {
                self.cursor_pos = self.text_base.text.len();
            }
            self.text_base.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if keycode == KEY_BACKSPACE {
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == KEY_DELETE {
            if self.cursor_pos < self.text_base.text.len() {
                self.text_base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == KEY_LEFT {
            if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            EventResponse::CONSUMED
        } else if keycode == KEY_RIGHT {
            if self.cursor_pos < self.text_base.text.len() { self.cursor_pos += 1; }
            EventResponse::CONSUMED
        } else if keycode == KEY_HOME {
            self.cursor_pos = 0;
            EventResponse::CONSUMED
        } else if keycode == KEY_END {
            self.cursor_pos = self.text_base.text.len();
            EventResponse::CONSUMED
        } else if keycode == KEY_ENTER {
            EventResponse::SUBMIT
        } else {
            EventResponse::IGNORED
        }
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
