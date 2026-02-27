//! Dropdown management and hit testing.

use alloc::vec::Vec;

use crate::compositor::{Compositor, Rect};

use super::MenuBar;
use super::types::*;

impl MenuBar {
    // ── Hit Testing ──────────────────────────────────────────────────────

    /// Hit test a click in the menubar region.
    pub fn hit_test_menubar(&self, mx: i32, my: i32) -> MenuBarHit {
        if my < 0 || my >= MENUBAR_HEIGHT as i32 {
            return MenuBarHit::None;
        }

        // Check status icons first
        for (i, icon) in self.status_icons.iter().enumerate() {
            if let Some(&ix) = self.status_icon_x.get(i) {
                let iy = ((MENUBAR_HEIGHT as i32 - 16) / 2).max(0);
                if mx >= ix && mx < ix + 16 && my >= iy && my < iy + 16 {
                    return MenuBarHit::StatusIcon {
                        owner_tid: icon.owner_tid,
                        icon_id: icon.icon_id,
                    };
                }
            }
        }

        // Check system menu (logo area at far left)
        if mx < SYSTEM_MENU_WIDTH {
            return MenuBarHit::SystemMenu;
        }

        // Check menu titles
        for layout in &self.title_layouts {
            if mx >= layout.x && mx < layout.x + layout.width as i32 {
                return MenuBarHit::MenuTitle {
                    menu_idx: layout.menu_idx,
                };
            }
        }

        MenuBarHit::None
    }

    /// Hit test within the open dropdown. Returns Some(item_id) if a clickable item was hit.
    pub fn hit_test_dropdown(&self, mx: i32, my: i32) -> Option<u32> {
        let dd = self.open_dropdown.as_ref()?;
        let def = self.active_def()?;
        let menu = def.menus.get(dd.menu_idx)?;

        if mx < dd.x || mx >= dd.x + dd.width as i32 {
            return None;
        }
        if my < dd.y || my >= dd.y + dd.height as i32 {
            return None;
        }

        let local_y = my - dd.y;
        for (i, item) in menu.items.iter().enumerate() {
            let item_y = dd.items_y.get(i).copied().unwrap_or(0);
            let item_h = if item.is_separator() {
                SEPARATOR_HEIGHT
            } else {
                ITEM_HEIGHT
            };
            if local_y >= item_y && local_y < item_y + item_h as i32 {
                if !item.is_disabled() && !item.is_separator() {
                    return Some(item.item_id);
                }
                return None;
            }
        }

        None
    }

    /// Check if a point is within the open dropdown bounds.
    pub fn is_in_dropdown(&self, mx: i32, my: i32) -> bool {
        if let Some(ref dd) = self.open_dropdown {
            mx >= dd.x
                && mx < dd.x + dd.width as i32
                && my >= dd.y
                && my < dd.y + dd.height as i32
        } else {
            false
        }
    }

    // ── Dropdown Management ──────────────────────────────────────────────

    /// Open a dropdown for the given menu index.
    pub fn open_menu(
        &mut self,
        menu_idx: usize,
        owner_window_id: u32,
        compositor: &mut Compositor,
    ) -> Option<u32> {
        // Close any existing dropdown first
        self.close_dropdown_with_compositor(compositor);

        let def = self.active_def()?;
        let menu = def.menus.get(menu_idx)?;
        let layout = self.title_layouts.iter().find(|l| l.menu_idx == menu_idx)?;

        // Compute dropdown dimensions
        let mut max_w: u32 = 0;
        let mut total_h: i32 = DROPDOWN_PADDING;
        let mut items_y = Vec::with_capacity(menu.items.len());

        for item in &menu.items {
            items_y.push(total_h);
            if item.is_separator() {
                total_h += SEPARATOR_HEIGHT as i32;
            } else {
                let (tw, _) =
                    anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                max_w = max_w.max(tw + 40); // label + padding + checkmark space
                total_h += ITEM_HEIGHT as i32;
            }
        }
        total_h += DROPDOWN_PADDING;
        let dd_width = max_w.max(120) + DROPDOWN_PADDING as u32 * 2;
        let dd_height = total_h as u32;

        let dd_x = layout.x;
        let dd_y = MENUBAR_HEIGHT as i32 + 1;

        // Create compositor layer (always on top = false, will be raised)
        let layer_id = compositor.add_layer(dd_x, dd_y, dd_width, dd_height, false);
        compositor.raise_layer(layer_id);

        self.open_dropdown = Some(OpenDropdown {
            menu_idx,
            owner_window_id,
            layer_id,
            x: dd_x,
            y: dd_y,
            width: dd_width,
            height: dd_height,
            hover_idx: None,
            items_y,
        });

        // Render the dropdown content
        self.render_dropdown(compositor);

        Some(layer_id)
    }

