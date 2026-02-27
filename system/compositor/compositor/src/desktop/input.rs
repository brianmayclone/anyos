//! Input handling — mouse, keyboard, scroll, drag, and resize interaction.

use crate::compositor::Rect;
use crate::keys::{encode_scancode, KEY_VOLUME_UP, KEY_VOLUME_DOWN, KEY_VOLUME_MUTE};
use crate::menu::MenuBarHit;

use super::cursors::CursorShape;
use super::theme::*;
use super::window::*;
use super::Desktop;

// ── Input Constants ────────────────────────────────────────────────────────

const INPUT_KEY_DOWN: u32 = 1;
const INPUT_KEY_UP: u32 = 2;
const INPUT_MOUSE_MOVE: u32 = 3;
const INPUT_MOUSE_BUTTON: u32 = 4;
const INPUT_MOUSE_SCROLL: u32 = 5;
const INPUT_MOUSE_MOVE_ABSOLUTE: u32 = 6;

// ── Desktop Input Methods ──────────────────────────────────────────────────

impl Desktop {
    /// Process a batch of raw input events. Returns true if a compose is needed.
    pub fn process_input(&mut self, events: &[[u32; 5]], count: usize) -> bool {
        let mut needs_compose = false;
        let mut cursor_moved = false;
        let mut last_dx: i32 = 0;
        let mut last_dy: i32 = 0;
        let mut absolute_move = false;
        let mut abs_x: i32 = 0;
        let mut abs_y: i32 = 0;

        // Batch mouse moves — only process the final position
        for i in 0..count {
            let evt = events[i];
            match evt[0] {
                INPUT_MOUSE_MOVE => {
                    let dx = evt[1] as i32;
                    let dy = evt[2] as i32;
                    if absolute_move {
                        absolute_move = false;
                        last_dx = 0;
                        last_dy = 0;
                    }
                    last_dx += dx;
                    last_dy += dy;
                    cursor_moved = true;
                }
                INPUT_MOUSE_MOVE_ABSOLUTE => {
                    abs_x = evt[1] as i32;
                    abs_y = evt[2] as i32;
                    absolute_move = true;
                    cursor_moved = true;
                    last_dx = 0;
                    last_dy = 0;
                }
                INPUT_MOUSE_BUTTON => {
                    if cursor_moved {
                        if absolute_move {
                            self.apply_mouse_move_absolute(abs_x, abs_y);
                            absolute_move = false;
                        } else {
                            self.apply_mouse_move(last_dx, last_dy);
                        }
                        last_dx = 0;
                        last_dy = 0;
                        cursor_moved = false;
                    }
                    let dx = evt[3] as i32;
                    let dy = evt[4] as i32;
                    if dx != 0 || dy != 0 {
                        self.apply_mouse_move(dx, dy);
                    }
                    let buttons = evt[1];
                    let down = evt[2] != 0;
                    self.handle_mouse_button(buttons, down);
                    needs_compose = true;
                }
                INPUT_MOUSE_SCROLL => {
                    let dz = evt[1] as i32;
                    self.handle_scroll(dz);
                }
                INPUT_KEY_DOWN => {
                    self.handle_key(evt[1], evt[2], evt[3], true);
                }
                INPUT_KEY_UP => {
                    self.handle_key(evt[1], evt[2], evt[3], false);
                }
                _ => {}
            }
        }

        // Apply any remaining batched mouse move
        if cursor_moved {
            if absolute_move {
                self.apply_mouse_move_absolute(abs_x, abs_y);
            } else {
                self.apply_mouse_move(last_dx, last_dy);
            }
            needs_compose = true;
        }

        needs_compose
    }

