use alloc::string::String;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;
use crate::ui::widgets::button::Button;

pub struct Dialog {
    pub title: String,
    pub message: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub buttons: [Option<Button>; 3],
}

impl Dialog {
    pub fn new(title: &str, message: &str) -> Self {
        Dialog {
            title: String::from(title),
            message: String::from(message),
            x: 0,
            y: 0,
            width: 400,
            height: 160,
            buttons: [None, None, None],
        }
    }

    pub fn center(&mut self, screen_width: u32, screen_height: u32) {
        self.x = (screen_width as i32 - self.width as i32) / 2;
        self.y = (screen_height as i32 - self.height as i32) / 2;
    }

    pub fn add_button(&mut self, label: &str, primary: bool) -> usize {
        for (i, slot) in self.buttons.iter_mut().enumerate() {
            if slot.is_none() {
                let btn_w = 80u32;
                let btn_h = 28u32;
                let btn_x = self.width as i32 - (i as i32 + 1) * (btn_w as i32 + 12) - 8;
                let btn_y = self.height as i32 - btn_h as i32 - 16;
                let mut btn = Button::new(btn_x, btn_y, btn_w, btn_h, label);
                if primary {
                    btn = btn.primary();
                }
                *slot = Some(btn);
                return i;
            }
        }
        0
    }

    pub fn render(&self, surface: &mut Surface) {
        let mut renderer = Renderer::new(surface);

        // Shadow
        renderer.fill_rounded_rect(
            Rect::new(self.x + 4, self.y + 4, self.width, self.height),
            12,
            Color::with_alpha(100, 0, 0, 0),
        );

        // Background
        let dialog_rect = Rect::new(self.x, self.y, self.width, self.height);
        renderer.fill_rounded_rect(dialog_rect, 12, Theme::WINDOW_BG);
        renderer.draw_rect(dialog_rect, Theme::WINDOW_BORDER, 1);
        drop(renderer);

        // Title
        let size = Theme::WIDGET_FONT_SIZE;
        let tx = self.x + 20;
        let ty = self.y + 20;
        font::draw_string_sized(surface, tx, ty, &self.title, Color::WHITE, size);

        // Message
        let mx = self.x + 20;
        let lh = font::line_height_sized(size);
        let my = ty + lh as i32 + 8;
        font::draw_string_sized(surface, mx, my, &self.message, Theme::TEXT_DIM, size);

        // Buttons (offset to dialog position)
        for btn_opt in &self.buttons {
            if let Some(btn) = btn_opt {
                // Offset button coordinates relative to dialog
                let offset_btn = Button::new(
                    btn.rect.x + self.x,
                    btn.rect.y + self.y,
                    btn.rect.width,
                    btn.rect.height,
                    &btn.label,
                );
                offset_btn.render(surface);
            }
        }
    }
}
