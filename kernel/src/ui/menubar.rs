//! macOS-style global menu bar at the top of the screen. Displays the Apple
//! menu, active application name, dropdown menus (File, Edit, View, Window),
//! and a real-time clock.

use alloc::string::String;
use alloc::vec::Vec;
use crate::drivers::rtc;
use crate::graphics::color::Color;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::graphics::surface::Surface;
use crate::ui::theme::Theme;
use crate::ui::widgets::menu::{Menu, MenuItem};

/// Action returned when a menu item is clicked
pub enum MenuAction {
    /// Close the focused window
    CloseWindow,
    /// Launch an app by path
    LaunchApp(String),
    /// Show About dialog (future)
    About,
    /// Restart the system (future)
    Restart,
    /// Shut down the system (future)
    Shutdown,
}

/// A menu bar item: label + dropdown menu + cached pixel position
struct MenuBarItem {
    label: String,
    menu: Menu,
    x_start: i32,
    x_end: i32,
}

/// Global menu bar (macOS-style, always at top)
pub struct MenuBar {
    width: u32,
    app_name: String,
    items: Vec<MenuBarItem>,
    /// Currently open dropdown index (None = all closed)
    active_menu: Option<usize>,
    /// Cached y for label text
    text_y: i32,
    /// Cached x position where menu labels start (after app name)
    labels_start_x: i32,
}

impl MenuBar {
    /// Create a new menu bar with default menus for the given screen width.
    pub fn new(screen_width: u32) -> Self {
        let mut bar = MenuBar {
            width: screen_width,
            app_name: String::from("Finder"),
            items: Vec::new(),
            active_menu: None,
            text_y: 0,
            labels_start_x: 0,
        };
        bar.build_default_menus();
        bar.recalc_positions();
        bar
    }

    /// Set the active application name displayed in the menu bar.
    pub fn set_app_name(&mut self, name: &str) {
        self.app_name = String::from(name);
        self.recalc_positions();
    }

    /// Returns true if any dropdown menu is currently open.
    pub fn is_menu_open(&self) -> bool {
        self.active_menu.is_some()
    }

    /// Build the default menus: Apple, File, Edit, View, Window
    fn build_default_menus(&mut self) {
        // Apple menu (@)
        let mut apple = Menu::new(0, 0);
        apple.add_action("About .anyOS", None);
        apple.add_separator();
        apple.add_action("Settings...", None);
        apple.add_separator();
        apple.add_action("Restart", None);
        apple.add_action("Shut Down", None);
        self.items.push(MenuBarItem {
            label: String::from("\x00"), // sentinel for Apple icon
            menu: apple,
            x_start: 0,
            x_end: 0,
        });

        // File
        let mut file = Menu::new(0, 0);
        file.add_action("New Terminal", Some("\u{2318}T"));
        file.add_separator();
        file.add_action("Close Window", Some("\u{2318}Q"));
        self.items.push(MenuBarItem {
            label: String::from("File"),
            menu: file,
            x_start: 0,
            x_end: 0,
        });

        // Edit
        let mut edit = Menu::new(0, 0);
        edit.add_action("Cut", Some("\u{2318}X"));
        edit.add_action("Copy", Some("\u{2318}C"));
        edit.add_action("Paste", Some("\u{2318}V"));
        self.items.push(MenuBarItem {
            label: String::from("Edit"),
            menu: edit,
            x_start: 0,
            x_end: 0,
        });

        // View
        let mut view = Menu::new(0, 0);
        view.add_action("Show All", None);
        self.items.push(MenuBarItem {
            label: String::from("View"),
            menu: view,
            x_start: 0,
            x_end: 0,
        });

        // Window
        let mut window = Menu::new(0, 0);
        window.add_action("Minimize", None);
        window.add_action("Maximize", None);
        self.items.push(MenuBarItem {
            label: String::from("Window"),
            menu: window,
            x_start: 0,
            x_end: 0,
        });
    }