    pub(crate) fn apply_mouse_move(&mut self, dx: i32, dy: i32) {
        self.mouse_x = (self.mouse_x + dx).clamp(0, self.screen_width as i32 - 1);
        self.mouse_y = (self.mouse_y + dy).clamp(0, self.screen_height as i32 - 1);

        // Handle desktop icon drag (start drag if mouse moved enough while held)
        if !self.desktop_icons.is_dragging() {
            if let Some((icon_idx, down_x, down_y)) = self.desktop_icons.mouse_down_icon {
                let move_dx = (self.mouse_x - down_x).abs();
                let move_dy = (self.mouse_y - down_y).abs();
                if move_dx > 4 || move_dy > 4 {
                    self.desktop_icons.start_drag(icon_idx, down_x, down_y);
                    self.desktop_icons.mouse_down_icon = None;
                }
            }
        }
        if self.desktop_icons.is_dragging() {
            if let Some((_old_rect, _new_rect)) = self.desktop_icons.update_drag(self.mouse_x, self.mouse_y) {
                self.reload_wallpaper_and_icons();
            }
        }

        // Handle window drag
        if let Some(ref drag) = self.dragging {
            let win_id = drag.window_id;
            let new_x = self.mouse_x - drag.offset_x;
            let new_y = self.mouse_y - drag.offset_y;
            if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                let layer_id = self.windows[idx].layer_id;
                self.windows[idx].x = new_x;
                self.windows[idx].y = new_y;
                self.compositor.move_layer(layer_id, new_x, new_y);
            }
        }

        // Handle resize (show outline)
        if let Some(ref resize) = self.resizing {
            let rdx = self.mouse_x - resize.start_mouse_x;
            let rdy = self.mouse_y - resize.start_mouse_y;
            let (ox, oy, ow, oh) = compute_resize(
                resize.edge,
                resize.start_x,
                resize.start_y,
                resize.start_w,
                resize.start_h,
                rdx,
                rdy,
            );
            let old_outline = self.compositor.resize_outline;
            self.compositor.resize_outline = Some(Rect::new(ox, oy, ow, oh));
            if let Some(old) = old_outline {
                self.compositor.add_damage(old.expand(2));
            }
            self.compositor
                .add_damage(Rect::new(ox, oy, ow, oh).expand(2));
        }

