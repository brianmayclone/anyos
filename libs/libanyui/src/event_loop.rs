//! Event loop — polls compositor events via DLL, dispatches via virtual methods, renders.
//!
//! Window management uses libcompositor.dlib (user-space compositor), NOT kernel syscalls.
//! Events are received via the compositor's IPC protocol (EVT_* = 0x3001-0x300A).
//! Rendering writes directly to the window's SHM surface, then calls present().
//!
//! # Event dispatch flow:
//!
//! 1. **MOUSE_MOVE**: Hit-test → update hovered control (fire MouseEnter/MouseLeave).
//!    If a control is pressed, dispatch handle_mouse_move for drag.
//! 2. **MOUSE_DOWN**: Hit-test → set pressed control, update focus (fire Focus/Blur),
//!    dispatch handle_mouse_down on the hit control.
//! 3. **MOUSE_UP**: If pressed control is still under cursor → dispatch handle_mouse_up,
//!    then handle_click. Check for double-click.
//! 4. **KEY_DOWN**: Dispatch to focused control via handle_key_down.
//! 5. **SCROLL**: Dispatch to control under cursor via handle_scroll.
//! 6. **WINDOW_CLOSE**: Fire close callback, queue window for removal.
//! 7. **WINDOW_RESIZE**: Update window size, fire resize callback.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::compositor;
use crate::control::{self, ControlId, ControlKind, Control, Callback};

/// Double-click threshold in frames (~400ms at 30fps).
const DOUBLE_CLICK_FRAMES: u64 = 12;

/// A pending callback to fire after all event processing.
struct PendingCallback {
    id: ControlId,
    event_type: u32,
    cb: Callback,
    userdata: u64,
}

/// Run the event loop. Blocks until all windows are closed or quit is requested.
/// Adaptive frame pacing: polls quickly when waiting for VSync (FRAME_ACK),
/// sleeps longer when idle. Matches Windows DWM / macOS CVDisplayLink model.
pub fn run() {
    loop {
        let t0 = crate::syscall::uptime_ms();
        if run_once() == 0 {
            break;
        }
        let elapsed = crate::syscall::uptime_ms().wrapping_sub(t0);
        // Adaptive sleep: fast polling when waiting for VSync ACK, idle sleep otherwise
        let st = crate::state();
        let any_pending = st.comp_windows.iter().any(|cw| cw.frame_presented);
        let target: u32 = if any_pending { 8 } else { 16 };
        if elapsed < target {
            crate::syscall::sleep(target - elapsed);
        }
    }
}

