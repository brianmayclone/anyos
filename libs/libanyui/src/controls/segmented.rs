use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct SegmentedControl {
    pub(crate) text_base: TextControlBase,
}

impl SegmentedControl {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }

    fn segment_count(&self) -> usize {
        if self.text_base.text.is_empty() { return 0; }
        self.text_base.text.iter().filter(|&&b| b == b'|').count() + 1
    }

    fn segment_label(&self, index: usize) -> &[u8] {
        let text = &self.text_base.text;
        let mut seg = 0;
        let mut start = 0;
        for i in 0..text.len() {
            if text[i] == b'|' {
                if seg == index { return &text[start..i]; }
                seg += 1;
                start = i + 1;
            }
        }
        if seg == index { &text[start..] } else { &[] }
    }
}

impl Control for SegmentedControl {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::SegmentedControl }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let focused = b.focused;
        let active = b.state as usize;
        let n = self.segment_count();
        if n == 0 { return; }

        let shell = crate::controls::chrome::neutral_palette(b.hovered, false, disabled);
        if focused && !disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, h / 2, shell);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, h / 2, shell);

        let seg_w = w / n as u32;

        for i in 0..n {
            let sx = x + (i as u32 * seg_w) as i32;
            let sw = if i == n - 1 { w - (i as u32 * seg_w) } else { seg_w };

            if i == active {
                let pad = crate::theme::scale_i32(2);
                let aw = sw.saturating_sub(pad as u32 * 2);
                let ah = h.saturating_sub(pad as u32 * 2);
                let ar = ah / 2;
                crate::controls::chrome::draw_surface(
                    surface,
                    sx + pad,
                    y + pad,
                    aw,
                    ah,
                    ar,
                    crate::controls::chrome::accent_palette(tc.accent, false, false, disabled),
                );
            }

            // Segment label text
            let label = self.segment_label(i);
            if !label.is_empty() {
                let logical_fs = if self.text_base.text_style.font_size > 0 { self.text_base.text_style.font_size } else { 12 };
                let font_size = crate::draw::scale_font(logical_fs);
                let (tw, _th) = crate::draw::text_size_at(label, font_size);
                let tx = sx + (sw as i32 - tw as i32) / 2;
                let ty = y + (h as i32 - font_size as i32) / 2;
                let text_color = if disabled {
                    tc.text_disabled
                } else if i == active {
                    0xFFFFFFFF
                } else {
                    tc.text_secondary
                };
                crate::draw::draw_text_sized(surface, tx, ty, text_color, label, font_size);
            }

            // Separator (not after last, not adjacent to active)
            if i < n - 1 && i != active && i + 1 != active {
                let sep_x = sx + sw as i32 - 1;
                let sep_pad = crate::theme::scale_i32(6);
                let sep_h = h.saturating_sub(crate::theme::scale(12));
                crate::draw::fill_rect(surface, sep_x, y + sep_pad, 1, sep_h, tc.separator);
            }
        }

    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let n = self.segment_count();
        if n == 0 { return EventResponse::IGNORED; }
        let seg_width = self.text_base.base.w as i32 / n as i32;
        if seg_width > 0 {
            let seg_idx = (lx / seg_width).max(0).min(n as i32 - 1) as u32;
            if self.text_base.base.state != seg_idx {
                self.text_base.base.state = seg_idx;
                self.text_base.base.mark_dirty();
                return EventResponse::CHANGED;
            }
        }
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        let n = self.segment_count();
        if n == 0 { return EventResponse::IGNORED; }
        match keycode {
            crate::control::KEY_LEFT => {
                if self.text_base.base.state > 0 {
                    self.text_base.base.state -= 1;
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            crate::control::KEY_RIGHT => {
                if self.text_base.base.state + 1 < n as u32 {
                    self.text_base.base.state += 1;
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            _ => EventResponse::IGNORED,
        }
    }
}
