use alloc::string::String;
use alloc::vec::Vec;
use crate::graphics::color::Color;
use crate::graphics::icon::{self, Icon};
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;

/// Action to perform when a dock item is clicked
pub enum DockAction {
    /// Focus an existing window
    Focus(u32),
    /// Launch a new program (bin_path, name)
    Launch(String, String),
}

/// A dock item (app shortcut)
pub struct DockItem {
    pub name: String,
    pub bin_path: String,
    pub icon: Option<Icon>,
    pub running: bool,
}

/// macOS-like dock at the bottom of the screen
pub struct Dock {
    screen_width: u32,
    items: Vec<DockItem>,
    icons_loaded: bool,
}

impl Dock {
    pub fn new(screen_width: u32) -> Self {
        let mut dock = Dock {
            screen_width,
            items: Vec::new(),
            icons_loaded: false,
        };

        // Default dock items (name must match the window title created by the app)
        dock.add_item("Terminal", "/system/terminal", "/system/icons/terminal.icon");
        dock.add_item("Activity Monitor", "/system/taskmanager", "/system/icons/taskmanager.icon");
        dock.add_item("Settings", "/system/settings", "/system/icons/settings.icon");

        dock
    }

    fn add_item(&mut self, name: &str, bin_path: &str, icon_path: &str) {
        self.items.push(DockItem {
            name: String::from(name),
            bin_path: String::from(bin_path),
            icon: None, // Loaded later after VFS is ready
            running: false,
        });
        // Store icon path in a way we can load later
        // (We can't load from VFS during new() because VFS isn't initialized yet)
        let _ = icon_path; // used in load_icons()
    }

    /// Load icon files from the filesystem. Call after VFS is initialized.
    pub fn load_icons(&mut self) {
        if self.icons_loaded {
            return;
        }

        let icon_paths = [
            "/system/icons/terminal.icon",
            "/system/icons/taskmanager.icon",
            "/system/icons/settings.icon",
        ];

        for (item, path) in self.items.iter_mut().zip(icon_paths.iter()) {
            match icon::load_icon(path) {
                Some(ic) => {
                    crate::serial_println!("  Dock: loaded icon '{}' ({}x{})", path, ic.width, ic.height);
                    item.icon = Some(ic);
                }
                None => {
                    crate::serial_println!("  Dock: icon not found: {}", path);
                }
            }
        }

        self.icons_loaded = true;
    }

    /// Determine what action to take when a dock item is clicked.
    pub fn launch_or_focus(&self, idx: usize, windows: &[crate::ui::window::Window]) -> Option<DockAction> {
        let item = self.items.get(idx)?;

        if item.bin_path.is_empty() {
            return None;
        }

        // Check if there's a window matching this dock item (by title)
        for window in windows.iter().rev() {
            if window.title == item.name {
                return Some(DockAction::Focus(window.id));
            }
        }

        // No matching window — launch the program
        Some(DockAction::Launch(item.bin_path.clone(), item.name.clone()))
    }

    /// Update running indicators based on current windows
    pub fn sync_running(&mut self, windows: &[crate::ui::window::Window]) {
        for item in &mut self.items {
            item.running = windows.iter().any(|w| w.title == item.name);
        }
    }

    pub fn render(&self, surface: &mut Surface) {
        let item_count = self.items.len() as u32;
        if item_count == 0 {
            return;
        }

        let icon_size = Theme::DOCK_ICON_SIZE;
        let spacing = Theme::DOCK_ICON_SPACING;
        let h_padding = 12u32;
        let total_width = item_count * icon_size
            + (item_count - 1) * spacing
            + h_padding * 2;

        let dock_x = (self.screen_width as i32 - total_width as i32) / 2;
        let dock_y = Theme::DOCK_MARGIN_BOTTOM as i32;

        let mut renderer = Renderer::new(surface);

        // Dock background — translucent glass pill
        renderer.fill_rounded_rect(
            Rect::new(dock_x, dock_y, total_width, Theme::DOCK_HEIGHT),
            Theme::DOCK_BORDER_RADIUS,
            Theme::DOCK_BG,
        );

        // Top highlight line for depth (1px lighter at top)
        renderer.fill_rect(
            Rect::new(dock_x + Theme::DOCK_BORDER_RADIUS, dock_y,
                       total_width - Theme::DOCK_BORDER_RADIUS as u32 * 2, 1),
            Color::with_alpha(25, 255, 255, 255),
        );

        // Subtle border
        renderer.draw_rect(
            Rect::new(dock_x, dock_y, total_width, Theme::DOCK_HEIGHT),
            Color::with_alpha(30, 255, 255, 255),
            1,
        );

        drop(renderer);

        // Draw each item — icons float directly on the glass background
        let icon_y = dock_y + ((Theme::DOCK_HEIGHT as i32 - icon_size as i32) / 2) - 2;
        let mut ix = dock_x + h_padding as i32;

        for item in &self.items {
            // Draw icon
            if let Some(ref ic) = item.icon {
                ic.draw_on(surface, ix, icon_y);
            } else {
                // Fallback: draw a rounded-rect placeholder with first letter
                let mut renderer = Renderer::new(surface);
                renderer.fill_rounded_rect(
                    Rect::new(ix, icon_y, icon_size, icon_size),
                    10,
                    Color::new(60, 60, 65),
                );
                drop(renderer);

                // Draw first letter of name centered
                let ch = item.name.chars().next().unwrap_or('?');
                let font_size = Theme::FONT_SIZE_LARGE;
                let lh = crate::graphics::font::line_height_sized(font_size);
                let cx = ix + icon_size as i32 / 2 - 5;
                let cy = icon_y + (icon_size as i32 - lh as i32) / 2;
                crate::graphics::font::draw_char_sized(surface, cx, cy, ch, Color::WHITE, font_size);
            }

            // Running indicator dot
            if item.running {
                let dot_x = ix + icon_size as i32 / 2;
                let dot_y = icon_y + icon_size as i32 + 5;
                let mut renderer = Renderer::new(surface);
                renderer.fill_circle(dot_x, dot_y, 2, Color::WHITE);
            }

            ix += (icon_size + spacing) as i32;
        }
    }

    /// Hit test: returns dock item index at screen position
    pub fn hit_test(&self, x: i32, y: i32, screen_height: u32) -> Option<usize> {
        let item_count = self.items.len() as u32;
        if item_count == 0 {
            return None;
        }

        let icon_size = Theme::DOCK_ICON_SIZE;
        let spacing = Theme::DOCK_ICON_SPACING;
        let h_padding = 12u32;
        let total_width = item_count * icon_size
            + (item_count - 1) * spacing
            + h_padding * 2;

        let dock_x = (self.screen_width as i32 - total_width as i32) / 2;
        let dock_y = screen_height as i32 - Theme::DOCK_HEIGHT as i32 - Theme::DOCK_MARGIN_BOTTOM as i32;

        let dock_rect = Rect::new(dock_x, dock_y, total_width, Theme::DOCK_HEIGHT);
        if !dock_rect.contains(x, y) {
            return None;
        }

        let local_x = x - dock_x - h_padding as i32;
        if local_x < 0 {
            return None;
        }

        let item_stride = icon_size + spacing;
        let idx = local_x as u32 / item_stride;
        if idx < item_count {
            Some(idx as usize)
        } else {
            None
        }
    }
}
