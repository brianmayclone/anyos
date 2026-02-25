//! IPC command handling — processes app commands and drains window events.

use alloc::string::String;
use alloc::vec::Vec;

use crate::compositor::Rect;
use crate::ipc_protocol as proto;
use crate::menu::MenuBarDef;

use super::window::*;
use super::Desktop;

// ── Desktop IPC Methods ────────────────────────────────────────────────────

impl Desktop {
    /// Forward all queued window events to apps via the event channel.
    /// Returns `(target_sub_id, event)` pairs.
    pub fn drain_ipc_events(&mut self) -> Vec<(Option<u32>, [u32; 5])> {
        let mut out = Vec::new();
        for win in &mut self.windows {
            if win.owner_tid == 0 {
                continue;
            }
            let target_sub = self.app_subs.iter()
                .find(|(t, _)| *t == win.owner_tid)
                .map(|(_, s)| *s);

            while let Some(evt) = win.events.pop_front() {
                let ipc_type = match evt[0] {
                    EVENT_KEY_DOWN => proto::EVT_KEY_DOWN,
                    EVENT_KEY_UP => proto::EVT_KEY_UP,
                    EVENT_MOUSE_DOWN => proto::EVT_MOUSE_DOWN,
                    EVENT_MOUSE_UP => proto::EVT_MOUSE_UP,
                    EVENT_MOUSE_SCROLL => proto::EVT_MOUSE_SCROLL,
                    EVENT_RESIZE => proto::EVT_RESIZE,
                    EVENT_WINDOW_CLOSE => proto::EVT_WINDOW_CLOSE,
                    EVENT_MENU_ITEM => proto::EVT_MENU_ITEM,
                    EVENT_STATUS_ICON_CLICK => proto::EVT_STATUS_ICON_CLICK,
                    EVENT_MOUSE_MOVE => proto::EVT_MOUSE_MOVE,
                    EVENT_FOCUS_LOST => proto::EVT_FOCUS_LOST,
                    _ => continue,
                };
                out.push((target_sub, [ipc_type, win.id, evt[1], evt[2], evt[3]]));
            }
        }
        // Drain tray events for windowless apps
        out.extend(self.tray_ipc_events.drain(..));
        out
    }

    /// Look up the event channel subscription ID for an app by TID.
    pub fn get_sub_id_for_tid(&self, tid: u32) -> Option<u32> {
        self.app_subs.iter().find(|(t, _)| *t == tid).map(|(_, s)| *s)
    }

