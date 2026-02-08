use alloc::string::String;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::event::HitTest;
use crate::ui::theme::Theme;

/// Minimum window dimensions
pub const MIN_WIDTH: u32 = 120;
pub const MIN_HEIGHT: u32 = 60;
/// Grab zone in pixels for resize edges
pub const RESIZE_BORDER: i32 = 5;

/// Window state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

/// A window with decorations
pub struct Window {
    pub id: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub state: WindowState,
    pub focused: bool,
    pub layer_id: u32,
    pub resizable: bool,
    /// Borderless window (no title bar, no decorations)
    pub borderless: bool,
    /// Always on top (e.g. dock, overlays)
    pub always_on_top: bool,
    /// Owner thread ID (for cleanup when process is killed)
    pub owner_tid: u32,
    /// The content surface (client area only)
    pub content: Surface,
    /// The full surface (decorations + content)
    surface: Surface,
    /// Whether the window needs redrawing
    pub dirty: bool,
    /// Saved position/size for maximize restore
    saved_rect: Option<Rect>,
}

impl Window {
    pub fn new(id: u32, title: &str, x: i32, y: i32, width: u32, height: u32) -> Self {
        let full_height = height + Theme::TITLEBAR_HEIGHT;
        Window {
            id,
            title: String::from(title),
            x,
            y,
            width,
            height,
            state: WindowState::Normal,
            focused: true,
            layer_id: 0,
            resizable: true,
            borderless: false,
            always_on_top: false,
            owner_tid: 0,
            content: Surface::new_with_color(width, height, Theme::WINDOW_BG),
            surface: Surface::new(width, full_height),
            dirty: true,
            saved_rect: None,
        }
    }

    /// Total height including title bar (or just content height if borderless)
    pub fn total_height(&self) -> u32 {
        if self.borderless { self.height } else { self.height + Theme::TITLEBAR_HEIGHT }
    }

