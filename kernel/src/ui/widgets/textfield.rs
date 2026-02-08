//! Single-line text input field with cursor, keyboard editing, placeholder
//! text, and focus-dependent border highlighting.

use alloc::string::String;
use crate::drivers::input::keyboard::Key;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

/// A single-line text input field with cursor and placeholder support.
pub struct TextField {
    pub rect: Rect,
    pub text: String,
    pub placeholder: String,
    pub focused: bool,
    pub cursor_pos: usize,
    pub scroll_offset: usize,
}

impl TextField {
    /// Create a new empty text field at the given position and size.
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        TextField {
            rect: Rect::new(x, y, width, height),
            text: String::new(),
            placeholder: String::new(),
            focused: false,
            cursor_pos: 0,
            scroll_offset: 0,
        }
    }

    /// Builder method: set placeholder text shown when the field is empty and unfocused.
    pub fn with_placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder = String::from(placeholder);
        self
    }

    /// Process a key press: insert characters, handle backspace/delete, and move cursor.
    pub fn handle_key(&mut self, key: Key) {
        match key {
            Key::Char(ch) => {
                self.text.insert(self.cursor_pos, ch);
                self.cursor_pos += 1;
            }
            Key::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.text.remove(self.cursor_pos);
                }
            }
            Key::Delete => {
                if self.cursor_pos < self.text.len() {
                    self.text.remove(self.cursor_pos);
                }
            }
            Key::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
            }
            Key::Right => {
                if self.cursor_pos < self.text.len() {
                    self.cursor_pos += 1;
                }
            }
            Key::Home => self.cursor_pos = 0,
            Key::End => self.cursor_pos = self.text.len(),
            _ => {}
        }
    }

    /// Render the text field onto the given surface, including text and cursor.
    pub fn render(&self, surface: &mut Surface) {
        let border_color = if self.focused {
            Theme::INPUT_BORDER_FOCUS
        } else {
            Theme::INPUT_BORDER
        };

        let mut renderer = Renderer::new(surface);

        // Background
        renderer.fill_rounded_rect(self.rect, 4, Theme::INPUT_BG);
        // Border
        renderer.draw_rect(self.rect, border_color, 1);
        drop(renderer);

        let size = Theme::WIDGET_FONT_SIZE;
        let lh = font::line_height_sized(size);
        let text_x = self.rect.x + 6;
        let text_y = self.rect.y + (self.rect.height as i32 - lh as i32) / 2;

        if self.text.is_empty() && !self.focused {
            font::draw_string_sized(surface, text_x, text_y, &self.placeholder, Theme::TEXT_DIM, size);
        } else {
            font::draw_string_sized(surface, text_x, text_y, &self.text, Theme::TEXT_COLOR, size);

            // Draw cursor (measure text up to cursor position for proportional font)
            if self.focused {
                let cursor_text = if self.cursor_pos <= self.text.len() {
                    &self.text[..self.cursor_pos]
                } else {
                    &self.text
                };
                let (cursor_w, _) = font::measure_string_sized(cursor_text, size);
                let cursor_x = text_x + cursor_w as i32;
                let mut renderer = Renderer::new(surface);
                renderer.fill_rect(
                    Rect::new(cursor_x, text_y, 1, lh),
                    Theme::TEXT_COLOR,
                );
            }
        }
    }

    /// Returns true if the point (x, y) is inside the text field bounds.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.rect.contains(x, y)
    }
}
