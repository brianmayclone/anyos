use alloc::string::String;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

pub struct Label {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub color: Color,
}

impl Label {
    pub fn new(x: i32, y: i32, text: &str) -> Self {
        Label {
            x,
            y,
            text: String::from(text),
            color: Theme::TEXT_COLOR,
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = String::from(text);
    }

    pub fn render(&self, surface: &mut Surface) {
        font::draw_string_sized(surface, self.x, self.y, &self.text, self.color, Theme::WIDGET_FONT_SIZE);
    }

    pub fn size(&self) -> (u32, u32) {
        font::measure_string_sized(&self.text, Theme::WIDGET_FONT_SIZE)
    }
}