/// Process one frame of events + rendering. Returns 1 if windows remain, 0 if done.
pub fn run_once() -> u32 {
    let mut pending_cbs: Vec<PendingCallback> = Vec::new();
    let mut windows_to_close: Vec<ControlId> = Vec::new();

    let st = crate::state();
    if st.quit_requested || st.windows.is_empty() {
        return 0;
    }

    // ── Phase 0: Drain marshal queue (cross-thread commands) ───────
    crate::marshal::drain(st);

    // ── Phase 0.5: Fire elapsed timers ──────────────────────────────
    {
        let now = crate::syscall::uptime_ms();
        for slot in &mut st.timers.slots {
            if now.wrapping_sub(slot.last_fired_ms) >= slot.interval_ms {
                pending_cbs.push(PendingCallback {
                    id: slot.id,
                    event_type: 0,
                    cb: slot.callback,
                    userdata: slot.userdata,
                });
                slot.last_fired_ms = now;
            }
        }
    }

    // ── Phase 1: Poll events from all windows ──────────────────────
    let win_count = st.windows.len();
    for wi in 0..win_count {
        if wi >= st.windows.len() { break; }
        let win_id = st.windows[wi];
        let comp_window_id = st.comp_windows[wi].window_id;

        let mut ev = [0u32; 5];
        // Poll events via compositor DLL
        // Buffer layout: [event_type, window_id, arg1, arg2, arg3]
        while compositor::poll_event(st.channel_id, st.sub_id, comp_window_id, &mut ev) {
            match ev[0] {
                compositor::EVT_WINDOW_CLOSE => {
                    fire_event_callback(&st.controls, win_id, control::EVENT_CLOSE, &mut pending_cbs);
                    windows_to_close.push(win_id);
                }

                compositor::EVT_MOUSE_MOVE => {
                    // arg1=local_x, arg2=local_y
                    let mx = ev[2] as i32;
                    let my = ev[3] as i32;

                    // Update hover tracking (MouseEnter / MouseLeave)
                    let new_hover = control::hit_test_any(&st.controls, win_id, mx, my, 0, 0);
                    let old_hover = st.hovered;

                    if new_hover != old_hover {
                        if let Some(old_id) = old_hover {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_mouse_leave();
                                st.controls[idx].base_mut().dirty = true;
                                fire_event_callback(&st.controls, old_id, control::EVENT_MOUSE_LEAVE, &mut pending_cbs);
                            }
                        }
                        if let Some(new_id) = new_hover {
                            if let Some(idx) = control::find_idx(&st.controls, new_id) {
                                st.controls[idx].handle_mouse_enter();
                                st.controls[idx].base_mut().dirty = true;
                                fire_event_callback(&st.controls, new_id, control::EVENT_MOUSE_ENTER, &mut pending_cbs);
                            }
                        }
                        st.hovered = new_hover;
                    }

                    // If a control is pressed, dispatch mouse_move for drag
                    if let Some(pressed_id) = st.pressed {
                        if let Some(idx) = control::find_idx(&st.controls, pressed_id) {
                            let (ax, ay) = control::abs_position(&st.controls, pressed_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;
                            let resp = st.controls[idx].handle_mouse_move(local_x, local_y);
                            if resp.consumed {
                                fire_event_callback(&st.controls, pressed_id, control::EVENT_MOUSE_MOVE, &mut pending_cbs);
                                if resp.fire_change {
                                    fire_event_callback(&st.controls, pressed_id, control::EVENT_CHANGE, &mut pending_cbs);
                                }
                                if resp.fire_click {
                                    fire_event_callback(&st.controls, pressed_id, control::EVENT_CLICK, &mut pending_cbs);
                                }
                            }
                        }
                    }
                }

                compositor::EVT_MOUSE_DOWN => {
                    // arg1=local_x, arg2=local_y, arg3=buttons
                    let mx = ev[2] as i32;
                    let my = ev[3] as i32;
                    let button = ev[4];

                    let hit_id = control::hit_test(&st.controls, win_id, mx, my, 0, 0);

                    // Update focus
                    if let Some(new_focus) = hit_id {
                        let old_focus = st.focused;
                        if old_focus != Some(new_focus) {
                            if let Some(old_id) = old_focus {
                                if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                    st.controls[idx].handle_blur();
                                    fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, &mut pending_cbs);
                                }
                            }
                            if let Some(idx) = control::find_idx(&st.controls, new_focus) {
                                if st.controls[idx].accepts_focus() {
                                    st.controls[idx].handle_focus();
                                    st.focused = Some(new_focus);
                                    fire_event_callback(&st.controls, new_focus, control::EVENT_FOCUS, &mut pending_cbs);
                                } else {
                                    st.focused = None;
                                }
                            }
                        }
                    } else {
                        if let Some(old_id) = st.focused {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_blur();
                                fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, &mut pending_cbs);
                            }
                        }
                        st.focused = None;
                    }

                    st.pressed = hit_id;
                    st.pressed_button = button;

                    if let Some(target_id) = hit_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;
                            let resp = st.controls[idx].handle_mouse_down(local_x, local_y, button);
                            st.controls[idx].base_mut().dirty = true;

                            fire_event_callback(&st.controls, target_id, control::EVENT_MOUSE_DOWN, &mut pending_cbs);

                            if resp.fire_change {
                                fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }
                            if resp.fire_click {
                                fire_event_callback(&st.controls, target_id, control::EVENT_CLICK, &mut pending_cbs);
                            }
                        }
                    }
                }

                compositor::EVT_MOUSE_UP => {
                    // arg1=local_x, arg2=local_y
                    let mx = ev[2] as i32;
                    let my = ev[3] as i32;
                    let button = ev[4];

                    let pressed_id = st.pressed.take();

                    if let Some(target_id) = pressed_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;

                            let resp = st.controls[idx].handle_mouse_up(local_x, local_y, button);
                            st.controls[idx].base_mut().dirty = true;
                            fire_event_callback(&st.controls, target_id, control::EVENT_MOUSE_UP, &mut pending_cbs);

                            if resp.fire_change {
                                fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }

                            // Check if mouse is still over the pressed control → Click
                            let still_over = is_point_in_control(&st.controls, target_id, mx, my);

                            if still_over {
                                if st.pressed_button & 0x02 != 0 {
                                    // Right-click → fire EVENT_CONTEXT_MENU
                                    fire_event_callback(&st.controls, target_id, control::EVENT_CONTEXT_MENU, &mut pending_cbs);

                                    // If control has a context menu, show it at cursor position
                                    if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                        if let Some(menu_id) = st.controls[idx2].base().context_menu {
                                            if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                                st.controls[mi].set_position(mx, my);
                                                st.controls[mi].base_mut().visible = true;
                                                st.controls[mi].base_mut().dirty = true;
                                                // Focus the menu so handle_blur hides it on outside click
                                                if let Some(old_fid) = st.focused {
                                                    if let Some(oi) = control::find_idx(&st.controls, old_fid) {
                                                        st.controls[oi].handle_blur();
                                                    }
                                                }
                                                st.focused = Some(menu_id);
                                            }
                                        }
                                    }
                                } else {
                                    // Left-click → normal click + double-click handling
                                    if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                        let click_resp = st.controls[idx2].handle_click(local_x, local_y, button);

                                        fire_event_callback(&st.controls, target_id, control::EVENT_CLICK, &mut pending_cbs);

                                        if click_resp.fire_change {
                                            fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                                        }

                                        // Double-click detection
                                        let tick = frame_tick();
                                        if st.last_click_id == Some(target_id)
                                            && tick.wrapping_sub(st.last_click_tick) <= DOUBLE_CLICK_FRAMES
                                        {
                                            if let Some(idx3) = control::find_idx(&st.controls, target_id) {
                                                let dc_resp = st.controls[idx3].handle_double_click(local_x, local_y, button);
                                                fire_event_callback(&st.controls, target_id, control::EVENT_DOUBLE_CLICK, &mut pending_cbs);
                                                if dc_resp.fire_change {
                                                    fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                                                }
                                                if dc_resp.fire_submit {
                                                    fire_event_callback(&st.controls, target_id, control::EVENT_SUBMIT, &mut pending_cbs);
                                                }
                                            }
                                            st.last_click_id = None;
                                            st.last_click_tick = 0;
                                        } else {
                                            st.last_click_id = Some(target_id);
                                            st.last_click_tick = tick;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                compositor::EVT_KEY_DOWN => {
                    // arg1=scancode, arg2=char_code, arg3=modifiers
                    let keycode = ev[2];
                    let char_code = ev[3];

                    let mut handled = false;

                    if let Some(focus_id) = st.focused {
                        if let Some(idx) = control::find_idx(&st.controls, focus_id) {
                            let resp = st.controls[idx].handle_key_down(keycode, char_code);
                            st.controls[idx].base_mut().dirty = true;

                            if resp.consumed {
                                handled = true;
                                fire_event_callback(&st.controls, focus_id, control::EVENT_KEY, &mut pending_cbs);
                            }
                            if resp.fire_change {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }
                            if resp.fire_click {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_CLICK, &mut pending_cbs);
                            }
                            if resp.fire_submit {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_SUBMIT, &mut pending_cbs);
                            }
                        }
                    }

                    if !handled {
                        // Tab: cycle focus to next focusable control
                        if keycode == control::KEY_TAB {
                            cycle_focus(st, win_id, &mut pending_cbs);
                        } else {
                            // Bubble unhandled key events to the window
                            fire_event_callback(&st.controls, win_id, control::EVENT_KEY, &mut pending_cbs);
                        }
                    }
                }

                compositor::EVT_MOUSE_SCROLL => {
                    // arg1=dz (signed), arg2=0, arg3=0
                    let dz = ev[2] as i32;

                    // Dispatch to hovered control, bubbling up to ScrollView if needed
                    if let Some(target_id) = st.hovered {
                        let mut cur = target_id;
                        loop {
                            if let Some(idx) = control::find_idx(&st.controls, cur) {
                                let resp = st.controls[idx].handle_scroll(dz);
                                if resp.consumed {
                                    st.controls[idx].base_mut().dirty = true;
                                    fire_event_callback(&st.controls, cur, control::EVENT_SCROLL, &mut pending_cbs);
                                    if resp.fire_change {
                                        fire_event_callback(&st.controls, cur, control::EVENT_CHANGE, &mut pending_cbs);
                                    }
                                    break;
                                }
                                // Bubble up to parent
                                let parent = st.controls[idx].parent_id();
                                if parent == 0 || parent == cur { break; }
                                cur = parent;
                            } else {
                                break;
                            }
                        }
                    }
                }

                compositor::EVT_RESIZE => {
                    // arg1=new_w, arg2=new_h
                    let new_w = ev[2];
                    let new_h = ev[3];
                    // Resize the SHM buffer to fit the new dimensions
                    if wi < st.comp_windows.len() {
                        let cw = &mut st.comp_windows[wi];
                        if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
                            st.channel_id,
                            cw.window_id,
                            cw.shm_id,
                            new_w,
                            new_h,
                        ) {
                            cw.shm_id = new_shm_id;
                            cw.surface = new_surface;
                        }
                        cw.width = new_w;
                        cw.height = new_h;
                    }
                    if let Some(idx) = control::find_idx(&st.controls, win_id) {
                        st.controls[idx].set_size(new_w, new_h);
                        fire_event_callback(&st.controls, win_id, control::EVENT_RESIZE, &mut pending_cbs);
                    }
                }

                compositor::EVT_FRAME_ACK => {
                    // VSync callback: compositor has composited our frame to screen.
                    // Clear back-pressure so we can present the next frame.
                    if wi < st.comp_windows.len() {
                        st.comp_windows[wi].frame_presented = false;
                    }
                }

                _ => {}
            }
            ev = [0u32; 5];
        }
    }

    // ── Phase 2: Close windows ──────────────────────────────────────
    let channel_id = st.channel_id;
    for win_id in &windows_to_close {
        if let Some(wi) = st.windows.iter().position(|&w| w == *win_id) {
            let cw = &st.comp_windows[wi];
            compositor::destroy_window(channel_id, cw.window_id, cw.shm_id);
            st.comp_windows.remove(wi);
            st.windows.remove(wi);
        }
        clear_tracking_for(st, *win_id);
        remove_subtree(&mut st.controls, *win_id);
    }

    // ── Phase 3: Invoke callbacks (no borrows held) ────────────────
    for pcb in pending_cbs {
        (pcb.cb)(pcb.id, pcb.event_type, pcb.userdata);
    }

    // Re-acquire state (callbacks may have modified it)
    let st = crate::state();
    if st.quit_requested || st.windows.is_empty() {
        return 0;
    }

    // ── Phase 3.5: Layout ───────────────────────────────────────────
    for wi in 0..st.windows.len() {
        let win_id = st.windows[wi];
        crate::layout::perform_layout(&mut st.controls, win_id);
    }

    // ── Phase 3.6: Update scroll bounds ─────────────────────────────
    crate::controls::scroll_view::update_scroll_bounds(&mut st.controls);

    // ── Phase 3.7: Compute per-window dirty flags (O(n) flat scan) ──
    // Replaces the O(n²) recursive any_dirty() tree walk with a single
    // linear pass over all controls.
    for cw in st.comp_windows.iter_mut() {
        cw.dirty = false;
    }
    'ctrl_scan: for ctrl in st.controls.iter() {
        if !ctrl.base().dirty { continue; }
        // Walk up parent chain to find which window this control belongs to
        let mut cur_id = ctrl.id();
        let mut cur_parent = ctrl.base().parent;
        for _ in 0..32 {
            // Check if cur_id is a window
            if let Some(wi) = st.windows.iter().position(|&w| w == cur_id) {
                st.comp_windows[wi].dirty = true;
                continue 'ctrl_scan;
            }
            if cur_parent == 0 || cur_parent == cur_id { continue 'ctrl_scan; }
            cur_id = cur_parent;
            cur_parent = st.controls.iter()
                .find(|c| c.id() == cur_id)
                .map(|c| c.base().parent)
                .unwrap_or(0);
        }
    }

    // ── Phase 4: Render dirty windows (with VSync back-pressure) ───
    let channel_id = st.channel_id;
    for wi in 0..st.windows.len() {
        let win_id = st.windows[wi];

        // Back-pressure: skip if previous frame hasn't been composited yet.
        // This prevents overwriting SHM while compositor is reading it.
        // Safety timeout after 64ms (~4 frames) to avoid hangs if ACK is lost.
        if st.comp_windows[wi].frame_presented {
            let now = crate::syscall::uptime_ms();
            if now.wrapping_sub(st.comp_windows[wi].last_present_ms) < 64 {
                continue;
            }
            st.comp_windows[wi].frame_presented = false;
        }

        // Skip rendering if no control in this window tree is dirty (O(1) check)
        if !st.comp_windows[wi].dirty {
            continue;
        }

        let surface_ptr = st.comp_windows[wi].surface;
        let sw = st.comp_windows[wi].width;
        let sh = st.comp_windows[wi].height;
        let comp_window_id = st.comp_windows[wi].window_id;
        let shm_id = st.comp_windows[wi].shm_id;

        let surf = crate::draw::Surface::new(surface_ptr, sw, sh);

        // No explicit background clear here — Window::render() paints its own
        // background as the first step of render_tree. Avoiding a separate clear
        // reduces the window where SHM contains a blank frame (present is async).

        // Render control tree (depth-first, parent before children)
        render_tree(&st.controls, win_id, &surf, 0, 0);

        // Clear dirty flags after rendering
        clear_dirty(&mut st.controls, win_id);

        // Present via compositor DLL + mark as pending VSync ACK
        compositor::present(channel_id, comp_window_id, shm_id);
        st.comp_windows[wi].frame_presented = true;
        st.comp_windows[wi].last_present_ms = crate::syscall::uptime_ms();
    }

    1
}

