use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct ScrollView {
    pub(crate) base: ControlBase,
    pub(crate) scroll_y: i32,
    /// Total content height (computed from children bounds).
    pub(crate) content_height: u32,
}

impl ScrollView {
    pub fn new(base: ControlBase) -> Self {
        Self { base, scroll_y: 0, content_height: 0 }
    }
}

impl Control for ScrollView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ScrollView }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let w = self.base.w;
        let h = self.base.h;

        // Scrollbar (only if content taller than viewport)
        if self.content_height > h && h > 4 {
            let tc = crate::theme::colors();
            let bar_w = 6u32;
            let bar_x = x + w as i32 - bar_w as i32 - 2;

            // Track
            crate::draw::fill_rect(surface, bar_x, y + 2, bar_w, h - 4, tc.scrollbar_track);

            // Thumb
            let track_h = (h - 4) as i32;
            let thumb_h = ((h as u64 * track_h as u64) / self.content_height as u64).max(20) as i32;
            let max_scroll = (self.content_height - h) as i32;
            let scroll_frac = if max_scroll > 0 {
                (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else {
                0
            };
            let thumb_y = y + 2 + scroll_frac.max(0).min(track_h - thumb_h);
            crate::draw::fill_rounded_rect(surface, bar_x, thumb_y, bar_w, thumb_h as u32, 3, tc.scrollbar);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let max_scroll = if self.content_height > self.base.h {
            (self.content_height - self.base.h) as i32
        } else {
            0
        };
        self.scroll_y = (self.scroll_y - delta * 20).max(0).min(max_scroll);
        self.base.state = self.scroll_y as u32;
        EventResponse::CHANGED
    }
}

/// Update content_height for all ScrollViews (called from event_loop after layout).
pub fn update_scroll_bounds(controls: &mut [alloc::boxed::Box<dyn Control>]) {
    for i in 0..controls.len() {
        if controls[i].kind() == ControlKind::ScrollView {
            let children: alloc::vec::Vec<u32> = controls[i].base().children.to_vec();
            let mut max_bottom = 0i32;
            for &child_id in &children {
                if let Some(idx) = crate::control::find_idx(controls, child_id) {
                    let b = controls[idx].base();
                    if b.visible {
                        let bottom = b.y + b.h as i32;
                        if bottom > max_bottom { max_bottom = bottom; }
                    }
                }
            }
            let raw: *mut dyn Control = &mut *controls[i];
            let sv = unsafe { &mut *(raw as *mut ScrollView) };
            sv.content_height = max_bottom.max(0) as u32;
            let max_scroll = if sv.content_height > sv.base.h {
                (sv.content_height - sv.base.h) as i32
            } else { 0 };
            sv.scroll_y = sv.scroll_y.min(max_scroll).max(0);
            sv.base.state = sv.scroll_y as u32;
        }
    }
}
