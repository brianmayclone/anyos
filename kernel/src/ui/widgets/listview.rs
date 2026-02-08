use alloc::string::String;
use alloc::vec::Vec;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

pub struct ListView {
    pub rect: Rect,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub scroll_offset: usize,
    pub item_height: u32,
}

impl ListView {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        ListView {
            rect: Rect::new(x, y, width, height),
            items: Vec::new(),
            selected: None,
            scroll_offset: 0,
            item_height: font::line_height_sized(Theme::WIDGET_FONT_SIZE) + 8,
        }
    }

    pub fn add_item(&mut self, text: &str) {
        self.items.push(String::from(text));
    }

    pub fn visible_items(&self) -> usize {
        (self.rect.height / self.item_height) as usize
    }

    pub fn click(&mut self, x: i32, y: i32) -> Option<usize> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let local_y = y - self.rect.y;
        let idx = self.scroll_offset + (local_y as u32 / self.item_height) as usize;
        if idx < self.items.len() {
            self.selected = Some(idx);
            Some(idx)
        } else {
            None
        }
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self.items.len().saturating_sub(self.visible_items());
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
        }
    }

    pub fn render(&self, surface: &mut Surface) {
        let mut renderer = Renderer::new(surface);

        // Background
        renderer.fill_rect(self.rect, Theme::INPUT_BG);
        // Border
        renderer.draw_rect(self.rect, Theme::INPUT_BORDER, 1);
        drop(renderer);

        let visible = self.visible_items();
        let end = (self.scroll_offset + visible).min(self.items.len());

        for (vi, idx) in (self.scroll_offset..end).enumerate() {
            let iy = self.rect.y + (vi as u32 * self.item_height) as i32;
            let item_rect = Rect::new(self.rect.x + 1, iy, self.rect.width - 2, self.item_height);

            // Highlight selected
            if self.selected == Some(idx) {
                let mut renderer = Renderer::new(surface);
                renderer.fill_rect(item_rect, Theme::ACCENT);
            }

            let text_color = if self.selected == Some(idx) {
                Color::WHITE
            } else {
                Theme::TEXT_COLOR
            };

            let size = Theme::WIDGET_FONT_SIZE;
            let lh = font::line_height_sized(size);
            let tx = self.rect.x + 8;
            let ty = iy + (self.item_height as i32 - lh as i32) / 2;
            font::draw_string_sized(surface, tx, ty, &self.items[idx], text_color, size);
        }
    }
}
