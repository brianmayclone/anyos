use crate::control::{Control, ControlBase, ControlKind, EventResponse};

/// Scrollbar track width in pixels.
const BAR_W: u32 = 10;
/// Padding around scrollbar edges.
const BAR_PAD: i32 = 2;
/// Minimum thumb height in pixels.
const MIN_THUMB: i32 = 20;
/// Corner radius for the rounded thumb.
const THUMB_RADIUS: u32 = 4;

pub struct ScrollView {
    pub(crate) base: ControlBase,
    pub(crate) scroll_y: i32,
    /// Total content height (computed from children bounds).
    pub(crate) content_height: u32,
    /// True while the user is dragging the scrollbar thumb.
    dragging_thumb: bool,
    /// Mouse-Y offset from thumb top when drag started.
    drag_anchor: i32,
}

impl ScrollView {
    pub fn new(base: ControlBase) -> Self {
        Self { base, scroll_y: 0, content_height: 0, dragging_thumb: false, drag_anchor: 0 }
    }

    /// Returns (track_h, thumb_h, max_scroll) if the scrollbar is visible.
    fn scrollbar_metrics(&self) -> Option<(i32, i32, i32)> {
        let h = self.base.h;
        if self.content_height <= h || h <= 4 {
            return None;
        }
        let track_h = (h - 4) as i32;
        let thumb_h = ((h as u64 * track_h as u64) / self.content_height as u64)
            .max(MIN_THUMB as u64) as i32;
        let max_scroll = (self.content_height - h) as i32;
        Some((track_h, thumb_h, max_scroll))
    }

    /// Y position of thumb top, relative to this control's top.
    fn thumb_y(&self, track_h: i32, thumb_h: i32, max_scroll: i32) -> i32 {
        let frac = if max_scroll > 0 {
            (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
        } else {
            0
        };
        BAR_PAD + frac.max(0).min(track_h - thumb_h)
    }

    /// Set scroll_y from a thumb-top position (inverse of thumb_y).
    fn set_scroll_from_thumb(&mut self, thumb_top: i32, track_h: i32, thumb_h: i32, max_scroll: i32) {
        let clamped = thumb_top.max(0).min(track_h - thumb_h);
        let new_scroll = if track_h > thumb_h {
            (clamped as i64 * max_scroll as i64 / (track_h - thumb_h) as i64) as i32
        } else {
            0
        };
        self.scroll_y = new_scroll.max(0).min(max_scroll);
        self.base.state = self.scroll_y as u32;
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

        if let Some((track_h, thumb_h, max_scroll)) = self.scrollbar_metrics() {
            let tc = crate::theme::colors();
            let bar_x = x + w as i32 - BAR_W as i32 - BAR_PAD;

            // Track
            crate::draw::fill_rect(surface, bar_x, y + BAR_PAD, BAR_W, h - 4, tc.scrollbar_track);

            // Thumb
            let ty = self.thumb_y(track_h, thumb_h, max_scroll);
            crate::draw::fill_rounded_rect(
                surface, bar_x, y + ty, BAR_W, thumb_h as u32, THUMB_RADIUS, tc.scrollbar,
            );
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn scrollbar_hit_x(&self) -> Option<i32> {
        if self.scrollbar_metrics().is_some() {
            // Hit area extends 2px left of the visible track for easier targeting.
            Some(self.base.w as i32 - BAR_W as i32 - BAR_PAD - 2)
        } else {
            None
        }
    }

    fn handle_mouse_down(&mut self, local_x: i32, local_y: i32, _button: u32) -> EventResponse {
        if let Some((track_h, thumb_h, max_scroll)) = self.scrollbar_metrics() {
            let hit_x = self.base.w as i32 - BAR_W as i32 - BAR_PAD - 2;
            if local_x >= hit_x {
                let ty = self.thumb_y(track_h, thumb_h, max_scroll);
                if local_y >= ty && local_y < ty + thumb_h {
                    // Click on thumb — start drag, remember offset within thumb.
                    self.dragging_thumb = true;
                    self.drag_anchor = local_y - ty;
                } else {
                    // Click on track — jump so thumb centres on click, then start drag.
                    self.dragging_thumb = true;
                    self.drag_anchor = thumb_h / 2;
                    let new_top = local_y - thumb_h / 2 - BAR_PAD;
                    self.set_scroll_from_thumb(new_top, track_h, thumb_h, max_scroll);
                }
                return EventResponse::CHANGED;
            }
        }
        EventResponse::IGNORED
    }

    fn handle_mouse_move(&mut self, _local_x: i32, local_y: i32) -> EventResponse {
        if !self.dragging_thumb {
            return EventResponse::IGNORED;
        }
        if let Some((track_h, thumb_h, max_scroll)) = self.scrollbar_metrics() {
            let new_top = local_y - self.drag_anchor - BAR_PAD;
            self.set_scroll_from_thumb(new_top, track_h, thumb_h, max_scroll);
            return EventResponse::CHANGED;
        }
        EventResponse::IGNORED
    }

    fn handle_mouse_up(&mut self, _local_x: i32, _local_y: i32, _button: u32) -> EventResponse {
        if self.dragging_thumb {
            self.dragging_thumb = false;
            return EventResponse::CONSUMED;
        }
        EventResponse::IGNORED
    }

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