    /// Recalculate cached x positions for all menu labels
    fn recalc_positions(&mut self) {
        let size = Theme::MENUBAR_FONT_SIZE;
        let lh = font::line_height_sized(size);
        self.text_y = (Theme::MENUBAR_HEIGHT as i32 - lh as i32) / 2;

        // Apple icon at x=10
        let apple_x = 10;
        let apple_w = 16i32; // icon width

        if !self.items.is_empty() {
            self.items[0].x_start = apple_x;
            self.items[0].x_end = apple_x + apple_w;
            self.items[0].menu.x = apple_x;
            self.items[0].menu.y = Theme::MENUBAR_HEIGHT as i32;
        }

        // App name (bold) after apple
        let app_x = apple_x + apple_w + 10;
        let (app_w, _) = font::measure_string_sized(&self.app_name, size);

        // Menu labels start after app name
        let mut mx = app_x + app_w as i32 + 16;
        self.labels_start_x = mx;

        for item in self.items.iter_mut().skip(1) {
            let (label_w, _) = font::measure_string_sized(&item.label, size);
            item.x_start = mx - 8; // padding
            item.x_end = mx + label_w as i32 + 8;
            item.menu.x = mx - 8;
            item.menu.y = Theme::MENUBAR_HEIGHT as i32;
            mx += label_w as i32 + 16;
        }
    }

    /// Hit test: returns which menu bar label index was clicked, if any
    pub fn hit_test_label(&self, x: i32, y: i32) -> Option<usize> {
        if y < 0 || y >= Theme::MENUBAR_HEIGHT as i32 {
            return None;
        }
        for (i, item) in self.items.iter().enumerate() {
            if x >= item.x_start && x < item.x_end {
                return Some(i);
            }
        }
        None
    }

    /// Handle a click at (x, y). Returns Some(MenuAction) if a menu item was activated.
    pub fn handle_click(&mut self, x: i32, y: i32) -> Option<MenuAction> {
        // If a dropdown is open, check if click is inside the dropdown
        if let Some(active_idx) = self.active_menu {
            let menu = &self.items[active_idx].menu;
            if menu.bounds().contains(x, y) {
                // Click inside dropdown — resolve which item
                if let Some(item_idx) = menu.hit_test(x, y) {
                    let action = self.resolve_action(active_idx, item_idx);
                    self.close_menu();
                    return action;
                }
                // Clicked on separator or padding — just consume
                return None;
            }

            // Click on menu bar labels (possibly switching menus)
            if y < Theme::MENUBAR_HEIGHT as i32 {
                if let Some(label_idx) = self.hit_test_label(x, y) {
                    if label_idx == active_idx {
                        // Toggle close
                        self.close_menu();
                    } else {
                        // Switch to different menu
                        self.open_menu(label_idx);
                    }
                    return None;
                }
            }

            // Click outside both menu and menu bar — close
            self.close_menu();
            return None;
        }

        // No menu open — check if clicking a menu bar label
        if y < Theme::MENUBAR_HEIGHT as i32 {
            if let Some(label_idx) = self.hit_test_label(x, y) {
                self.open_menu(label_idx);
                return None;
            }
        }

        None
    }

    /// Handle mouse move for hover highlighting in open dropdowns
    pub fn handle_mouse_move(&mut self, x: i32, y: i32) {
        if let Some(active_idx) = self.active_menu {
            // Check if hovering over a different menu bar label
            if y < Theme::MENUBAR_HEIGHT as i32 {
                if let Some(label_idx) = self.hit_test_label(x, y) {
                    if label_idx != active_idx {
                        self.open_menu(label_idx);
                        return;
                    }
                }
            }

            // Update hover highlight in the dropdown
            let menu = &mut self.items[active_idx].menu;
            let new_highlight = menu.hit_test(x, y);
            if menu.highlight != new_highlight {
                menu.highlight = new_highlight;
            }
        }
    }

