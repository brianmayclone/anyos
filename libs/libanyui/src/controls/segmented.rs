use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct SegmentedControl {
    pub(crate) text_base: TextControlBase,
}

impl SegmentedControl {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }

    /// Count the number of segments (pipe-separated labels in text).
    fn segment_count(&self) -> usize {
        if self.text_base.text.is_empty() {
            return 0;
        }
        self.text_base.text.iter().filter(|&&b| b == b'|').count() + 1
    }

    /// Get the label bytes for segment at `index`.
    fn segment_label(&self, index: usize) -> &[u8] {
        let text = &self.text_base.text;
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
        if seg == index {
            &text[start..]
        } else {
            &[]
        }
    }
}

impl Control for SegmentedControl {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::SegmentedControl }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let active = self.text_base.base.state as usize;
        let n = self.segment_count();
        if n == 0 {
            return;
        }

        // Background pill
        crate::draw::fill_rounded_rect(surface, x, y, w, h, 4, tc.control_bg);

        let seg_w = w / n as u32;

        for i in 0..n {
            let sx = x + (i as u32 * seg_w) as i32;
            let sw = if i == n - 1 { w - (i as u32 * seg_w) } else { seg_w };

            // Active segment highlight
            if i == active {
                let pad = 2i32;
                crate::draw::fill_rounded_rect(
                    surface,
                    sx + pad,
                    y + pad,
                    sw.saturating_sub(pad as u32 * 2),
                    h.saturating_sub(pad as u32 * 2),
                    3,
                    tc.accent,
                );
            }

            // Segment label text
            let label = self.segment_label(i);
            if !label.is_empty() {
                let font_size = if self.text_base.text_style.font_size > 0 {
                    self.text_base.text_style.font_size
                } else {
                    12
                };
                let (tw, _th) = crate::draw::text_size_at(label, font_size);
                let tx = sx + (sw as i32 - tw as i32) / 2;
                let ty = y + (h as i32 - font_size as i32) / 2;
                let text_color = if i == active {
                    0xFFFFFFFF
                } else {
                    tc.text_secondary
                };
                crate::draw::draw_text_sized(surface, tx, ty, text_color, label, font_size);
            }

            // Separator between segments (not after last)
            if i < n - 1 && i != active && i + 1 != active {
                let sep_x = sx + sw as i32 - 1;
                crate::draw::fill_rect(surface, sep_x, y + 4, 1, h.saturating_sub(8), tc.separator);
            }
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let n = self.segment_count();
        if n == 0 {
            return EventResponse::IGNORED;
        }
        let seg_width = self.text_base.base.w as i32 / n as i32;
        if seg_width > 0 {
            let seg_idx = (lx / seg_width).max(0).min(n as i32 - 1) as u32;
            if self.text_base.base.state != seg_idx {
                self.text_base.base.state = seg_idx;
                self.text_base.base.dirty = true;
                return EventResponse::CHANGED;
            }
        }
        EventResponse::CONSUMED
    }
}
