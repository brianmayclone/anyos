use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};
use crate::control::{
    KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_ESCAPE, KEY_HOME, KEY_LEFT,
    KEY_RIGHT, KEY_TAB, KEY_UP, MOD_CTRL, MOD_SHIFT,
};
use alloc::string::ToString;
use alloc::vec::Vec;

const CORNER: u32 = 6;
const NO_SELECTION: u32 = u32::MAX;

pub struct ComboBox {
    pub(crate) text_base: TextControlBase,
    pub(crate) items: Vec<u8>,
    pub(crate) open: bool,
    pub(crate) editable: bool,
    pub(crate) cursor_pos: usize,
    pub(crate) sel_anchor: usize,
    pub(crate) placeholder: Vec<u8>,
    pub(crate) popup_nav: i32,
    pub(crate) popup_accept: bool,
    pub(crate) request_popup: bool,
    scroll_x: i32,
}

impl ComboBox {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            items: Vec::new(),
            open: false,
            editable: false,
            cursor_pos: 0,
            sel_anchor: 0,
            placeholder: Vec::new(),
            popup_nav: 0,
            popup_accept: false,
            request_popup: false,
            scroll_x: 0,
        }
    }

    pub(crate) fn item_count(&self) -> usize {
        if self.items.is_empty() {
            0
        } else {
            self.items.iter().filter(|&&b| b == b'|').count() + 1
        }
    }

    pub(crate) fn item_label(&self, index: usize) -> &[u8] {
        let text = &self.items;
        let mut seg = 0;
        let mut start = 0;
        for i in 0..text.len() {
            if text[i] == b'|' {
                if seg == index {
                    return &text[start..i];
                }
                seg += 1;
                start = i + 1;
            }
        }
        if seg == index { &text[start..] } else { &[] }
    }

    pub(crate) fn set_items(&mut self, items: &[u8]) {
        self.items.clear();
        self.items.extend_from_slice(items);
        if !self.editable {
            self.apply_selected_state();
        }
        self.text_base.base.mark_dirty();
    }

    pub(crate) fn set_editable(&mut self, editable: bool) {
        if self.editable != editable {
            self.editable = editable;
            if !editable {
                self.apply_selected_state();
            }
            self.text_base.base.mark_dirty();
        }
    }

    pub(crate) fn set_placeholder(&mut self, text: &[u8]) {
        self.placeholder.clear();
        self.placeholder.extend_from_slice(text);
        self.text_base.base.mark_dirty();
    }

    fn selected_index(&self) -> Option<usize> {
        let idx = self.text_base.base.state;
        if idx == NO_SELECTION {
            None
        } else {
            Some(idx as usize)
        }
    }

    fn has_selection(&self) -> bool {
        self.editable && self.cursor_pos != self.sel_anchor
    }

    fn selection_range(&self) -> (usize, usize) {
        if self.cursor_pos <= self.sel_anchor {
            (self.cursor_pos, self.sel_anchor)
        } else {
            (self.sel_anchor, self.cursor_pos)
        }
    }

    fn delete_selection(&mut self) {
        if !self.has_selection() {
            return;
        }
        let (start, end) = self.selection_range();
        self.text_base.text.drain(start..end);
        self.cursor_pos = start;
        self.sel_anchor = start;
    }

    fn ensure_cursor_visible(&mut self) {
        if !self.editable {
            self.scroll_x = 0;
            return;
        }
        let font_size = crate::draw::scale_font(if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        });
        let pos = self.cursor_pos.min(self.text_base.text.len());
        let (cursor_px, _) =
            crate::draw::measure_text_ex(&self.text_base.text[..pos], 0, font_size);
        let field_w = crate::theme::scale(self.text_base.base.w)
            .saturating_sub(crate::theme::scale(34));
        let cursor_x = cursor_px as i32 - self.scroll_x;
        if cursor_x < 0 {
            self.scroll_x = cursor_px as i32;
        } else if cursor_x > field_w as i32 {
            self.scroll_x = cursor_px as i32 - field_w as i32;
        }
    }

    fn set_current_text(&mut self, text: &[u8]) {
        self.text_base.text.clear();
        self.text_base.text.extend_from_slice(text);
        self.cursor_pos = self.text_base.text.len();
        self.sel_anchor = self.cursor_pos;
        self.ensure_cursor_visible();
    }

    pub(crate) fn apply_selected_index(&mut self, index: usize) {
        if index < self.item_count() {
            self.text_base.base.state = index as u32;
            let label = self.item_label(index).to_vec();
            self.set_current_text(&label);
            self.open = false;
            self.request_popup = false;
            self.text_base.base.mark_dirty();
        }
    }

    fn apply_selected_state(&mut self) {
        if let Some(index) = self.selected_index() {
            if index < self.item_count() {
                let label = self.item_label(index).to_vec();
                self.set_current_text(&label);
                return;
            }
        }
        self.text_base.base.state = NO_SELECTION;
        self.set_current_text(&[]);
    }

    pub(crate) fn popup_items(&self) -> Vec<u8> {
        let query = if self.editable {
            to_lower_u8(&self.text_base.text)
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for index in 0..self.item_count() {
            let label = self.item_label(index);
            if !query.is_empty() && !contains_bytes(&to_lower_u8(label), &query) {
                continue;
            }
            if !out.is_empty() {
                out.push(b'|');
            }
            out.extend_from_slice(index.to_string().as_bytes());
            out.push(0x1F);
            out.extend_from_slice(label);
        }
        out
    }

    fn move_selection_by(&mut self, delta: i32) -> EventResponse {
        let count = self.item_count();
        if count == 0 {
            return EventResponse::IGNORED;
        }
        let next = match self.selected_index() {
            Some(cur) => (cur as i32 + delta).clamp(0, count.saturating_sub(1) as i32) as usize,
            None if delta >= 0 => 0,
            None => count.saturating_sub(1),
        };
        self.apply_selected_index(next);
        EventResponse::CHANGED
    }

    fn select_all(&mut self) {
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
    }
}