    /// Update hover state for mouse move. Returns true if dropdown needs redraw.
    pub fn update_hover(&mut self, mx: i32, my: i32) -> bool {
        let dd = match &mut self.open_dropdown {
            Some(d) => d,
            None => return false,
        };

        if mx < dd.x
            || mx >= dd.x + dd.width as i32
            || my < dd.y
            || my >= dd.y + dd.height as i32
        {
            if dd.hover_idx.is_some() {
                dd.hover_idx = None;
                return true;
            }
            return false;
        }

        let local_y = my - dd.y;
        let mut new_hover = None;

        for (i, &item_y) in dd.items_y.iter().enumerate() {
            if local_y >= item_y && local_y < item_y + ITEM_HEIGHT as i32 {
                new_hover = Some(i);
                break;
            }
        }

        if new_hover != dd.hover_idx {
            dd.hover_idx = new_hover;
            true
        } else {
            false
        }
    }

    /// Close the dropdown and remove its compositor layer.
    pub fn close_dropdown_with_compositor(&mut self, compositor: &mut Compositor) {
        if let Some(dd) = self.open_dropdown.take() {
            let bounds = Rect::new(dd.x, dd.y, dd.width, dd.height);
            compositor.remove_layer(dd.layer_id);
            compositor.add_damage(bounds);
        }
        self.system_menu_open = false;
    }

    pub fn is_dropdown_open(&self) -> bool {
        self.open_dropdown.is_some()
    }

    /// Open the system menu (logo dropdown) — "About anyOS", "Settings...", etc.
    pub fn open_system_menu(&mut self, compositor: &mut Compositor) -> Option<u32> {
        use alloc::string::String;
        use alloc::vec::Vec;

        // Close any existing dropdown first
        self.close_dropdown_with_compositor(compositor);

        let items = Vec::from([
            super::MenuItem { item_id: SYS_MENU_ABOUT, flags: 0, label: String::from("About anyOS") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_SETTINGS, flags: 0, label: String::from("System Settings...") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_LOGOUT, flags: 0, label: String::from("Log Out") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_SLEEP, flags: 0, label: String::from("Sleep") },
            super::MenuItem { item_id: SYS_MENU_RESTART, flags: 0, label: String::from("Restart") },
            super::MenuItem { item_id: SYS_MENU_SHUTDOWN, flags: 0, label: String::from("Shut Down") },
        ]);

        // Compute dropdown dimensions
        let mut max_w: u32 = 0;
        let mut total_h: i32 = DROPDOWN_PADDING;
        let mut items_y = Vec::with_capacity(items.len());

        for item in &items {
            items_y.push(total_h);
            if item.is_separator() {
                total_h += SEPARATOR_HEIGHT as i32;
            } else {
                let (tw, _) =
                    anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                max_w = max_w.max(tw + 40);
                total_h += ITEM_HEIGHT as i32;
            }
        }
        total_h += DROPDOWN_PADDING;
        let dd_width = max_w.max(180) + DROPDOWN_PADDING as u32 * 2;
        let dd_height = total_h as u32;
        let dd_x = 0i32;
        let dd_y = MENUBAR_HEIGHT as i32 + 1;

        let layer_id = compositor.add_layer(dd_x, dd_y, dd_width, dd_height, false);
        compositor.raise_layer(layer_id);

        self.open_dropdown = Some(OpenDropdown {
            menu_idx: usize::MAX, // sentinel: system menu, not a window menu
            owner_window_id: 0,
            layer_id,
            x: dd_x,
            y: dd_y,
            width: dd_width,
            height: dd_height,
            hover_idx: None,
            items_y,
        });
        self.system_menu_open = true;

        // Render the dropdown (we need to do it manually since it's not a window menu)
        self.render_system_dropdown(compositor, &items);

        Some(layer_id)
    }

