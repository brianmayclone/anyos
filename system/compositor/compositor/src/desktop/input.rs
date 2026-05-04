//! Input handling — mouse, keyboard, scroll, drag, and resize interaction.

use crate::compositor::Rect;
use crate::keys::{
    encode_scancode, KEY_DELETE, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_F1, KEY_F10, KEY_F11,
    KEY_F12, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9, KEY_LEFT,
    KEY_LEFT_SUPER, KEY_RIGHT, KEY_RIGHT_SUPER, KEY_TAB, KEY_UP, KEY_VOLUME_DOWN, KEY_VOLUME_MUTE,
    KEY_VOLUME_UP,
};
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
    fn pointer_locked_window(&self) -> Option<u32> {
        if self.current_cursor != CursorShape::Hidden {
            return None;
        }
        if let Some(fs_win_id) = self.fullscreen_window {
            return Some(fs_win_id);
        }
        self.focused_window
    }

    fn topmost_hit_in_group<F>(
        &self,
        mx: i32,
        my: i32,
        ipc_only: bool,
        predicate: F,
    ) -> Option<(u32, HitTest)>
    where
        F: Fn(&WindowInfo) -> bool,
    {
        for win in self.windows.iter().rev() {
            if ipc_only && win.owner_tid == 0 {
                continue;
            }
            if !predicate(win) {
                continue;
            }
            let hit = win.hit_test(mx, my);
            if hit == HitTest::None {
                continue;
            }
            // Multi-monitor: when the hit lands on the title bar, see if
            // the cursor is over one of the right-side "send to monitor"
            // buttons and upgrade the HitTest accordingly. Done here
            // (Desktop level) rather than inside WindowInfo::hit_test
            // because the per-window button layout depends on
            // self.compositor.outputs which WindowInfo can't see.
            if hit == HitTest::TitleBar && self.compositor.outputs.len() >= 2 {
                let wx = mx - win.x;
                let wy = my - win.y;
                let by = super::window::monitor_btn_y();
                let bw = super::window::monitor_btn_w() as i32;
                let bh = super::window::monitor_btn_h() as i32;
                if wy >= by && wy < by + bh {
                    let other_ids = self.other_outputs_for_window(win.id);
                    for (slot, &target_id) in other_ids.iter().enumerate() {
                        let bx = super::window::monitor_btn_x_at(
                            win.content_width,
                            slot as u32,
                        );
                        if wx >= bx && wx < bx + bw {
                            return Some((win.id, HitTest::MonitorButton(target_id)));
                        }
                    }
                }
            }
            return Some((win.id, hit));
        }
        None
    }

    pub(crate) fn topmost_window_hit(
        &self,
        mx: i32,
        my: i32,
        ipc_only: bool,
    ) -> Option<(u32, HitTest)> {
        self.topmost_hit_in_group(mx, my, ipc_only, |w| w.is_always_on_top())
            .or_else(|| {
                self.topmost_hit_in_group(mx, my, ipc_only, |w| {
                    !w.is_always_on_top() && w.modal_owner != 0
                })
            })
            .or_else(|| {
                self.topmost_hit_in_group(mx, my, ipc_only, |w| {
                    !w.is_always_on_top() && w.modal_owner == 0
                })
            })
    }

    /// Process a batch of raw input events. Returns true if a compose is needed.
    pub fn process_input(&mut self, events: &[[u32; 5]], count: usize) -> bool {
        let mut needs_compose = false;
        let mut cursor_moved = false;
        let mut last_dx: i32 = 0;
        let mut last_dy: i32 = 0;
        let mut absolute_move = false;
        // True when the absolute event already carries virtual-desktop
        // coords (per-output virtio-input). Skips the legacy
        // delta-derivation fallback that exists for vmmouse-scoped
        // events on multi-monitor.
        let mut absolute_translated = false;
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
                    let raw_x = evt[1] as i32;
                    let raw_y = evt[2] as i32;
                    // arg3 = producing output id (multi-monitor), or
                    // 0xFF for legacy paths (vmmouse / VMMDev). When
                    // a per-output virtio-input device produced the
                    // event, raw_x/raw_y are in that output's local
                    // pixel coords — translate to virtual desktop
                    // coords via the output's virtual_x/y. Translated
                    // events are "real" absolute and bypass the
                    // multi-monitor delta-derivation safety net in
                    // apply_mouse_move_absolute.
                    let oid = evt[4] as u8;
                    if oid != 0xFF {
                        if let Some(o) = self
                            .compositor
                            .outputs
                            .iter()
                            .find(|o| o.id as u8 == oid)
                        {
                            abs_x = o.virtual_x + raw_x;
                            abs_y = o.virtual_y + raw_y;
                        } else {
                            abs_x = raw_x;
                            abs_y = raw_y;
                        }
                        absolute_translated = true;
                    } else {
                        abs_x = raw_x;
                        abs_y = raw_y;
                        absolute_translated = false;
                    }
                    absolute_move = true;
                    cursor_moved = true;
                    last_dx = 0;
                    last_dy = 0;
                }
                INPUT_MOUSE_BUTTON => {
                    if cursor_moved {
                        if absolute_move {
                            if absolute_translated {
                                // Coords are already in virtual desktop
                                // — skip the delta-derivation that the
                                // legacy multi-monitor branch does.
                                self.apply_mouse_move_absolute_virtual(abs_x, abs_y);
                            } else {
                                self.apply_mouse_move_absolute(abs_x, abs_y);
                            }
                            absolute_move = false;
                            absolute_translated = false;
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
                if absolute_translated {
                    self.apply_mouse_move_absolute_virtual(abs_x, abs_y);
                } else {
                    self.apply_mouse_move_absolute(abs_x, abs_y);
                }
            } else {
                self.apply_mouse_move(last_dx, last_dy);
            }
            needs_compose = true;
        }

        needs_compose
    }

    pub(crate) fn apply_mouse_move(&mut self, dx: i32, dy: i32) {
        // Pointer lock: when the focused app hides the cursor, skip clamping,
        // HW cursor movement, drag/resize/hover logic and forward raw deltas.
        if let Some(lock_win_id) = self.pointer_locked_window() {
            // Accumulate unclamped so the app can compute deltas normally
            self.mouse_x += dx;
            self.mouse_y += dy;
            self.push_event(
                lock_win_id,
                [
                    EVENT_MOUSE_MOVE,
                    self.mouse_x as u32,
                    self.mouse_y as u32,
                    0,
                    0,
                ],
            );
            return;
        }

        // Multi-monitor cursor clamping. The cursor lives in virtual desktop
        // coordinates; outputs ≥ 1 sit to the right of the primary, so the
        // legal x range is the bounding box of all output rects, not just
        // the primary's screen_width. y is clamped to the union as well.
        // For pure single-output setups virtual_desktop_bounds() returns
        // (0, 0, screen_width, screen_height) so the behaviour is identical.
        let (vmin_x, vmin_y, vmax_x, vmax_y) = self.compositor.virtual_desktop_bounds();
        self.mouse_x = (self.mouse_x + dx).clamp(vmin_x, vmax_x - 1);
        self.mouse_y = (self.mouse_y + dy).clamp(vmin_y, vmax_y - 1);

        // Handle window drag — clamp Y so windows can never go under the menubar.
        if let Some(ref mut drag) = self.dragging {
            drag.moved = true;
            let win_id = drag.window_id;
            let new_x = self.mouse_x - drag.offset_x;
            let min_y = menubar_height() as i32 + 1;
            let new_y = (self.mouse_y - drag.offset_y).max(min_y);
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
                if self.mouse_y < menubar_height() as i32 {
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
                            0,
                            0,
                            self.screen_width,
                            menubar_height() + 1,
                        ));
                    }
                }
            } else {
                // App menu hover update
                if self.menu_bar.update_hover(self.mouse_x, self.mouse_y) {
                    self.menu_bar.render_dropdown(&mut self.compositor);
                }
                // Slide between app menus or to system menu
                if self.mouse_y < menubar_height() as i32 {
                    match self.menu_bar.hit_test_menubar(self.mouse_x, self.mouse_y) {
                        MenuBarHit::SystemMenu => {
                            self.menu_bar
                                .close_dropdown_with_compositor(&mut self.compositor);
                            self.menu_bar.open_system_menu(&mut self.compositor);
                            self.draw_menubar();
                            self.compositor.add_damage(Rect::new(
                                0,
                                0,
                                self.screen_width,
                                menubar_height() + 1,
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
                                    0,
                                    0,
                                    self.screen_width,
                                    menubar_height() + 1,
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Update HW cursor position
        // Multi-monitor HW cursor routing: locate the output whose
        // virtual rect currently contains the cursor, send a per-
        // scanout MOVE_CURSOR with the local coordinates, and hide /
        // show the cursor on the others as needed. Single-output
        // setups skip the routing and go through the legacy path.
        if self.compositor.outputs.len() >= 2 {
            // Find the output the cursor is on. output_at falls back
            // to the primary if the cursor is in a gap, which is
            // what we want.
            let (target_id, lx, ly) = {
                let o = self.compositor.output_at(self.mouse_x, self.mouse_y);
                let lx = (self.mouse_x - o.virtual_x).max(0);
                let ly = (self.mouse_y - o.virtual_y).max(0);
                (o.id, lx, ly)
            };
            // If the cursor crossed an output boundary since the
            // last move, hide it on the previous one before showing
            // on the new one. We track the last output id in
            // self.last_cursor_output (None on first call).
            let prev = self.last_cursor_output.replace(target_id);
            if prev != Some(target_id) {
                if let Some(p) = prev {
                    if p != target_id {
                        self.compositor.set_hw_cursor_visible_on_output(p, false);
                    }
                }
                self.compositor.set_hw_cursor_visible_on_output(target_id, true);
            }
            self.compositor.move_hw_cursor_on_output(target_id, lx, ly);
        } else {
            // Single-output: legacy path. Clamp inside the primary's
            // bounds so the HW cursor never lands on a garbage offscreen
            // address.
            let pw = self.compositor.width() as i32;
            let ph = self.compositor.height() as i32;
            let hw_x = self.mouse_x.clamp(0, pw - 1);
            let hw_y = self.mouse_y.clamp(0, ph - 1);
            self.compositor.move_hw_cursor(hw_x, hw_y);
        }

        // Update cursor shape
        if self.dragging.is_some() {
            self.set_cursor_shape(CursorShape::Move);
        } else if self.resizing.is_some() {
            // Keep current resize cursor
        } else {
            let mx = self.mouse_x;
            let my = self.mouse_y;
            let hit = self.topmost_window_hit(mx, my, false);
            // Honour an app-requested cursor override while the pointer is
            // still on the focused window's content area. This lets an app
            // keep e.g. the Move cursor during its own drag operation
            // without us snapping it back to Arrow on the next motion event.
            let app_override = self.app_cursor.filter(|_| {
                matches!(
                    hit,
                    Some((win_id, HitTest::Content))
                        if Some(win_id) == self.focused_window
                )
            });
            let shape = app_override.unwrap_or_else(|| match hit {
                Some((_, h)) => self.cursor_for_hit(h),
                None => CursorShape::Arrow,
            });
            self.set_cursor_shape(shape);
        }

        // Track button hover for animated colour transitions
        if self.has_gpu_accel && self.dragging.is_none() && self.resizing.is_none() {
            let new_hover = self.get_button_under_cursor();
            if new_hover != self.btn_hover {
                let now = anyos_std::sys::uptime();
                if let Some((old_wid, old_btn)) = self.btn_hover {
                    let aid = button_anim_id(old_wid, old_btn);
                    self.btn_anims
                        .start(aid, 1000, 0, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(old_wid);
                }
                if let Some((new_wid, new_btn)) = new_hover {
                    let aid = button_anim_id(new_wid, new_btn);
                    self.btn_anims
                        .start(aid, 0, 1000, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(new_wid);
                }
                self.btn_hover = new_hover;
            }
        }

        // Cross-window drag: route DRAG_OVER / DRAG_ENTER / DRAG_LEAVE.
        // While a global drag is active we suppress regular MOUSE_MOVE
        // forwarding; the target window receives motion via EVT_DRAG_OVER
        // and the source learns about target changes via EVT_DRAG_FEEDBACK.
        if self.global_drag.is_some() {
            self.drag_update_target(self.mouse_x, self.mouse_y);
            return;
        }

        // Forward mouse move to topmost IPC window under cursor
        if self.dragging.is_none() && self.resizing.is_none() {
            if let Some((win_id, _)) = self.topmost_window_hit(self.mouse_x, self.mouse_y, true) {
                if let Some(win) = self.windows.iter().find(|w| w.id == win_id) {
                    let lx = self.mouse_x - win.x;
                    let mut ly = self.mouse_y - win.y;
                    if !win.is_borderless() {
                        ly -= title_bar_height() as i32;
                    }
                    self.push_event(win.id, [EVENT_MOUSE_MOVE, lx as u32, ly as u32, 0, 0]);
                }
            }
        }
    }

    /// Multi-monitor absolute event whose `(x, y)` are already in
    /// virtual-desktop coordinates (translated by the kernel-side
    /// per-output virtio-input path). Bypasses the delta-derivation
    /// safety net that exists for legacy vmmouse-scoped absolute
    /// events. Just clamps to the virtual desktop bounds and sets
    /// the cursor straight to the requested point.
    fn apply_mouse_move_absolute_virtual(&mut self, x: i32, y: i32) {
        let (vmin_x, vmin_y, vmax_x, vmax_y) = self.compositor.virtual_desktop_bounds();
        let target_x = x.clamp(vmin_x, vmax_x - 1);
        let target_y = y.clamp(vmin_y, vmax_y - 1);
        let dx = target_x - self.mouse_x;
        let dy = target_y - self.mouse_y;
        if dx != 0 || dy != 0 {
            // Re-enter the relative path so window-drag / resize /
            // hover state stays consistent — same approach the legacy
            // absolute path uses on single-output.
            self.apply_mouse_move(dx, dy);
        }
    }

    /// Apply an absolute mouse position (from VMMDev).
    fn apply_mouse_move_absolute(&mut self, x: i32, y: i32) {
        // Multi-monitor escape hatch: QEMU's SDL multi-window backend
        // can't tell the guest which window a vmmouse click came from
        // — the absolute coords are always scoped to the primary
        // scanout's framebuffer dimensions, so any click on a
        // secondary SDL window mis-routes back to the primary and the
        // cursor can never reach a secondary at all.
        //
        // When more than one output is active we convert the absolute
        // event into a relative delta against the previous absolute
        // position, then funnel that delta through the relative path
        // — that one is correctly output-agnostic because dx/dy
        // accumulate across the virtual desktop. Dropping the events
        // entirely (the previous defensive behaviour) leaves the
        // cursor frozen if vmmouse stays engaged for any reason —
        // a docked tablet, a stale --machine pc setting, or a SPICE
        // vdagent path that still routes absolute. The delta-fallback
        // keeps the cursor live regardless.
        if self.compositor.outputs.len() >= 2 {
            let prev_x = self.last_absolute_mouse_x.replace(x);
            let prev_y = self.last_absolute_mouse_y.replace(y);
            if let (Some(px), Some(py)) = (prev_x, prev_y) {
                let dx = x - px;
                let dy = y - py;
                if dx != 0 || dy != 0 {
                    self.apply_mouse_move(dx, dy);
                }
            }
            return;
        }

        // Absolute pointer (vmmouse / tablet) — clamp to the union of
        // all outputs, same logic as the relative path above.
        let (vmin_x, _vmin_y, vmax_x, _vmax_y) = self.compositor.virtual_desktop_bounds();
        let target_x = x.clamp(vmin_x, vmax_x - 1);
        let (_vmin_x2, vmin_y, _vmax_x2, vmax_y) = self.compositor.virtual_desktop_bounds();
        let target_y = y.clamp(vmin_y, vmax_y - 1);

        if self.pointer_locked_window().is_some() {
            let prev_x = self.last_absolute_mouse_x.replace(target_x);
            let prev_y = self.last_absolute_mouse_y.replace(target_y);
            if let (Some(px), Some(py)) = (prev_x, prev_y) {
                let dx = target_x - px;
                let dy = target_y - py;
                if dx != 0 || dy != 0 {
                    self.apply_mouse_move(dx, dy);
                }
            }
            return;
        }

        self.last_absolute_mouse_x = Some(target_x);
        self.last_absolute_mouse_y = Some(target_y);

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
        if let Some((win_id, ht)) = self.topmost_window_hit(mx, my, false) {
            match ht {
                HitTest::CloseButton => return Some((win_id, 0)),
                HitTest::MinButton => return Some((win_id, 1)),
                HitTest::MaxButton => return Some((win_id, 2)),
                _ => return None,
            }
        }
        None
    }

    /// Tick active animations. Returns true if any animation was active.
    pub fn tick_animations(&mut self) -> bool {
        let now = anyos_std::sys::uptime();

        // Always tick overlays (independent of button animations)
        let hud_active = self.volume_hud.tick(&mut self.compositor);

        if !self.btn_anims.has_active(now) {
            return hud_active;
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

        wid_count > 0 || hud_active
    }

    pub(crate) fn handle_mouse_button(&mut self, buttons: u32, down: bool) {
        let previous_buttons = self.mouse_buttons;

        if down {
            if buttons == previous_buttons || (buttons & !previous_buttons) == 0 {
                return;
            }
            self.mouse_buttons = buttons;

            // Check if clicking within the shortcut overlay
            if self.shortcut_overlay_visible {
                let is_inside = self.is_point_in_shortcut_overlay(self.mouse_x, self.mouse_y);
                let newly_pressed = buttons & !previous_buttons;
                let right_click = (newly_pressed & 2) != 0;
                if is_inside {
                    // Right-click on an occupied card cycles the window
                    // to the next output (multi-monitor convenience).
                    // Stays inside the overlay so the user can chain the
                    // gesture for cards on still-other monitors.
                    if right_click {
                        if let Some(slot) =
                            self.hit_test_shortcut_overlay(self.mouse_x, self.mouse_y)
                        {
                            let win_id = self.fkey_slots[slot];
                            if win_id != 0 && self.compositor.outputs.len() >= 2 {
                                let others = self.other_outputs_for_window(win_id);
                                if let Some(&first_other) = others.first() {
                                    self.move_window_to_output(win_id, first_other);
                                    self.render_shortcut_overlay();
                                    self.compositor.damage_all();
                                }
                            }
                            return;
                        }
                    }
                    // Check close button (X) first
                    if let Some(slot) =
                        self.hit_test_shortcut_overlay_close(self.mouse_x, self.mouse_y)
                    {
                        let win_id = self.fkey_slots[slot];
                        if win_id != 0 {
                            // Send close event to the window
                            self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                        }
                        // Re-render overlay after a short delay (window will be removed by app)
                        // For now, just re-render — the slot will update on next open
                        return;
                    }
                    if let Some(slot) = self.hit_test_shortcut_overlay(self.mouse_x, self.mouse_y) {
                        // Clicked on a slot card — focus that window
                        let win_id = self.fkey_slots[slot];
                        self.close_shortcut_overlay();
                        if win_id != 0 {
                            // Un-minimize if needed
                            if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                                if self.windows[idx].x < -9000 {
                                    if let Some((sx, sy, _sw, _sh)) =
                                        self.windows[idx].saved_bounds.take()
                                    {
                                        self.windows[idx].x = sx;
                                        self.windows[idx].y = sy;
                                        let layer_id = self.windows[idx].layer_id;
                                        self.compositor.move_layer(layer_id, sx, sy);
                                    }
                                }
                            }
                            self.focus_window(win_id);
                        }
                    }
                    // Clicked inside overlay but not on a card — absorb click
                    return;
                } else {
                    // Clicked outside overlay — close it
                    self.close_shortcut_overlay();
                    return;
                }
            }

            // Check if clicking within open dropdown
            if self.menu_bar.is_dropdown_open() {
                if self.menu_bar.is_in_dropdown(self.mouse_x, self.mouse_y) {
                    if self.menu_bar.system_menu_open {
                        // System menu dropdown click
                        if let Some(item_id) = self
                            .menu_bar
                            .hit_test_system_menu(self.mouse_x, self.mouse_y)
                        {
                            self.handle_system_menu_action(item_id);
                        }
                    } else if let Some(item_id) =
                        self.menu_bar.hit_test_dropdown(self.mouse_x, self.mouse_y)
                    {
                        // App menu dropdown click
                        if let Some(win_id) = self.focused_window {
                            match item_id {
                                crate::menu::APP_MENU_QUIT => {
                                    self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                                }
                                crate::menu::APP_MENU_HIDE => {
                                    if let Some(idx) =
                                        self.windows.iter().position(|w| w.id == win_id)
                                    {
                                        let layer_id = self.windows[idx].layer_id;
                                        self.windows[idx].saved_bounds = Some((
                                            self.windows[idx].x,
                                            self.windows[idx].y,
                                            self.windows[idx].content_width,
                                            self.windows[idx].full_height(),
                                        ));
                                        self.compositor.move_layer(layer_id, -10000, -10000);
                                    }
                                    let next = self
                                        .windows
                                        .iter()
                                        .rev()
                                        .find(|w| w.id != win_id && w.x >= 0)
                                        .map(|w| w.id);
                                    if let Some(nid) = next {
                                        self.focus_window(nid);
                                    }
                                }
                                _ => {
                                    let menu_idx = self
                                        .menu_bar
                                        .open_dropdown
                                        .as_ref()
                                        .map(|d| d.menu_idx as u32)
                                        .unwrap_or(0);
                                    self.push_event(
                                        win_id,
                                        [EVENT_MENU_ITEM, menu_idx, item_id, 0, 0],
                                    );
                                }
                            }
                        }
                    }
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0,
                        0,
                        self.screen_width,
                        menubar_height() + 1,
                    ));
                    return;
                }

                if self.fullscreen_window.is_none() && self.mouse_y < menubar_height() as i32 {
                    self.handle_menubar_click();
                    return;
                }
                self.menu_bar
                    .close_dropdown_with_compositor(&mut self.compositor);
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
                ));
            }

            // Check menubar click (skip in fullscreen — menubar is hidden)
            if self.fullscreen_window.is_none() && self.mouse_y < menubar_height() as i32 {
                self.handle_menubar_click();
                return;
            }

            // Check window hits
            let mx = self.mouse_x;
            let my = self.mouse_y;

            if let Some((win_id, hit_test)) = self.topmost_window_hit(mx, my, false) {
                // If the clicked window has a modal child, block the click and
                // redirect focus to the modal child instead.
                let has_modal_child = self.windows.iter().any(|w| w.modal_owner == win_id);
                if has_modal_child {
                    // focus_window will walk the chain to the topmost modal
                    self.focus_window(win_id);
                    return;
                }

                if self.focused_window != Some(win_id) {
                    self.focus_window(win_id);
                }

                match hit_test {
                    HitTest::CloseButton => {
                        let no_close = self
                            .windows
                            .iter()
                            .find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_CLOSE != 0)
                            .unwrap_or(false);
                        if !no_close {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 0));
                                let aid = button_anim_id(win_id, 0);
                                self.btn_anims.start(
                                    aid,
                                    0,
                                    1000,
                                    100,
                                    anyos_std::anim::Easing::EaseOut,
                                );
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
                                    moved: false,
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
                        let no_min = self
                            .windows
                            .iter()
                            .find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_MINIMIZE != 0)
                            .unwrap_or(false);
                        if !no_min {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 1));
                                let aid = button_anim_id(win_id, 1);
                                self.btn_anims.start(
                                    aid,
                                    0,
                                    1000,
                                    100,
                                    anyos_std::anim::Easing::EaseOut,
                                );
                                self.render_window(win_id);
                            }
                            self.minimize_window(win_id);
                        }
                    }
                    HitTest::MaxButton => {
                        let no_max = self
                            .windows
                            .iter()
                            .find(|w| w.id == win_id)
                            .map(|w| w.flags & WIN_FLAG_NO_MAXIMIZE != 0)
                            .unwrap_or(false);
                        if !no_max {
                            if self.has_gpu_accel {
                                self.btn_pressed = Some((win_id, 2));
                                let aid = button_anim_id(win_id, 2);
                                self.btn_anims.start(
                                    aid,
                                    0,
                                    1000,
                                    100,
                                    anyos_std::anim::Easing::EaseOut,
                                );
                                self.render_window(win_id);
                            }
                            self.toggle_maximize(win_id);
                        }
                    }
                    HitTest::ShortcutButton => {
                        self.toggle_shortcut_overlay();
                    }
                    HitTest::MonitorButton(target_id) => {
                        // Multi-monitor "send window to monitor N" button.
                        // Move the window to the target output, preserving
                        // the position-relative-to-source-output where it
                        // fits and clamping into the target rect otherwise.
                        self.move_window_to_output(win_id, target_id);
                    }
                    HitTest::Content => {
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            let lx = mx - self.windows[idx].x;
                            let ly = my - self.windows[idx].y;
                            let mut content_ly = ly;
                            if !self.windows[idx].is_borderless() {
                                content_ly -= title_bar_height() as i32;
                            }
                            // Remember which window owns the in-progress
                            // press. Subsequent MOUSE_UP gets routed back
                            // here instead of to the (possibly newly
                            // focused) window — so click handlers that
                            // open dialogs don't strand the source widget
                            // with a stale pressed_button.
                            self.mouse_down_capture = Some(win_id);
                            self.push_event(
                                win_id,
                                [
                                    EVENT_MOUSE_DOWN,
                                    lx as u32,
                                    content_ly as u32,
                                    buttons | (self.current_modifiers << 8),
                                    0,
                                ],
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
                // Clicked on empty desktop — defocus window
                if let Some(old_id) = self.focused_window {
                    if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                        self.windows[idx].focused = false;
                        let win_id = self.windows[idx].id;
                        self.render_titlebar(win_id);
                        self.push_event(win_id, [EVENT_FOCUS_LOST, 0, 0, 0, 0]);
                    }
                    self.focused_window = None;
                    self.app_cursor = None;
                    self.compositor.set_focused_layer(None);
                    self.emit_focus_changed(0, 0);
                }
            }
        } else {
            // Mouse up
            if previous_buttons == buttons || previous_buttons == 0 {
                return;
            }
            let released_buttons = previous_buttons & !buttons;
            if released_buttons == 0 {
                self.mouse_buttons = buttons;
                return;
            }
            self.mouse_buttons = buttons;

            if let Some((wid, btn)) = self.btn_pressed.take() {
                if self.has_gpu_accel {
                    let aid = button_anim_id(wid, btn);
                    self.btn_anims
                        .start(aid, 1000, 0, 150, anyos_std::anim::Easing::EaseOut);
                    self.render_window(wid);
                }
            }

            // End drag — re-enable shadow + edge snapping
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
                // Edge snapping: snap to half-screen when dragged to screen edge.
                // Only activate if the mouse actually moved during the drag —
                // otherwise a simple click on the title bar near a screen edge
                // would trigger an unwanted snap (e.g. after "Fenster anordnen").
                if drag.moved {
                    let snap_margin = 20i32;
                    let mx = self.mouse_x;
                    let sw = self.screen_width as i32;
                    if mx <= snap_margin {
                        self.snap_window_to_half(win_id, 0); // left half
                    } else if mx >= sw - snap_margin - 1 {
                        self.snap_window_to_half(win_id, 1); // right half
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
                        nh.saturating_sub(title_bar_height())
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
                    self.push_event(win_id, [EVENT_RESIZE, nw, content_h, 0, 0]);
                }
            }

            // Cross-window drag: finish the drag (drop or cancel) instead
            // of forwarding a regular MOUSE_UP. EVT_DROP / EVT_DRAG_END are
            // dispatched to the target / source by drag_finish_on_release.
            if self.global_drag.is_some() {
                self.drag_finish_on_release(self.mouse_x, self.mouse_y);
                return;
            }

            // Forward mouse up to whichever window started this press
            // (implicit mouse capture). Falls back to the focused window
            // if no capture was registered (e.g. press happened on a
            // non-content hit area).
            let target = self
                .mouse_down_capture
                .take()
                .or(self.focused_window);
            if let Some(win_id) = target {
                if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                    let lx = self.mouse_x - self.windows[idx].x;
                    let mut ly = self.mouse_y - self.windows[idx].y;
                    if !self.windows[idx].is_borderless() {
                        ly -= title_bar_height() as i32;
                    }
                    self.push_event(
                        win_id,
                        [
                            EVENT_MOUSE_UP,
                            lx as u32,
                            ly as u32,
                            self.current_modifiers << 8,
                            0,
                        ],
                    );
                }
            }
        }
    }

    fn handle_scroll(&mut self, dz: i32) {
        if let Some(win_id) = self.focused_window {
            self.push_event(
                win_id,
                [EVENT_MOUSE_SCROLL, dz as u32, self.current_modifiers, 0, 0],
            );
        }
    }

    fn handle_key(&mut self, scancode: u32, chr: u32, mods: u32, down: bool) {
        self.current_modifiers = mods;
        let key_code = encode_scancode(scancode);
        let ctrl = mods & 2 != 0;
        let alt = mods & 4 != 0;

        // Debug: log Escape key events
        if key_code == KEY_ESCAPE && down {
            anyos_std::println!(
                "[compositor] ESC down: scancode=0x{:x} mods=0x{:x} ctrl={} alt={}",
                scancode,
                mods,
                ctrl,
                alt
            );
        }

        // ESC cancels an active cross-window drag, regardless of focus.
        // Apps that wanted to use ESC for their own UI still get the key
        // event; we just additionally tear the drag down.
        if key_code == KEY_ESCAPE && down && self.global_drag.is_some() {
            if let Some(d) = self.global_drag.as_ref() {
                let tid = d.source_tid;
                let wid = d.source_window_id;
                self.drag_cancel(tid, wid);
            }
        }

        // ── Super key tap detection ─────────────────────────────────────────
        if key_code == KEY_LEFT_SUPER || key_code == KEY_RIGHT_SUPER {
            self.super_held = down;
            if down {
                self.super_key_solo = true;
            } else if self.super_key_solo {
                // Super released without pressing any other key → toggle system menu
                self.super_key_solo = false;
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                } else {
                    self.menu_bar.open_system_menu(&mut self.compositor);
                    // Set hover to first non-separator item for keyboard nav
                    if let Some(ref mut dd) = self.menu_bar.open_dropdown {
                        dd.hover_idx = Some(0);
                    }
                    self.menu_bar.rerender_system_dropdown(&mut self.compositor);
                }
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
                ));
                // Also broadcast for apps that want to react
                if self.tray_ipc_events.len() < 256 {
                    self.tray_ipc_events
                        .push((None, [crate::ipc_protocol::EVT_SUPER_KEY, 0, 0, 0, 0]));
                }
            }
            return;
        }
        // Any other key pressed while Super is held cancels the solo-tap
        if down {
            self.super_key_solo = false;
        }

        // ── System hotkeys (intercepted before apps) ──────────────────────

        if down {
            // Ctrl+Alt+Delete: System escape — exit fullscreen + show system dialog
            if ctrl && alt && key_code == KEY_DELETE {
                if let Some(fs_win) = self.fullscreen_window {
                    // Notify the fullscreen app that fullscreen is ending
                    self.push_event(fs_win, [EVENT_FULLSCREEN_EXIT, 0, 0, 0, 0]);
                    self.exit_fullscreen();
                }
                // TODO: Phase 8 — show system dialog (force-quit, task manager, logout, shutdown)
                return;
            }

            // Alt+Enter: Fullscreen toggle
            if alt && key_code == KEY_ENTER {
                if let Some(fs_win) = self.fullscreen_window {
                    // Exit fullscreen — notify app
                    self.push_event(fs_win, [EVENT_FULLSCREEN_EXIT, 0, 0, 0, 0]);
                    self.exit_fullscreen();
                } else if let Some(focused) = self.focused_window {
                    // Enter fullscreen — only if the app registered as fullscreen-capable
                    let is_capable = self
                        .windows
                        .iter()
                        .find(|w| w.id == focused)
                        .map(|w| w.fullscreen_capable)
                        .unwrap_or(false);
                    if is_capable {
                        if let Some(resp) = self.enter_fullscreen(focused, false) {
                            // resp = [RESP_FULLSCREEN_ENTERED, win_id, (sw<<16)|sh, stride, fb_ptr]
                            self.push_event(
                                focused,
                                [
                                    EVENT_FULLSCREEN_ENTER,
                                    resp[2], // (sw<<16)|sh
                                    resp[3], // stride
                                    resp[4], // fb_ptr (direct FB if granted, 0 otherwise)
                                    0,
                                ],
                            );
                        }
                    }
                }
                return;
            }

            // Alt+F4: Close focused window
            if alt && key_code == KEY_F4 {
                if let Some(fs_win) = self.fullscreen_window {
                    // In fullscreen: exit fullscreen first, then close
                    self.push_event(fs_win, [EVENT_FULLSCREEN_EXIT, 0, 0, 0, 0]);
                    self.exit_fullscreen();
                    self.push_event(fs_win, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                } else if let Some(win_id) = self.focused_window {
                    self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                }
                return;
            }

            // Alt+Tab / Alt+Shift+Tab: Cycle windows (MRU order)
            if alt && key_code == KEY_TAB {
                if self.shortcut_overlay_visible {
                    self.close_shortcut_overlay();
                }
                let shift = mods & 1 != 0;
                self.cycle_window_focus(shift);
                return;
            }

            // Alt+R: Launch app runner
            if alt && !ctrl && key_code == 0x13 {
                anyos_std::process::spawn("/System/Runner.app", "");
                return;
            }

            // Ctrl+F1..F12: Focus window assigned to that shortcut slot
            if ctrl && !alt {
                let fkey_table: [u32; 12] = [
                    KEY_F1, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9,
                    KEY_F10, KEY_F11, KEY_F12,
                ];
                for (slot, &fk) in fkey_table.iter().enumerate() {
                    if key_code == fk {
                        if self.shortcut_overlay_visible {
                            self.close_shortcut_overlay();
                        }
                        let win_id = self.fkey_slots[slot];
                        if win_id != 0 {
                            // Un-minimize if needed
                            if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                                if self.windows[idx].x < -9000 {
                                    if let Some((sx, sy, _sw, _sh)) =
                                        self.windows[idx].saved_bounds.take()
                                    {
                                        self.windows[idx].x = sx;
                                        self.windows[idx].y = sy;
                                        let layer_id = self.windows[idx].layer_id;
                                        self.compositor.move_layer(layer_id, sx, sy);
                                    }
                                }
                            }
                            self.focus_window(win_id);
                        }
                        return;
                    }
                }
            }

            // Ctrl+Escape: Toggle shortcut overlay
            if ctrl && key_code == KEY_ESCAPE {
                anyos_std::println!(
                    "[compositor] Ctrl+Esc: toggle shortcut overlay (visible={})",
                    self.shortcut_overlay_visible
                );
                self.toggle_shortcut_overlay();
                return;
            }

            // Keyboard navigation in shortcut overlay
            if self.shortcut_overlay_visible {
                match key_code {
                    KEY_TAB | KEY_RIGHT => {
                        let shift = mods & 1 != 0;
                        if shift {
                            self.shortcut_overlay_select_prev();
                        } else {
                            self.shortcut_overlay_select_next();
                        }
                        return;
                    }
                    KEY_LEFT => {
                        self.shortcut_overlay_select_prev();
                        return;
                    }
                    KEY_DOWN => {
                        // Move down one row (4 columns)
                        let sel = self.shortcut_overlay_selection;
                        let next = sel + 4;
                        if next < 12 {
                            self.shortcut_overlay_selection = next;
                            self.render_shortcut_overlay();
                        }
                        return;
                    }
                    KEY_UP => {
                        // Move up one row (4 columns)
                        let sel = self.shortcut_overlay_selection;
                        let next = sel - 4;
                        if next >= 0 {
                            self.shortcut_overlay_selection = next;
                            self.render_shortcut_overlay();
                        }
                        return;
                    }
                    // 'X' key (scancode 0x2D) — close selected window
                    0x2D => {
                        let sel = self.shortcut_overlay_selection;
                        if sel >= 0 && sel < 12 {
                            let win_id = self.fkey_slots[sel as usize];
                            if win_id != 0 {
                                self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                            }
                        }
                        return;
                    }
                    KEY_ENTER => {
                        let sel = self.shortcut_overlay_selection;
                        if sel >= 0 && sel < 12 {
                            let win_id = self.fkey_slots[sel as usize];
                            self.close_shortcut_overlay();
                            if win_id != 0 {
                                // Un-minimize if needed
                                if let Some(idx) = self.windows.iter().position(|w| w.id == win_id)
                                {
                                    if self.windows[idx].x < -9000 {
                                        if let Some((sx, sy, _sw, _sh)) =
                                            self.windows[idx].saved_bounds.take()
                                        {
                                            self.windows[idx].x = sx;
                                            self.windows[idx].y = sy;
                                            let layer_id = self.windows[idx].layer_id;
                                            self.compositor.move_layer(layer_id, sx, sy);
                                        }
                                    }
                                }
                                self.focus_window(win_id);
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }

            // Escape: Close shortcut overlay, exit fullscreen, close open menus
            if key_code == KEY_ESCAPE {
                if self.shortcut_overlay_visible {
                    self.close_shortcut_overlay();
                    return;
                }
                if let Some(fs_win) = self.fullscreen_window {
                    self.push_event(fs_win, [EVENT_FULLSCREEN_EXIT, 0, 0, 0, 0]);
                    self.exit_fullscreen();
                    return;
                }
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0,
                        0,
                        self.screen_width,
                        menubar_height() + 1,
                    ));
                    return;
                }
                // If the dock has focus, release focus on Escape
                if let Some(fid) = self.focused_window {
                    let is_dock = self
                        .windows
                        .iter()
                        .find(|w| w.id == fid)
                        .map(|w| w.is_always_on_top() && w.is_borderless() && w.title == "Dock")
                        .unwrap_or(false);
                    if is_dock {
                        // Send Escape to dock first (so it exits keyboard mode)
                        self.push_event(fid, [EVENT_KEY_DOWN, key_code, chr, mods, 0]);
                        // Then defocus the dock
                        if let Some(idx) = self.windows.iter().position(|w| w.id == fid) {
                            self.windows[idx].focused = false;
                            self.render_titlebar(fid);
                            self.push_event(fid, [EVENT_FOCUS_LOST, 0, 0, 0, 0]);
                        }
                        self.focused_window = None;
                        self.compositor.set_focused_layer(None);
                        self.emit_focus_changed(0, 0);
                        return;
                    }
                }
                // Forward Escape to focused app (for dialog/popup dismiss)
            }

            // F10: Activate menubar keyboard navigation
            if key_code == KEY_F10 && !alt && !ctrl {
                self.activate_menubar_keyboard();
                return;
            }

            // Keyboard navigation for open dropdown menus
            if self.menu_bar.is_dropdown_open() {
                match key_code {
                    KEY_DOWN => {
                        self.menu_navigate_down();
                        return;
                    }
                    KEY_UP => {
                        self.menu_navigate_up();
                        return;
                    }
                    KEY_LEFT => {
                        self.menu_navigate_left();
                        return;
                    }
                    KEY_RIGHT => {
                        self.menu_navigate_right();
                        return;
                    }
                    KEY_ENTER => {
                        self.menu_activate_item();
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Volume keys: intercept globally, don't forward to apps
        if down {
            match key_code {
                KEY_VOLUME_UP => {
                    let vol = anyos_std::audio::audio_get_volume();
                    let new_vol = (vol as u32 + 5).min(100) as u8;
                    anyos_std::audio::audio_set_volume(new_vol);
                    let (sw, sh, mb) =
                        (self.screen_width, self.screen_height, self.menubar_layer_id);
                    self.volume_hud
                        .show(&mut self.compositor, sw, sh, new_vol, false, mb);
                    return;
                }
                KEY_VOLUME_DOWN => {
                    let vol = anyos_std::audio::audio_get_volume();
                    let new_vol = vol.saturating_sub(5);
                    anyos_std::audio::audio_set_volume(new_vol);
                    let (sw, sh, mb) =
                        (self.screen_width, self.screen_height, self.menubar_layer_id);
                    self.volume_hud
                        .show(&mut self.compositor, sw, sh, new_vol, false, mb);
                    return;
                }
                KEY_VOLUME_MUTE => {
                    let vol = anyos_std::audio::audio_get_volume();
                    if vol > 0 {
                        self.volume_hud.saved_volume = vol;
                        anyos_std::audio::audio_set_volume(0);
                        let (sw, sh, mb) =
                            (self.screen_width, self.screen_height, self.menubar_layer_id);
                        self.volume_hud
                            .show(&mut self.compositor, sw, sh, 0, true, mb);
                    } else {
                        let restore = if self.volume_hud.saved_volume > 0 {
                            self.volume_hud.saved_volume
                        } else {
                            50
                        };
                        anyos_std::audio::audio_set_volume(restore);
                        let (sw, sh, mb) =
                            (self.screen_width, self.screen_height, self.menubar_layer_id);
                        self.volume_hud
                            .show(&mut self.compositor, sw, sh, restore, false, mb);
                    }
                    return;
                }
                _ => {}
            }
        }

        // ── Configurable shortcuts (from [shortcuts] in compositor.conf) ───
        if down {
            let shift = mods & 1 != 0;
            let super_held = self.super_held;
            // Build modifier mask matching the config format:
            // bit 0 = Shift, bit 1 = Ctrl, bit 2 = Alt, bit 3 = Super
            let effective_mods: u8 = (shift as u8)
                | ((ctrl as u8) << 1)
                | ((alt as u8) << 2)
                | ((super_held as u8) << 3);

            for i in 0..self.shortcuts.len() {
                if self.shortcuts[i].key_code == key_code
                    && self.shortcuts[i].modifiers == effective_mods
                {
                    match &self.shortcuts[i].action {
                        crate::config::ShortcutAction::Launch(path) => {
                            anyos_std::process::spawn(path, "");
                        }
                        crate::config::ShortcutAction::ShowDesktop => {
                            self.toggle_show_desktop();
                        }
                        crate::config::ShortcutAction::TileWindows => {
                            self.tile_all_windows();
                        }
                        crate::config::ShortcutAction::LockScreen => {
                            // TODO: implement lock screen
                        }
                    }
                    return;
                }
            }
        }

        // Focus the dock via keyboard:
        // - Tab when no app window is focused (empty desktop or dock already focused)
        // - Ctrl+D always (even with an app focused)
        if down {
            let focus_dock = if key_code == KEY_TAB && !alt {
                // Tab: only if no normal app window is focused
                match self.focused_window {
                    None => true,
                    Some(fid) => self
                        .windows
                        .iter()
                        .find(|w| w.id == fid)
                        .map(|w| w.is_borderless() && w.is_always_on_top())
                        .unwrap_or(true),
                }
            } else if ctrl && !alt && key_code == 0x20 {
                // Ctrl+D (scancode 0x20 = 'D' on QWERTY)
                true
            } else {
                false
            };
            if focus_dock {
                if let Some(dock_id) = self.find_dock_window() {
                    if self.focused_window != Some(dock_id) {
                        self.focus_window(dock_id);
                    }
                    self.push_event(dock_id, [EVENT_KEY_DOWN, KEY_TAB, chr, mods, 0]);
                    return;
                }
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
                    self.menu_bar.open_system_menu(&mut self.compositor);
                }
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
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
                    menubar_height() + 1,
                ));
            }
            MenuBarHit::StatusIcon { owner_tid, icon_id } => {
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
                        menubar_height() + 1,
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
            crate::menu::types::SYS_MENU_NOTIFICATIONS => {
                anyos_std::process::spawn("/Applications/Notifications.app", "");
            }
            crate::menu::types::SYS_MENU_TILE_WINDOWS => {
                self.tile_all_windows();
            }
            crate::menu::SYS_MENU_SETTINGS | crate::menu::SYS_MENU_SLEEP => {
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
        let target_sub = self
            .app_subs
            .iter()
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

        // We track the previous VNC button mask in a dedicated field so that
        // we can synthesise proper press/release pairs. handle_mouse_button
        // expects the *full* new button state (not just the changed bit),
        // so we forward the whole 0x07 mask once per transition direction.
        let prev = self.vnc_buttons;
        let prim = (buttons & 0x07) as u32;
        let prim_prev = (prev & 0x07) as u32;
        if prim != prim_prev {
            let new_presses = prim & !prim_prev;
            let new_releases = prim_prev & !prim;
            if new_presses != 0 {
                self.handle_mouse_button(prim, true);
            }
            // Issue the release in addition to the press so a single inject
            // that both presses *and* releases bits in the same frame still
            // fires a MOUSE_UP for the released bits. handle_mouse_button
            // updates self.mouse_buttons internally, so the second call sees
            // the correct previous_buttons state.
            if new_releases != 0 {
                self.handle_mouse_button(prim, false);
            }
        }

        // Wheel ticks: bit 3 = up, bit 4 = down (RFB / SPICE-shifted
        // convention shared by vncd and vdagent). Each rising edge is one
        // scroll notch — the same way RFB and SPICE encode mouse-wheel
        // events as transient button "taps".
        let wheel_up_edge = (buttons & 0x08) != 0 && (prev & 0x08) == 0;
        let wheel_down_edge = (buttons & 0x10) != 0 && (prev & 0x10) == 0;
        if wheel_up_edge {
            self.handle_scroll(-1);
        }
        if wheel_down_edge {
            self.handle_scroll(1);
        }

        self.vnc_buttons = buttons;
    }

    // ── Keyboard Accessibility ──────────────────────────────────────────

    /// Find the dock window (always-on-top borderless window named "Dock").
    fn find_dock_window(&self) -> Option<u32> {
        self.windows
            .iter()
            .find(|w| {
                w.is_always_on_top() && w.is_borderless() && w.owner_tid != 0 && w.title == "Dock"
            })
            .map(|w| w.id)
    }

    /// Cycle window focus in MRU order.
    /// The `windows` vec is MRU-ordered: last element = currently focused.
    /// Alt+Tab focuses the second-to-last visible window.
    /// Alt+Shift+Tab goes in reverse (forward in the vec).
    fn cycle_window_focus(&mut self, reverse: bool) {
        // Collect visible (not minimized/hidden) windows with owner_tid != 0 (IPC windows only)
        let visible: alloc::vec::Vec<u32> = self
            .windows
            .iter()
            .filter(|w| w.owner_tid != 0 && w.x >= -9000)
            .map(|w| w.id)
            .collect();

        if visible.is_empty() {
            return;
        }

        if visible.len() == 1 {
            // Only one window (e.g. dock) — focus it
            self.focus_window(visible[0]);
            return;
        }

        let target = if reverse {
            // Alt+Shift+Tab: go forward (first visible window in MRU order)
            visible[0]
        } else {
            // Alt+Tab: second-to-last (previous window in MRU order)
            visible[visible.len() - 2]
        };

        self.focus_window(target);
    }

    /// Activate menubar keyboard navigation: open the first app menu.
    fn activate_menubar_keyboard(&mut self) {
        if self.menu_bar.is_dropdown_open() {
            self.menu_bar
                .close_dropdown_with_compositor(&mut self.compositor);
            self.draw_menubar();
            self.compositor
                .add_damage(Rect::new(0, 0, self.screen_width, menubar_height() + 1));
            return;
        }
        // Open the first menu (index 0 = app name menu)
        if !self.menu_bar.title_layouts.is_empty() {
            let owner_wid = self.focused_window.unwrap_or(0);
            self.menu_bar.open_menu(0, owner_wid, &mut self.compositor);
            // Set hover to first non-separator item
            if let Some(ref mut dd) = self.menu_bar.open_dropdown {
                dd.hover_idx = Some(0);
            }
            self.menu_bar.render_dropdown(&mut self.compositor);
            self.draw_menubar();
            self.compositor
                .add_damage(Rect::new(0, 0, self.screen_width, menubar_height() + 1));
        }
    }

    /// Navigate down in the open dropdown menu.
    fn menu_navigate_down(&mut self) {
        let item_count = self.menu_dropdown_item_count();
        if item_count == 0 {
            return;
        }
        let cur = self
            .menu_bar
            .open_dropdown
            .as_ref()
            .and_then(|dd| dd.hover_idx)
            .unwrap_or(item_count.wrapping_sub(1));
        let mut next = (cur + 1) % item_count;
        for _ in 0..item_count {
            if !self.menu_item_is_separator(next) {
                break;
            }
            next = (next + 1) % item_count;
        }
        if let Some(ref mut dd) = self.menu_bar.open_dropdown {
            dd.hover_idx = Some(next);
        }
        self.menu_bar.render_dropdown(&mut self.compositor);
    }

    /// Navigate up in the open dropdown menu.
    fn menu_navigate_up(&mut self) {
        let item_count = self.menu_dropdown_item_count();
        if item_count == 0 {
            return;
        }
        let cur = self
            .menu_bar
            .open_dropdown
            .as_ref()
            .and_then(|dd| dd.hover_idx)
            .unwrap_or(0);
        let mut prev = if cur == 0 { item_count - 1 } else { cur - 1 };
        for _ in 0..item_count {
            if !self.menu_item_is_separator(prev) {
                break;
            }
            prev = if prev == 0 { item_count - 1 } else { prev - 1 };
        }
        if let Some(ref mut dd) = self.menu_bar.open_dropdown {
            dd.hover_idx = Some(prev);
        }
        self.menu_bar.render_dropdown(&mut self.compositor);
    }

    /// Navigate left to previous menu.
    fn menu_navigate_left(&mut self) {
        let menu_idx = match self.menu_bar.open_dropdown {
            Some(ref dd) => dd.menu_idx,
            None => return,
        };
        let menu_count = self.menu_bar.title_layouts.len();
        if menu_count == 0 {
            return;
        }
        let prev = if menu_idx == 0 {
            menu_count - 1
        } else {
            menu_idx - 1
        };

        let owner_wid = self.focused_window.unwrap_or(0);
        self.menu_bar
            .close_dropdown_with_compositor(&mut self.compositor);
        self.menu_bar
            .open_menu(prev, owner_wid, &mut self.compositor);
        if let Some(ref mut dd) = self.menu_bar.open_dropdown {
            dd.hover_idx = Some(0);
        }
        self.menu_bar.render_dropdown(&mut self.compositor);
        self.draw_menubar();
        self.compositor
            .add_damage(Rect::new(0, 0, self.screen_width, menubar_height() + 1));
    }

    /// Navigate right to next menu.
    fn menu_navigate_right(&mut self) {
        let menu_idx = match self.menu_bar.open_dropdown {
            Some(ref dd) => dd.menu_idx,
            None => return,
        };
        let menu_count = self.menu_bar.title_layouts.len();
        if menu_count == 0 {
            return;
        }
        let next = (menu_idx + 1) % menu_count;

        let owner_wid = self.focused_window.unwrap_or(0);
        self.menu_bar
            .close_dropdown_with_compositor(&mut self.compositor);
        self.menu_bar
            .open_menu(next, owner_wid, &mut self.compositor);
        if let Some(ref mut dd) = self.menu_bar.open_dropdown {
            dd.hover_idx = Some(0);
        }
        self.menu_bar.render_dropdown(&mut self.compositor);
        self.draw_menubar();
        self.compositor
            .add_damage(Rect::new(0, 0, self.screen_width, menubar_height() + 1));
    }

    /// Activate the currently hovered menu item.
    fn menu_activate_item(&mut self) {
        let (hover_idx, menu_idx, is_system) = match self.menu_bar.open_dropdown {
            Some(ref dd) => (dd.hover_idx, dd.menu_idx, self.menu_bar.system_menu_open),
            None => return,
        };
        let hover_idx = match hover_idx {
            Some(i) => i,
            None => return,
        };

        if is_system {
            if let Some(item_id) = self.menu_bar.get_system_menu_item_id(hover_idx) {
                self.menu_bar
                    .close_dropdown_with_compositor(&mut self.compositor);
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    menubar_height() + 1,
                ));
                self.handle_system_menu_action(item_id);
            }
        } else if let Some(item_id) = self.menu_bar.get_menu_item_id(menu_idx, hover_idx) {
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
                        let next = self
                            .windows
                            .iter()
                            .rev()
                            .find(|w| w.id != win_id && w.x >= 0)
                            .map(|w| w.id);
                        if let Some(nid) = next {
                            self.focus_window(nid);
                        }
                    }
                    _ => {
                        self.push_event(win_id, [EVENT_MENU_ITEM, menu_idx as u32, item_id, 0, 0]);
                    }
                }
            }
            self.menu_bar
                .close_dropdown_with_compositor(&mut self.compositor);
            self.draw_menubar();
            self.compositor
                .add_damage(Rect::new(0, 0, self.screen_width, menubar_height() + 1));
        }
    }

    /// Get the number of items in the currently open dropdown.
    fn menu_dropdown_item_count(&self) -> usize {
        if self.menu_bar.system_menu_open {
            return 11; // system menu has 11 items (incl. separators)
        }
        let dd = match &self.menu_bar.open_dropdown {
            Some(d) => d,
            None => return 0,
        };
        let def = match self.menu_bar.active_def() {
            Some(d) => d,
            None => return 0,
        };
        def.menus
            .get(dd.menu_idx)
            .map(|m| m.items.len())
            .unwrap_or(0)
    }

    /// Check if a menu item at the given index is a separator.
    fn menu_item_is_separator(&self, idx: usize) -> bool {
        if self.menu_bar.system_menu_open {
            // System menu separator positions: 1, 5, 7
            return idx == 1 || idx == 5 || idx == 7;
        }
        let dd = match &self.menu_bar.open_dropdown {
            Some(d) => d,
            None => return false,
        };
        let def = match self.menu_bar.active_def() {
            Some(d) => d,
            None => return false,
        };
        def.menus
            .get(dd.menu_idx)
            .and_then(|m| m.items.get(idx))
            .map(|item| item.is_separator())
            .unwrap_or(false)
    }
}