    /// Bounds of the full window (including decorations)
    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.total_height())
    }

    /// Bounds of the client area in screen coordinates
    pub fn client_bounds(&self) -> Rect {
        if self.borderless {
            Rect::new(self.x, self.y, self.width, self.height)
        } else {
            Rect::new(
                self.x,
                self.y + Theme::TITLEBAR_HEIGHT as i32,
                self.width,
                self.height,
            )
        }
    }

    /// Hit test a point (in screen coordinates) against window regions
    pub fn hit_test(&self, px: i32, py: i32) -> HitTest {
        let bounds = self.bounds();
        if !bounds.contains(px, py) {
            return HitTest::None;
        }

        // Borderless windows have no decorations — everything is client area
        if self.borderless {
            return HitTest::Client;
        }

        // Check title bar area
        let titlebar = Rect::new(self.x, self.y, self.width, Theme::TITLEBAR_HEIGHT);
        if titlebar.contains(px, py) {
            // Check traffic light buttons
            let local_x = px - self.x;
            let local_y = py - self.y;
            let btn_y = Theme::BUTTON_Y_CENTER;
            let r = Theme::BUTTON_RADIUS;

            // Close button
            let close_x = Theme::BUTTON_LEFT_MARGIN;
            if (local_x - close_x).pow(2) + (local_y - btn_y).pow(2) <= r * r {
                return HitTest::CloseButton;
            }

            // Minimize button
            let min_x = close_x + Theme::BUTTON_SPACING;
            if (local_x - min_x).pow(2) + (local_y - btn_y).pow(2) <= r * r {
                return HitTest::MinimizeButton;
            }

            // Maximize button
            let max_x = min_x + Theme::BUTTON_SPACING;
            if (local_x - max_x).pow(2) + (local_y - btn_y).pow(2) <= r * r {
                return HitTest::MaximizeButton;
            }

            return HitTest::TitleBar;
        }

        // Check resize zones (only for normal, resizable windows)
        if self.resizable && self.state == WindowState::Normal {
            let local_x = px - self.x;
            let local_y = py - self.y;
            let total_h = self.total_height() as i32;
            let w = self.width as i32;
            let b = RESIZE_BORDER;

            let on_left = local_x < b;
            let on_right = local_x >= w - b;
            let on_bottom = local_y >= total_h - b;

            // Corners first (higher priority)
            if on_bottom && on_left {
                return HitTest::ResizeBottomLeft;
            }
            if on_bottom && on_right {
                return HitTest::ResizeBottomRight;
            }
            // Edges
            if on_left {
                return HitTest::ResizeLeft;
            }
            if on_right {
                return HitTest::ResizeRight;
            }
            if on_bottom {
                return HitTest::ResizeBottom;
            }
        }

        HitTest::Client
    }

    /// Render the window decorations + content into the full surface
    pub fn render(&mut self) -> &Surface {
        if !self.dirty {
            return &self.surface;
        }

        // Resize surface if needed
        let total_h = self.total_height();
        if self.surface.width != self.width || self.surface.height != total_h {
            self.surface = Surface::new(self.width, total_h);
        }

        // Borderless windows: just copy content directly
        if self.borderless {
            self.surface.pixels.copy_from_slice(&self.content.pixels);
            self.dirty = false;
            return &self.surface;
        }

        let size = Theme::WINDOW_TITLE_FONT_SIZE;

        let mut renderer = Renderer::new(&mut self.surface);

        // Draw window background with rounded top corners
        renderer.fill_rounded_rect(
            Rect::new(0, 0, self.width, total_h),
            Theme::WINDOW_BORDER_RADIUS,
            Theme::WINDOW_BG,
        );

        // Draw title bar
        let titlebar_color = if self.focused {
            Theme::TITLEBAR_BG
        } else {
            Theme::TITLEBAR_BG_INACTIVE
        };
        renderer.fill_rounded_rect(
            Rect::new(0, 0, self.width, Theme::TITLEBAR_HEIGHT + Theme::WINDOW_BORDER_RADIUS as u32),
            Theme::WINDOW_BORDER_RADIUS,
            titlebar_color,
        );
        // Square off the bottom of the title bar
        renderer.fill_rect(
            Rect::new(0, Theme::TITLEBAR_HEIGHT as i32 - Theme::WINDOW_BORDER_RADIUS, self.width, Theme::WINDOW_BORDER_RADIUS as u32),
            titlebar_color,
        );

        // Draw title text centered
        let text_color = if self.focused {
            Theme::TITLEBAR_TEXT
        } else {
            Theme::TITLEBAR_TEXT_INACTIVE
        };
        let (tw, _) = font::measure_string_sized(&self.title, size);
        let tx = (self.width as i32 - tw as i32) / 2;
        let lh = font::line_height_sized(size);
        let ty = (Theme::TITLEBAR_HEIGHT as i32 - lh as i32) / 2;
        drop(renderer);
        font::draw_string_sized(&mut self.surface, tx, ty, &self.title, text_color, size);

        // Draw traffic light buttons
        self.draw_traffic_lights();

        // Draw border line between title bar and content
        {
            let mut renderer = Renderer::new(&mut self.surface);
            renderer.fill_rect(
                Rect::new(0, Theme::TITLEBAR_HEIGHT as i32, self.width, 1),
                Theme::WINDOW_BORDER,
            );
        }

        // Blit content into the window surface below the title bar
        self.surface.blit(&self.content, 0, Theme::TITLEBAR_HEIGHT as i32);

        self.dirty = false;
        &self.surface
    }

    fn draw_traffic_lights(&mut self) {
        let btn_y = Theme::BUTTON_Y_CENTER;
        let r = Theme::BUTTON_RADIUS;
        let mut renderer = Renderer::new(&mut self.surface);

        if self.focused {
            // Close (red)
            renderer.fill_circle(
                Theme::BUTTON_LEFT_MARGIN,
                btn_y,
                r,
                Theme::BUTTON_CLOSE,
            );
            // Minimize (yellow)
            renderer.fill_circle(
                Theme::BUTTON_LEFT_MARGIN + Theme::BUTTON_SPACING,
                btn_y,
                r,
                Theme::BUTTON_MINIMIZE,
            );
            // Maximize (green)
            renderer.fill_circle(
                Theme::BUTTON_LEFT_MARGIN + 2 * Theme::BUTTON_SPACING,
                btn_y,
                r,
                Theme::BUTTON_MAXIMIZE,
            );
        } else {
            // Inactive buttons (gray)
            for i in 0..3 {
                renderer.fill_circle(
                    Theme::BUTTON_LEFT_MARGIN + i * Theme::BUTTON_SPACING,
                    btn_y,
                    r,
                    Theme::BUTTON_INACTIVE,
                );
            }
        }
    }

    /// Get a reference to the rendered surface
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Mark the window as needing redraw
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Move the window to a new position
    pub fn move_to(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    /// Resize the window content area, preserving existing content
    pub fn resize(&mut self, width: u32, height: u32) {
        let old_content = core::mem::replace(
            &mut self.content,
            Surface::new_with_color(width, height, Theme::WINDOW_BG),
        );
        // Blit old content into new surface (clipped if shrinking, padded if growing)
        self.content.blit(&old_content, 0, 0);
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

    /// Toggle maximized state
    pub fn toggle_maximize(&mut self, screen_width: u32, screen_height: u32) {
        match self.state {
            WindowState::Normal => {
                self.saved_rect = Some(Rect::new(self.x, self.y, self.width, self.height));
                self.x = 0;
                self.y = Theme::MENUBAR_HEIGHT as i32;
                self.width = screen_width;
                self.height = screen_height - Theme::MENUBAR_HEIGHT - Theme::TITLEBAR_HEIGHT;
                self.state = WindowState::Maximized;
                self.dirty = true;
            }
            WindowState::Maximized => {
                if let Some(rect) = self.saved_rect {
                    self.x = rect.x;
                    self.y = rect.y;
                    self.width = rect.width;
                    self.height = rect.height;
                }
                self.state = WindowState::Normal;
                self.dirty = true;
            }
            WindowState::Minimized => {}
        }
    }
}
