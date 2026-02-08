use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

pub struct Scrollbar {
    pub rect: Rect,
    pub total_items: u32,
    pub visible_items: u32,
    pub scroll_offset: u32,
    pub horizontal: bool,
}

impl Scrollbar {
    pub fn vertical(x: i32, y: i32, height: u32) -> Self {
        Scrollbar {
            rect: Rect::new(x, y, Theme::SCROLLBAR_WIDTH, height),
            total_items: 0,
            visible_items: 0,
            scroll_offset: 0,
            horizontal: false,
        }
    }

    pub fn horizontal(x: i32, y: i32, width: u32) -> Self {
        Scrollbar {
            rect: Rect::new(x, y, width, Theme::SCROLLBAR_WIDTH),
            total_items: 0,
            visible_items: 0,
            scroll_offset: 0,
            horizontal: true,
        }
    }

    pub fn set_range(&mut self, total: u32, visible: u32) {
        self.total_items = total;
        self.visible_items = visible;
    }

    fn thumb_rect(&self) -> Rect {
        if self.total_items == 0 || self.visible_items >= self.total_items {
            return self.rect;
        }

        if self.horizontal {
            let track_len = self.rect.width;
            let thumb_len = (self.visible_items * track_len / self.total_items).max(20);
            let max_offset = self.total_items - self.visible_items;
            let thumb_pos = if max_offset > 0 {
                self.scroll_offset * (track_len - thumb_len) / max_offset
            } else {
                0
            };
            Rect::new(
                self.rect.x + thumb_pos as i32,
                self.rect.y,
                thumb_len,
                self.rect.height,
            )
        } else {
            let track_len = self.rect.height;
            let thumb_len = (self.visible_items * track_len / self.total_items).max(20);
            let max_offset = self.total_items - self.visible_items;
            let thumb_pos = if max_offset > 0 {
                self.scroll_offset * (track_len - thumb_len) / max_offset
            } else {
                0
            };
            Rect::new(
                self.rect.x,
                self.rect.y + thumb_pos as i32,
                self.rect.width,
                thumb_len,
            )
        }
    }

    pub fn render(&self, surface: &mut Surface) {
        let mut renderer = Renderer::new(surface);

        // Track
        renderer.fill_rounded_rect(self.rect, 4, Theme::SCROLLBAR_BG);

        // Thumb
        if self.total_items > self.visible_items {
            let thumb = self.thumb_rect();
            renderer.fill_rounded_rect(thumb, 4, Theme::SCROLLBAR_THUMB);
        }
    }
}