    /// Render the system menu dropdown.
    fn render_system_dropdown(&self, compositor: &mut Compositor, items: &[super::MenuItem]) {
        use crate::desktop::drawing::{fill_rect, fill_rounded_rect};
        use super::rendering::draw_rect_outline;

        let dd = match &self.open_dropdown {
            Some(d) => d,
            None => return,
        };

        if let Some(pixels) = compositor.layer_pixels(dd.layer_id) {
            let w = dd.width;
            let h = dd.height;

            for p in pixels.iter_mut() { *p = 0x00000000; }
            fill_rounded_rect(pixels, w, h, 0, 0, w, h, 6, color_dropdown_bg());
            draw_rect_outline(pixels, w, 0, 0, w, h, color_dropdown_border());

            for (i, item) in items.iter().enumerate() {
                let iy = dd.items_y[i];

                if item.is_separator() {
                    let line_y = iy + SEPARATOR_HEIGHT as i32 / 2;
                    if line_y >= 0 && (line_y as u32) < h {
                        for x in 8i32..(w as i32 - 8) {
                            if x >= 0 && (x as u32) < w {
                                let idx = (line_y as u32 * w + x as u32) as usize;
                                if idx < pixels.len() { pixels[idx] = color_separator(); }
                            }
                        }
                    }
                    continue;
                }

                if dd.hover_idx == Some(i) && !item.is_disabled() {
                    fill_rect(pixels, w, h, 4, iy, w - 8, ITEM_HEIGHT, COLOR_HOVER_BG);
                }

                let text_x = 16i32;
                let (_, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                let text_y = iy + ((ITEM_HEIGHT as i32 - th as i32) / 2).max(0);
                let text_color = if item.is_disabled() {
                    color_disabled_text()
                } else if dd.hover_idx == Some(i) {
                    0xFFFFFFFF
                } else {
                    color_menubar_text()
                };
                anyos_std::ui::window::font_render_buf(
                    FONT_ID, FONT_SIZE, pixels, w, h, text_x, text_y, text_color, &item.label,
                );
            }
        }

        compositor.mark_layer_dirty(dd.layer_id);
        let bounds = Rect::new(dd.x, dd.y, dd.width, dd.height);
        compositor.add_damage(bounds);
    }

    /// Hit test within the system menu dropdown. Returns Some(item_id) if a clickable item was hit.
    pub fn hit_test_system_menu(&self, mx: i32, my: i32) -> Option<u32> {
        if !self.system_menu_open { return None; }
        let dd = self.open_dropdown.as_ref()?;

        if mx < dd.x || mx >= dd.x + dd.width as i32 { return None; }
        if my < dd.y || my >= dd.y + dd.height as i32 { return None; }

        let local_y = my - dd.y;
        let items = Self::system_menu_items();
        for (i, item) in items.iter().enumerate() {
            let item_y = dd.items_y.get(i).copied().unwrap_or(0);
            let item_h = if item.is_separator() { SEPARATOR_HEIGHT } else { ITEM_HEIGHT };
            if local_y >= item_y && local_y < item_y + item_h as i32 {
                if !item.is_disabled() && !item.is_separator() {
                    return Some(item.item_id);
                }
                return None;
            }
        }
        None
    }

    /// Build the list of system menu items (used for hit testing and re-rendering).
    pub fn system_menu_items() -> Vec<super::MenuItem> {
        use alloc::string::String;
        Vec::from([
            super::MenuItem { item_id: SYS_MENU_ABOUT, flags: 0, label: String::from("About anyOS") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_SETTINGS, flags: 0, label: String::from("System Settings...") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_LOGOUT, flags: 0, label: String::from("Log Out") },
            super::MenuItem { item_id: 0, flags: MENU_FLAG_SEPARATOR, label: String::new() },
            super::MenuItem { item_id: SYS_MENU_SLEEP, flags: 0, label: String::from("Sleep") },
            super::MenuItem { item_id: SYS_MENU_RESTART, flags: 0, label: String::from("Restart") },
            super::MenuItem { item_id: SYS_MENU_SHUTDOWN, flags: 0, label: String::from("Shut Down") },
        ])
    }

    /// Re-render the system dropdown after hover changes.
    pub fn rerender_system_dropdown(&self, compositor: &mut Compositor) {
        let items = Self::system_menu_items();
        self.render_system_dropdown(compositor, &items);
    }
}
