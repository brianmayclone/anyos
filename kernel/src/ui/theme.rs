//! Dark theme color and dimension constants used throughout the UI.
//! Defines colors for desktop, menu bar, window chrome, dock, widgets, and menus,
//! as well as standard font sizes.

use crate::graphics::color::Color;

/// Central collection of macOS-inspired dark theme constants for colors,
/// dimensions, border radii, and font sizes.
pub struct Theme;

impl Theme {
    // Desktop
    pub const DESKTOP_BG: Color = Color::MACOS_BG;

    // Menu bar
    pub const MENUBAR_HEIGHT: u32 = 24;
    pub const MENUBAR_BG: Color = Color::MACOS_MENUBAR;
    pub const MENUBAR_TEXT: Color = Color::MACOS_TEXT;
    pub const MENUBAR_HIGHLIGHT: Color = Color::MACOS_ACCENT;

    // Window
    pub const TITLEBAR_HEIGHT: u32 = 28;
    pub const TITLEBAR_BG: Color = Color::MACOS_TITLEBAR;
    pub const TITLEBAR_BG_INACTIVE: Color = Color::new(50, 50, 50);
    pub const TITLEBAR_TEXT: Color = Color::MACOS_TEXT;
    pub const TITLEBAR_TEXT_INACTIVE: Color = Color::MACOS_TEXT_DIM;
    pub const WINDOW_BG: Color = Color::MACOS_WINDOW_BG;
    pub const WINDOW_BORDER: Color = Color::MACOS_BORDER;
    pub const WINDOW_BORDER_RADIUS: i32 = 10;
    pub const WINDOW_SHADOW_COLOR: Color = Color::with_alpha(80, 0, 0, 0);
    pub const WINDOW_SHADOW_OFFSET: i32 = 4;

    // Traffic light buttons
    pub const BUTTON_CLOSE: Color = Color::MACOS_CLOSE;
    pub const BUTTON_MINIMIZE: Color = Color::MACOS_MINIMIZE;
    pub const BUTTON_MAXIMIZE: Color = Color::MACOS_MAXIMIZE;
    pub const BUTTON_INACTIVE: Color = Color::new(80, 80, 80);
    pub const BUTTON_RADIUS: i32 = 6;
    pub const BUTTON_SPACING: i32 = 20;
    pub const BUTTON_LEFT_MARGIN: i32 = 14;
    pub const BUTTON_Y_CENTER: i32 = 14; // Center of titlebar

    // Dock
    pub const DOCK_HEIGHT: u32 = 64;
    pub const DOCK_BG: Color = Color::MACOS_DOCK;
    pub const DOCK_ICON_SIZE: u32 = 48;
    pub const DOCK_ICON_SPACING: u32 = 6;
    pub const DOCK_BORDER_RADIUS: i32 = 16;
    pub const DOCK_MARGIN_BOTTOM: u32 = 8;

    // Widgets
    pub const TEXT_COLOR: Color = Color::MACOS_TEXT;
    pub const TEXT_DIM: Color = Color::MACOS_TEXT_DIM;
    pub const ACCENT: Color = Color::MACOS_ACCENT;
    pub const BUTTON_BG: Color = Color::new(60, 60, 60);
    pub const BUTTON_BG_HOVER: Color = Color::new(70, 70, 70);
    pub const BUTTON_BG_PRESSED: Color = Color::new(50, 50, 50);
    pub const INPUT_BG: Color = Color::new(35, 35, 35);
    pub const INPUT_BORDER: Color = Color::new(80, 80, 80);
    pub const INPUT_BORDER_FOCUS: Color = Color::MACOS_ACCENT;
    pub const SCROLLBAR_BG: Color = Color::new(45, 45, 45);
    pub const SCROLLBAR_THUMB: Color = Color::new(100, 100, 100);
    pub const SCROLLBAR_WIDTH: u32 = 8;

    // Menu
    pub const MENU_BG: Color = Color::new(50, 50, 50);
    pub const MENU_HIGHLIGHT: Color = Color::MACOS_ACCENT;
    pub const MENU_TEXT: Color = Color::MACOS_TEXT;
    pub const MENU_TEXT_DIM: Color = Color::MACOS_TEXT_DIM;
    pub const MENU_SEPARATOR: Color = Color::new(70, 70, 70);
    pub const MENU_ITEM_HEIGHT: u32 = 22;
    pub const MENU_PADDING: u32 = 4;

    // Font sizes (Cape Coral font)
    pub const FONT_SIZE_SMALL: u16 = 13;
    pub const FONT_SIZE_NORMAL: u16 = 16;
    pub const FONT_SIZE_LARGE: u16 = 20;
    pub const FONT_SIZE_TITLE: u16 = 24;
    pub const MENUBAR_FONT_SIZE: u16 = 13;
    pub const WINDOW_TITLE_FONT_SIZE: u16 = 13;
    pub const MENU_FONT_SIZE: u16 = 13;
    pub const WIDGET_FONT_SIZE: u16 = 13;
}