    fn open_menu(&mut self, idx: usize) {
        // Close any previously open menu
        if let Some(prev) = self.active_menu {
            self.items[prev].menu.visible = false;
            self.items[prev].menu.highlight = None;
        }
        self.active_menu = Some(idx);
        self.items[idx].menu.visible = true;
    }

    fn close_menu(&mut self) {
        if let Some(idx) = self.active_menu {
            self.items[idx].menu.visible = false;
            self.items[idx].menu.highlight = None;
        }
        self.active_menu = None;
    }

    /// Resolve a menu item click into a MenuAction
    fn resolve_action(&self, menu_idx: usize, item_idx: usize) -> Option<MenuAction> {
        let item = &self.items[menu_idx].menu.items[item_idx];
        match item {
            MenuItem::Action { label, .. } => {
                let label = label.as_str();
                match label {
                    "About .anyOS" => Some(MenuAction::About),
                    "Settings..." => Some(MenuAction::LaunchApp(String::from("/system/settings"))),
                    "Restart" => Some(MenuAction::Restart),
                    "Shut Down" => Some(MenuAction::Shutdown),
                    "New Terminal" => Some(MenuAction::LaunchApp(String::from("/system/terminal"))),
                    "Close Window" => Some(MenuAction::CloseWindow),
                    "Minimize" => None, // TODO
                    "Maximize" => None, // TODO
                    _ => None,
                }
            }
            MenuItem::Separator => None,
        }
    }

    /// Render the menu bar onto the given surface
    pub fn render(&self, surface: &mut Surface) {
        let size = Theme::MENUBAR_FONT_SIZE;
        let mut renderer = Renderer::new(surface);

        // Background
        renderer.fill_rect(
            Rect::new(0, 0, self.width, Theme::MENUBAR_HEIGHT),
            Theme::MENUBAR_BG,
        );

        // Bottom border
        renderer.fill_rect(
            Rect::new(0, Theme::MENUBAR_HEIGHT as i32 - 1, self.width, 1),
            Color::new(60, 60, 60),
        );

        drop(renderer);

        // Apple menu icon (@)
        if !self.items.is_empty() {
            let apple_item = &self.items[0];
            // Highlight if active
            if self.active_menu == Some(0) {
                let mut renderer = Renderer::new(surface);
                renderer.fill_rounded_rect(
                    Rect::new(apple_item.x_start, 2, (apple_item.x_end - apple_item.x_start) as u32, Theme::MENUBAR_HEIGHT - 4),
                    4,
                    Theme::MENUBAR_HIGHLIGHT,
                );
            }
            font::draw_char_sized(surface, apple_item.x_start + 2, self.text_y, '@', Theme::MENUBAR_TEXT, size);
        }

        // App name (bold = brighter)
        let apple_end = if !self.items.is_empty() { self.items[0].x_end } else { 26 };
        let app_x = apple_end + 10;
        font::draw_string_sized(surface, app_x, self.text_y, &self.app_name, Color::WHITE, size);

        // Menu labels
        for (i, item) in self.items.iter().enumerate().skip(1) {
            // Highlight active label
            if self.active_menu == Some(i) {
                let mut renderer = Renderer::new(surface);
                renderer.fill_rounded_rect(
                    Rect::new(item.x_start, 2, (item.x_end - item.x_start) as u32, Theme::MENUBAR_HEIGHT - 4),
                    4,
                    Theme::MENUBAR_HIGHLIGHT,
                );
            }

            let label_x = item.x_start + 8; // inner padding
            font::draw_string_sized(surface, label_x, self.text_y, &item.label, Theme::MENUBAR_TEXT, size);
        }

        // Clock on the right
        let time = rtc::read_time();
        let mut time_buf = [0u8; 8];
        let time_str = format_time(&time, &mut time_buf);
        let (tw, _) = font::measure_string_sized(time_str, size);
        let tx = self.width as i32 - tw as i32 - 14;
        font::draw_string_sized(surface, tx, self.text_y, time_str, Theme::MENUBAR_TEXT, size);
    }