    /// Process an IPC command from an app.
    pub fn handle_ipc_command(&mut self, cmd: &[u32; 5]) -> Option<(Option<u32>, [u32; 5])> {
        match cmd[0] {
            proto::CMD_CREATE_WINDOW => {
                let app_tid = cmd[1];
                let wh = cmd[2];
                let width = wh >> 16;
                let height = wh & 0xFFFF;
                let xy = cmd[3];
                let raw_x = (xy >> 16) as u16;
                let raw_y = (xy & 0xFFFF) as u16;
                let shm_id_and_flags = cmd[4];
                let shm_id = shm_id_and_flags >> 16;
                let flags = shm_id_and_flags & 0xFFFF;

                if shm_id == 0 || width == 0 || height == 0 {
                    return None;
                }

                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }

                let win_id = self.create_ipc_window(
                    app_tid,
                    width,
                    height,
                    flags,
                    shm_id,
                    shm_addr as *mut u32,
                    raw_x,
                    raw_y,
                );

                let target = self.get_sub_id_for_tid(app_tid);
                Some((target, [proto::RESP_WINDOW_CREATED, win_id, shm_id, app_tid, 0]))
            }
            proto::CMD_DESTROY_WINDOW => {
                let window_id = cmd[1];
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let shm_id = self.windows[idx].shm_id;
                    let app_tid = self.windows[idx].owner_tid;
                    if shm_id > 0 {
                        anyos_std::ipc::shm_unmap(shm_id);
                    }
                    self.destroy_window(window_id);
                    let remaining = self.windows.iter().filter(|w| w.owner_tid == app_tid).count() as u32;
                    let target = self.get_sub_id_for_tid(app_tid);
                    Some((target, [proto::RESP_WINDOW_DESTROYED, window_id, app_tid, remaining, 0]))
                } else {
                    None
                }
            }
            proto::CMD_PRESENT => {
                let window_id = cmd[1];
                // Extract optional dirty rect from packed fields:
                // cmd[3] = (x << 16) | y, cmd[4] = (w << 16) | h
                // Both zero = full present (backward compatible)
                let dirty_rect = if cmd[3] != 0 || cmd[4] != 0 {
                    let x = (cmd[3] >> 16) as u32;
                    let y = (cmd[3] & 0xFFFF) as u32;
                    let w = (cmd[4] >> 16) as u32;
                    let h = (cmd[4] & 0xFFFF) as u32;
                    if w > 0 && h > 0 {
                        Some(Rect::new(x as i32, y as i32, w, h))
                    } else {
                        None
                    }
                } else {
                    None
                };
                self.present_ipc_window(window_id, dirty_rect);
                None
            }
            proto::CMD_SET_TITLE => {
                let window_id = cmd[1];
                let title_words = [cmd[2], cmd[3], cmd[4]];
                let title_bytes = proto::unpack_title(title_words);
                let len = title_bytes.iter().position(|&b| b == 0).unwrap_or(12);
                if let Ok(title) = core::str::from_utf8(&title_bytes[..len]) {
                    if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                        self.windows[idx].title = String::from(title);
                        self.render_window(window_id);
                    }
                }
                None
            }
            proto::CMD_MOVE_WINDOW => {
                let window_id = cmd[1];
                let x = cmd[2] as i32;
                let y = cmd[3] as i32;
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let layer_id = self.windows[idx].layer_id;
                    self.windows[idx].x = x;
                    self.windows[idx].y = y;
                    self.compositor.move_layer(layer_id, x, y);
                }
                None
            }
            proto::CMD_SET_MENU => {
                let window_id = cmd[1];
                let shm_id = cmd[2];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u8, 4096)
                };
                if let Some(def) = MenuBarDef::parse(data) {
                    let app_name = self.windows.iter()
                        .find(|w| w.id == window_id)
                        .map(|w| w.title.as_str())
                        .unwrap_or("App");
                    let app_name_owned = String::from(app_name);
                    self.menu_bar.set_menu(window_id, def, &app_name_owned);
                    if self.focused_window == Some(window_id) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
                anyos_std::ipc::shm_unmap(shm_id);
                let owner_tid = self.windows.iter()
                    .find(|w| w.id == window_id)
                    .map(|w| w.owner_tid)
                    .unwrap_or(0);
                let target = self.get_sub_id_for_tid(owner_tid);
                Some((target, [proto::RESP_MENU_SET, window_id, 0, owner_tid, 0]))
            }
            proto::CMD_ADD_STATUS_ICON => {
                let app_tid = cmd[1];
                let icon_id = cmd[2];
                let shm_id = cmd[3];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let pixel_data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u32, 256)
                };
                if self.menu_bar.add_status_icon(app_tid, icon_id, pixel_data, self.screen_width) {
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                }
                anyos_std::ipc::shm_unmap(shm_id);
                None
            }
            proto::CMD_REMOVE_STATUS_ICON => {
                let app_tid = cmd[1];
                let icon_id = cmd[2];
                if self.menu_bar.remove_status_icon(app_tid, icon_id, self.screen_width) {
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                }
                None
            }
            proto::CMD_UPDATE_MENU_ITEM => {
                let window_id = cmd[1];
                let item_id = cmd[2];
                let new_flags = cmd[3];
                if self.menu_bar.update_item_flags(window_id, item_id, new_flags) {
                    if self.focused_window == Some(window_id) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                    if let Some(ref dd) = self.menu_bar.open_dropdown {
                        if dd.owner_window_id == window_id {
                            self.menu_bar.render_dropdown(&mut self.compositor);
                        }
                    }
                }
                None
            }
            proto::CMD_RESIZE_SHM => {
                let window_id = cmd[1];
                let new_shm_id = cmd[2];
                let new_w = cmd[3];
                let new_h = cmd[4];

                if new_shm_id == 0 || new_w == 0 || new_h == 0 {
                    return None;
                }

                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let old_shm_id = self.windows[idx].shm_id;
                    let old_shm_ptr = self.windows[idx].shm_ptr;

                    let new_shm_addr = anyos_std::ipc::shm_map(new_shm_id);
                    if new_shm_addr == 0 {
                        return None;
                    }

                    if !old_shm_ptr.is_null() && old_shm_id != 0 {
                        anyos_std::ipc::shm_unmap(old_shm_id);
                    }

                    self.windows[idx].shm_id = new_shm_id;
                    self.windows[idx].shm_ptr = new_shm_addr as *mut u32;
                    self.windows[idx].shm_width = new_w;
                    self.windows[idx].shm_height = new_h;
                    self.windows[idx].content_width = new_w;
                    self.windows[idx].content_height = new_h;

                    let layer_id = self.windows[idx].layer_id;
                    let full_h = self.windows[idx].full_height();
                    self.compositor.resize_layer(layer_id, new_w, full_h);

                    self.render_window(window_id);
                }
                None
            }
            proto::CMD_REGISTER_SUB => {
                let app_tid = cmd[1];
                let sub_id = cmd[2];
                if let Some(entry) = self.app_subs.iter_mut().find(|(t, _)| *t == app_tid) {
                    entry.1 = sub_id;
                } else {
                    self.app_subs.push((app_tid, sub_id));
                }
                None
            }
            proto::CMD_FOCUS_BY_TID => {
                let owner_tid = cmd[1];
                if let Some(win_id) = self.windows.iter()
                    .find(|w| w.owner_tid == owner_tid)
                    .map(|w| w.id)
                {
                    if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                        if let Some((sx, sy, _sw, _sh)) = self.windows[idx].saved_bounds.take() {
                            let layer_id = self.windows[idx].layer_id;
                            self.windows[idx].x = sx;
                            self.windows[idx].y = sy;
                            self.compositor.move_layer(layer_id, sx, sy);
                        }
                    }
                    self.focus_window(win_id);
                }
                None
            }
            proto::CMD_SET_BLUR_BEHIND => {
                let window_id = cmd[1];
                let radius = cmd[2];
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let layer_id = self.windows[idx].layer_id;
                    if let Some(layer) = self.compositor.get_layer_mut(layer_id) {
                        layer.blur_behind = radius > 0;
                        layer.blur_radius = radius;
                    }
                }
                None
            }
            proto::CMD_CREATE_VRAM_WINDOW => {
                let app_tid = cmd[1];
                let wh = cmd[2];
                let width = wh >> 16;
                let height = wh & 0xFFFF;
                let xy = cmd[3];
                let raw_x = (xy >> 16) as u16;
                let raw_y = (xy & 0xFFFF) as u16;
                let flags = cmd[4];

                if width == 0 || height == 0 {
                    return None;
                }

                // Try to allocate VRAM and create the window
                if let Some(result) = self.create_vram_window(app_tid, width, height, flags, raw_x, raw_y) {
                    let target = self.get_sub_id_for_tid(app_tid);
                    Some((target, result))
                } else {
                    // VRAM allocation failed — tell app to fall back to SHM
                    let target = self.get_sub_id_for_tid(app_tid);
                    Some((target, [proto::RESP_VRAM_WINDOW_FAILED, 0, 0, app_tid, 0]))
                }
            }
            proto::CMD_SET_CLIPBOARD => {
                let shm_id = cmd[1];
                let len = cmd[2] as usize;
                let format = cmd[3];
                if shm_id == 0 || len == 0 || len > 65536 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u8, len)
                };
                self.clipboard_data = data.to_vec();
                self.clipboard_format = format;
                anyos_std::ipc::shm_unmap(shm_id);
                None
            }
            proto::CMD_GET_CLIPBOARD => {
                let shm_id = cmd[1];
                let capacity = cmd[2] as usize;
                let requester_tid = cmd[3];
                if shm_id == 0 || capacity == 0 {
                    let target = self.get_sub_id_for_tid(requester_tid);
                    return Some((target, [proto::RESP_CLIPBOARD_DATA, 0, 0, 0, requester_tid]));
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    let target = self.get_sub_id_for_tid(requester_tid);
                    return Some((target, [proto::RESP_CLIPBOARD_DATA, 0, 0, 0, requester_tid]));
                }
                let copy_len = self.clipboard_data.len().min(capacity);
                if copy_len > 0 {
                    let dst = unsafe {
                        core::slice::from_raw_parts_mut(shm_addr as *mut u8, copy_len)
                    };
                    dst.copy_from_slice(&self.clipboard_data[..copy_len]);
                }
                anyos_std::ipc::shm_unmap(shm_id);
                let target = self.get_sub_id_for_tid(requester_tid);
                Some((target, [proto::RESP_CLIPBOARD_DATA, shm_id, copy_len as u32, self.clipboard_format, requester_tid]))
            }
            proto::CMD_SET_WALLPAPER => {
                let shm_id = cmd[1];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u8, 256)
                };
                let path_len = data.iter().position(|&b| b == 0).unwrap_or(256);
                let path_len = path_len.min(127);
                if path_len > 0 {
                    self.wallpaper_path[..path_len].copy_from_slice(&data[..path_len]);
                    self.wallpaper_path_len = path_len;
                    if let Ok(_path) = core::str::from_utf8(&self.wallpaper_path[..path_len]) {
                        let mut path_buf = [0u8; 128];
                        path_buf[..path_len].copy_from_slice(&data[..path_len]);
                        let path_str = core::str::from_utf8(&path_buf[..path_len]).unwrap_or("");
                        if self.load_wallpaper(path_str) {
                            // Re-render desktop icons on top of new wallpaper
                            self.render_desktop_icons();
                            self.compositor.damage_all();
                        }
                    }
                    // Persist the wallpaper preference to disk
                    self.save_user_wallpaper();
                }
                anyos_std::ipc::shm_unmap(shm_id);
                None
            }
            proto::CMD_SHOW_NOTIFICATION => {
                let sender_tid = cmd[1];
                let shm_id = cmd[2];
                let timeout_ms = cmd[3];
                let _flags = cmd[4];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                // Parse SHM layout: [title_len:u16, msg_len:u16, has_icon:u8, pad:3, title..., msg..., icon...]
                let data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u8, 4096)
                };
                let title_len = (data[0] as u16 | ((data[1] as u16) << 8)).min(64) as usize;
                let msg_len = (data[2] as u16 | ((data[3] as u16) << 8)).min(128) as usize;
                let has_icon = data[4] != 0;

                let title_start = 8;
                let title_end = title_start + title_len;
                let msg_start = title_end;
                let msg_end = msg_start + msg_len;

                let title = core::str::from_utf8(&data[title_start..title_end]).unwrap_or("");
                let message = core::str::from_utf8(&data[msg_start..msg_end]).unwrap_or("");

                let icon = if has_icon {
                    // Icon data starts at next 4-byte aligned offset after message
                    let icon_start = (msg_end + 3) & !3;
                    if icon_start + 1024 <= 4096 {
                        let icon_slice = unsafe {
                            core::slice::from_raw_parts(
                                (shm_addr as usize + icon_start) as *const u32,
                                256,
                            )
                        };
                        let mut icon_buf = [0u32; 256];
                        icon_buf.copy_from_slice(icon_slice);
                        Some(icon_buf)
                    } else {
                        None
                    }
                } else {
                    None
                };

                anyos_std::ipc::shm_unmap(shm_id);

                if title.is_empty() {
                    return None;
                }

                let menubar_id = self.menubar_layer_id;
                let sw = self.screen_width;
                let notif_id = self.notifications.show(
                    &mut self.compositor,
                    sw,
                    menubar_id,
                    sender_tid,
                    title,
                    message,
                    icon,
                    timeout_ms,
                );

                // Broadcast EVT_NOTIFICATION_DISMISSED will be sent later on dismiss
                None
            }
            proto::CMD_DISMISS_NOTIFICATION => {
                let notif_id = cmd[1];
                let sw = self.screen_width;
                self.notifications.dismiss(notif_id, sw);
                None
            }
            proto::CMD_GET_WINDOW_POS => {
                let window_id = cmd[1];
                let requester_tid = cmd[2];
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let win = &self.windows[idx];
                    let content_x = win.x;
                    let content_y = if win.is_borderless() {
                        win.y
                    } else {
                        win.y + TITLE_BAR_HEIGHT as i32
                    };
                    let target = self.get_sub_id_for_tid(requester_tid);
                    Some((target, [proto::RESP_WINDOW_POS, window_id, content_x as u32, content_y as u32, requester_tid]))
                } else {
                    let target = self.get_sub_id_for_tid(requester_tid);
                    Some((target, [proto::RESP_WINDOW_POS, window_id, 0, 0, requester_tid]))
                }
            }
            proto::CMD_INJECT_KEY => {
                // vncd: relay keyboard input from VNC client into the focused window.
                // [CMD, scancode, char_val, is_down (1/0), modifiers]
                let scancode = cmd[1];
                let char_val = cmd[2];
                let is_down = cmd[3] != 0;
                let mods = cmd[4];
                self.inject_key_event(scancode, char_val, mods, is_down);
                None
            }
            proto::CMD_INJECT_POINTER => {
                // vncd: relay pointer input from VNC client — move cursor and dispatch click.
                // [CMD, x, y, buttons_mask, 0]
                let x = cmd[1] as i32;
                let y = cmd[2] as i32;
                let buttons = cmd[3] as u8;
                self.inject_pointer_event(x, y, buttons);
                None
            }
            _ => None,
        }
    }

    // ── Pre-mapped IPC Handlers ────────────────────────────────────────

    /// Handle CMD_CREATE_WINDOW with a pre-mapped SHM address.
    pub fn handle_create_window_pre_mapped(
        &mut self,
        cmd: &[u32; 5],
        shm_addr: usize,
    ) -> Option<(Option<u32>, [u32; 5])> {
        let app_tid = cmd[1];
        let wh = cmd[2];
        let width = wh >> 16;
        let height = wh & 0xFFFF;
        let xy = cmd[3];
        let raw_x = (xy >> 16) as u16;
        let raw_y = (xy & 0xFFFF) as u16;
        let shm_id_and_flags = cmd[4];
        let shm_id = shm_id_and_flags >> 16;
        let flags = shm_id_and_flags & 0xFFFF;

        if shm_id == 0 || width == 0 || height == 0 || shm_addr == 0 {
            return None;
        }

        let win_id = self.create_ipc_window(
            app_tid,
            width,
            height,
            flags,
            shm_id,
            shm_addr as *mut u32,
            raw_x,
            raw_y,
        );

        let target = self.get_sub_id_for_tid(app_tid);
        Some((target, [proto::RESP_WINDOW_CREATED, win_id, shm_id, app_tid, 0]))
    }

    /// Handle CMD_RESIZE_SHM with a pre-mapped new SHM address.
    pub fn handle_resize_shm_pre_mapped(
        &mut self,
        cmd: &[u32; 5],
        new_shm_addr: usize,
    ) -> Option<(Option<u32>, [u32; 5])> {
        let window_id = cmd[1];
        let new_shm_id = cmd[2];
        let new_w = cmd[3];
        let new_h = cmd[4];

        if new_shm_id == 0 || new_w == 0 || new_h == 0 || new_shm_addr == 0 {
            return None;
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
            let old_shm_id = self.windows[idx].shm_id;
            let old_shm_ptr = self.windows[idx].shm_ptr;

            if !old_shm_ptr.is_null() && old_shm_id != 0 {
                anyos_std::ipc::shm_unmap(old_shm_id);
            }

            self.windows[idx].shm_id = new_shm_id;
            self.windows[idx].shm_ptr = new_shm_addr as *mut u32;
            self.windows[idx].shm_width = new_w;
            self.windows[idx].shm_height = new_h;
            self.windows[idx].content_width = new_w;
            self.windows[idx].content_height = new_h;

            let layer_id = self.windows[idx].layer_id;
            let full_h = self.windows[idx].full_height();
            self.compositor.resize_layer(layer_id, new_w, full_h);

            self.render_window(window_id);
        }
        None
    }
}