        // Update dropdown hover state and menu-slide
        if self.menu_bar.is_dropdown_open() {
            if self.menu_bar.system_menu_open {
                // System menu hover update
                if self.menu_bar.update_hover(self.mouse_x, self.mouse_y) {
                    self.menu_bar.rerender_system_dropdown(&mut self.compositor);
                }
                // Slide from system menu to app menus
                if self.mouse_y < MENUBAR_HEIGHT as i32 {
                    if let MenuBarHit::MenuTitle { menu_idx } =
                        self.menu_bar.hit_test_menubar(self.mouse_x, self.mouse_y)
                    {
                        let owner_wid = self.focused_window.unwrap_or(0);
                        self.menu_bar
                            .close_dropdown_with_compositor(&mut self.compositor);
                        self.menu_bar
                            .open_menu(menu_idx, owner_wid, &mut self.compositor);
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
            } else {
                // App menu hover update
                if self.menu_bar.update_hover(self.mouse_x, self.mouse_y) {
                    self.menu_bar.render_dropdown(&mut self.compositor);
                }
                // Slide between app menus or to system menu
                if self.mouse_y < MENUBAR_HEIGHT as i32 {
                    match self.menu_bar.hit_test_menubar(self.mouse_x, self.mouse_y) {
                        MenuBarHit::SystemMenu => {
                            self.menu_bar
                                .close_dropdown_with_compositor(&mut self.compositor);
                            self.menu_bar
                                .open_system_menu(&mut self.compositor);
                            self.draw_menubar();
                            self.compositor.add_damage(Rect::new(
                                0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                            ));
                        }
                        MenuBarHit::MenuTitle { menu_idx } => {
                            let current_idx =
                                self.menu_bar.open_dropdown.as_ref().map(|d| d.menu_idx);
                            if current_idx != Some(menu_idx) {
                                let owner_wid = self.focused_window.unwrap_or(0);
                                self.menu_bar
                                    .close_dropdown_with_compositor(&mut self.compositor);
                                self.menu_bar
                                    .open_menu(menu_idx, owner_wid, &mut self.compositor);
                                self.draw_menubar();
                                self.compositor.add_damage(Rect::new(
                                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Update desktop icon context menu hover
        if self.desktop_icons.has_context_menu() {
            if self.desktop_icons.update_context_hover(self.mouse_x, self.mouse_y) {
                self.desktop_icons.render_context_menu(&mut self.compositor);
            }
        }

        // Update HW cursor position
        self.compositor
            .move_hw_cursor(self.mouse_x, self.mouse_y);

        // Update cursor shape
        if self.dragging.is_some() {
            self.set_cursor_shape(CursorShape::Move);
        } else if self.resizing.is_some() {
            // Keep current resize cursor
        } else {
            let mx = self.mouse_x;
            let my = self.mouse_y;
            let mut shape = CursorShape::Arrow;
            for w in self.windows.iter().rev() {
                let hit = w.hit_test(mx, my);
                if hit != HitTest::None {
                    shape = self.cursor_for_hit(hit);
                    break;
                }
            }
            self.set_cursor_shape(shape);
        }

        // Track button hover for animated colour transitions
        if self.has_gpu_accel && self.dragging.is_none() && self.resizing.is_none() {
            let new_hover = self.get_button_under_cursor();
            if new_hover != self.btn_hover {
                let now = anyos_std::sys::uptime();
                if let Some((old_wid, old_btn)) = self.btn_hover {
                    let aid = button_anim_id(old_wid, old_btn);
                    self.btn_anims.start(aid, 1000, 0, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(old_wid);
                }
                if let Some((new_wid, new_btn)) = new_hover {
                    let aid = button_anim_id(new_wid, new_btn);
                    self.btn_anims.start(aid, 0, 1000, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(new_wid);
                }
                self.btn_hover = new_hover;
            }
        }

        // Forward mouse move to topmost IPC window under cursor
        if self.dragging.is_none() && self.resizing.is_none() {
            for win in self.windows.iter().rev() {
                if win.owner_tid != 0 {
                    let ht = win.hit_test(self.mouse_x, self.mouse_y);
                    if ht != HitTest::None {
                        let lx = self.mouse_x - win.x;
                        let mut ly = self.mouse_y - win.y;
                        if !win.is_borderless() {
                            ly -= TITLE_BAR_HEIGHT as i32;
                        }
                        self.push_event(
                            win.id,
                            [EVENT_MOUSE_MOVE, lx as u32, ly as u32, 0, 0],
                        );
                        break;
                    }
                }
            }
        }
    }

    /// Apply an absolute mouse position (from VMMDev).
    fn apply_mouse_move_absolute(&mut self, x: i32, y: i32) {
        let target_x = x.clamp(0, self.screen_width as i32 - 1);
        let target_y = y.clamp(0, self.screen_height as i32 - 1);
        let dx = target_x - self.mouse_x;
        let dy = target_y - self.mouse_y;
        if dx == 0 && dy == 0 {
            return;
        }
        self.apply_mouse_move(dx, dy);
    }

    /// Returns (window_id, btn_index) of the button under the cursor, if any.
    fn get_button_under_cursor(&self) -> Option<(u32, u8)> {
        let mx = self.mouse_x;
        let my = self.mouse_y;
        for win in self.windows.iter().rev() {
            let ht = win.hit_test(mx, my);
            match ht {
                HitTest::CloseButton => return Some((win.id, 0)),
                HitTest::MinButton => return Some((win.id, 1)),
                HitTest::MaxButton => return Some((win.id, 2)),
                HitTest::None => continue,
                _ => return None,
            }
        }
        None
    }

    /// Tick active button animations. Returns true if any animation was active.
    pub fn tick_animations(&mut self) -> bool {
        let now = anyos_std::sys::uptime();
        if !self.btn_anims.has_active(now) {
            return false;
        }
        let mut wids = [0u32; 16];
        let mut wid_count = 0usize;
        for win in &self.windows {
            if win.focused {
                let w = win.id;
                for btn in 0u8..3 {
                    let aid = button_anim_id(w, btn);
                    if self.btn_anims.is_active(aid, now) {
                        let already = wids[..wid_count].contains(&w);
                        if !already && wid_count < 16 {
                            wids[wid_count] = w;
                            wid_count += 1;
                        }
                        break;
                    }
                }
            }
        }
        for i in 0..wid_count {
            self.render_window(wids[i]);
        }
        self.btn_anims.remove_done(now);

        // Tick notification animations and auto-dismiss
        let sw = self.screen_width;
        let notif_active = self.notifications.tick(&mut self.compositor, sw);

        // Tick volume HUD animation and auto-dismiss
        let hud_active = self.volume_hud.tick(&mut self.compositor);

        wid_count > 0 || notif_active || hud_active
    }

    pub(crate) fn handle_mouse_button(&mut self, buttons: u32, down: bool) {
        if down {
            self.mouse_buttons = buttons;

            // Check if clicking within open desktop icon context menu
            if self.desktop_icons.has_context_menu() {
                if self.desktop_icons.is_in_context_menu(self.mouse_x, self.mouse_y) {
                    if let Some((item_id, icon_idx)) = self.desktop_icons.hit_test_context_menu(self.mouse_x, self.mouse_y) {
                        let action = self.desktop_icons.handle_context_action(item_id, icon_idx);
                        self.desktop_icons.close_context_menu(&mut self.compositor);
                        match action {
                            super::desktop_icons::ContextAction::OpenFinder(path) => {
                                anyos_std::process::spawn("/Applications/Finder.app", &path);
                            }
                            super::desktop_icons::ContextAction::Eject(path) => {
                                anyos_std::fs::umount(&path);
                                // Icons will update on next poll_mounts cycle
                            }
                            super::desktop_icons::ContextAction::None => {}
                        }
                    } else {
                        self.desktop_icons.close_context_menu(&mut self.compositor);
                    }
                    return;
                }
                // Clicked outside context menu — close it
                self.desktop_icons.close_context_menu(&mut self.compositor);
            }

            // Check if clicking within open dropdown
            if self.menu_bar.is_dropdown_open() {
                if self.menu_bar.is_in_dropdown(self.mouse_x, self.mouse_y) {
                    if self.menu_bar.system_menu_open {
                        // System menu dropdown click
                        if let Some(item_id) = self.menu_bar.hit_test_system_menu(self.mouse_x, self.mouse_y) {
                            self.handle_system_menu_action(item_id);
                        }
                    } else if let Some(item_id) = self.menu_bar.hit_test_dropdown(self.mouse_x, self.mouse_y) {
                        // App menu dropdown click
                        if let Some(win_id) = self.focused_window {
                            match item_id {
                                crate::menu::APP_MENU_QUIT => {
                                    self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                                }
                                crate::menu::APP_MENU_HIDE => {
                                    if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                                        let layer_id = self.windows[idx].layer_id;
                                        self.windows[idx].saved_bounds = Some((
                                            self.windows[idx].x,
                                            self.windows[idx].y,
                                            self.windows[idx].content_width,
                                            self.windows[idx].full_height(),
                                        ));
                                        self.compositor.move_layer(layer_id, -10000, -10000);
                                    }
                                    let next = self.windows.iter().rev()
                                        .find(|w| w.id != win_id && w.x >= 0)
                                        .map(|w| w.id);
                                    if let Some(nid) = next {
                                        self.focus_window(nid);
                                    }
                                }
                                _ => {
                                    let menu_idx = self.menu_bar.open_dropdown
                                        .as_ref()
                                        .map(|d| d.menu_idx as u32)
                                        .unwrap_or(0);
                                    self.push_event(win_id, [EVENT_MENU_ITEM, menu_idx, item_id, 0, 0]);
                                }
                            }
                        }
                    }
                    self.menu_bar.close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                    return;
                }

                if self.mouse_y < MENUBAR_HEIGHT as i32 {
                    self.handle_menubar_click();
                    return;
                }
                self.menu_bar.close_dropdown_with_compositor(&mut self.compositor);
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }

            // Check menubar click
            if self.mouse_y < MENUBAR_HEIGHT as i32 {
                self.handle_menubar_click();
                return;
            }

            // Check notification clicks (before window hit testing)
            {
                let sw = self.screen_width;
                if let Some((notif_id, sender_tid)) =
                    self.notifications.handle_click(self.mouse_x, self.mouse_y, sw)
                {
                    // Emit EVT_NOTIFICATION_CLICK to the sender app
                    let target = self.get_sub_id_for_tid(sender_tid);
                    if self.tray_ipc_events.len() < 256 {
                        self.tray_ipc_events.push((
                            target,
                            [crate::ipc_protocol::EVT_NOTIFICATION_CLICK, notif_id, sender_tid, 0, 0],
                        ));
                    }
                    return;
                }
            }

            // Check window hits
            let mx = self.mouse_x;
            let my = self.mouse_y;

            let mut hit_win_id = None;
            let mut hit_test = HitTest::None;
            for win in self.windows.iter().rev() {
                let ht = win.hit_test(mx, my);
                if ht != HitTest::None {
                    hit_win_id = Some(win.id);
                    hit_test = ht;
                    break;
                }
            }

            if let Some(win_id) = hit_win_id {
                if self.focused_window != Some(win_id) {
                    self.focus_window(win_id);
                }

                match hit_test {
                    HitTest::CloseButton => {
                        // Crash dialogs: close button dismisses the dialog directly
                        if self.handle_crash_dialog_click(win_id, 0, 0) {
                            return;
                        }
                        let no_close = self.windows.iter().find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_CLOSE != 0).unwrap_or(false);
                        if !no_close {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 0));
                                let aid = button_anim_id(win_id, 0);
                                self.btn_anims.start(aid, 0, 1000, 100, anyos_std::anim::Easing::EaseOut);
                                self.render_window(win_id);
                            }
                            self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                        }
                    }
                    HitTest::TitleBar => {
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            if self.windows[idx].flags & WIN_FLAG_NO_MOVE != 0 {
                                // Not movable
                            } else {
                                self.dragging = Some(DragState {
                                    window_id: win_id,
                                    offset_x: mx - self.windows[idx].x,
                                    offset_y: my - self.windows[idx].y,
                                });
                                let layer_id = self.windows[idx].layer_id;
                                let old_shadow = {
                                    if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                                        if layer.has_shadow {
                                            let sb = layer.shadow_bounds();
                                            layer.has_shadow = false;
                                            Some(sb)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                };
                                if let Some(sb) = old_shadow {
                                    self.compositor.add_damage(sb);
                                }
                            }
                        }
                    }
                    HitTest::MinButton => {
                        let no_min = self.windows.iter().find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_MINIMIZE != 0).unwrap_or(false);
                        if !no_min {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 1));
                                let aid = button_anim_id(win_id, 1);
                                self.btn_anims.start(aid, 0, 1000, 100, anyos_std::anim::Easing::EaseOut);
                                self.render_window(win_id);
                            }
                            self.minimize_window(win_id);
                        }
                    }
                    HitTest::MaxButton => {
                        let no_max = self.windows.iter().find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_MAXIMIZE != 0).unwrap_or(false);
                        if !no_max {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 2));
                                let aid = button_anim_id(win_id, 2);
                                self.btn_anims.start(aid, 0, 1000, 100, anyos_std::anim::Easing::EaseOut);
                                self.render_window(win_id);
                            }
                            self.toggle_maximize(win_id);
                        }
                    }
                    HitTest::Content => {
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            let lx = mx - self.windows[idx].x;
                            let ly = my - self.windows[idx].y;
                            // Check if this is a crash dialog click
                            if self.handle_crash_dialog_click(win_id, lx, ly) {
                                return;
                            }
                            let mut content_ly = ly;
                            if !self.windows[idx].is_borderless() {
                                content_ly -= TITLE_BAR_HEIGHT as i32;
                            }
                            self.push_event(
                                win_id,
                                [EVENT_MOUSE_DOWN, lx as u32, content_ly as u32, buttons | (self.current_modifiers << 8), 0],
                            );
                        }
                    }
                    ht if is_resize_edge(ht) => {
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            if self.windows[idx].is_resizable() {
                                self.resizing = Some(ResizeState {
                                    window_id: win_id,
                                    start_mouse_x: mx,
                                    start_mouse_y: my,
                                    start_x: self.windows[idx].x,
                                    start_y: self.windows[idx].y,
                                    start_w: self.windows[idx].content_width,
                                    start_h: self.windows[idx].full_height(),
                                    edge: ht,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                // Clicked on empty desktop — check desktop icons first
                let is_right_click = buttons & 2 != 0;
                let mut needs_icon_redraw = false;

                // Double-click detection
                let now = anyos_std::sys::uptime_ms();
                let dt = now.wrapping_sub(self.last_click_time);
                let click_dx = (mx - self.last_click_x).abs();
                let click_dy = (my - self.last_click_y).abs();
                let is_double = dt < 400 && click_dx < 5 && click_dy < 5 && !is_right_click;
                self.last_click_time = now;
                self.last_click_x = mx;
                self.last_click_y = my;

                if let Some(icon_idx) = self.desktop_icons.hit_test(mx, my) {
                    if is_right_click {
                        // Right-click → open context menu
                        self.desktop_icons.selected_icon = Some(icon_idx);
                        self.desktop_icons.open_context_menu(icon_idx, mx, my, &mut self.compositor);
                        needs_icon_redraw = true;
                    } else if is_double {
                        // Double-click → open Finder at mount path
                        let mount_path = self.desktop_icons.icons[icon_idx].mount_path_str();
                        let path = alloc::string::String::from(mount_path);
                        anyos_std::process::spawn("/Applications/Finder.app", &path);
                    } else {
                        // Single click → select icon and prepare for drag on move
                        let old_sel = self.desktop_icons.selected_icon;
                        self.desktop_icons.selected_icon = Some(icon_idx);
                        self.desktop_icons.mouse_down_icon = Some((icon_idx, mx, my));
                        if old_sel != Some(icon_idx) {
                            needs_icon_redraw = true;
                        }
                    }
                } else {
                    // Clicked on empty desktop (no icon) — deselect icon & defocus window
                    if self.desktop_icons.selected_icon.is_some() {
                        self.desktop_icons.selected_icon = None;
                        needs_icon_redraw = true;
                    }
                    if let Some(old_id) = self.focused_window {
                        if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                            self.windows[idx].focused = false;
                            let win_id = self.windows[idx].id;
                            self.render_titlebar(win_id);
                            self.push_event(win_id, [EVENT_FOCUS_LOST, 0, 0, 0, 0]);
                        }
                        self.focused_window = None;
                        self.compositor.set_focused_layer(None);
                    }
                }

                if needs_icon_redraw {
                    self.reload_wallpaper_and_icons();
                }
            }
        } else {
            // Mouse up
            self.mouse_buttons = 0;

            // Clear mouse_down_icon state
            self.desktop_icons.mouse_down_icon = None;

            // End desktop icon drag
            if self.desktop_icons.end_drag() {
                self.reload_wallpaper_and_icons();
            }

            if let Some((wid, btn)) = self.btn_pressed.take() {
                if self.has_gpu_accel {
                    let aid = button_anim_id(wid, btn);
                    self.btn_anims.start(aid, 1000, 0, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(wid);
                }
            }

            // End drag — re-enable shadow
            if let Some(ref drag) = self.dragging {
                let win_id = drag.window_id;
                if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                    if !self.windows[idx].is_borderless() {
                        let layer_id = self.windows[idx].layer_id;
                        let new_shadow = {
                            if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                                layer.has_shadow = true;
                                Some(layer.shadow_bounds())
                            } else {
                                None
                            }
                        };
                        if let Some(sb) = new_shadow {
                            self.compositor.add_damage(sb);
                        }
                    }
                }
                self.set_cursor_shape(CursorShape::Arrow);
            }
            self.dragging = None;

            // End resize — apply final size
            if let Some(resize) = self.resizing.take() {
                let rdx = self.mouse_x - resize.start_mouse_x;
                let rdy = self.mouse_y - resize.start_mouse_y;
                let (nx, ny, nw, nh) = compute_resize(
                    resize.edge,
                    resize.start_x,
                    resize.start_y,
                    resize.start_w,
                    resize.start_h,
                    rdx,
                    rdy,
                );

                self.set_cursor_shape(CursorShape::Arrow);
                if let Some(outline) = self.compositor.resize_outline.take() {
                    self.compositor.add_damage(outline.expand(2));
                }

                if let Some(idx) = self.windows.iter().position(|w| w.id == resize.window_id) {
                    let borderless = self.windows[idx].is_borderless();
                    let content_h = if borderless {
                        nh
                    } else {
                        nh.saturating_sub(TITLE_BAR_HEIGHT)
                    };
                    let win_id = resize.window_id;

                    self.windows[idx].x = nx;
                    self.windows[idx].y = ny;
                    let layer_id = self.windows[idx].layer_id;
                    self.compositor.move_layer(layer_id, nx, ny);

                    // Always resize the layer immediately so the compositor
                    // repaints the exposed background in the same frame.
                    // For IPC windows the old SHM content is clipped to the
                    // new dimensions until the client provides new content.
                    self.windows[idx].content_width = nw;
                    self.windows[idx].content_height = content_h;
                    let full_h = self.windows[idx].full_height();
                    self.compositor.resize_layer(layer_id, nw, full_h);
                    self.render_window(win_id);
                    self.push_event(
                        win_id,
                        [EVENT_RESIZE, nw, content_h, 0, 0],
                    );
                }
            }

            // Forward mouse up to focused window
            if let Some(win_id) = self.focused_window {
                if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                    let lx = self.mouse_x - self.windows[idx].x;
                    let mut ly = self.mouse_y - self.windows[idx].y;
                    if !self.windows[idx].is_borderless() {
                        ly -= TITLE_BAR_HEIGHT as i32;
                    }
                    self.push_event(win_id, [EVENT_MOUSE_UP, lx as u32, ly as u32, self.current_modifiers << 8, 0]);
                }
            }
        }
    }

    fn handle_scroll(&mut self, dz: i32) {
        if let Some(win_id) = self.focused_window {
            self.push_event(win_id, [EVENT_MOUSE_SCROLL, dz as u32, 0, 0, 0]);
        }
    }

    fn handle_key(&mut self, scancode: u32, chr: u32, mods: u32, down: bool) {
        self.current_modifiers = mods;
        let key_code = encode_scancode(scancode);

        // Volume keys: intercept globally, don't forward to apps
        if down {
            match key_code {
                KEY_VOLUME_UP => {
                    let vol = anyos_std::audio::audio_get_volume();
                    let new_vol = (vol as u32 + 5).min(100) as u8;
                    anyos_std::audio::audio_set_volume(new_vol);
                    let (sw, sh, mb) = (self.screen_width, self.screen_height, self.menubar_layer_id);
                    self.volume_hud.show(&mut self.compositor, sw, sh, new_vol, false, mb);
                    return;
                }
                KEY_VOLUME_DOWN => {
                    let vol = anyos_std::audio::audio_get_volume();
                    let new_vol = vol.saturating_sub(5);
                    anyos_std::audio::audio_set_volume(new_vol);
                    let (sw, sh, mb) = (self.screen_width, self.screen_height, self.menubar_layer_id);
                    self.volume_hud.show(&mut self.compositor, sw, sh, new_vol, false, mb);
                    return;
                }
                KEY_VOLUME_MUTE => {
                    let vol = anyos_std::audio::audio_get_volume();
                    if vol > 0 {
                        self.volume_hud.saved_volume = vol;
                        anyos_std::audio::audio_set_volume(0);
                        let (sw, sh, mb) = (self.screen_width, self.screen_height, self.menubar_layer_id);
                        self.volume_hud.show(&mut self.compositor, sw, sh, 0, true, mb);
                    } else {
                        let restore = if self.volume_hud.saved_volume > 0 {
                            self.volume_hud.saved_volume
                        } else {
                            50
                        };
                        anyos_std::audio::audio_set_volume(restore);
                        let (sw, sh, mb) = (self.screen_width, self.screen_height, self.menubar_layer_id);
                        self.volume_hud.show(&mut self.compositor, sw, sh, restore, false, mb);
                    }
                    return;
                }
                _ => {}
            }
        }

        if let Some(win_id) = self.focused_window {
            let evt_type = if down { EVENT_KEY_DOWN } else { EVENT_KEY_UP };
            self.push_event(win_id, [evt_type, key_code, chr, mods, 0]);
        }
    }

    pub(crate) fn handle_menubar_click(&mut self) {
        let mx = self.mouse_x;
        let my = self.mouse_y;
        match self.menu_bar.hit_test_menubar(mx, my) {
            MenuBarHit::SystemMenu => {
                let was_system = self.menu_bar.system_menu_open;
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                }
                if !was_system {
                    self.menu_bar
                        .open_system_menu(&mut self.compositor);
                }
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }
            MenuBarHit::MenuTitle { menu_idx } => {
                let same = self
                    .menu_bar
                    .open_dropdown
                    .as_ref()
                    .map(|d| d.menu_idx == menu_idx && !self.menu_bar.system_menu_open)
                    .unwrap_or(false);
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                }
                if !same {
                    let owner_wid = self.focused_window.unwrap_or(0);
                    self.menu_bar
                        .open_menu(menu_idx, owner_wid, &mut self.compositor);
                }
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    MENUBAR_HEIGHT + 1,
                ));
            }
            MenuBarHit::StatusIcon {
                owner_tid,
                icon_id,
            } => {
                self.push_status_icon_event(owner_tid, icon_id);
            }
            MenuBarHit::None => {
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0,
                        0,
                        self.screen_width,
                        MENUBAR_HEIGHT + 1,
                    ));
                }
            }
        }
    }

    /// Handle an action from the system menu dropdown (logo menu).
    fn handle_system_menu_action(&mut self, item_id: u32) {
        match item_id {
            crate::menu::SYS_MENU_LOGOUT => {
                self.logout_requested = true;
            }
            crate::menu::SYS_MENU_ABOUT => {
                anyos_std::process::spawn("/Applications/About anyOS.app", "");
            }
            crate::menu::SYS_MENU_SETTINGS
            | crate::menu::SYS_MENU_SLEEP => {
                // Not yet implemented
            }
            crate::menu::SYS_MENU_SHUTDOWN => {
                self.shutdown_mode = 1;
            }
            crate::menu::SYS_MENU_RESTART => {
                self.shutdown_mode = 2;
            }
            _ => {}
        }
    }

    pub(crate) fn push_status_icon_event(&mut self, owner_tid: u32, icon_id: u32) {
        for win in &mut self.windows {
            if win.owner_tid == owner_tid {
                if win.events.len() < 256 {
                    win.events
                        .push_back([EVENT_STATUS_ICON_CLICK, icon_id, 0, 0, 0]);
                }
                return;
            }
        }
        let target_sub = self.app_subs.iter()
            .find(|(t, _)| *t == owner_tid)
            .map(|(_, s)| *s);
        if self.tray_ipc_events.len() < 256 {
            self.tray_ipc_events.push((
                target_sub,
                [crate::ipc_protocol::EVT_STATUS_ICON_CLICK, 0, icon_id, 0, 0],
            ));
        }
    }

    // ── VNC Injection ──────────────────────────────────────────────────

    /// Synthesize a key event from a VNC client into the focused window.
    ///
    /// `keysym` follows the X11 / RFB KeySym convention; `vncd`'s `input.rs`
    /// maps RFB keysyms to `(scancode, char_val)` before calling this.
    /// `mods` uses the same modifier bit encoding as hardware key events.
    pub(crate) fn inject_key_event(&mut self, scancode: u32, char_val: u32, mods: u32, down: bool) {
        // Reuse the hardware path — volume-key intercept is intentionally skipped
        // for injected events so vncd cannot mute/unmute the system.
        self.current_modifiers = mods;
        if let Some(win_id) = self.focused_window {
            let key_code = encode_scancode(scancode);
            let evt_type = if down { EVENT_KEY_DOWN } else { EVENT_KEY_UP };
            self.push_event(win_id, [evt_type, key_code, char_val, mods, 0]);
        }
    }

    /// Synthesize an absolute pointer event from a VNC client.
    ///
    /// Moves the compositor cursor to `(x, y)` and dispatches a button
    /// state change if any button bit differs from the previous state.
    /// `buttons` uses the RFB mask: bit 0 = left, bit 1 = middle, bit 2 = right.
    pub(crate) fn inject_pointer_event(&mut self, x: i32, y: i32, buttons: u8) {
        // Move cursor to the absolute position.
        self.apply_mouse_move_absolute(x, y);

        // Determine which buttons changed state since last injection.
        // We track the previous VNC button mask in a dedicated field so that
        // we can synthesise proper press/release pairs.
        let prev = self.vnc_buttons;
        let changed = prev ^ buttons;

        for bit in 0..3u8 {
            if changed & (1 << bit) != 0 {
                let btn_mask = 1u32 << bit;
                let down = (buttons & (1 << bit)) != 0;
                self.handle_mouse_button(btn_mask, down);
            }
        }

        self.vnc_buttons = buttons;
    }
}
