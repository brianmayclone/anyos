//! Event loop — polls compositor events, dispatches via virtual methods, renders.
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
use crate::control::{self, ControlId, Control, Callback};

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
pub fn run() {
    loop {
        if run_once() == 0 {
            break;
        }
        crate::syscall::yield_cpu();
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

    // Increment frame counter for double-click detection
    st.last_click_tick = st.last_click_tick.wrapping_add(0); // frame count updated per-click

    // ── Phase 1: Poll events from all windows ──────────────────────
    let win_count = st.windows.len();
    for wi in 0..win_count {
        if wi >= st.windows.len() { break; }
        let win_id = st.windows[wi];
        let comp_win = st.comp_wins[wi];

        let mut ev = [0u32; 5];
        while crate::syscall::win_get_event(comp_win, &mut ev) != 0 {
            match ev[0] {
                control::COMP_EVENT_WINDOW_CLOSE => {
                    // Fire EVENT_CLOSE callback on the window control
                    fire_event_callback(&st.controls, win_id, control::EVENT_CLOSE, &mut pending_cbs);
                    windows_to_close.push(win_id);
                }

                control::COMP_EVENT_MOUSE_MOVE => {
                    let mx = ev[1] as i32;
                    let my = ev[2] as i32;

                    // Update hover tracking (MouseEnter / MouseLeave)
                    let new_hover = control::hit_test_any(&st.controls, win_id, mx, my, 0, 0);
                    let old_hover = st.hovered;

                    if new_hover != old_hover {
                        // MouseLeave on old control
                        if let Some(old_id) = old_hover {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_mouse_leave();
                                fire_event_callback(&st.controls, old_id, control::EVENT_MOUSE_LEAVE, &mut pending_cbs);
                            }
                        }
                        // MouseEnter on new control
                        if let Some(new_id) = new_hover {
                            if let Some(idx) = control::find_idx(&st.controls, new_id) {
                                st.controls[idx].handle_mouse_enter();
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

                control::COMP_EVENT_MOUSE_DOWN => {
                    let mx = ev[1] as i32;
                    let my = ev[2] as i32;
                    let button = ev[3];

                    let hit_id = control::hit_test(&st.controls, win_id, mx, my, 0, 0);

                    // Update focus
                    if let Some(new_focus) = hit_id {
                        let old_focus = st.focused;
                        if old_focus != Some(new_focus) {
                            // Blur old focused control
                            if let Some(old_id) = old_focus {
                                if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                    st.controls[idx].handle_blur();
                                    fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, &mut pending_cbs);
                                }
                            }
                            // Focus new control (only if it accepts focus)
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
                        // Clicked on empty space - blur current focus
                        if let Some(old_id) = st.focused {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_blur();
                                fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, &mut pending_cbs);
                            }
                        }
                        st.focused = None;
                    }

                    // Set pressed control
                    st.pressed = hit_id;

                    // Dispatch handle_mouse_down
                    if let Some(target_id) = hit_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;
                            let resp = st.controls[idx].handle_mouse_down(local_x, local_y, button);

                            // Always fire EVENT_MOUSE_DOWN callback
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

                control::COMP_EVENT_MOUSE_UP => {
                    let mx = ev[1] as i32;
                    let my = ev[2] as i32;
                    let button = ev[3];

                    let pressed_id = st.pressed.take();

                    if let Some(target_id) = pressed_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;

                            // Dispatch handle_mouse_up
                            let resp = st.controls[idx].handle_mouse_up(local_x, local_y, button);
                            fire_event_callback(&st.controls, target_id, control::EVENT_MOUSE_UP, &mut pending_cbs);

                            if resp.fire_change {
                                fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }

                            // Check if mouse is still over the pressed control → Click
                            let hit_now = control::hit_test(&st.controls, target_id,
                                mx - ax + local_x, my - ay + local_y, 0, 0);
                            let still_over = hit_now == Some(target_id)
                                || is_point_in_control(&st.controls, target_id, mx, my);

                            if still_over {
                                // Dispatch handle_click (virtual method)
                                if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                    let click_resp = st.controls[idx2].handle_click(local_x, local_y, button);

                                    // Fire EVENT_CLICK callback
                                    fire_event_callback(&st.controls, target_id, control::EVENT_CLICK, &mut pending_cbs);

                                    if click_resp.fire_change {
                                        fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                                    }

                                    // Double-click detection
                                    let tick = frame_tick();
                                    if st.last_click_id == Some(target_id)
                                        && tick.wrapping_sub(st.last_click_tick) <= DOUBLE_CLICK_FRAMES
                                    {
                                        // Double-click!
                                        if let Some(idx3) = control::find_idx(&st.controls, target_id) {
                                            let dc_resp = st.controls[idx3].handle_double_click(local_x, local_y, button);
                                            fire_event_callback(&st.controls, target_id, control::EVENT_DOUBLE_CLICK, &mut pending_cbs);
                                            if dc_resp.fire_change {
                                                fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
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

                control::COMP_EVENT_KEY_DOWN => {
                    let keycode = ev[1];
                    let char_code = ev[2];

                    // Dispatch to focused control
                    if let Some(focus_id) = st.focused {
                        if let Some(idx) = control::find_idx(&st.controls, focus_id) {
                            let resp = st.controls[idx].handle_key_down(keycode, char_code);

                            if resp.consumed {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_KEY, &mut pending_cbs);
                            }
                            if resp.fire_change {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }
                            if resp.fire_click {
                                fire_event_callback(&st.controls, focus_id, control::EVENT_CLICK, &mut pending_cbs);
                            }
                        }
                    }
                }

                control::COMP_EVENT_MOUSE_SCROLL => {
                    let dz = ev[1] as i32;
                    let mx = ev[2] as i32;
                    let my = ev[3] as i32;

                    // Dispatch to control under cursor, or hovered control
                    let target = if mx != 0 || my != 0 {
                        control::hit_test(&st.controls, win_id, mx, my, 0, 0)
                    } else {
                        st.hovered
                    };

                    if let Some(target_id) = target {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let resp = st.controls[idx].handle_scroll(dz);
                            if resp.consumed {
                                fire_event_callback(&st.controls, target_id, control::EVENT_SCROLL, &mut pending_cbs);
                            }
                            if resp.fire_change {
                                fire_event_callback(&st.controls, target_id, control::EVENT_CHANGE, &mut pending_cbs);
                            }
                        }
                    }
                }

                control::COMP_EVENT_WINDOW_RESIZE => {
                    let new_w = ev[1];
                    let new_h = ev[2];
                    if let Some(idx) = control::find_idx(&st.controls, win_id) {
                        st.controls[idx].set_size(new_w, new_h);
                        fire_event_callback(&st.controls, win_id, control::EVENT_RESIZE, &mut pending_cbs);
                    }
                }

                _ => {}
            }
            ev = [0u32; 5];
        }
    }

    // ── Phase 2: Close windows ──────────────────────────────────────
    for win_id in &windows_to_close {
        if let Some(wi) = st.windows.iter().position(|&w| w == *win_id) {
            let comp_win = st.comp_wins[wi];
            crate::syscall::win_destroy(comp_win);
            st.windows.remove(wi);
            st.comp_wins.remove(wi);
        }
        // Clear tracking for controls being removed
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

    // ── Phase 4: Render all windows ────────────────────────────────
    for wi in 0..st.windows.len() {
        let win_id = st.windows[wi];
        let comp_win = st.comp_wins[wi];

        // Clear window background
        if let Some(idx) = control::find_idx(&st.controls, win_id) {
            let (w, h) = st.controls[idx].size();
            crate::syscall::win_fill_rect(comp_win, 0, 0, w, h, crate::uisys::color_window_bg());
        }

        // Render control tree (depth-first, parent before children)
        render_tree(&st.controls, win_id, comp_win, 0, 0);

        // Present
        crate::syscall::win_present(comp_win);
    }

    1
}

// ── Helper functions ────────────────────────────────────────────────

/// Fire a user callback if registered for the given event type.
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

/// Check if a point (in window coords) is within a control's bounds.
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

/// Simple frame counter for double-click detection.
/// Incremented globally each time run_once() processes a MOUSE_UP with click.
static mut FRAME_COUNTER: u64 = 0;

fn frame_tick() -> u64 {
    unsafe {
        FRAME_COUNTER += 1;
        FRAME_COUNTER
    }
}

/// Clear event tracking state for a control and its descendants.
fn clear_tracking_for(st: &mut crate::AnyuiState, id: ControlId) {
    if st.focused == Some(id) { st.focused = None; }
    if st.pressed == Some(id) { st.pressed = None; }
    if st.hovered == Some(id) { st.hovered = None; }

    // Also clear for descendants
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let children: Vec<ControlId> = ctrl.children().to_vec();
        for &child in &children {
            clear_tracking_for(st, child);
        }
    }
}

// ── Tree rendering ──────────────────────────────────────────────────

fn render_tree(
    controls: &[Box<dyn Control>],
    id: ControlId,
    win: u32,
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

    // Render this control
    controls[idx].render(win, parent_abs_x, parent_abs_y);

    // Calculate absolute position for children
    let (cx, cy) = controls[idx].position();
    let child_abs_x = parent_abs_x + cx;
    let child_abs_y = parent_abs_y + cy;

    // Render children
    let children: Vec<ControlId> = controls[idx].children().to_vec();
    for &child_id in &children {
        render_tree(controls, child_id, win, child_abs_x, child_abs_y);
    }
}

// ── Subtree removal ─────────────────────────────────────────────────

fn remove_subtree(controls: &mut Vec<Box<dyn Control>>, id: ControlId) {
    let mut to_remove = Vec::new();
    collect_descendants(controls, id, &mut to_remove);
    to_remove.push(id);

    // Remove from parent's children
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