    /// Render the active dropdown menu onto a surface, offset so (origin_x, origin_y)
    /// maps to (0,0) in the surface. Used when the surface is a layer positioned at the
    /// dropdown bounds.
    pub fn render_dropdown_at(&self, surface: &mut Surface, origin_x: i32, origin_y: i32) {
        if let Some(idx) = self.active_menu {
            let menu = &self.items[idx].menu;
            // Temporarily adjust menu position for local rendering
            let local_x = menu.x - origin_x;
            let local_y = menu.y - origin_y;
            // Render using a shifted copy approach
            render_menu_at(surface, menu, local_x, local_y);
        }
    }

    /// Get the bounds of the active dropdown (for compositor layer sizing)
    pub fn active_dropdown_bounds(&self) -> Option<Rect> {
        if let Some(idx) = self.active_menu {
            let bounds = self.items[idx].menu.bounds();
            // Add padding for shadow
            Some(Rect::new(bounds.x, bounds.y, bounds.width + 4, bounds.height + 4))
            } else {
            None
        }
    }
}

/// Render a menu at local coordinates (shifted from menu's global position).
fn render_menu_at(surface: &mut Surface, menu: &Menu, x: i32, y: i32) {
    let size = Theme::MENU_FONT_SIZE;
    let lh = font::line_height_sized(size);
    let width = menu.bounds().width;
    let total_height = menu.total_height();
    let bounds = Rect::new(x, y, width, total_height);

    let mut renderer = Renderer::new(surface);

    // Shadow
    renderer.fill_rounded_rect(bounds.offset(2, 2), 8, Color::with_alpha(60, 0, 0, 0));
    // Background
    renderer.fill_rounded_rect(bounds, 8, Theme::MENU_BG);
    // Border
    renderer.draw_rect(bounds, Color::with_alpha(40, 255, 255, 255), 1);
    drop(renderer);

    let mut iy = y + Theme::MENU_PADDING as i32;
    for (i, item) in menu.items.iter().enumerate() {
        match item {
            MenuItem::Action { label, shortcut } => {
                let item_rect = Rect::new(
                    x + 4, iy, width - 8, Theme::MENU_ITEM_HEIGHT,
                );

                if menu.highlight == Some(i) {
                    let mut renderer = Renderer::new(surface);
                    renderer.fill_rounded_rect(item_rect, 4, Theme::MENU_HIGHLIGHT);
                }

                let ty = iy + (Theme::MENU_ITEM_HEIGHT as i32 - lh as i32) / 2;
                font::draw_string_sized(surface, x + 16, ty, label, Theme::MENU_TEXT, size);

                if let Some(sc) = shortcut {
                    let (sw, _) = font::measure_string_sized(sc, size);
                    let sx = x + width as i32 - sw as i32 - 16;
                    font::draw_string_sized(surface, sx, ty, sc, Theme::MENU_TEXT_DIM, size);
                }

                iy += Theme::MENU_ITEM_HEIGHT as i32;
            }
            MenuItem::Separator => {
                let mut renderer = Renderer::new(surface);
                renderer.fill_rect(
                    Rect::new(x + 8, iy + 4, width - 16, 1),
                    Theme::MENU_SEPARATOR,
                );
                iy += 9;
            }
        }
    }
}

fn format_time<'a>(time: &rtc::RtcTime, buf: &'a mut [u8; 8]) -> &'a str {
    buf[0] = b'0' + time.hours / 10;
    buf[1] = b'0' + time.hours % 10;
    buf[2] = b':';
    buf[3] = b'0' + time.minutes / 10;
    buf[4] = b'0' + time.minutes % 10;
    buf[5] = b':';
    buf[6] = b'0' + time.seconds / 10;
    buf[7] = b'0' + time.seconds % 10;
    core::str::from_utf8(buf).unwrap_or("??:??:??")
}