// ── Helper functions ────────────────────────────────────────────────

fn fire_event_callback(
    controls: &[Box<dyn Control>],
    id: ControlId,
    event_type: u32,
    pending: &mut Vec<PendingCallback>,
) {
    if let Some(idx) = control::find_idx(controls, id) {
        if let Some(slot) = controls[idx].get_event_callback(event_type) {
            pending.push(PendingCallback {
                id,
                event_type,
                cb: slot.cb,
                userdata: slot.userdata,
            });
        }
    }
}

/// Build a cascaded tab sort key for a control: (parent_tab_index, own_tab_index, insertion_order).
/// This ensures controls are grouped by parent tab_index first, then sorted within the group.
fn tab_sort_key(controls: &[Box<dyn control::Control>], id: ControlId, insertion_idx: usize) -> (u32, u32, usize) {
    let own = control::find_idx(controls, id)
        .map(|i| controls[i].base().tab_index)
        .unwrap_or(0);
    let parent_id = control::find_idx(controls, id)
        .map(|i| controls[i].parent_id())
        .unwrap_or(0);
    let parent_tab = control::find_idx(controls, parent_id)
        .map(|i| controls[i].base().tab_index)
        .unwrap_or(0);
    (parent_tab, own, insertion_idx)
}

/// Cycle keyboard focus to the next focusable control within the window.
/// Controls are ordered by cascaded tab_index (parent tab_index, own tab_index, insertion order).
fn cycle_focus(
    st: &mut crate::AnyuiState,
    win_id: ControlId,
    pending: &mut Vec<PendingCallback>,
) {
    // Collect all focusable controls that belong to this window (with insertion index for stable sort)
    let mut focusable: Vec<(ControlId, usize)> = Vec::new();
    for (ins_idx, c) in st.controls.iter().enumerate() {
        if !c.accepts_focus() || c.id() == win_id || !c.base().visible { continue; }
        // Check that this control belongs to the window
        let mut cur = c.parent_id();
        let belongs = loop {
            if cur == win_id { break true; }
            if cur == 0 { break false; }
            match control::find_idx(&st.controls, cur) {
                Some(idx) => {
                    // Skip controls whose parent is invisible
                    if !st.controls[idx].base().visible { break false; }
                    cur = st.controls[idx].parent_id();
                }
                None => break false,
            }
        };
        if belongs { focusable.push((c.id(), ins_idx)); }
    }

    if focusable.is_empty() { return; }

    // Sort by cascaded tab_index
    focusable.sort_by(|a, b| {
        let ka = tab_sort_key(&st.controls, a.0, a.1);
        let kb = tab_sort_key(&st.controls, b.0, b.1);
        ka.cmp(&kb)
    });

    let ids: Vec<ControlId> = focusable.iter().map(|f| f.0).collect();

    // Find current focused index
    let cur_idx = st.focused
        .and_then(|fid| ids.iter().position(|&id| id == fid))
        .unwrap_or(0);

    let next_idx = (cur_idx + 1) % ids.len();
    let next_id = ids[next_idx];

    // Blur old
    if let Some(old_id) = st.focused {
        if let Some(idx) = control::find_idx(&st.controls, old_id) {
            st.controls[idx].handle_blur();
            st.controls[idx].base_mut().dirty = true;
            fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, pending);
        }
    }

    // Focus new
    if let Some(idx) = control::find_idx(&st.controls, next_id) {
        st.controls[idx].handle_focus();
        st.controls[idx].base_mut().dirty = true;
        st.focused = Some(next_id);
        fire_event_callback(&st.controls, next_id, control::EVENT_FOCUS, pending);
    }
}

