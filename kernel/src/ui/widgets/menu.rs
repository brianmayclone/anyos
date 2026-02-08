use alloc::string::String;
use alloc::vec::Vec;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

pub enum MenuItem {
    Action { label: String, shortcut: Option<String> },
    Separator,
}

pub struct Menu {
    pub x: i32,
    pub y: i32,
    pub items: Vec<MenuItem>,
    pub highlight: Option<usize>,
    pub visible: bool,
    width: u32,
}

impl Menu {
    pub fn new(x: i32, y: i32) -> Self {
        Menu {
            x,
            y,
            items: Vec::new(),
            highlight: None,
            visible: false,
            width: 200,
        }
    }

    pub fn add_action(&mut self, label: &str, shortcut: Option<&str>) {
        self.items.push(MenuItem::Action {
            label: String::from(label),
            shortcut: shortcut.map(String::from),
        });
        // Adjust width
        let size = Theme::MENU_FONT_SIZE;
        let (w, _) = font::measure_string_sized(label, size);
        let extra = shortcut.map(|s| font::measure_string_sized(s, size).0 + 40).unwrap_or(0);
        let needed = w + extra + 40;
        if needed > self.width {
            self.width = needed;
        }
    }

    pub fn add_separator(&mut self) {
        self.items.push(MenuItem::Separator);
    }

    pub fn total_height(&self) -> u32 {
        let mut h = Theme::MENU_PADDING * 2;
        for item in &self.items {
            match item {
                MenuItem::Action { .. } => h += Theme::MENU_ITEM_HEIGHT,
                MenuItem::Separator => h += 9,
            }
        }
        h
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.total_height())
    }

    pub fn hit_test(&self, px: i32, py: i32) -> Option<usize> {
        if !self.bounds().contains(px, py) {
            return None;
        }

        let local_y = py - self.y - Theme::MENU_PADDING as i32;
        if local_y < 0 {
            return None;
        }

        let mut y = 0i32;
        for (i, item) in self.items.iter().enumerate() {
            let h = match item {
                MenuItem::Action { .. } => Theme::MENU_ITEM_HEIGHT as i32,
                MenuItem::Separator => 9,
            };
            if local_y >= y && local_y < y + h {
                return match item {
                    MenuItem::Action { .. } => Some(i),
                    MenuItem::Separator => None,
                };
            }
            y += h;
        }
        None
    }

    pub fn render(&self, surface: &mut Surface) {
        if !self.visible {
            return;
        }

        let bounds = self.bounds();
        let mut renderer = Renderer::new(surface);

        // Shadow
        renderer.fill_rounded_rect(bounds.offset(2, 2), 8, Color::with_alpha(60, 0, 0, 0));
        // Background
        renderer.fill_rounded_rect(bounds, 8, Theme::MENU_BG);
        // Border
        renderer.draw_rect(bounds, Color::with_alpha(40, 255, 255, 255), 1);
        drop(renderer);

        let size = Theme::MENU_FONT_SIZE;
        let lh = font::line_height_sized(size);
        let mut iy = self.y + Theme::MENU_PADDING as i32;
        for (i, item) in self.items.iter().enumerate() {
            match item {
                MenuItem::Action { label, shortcut } => {
                    let item_rect = Rect::new(
                        self.x + 4,
                        iy,
                        self.width - 8,
                        Theme::MENU_ITEM_HEIGHT,
                    );

                    if self.highlight == Some(i) {
                        let mut renderer = Renderer::new(surface);
                        renderer.fill_rounded_rect(item_rect, 4, Theme::MENU_HIGHLIGHT);
                    }

                    let text_color = Theme::MENU_TEXT;
                    let ty = iy + (Theme::MENU_ITEM_HEIGHT as i32 - lh as i32) / 2;
                    font::draw_string_sized(surface, self.x + 16, ty, label, text_color, size);

                    if let Some(sc) = shortcut {
                        let (sw, _) = font::measure_string_sized(sc, size);
                        let sx = self.x + self.width as i32 - sw as i32 - 16;
                        font::draw_string_sized(surface, sx, ty, sc, Theme::MENU_TEXT_DIM, size);
                    }

                    iy += Theme::MENU_ITEM_HEIGHT as i32;
                }
                MenuItem::Separator => {
                    let mut renderer = Renderer::new(surface);
                    renderer.fill_rect(
                        Rect::new(self.x + 8, iy + 4, self.width - 16, 1),
                        Theme::MENU_SEPARATOR,
                    );
                    iy += 9;
                }
            }
        }
    }
}