impl Control for ComboBox {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }

    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }

    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.text_base)
    }

    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.text_base)
    }

    fn kind(&self) -> ControlKind {
        ControlKind::ComboBox
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = crate::control::prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::scale(CORNER);
        let palette =
            crate::controls::chrome::flat_field_palette(0, ctx.hovered, ctx.focused, ctx.disabled);
        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        let text_color = if ctx.disabled { tc.text_disabled } else { tc.text };
        let text_x = x + crate::theme::scale_i32(10);
        let ty = y + (h as i32 - font_size as i32) / 2;

        if self.text_base.text.is_empty() && !self.placeholder.is_empty() {
            crate::draw::draw_text_sized(
                surface,
                text_x,
                ty,
                tc.text_secondary,
                &self.placeholder,
                font_size,
            );
        } else {
            crate::draw::draw_text_sized(
                surface,
                text_x - crate::theme::scale_i32(self.scroll_x),
                ty,
                text_color,
                &self.text_base.text,
                font_size,
            );
        }

        if self.editable && ctx.focused && !ctx.disabled {
            let cursor_px = if self.cursor_pos > 0 && self.cursor_pos <= self.text_base.text.len() {
                let (tw, _) =
                    crate::draw::measure_text_ex(&self.text_base.text[..self.cursor_pos], 0, font_size);
                tw as i32
            } else {
                0
            };
            let cursor_x = text_x + cursor_px - crate::theme::scale_i32(self.scroll_x);
            crate::draw::fill_rect(surface, cursor_x, ty, 2, font_size as u32, text_color);
        }

        let chevron_rows = crate::theme::scale_i32(5);
        let chevron_x = x + w as i32 - crate::theme::scale_i32(20);
        let chevron_y = y + (h as i32 / 2) - crate::theme::scale_i32(2);
        let chevron_color = if ctx.disabled {
            tc.text_disabled
        } else {
            tc.text_secondary
        };
        let half_max = chevron_rows - 1;
        for row in 0..chevron_rows {
            let half = half_max - row;
            let cx = chevron_x + (half_max - half);
            let cw = 1 + half * 2;
            crate::draw::fill_rect(surface, cx, chevron_y + row, cw as u32, 1, chevron_color);
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.open = true;
        self.request_popup = true;
        self.text_base.base.focused = true;
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        if !self.editable {
            return match keycode {
                KEY_DOWN => self.move_selection_by(1),
                KEY_UP => self.move_selection_by(-1),
                KEY_ENTER | KEY_TAB => {
                    self.open = true;
                    self.request_popup = true;
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                }
                KEY_ESCAPE => EventResponse::CONSUMED,
                _ => EventResponse::IGNORED,
            };
        }

        let shift = modifiers & MOD_SHIFT != 0;
        let ctrl = modifiers & MOD_CTRL != 0;

        if ctrl && (char_code == b'a' as u32 || char_code == b'A' as u32) {
            self.select_all();
            self.text_base.base.mark_dirty();
            return EventResponse::CONSUMED;
        }

        if ctrl && (char_code == b'c' as u32 || char_code == b'C' as u32) {
            if self.has_selection() {
                let (start, end) = self.selection_range();
                crate::compositor::clipboard_set(&self.text_base.text[start..end]);
            }
            return EventResponse::CONSUMED;
        }

        if ctrl && (char_code == b'v' as u32 || char_code == b'V' as u32) {
            if let Some(clip) = crate::compositor::clipboard_get() {
                let filtered: Vec<u8> = clip
                    .into_iter()
                    .filter(|&b| b >= 0x20 && b < 0x7F)
                    .collect();
                if !filtered.is_empty() {
                    self.delete_selection();
                    let pos = self.cursor_pos.min(self.text_base.text.len());
                    for (i, &b) in filtered.iter().enumerate() {
                        self.text_base.text.insert(pos + i, b);
                    }
                    self.cursor_pos = pos + filtered.len();
                    self.sel_anchor = self.cursor_pos;
                    self.ensure_cursor_visible();
                    self.text_base.base.state = NO_SELECTION;
                    self.open = true;
                    self.request_popup = true;
                    self.text_base.base.mark_dirty();
                    return EventResponse::CHANGED;
                }
            }
            return EventResponse::CONSUMED;
        }

        if char_code >= 0x20 && char_code < 0x7F && !ctrl {
            self.delete_selection();
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, char_code as u8);
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            self.text_base.base.state = NO_SELECTION;
            self.open = true;
            self.request_popup = true;
            self.text_base.base.mark_dirty();
            return EventResponse::CHANGED;
        }

        match keycode {
            KEY_LEFT => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    if !shift {
                        self.sel_anchor = self.cursor_pos;
                    }
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_RIGHT => {
                if self.cursor_pos < self.text_base.text.len() {
                    self.cursor_pos += 1;
                    if !shift {
                        self.sel_anchor = self.cursor_pos;
                    }
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_HOME => {
                self.cursor_pos = 0;
                if !shift {
                    self.sel_anchor = 0;
                }
                self.scroll_x = 0;
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_END => {
                self.cursor_pos = self.text_base.text.len();
                if !shift {
                    self.sel_anchor = self.cursor_pos;
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_BACKSPACE => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.text_base.text.remove(self.cursor_pos);
                    self.sel_anchor = self.cursor_pos;
                }
                self.text_base.base.state = NO_SELECTION;
                self.open = true;
                self.request_popup = true;
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CHANGED
            }
            KEY_DELETE => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor_pos < self.text_base.text.len() {
                    self.text_base.text.remove(self.cursor_pos);
                }
                self.text_base.base.state = NO_SELECTION;
                self.open = true;
                self.request_popup = true;
                self.text_base.base.mark_dirty();
                EventResponse::CHANGED
            }
            KEY_DOWN => {
                self.popup_nav = 1;
                self.open = true;
                self.request_popup = true;
                EventResponse::CONSUMED
            }
            KEY_UP => {
                self.popup_nav = -1;
                self.open = true;
                self.request_popup = true;
                EventResponse::CONSUMED
            }
            KEY_ENTER | KEY_TAB => {
                self.popup_accept = true;
                self.open = true;
                self.request_popup = true;
                EventResponse::CONSUMED
            }
            KEY_ESCAPE => {
                self.open = false;
                self.request_popup = false;
                EventResponse::CONSUMED
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_blur(&mut self) {
        self.text_base.base.focused = false;
        self.text_base.base.mark_dirty();
    }
}

fn to_lower_u8(s: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(s.len());
    for &b in s {
        if b.is_ascii_uppercase() {
            r.push(b + 32);
        } else {
            r.push(b);
        }
    }
    r
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}