fn is_point_in_control(
    controls: &[Box<dyn Control>],
    id: ControlId,
    px: i32,
    py: i32,
) -> bool {
    let (ax, ay) = control::abs_position(controls, id);
    if let Some(idx) = control::find_idx(controls, id) {
        let (w, h) = controls[idx].size();
        px >= ax && py >= ay && px < ax + w as i32 && py < ay + h as i32
    } else {
        false
    }
}

static mut FRAME_COUNTER: u64 = 0;

fn frame_tick() -> u64 {
    unsafe {
        FRAME_COUNTER += 1;
        FRAME_COUNTER
    }
}

fn clear_tracking_for(st: &mut crate::AnyuiState, id: ControlId) {
    if st.focused == Some(id) { st.focused = None; }
    if st.pressed == Some(id) { st.pressed = None; }
    if st.hovered == Some(id) { st.hovered = None; }

    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let children: Vec<ControlId> = ctrl.children().to_vec();
        for &child in &children {
            clear_tracking_for(st, child);
        }
    }
}

// ── Dirty tracking ─────────────────────────────────────────────────

/// Clear dirty flags for all controls in the subtree rooted at `id`.
/// Uses a stack buffer instead of Vec::to_vec() to avoid heap allocation per node.
fn clear_dirty(controls: &mut [Box<dyn Control>], id: ControlId) {
    let idx = match control::find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };
    controls[idx].base_mut().dirty = false;
    let mut buf = [0u32; 64];
    let n = controls[idx].children().len().min(64);
    buf[..n].copy_from_slice(&controls[idx].children()[..n]);
    for i in 0..n {
        clear_dirty(controls, buf[i]);
    }
}

