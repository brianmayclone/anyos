//! MenuBar — macOS-like global menu bar with per-app menus, dropdowns, and status icons.

mod dropdown;
mod rendering;
pub(crate) mod types;

pub use types::{
    MenuBarDef, MenuBarHit, MenuItem, Menu, OpenDropdown, StatusIcon,
    MENU_FLAG_SEPARATOR,
    APP_MENU_ABOUT, APP_MENU_HIDE, APP_MENU_QUIT,
    SYS_MENU_ABOUT, SYS_MENU_SETTINGS, SYS_MENU_LOGOUT,
    SYS_MENU_SLEEP, SYS_MENU_RESTART, SYS_MENU_SHUTDOWN,
};

use alloc::string::String;
use alloc::vec::Vec;
use types::*;

// ── MenuBar ──────────────────────────────────────────────────────────────────

pub struct MenuBar {
    /// Per-window menu definitions: (window_id, MenuBarDef)
    pub(crate) window_menus: Vec<(u32, MenuBarDef)>,

    /// Layout of current menu titles (recomputed on focus change)
    pub(crate) title_layouts: Vec<MenuTitleLayout>,

    /// Currently open dropdown (if any)
    pub open_dropdown: Option<OpenDropdown>,

    /// Registered status icons (right side of menubar, left of clock)
    pub(crate) status_icons: Vec<StatusIcon>,

    /// Layout positions of status icons (computed when icons change)
    pub(crate) status_icon_x: Vec<i32>,

    /// The window_id whose menus are currently displayed
    pub(crate) active_window_id: Option<u32>,

    /// Whether the system menu (logo dropdown) is currently open.
    pub system_menu_open: bool,
}

impl MenuBar {
    pub fn new() -> Self {
        MenuBar {
            window_menus: Vec::new(),
            title_layouts: Vec::new(),
            open_dropdown: None,
            status_icons: Vec::with_capacity(8),
            status_icon_x: Vec::new(),
            active_window_id: None,
            system_menu_open: false,
        }
    }

    // ── Menu Registration ────────────────────────────────────────────────

    /// Register/update menu bar for a window.
    /// Automatically prepends an app-name menu (About/Hide/Quit) using `app_name`.
    pub fn set_menu(&mut self, window_id: u32, mut def: MenuBarDef, app_name: &str) {
        // Auto-prepend app-name menu (macOS style)
        let app_menu = Menu {
            title: String::from(app_name),
            items: Vec::from([
                MenuItem { item_id: APP_MENU_ABOUT, flags: 0, label: String::from("About") },
                MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
                MenuItem { item_id: APP_MENU_HIDE, flags: 0, label: String::from("Hide") },
                MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
                MenuItem { item_id: APP_MENU_QUIT, flags: 0, label: String::from("Quit") },
            ]),
        };
        def.menus.insert(0, app_menu);

        if let Some(entry) = self.window_menus.iter_mut().find(|(id, _)| *id == window_id) {
            entry.1 = def;
        } else {
            self.window_menus.push((window_id, def));
        }
        if self.active_window_id == Some(window_id) {
            self.recompute_title_layout();
        }
    }

    /// Remove all menus associated with a window.
    pub fn remove_menu(&mut self, window_id: u32) {
        self.window_menus.retain(|(id, _)| *id != window_id);
        if self.active_window_id == Some(window_id) {
            self.active_window_id = None;
            self.title_layouts.clear();
        }
    }

    /// Update the flags for a specific item_id in a window's menu.
    /// Returns true if the item was found and flags changed.
    pub fn update_item_flags(&mut self, window_id: u32, item_id: u32, new_flags: u32) -> bool {
        if let Some(entry) = self.window_menus.iter_mut().find(|(id, _)| *id == window_id) {
            for menu in &mut entry.1.menus {
                if let Some(item) = menu.items.iter_mut().find(|i| i.item_id == item_id) {
                    if item.flags != new_flags {
                        item.flags = new_flags;
                        return true;
                    }
                    return false;
                }
            }
        }
        false
    }

    /// Called when focused window changes. Returns true if menubar needs redraw.
    pub fn on_focus_change(&mut self, window_id: Option<u32>) -> bool {
        if self.active_window_id == window_id {
            return false;
        }
        self.active_window_id = window_id;
        self.recompute_title_layout();
        true
    }

    /// Get the active MenuBarDef (for the focused window), if any.
    pub(crate) fn active_def(&self) -> Option<&MenuBarDef> {
        let wid = self.active_window_id?;
        self.window_menus
            .iter()
            .find(|(id, _)| *id == wid)
            .map(|(_, def)| def)
    }

    fn recompute_title_layout(&mut self) {
        self.title_layouts.clear();
        let wid = match self.active_window_id {
            Some(w) => w,
            None => return,
        };
        let def = match self.window_menus.iter().find(|(id, _)| *id == wid) {
            Some((_, d)) => d,
            None => return,
        };

        let mut x: i32 = MENU_TITLE_START_X;

        for (idx, menu) in def.menus.iter().enumerate() {
            // App name (first menu) uses bold font, like macOS
            let font = if idx == 0 { FONT_ID_BOLD } else { FONT_ID };
            let (tw, _) = anyos_std::ui::window::font_measure(font, FONT_SIZE, &menu.title);
            let padding: u32 = 16;
            let total_w = tw + padding;
            self.title_layouts.push(MenuTitleLayout {
                x,
                width: total_w,
                menu_idx: idx,
            });
            x += total_w as i32;
        }
    }

    // ── Status Icons ─────────────────────────────────────────────────────

    /// Add a status icon. Returns true if added.
    pub fn add_status_icon(
        &mut self,
        owner_tid: u32,
        icon_id: u32,
        pixel_data: &[u32],
        screen_width: u32,
    ) -> bool {
        if pixel_data.len() < 256 {
            return false;
        }
        // Upsert: if icon already exists, update its pixels in-place
        if let Some(existing) = self
            .status_icons
            .iter_mut()
            .find(|i| i.owner_tid == owner_tid && i.icon_id == icon_id)
        {
            existing.pixels.copy_from_slice(&pixel_data[..256]);
            return true;
        }
        if self.status_icons.len() >= 8 {
            return false;
        }

        let mut pixels = [0u32; 256];
        pixels.copy_from_slice(&pixel_data[..256]);

        self.status_icons.push(StatusIcon {
            owner_tid,
            icon_id,
            pixels,
        });
        self.recompute_status_positions(screen_width);
        true
    }

    /// Remove a status icon. Returns true if removed.
    pub fn remove_status_icon(
        &mut self,
        owner_tid: u32,
        icon_id: u32,
        screen_width: u32,
    ) -> bool {
        let before = self.status_icons.len();
        self.status_icons
            .retain(|i| !(i.owner_tid == owner_tid && i.icon_id == icon_id));
        if self.status_icons.len() != before {
            self.recompute_status_positions(screen_width);
            true
        } else {
            false
        }
    }

    fn recompute_status_positions(&mut self, screen_width: u32) {
        self.status_icon_x.clear();
        let clock_region = 60i32;
        let icon_spacing = 8i32;
        let icon_size = 16i32;
        let mut x = screen_width as i32 - clock_region - icon_spacing;

        // Layout right-to-left
        for _ in self.status_icons.iter().rev() {
            x -= icon_size;
            self.status_icon_x.push(x);
            x -= icon_spacing;
        }
        self.status_icon_x.reverse();
    }
}
