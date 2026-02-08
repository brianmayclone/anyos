use alloc::string::String;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hover,
    Pressed,
}

pub struct Button {
    pub rect: Rect,
    pub label: String,
    pub state: ButtonState,
    pub primary: bool, // Blue accent button
}

impl Button {
    pub fn new(x: i32, y: i32, width: u32, height: u32, label: &str) -> Self {
        Button {
            rect: Rect::new(x, y, width, height),
            label: String::from(label),
            state: ButtonState::Normal,
            primary: false,
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn render(&self, surface: &mut Surface) {
        let bg = if self.primary {
            match self.state {
                ButtonState::Normal => Theme::ACCENT,
                ButtonState::Hover => Color::new(10, 132, 255),
                ButtonState::Pressed => Color::new(0, 100, 220),
            }
        } else {
            match self.state {
                ButtonState::Normal => Theme::BUTTON_BG,
                ButtonState::Hover => Theme::BUTTON_BG_HOVER,
                ButtonState::Pressed => Theme::BUTTON_BG_PRESSED,
            }
        };

        let mut renderer = Renderer::new(surface);
        renderer.fill_rounded_rect(self.rect, 6, bg);

        // Border
        renderer.draw_rect(self.rect, Color::with_alpha(40, 255, 255, 255), 1);
        drop(renderer);

        // Label centered
        let size = Theme::WIDGET_FONT_SIZE;
        let (tw, th) = font::measure_string_sized(&self.label, size);
        let tx = self.rect.x + (self.rect.width as i32 - tw as i32) / 2;
        let ty = self.rect.y + (self.rect.height as i32 - th as i32) / 2;
        font::draw_string_sized(surface, tx, ty, &self.label, Color::WHITE, size);
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.rect.contains(x, y)
    }
}