// ── Tree rendering ──────────────────────────────────────────────────

fn render_tree(
    controls: &[Box<dyn Control>],
    id: ControlId,
    surface: &crate::draw::Surface,
    parent_abs_x: i32,
    parent_abs_y: i32,
) {
    let idx = match control::find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };

    if !controls[idx].visible() {
        return;
    }

    controls[idx].render(surface, parent_abs_x, parent_abs_y);

    let (cx, cy) = controls[idx].position();
    let child_abs_x = parent_abs_x + cx;
    let child_abs_y = parent_abs_y + cy;

    // Copy children to stack buffer to avoid Vec allocation
    let mut child_buf = [0u32; 64];
    let child_n = controls[idx].children().len().min(64);
    child_buf[..child_n].copy_from_slice(&controls[idx].children()[..child_n]);
    // Skip children if this is a collapsed Expander
    if controls[idx].kind() == ControlKind::Expander && controls[idx].base().state == 0 {
        return;
    }
    // ScrollView: offset children by -scroll_y and clip to viewport
    // Expander: offset children by +HEADER_HEIGHT (below header)
    let (child_abs_y, child_surface) = match controls[idx].kind() {
        ControlKind::ScrollView => {
            let sv_x = parent_abs_x + controls[idx].base().x;
            let sv_y = parent_abs_y + controls[idx].base().y;
            let sv_w = controls[idx].base().w;
            let sv_h = controls[idx].base().h;
            (
                child_abs_y - controls[idx].base().state as i32,
                surface.with_clip(sv_x, sv_y, sv_w, sv_h),
            )
        }
        ControlKind::Expander => (
            child_abs_y + crate::controls::expander::HEADER_HEIGHT as i32,
            *surface,
        ),
        _ => (child_abs_y, *surface),
    };
    for i in 0..child_n {
        render_tree(controls, child_buf[i], &child_surface, child_abs_x, child_abs_y);
    }
}

// ── Subtree removal ─────────────────────────────────────────────────

fn remove_subtree(controls: &mut Vec<Box<dyn Control>>, id: ControlId) {
    let mut to_remove = Vec::new();
    collect_descendants(controls, id, &mut to_remove);
    to_remove.push(id);

    if let Some(idx) = control::find_idx(controls, id) {
        let parent = controls[idx].parent_id();
        if let Some(pi) = control::find_idx(controls, parent) {
            controls[pi].remove_child(id);
        }
    }

    controls.retain(|c| !to_remove.contains(&c.id()));
}

fn collect_descendants(
    controls: &[Box<dyn Control>],
    id: ControlId,
    out: &mut Vec<ControlId>,
) {
    if let Some(idx) = control::find_idx(controls, id) {
        let children: Vec<ControlId> = controls[idx].children().to_vec();
        for &child in &children {
            out.push(child);
            collect_descendants(controls, child, out);
        }
    }
}
