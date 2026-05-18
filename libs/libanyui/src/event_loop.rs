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

use crate::compositor;
use crate::control::{self, Callback, Control, ControlId, ControlKind};
use alloc::boxed::Box;
use alloc::vec::Vec;

/// Double-click threshold (standard: 400ms). Phase-2 confd-backed
/// override was reverted because every read blocks up to 5 s when
/// confd is unreachable, which froze the entire UI hot-path on every
/// click. A confd-watch-based path will replace this once available.
const DOUBLE_CLICK_MS: u32 = 400;
const FRAME_ACK_TIMEOUT_MS: u32 = 100;

/// A pending callback to fire after all event processing.
struct PendingCallback {
    id: ControlId,
    event_type: u32,
    cb: Callback,
    userdata: u64,
}

#[derive(Clone, Copy)]
struct ScrollBlitDamage {
    id: ControlId,
    parent_abs_x: i32,
    parent_abs_y: i32,
    abs_x: i32,
    abs_y: i32,
    view_w: u32,
    view_h: u32,
    dx: i32,
    dy: i32,
}

/// Run the event loop. Blocks until all windows are closed or quit is requested.
/// Event-driven: blocks on `evt_chan_wait` until the compositor delivers an event
/// or the next timer fires. VSync back-pressure uses a shorter timeout.
pub fn run() {
    loop {
        if run_once() == 0 {
            break;
        }

        // Compute time until next timer fires
        let st = crate::state();
        let now = crate::syscall::uptime_ms();
        let mut min_wait: u32 = 1000; // default: wake every 1s max
        for slot in &st.timers.slots {
            let elapsed_since_fire = now.wrapping_sub(slot.last_fired_ms);
            if elapsed_since_fire >= slot.interval_ms {
                min_wait = 0; // timer already overdue — don't block
                break;
            }
            let remaining = slot.interval_ms - elapsed_since_fire;
            if remaining < min_wait {
                min_wait = remaining;
            }
        }

        // Tooltip delay: wake up when tooltip should appear
        if st.tooltip_pending_id.is_some() {
            let elapsed = now.wrapping_sub(st.tooltip_hover_start);
            if elapsed >= 500 {
                min_wait = 0;
            } else {
                min_wait = min_wait.min(500 - elapsed);
            }
        }

        // VSync back-pressure: poll faster when a frame is pending ACK
        if st.comp_windows.iter().any(|cw| cw.frame_presented) {
            min_wait = min_wait.min(8);
        }

        if min_wait > 0 {
            // Block until compositor sends event OR timer timeout
            crate::syscall::evt_chan_wait(st.reply_channel_id, st.sub_id, min_wait);
        }
    }
}

/// Process one frame of events + rendering. Returns 1 if windows remain, 0 if done.
pub fn run_once() -> u32 {
    let mut pending_cbs: Vec<PendingCallback> = Vec::new();
    let mut windows_to_close: Vec<ControlId> = Vec::new();

    // Refresh the cached DPI scale factor once per frame so all
    // scale()/unscale() calls within this iteration use a consistent value.
    crate::theme::refresh_scale_cache();

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

    // ── Phase 0.75: Show pending tooltip after hover delay ─────────
    {
        const TOOLTIP_DELAY_MS: u32 = 500;
        if let Some(pending_id) = st.tooltip_pending_id {
            let now = crate::syscall::uptime_ms();
            if now.wrapping_sub(st.tooltip_hover_start) >= TOOLTIP_DELAY_MS {
                // Still hovering the same control?
                if st.hovered == Some(pending_id) {
                    st.tooltip_pending_id = None;
                    show_tooltip(st, pending_id);
                } else {
                    st.tooltip_pending_id = None;
                }
            }
        }
    }

    // ── Phase 1: Poll events from all windows ──────────────────────
    // Drain ALL events from the channel first, then dispatch per window.
    // This avoids the compositor's poll_event discarding events for other
    // windows when multiple windows share the same event channel.
    let mut all_events: Vec<[u32; 5]> = Vec::new();
    {
        let mut tmp = [0u32; 5];
        while crate::syscall::evt_chan_poll(st.reply_channel_id, st.sub_id, &mut tmp) {
            all_events.push(tmp);
            tmp = [0u32; 5];
        }
    }

    // ── Phase 1.05: Dispatch tray icon click events ───────────────
    // EVT_STATUS_ICON_CLICK uses icon_id in ev[1], not a window_id,
    // so it would be filtered out by the per-window loop below.
    for ev in all_events.iter_mut() {
        if ev[0] == compositor::EVT_STATUS_ICON_CLICK {
            let icon_id = ev[1];
            if let Some(&(_, cb, ud)) = st.tray_callbacks.iter().find(|e| e.0 == icon_id) {
                pending_cbs.push(PendingCallback {
                    id: icon_id,
                    event_type: compositor::EVT_STATUS_ICON_CLICK,
                    cb,
                    userdata: ud,
                });
            }
            ev[0] = 0; // consume
        }
    }

    // ── Phase 1.1: Process popup events (before per-window dispatch) ──
    // Context menu popups are separate compositor windows. Their events must
    // be handled before normal window events to ensure dismiss-on-outside-click.
    {
        let popup_wid = st.popup.as_ref().map(|p| p.window_id);
        if let Some(popup_window_id) = popup_wid {
            for ev in all_events.iter_mut() {
                // Handle events for the popup window
                if ev[0] >= 0x3000 && ev[1] == popup_window_id {
                    match ev[0] {
                        compositor::EVT_MOUSE_MOVE => {
                            // Physical pixels from compositor — convert to logical
                            // so bounds checks align with the menu control's logical dimensions.
                            let mx = crate::theme::unscale(ev[2] as i32);
                            let my = crate::theme::unscale(ev[3] as i32);
                            // Extract popup data to release borrow before accessing controls
                            let popup_data = st.popup.as_ref().map(|p| (p.margin, p.menu_id));
                            if let Some((margin, menu_id)) = popup_data {
                                if let Some(idx) = control::find_idx(&st.controls, menu_id) {
                                    let mw = st.controls[idx].base().w;
                                    let mh = st.controls[idx].base().h;
                                    if mx >= margin
                                        && my >= margin
                                        && mx < margin + mw as i32
                                        && my < margin + mh as i32
                                    {
                                        // Inside menu bounds → dispatch move
                                        let local_x = mx - margin;
                                        let local_y = my - margin;
                                        st.controls[idx].handle_mouse_move(local_x, local_y);
                                    } else {
                                        // In margin area → de-highlight
                                        st.controls[idx].handle_mouse_leave();
                                    }
                                    if let Some(ref mut p) = st.popup {
                                        p.dirty = true;
                                    }
                                }
                            }
                        }
                        compositor::EVT_MOUSE_DOWN => {
                            // Physical pixels from compositor — convert to logical.
                            let mx = crate::theme::unscale(ev[2] as i32);
                            let my = crate::theme::unscale(ev[3] as i32);
                            let popup_data = st.popup.as_ref().map(|p| (p.margin, p.menu_id));
                            if let Some((margin, menu_id)) = popup_data {
                                if let Some(idx) = control::find_idx(&st.controls, menu_id) {
                                    let mw = st.controls[idx].base().w;
                                    let mh = st.controls[idx].base().h;
                                    if mx >= margin
                                        && my >= margin
                                        && mx < margin + mw as i32
                                        && my < margin + mh as i32
                                    {
                                        st.pressed = Some(menu_id);
                                        st.pressed_button = ev[4] & 0xFF;
                                    } else {
                                        // Click outside menu area in popup → dismiss
                                        dismiss_popup(st);
                                    }
                                }
                            }
                        }
                        compositor::EVT_MOUSE_UP => {
                            // Physical pixels from compositor — convert to logical.
                            let mx = crate::theme::unscale(ev[2] as i32);
                            let my = crate::theme::unscale(ev[3] as i32);
                            if let Some(menu_id) = st.pressed.take() {
                                let margin = st.popup.as_ref().map(|p| p.margin).unwrap_or(0);
                                let owner_dd = st.popup.as_ref().and_then(|p| p.owner_dropdown);
                                let owner_cb = st.popup.as_ref().and_then(|p| p.owner_combobox);
                                if let Some(idx) = control::find_idx(&st.controls, menu_id) {
                                    let (ax, ay) =
                                        (st.controls[idx].base().x, st.controls[idx].base().y);
                                    let local_x = mx - margin - ax;
                                    let local_y = my - margin - ay;
                                    let click_resp =
                                        st.controls[idx].handle_click(local_x, local_y, 0x01);

                                    if click_resp.fire_click {
                                        let owner_ac =
                                            st.popup.as_ref().and_then(|p| p.owner_autocomplete);
                                        if let Some(ac_id) = owner_ac {
                                            // AutoComplete popup: transfer selected text
                                            let selected_idx =
                                                st.controls[idx].base().state as usize;
                                            // Extract the Nth pipe-separated item from menu text
                                            let menu_text = st.controls[idx]
                                                .text_base()
                                                .map(|tb| tb.text.clone())
                                                .unwrap_or_default();
                                            let full_item: alloc::vec::Vec<u8> = menu_text
                                                .split(|&b| b == b'|')
                                                .nth(selected_idx)
                                                .unwrap_or(&[])
                                                .to_vec();
                                            // Extract label (after \x1F if present)
                                            let selected_text = if let Some(sep) =
                                                full_item.iter().position(|&b| b == 0x1F)
                                            {
                                                full_item[sep + 1..].to_vec()
                                            } else {
                                                full_item
                                            };
                                            dismiss_popup(st);
                                            if !selected_text.is_empty() {
                                                if let Some(ac_idx) =
                                                    control::find_idx(&st.controls, ac_id)
                                                {
                                                    if let Some(ac) = control::cast_mut::<crate::controls::autocomplete_textfield::AutoCompleteTextField>(
                                                        &mut st.controls[ac_idx],
                                                        ControlKind::AutoCompleteTextField,
                                                    ) {
                                                        ac.text_base.text = selected_text;
                                                        ac.cursor_pos = ac.text_base.text.len();
                                                        ac.sel_anchor = ac.cursor_pos;
                                                        ac.text_base.base.mark_dirty();
                                                    }
                                                }
                                                fire_event_callback(
                                                    &st.controls,
                                                    ac_id,
                                                    control::EVENT_CHANGE,
                                                    &mut pending_cbs,
                                                );
                                            }
                                        } else if let Some(dd_id) = owner_dd {
                                            // DropDown popup: transfer selected index to the DropDown
                                            let selected_idx = st.controls[idx].base().state;
                                            dismiss_popup(st);
                                            if let Some(dd_idx) =
                                                control::find_idx(&st.controls, dd_id)
                                            {
                                                st.controls[dd_idx].base_mut().state = selected_idx;
                                                st.controls[dd_idx].base_mut().mark_dirty();
                                            }
                                            fire_event_callback(
                                                &st.controls,
                                                dd_id,
                                                control::EVENT_CHANGE,
                                                &mut pending_cbs,
                                            );
                                        } else if let Some(cb_id) = owner_cb {
                                            let selected_idx =
                                                st.controls[idx].base().state as usize;
                                            let menu_text = st.controls[idx]
                                                .text_base()
                                                .map(|tb| tb.text.clone())
                                                .unwrap_or_default();
                                            let full_item: alloc::vec::Vec<u8> = menu_text
                                                .split(|&b| b == b'|')
                                                .nth(selected_idx)
                                                .unwrap_or(&[])
                                                .to_vec();
                                            let actual_idx = full_item
                                                .iter()
                                                .position(|&b| b == 0x1F)
                                                .and_then(|sep| {
                                                    core::str::from_utf8(&full_item[..sep]).ok()
                                                })
                                                .and_then(|s| s.parse::<usize>().ok());
                                            dismiss_popup(st);
                                            if let Some(actual_idx) = actual_idx {
                                                if let Some(cb_idx) =
                                                    control::find_idx(&st.controls, cb_id)
                                                {
                                                    if let Some(cb) = control::cast_mut::<
                                                        crate::controls::combobox::ComboBox,
                                                    >(
                                                        &mut st.controls[cb_idx],
                                                        ControlKind::ComboBox,
                                                    ) {
                                                        cb.apply_selected_index(actual_idx);
                                                    }
                                                }
                                                fire_event_callback(
                                                    &st.controls,
                                                    cb_id,
                                                    control::EVENT_CHANGE,
                                                    &mut pending_cbs,
                                                );
                                            }
                                        } else if let Some(te_id) =
                                            st.popup.as_ref().and_then(|p| p.owner_text_edit)
                                        {
                                            // Built-in text-edit context menu:
                                            //   0 = Cut, 1 = Copy, 2 = Paste,
                                            //   3 = divider (skipped by ContextMenu),
                                            //   4 = Select All
                                            let selected_idx = st.controls[idx].base().state;
                                            let action_char: u32 = match selected_idx {
                                                0 => b'x' as u32,
                                                1 => b'c' as u32,
                                                2 => b'v' as u32,
                                                4 => b'a' as u32,
                                                _ => 0,
                                            };
                                            dismiss_popup(st);
                                            if action_char != 0 {
                                                if let Some(te_idx) =
                                                    control::find_idx(&st.controls, te_id)
                                                {
                                                    let resp = st.controls[te_idx].handle_key_down(
                                                        0,
                                                        action_char,
                                                        control::MOD_CTRL,
                                                    );
                                                    st.controls[te_idx].base_mut().mark_dirty();
                                                    if resp.fire_change {
                                                        fire_event_callback(
                                                            &st.controls,
                                                            te_id,
                                                            control::EVENT_CHANGE,
                                                            &mut pending_cbs,
                                                        );
                                                    }
                                                }
                                            }
                                        } else {
                                            // Normal context menu
                                            dismiss_popup(st);
                                            fire_event_callback(
                                                &st.controls,
                                                menu_id,
                                                control::EVENT_CLICK,
                                                &mut pending_cbs,
                                            );
                                        }
                                    } else {
                                        // Clicked on divider or empty area — keep popup open
                                    }
                                }
                            }
                        }
                        compositor::EVT_FOCUS_LOST => {
                            // Another window gained focus → dismiss popup
                            // But NOT for AutoComplete popups — they intentionally
                            // keep focus on the main window's text field.
                            let is_ac = st
                                .popup
                                .as_ref()
                                .map(|p| p.owner_autocomplete.is_some())
                                .unwrap_or(false);
                            if !is_ac {
                                dismiss_popup(st);
                            }
                        }
                        compositor::EVT_KEY_DOWN => {
                            let keycode = ev[2];
                            if keycode == control::KEY_ESCAPE {
                                dismiss_popup(st);
                            } else {
                                let menu_id = st.popup.as_ref().map(|p| p.menu_id);
                                let owner_ac = st.popup.as_ref().and_then(|p| p.owner_autocomplete);
                                if owner_ac.is_none() {
                                    if let Some(menu_id) = menu_id {
                                        if let Some(idx) = control::find_idx(&st.controls, menu_id)
                                        {
                                            let resp =
                                                st.controls[idx].handle_key_down(keycode, 0, 0);
                                            st.controls[idx].base_mut().mark_dirty();
                                            if let Some(ref mut popup) = st.popup {
                                                popup.dirty = true;
                                            }
                                            if resp.fire_click {
                                                dismiss_popup(st);
                                                fire_event_callback(
                                                    &st.controls,
                                                    menu_id,
                                                    control::EVENT_CLICK,
                                                    &mut pending_cbs,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    // Mark event as consumed so per-window loop skips it
                    ev[0] = 0;
                }
            }

            // Check if any MOUSE_DOWN for a non-popup window while popup is active → dismiss
            if st.popup.is_some() {
                for ev in all_events.iter_mut() {
                    if ev[0] == compositor::EVT_MOUSE_DOWN && ev[1] != popup_window_id {
                        dismiss_popup(st);
                        break;
                    }
                }
            }
        }
    }

    // ── Phase 1.2: Process broadcasts (theme change, window lifecycle) ──
    for ev in all_events.iter() {
        if ev[0] == 0 {
            continue;
        }
        match ev[0] {
            // EVT_THEME_CHANGED (0x0050): mark all windows dirty so every
            // control re-renders with the new theme palette.
            0x0050 => {
                for &win_id in &st.windows {
                    if let Some(idx) = crate::control::find_idx(&st.controls, win_id) {
                        mark_tree_dirty(&mut st.controls, idx);
                    }
                }
            }
            // EVT_FONT_SMOOTHING_CHANGED (0x0051): mark all windows dirty
            // so text re-renders with the new smoothing mode.
            0x0051 => {
                for &win_id in &st.windows {
                    if let Some(idx) = crate::control::find_idx(&st.controls, win_id) {
                        mark_tree_dirty(&mut st.controls, idx);
                    }
                }
            }
            // EVT_SCALE_CHANGED (0x0052): DPI scale factor changed at runtime.
            // Refresh cached scale, resize SHM buffers to new physical dimensions,
            // and force a full redraw of all windows.
            0x0052 => {
                crate::theme::refresh_scale_cache();
                for cw in st.comp_windows.iter_mut() {
                    let phys_w = crate::theme::scale(cw.logical_width);
                    let phys_h = crate::theme::scale(cw.logical_height);
                    if phys_w != cw.width || phys_h != cw.height {
                        if let Some((new_shm_id, new_surface)) = crate::compositor::resize_shm(
                            st.channel_id,
                            cw.window_id,
                            cw.shm_id,
                            phys_w,
                            phys_h,
                        ) {
                            cw.shm_id = new_shm_id;
                            cw.surface = new_surface;
                        }
                        cw.width = phys_w;
                        cw.height = phys_h;
                        let new_count = (phys_w as usize) * (phys_h as usize);
                        cw.back_buffer.resize(new_count, 0);
                    }
                }
                for &win_id in &st.windows {
                    if let Some(idx) = crate::control::find_idx(&st.controls, win_id) {
                        mark_tree_dirty(&mut st.controls, idx);
                    }
                }
                st.needs_layout = true;
            }
            0x0060 => {
                // EVT_WINDOW_OPENED: ev[1] = app_tid
                if let Some((cb, ud)) = st.on_window_opened {
                    pending_cbs.push(PendingCallback {
                        id: ev[1],
                        event_type: 0x0060,
                        cb,
                        userdata: ud,
                    });
                }
            }
            0x0061 => {
                // EVT_WINDOW_CLOSED: ev[1] = app_tid
                if let Some((cb, ud)) = st.on_window_closed {
                    pending_cbs.push(PendingCallback {
                        id: ev[1],
                        event_type: 0x0061,
                        cb,
                        userdata: ud,
                    });
                }
            }
            0x0064 => {
                // EVT_WINDOW_LIST_ENTRY: ev[1] = app_tid
                st.window_list_buffer.push(ev[1]);
            }
            0x0065 => {
                // EVT_WINDOW_LIST_END: ev[1] = count
                if let Some((cb, ud)) = st.on_window_list {
                    pending_cbs.push(PendingCallback {
                        id: ev[1], // count
                        event_type: 0x0065,
                        cb,
                        userdata: ud,
                    });
                }
                // Buffer is cleared on next request_window_list() call
            }
            _ => {}
        }
    }

    // ── Modal filtering: determine which window index is allowed to receive events ──
    // When a modal is active, only the topmost modal's window gets input events.
    // For in-window modals (overlay_id != 0), the overlay catches clicks within the window.
    // For separate-window modals (modal_win_id != 0), other windows' events are discarded.
    let modal_allowed_win_idx: Option<usize> = if !st.modal_stack.is_empty() {
        let top = st.modal_stack.last().unwrap();
        if top.modal_win_id != 0 {
            // Separate-window modal: only the modal window gets events
            st.windows.iter().position(|&w| w == top.modal_win_id)
        } else if top.overlay_id != 0 {
            // In-window overlay modal: the owner window gets events (overlay blocks clicks)
            st.windows.iter().position(|&w| w == top.owner_win_id)
        } else {
            None
        }
    } else {
        None // No modal active — all windows receive events
    };

    let win_count = st.windows.len();
    for wi in 0..win_count {
        if wi >= st.windows.len() {
            break;
        }
        let win_id = st.windows[wi];
        let comp_window_id = st.comp_windows[wi].window_id;

        // Modal event filtering: skip input events for non-modal windows.
        // Allow EVT_WINDOW_CLOSE (0x3007) and EVT_FRAME_ACK (0x300B) through
        // so that windows can still be closed and rendering continues.
        if let Some(allowed_wi) = modal_allowed_win_idx {
            if wi != allowed_wi {
                // Still process non-input events for this window
                for ev in all_events.iter() {
                    if ev[0] == 0 {
                        continue;
                    }
                    if ev[0] >= 0x3000 && ev[1] != comp_window_id {
                        continue;
                    }
                    match ev[0] {
                        compositor::EVT_FRAME_ACK => {
                            if let Some(cw) = st.comp_windows.get_mut(wi) {
                                cw.frame_presented = false;
                            }
                        }
                        _ => {} // Discard all other events for blocked windows
                    }
                }
                continue;
            }
        }

        // Process events that belong to this window
        // Buffer layout: [event_type, window_id, arg1, arg2, arg3]
        for ev in all_events.iter() {
            // Skip consumed popup events
            if ev[0] == 0 {
                continue;
            }
            // Window-specific events (0x3000+): filter by window_id
            if ev[0] >= 0x3000 && ev[1] != comp_window_id {
                continue;
            }
            // Broadcast events (<0x1000): only process on first window
            if ev[0] < 0x1000 && wi > 0 {
                continue;
            }
            // Skip unknown range
            if ev[0] >= 0x1000 && ev[0] < 0x3000 {
                continue;
            }

            match ev[0] {
                compositor::EVT_WINDOW_CLOSE => {
                    fire_event_callback(
                        &st.controls,
                        win_id,
                        control::EVENT_CLOSE,
                        &mut pending_cbs,
                    );
                    windows_to_close.push(win_id);
                }

                compositor::EVT_MOUSE_MOVE => {
                    // arg1=local_x, arg2=local_y (physical pixels from compositor).
                    // Convert to logical pixels for the control tree.
                    let mx = crate::theme::unscale(ev[2] as i32);
                    let my = crate::theme::unscale(ev[3] as i32);
                    st.last_mouse_x = mx;
                    st.last_mouse_y = my;

                    // Update hover tracking (MouseEnter / MouseLeave)
                    let new_hover = control::hit_test_any(&st.controls, win_id, mx, my, 0, 0);
                    let old_hover = st.hovered;

                    if new_hover != old_hover {
                        if let Some(old_id) = old_hover {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_mouse_leave();
                                st.controls[idx].base_mut().mark_dirty();
                                fire_event_callback(
                                    &st.controls,
                                    old_id,
                                    control::EVENT_MOUSE_LEAVE,
                                    &mut pending_cbs,
                                );
                            }
                        }
                        if let Some(new_id) = new_hover {
                            if let Some(idx) = control::find_idx(&st.controls, new_id) {
                                st.controls[idx].handle_mouse_enter();
                                st.controls[idx].base_mut().mark_dirty();
                                fire_event_callback(
                                    &st.controls,
                                    new_id,
                                    control::EVENT_MOUSE_ENTER,
                                    &mut pending_cbs,
                                );
                            }
                        }
                        st.hovered = new_hover;

                        // --- Tooltip management ---
                        // Hide tooltip when hover changes
                        if let Some(tip_id) = st.active_tooltip {
                            if let Some(ti) = control::find_idx(&st.controls, tip_id) {
                                if st.controls[ti].base().visible {
                                    st.controls[ti].base_mut().visible = false;
                                    st.controls[ti].base_mut().mark_dirty();
                                }
                            }
                        }
                        // Schedule tooltip after delay if newly hovered control has tooltip_text
                        if let Some(new_id) = new_hover {
                            let has_tip = control::find_idx(&st.controls, new_id)
                                .map(|i| !st.controls[i].base().tooltip_text.is_empty())
                                .unwrap_or(false);
                            if has_tip {
                                st.tooltip_pending_id = Some(new_id);
                                st.tooltip_hover_start = crate::syscall::uptime_ms();
                            } else {
                                st.tooltip_pending_id = None;
                            }
                        } else {
                            st.tooltip_pending_id = None;
                        }
                    }

                    // Update cursor shape (check SplitView dividers etc.).
                    // Skip while a drag is in progress — the drag code owns
                    // the cursor (Move shape) until the drop or cancel.
                    if st.drag.is_none() {
                        let desired_cursor =
                            control::cursor_at_point(&st.controls, win_id, mx, my, 0, 0);
                        if desired_cursor != st.current_cursor {
                            st.current_cursor = desired_cursor;
                            // CMD_SET_CURSOR = 0x1018
                            let cmd: [u32; 5] = [0x1018, comp_window_id, desired_cursor, 0, 0];
                            crate::syscall::evt_chan_emit(st.channel_id, &cmd);
                        }
                    }

                    // Dispatch mouse_move to hovered control — always call
                    // handle_mouse_move to update internal state (e.g. Canvas
                    // last_mouse_x/y) and always fire EVENT_MOUSE_MOVE so apps
                    // can react to hover position changes (tooltips, highlights).
                    if let Some(hover_id) = st.hovered {
                        if let Some(idx) = control::find_idx(&st.controls, hover_id) {
                            let (ax, ay) = control::abs_position(&st.controls, hover_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;
                            let resp = st.controls[idx].handle_mouse_move(local_x, local_y);
                            if resp.consumed {
                                st.controls[idx].base_mut().mark_dirty();
                            }
                        }
                        fire_event_callback(
                            &st.controls,
                            hover_id,
                            control::EVENT_MOUSE_MOVE,
                            &mut pending_cbs,
                        );
                    }

                    // If a different control is pressed (drag outside), dispatch
                    // mouse_move to it as well for drag tracking.
                    if let Some(pressed_id) = st.pressed {
                        if st.hovered != Some(pressed_id) {
                            if let Some(idx) = control::find_idx(&st.controls, pressed_id) {
                                let (ax, ay) = control::abs_position(&st.controls, pressed_id);
                                let local_x = mx - ax;
                                let local_y = my - ay;
                                let resp = st.controls[idx].handle_mouse_move(local_x, local_y);
                                if resp.consumed {
                                    st.controls[idx].base_mut().mark_dirty();
                                    fire_event_callback(
                                        &st.controls,
                                        pressed_id,
                                        control::EVENT_MOUSE_MOVE,
                                        &mut pending_cbs,
                                    );
                                    if resp.fire_change {
                                        fire_event_callback(
                                            &st.controls,
                                            pressed_id,
                                            control::EVENT_CHANGE,
                                            &mut pending_cbs,
                                        );
                                    }
                                    if resp.fire_click {
                                        fire_event_callback(
                                            &st.controls,
                                            pressed_id,
                                            control::EVENT_CLICK,
                                            &mut pending_cbs,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    maybe_begin_drag(st, win_id, comp_window_id, mx, my, &mut pending_cbs);
                    // Note: target detection during a drag is driven by
                    // compositor EVT_DRAG_OVER events, not by mouse_move.
                }

                compositor::EVT_MOUSE_DOWN => {
                    // arg1=local_x, arg2=local_y (physical), arg3=buttons|modifiers<<8.
                    // Convert to logical pixels for the control tree.
                    let mx = crate::theme::unscale(ev[2] as i32);
                    let my = crate::theme::unscale(ev[3] as i32);
                    let button = ev[4] & 0xFF;
                    st.last_modifiers = (ev[4] >> 8) & 0xFF;

                    let hit_id = control::hit_test(&st.controls, win_id, mx, my, 0, 0);

                    // Update focus
                    if let Some(new_focus) = hit_id {
                        let old_focus = st.focused;
                        if old_focus != Some(new_focus) {
                            if let Some(old_id) = old_focus {
                                if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                    st.controls[idx].handle_blur();
                                    fire_event_callback(
                                        &st.controls,
                                        old_id,
                                        control::EVENT_BLUR,
                                        &mut pending_cbs,
                                    );
                                }
                            }
                            if let Some(idx) = control::find_idx(&st.controls, new_focus) {
                                if st.controls[idx].accepts_focus() {
                                    st.controls[idx].handle_focus();
                                    st.focused = Some(new_focus);
                                    fire_event_callback(
                                        &st.controls,
                                        new_focus,
                                        control::EVENT_FOCUS,
                                        &mut pending_cbs,
                                    );
                                } else {
                                    st.focused = None;
                                }
                            }
                        }
                    } else {
                        if let Some(old_id) = st.focused {
                            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                                st.controls[idx].handle_blur();
                                fire_event_callback(
                                    &st.controls,
                                    old_id,
                                    control::EVENT_BLUR,
                                    &mut pending_cbs,
                                );
                            }
                        }
                        st.focused = None;
                    }

                    st.pressed = hit_id;
                    st.pressed_button = button;
                    st.press_mouse_x = mx;
                    st.press_mouse_y = my;
                    crate::log!(
                        "[dnd] MOUSE_DOWN hit={} button={} mx={} my={}",
                        hit_id.unwrap_or(0),
                        button,
                        mx,
                        my
                    );

                    if let Some(target_id) = hit_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;
                            let resp = st.controls[idx].handle_mouse_down(local_x, local_y, button);
                            st.controls[idx].base_mut().mark_dirty();

                            fire_event_callback(
                                &st.controls,
                                target_id,
                                control::EVENT_MOUSE_DOWN,
                                &mut pending_cbs,
                            );

                            if resp.fire_change {
                                fire_event_callback(
                                    &st.controls,
                                    target_id,
                                    control::EVENT_CHANGE,
                                    &mut pending_cbs,
                                );
                            }
                            if resp.fire_click {
                                fire_event_callback(
                                    &st.controls,
                                    target_id,
                                    control::EVENT_CLICK,
                                    &mut pending_cbs,
                                );
                            }
                        }
                    }
                }

                compositor::EVT_MOUSE_UP => {
                    // arg1=local_x, arg2=local_y (physical), arg3=modifiers<<8.
                    // Convert to logical pixels for the control tree.
                    let mx = crate::theme::unscale(ev[2] as i32);
                    let my = crate::theme::unscale(ev[3] as i32);
                    let button = ev[4] & 0xFF;
                    st.last_modifiers = (ev[4] >> 8) & 0xFF;

                    // Aborted drag (DRAG_START callback never installed a
                    // payload, so we never announced to the compositor).
                    // DROP/DRAG_LEAVE come via EVT_DROP / EVT_DRAG_END for
                    // announced drags. Here we just fire DRAG_END and
                    // restore the cursor.
                    if let Some(drag) = st.drag.as_ref() {
                        let source_id = drag.source_id;
                        fire_event_callback(
                            &st.controls,
                            source_id,
                            control::EVENT_DRAG_END,
                            &mut pending_cbs,
                        );
                        if st.current_cursor != 0 {
                            st.current_cursor = 0;
                            let cmd: [u32; 5] = [0x1018, comp_window_id, 0, 0, 0];
                            crate::syscall::evt_chan_emit(st.channel_id, &cmd);
                        }
                        st.pressed = None;
                        st.pressed_button = 0;
                        st.drag_release_pending = true;
                        continue;
                    }

                    let pressed_id = st.pressed.take();

                    if let Some(target_id) = pressed_id {
                        if let Some(idx) = control::find_idx(&st.controls, target_id) {
                            let (ax, ay) = control::abs_position(&st.controls, target_id);
                            let local_x = mx - ax;
                            let local_y = my - ay;

                            let resp = st.controls[idx].handle_mouse_up(local_x, local_y, button);
                            st.controls[idx].base_mut().mark_dirty();
                            fire_event_callback(
                                &st.controls,
                                target_id,
                                control::EVENT_MOUSE_UP,
                                &mut pending_cbs,
                            );

                            if resp.fire_change {
                                fire_event_callback(
                                    &st.controls,
                                    target_id,
                                    control::EVENT_CHANGE,
                                    &mut pending_cbs,
                                );
                            }

                            // Check if mouse is still over the pressed control → Click
                            let still_over = is_point_in_control(&st.controls, target_id, mx, my);

                            if still_over {
                                if st.pressed_button & 0x02 != 0 {
                                    // Right-click → fire EVENT_CONTEXT_MENU
                                    fire_event_callback(
                                        &st.controls,
                                        target_id,
                                        control::EVENT_CONTEXT_MENU,
                                        &mut pending_cbs,
                                    );

                                    // Auto-attach a built-in Cut/Copy/Paste/Select All
                                    // context menu for text-input controls that don't
                                    // already have a user-defined context menu.
                                    let mut dyn_text_target: Option<ControlId> = None;
                                    if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                        let kind = st.controls[idx2].kind();
                                        let has_menu =
                                            st.controls[idx2].base().context_menu.is_some();
                                        let is_text_input = matches!(
                                            kind,
                                            ControlKind::TextField
                                                | ControlKind::TextArea
                                                | ControlKind::SearchField
                                                | ControlKind::ComboBox
                                                | ControlKind::AutoCompleteTextField
                                        );
                                        if !has_menu && is_text_input {
                                            let new_menu_id = st.next_id;
                                            st.next_id += 1;
                                            let items_text: alloc::vec::Vec<u8> =
                                                b"Cut|Copy|Paste|-|Select All".to_vec();
                                            let menu_ctrl = crate::controls::create_control(
                                                ControlKind::ContextMenu,
                                                new_menu_id,
                                                0,
                                                0,
                                                0,
                                                0,
                                                0,
                                                &items_text,
                                            );
                                            st.controls.push(menu_ctrl);
                                            if let Some(i2) =
                                                control::find_idx(&st.controls, target_id)
                                            {
                                                st.controls[i2].base_mut().context_menu =
                                                    Some(new_menu_id);
                                            }
                                            dyn_text_target = Some(target_id);
                                        }
                                    }

                                    // If control has a context menu, show it as a popup window
                                    if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                        if let Some(menu_id) = st.controls[idx2].base().context_menu
                                        {
                                            if let Some(mi) =
                                                control::find_idx(&st.controls, menu_id)
                                            {
                                                // Dismiss any existing popup first
                                                dismiss_popup(st);

                                                // Get menu dimensions (logical)
                                                let menu_w = st.controls[mi].base().w;
                                                let menu_h = st.controls[mi].base().h;

                                                // Shadow margin (logical pixels)
                                                let margin: i32 = 16;
                                                let popup_w = menu_w + (margin as u32) * 2;
                                                let popup_h = menu_h + (margin as u32) * 2;

                                                // Physical popup dimensions for SHM surface
                                                let phys_popup_w = crate::theme::scale(popup_w);
                                                let phys_popup_h = crate::theme::scale(popup_h);

                                                // Get parent window's content-area screen position (physical)
                                                let (content_x, content_y) =
                                                    compositor::get_window_position(
                                                        st.channel_id,
                                                        st.sub_id,
                                                        comp_window_id,
                                                    );

                                                // Calculate popup screen position (physical coords).
                                                // mx/my are logical — scale to physical for screen placement.
                                                let phys_mx = crate::theme::scale_i32(mx);
                                                let phys_my = crate::theme::scale_i32(my);
                                                let phys_margin = crate::theme::scale_i32(margin);
                                                let mut popup_x = content_x + phys_mx - phys_margin;
                                                let mut popup_y = content_y + phys_my - phys_margin;

                                                // Clamp to screen bounds (physical)
                                                let (scr_w, scr_h) = compositor::screen_size();
                                                if popup_x + phys_popup_w as i32 > scr_w as i32 {
                                                    popup_x = scr_w as i32 - phys_popup_w as i32;
                                                }
                                                if popup_y + phys_popup_h as i32 > scr_h as i32 {
                                                    popup_y = scr_h as i32 - phys_popup_h as i32;
                                                }
                                                if popup_x < 0 {
                                                    popup_x = 0;
                                                }
                                                if popup_y < 0 {
                                                    popup_y = 0;
                                                }

                                                // Create popup compositor window (borderless, always-on-top, immovable)
                                                // Flags: BORDERLESS=0x01 | NOT_RESIZABLE=0x02 | ALWAYS_ON_TOP=0x04 | NO_MOVE=0x100
                                                let popup_flags: u32 = 0x01 | 0x02 | 0x04 | 0x100;
                                                if let Some((popup_win_id, shm_id, surface)) =
                                                    compositor::create_window(
                                                        st.channel_id,
                                                        st.sub_id,
                                                        popup_x,
                                                        popup_y,
                                                        phys_popup_w,
                                                        phys_popup_h,
                                                        popup_flags,
                                                    )
                                                {
                                                    // Position menu at origin for clean popup rendering
                                                    st.controls[mi].set_position(0, 0);
                                                    // Menu stays invisible in parent (rendered directly in popup)
                                                    st.controls[mi].base_mut().visible = false;

                                                    // Back buffer at physical dimensions.
                                                    let back_buffer = alloc::vec![0u32; (phys_popup_w * phys_popup_h) as usize];
                                                    st.popup = Some(crate::PopupInfo {
                                                        window_id: popup_win_id,
                                                        shm_id,
                                                        surface,
                                                        width: phys_popup_w,
                                                        height: phys_popup_h,
                                                        back_buffer,
                                                        menu_id,
                                                        owner_win_idx: wi,
                                                        margin, // logical — used for hit-testing and render offset
                                                        dirty: true,
                                                        owner_dropdown: None,
                                                        owner_combobox: None,
                                                        owner_autocomplete: None,
                                                        owner_text_edit: dyn_text_target,
                                                    });
                                                    // Popup keeps focus from create_window so it
                                                    // receives keyboard navigation; clicking outside
                                                    // dismisses via line ~409 / EVT_FOCUS_LOST.
                                                }
                                            }
                                        }
                                    }

                                    // If we synthesized a built-in text-edit menu but the
                                    // popup never opened (e.g. compositor::create_window
                                    // failed), tear down the orphan ContextMenu and clear
                                    // the temporary context_menu pointer.
                                    if let Some(te_id) = dyn_text_target {
                                        let popup_owns_menu = st
                                            .popup
                                            .as_ref()
                                            .map(|p| p.owner_text_edit == Some(te_id))
                                            .unwrap_or(false);
                                        if !popup_owns_menu {
                                            if let Some(te_idx) =
                                                control::find_idx(&st.controls, te_id)
                                            {
                                                if let Some(orphan) =
                                                    st.controls[te_idx].base().context_menu
                                                {
                                                    st.controls[te_idx].base_mut().context_menu =
                                                        None;
                                                    st.controls.retain(|c| c.id() != orphan);
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Left-click → normal click + double-click handling
                                    if let Some(idx2) = control::find_idx(&st.controls, target_id) {
                                        let click_resp = st.controls[idx2]
                                            .handle_click(local_x, local_y, button);

                                        // ── DropDown popup ────────────────────────────────
                                        // If the clicked control is a DropDown with open==true,
                                        // create a popup compositor window with a ContextMenu.
                                        if st.controls[idx2].kind() == ControlKind::DropDown {
                                            let raw: *mut dyn Control = &mut *st.controls[idx2];
                                            let dd = unsafe {
                                                &mut *(raw
                                                    as *mut crate::controls::dropdown::DropDown)
                                            };
                                            if dd.open {
                                                dd.open = false; // clear immediately; popup takes over

                                                // Gather items text and DropDown dimensions
                                                let items_text: alloc::vec::Vec<u8> =
                                                    dd.text_base.text.clone();
                                                let dd_w = dd.text_base.base.w;
                                                let dd_h = dd.text_base.base.h;
                                                let dd_abs =
                                                    control::abs_position(&st.controls, target_id);

                                                // Dismiss any existing popup
                                                dismiss_popup(st);

                                                // Create a temporary ContextMenu control
                                                let menu_id = st.next_id;
                                                st.next_id += 1;
                                                let menu_ctrl = crate::controls::create_control(
                                                    ControlKind::ContextMenu,
                                                    menu_id,
                                                    0,
                                                    0,
                                                    0,
                                                    0,
                                                    0,
                                                    &items_text,
                                                );
                                                st.controls.push(menu_ctrl);

                                                // Force menu width to match DropDown width (min)
                                                if let Some(mi) =
                                                    control::find_idx(&st.controls, menu_id)
                                                {
                                                    let menu_w = st.controls[mi].base().w.max(dd_w);
                                                    st.controls[mi].base_mut().w = menu_w;
                                                    let menu_h = st.controls[mi].base().h;

                                                    // Shadow margin (logical pixels)
                                                    let margin: i32 = 16;
                                                    let popup_w = menu_w + (margin as u32) * 2;
                                                    let popup_h = menu_h + (margin as u32) * 2;

                                                    // Physical popup dimensions for SHM surface
                                                    let phys_popup_w = crate::theme::scale(popup_w);
                                                    let phys_popup_h = crate::theme::scale(popup_h);

                                                    // Position popup below the DropDown (physical coords).
                                                    // dd_abs is logical — scale for compositor screen placement.
                                                    let (content_x, content_y) =
                                                        compositor::get_window_position(
                                                            st.channel_id,
                                                            st.sub_id,
                                                            comp_window_id,
                                                        );
                                                    let phys_dd_x =
                                                        crate::theme::scale_i32(dd_abs.0);
                                                    let phys_dd_y =
                                                        crate::theme::scale_i32(dd_abs.1);
                                                    let phys_dd_h = crate::theme::scale(dd_h);
                                                    let phys_margin =
                                                        crate::theme::scale_i32(margin);
                                                    let phys_menu_h = crate::theme::scale(menu_h);
                                                    let mut popup_x =
                                                        content_x + phys_dd_x - phys_margin;
                                                    let mut popup_y =
                                                        content_y + phys_dd_y + phys_dd_h as i32
                                                            - phys_margin;

                                                    // Clamp to screen bounds (physical)
                                                    let (scr_w, scr_h) = compositor::screen_size();
                                                    if popup_x + phys_popup_w as i32 > scr_w as i32
                                                    {
                                                        popup_x =
                                                            scr_w as i32 - phys_popup_w as i32;
                                                    }
                                                    if popup_y + phys_popup_h as i32 > scr_h as i32
                                                    {
                                                        // Open upward if no room below
                                                        popup_y = content_y + phys_dd_y
                                                            - phys_menu_h as i32
                                                            - phys_margin;
                                                    }
                                                    if popup_x < 0 {
                                                        popup_x = 0;
                                                    }
                                                    if popup_y < 0 {
                                                        popup_y = 0;
                                                    }

                                                    // Create popup compositor window (physical dimensions)
                                                    let popup_flags: u32 =
                                                        0x01 | 0x02 | 0x04 | 0x100;
                                                    if let Some((popup_win_id, shm_id, surface)) =
                                                        compositor::create_window(
                                                            st.channel_id,
                                                            st.sub_id,
                                                            popup_x,
                                                            popup_y,
                                                            phys_popup_w,
                                                            phys_popup_h,
                                                            popup_flags,
                                                        )
                                                    {
                                                        st.controls[mi].set_position(0, 0);
                                                        st.controls[mi].base_mut().visible = false;

                                                        // Back buffer at physical dimensions.
                                                        let back_buffer = alloc::vec![0u32; (phys_popup_w * phys_popup_h) as usize];
                                                        st.popup = Some(crate::PopupInfo {
                                                            window_id: popup_win_id,
                                                            shm_id,
                                                            surface,
                                                            width: phys_popup_w,
                                                            height: phys_popup_h,
                                                            back_buffer,
                                                            menu_id,
                                                            owner_win_idx: wi,
                                                            margin, // logical — used for hit-testing and render offset
                                                            dirty: true,
                                                            owner_dropdown: Some(target_id),
                                                            owner_combobox: None,
                                                            owner_autocomplete: None,
                                                            owner_text_edit: None,
                                                        });
                                                        // Popup keeps focus from create_window so it
                                                        // receives clicks/keys; clicking the main
                                                        // window dismisses via EVT_FOCUS_LOST.
                                                    }
                                                }
                                            }
                                        }

                                        if st.controls[idx2].kind() == ControlKind::ComboBox {
                                            let (should_open, items_text, cb_w, cb_h, cb_abs) =
                                                if let Some(cb) = control::cast_mut::<
                                                    crate::controls::combobox::ComboBox,
                                                >(
                                                    &mut st.controls[idx2],
                                                    ControlKind::ComboBox,
                                                ) {
                                                    let should_open = cb.request_popup || cb.open;
                                                    cb.request_popup = false;
                                                    cb.open = should_open;
                                                    (
                                                        should_open,
                                                        cb.popup_items(),
                                                        cb.text_base.base.w,
                                                        cb.text_base.base.h,
                                                        control::abs_position(
                                                            &st.controls,
                                                            target_id,
                                                        ),
                                                    )
                                                } else {
                                                    (false, alloc::vec::Vec::new(), 0, 0, (0, 0))
                                                };

                                            if should_open {
                                                dismiss_popup(st);

                                                if !items_text.is_empty() {
                                                    let menu_id = st.next_id;
                                                    st.next_id += 1;
                                                    let menu_ctrl = crate::controls::create_control(
                                                        ControlKind::ContextMenu,
                                                        menu_id,
                                                        0,
                                                        0,
                                                        0,
                                                        0,
                                                        0,
                                                        &items_text,
                                                    );
                                                    st.controls.push(menu_ctrl);

                                                    if let Some(mi) =
                                                        control::find_idx(&st.controls, menu_id)
                                                    {
                                                        let menu_w =
                                                            st.controls[mi].base().w.max(cb_w);
                                                        st.controls[mi].base_mut().w = menu_w;
                                                        let menu_h = st.controls[mi].base().h;
                                                        let margin: i32 = 16;
                                                        let popup_w = menu_w + (margin as u32) * 2;
                                                        let popup_h = menu_h + (margin as u32) * 2;
                                                        let phys_popup_w =
                                                            crate::theme::scale(popup_w);
                                                        let phys_popup_h =
                                                            crate::theme::scale(popup_h);
                                                        let (content_x, content_y) =
                                                            compositor::get_window_position(
                                                                st.channel_id,
                                                                st.sub_id,
                                                                comp_window_id,
                                                            );
                                                        let phys_cb_x =
                                                            crate::theme::scale_i32(cb_abs.0);
                                                        let phys_cb_y =
                                                            crate::theme::scale_i32(cb_abs.1);
                                                        let phys_cb_h = crate::theme::scale(cb_h);
                                                        let phys_margin =
                                                            crate::theme::scale_i32(margin);
                                                        let phys_menu_h =
                                                            crate::theme::scale(menu_h);
                                                        let mut popup_x =
                                                            content_x + phys_cb_x - phys_margin;
                                                        let mut popup_y = content_y
                                                            + phys_cb_y
                                                            + phys_cb_h as i32
                                                            - phys_margin;
                                                        let (scr_w, scr_h) =
                                                            compositor::screen_size();
                                                        if popup_x + phys_popup_w as i32
                                                            > scr_w as i32
                                                        {
                                                            popup_x =
                                                                scr_w as i32 - phys_popup_w as i32;
                                                        }
                                                        if popup_y + phys_popup_h as i32
                                                            > scr_h as i32
                                                        {
                                                            popup_y = content_y + phys_cb_y
                                                                - phys_menu_h as i32
                                                                - phys_margin;
                                                        }
                                                        if popup_x < 0 {
                                                            popup_x = 0;
                                                        }
                                                        if popup_y < 0 {
                                                            popup_y = 0;
                                                        }
                                                        let popup_flags: u32 =
                                                            0x01 | 0x02 | 0x04 | 0x100;
                                                        if let Some((
                                                            popup_win_id,
                                                            shm_id,
                                                            surface,
                                                        )) = compositor::create_window(
                                                            st.channel_id,
                                                            st.sub_id,
                                                            popup_x,
                                                            popup_y,
                                                            phys_popup_w,
                                                            phys_popup_h,
                                                            popup_flags,
                                                        ) {
                                                            st.controls[mi].set_position(0, 0);
                                                            st.controls[mi].base_mut().visible =
                                                                false;
                                                            let back_buffer = alloc::vec![
                                                                0u32;
                                                                (phys_popup_w * phys_popup_h) as usize
                                                            ];
                                                            st.popup = Some(crate::PopupInfo {
                                                                window_id: popup_win_id,
                                                                shm_id,
                                                                surface,
                                                                width: phys_popup_w,
                                                                height: phys_popup_h,
                                                                back_buffer,
                                                                menu_id,
                                                                owner_win_idx: wi,
                                                                margin,
                                                                dirty: true,
                                                                owner_dropdown: None,
                                                                owner_combobox: Some(target_id),
                                                                owner_autocomplete: None,
                                                                owner_text_edit: None,
                                                            });
                                                            // Popup keeps focus from create_window;
                                                            // EVT_FOCUS_LOST dismisses on outside click.
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        // RadioGroup: drain deferred deselection requests
                                        let radio_groups =
                                            crate::controls::radio_group::drain_deselects(
                                                &mut st.controls,
                                            );

                                        fire_event_callback(
                                            &st.controls,
                                            target_id,
                                            control::EVENT_CLICK,
                                            &mut pending_cbs,
                                        );

                                        if click_resp.fire_change {
                                            fire_event_callback(
                                                &st.controls,
                                                target_id,
                                                control::EVENT_CHANGE,
                                                &mut pending_cbs,
                                            );
                                        }

                                        // Fire EVENT_CHANGE on RadioGroup parents so on_selection_changed works
                                        for group_id in radio_groups {
                                            fire_event_callback(
                                                &st.controls,
                                                group_id,
                                                control::EVENT_CHANGE,
                                                &mut pending_cbs,
                                            );
                                        }

                                        if click_resp.fire_submit {
                                            fire_event_callback(
                                                &st.controls,
                                                target_id,
                                                control::EVENT_SUBMIT,
                                                &mut pending_cbs,
                                            );
                                        }

                                        // Multi-click detection (double & triple click)
                                        let now_ms = crate::syscall::uptime_ms();
                                        if st.last_click_id == Some(target_id)
                                            && now_ms.wrapping_sub(st.last_click_tick)
                                                <= DOUBLE_CLICK_MS
                                        {
                                            st.click_count += 1;
                                            st.last_click_tick = now_ms;

                                            if st.click_count == 2 {
                                                if let Some(idx3) =
                                                    control::find_idx(&st.controls, target_id)
                                                {
                                                    let dc_resp = st.controls[idx3]
                                                        .handle_double_click(
                                                            local_x, local_y, button,
                                                        );
                                                    // Only fire the double-click callback if the control
                                                    // didn't consume it internally (e.g. TabBar overflow buttons).
                                                    if !dc_resp.consumed {
                                                        fire_event_callback(
                                                            &st.controls,
                                                            target_id,
                                                            control::EVENT_DOUBLE_CLICK,
                                                            &mut pending_cbs,
                                                        );
                                                    }
                                                    if dc_resp.fire_change {
                                                        fire_event_callback(
                                                            &st.controls,
                                                            target_id,
                                                            control::EVENT_CHANGE,
                                                            &mut pending_cbs,
                                                        );
                                                    }
                                                    if dc_resp.fire_submit {
                                                        fire_event_callback(
                                                            &st.controls,
                                                            target_id,
                                                            control::EVENT_SUBMIT,
                                                            &mut pending_cbs,
                                                        );
                                                    }
                                                }
                                            } else if st.click_count >= 3 {
                                                if let Some(idx3) =
                                                    control::find_idx(&st.controls, target_id)
                                                {
                                                    let tc_resp = st.controls[idx3]
                                                        .handle_triple_click(
                                                            local_x, local_y, button,
                                                        );
                                                    if tc_resp.fire_change {
                                                        fire_event_callback(
                                                            &st.controls,
                                                            target_id,
                                                            control::EVENT_CHANGE,
                                                            &mut pending_cbs,
                                                        );
                                                    }
                                                }
                                                // Reset after triple click.
                                                st.click_count = 0;
                                                st.last_click_id = None;
                                            }
                                        } else {
                                            st.last_click_id = Some(target_id);
                                            st.last_click_tick = now_ms;
                                            st.click_count = 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    st.pressed_button = 0;
                }

                compositor::EVT_KEY_DOWN => {
                    // arg1=scancode, arg2=char_code, arg3=modifiers
                    let keycode = ev[2];
                    let char_code = ev[3];
                    let raw_modifiers = ev[4];

                    // Track modifier keys from their scancodes (safety net
                    // in case the event's modifier field is wrong/stale).
                    // PS/2: Left Ctrl = 0x1D, Right Ctrl = 0x1D (E0-prefixed
                    //   → different scancode after encode_scancode only for
                    //   nav keys; Ctrl passes through as 0x1D).
                    // VNC/HID: 'z' = 0x1D collision — but char_code will be
                    //   non-zero for letters, so we only set tracked_ctrl
                    //   when char_code == 0 (modifier-only key event).
                    if char_code == 0 {
                        if keycode == 0x1D {
                            // Left/Right Ctrl scancode
                            st.tracked_modifiers |= control::MOD_CTRL;
                        }
                        if keycode == 0x2A || keycode == 0x36 {
                            // Shift scancodes
                            st.tracked_modifiers |= control::MOD_SHIFT;
                        }
                    }

                    // Merge tracked modifiers with event-reported modifiers
                    let modifiers = raw_modifiers | st.tracked_modifiers;

                    // Store last key event info for queryable API
                    st.last_keycode = keycode;
                    st.last_char_code = char_code;
                    st.last_modifiers = modifiers;

                    let mut handled = false;

                    if let Some(focus_id) = st.focused {
                        if let Some(idx) = control::find_idx(&st.controls, focus_id) {
                            let resp =
                                st.controls[idx].handle_key_down(keycode, char_code, modifiers);
                            st.controls[idx].base_mut().mark_dirty();

                            if resp.consumed {
                                handled = true;
                                fire_event_callback(
                                    &st.controls,
                                    focus_id,
                                    control::EVENT_KEY,
                                    &mut pending_cbs,
                                );
                            }
                            if resp.fire_change {
                                fire_event_callback(
                                    &st.controls,
                                    focus_id,
                                    control::EVENT_CHANGE,
                                    &mut pending_cbs,
                                );
                            }
                            if resp.fire_click {
                                fire_event_callback(
                                    &st.controls,
                                    focus_id,
                                    control::EVENT_CLICK,
                                    &mut pending_cbs,
                                );
                            }
                            if resp.fire_submit {
                                // Don't fire submit if AutoComplete popup will handle Enter
                                let ac_popup_open = st.controls[idx].kind()
                                    == ControlKind::AutoCompleteTextField
                                    && st
                                        .popup
                                        .as_ref()
                                        .map(|p| p.owner_autocomplete == Some(focus_id))
                                        .unwrap_or(false);
                                if !ac_popup_open {
                                    fire_event_callback(
                                        &st.controls,
                                        focus_id,
                                        control::EVENT_SUBMIT,
                                        &mut pending_cbs,
                                    );
                                }
                            }
                        }
                    }

                    // ── ComboBox popup ─────────────────────────────────
                    if let Some(focus_id) = st.focused {
                        if let Some(idx) = control::find_idx(&st.controls, focus_id) {
                            let (triggered_popup, matches, cb_w, cb_h, nav, accept) =
                                if let Some(cb) = control::cast_mut::<
                                    crate::controls::combobox::ComboBox,
                                >(
                                    &mut st.controls[idx], ControlKind::ComboBox
                                ) {
                                    let nav = cb.popup_nav;
                                    cb.popup_nav = 0;
                                    let accept = cb.popup_accept;
                                    cb.popup_accept = false;
                                    let cb_w = cb.text_base.base.w;
                                    let cb_h = cb.text_base.base.h;
                                    if cb.request_popup {
                                        cb.request_popup = false;
                                        (true, cb.popup_items(), cb_w, cb_h, nav, accept)
                                    } else {
                                        (false, alloc::vec::Vec::new(), cb_w, cb_h, nav, accept)
                                    }
                                } else {
                                    (false, alloc::vec::Vec::new(), 0, 0, 0, false)
                                };

                            if triggered_popup {
                                if !matches.is_empty() {
                                    let cb_abs = control::abs_position(&st.controls, focus_id);
                                    dismiss_popup(st);
                                    let menu_id = st.next_id;
                                    st.next_id += 1;
                                    let menu_ctrl = crate::controls::create_control(
                                        ControlKind::ContextMenu,
                                        menu_id,
                                        0,
                                        0,
                                        0,
                                        0,
                                        0,
                                        &matches,
                                    );
                                    st.controls.push(menu_ctrl);

                                    if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                        let menu_w = st.controls[mi].base().w.max(cb_w);
                                        st.controls[mi].base_mut().w = menu_w;
                                        let menu_h = st.controls[mi].base().h;
                                        let margin: i32 = 16;
                                        let popup_w = menu_w + (margin as u32) * 2;
                                        let popup_h = menu_h + (margin as u32) * 2;
                                        let phys_popup_w = crate::theme::scale(popup_w);
                                        let phys_popup_h = crate::theme::scale(popup_h);
                                        let (content_x, content_y) =
                                            compositor::get_window_position(
                                                st.channel_id,
                                                st.sub_id,
                                                comp_window_id,
                                            );
                                        let phys_cb_x = crate::theme::scale_i32(cb_abs.0);
                                        let phys_cb_y = crate::theme::scale_i32(cb_abs.1);
                                        let phys_cb_h = crate::theme::scale(cb_h);
                                        let phys_margin = crate::theme::scale_i32(margin);
                                        let mut popup_x = content_x + phys_cb_x - phys_margin;
                                        let mut popup_y =
                                            content_y + phys_cb_y + phys_cb_h as i32 - phys_margin;
                                        let (scr_w, scr_h) = compositor::screen_size();
                                        if popup_x + phys_popup_w as i32 > scr_w as i32 {
                                            popup_x = scr_w as i32 - phys_popup_w as i32;
                                        }
                                        if popup_y + phys_popup_h as i32 > scr_h as i32 {
                                            let phys_menu_h = crate::theme::scale(menu_h);
                                            popup_y = content_y + phys_cb_y
                                                - phys_menu_h as i32
                                                - phys_margin;
                                        }
                                        if popup_x < 0 {
                                            popup_x = 0;
                                        }
                                        if popup_y < 0 {
                                            popup_y = 0;
                                        }
                                        let popup_flags: u32 = 0x01 | 0x02 | 0x04 | 0x100;
                                        if let Some((popup_win_id, shm_id, surface)) =
                                            compositor::create_window(
                                                st.channel_id,
                                                st.sub_id,
                                                popup_x,
                                                popup_y,
                                                phys_popup_w,
                                                phys_popup_h,
                                                popup_flags,
                                            )
                                        {
                                            st.controls[mi].set_position(0, 0);
                                            st.controls[mi].base_mut().visible = false;
                                            let back_buffer = alloc::vec![0u32; (phys_popup_w * phys_popup_h) as usize];
                                            st.popup = Some(crate::PopupInfo {
                                                window_id: popup_win_id,
                                                shm_id,
                                                surface,
                                                width: phys_popup_w,
                                                height: phys_popup_h,
                                                back_buffer,
                                                menu_id,
                                                owner_win_idx: wi,
                                                margin,
                                                dirty: true,
                                                owner_dropdown: None,
                                                owner_combobox: Some(focus_id),
                                                owner_autocomplete: None,
                                                owner_text_edit: None,
                                            });
                                            // Popup keeps focus from create_window;
                                            // EVT_FOCUS_LOST dismisses on outside click.
                                        }
                                    }
                                } else if st
                                    .popup
                                    .as_ref()
                                    .map(|p| p.owner_combobox == Some(focus_id))
                                    .unwrap_or(false)
                                {
                                    dismiss_popup(st);
                                }
                            }

                            if nav != 0 {
                                if let Some(ref popup) = st.popup {
                                    if popup.owner_combobox == Some(focus_id) {
                                        let menu_id = popup.menu_id;
                                        if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                            let item_count = st.controls[mi]
                                                .text_base()
                                                .map(|tb| tb.text.split(|&b| b == b'|').count())
                                                .unwrap_or(0)
                                                as i32;
                                            if item_count > 0 {
                                                let cur = st.controls[mi].base().state as i32;
                                                let next = if nav > 0 {
                                                    (cur + 1).min(item_count - 1)
                                                } else {
                                                    (cur - 1).max(0)
                                                };
                                                st.controls[mi].base_mut().state = next as u32;
                                                st.controls[mi].base_mut().mark_dirty();
                                                if let Some(ref mut p) = st.popup {
                                                    p.dirty = true;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if accept {
                                let has_cb_popup = st
                                    .popup
                                    .as_ref()
                                    .map(|p| p.owner_combobox == Some(focus_id))
                                    .unwrap_or(false);
                                if has_cb_popup {
                                    let menu_id = st.popup.as_ref().unwrap().menu_id;
                                    if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                        let selected_idx = st.controls[mi].base().state as usize;
                                        let menu_text = st.controls[mi]
                                            .text_base()
                                            .map(|tb| tb.text.clone())
                                            .unwrap_or_default();
                                        let full_item: alloc::vec::Vec<u8> = menu_text
                                            .split(|&b| b == b'|')
                                            .nth(selected_idx)
                                            .unwrap_or(&[])
                                            .to_vec();
                                        let actual_idx = full_item
                                            .iter()
                                            .position(|&b| b == 0x1F)
                                            .and_then(|sep| {
                                                core::str::from_utf8(&full_item[..sep]).ok()
                                            })
                                            .and_then(|s| s.parse::<usize>().ok());
                                        dismiss_popup(st);
                                        if let Some(actual_idx) = actual_idx {
                                            if let Some(idx2) =
                                                control::find_idx(&st.controls, focus_id)
                                            {
                                                if let Some(cb2) = control::cast_mut::<
                                                    crate::controls::combobox::ComboBox,
                                                >(
                                                    &mut st.controls[idx2],
                                                    ControlKind::ComboBox,
                                                ) {
                                                    cb2.apply_selected_index(actual_idx);
                                                }
                                            }
                                            fire_event_callback(
                                                &st.controls,
                                                focus_id,
                                                control::EVENT_CHANGE,
                                                &mut pending_cbs,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── AutoComplete popup ────────────────────────────
                    // After key handling, check if the focused control is an
                    // AutoCompleteTextField with `suggest == true`.
                    if let Some(focus_id) = st.focused {
                        if let Some(idx) = control::find_idx(&st.controls, focus_id) {
                            let (triggered_suggest, matches, ac_w, ac_h, nav, accept) =
                                if let Some(ac) = control::cast_mut::<
                                    crate::controls::autocomplete_textfield::AutoCompleteTextField,
                                >(
                                    &mut st.controls[idx],
                                    ControlKind::AutoCompleteTextField,
                                ) {
                                    let nav = ac.popup_nav;
                                    ac.popup_nav = 0;
                                    let accept = ac.popup_accept;
                                    ac.popup_accept = false;
                                    let ac_w = ac.text_base.base.w;
                                    let ac_h = ac.text_base.base.h;
                                    if ac.suggest {
                                        ac.suggest = false;
                                        (true, ac.filtered_items(), ac_w, ac_h, nav, accept)
                                    } else {
                                        (false, alloc::vec::Vec::new(), ac_w, ac_h, nav, accept)
                                    }
                                } else {
                                    (false, alloc::vec::Vec::new(), 0, 0, 0, false)
                                };

                            if triggered_suggest {
                                if !matches.is_empty() {
                                    let ac_abs = control::abs_position(&st.controls, focus_id);

                                    dismiss_popup(st);

                                    let menu_id = st.next_id;
                                    st.next_id += 1;
                                    let menu_ctrl = crate::controls::create_control(
                                        ControlKind::ContextMenu,
                                        menu_id,
                                        0,
                                        0,
                                        0,
                                        0,
                                        0,
                                        &matches,
                                    );
                                    st.controls.push(menu_ctrl);

                                    if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                        let menu_w = st.controls[mi].base().w.max(ac_w);
                                        st.controls[mi].base_mut().w = menu_w;
                                        let menu_h = st.controls[mi].base().h;

                                        let margin: i32 = 16;
                                        let popup_w = menu_w + (margin as u32) * 2;
                                        let popup_h = menu_h + (margin as u32) * 2;
                                        let phys_popup_w = crate::theme::scale(popup_w);
                                        let phys_popup_h = crate::theme::scale(popup_h);

                                        let (content_x, content_y) =
                                            compositor::get_window_position(
                                                st.channel_id,
                                                st.sub_id,
                                                comp_window_id,
                                            );
                                        let phys_ac_x = crate::theme::scale_i32(ac_abs.0);
                                        let phys_ac_y = crate::theme::scale_i32(ac_abs.1);
                                        let phys_ac_h = crate::theme::scale(ac_h);
                                        let phys_margin = crate::theme::scale_i32(margin);
                                        let mut popup_x = content_x + phys_ac_x - phys_margin;
                                        let mut popup_y =
                                            content_y + phys_ac_y + phys_ac_h as i32 - phys_margin;

                                        let (scr_w, scr_h) = compositor::screen_size();
                                        if popup_x + phys_popup_w as i32 > scr_w as i32 {
                                            popup_x = scr_w as i32 - phys_popup_w as i32;
                                        }
                                        if popup_y + phys_popup_h as i32 > scr_h as i32 {
                                            let phys_menu_h = crate::theme::scale(menu_h);
                                            popup_y = content_y + phys_ac_y
                                                - phys_menu_h as i32
                                                - phys_margin;
                                        }
                                        if popup_x < 0 {
                                            popup_x = 0;
                                        }
                                        if popup_y < 0 {
                                            popup_y = 0;
                                        }

                                        let popup_flags: u32 = 0x01 | 0x02 | 0x04 | 0x100;
                                        if let Some((popup_win_id, shm_id, surface)) =
                                            compositor::create_window(
                                                st.channel_id,
                                                st.sub_id,
                                                popup_x,
                                                popup_y,
                                                phys_popup_w,
                                                phys_popup_h,
                                                popup_flags,
                                            )
                                        {
                                            st.controls[mi].set_position(0, 0);
                                            st.controls[mi].base_mut().visible = false;
                                            let back_buffer = alloc::vec![0u32; (phys_popup_w * phys_popup_h) as usize];
                                            st.popup = Some(crate::PopupInfo {
                                                window_id: popup_win_id,
                                                shm_id,
                                                surface,
                                                width: phys_popup_w,
                                                height: phys_popup_h,
                                                back_buffer,
                                                menu_id,
                                                owner_win_idx: wi,
                                                margin,
                                                dirty: true,
                                                owner_dropdown: None,
                                                owner_combobox: None,
                                                owner_autocomplete: Some(focus_id),
                                                owner_text_edit: None,
                                            });
                                            let tid = libsyscall::get_tid();
                                            compositor::focus_by_tid(st.channel_id, tid);
                                        }
                                    }
                                } else if st
                                    .popup
                                    .as_ref()
                                    .map(|p| p.owner_autocomplete == Some(focus_id))
                                    .unwrap_or(false)
                                {
                                    dismiss_popup(st);
                                }
                            }

                            if nav != 0 {
                                if let Some(ref popup) = st.popup {
                                    if popup.owner_autocomplete == Some(focus_id) {
                                        let menu_id = popup.menu_id;
                                        if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                            let item_count = st.controls[mi]
                                                .text_base()
                                                .map(|tb| tb.text.split(|&b| b == b'|').count())
                                                .unwrap_or(0)
                                                as i32;
                                            let cur = st.controls[mi].base().state as i32;
                                            let next = if nav > 0 {
                                                (cur + 1).min(item_count - 1)
                                            } else {
                                                (cur - 1).max(0)
                                            };
                                            st.controls[mi].base_mut().state = next as u32;
                                            st.controls[mi].base_mut().mark_dirty();
                                            if let Some(ref mut p) = st.popup {
                                                p.dirty = true;
                                            }
                                        }
                                    }
                                }
                            }

                            if accept {
                                let has_ac_popup = st
                                    .popup
                                    .as_ref()
                                    .map(|p| p.owner_autocomplete == Some(focus_id))
                                    .unwrap_or(false);
                                if has_ac_popup {
                                    let menu_id = st.popup.as_ref().unwrap().menu_id;
                                    if let Some(mi) = control::find_idx(&st.controls, menu_id) {
                                        let selected_idx = st.controls[mi].base().state as usize;
                                        let menu_text = st.controls[mi]
                                            .text_base()
                                            .map(|tb| tb.text.clone())
                                            .unwrap_or_default();
                                        let full_item: alloc::vec::Vec<u8> = menu_text
                                            .split(|&b| b == b'|')
                                            .nth(selected_idx)
                                            .unwrap_or(&[])
                                            .to_vec();
                                        let label = if let Some(sep) =
                                            full_item.iter().position(|&b| b == 0x1F)
                                        {
                                            full_item[sep + 1..].to_vec()
                                        } else {
                                            full_item
                                        };
                                        dismiss_popup(st);
                                        if !label.is_empty() {
                                            if let Some(idx2) =
                                                control::find_idx(&st.controls, focus_id)
                                            {
                                                if let Some(ac2) = control::cast_mut::<crate::controls::autocomplete_textfield::AutoCompleteTextField>(
                                                    &mut st.controls[idx2],
                                                    ControlKind::AutoCompleteTextField,
                                                ) {
                                                    ac2.text_base.text = label;
                                                    ac2.cursor_pos = ac2.text_base.text.len();
                                                    ac2.sel_anchor = ac2.cursor_pos;
                                                    ac2.suggest = false;
                                                    ac2.text_base.base.mark_dirty();
                                                }
                                            }
                                            fire_event_callback(
                                                &st.controls,
                                                focus_id,
                                                control::EVENT_SUBMIT,
                                                &mut pending_cbs,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !handled {
                        // Tab / Shift+Tab: cycle focus forward / backward
                        if keycode == control::KEY_TAB {
                            let reverse = modifiers & control::MOD_SHIFT != 0;
                            cycle_focus(st, win_id, &mut pending_cbs, reverse);
                        }
                        // Always bubble unhandled key events to the window
                        fire_event_callback(
                            &st.controls,
                            win_id,
                            control::EVENT_KEY,
                            &mut pending_cbs,
                        );
                    }
                }

                compositor::EVT_KEY_UP => {
                    // Track modifier key releases to clear tracked_modifiers.
                    let keycode = ev[2];
                    let char_code = ev[3];
                    if char_code == 0 {
                        if keycode == 0x1D {
                            st.tracked_modifiers &= !control::MOD_CTRL;
                        }
                        if keycode == 0x2A || keycode == 0x36 {
                            st.tracked_modifiers &= !control::MOD_SHIFT;
                        }
                    }

                    // Store key info for the callback
                    st.last_keycode = keycode;
                    st.last_char_code = char_code;
                    st.last_modifiers = ev[4] | st.tracked_modifiers;

                    // Dispatch KEY_UP to the window
                    fire_event_callback(
                        &st.controls,
                        win_id,
                        control::EVENT_KEY_UP,
                        &mut pending_cbs,
                    );
                }

                compositor::EVT_MOUSE_SCROLL => {
                    // arg1=dz (signed), arg2=modifiers, arg3=0
                    let dz = ev[2] as i32;
                    let modifiers = ev[3];
                    st.last_modifiers = modifiers;

                    // Shift + wheel → horizontal scroll. Bubble up from
                    // the hovered control until we find a ScrollView and
                    // shift its scroll_x. We bypass handle_scroll() (which
                    // is the trait's vertical-only path) because changing
                    // the trait signature would touch every control.
                    let mut consumed = false;
                    if (modifiers & control::MOD_SHIFT) != 0 {
                        if let Some(target_id) = st.hovered {
                            let mut cur = target_id;
                            loop {
                                if let Some(idx) = control::find_idx(&st.controls, cur) {
                                    if st.controls[idx].kind() == control::ControlKind::ScrollView {
                                        if let Some(sv) = control::cast_mut::<
                                            crate::controls::scroll_view::ScrollView,
                                        >(
                                            &mut st.controls[idx],
                                            control::ControlKind::ScrollView,
                                        ) {
                                            // Same step size as vertical (dz * 20).
                                            // Negate so wheel-up scrolls left, matching
                                            // the vertical convention (wheel-up → up).
                                            if sv.apply_scroll_delta_x(-dz * 20) {
                                                fire_event_callback(
                                                    &st.controls,
                                                    cur,
                                                    control::EVENT_SCROLL,
                                                    &mut pending_cbs,
                                                );
                                                fire_event_callback(
                                                    &st.controls,
                                                    cur,
                                                    control::EVENT_CHANGE,
                                                    &mut pending_cbs,
                                                );
                                            }
                                            consumed = true;
                                            break;
                                        }
                                    }
                                    let parent = st.controls[idx].parent_id();
                                    if parent == 0 || parent == cur {
                                        break;
                                    }
                                    cur = parent;
                                } else {
                                    break;
                                }
                            }
                        }
                    }

                    // Dispatch to hovered control, bubbling up to ScrollView if needed
                    if !consumed {
                        if let Some(target_id) = st.hovered {
                            let mut cur = target_id;
                            loop {
                                if let Some(idx) = control::find_idx(&st.controls, cur) {
                                    let resp = st.controls[idx].handle_scroll(dz);
                                    if resp.consumed {
                                        st.controls[idx].base_mut().mark_dirty();
                                        fire_event_callback(
                                            &st.controls,
                                            cur,
                                            control::EVENT_SCROLL,
                                            &mut pending_cbs,
                                        );
                                        if resp.fire_change {
                                            fire_event_callback(
                                                &st.controls,
                                                cur,
                                                control::EVENT_CHANGE,
                                                &mut pending_cbs,
                                            );
                                        }
                                        consumed = true;
                                        break;
                                    }
                                    // Bubble up to parent
                                    let parent = st.controls[idx].parent_id();
                                    if parent == 0 || parent == cur {
                                        break;
                                    }
                                    cur = parent;
                                } else {
                                    break;
                                }
                            }
                        }
                    } // end `if !consumed` for shift-aware horizontal path

                    // If scroll was not consumed (e.g. Canvas), still fire
                    // EVENT_SCROLL on the hovered control so apps that
                    // explicitly subscribe via on_scroll_raw receive the
                    // event. Stash the dz on Canvas controls so they can
                    // read the direction via anyui_canvas_get_wheel.
                    // For backwards-compatibility we ALSO synthesise a
                    // mouse_down with button 2 / 3, which is what older
                    // apps (e.g. surf) listen for.
                    if !consumed {
                        if let Some(target_id) = st.hovered {
                            if let Some(idx) = control::find_idx(&st.controls, target_id) {
                                // Stash the dz on Canvas controls so apps
                                // can read scroll direction via the
                                // anyui_canvas_get_wheel export.
                                if st.controls[idx].kind() == control::ControlKind::Canvas {
                                    if let Some(cv) =
                                        control::cast_mut::<crate::controls::canvas::Canvas>(
                                            &mut st.controls[idx],
                                            control::ControlKind::Canvas,
                                        )
                                    {
                                        cv.last_wheel_dz = dz;
                                    }
                                }
                                fire_event_callback(
                                    &st.controls,
                                    target_id,
                                    control::EVENT_SCROLL,
                                    &mut pending_cbs,
                                );
                                let button = if dz < 0 { 2u32 } else { 3u32 };
                                let (ax, ay) = control::abs_position(&st.controls, target_id);
                                let local_x = st.last_mouse_x - ax;
                                let local_y = st.last_mouse_y - ay;
                                st.controls[idx].handle_mouse_down(local_x, local_y, button);
                                st.controls[idx].base_mut().mark_dirty();
                                fire_event_callback(
                                    &st.controls,
                                    target_id,
                                    control::EVENT_MOUSE_DOWN,
                                    &mut pending_cbs,
                                );
                            }
                        }
                    }
                }

                compositor::EVT_RESIZE => {
                    // arg1=new_w, arg2=new_h — physical pixels from compositor.
                    let phys_w = ev[2];
                    let phys_h = ev[3];
                    // Convert to logical for the control tree.
                    let logical_w = crate::theme::unscale_u32(phys_w);
                    let logical_h = crate::theme::unscale_u32(phys_h);
                    // Resize the SHM buffer at physical dimensions.
                    if wi < st.comp_windows.len() {
                        let cw = &mut st.comp_windows[wi];
                        if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
                            st.channel_id,
                            cw.window_id,
                            cw.shm_id,
                            phys_w,
                            phys_h,
                        ) {
                            cw.shm_id = new_shm_id;
                            cw.surface = new_surface;
                        }
                        cw.width = phys_w;
                        cw.height = phys_h;
                        cw.logical_width = logical_w;
                        cw.logical_height = logical_h;
                        // Resize back buffer at physical dimensions.
                        let new_count = (phys_w as usize) * (phys_h as usize);
                        cw.back_buffer.resize(new_count, 0);
                    }
                    if let Some(idx) = control::find_idx(&st.controls, win_id) {
                        // Control tree uses logical dimensions.
                        st.controls[idx].set_size(logical_w, logical_h);
                        // Run layout BEFORE the resize callback so that
                        // children (e.g. DOCK_FILL canvas) already have
                        // their updated sizes when app code queries them.
                        crate::layout::perform_layout(&mut st.controls, win_id);
                        fire_event_callback(
                            &st.controls,
                            win_id,
                            control::EVENT_RESIZE,
                            &mut pending_cbs,
                        );
                    }
                    st.needs_layout = true;
                }

                compositor::EVT_WINDOW_MOVED => {
                    // Compositor moved the window's frame (drag-end, "move
                    // to other monitor" menubar button, maximize/restore,
                    // tile, snap, …). Cache the new frame position on the
                    // CompWindow so libanyui_client's
                    // `Window::get_position()` returns the truth instead
                    // of the stale spawn coords.
                    //
                    // IMPORTANT: do NOT write into `controls[Window].base().x/y`.
                    // `abs_position` walks up the parent chain and adds
                    // those coords to every descendant control, so a
                    // non-zero Window x/y would shift all hit-tests and
                    // child layout origins by that offset (visible as
                    // wrongly-positioned panels until the next full
                    // relayout). Frame position is purely a compositor-
                    // owned property; the control tree stays at (0,0).
                    let phys_x = ev[2] as i32;
                    let phys_y = ev[3] as i32;
                    if wi < st.comp_windows.len() {
                        st.comp_windows[wi].frame_x = phys_x;
                        st.comp_windows[wi].frame_y = phys_y;
                    }
                }

                compositor::EVT_FULLSCREEN_ENTER => {
                    // Fullscreen entered: ev[2] = (width<<16)|height, ev[3] = stride, ev[4] = fb_ptr
                    crate::FULLSCREEN_INFO.store(
                        ev[2] as u64 | ((ev[3] as u64) << 32),
                        core::sync::atomic::Ordering::Relaxed,
                    );
                    crate::FULLSCREEN_FB_PTR.store(ev[4], core::sync::atomic::Ordering::Relaxed);

                    // Resize window SHM to fullscreen dimensions
                    let fs_w = (ev[2] >> 16) & 0xFFFF;
                    let fs_h = ev[2] & 0xFFFF;
                    if wi < st.comp_windows.len() && fs_w > 0 && fs_h > 0 {
                        let cw = &mut st.comp_windows[wi];
                        // Save original logical size for restore on exit
                        cw.saved_logical_size_fs = Some((cw.logical_width, cw.logical_height));
                        let logical_w = crate::theme::unscale_u32(fs_w);
                        let logical_h = crate::theme::unscale_u32(fs_h);
                        if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
                            st.channel_id,
                            cw.window_id,
                            cw.shm_id,
                            fs_w,
                            fs_h,
                        ) {
                            cw.shm_id = new_shm_id;
                            cw.surface = new_surface;
                        }
                        cw.width = fs_w;
                        cw.height = fs_h;
                        cw.logical_width = logical_w;
                        cw.logical_height = logical_h;
                        let new_count = (fs_w as usize) * (fs_h as usize);
                        cw.back_buffer.resize(new_count, 0);
                        cw.dirty = true;
                        cw.dirty_rect = None;
                        // Update control tree size and re-layout
                        if let Some(idx) = control::find_idx(&st.controls, win_id) {
                            st.controls[idx].set_size(logical_w, logical_h);
                        }
                        crate::layout::perform_layout(&mut st.controls, win_id);
                    }

                    fire_event_callback(
                        &st.controls,
                        win_id,
                        control::EVENT_FULLSCREEN_ENTER,
                        &mut pending_cbs,
                    );
                }

                compositor::EVT_FULLSCREEN_EXIT => {
                    crate::FULLSCREEN_INFO.store(0, core::sync::atomic::Ordering::Relaxed);
                    crate::FULLSCREEN_FB_PTR.store(0, core::sync::atomic::Ordering::Relaxed);

                    // Restore window SHM to original size
                    if wi < st.comp_windows.len() {
                        let cw = &mut st.comp_windows[wi];
                        if let Some((orig_lw, orig_lh)) = cw.saved_logical_size_fs.take() {
                            let phys_w = crate::theme::scale(orig_lw);
                            let phys_h = crate::theme::scale(orig_lh);
                            if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
                                st.channel_id,
                                cw.window_id,
                                cw.shm_id,
                                phys_w,
                                phys_h,
                            ) {
                                cw.shm_id = new_shm_id;
                                cw.surface = new_surface;
                            }
                            cw.width = phys_w;
                            cw.height = phys_h;
                            cw.logical_width = orig_lw;
                            cw.logical_height = orig_lh;
                            let new_count = (phys_w as usize) * (phys_h as usize);
                            cw.back_buffer.resize(new_count, 0);
                            cw.present_pending = false;
                            cw.pending_present_rect = None;
                            cw.dirty = true;
                            cw.dirty_rect = None;
                            // Update control tree size and re-layout
                            if let Some(idx) = control::find_idx(&st.controls, win_id) {
                                st.controls[idx].set_size(orig_lw, orig_lh);
                            }
                            crate::layout::perform_layout(&mut st.controls, win_id);
                        }
                    }

                    fire_event_callback(
                        &st.controls,
                        win_id,
                        control::EVENT_FULLSCREEN_EXIT,
                        &mut pending_cbs,
                    );
                }

                compositor::EVT_FRAME_ACK => {
                    // VSync callback: compositor has composited our frame to screen.
                    // Clear back-pressure so we can present the next frame.
                    if wi < st.comp_windows.len() {
                        st.comp_windows[wi].frame_presented = false;
                    }
                }

                compositor::EVT_MENU_ITEM => {
                    // ev[2] = menu_index, ev[3] = item_id
                    let item_id = ev[3];
                    if let Some(&(_, cb, ud)) = st.menu_callbacks.iter().find(|e| e.0 == win_id) {
                        pending_cbs.push(PendingCallback {
                            id: item_id,
                            event_type: compositor::EVT_MENU_ITEM,
                            cb,
                            userdata: ud,
                        });
                    }
                }

                compositor::EVT_DRAG_ENTER => {
                    // [EVT, target_window_id, format, payload_shm_id, packed_meta]
                    let format = ev[2];
                    let payload_shm_id = ev[3];
                    let packed_meta = ev[4];
                    let allowed_effects = packed_meta & 0xFF;
                    let source_tid = (packed_meta >> 8) & 0x00FF_FFFF;
                    // Map the payload SHM read-only into our address space.
                    let addr = crate::syscall::shm_map(payload_shm_id) as u32;
                    st.incoming_drag = Some(crate::IncomingDrag {
                        window_id: win_id,
                        comp_window_id,
                        payload_shm_id,
                        payload_addr: addr,
                        payload_len: 0, // updated on first OVER
                        format,
                        allowed_effects,
                        source_tid,
                        target_control: None,
                        target_accepted: false,
                        negotiated_effect: 0,
                        pointer_x: 0,
                        pointer_y: 0,
                        modifiers: 0,
                        accept_modifiers: 0,
                    });
                }

                compositor::EVT_DRAG_OVER => {
                    // Wire: [type, target_window_id, payload_len, packed_xy, packed_mod_eff]
                    let payload_len = ev[2];
                    let packed_xy = ev[3];
                    let packed_mod_eff = ev[4];
                    let lx = (packed_xy >> 16) as i32;
                    let ly = (packed_xy & 0xFFFF) as i32;
                    let modifiers = packed_mod_eff & 0xFF;
                    if let Some(inc) = st.incoming_drag.as_mut() {
                        inc.payload_len = payload_len;
                    }
                    dispatch_drag_over(st, win_id, lx, ly, modifiers, &mut pending_cbs);
                }

                compositor::EVT_DRAG_LEAVE => {
                    if let Some(target_id) =
                        st.incoming_drag.as_ref().and_then(|i| i.target_control)
                    {
                        set_drop_hover(&mut st.controls, target_id, false);
                        fire_event_callback(
                            &st.controls,
                            target_id,
                            control::EVENT_DRAG_LEAVE,
                            &mut pending_cbs,
                        );
                    }
                    if let Some(inc) = st.incoming_drag.as_mut() {
                        inc.target_control = None;
                        inc.target_accepted = false;
                        inc.negotiated_effect = 0;
                    }
                    // Don't drop the SHM mapping here — the drag may re-enter
                    // this window. The mapping is released on EVT_DROP, on
                    // EVT_DRAG_END (when the source-side cleanup fires for
                    // the same app), or when a new EVT_DRAG_ENTER arrives.
                }

                compositor::EVT_DROP => {
                    // [EVT, target_window_id, packed_xy, negotiated_effect, source_tid]
                    let packed_xy = ev[2];
                    let negotiated_effect = ev[3];
                    let lx = (packed_xy >> 16) as i32;
                    let ly = (packed_xy & 0xFFFF) as i32;
                    let target_control = st.incoming_drag.as_ref().and_then(|i| i.target_control);
                    if let Some(inc) = st.incoming_drag.as_mut() {
                        inc.pointer_x = lx;
                        inc.pointer_y = ly;
                        inc.negotiated_effect = negotiated_effect;
                        inc.target_accepted = negotiated_effect != 0;
                    }
                    if let Some(target_id) = target_control {
                        set_drop_hover(&mut st.controls, target_id, false);
                        if negotiated_effect != 0 {
                            fire_event_callback(
                                &st.controls,
                                target_id,
                                control::EVENT_DROP,
                                &mut pending_cbs,
                            );
                        }
                        fire_event_callback(
                            &st.controls,
                            target_id,
                            control::EVENT_DRAG_LEAVE,
                            &mut pending_cbs,
                        );
                    }
                    // Defer SHM unmap + state teardown until after Phase 3
                    // so the DROP callback can read the payload.
                    st.incoming_release_pending = true;
                }

                compositor::EVT_DRAG_FEEDBACK => {
                    // Source-side: [EVT, src_window_id, target_present, negotiated_effect, 0]
                    // Adjust the cursor shape to mirror the negotiated effect.
                    let target_present = ev[2] != 0;
                    let effect = ev[3];
                    let new_shape: u32 = if !target_present {
                        // Cursor over no drop target — show No-Drop badge.
                        8
                    } else if effect & 0x02 != 0 {
                        5 // Move
                    } else if effect & 0x01 != 0 {
                        6 // Copy
                    } else if effect & 0x04 != 0 {
                        7 // Link
                    } else {
                        // Target present but acceptance not yet negotiated
                        // (transient — accept callback queued in pending_cbs
                        // but hasn't fired yet). Keep showing Move so we
                        // don't flicker the cursor every time the target
                        // changes; if the target eventually rejects we'll
                        // get FEEDBACK[present=0] and switch to NoDrop.
                        5
                    };
                    if st.current_cursor != new_shape {
                        st.current_cursor = new_shape;
                        let cmd: [u32; 5] = [0x1018, comp_window_id, new_shape, 0, 0];
                        crate::syscall::evt_chan_emit(st.channel_id, &cmd);
                    }
                }

                compositor::EVT_DRAG_END => {
                    // Source-side: [EVT, src_window_id, completed, negotiated_effect, 0]
                    // The drag ended (either by drop or cancel). Tear down
                    // the source-side session and the SHM bridge.
                    if let Some(drag) = st.drag.as_ref() {
                        let source_id = drag.source_id;
                        fire_event_callback(
                            &st.controls,
                            source_id,
                            control::EVENT_DRAG_END,
                            &mut pending_cbs,
                        );
                    }
                    if st.current_cursor != 0 {
                        st.current_cursor = 0;
                        let cmd: [u32; 5] = [0x1018, comp_window_id, 0, 0, 0];
                        crate::syscall::evt_chan_emit(st.channel_id, &cmd);
                    }
                    st.pressed = None;
                    st.pressed_button = 0;
                    // Defer drag teardown until after callbacks fire.
                    st.drag_release_pending = true;
                }

                _ => {}
            }
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

    // ── Phase 2.9: Accessibility pipe — poll & inject ──────────────
    {
        let st = crate::state();
        let mut acc_events: alloc::vec::Vec<(crate::control::ControlId, u32)> =
            alloc::vec::Vec::new();
        // Temporarily take `acc` out of `st` to satisfy the borrow checker:
        // poll_and_handle needs &mut AccState (from st.acc) AND &mut AnyuiState.
        if let Some(mut acc) = st.acc.take() {
            crate::accessibility::poll_and_handle(&mut acc, st, &mut acc_events);
            st.acc = Some(acc);
        }
        // Fire each queued event (CLICK, SUBMIT, …) through the normal callback path.
        for (id, event_type) in acc_events {
            fire_event_callback(&st.controls, id, event_type, &mut pending_cbs);
        }
    }

    // ── Phase 3: Invoke callbacks (no borrows held) ────────────────
    for pcb in pending_cbs {
        (pcb.cb)(pcb.id, pcb.event_type, pcb.userdata);
    }

    // Tear down a finished drag session now that DROP / DRAG_END callbacks
    // had a chance to read the payload via anyui_drag_get_payload.
    {
        let st = crate::state();
        if st.incoming_release_pending {
            if let Some(inc) = st.incoming_drag.take() {
                if inc.payload_addr != 0 {
                    crate::syscall::shm_unmap(inc.payload_shm_id);
                }
            }
            st.incoming_release_pending = false;
        }
        if st.drag_release_pending {
            // Source-side: release the bridge SHM (unmap + destroy).
            crate::dnd_ipc::release_bridge(st);
            st.drag = None;
            st.drag_release_pending = false;
        }
    }

    // Re-acquire state (callbacks may have modified it)
    let st = crate::state();
    if st.quit_requested || st.windows.is_empty() {
        return 0;
    }

    // ── Phase 3.5: Layout (skipped when no layout-affecting changes) ──
    if st.needs_layout {
        for wi in 0..st.windows.len() {
            let win_id = st.windows[wi];
            crate::layout::perform_layout(&mut st.controls, win_id);
        }

        // Phase 3.6: Update scroll bounds (only after layout)
        crate::controls::scroll_view::update_scroll_bounds(&mut st.controls);

        st.needs_layout = false;
    }

    // ── Phase 3.7: Compute per-window dirty flags + dirty rects ─────
    // Push-based: only scan when mark_dirty() was called since last render.
    // On idle frames (no events, no timers), this entire phase is skipped.
    // Walks the control tree to compute absolute positions and union dirty
    // rects — enabling selective rendering in Phase 4 (only controls that
    // intersect the dirty rect are re-rendered).
    if st.needs_repaint {
        for cw in st.comp_windows.iter_mut() {
            cw.dirty = false;
            cw.dirty_rect = None;
        }
        for wi in 0..st.windows.len() {
            let win_id = st.windows[wi];
            collect_dirty_rects(&st.controls, win_id, 0, 0, &mut st.comp_windows[wi]);
        }
        st.needs_repaint = false;
    }

    // ── Phase 4: Render dirty windows (with VSync back-pressure) ───
    // Incremental rendering: only re-render controls that intersect the dirty
    // rect, copy only the dirty region to SHM, and tell the compositor which
    // rect changed. For typical interactions (hover, click, typing) this is
    // 50-500x faster than a full-window redraw.
    let channel_id = st.channel_id;
    for wi in 0..st.windows.len() {
        let win_id = st.windows[wi];

        let now_ms = crate::syscall::uptime_ms();
        if st.comp_windows[wi].frame_presented
            && now_ms.wrapping_sub(st.comp_windows[wi].last_present_ms) > FRAME_ACK_TIMEOUT_MS
        {
            st.comp_windows[wi].frame_presented = false;
        }

        let frame_presented = st.comp_windows[wi].frame_presented;
        let needs_render = st.comp_windows[wi].dirty;
        let has_pending_present = st.comp_windows[wi].present_pending;

        // No new rendering work and nothing staged for upload.
        if !needs_render && !has_pending_present {
            continue;
        }

        let surface_ptr = st.comp_windows[wi].surface;
        let sw = st.comp_windows[wi].width;
        let sh = st.comp_windows[wi].height;
        let comp_window_id = st.comp_windows[wi].window_id;
        let shm_id = st.comp_windows[wi].shm_id;
        let dirty_rect = st.comp_windows[wi].dirty_rect;
        let logical_w = st.comp_windows[wi].logical_width;
        let logical_h = st.comp_windows[wi].logical_height;
        let mut staged_rect = st.comp_windows[wi].pending_present_rect;

        if needs_render {
            // Clamp dirty rect in logical space (for render_tree intersection tests)
            let logical_dr = dirty_rect
                .map(|(dx, dy, dw, dh)| {
                    let x0 = dx.max(0) as u32;
                    let y0 = dy.max(0) as u32;
                    let x1 = ((dx + dw as i32).max(0) as u32).min(logical_w);
                    let y1 = ((dy + dh as i32).max(0) as u32).min(logical_h);
                    (
                        x0 as i32,
                        y0 as i32,
                        x1.saturating_sub(x0),
                        y1.saturating_sub(y0),
                    )
                })
                .filter(|&(_, _, w, h)| w > 0 && h > 0);

            // Double-buffered rendering: draw to a local back buffer first.
            let back_buf = st.comp_windows[wi].back_buffer.as_mut_ptr();
            let full_surf = crate::draw::Surface::new(back_buf, sw, sh);

            if let Some(scroll_damage) = find_scroll_blit_damage(&st.controls, win_id) {
                let mut scroll_present_rect = scale_and_clamp_rect(
                    (
                        scroll_damage.abs_x,
                        scroll_damage.abs_y,
                        scroll_damage.view_w,
                        scroll_damage.view_h,
                    ),
                    sw,
                    sh,
                );
                blit_back_buffer_scroll(back_buf, sw, sh, scroll_damage);

                for rect in scroll_exposed_rects(scroll_damage).iter().flatten() {
                    if let Some((dx, dy, dw, dh)) = scale_and_clamp_rect(*rect, sw, sh) {
                        clear_back_buffer_rect(back_buf, sw, sh, dx, dy, dw, dh);
                        let surf = full_surf.with_clip(dx, dy, dw, dh);
                        render_tree(&st.controls, win_id, &surf, 0, 0, Some(*rect));
                        scroll_present_rect =
                            merge_pending_present_rect(scroll_present_rect, Some((dx, dy, dw, dh)));
                    }
                }

                for rect in scroll_chrome_rects(&st.controls, scroll_damage)
                    .iter()
                    .flatten()
                {
                    if let Some((dx, dy, dw, dh)) = scale_and_clamp_rect(*rect, sw, sh) {
                        clear_back_buffer_rect(back_buf, sw, sh, dx, dy, dw, dh);
                        let surf = full_surf.with_clip(dx, dy, dw, dh);
                        render_scrollview_chrome(&st.controls, scroll_damage, &surf);
                        scroll_present_rect =
                            merge_pending_present_rect(scroll_present_rect, Some((dx, dy, dw, dh)));
                    }
                }

                clear_dirty(&mut st.controls, win_id);
                sync_rendered_scroll_offsets(&mut st.controls, win_id);
                st.comp_windows[wi].dirty = false;
                st.comp_windows[wi].dirty_rect = None;
                staged_rect = if st.comp_windows[wi].present_pending {
                    merge_pending_present_rect(staged_rect, scroll_present_rect)
                } else {
                    scroll_present_rect
                };
                st.comp_windows[wi].present_pending = true;
                st.comp_windows[wi].pending_present_rect = staged_rect;
            } else {
                // Scale dirty rect to physical space (for Surface clip, SHM copy, present_rect)
                let physical_dr = logical_dr
                    .map(|(dx, dy, dw, dh)| {
                        let px = crate::theme::scale_i32(dx);
                        let py = crate::theme::scale_i32(dy);
                        let pw = crate::theme::scale(dw as u32);
                        let ph = crate::theme::scale(dh as u32);
                        // Clamp to physical surface bounds
                        let px = px.max(0);
                        let py = py.max(0);
                        let pw = pw.min(sw.saturating_sub(px as u32));
                        let ph = ph.min(sh.saturating_sub(py as u32));
                        (px, py, pw, ph)
                    })
                    .filter(|&(_, _, w, h)| w > 0 && h > 0);

                let surf = if let Some((dx, dy, dw, dh)) = physical_dr {
                    full_surf.with_clip(dx, dy, dw, dh)
                } else {
                    full_surf
                };

                if let Some((dx, dy, dw, dh)) = physical_dr {
                    clear_back_buffer_rect(back_buf, sw, sh, dx, dy, dw, dh);
                } else {
                    st.comp_windows[wi].back_buffer.fill(0x00000000);
                }

                render_tree(&st.controls, win_id, &surf, 0, 0, logical_dr);

                clear_dirty(&mut st.controls, win_id);
                sync_rendered_scroll_offsets(&mut st.controls, win_id);
                st.comp_windows[wi].dirty = false;
                st.comp_windows[wi].dirty_rect = None;
                staged_rect = if st.comp_windows[wi].present_pending {
                    merge_pending_present_rect(staged_rect, physical_dr)
                } else {
                    physical_dr
                };
                st.comp_windows[wi].present_pending = true;
                st.comp_windows[wi].pending_present_rect = staged_rect;
            }
        }

        // Respect compositor ACK strictly: never overwrite the SHM surface while
        // the previous frame may still be in flight. New renders stay staged in
        // the persistent back buffer until EVT_FRAME_ACK arrives.
        if frame_presented || !st.comp_windows[wi].present_pending {
            continue;
        }

        let pending_rect = st.comp_windows[wi].pending_present_rect;
        let back_buf = st.comp_windows[wi].back_buffer.as_ptr();

        unsafe {
            if let Some((dx, dy, dw, dh)) = pending_rect {
                let dx = dx as usize;
                let dy = dy as usize;
                let dw = dw as usize;
                let stride = sw as usize;
                for row in 0..dh as usize {
                    let off = (dy + row) * stride + dx;
                    core::ptr::copy_nonoverlapping(back_buf.add(off), surface_ptr.add(off), dw);
                }
            } else {
                let pixel_count = (sw as usize) * (sh as usize);
                core::ptr::copy_nonoverlapping(back_buf, surface_ptr, pixel_count);
            }
        }

        if let Some((dx, dy, dw, dh)) = pending_rect {
            compositor::present_rect(
                channel_id,
                comp_window_id,
                shm_id,
                dx as u32,
                dy as u32,
                dw,
                dh,
            );
        } else {
            compositor::present(channel_id, comp_window_id, shm_id);
        }
        st.comp_windows[wi].present_pending = false;
        st.comp_windows[wi].pending_present_rect = None;
        st.comp_windows[wi].frame_presented = true;
        st.comp_windows[wi].last_present_ms = crate::syscall::uptime_ms();
    }

    // ── Phase 4.1: Render popup (if active and dirty) ──────────────
    // Popup rendering is separate from regular windows because the popup
    // is not tracked in comp_windows. It has its own back buffer and SHM.
    // We use popup.dirty (not the control's dirty flag) because Phase 4's
    // clear_dirty already cleared the menu control's flag.
    let popup_render_info: Option<(control::ControlId, i32, u32, u32, *mut u32, u32, u32)> = {
        if let Some(ref popup) = st.popup {
            if popup.dirty {
                Some((
                    popup.menu_id,
                    popup.margin,
                    popup.width,
                    popup.height,
                    popup.surface,
                    popup.window_id,
                    popup.shm_id,
                ))
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((menu_id, margin, pw, ph, surface, popup_win_id, shm_id)) = popup_render_info {
        // Clear dirty flag and back buffer
        if let Some(ref mut p) = st.popup {
            p.dirty = false;
            p.back_buffer.fill(0x00000000);
        }
        let back_ptr = st.popup.as_mut().unwrap().back_buffer.as_mut_ptr();
        let surf = crate::draw::Surface::new(back_ptr, pw, ph);

        // Render menu directly (bypasses render_tree visibility check)
        if let Some(idx) = control::find_idx(&st.controls, menu_id) {
            st.controls[idx].render(&surf, margin, margin);
        }

        // Copy back buffer → SHM
        unsafe {
            let pixel_count = (pw as usize) * (ph as usize);
            core::ptr::copy_nonoverlapping(back_ptr, surface, pixel_count);
        }

        // Present the popup
        compositor::present(st.channel_id, popup_win_id, shm_id);
    }

    1
}

// ── External resize (triggered programmatically, e.g. from uictl) ────

/// Programmatically resize a window. `window_id` is the compositor window ID,
/// `logical_w` / `logical_h` are in logical (unscaled) pixels.
/// Mirrors the EVT_RESIZE path in run_once() but without a compositor event.
pub(crate) fn external_resize(
    st: &mut crate::AnyuiState,
    comp_window_id: u32,
    logical_w: u32,
    logical_h: u32,
) {
    let phys_w = crate::theme::scale(logical_w);
    let phys_h = crate::theme::scale(logical_h);

    // Find the comp_window index.
    let wi = match st
        .comp_windows
        .iter()
        .position(|cw| cw.window_id == comp_window_id)
    {
        Some(i) => i,
        None => return,
    };

    if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
        st.channel_id,
        comp_window_id,
        st.comp_windows[wi].shm_id,
        phys_w,
        phys_h,
    ) {
        st.comp_windows[wi].shm_id = new_shm_id;
        st.comp_windows[wi].surface = new_surface;
    }
    st.comp_windows[wi].width = phys_w;
    st.comp_windows[wi].height = phys_h;
    st.comp_windows[wi].logical_width = logical_w;
    st.comp_windows[wi].logical_height = logical_h;
    st.comp_windows[wi]
        .back_buffer
        .resize((phys_w as usize) * (phys_h as usize), 0);
    st.comp_windows[wi].dirty = true;
    st.comp_windows[wi].dirty_rect = None;

    // Update the Window control and re-layout.
    let win_id = if wi < st.windows.len() {
        st.windows[wi]
    } else {
        return;
    };
    if let Some(idx) = control::find_idx(&st.controls, win_id) {
        st.controls[idx].set_size(logical_w, logical_h);
        crate::layout::perform_layout(&mut st.controls, win_id);
    }
    st.needs_layout = true;
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

/// Walk up the ancestor chain of `id`, returning the nearest control that
/// is enabled, visible, and marked as `draggable`. Lets a child (e.g. a
/// non-interactive Label inside a draggable Card) transparently trigger
/// the drag on its parent.
fn nearest_draggable(
    controls: &[Box<dyn Control>],
    mut id: Option<ControlId>,
) -> Option<ControlId> {
    while let Some(cur) = id {
        if let Some(idx) = control::find_idx(controls, cur) {
            let base = controls[idx].base();
            if base.draggable && base.visible && !base.disabled {
                return Some(cur);
            }
            id = if base.parent == 0 {
                None
            } else {
                Some(base.parent)
            };
        } else {
            return None;
        }
    }
    None
}

/// Walk up the ancestor chain of `id`, returning the nearest control that
/// is enabled, visible, and marked as a drop target whose `drop_formats`
/// mask accepts the current drag payload format.
fn nearest_drop_target(
    controls: &[Box<dyn Control>],
    mut id: Option<ControlId>,
    payload_format: u32,
) -> Option<ControlId> {
    while let Some(cur) = id {
        if let Some(idx) = control::find_idx(controls, cur) {
            let base = controls[idx].base();
            if base.drop_target
                && base.visible
                && !base.disabled
                && crate::dnd::format_mask_contains(base.drop_formats, payload_format)
            {
                return Some(cur);
            }
            id = if base.parent == 0 {
                None
            } else {
                Some(base.parent)
            };
        } else {
            return None;
        }
    }
    None
}

fn drop_target_at_point(
    controls: &[Box<dyn Control>],
    win_id: ControlId,
    mx: i32,
    my: i32,
    payload_format: u32,
) -> Option<ControlId> {
    let hovered = control::hit_test_any(controls, win_id, mx, my, 0, 0);
    nearest_drop_target(controls, hovered, payload_format)
}

/// Modifier bits as expected by `dnd::negotiate_effect`: bit 0 = Ctrl, bit 1 = Shift.
fn dnd_modifier_bits(st: &crate::AnyuiState) -> u32 {
    let mut bits = 0u32;
    if (st.last_modifiers & control::MOD_CTRL) != 0 {
        bits |= 1;
    }
    if (st.last_modifiers & control::MOD_SHIFT) != 0 {
        bits |= 2;
    }
    bits
}

fn set_drop_hover(controls: &mut [Box<dyn Control>], id: ControlId, hovered: bool) {
    if let Some(idx) = control::find_idx(&*controls, id) {
        let base = controls[idx].base_mut();
        if base.drop_hover != hovered {
            base.drop_hover = hovered;
            base.mark_dirty();
        }
    }
}

fn maybe_begin_drag(
    st: &mut crate::AnyuiState,
    win_id: ControlId,
    comp_window_id: u32,
    mx: i32,
    my: i32,
    pending: &mut Vec<PendingCallback>,
) {
    if st.drag.is_some() {
        return;
    }
    if (st.pressed_button & 0x01) == 0 {
        return;
    }
    let pressed_id = match st.pressed {
        Some(id) => id,
        None => {
            crate::log!("[dnd] maybe_begin_drag: no pressed");
            return;
        }
    };
    let dx = mx - st.press_mouse_x;
    let dy = my - st.press_mouse_y;
    if !crate::dnd::drag_threshold_exceeded(dx, dy) {
        return;
    }
    crate::log!(
        "[dnd] threshold crossed pressed={} dx={} dy={} btn={}",
        pressed_id,
        dx,
        dy,
        st.pressed_button
    );
    // Resolve the actual drag source: walk up from the pressed control
    // until we find a draggable ancestor. Lets a child widget (e.g. a
    // Label painted inside a draggable Card) initiate the drag on its
    // parent without having to be draggable itself.
    let source_id = match nearest_draggable(&st.controls, Some(pressed_id)) {
        Some(id) => id,
        None => {
            crate::log!("[dnd] no draggable ancestor for pressed={}", pressed_id);
            return;
        }
    };
    crate::log!("[dnd] starting drag source={}", source_id);

    st.drag = Some(crate::DragSession {
        source_id,
        target_id: None,
        data: Vec::new(),
        // Default to text + copy|move. The source's DRAG_START callback can
        // override these via anyui_drag_set_payload / anyui_drag_set_text.
        format: crate::dnd::DND_FORMAT_TEXT,
        allowed_effects: crate::dnd::DND_EFFECT_COPY | crate::dnd::DND_EFFECT_MOVE,
        negotiated_effect: crate::dnd::DND_EFFECT_NONE,
        target_accepted: false,
        pointer_x: mx,
        pointer_y: my,
        modifiers: dnd_modifier_bits(st),
        bridge: None,
        accept_modifiers: 0,
    });
    fire_event_callback(&st.controls, source_id, control::EVENT_DRAG_START, pending);

    // Switch cursor to Move while dragging (see cursors::CursorShape::Move = 5).
    if st.current_cursor != 5 {
        st.current_cursor = 5;
        let cmd: [u32; 5] = [0x1018, comp_window_id, 5, 0, 0];
        crate::syscall::evt_chan_emit(st.channel_id, &cmd);
    }

    // Cross-window DnD: target detection is now driven by the compositor's
    // EVT_DRAG_ENTER / EVT_DRAG_OVER. No local hit-test here — the source's
    // DRAG_START callback (queued above) installs the payload via
    // anyui_drag_set_payload, which sends CMD_DRAG_BEGIN to the compositor.
    // Once the compositor sees the cursor over a target window, it dispatches
    // EVT_DRAG_ENTER, which our event-loop translates back into local
    // EVENT_DRAG_ENTER callbacks on the hit-tested control.
    let _ = (win_id, mx, my);
}

/// Drive target hit-testing for an incoming cross-process drag based on
/// pointer coordinates from `EVT_DRAG_OVER`. Replaces the old mouse_move-
/// driven local target detection.
fn dispatch_drag_over(
    st: &mut crate::AnyuiState,
    win_id: ControlId,
    mx: i32,
    my: i32,
    modifiers: u32,
    pending: &mut Vec<PendingCallback>,
) {
    let payload_format = match st.incoming_drag.as_ref() {
        Some(i) => i.format,
        None => return,
    };
    if let Some(inc) = st.incoming_drag.as_mut() {
        inc.pointer_x = mx;
        inc.pointer_y = my;
        inc.modifiers = modifiers;
    }
    let new_target = drop_target_at_point(&st.controls, win_id, mx, my, payload_format);
    let old_target = st.incoming_drag.as_ref().and_then(|i| i.target_control);

    if new_target != old_target {
        if let Some(old_id) = old_target {
            set_drop_hover(&mut st.controls, old_id, false);
            fire_event_callback(&st.controls, old_id, control::EVENT_DRAG_LEAVE, pending);
        }
        if let Some(inc) = st.incoming_drag.as_mut() {
            inc.target_control = new_target;
            inc.target_accepted = false;
            inc.negotiated_effect = 0;
        }
        if let Some(new_id) = new_target {
            set_drop_hover(&mut st.controls, new_id, true);
            fire_event_callback(&st.controls, new_id, control::EVENT_DRAG_ENTER, pending);
        }
    } else if let Some(target_id) = new_target {
        // Same control: reset acceptance only on modifier change so modifier-
        // aware targets can re-evaluate the negotiated effect.
        if let Some(inc) = st.incoming_drag.as_mut() {
            if inc.modifiers != inc.accept_modifiers {
                inc.target_accepted = false;
                inc.negotiated_effect = 0;
            }
        }
        fire_event_callback(&st.controls, target_id, control::EVENT_DRAG, pending);
    }

    drag_autoscroll(st, mx, my);
}

/// Walk up from the current drop target and apply scroll to the nearest
/// ancestor whose control opts in as a drag-autoscroll target. Quiet when
/// no drag is active or no scrollable ancestor exists.
fn drag_autoscroll(st: &mut crate::AnyuiState, mx: i32, my: i32) {
    let mut cur = st
        .incoming_drag
        .as_ref()
        .and_then(|i| i.target_control)
        .or(st.hovered);
    while let Some(id) = cur {
        let idx = match control::find_idx(&st.controls, id) {
            Some(i) => i,
            None => break,
        };
        let parent = st.controls[idx].base().parent;
        if st.controls[idx].is_drag_autoscroll_target() {
            let (ax, ay) = control::abs_position(&st.controls, id);
            let local_y = my - ay;
            let local_x = mx - ax;
            let h = st.controls[idx].base().h as i32;
            let w = st.controls[idx].base().w as i32;
            let dx = crate::dnd::autoscroll_delta(local_x, w);
            let dy = crate::dnd::autoscroll_delta(local_y, h);
            if dx != 0 || dy != 0 {
                st.controls[idx].drag_autoscroll(dx, dy);
            }
            break;
        }
        cur = if parent == 0 { None } else { Some(parent) };
    }
}

/// Build a cascaded tab sort key for a control: (parent_tab_index, own_tab_index, insertion_order).
/// This ensures controls are grouped by parent tab_index first, then sorted within the group.
fn tab_sort_key(
    controls: &[Box<dyn control::Control>],
    id: ControlId,
    insertion_idx: usize,
) -> (u32, u32, usize) {
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
    reverse: bool,
) {
    // Collect all focusable controls that belong to this window (with insertion index for stable sort)
    let mut focusable: Vec<(ControlId, usize)> = Vec::new();
    for (ins_idx, c) in st.controls.iter().enumerate() {
        if !c.accepts_focus() || c.id() == win_id || !c.base().visible {
            continue;
        }
        // Check that this control belongs to the window
        let mut cur = c.parent_id();
        let belongs = loop {
            if cur == win_id {
                break true;
            }
            if cur == 0 {
                break false;
            }
            match control::find_idx(&st.controls, cur) {
                Some(idx) => {
                    // Skip controls whose parent is invisible
                    if !st.controls[idx].base().visible {
                        break false;
                    }
                    cur = st.controls[idx].parent_id();
                }
                None => break false,
            }
        };
        if belongs {
            focusable.push((c.id(), ins_idx));
        }
    }

    if focusable.is_empty() {
        return;
    }

    // Sort by cascaded tab_index
    focusable.sort_by(|a, b| {
        let ka = tab_sort_key(&st.controls, a.0, a.1);
        let kb = tab_sort_key(&st.controls, b.0, b.1);
        ka.cmp(&kb)
    });

    let ids: Vec<ControlId> = focusable.iter().map(|f| f.0).collect();

    // Find current focused index
    let cur_idx = st
        .focused
        .and_then(|fid| ids.iter().position(|&id| id == fid))
        .unwrap_or(0);

    let next_idx = if reverse {
        if cur_idx == 0 {
            ids.len() - 1
        } else {
            cur_idx - 1
        }
    } else {
        (cur_idx + 1) % ids.len()
    };
    let next_id = ids[next_idx];

    // Blur old
    if let Some(old_id) = st.focused {
        if let Some(idx) = control::find_idx(&st.controls, old_id) {
            st.controls[idx].handle_blur();
            st.controls[idx].base_mut().mark_dirty();
            fire_event_callback(&st.controls, old_id, control::EVENT_BLUR, pending);
        }
    }

    // Focus new
    if let Some(idx) = control::find_idx(&st.controls, next_id) {
        st.controls[idx].handle_focus();
        st.controls[idx].base_mut().mark_dirty();
        st.focused = Some(next_id);
        fire_event_callback(&st.controls, next_id, control::EVENT_FOCUS, pending);
    }
}

fn is_point_in_control(controls: &[Box<dyn Control>], id: ControlId, px: i32, py: i32) -> bool {
    let (ax, ay) = control::abs_position(controls, id);
    if let Some(idx) = control::find_idx(controls, id) {
        let (w, h) = controls[idx].size();
        px >= ax && py >= ay && px < ax + w as i32 && py < ay + h as i32
    } else {
        false
    }
}

/// Show the tooltip for the given control (called after hover delay).
pub(crate) fn show_tooltip(st: &mut crate::AnyuiState, ctrl_id: ControlId) {
    let idx2 = match control::find_idx(&st.controls, ctrl_id) {
        Some(i) => i,
        None => return,
    };
    if st.controls[idx2].base().tooltip_text.is_empty() {
        return;
    }
    let text = st.controls[idx2].base().tooltip_text.clone();
    let win_id = st.controls[idx2].base().parent;
    let (ax, ay) = control::abs_position(&st.controls, ctrl_id);
    let ctrl_h = st.controls[idx2].base().h;
    // Estimate tooltip width: ~8px per char + 16px padding
    let tip_w = (text.len() as u32 * 8 + 16).max(40);

    // Lazily create the tooltip or reuse existing one
    let tip_id = if let Some(tid) = st.active_tooltip {
        tid
    } else {
        let tid = st.next_id;
        st.next_id += 1;
        let ctrl = crate::controls::create_control(
            control::ControlKind::Tooltip,
            tid,
            win_id,
            0,
            0,
            200,
            28,
            &text,
        );
        st.controls.push(ctrl);
        // Add tooltip as child of the top-level window
        let top_win = find_top_window(&st.controls, ctrl_id).unwrap_or(win_id);
        if let Some(p) = st.controls.iter_mut().find(|c| c.id() == top_win) {
            p.add_child(tid);
        }
        st.active_tooltip = Some(tid);
        tid
    };

    if let Some(ti) = control::find_idx(&st.controls, tip_id) {
        // Update text
        if let Some(tb) = st.controls[ti].text_base_mut() {
            tb.text = text;
        }
        // Position near mouse cursor for large controls (Canvas, etc.) so the
        // tooltip stays visible and close to the point of interest.  For small
        // controls (e.g. buttons) fall back to just below the widget.
        let (tx, ty) = if ctrl_h > 50 {
            (st.last_mouse_x + 16, st.last_mouse_y + 16)
        } else {
            (ax, ay + ctrl_h as i32 + 4)
        };
        st.controls[ti].set_position(tx, ty);
        st.controls[ti].base_mut().w = tip_w;
        st.controls[ti].base_mut().h = 28;
        st.controls[ti].base_mut().visible = true;
        st.controls[ti].base_mut().mark_dirty();
    }
}

/// Walk up the parent chain to find the top-level window control.
fn find_top_window(controls: &[Box<dyn control::Control>], id: ControlId) -> Option<ControlId> {
    let mut cur = id;
    loop {
        let parent = control::find_idx(controls, cur).map(|i| controls[i].base().parent)?;
        if parent == 0 || parent == cur {
            return Some(cur);
        }
        cur = parent;
    }
}

fn clear_tracking_for(st: &mut crate::AnyuiState, id: ControlId) {
    if st.focused == Some(id) {
        st.focused = None;
    }
    if st.pressed == Some(id) {
        st.pressed = None;
    }
    if st.hovered == Some(id) {
        st.hovered = None;
    }

    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let children: Vec<ControlId> = ctrl.children().to_vec();
        for &child in &children {
            clear_tracking_for(st, child);
        }
    }
}

// ── Popup dismiss ──────────────────────────────────────────────────

/// Dismiss the active context menu popup window.
/// Destroys the compositor window and clears the popup state.
/// Public wrapper so `lib.rs` can dismiss popups.
pub(crate) fn dismiss_popup_pub(st: &mut crate::AnyuiState) {
    dismiss_popup(st);
}

fn dismiss_popup(st: &mut crate::AnyuiState) {
    if let Some(popup) = st.popup.take() {
        // If this popup was owned by a DropDown, clear its open flag
        if let Some(dd_id) = popup.owner_dropdown {
            if let Some(dd_idx) = control::find_idx(&st.controls, dd_id) {
                if let Some(dd) = control::cast_mut::<crate::controls::dropdown::DropDown>(
                    &mut st.controls[dd_idx],
                    ControlKind::DropDown,
                ) {
                    dd.open = false;
                    dd.text_base.base.mark_dirty();
                }
            }
            // Remove the temporary ContextMenu control we created
            st.controls.retain(|c| c.id() != popup.menu_id);
        }
        if let Some(cb_id) = popup.owner_combobox {
            if let Some(cb_idx) = control::find_idx(&st.controls, cb_id) {
                if let Some(cb) = control::cast_mut::<crate::controls::combobox::ComboBox>(
                    &mut st.controls[cb_idx],
                    ControlKind::ComboBox,
                ) {
                    cb.open = false;
                    cb.request_popup = false;
                    cb.text_base.base.mark_dirty();
                }
            }
            st.controls.retain(|c| c.id() != popup.menu_id);
        }
        // Built-in text-edit context menu: clear the synthetic context_menu
        // pointer on the owner and remove the temporary ContextMenu control.
        if let Some(te_id) = popup.owner_text_edit {
            if let Some(te_idx) = control::find_idx(&st.controls, te_id) {
                st.controls[te_idx].base_mut().context_menu = None;
            }
            st.controls.retain(|c| c.id() != popup.menu_id);
        }
        compositor::destroy_window(st.channel_id, popup.window_id, popup.shm_id);
    }
}

// ── Dirty tracking ─────────────────────────────────────────────────

/// Clear dirty flags and reset prev_x/y/w/h for all controls in the subtree rooted at `id`.
/// Uses a stack buffer instead of Vec::to_vec() to avoid heap allocation per node.
fn clear_dirty(controls: &mut [Box<dyn Control>], id: ControlId) {
    let idx = match control::find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };
    let b = controls[idx].base_mut();
    b.dirty = false;
    b.prev_x = b.x;
    b.prev_y = b.y;
    b.prev_w = b.w;
    b.prev_h = b.h;
    let children: Vec<u32> = controls[idx].children().to_vec();
    for &cid in &children {
        clear_dirty(controls, cid);
    }
}

// ── Dirty rect collection ───────────────────────────────────────────

/// Union two rects: expand `existing` to also cover `(x, y, w, h)`.
fn union_rect(
    existing: Option<(i32, i32, u32, u32)>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> (i32, i32, u32, u32) {
    if w == 0 || h == 0 {
        return existing.unwrap_or((x, y, w, h));
    }
    match existing {
        None => (x, y, w, h),
        Some((ex, ey, ew, eh)) => {
            let x0 = ex.min(x);
            let y0 = ey.min(y);
            let x1 = (ex + ew as i32).max(x + w as i32);
            let y1 = (ey + eh as i32).max(y + h as i32);
            (x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
        }
    }
}

/// Merge two pending physical present rects.
/// `None` means "full window", which dominates any partial rect.
fn merge_pending_present_rect(
    existing: Option<(i32, i32, u32, u32)>,
    next: Option<(i32, i32, u32, u32)>,
) -> Option<(i32, i32, u32, u32)> {
    match (existing, next) {
        (None, _) | (_, None) => None,
        (Some((ex, ey, ew, eh)), Some((nx, ny, nw, nh))) => {
            Some(union_rect(Some((ex, ey, ew, eh)), nx, ny, nw, nh))
        }
    }
}

/// Check if two rectangles intersect.
fn rects_intersect(ax: i32, ay: i32, aw: u32, ah: u32, bx: i32, by: i32, bw: u32, bh: u32) -> bool {
    ax < bx + bw as i32 && ax + aw as i32 > bx && ay < by + bh as i32 && ay + ah as i32 > by
}

fn scale_and_clamp_rect(
    rect: (i32, i32, u32, u32),
    sw: u32,
    sh: u32,
) -> Option<(i32, i32, u32, u32)> {
    let (x, y, w, h) = rect;
    let px = crate::theme::scale_i32(x).max(0);
    let py = crate::theme::scale_i32(y).max(0);
    let pw = crate::theme::scale(w);
    let ph = crate::theme::scale(h);
    let pw = pw.min(sw.saturating_sub(px as u32));
    let ph = ph.min(sh.saturating_sub(py as u32));
    if pw == 0 || ph == 0 {
        return None;
    }
    Some((px, py, pw, ph))
}

fn scale_delta_i32(v: i32) -> i32 {
    if v < 0 {
        -crate::theme::scale_i32(-v)
    } else {
        crate::theme::scale_i32(v)
    }
}

fn find_scroll_blit_damage(
    controls: &[Box<dyn Control>],
    root_id: ControlId,
) -> Option<ScrollBlitDamage> {
    let mut found = None;
    let mut rejected = false;
    find_scroll_blit_damage_walk(controls, root_id, 0, 0, &mut found, &mut rejected);
    if rejected {
        None
    } else {
        found
    }
}

fn find_scroll_blit_damage_walk(
    controls: &[Box<dyn Control>],
    id: ControlId,
    parent_abs_x: i32,
    parent_abs_y: i32,
    found: &mut Option<ScrollBlitDamage>,
    rejected: &mut bool,
) {
    if *rejected {
        return;
    }
    let Some(idx) = control::find_idx(controls, id) else {
        return;
    };
    let b = controls[idx].base();
    let abs_x = parent_abs_x + b.x;
    let abs_y = parent_abs_y + b.y;

    if b.dirty {
        if controls[idx].kind() != ControlKind::ScrollView || found.is_some() {
            *rejected = true;
            return;
        }
        if b.prev_x != b.x || b.prev_y != b.y || b.prev_w != b.w || b.prev_h != b.h {
            *rejected = true;
            return;
        }
        let Some(sv) = control::cast_ref::<crate::controls::scroll_view::ScrollView>(
            &controls[idx],
            ControlKind::ScrollView,
        ) else {
            *rejected = true;
            return;
        };
        let (sx, sy) = sv.scroll_offsets();
        let (rendered_sx, rendered_sy) = sv.rendered_scroll_offsets();
        let dx = sx - rendered_sx;
        let dy = sy - rendered_sy;
        let (view_w, view_h) = sv.viewport_size();
        if subtree_has_dirty_in_rect(
            controls,
            id,
            abs_x - sx,
            abs_y - sy,
            (abs_x, abs_y, view_w, view_h),
        ) {
            *rejected = true;
            return;
        }
        if (dx == 0 && dy == 0)
            || view_w == 0
            || view_h == 0
            || dx.abs() as u32 >= view_w
            || dy.abs() as u32 >= view_h
        {
            *rejected = true;
            return;
        }
        *found = Some(ScrollBlitDamage {
            id,
            parent_abs_x,
            parent_abs_y,
            abs_x,
            abs_y,
            view_w,
            view_h,
            dx,
            dy,
        });
        return;
    }

    let (child_abs_x, child_abs_y) = match controls[idx].kind() {
        ControlKind::ScrollView => {
            let (sx, sy) =
                crate::controls::scroll_view::scroll_offsets(controls, controls[idx].id());
            (abs_x - sx, abs_y - sy)
        }
        ControlKind::Expander => (
            abs_x,
            abs_y + crate::controls::expander::HEADER_HEIGHT as i32,
        ),
        _ => (abs_x, abs_y),
    };
    for &cid in controls[idx].children() {
        find_scroll_blit_damage_walk(controls, cid, child_abs_x, child_abs_y, found, rejected);
        if *rejected {
            return;
        }
    }
}

fn subtree_has_dirty_in_rect(
    controls: &[Box<dyn Control>],
    id: ControlId,
    parent_abs_x: i32,
    parent_abs_y: i32,
    rect: (i32, i32, u32, u32),
) -> bool {
    let Some(idx) = control::find_idx(controls, id) else {
        return false;
    };
    for &cid in controls[idx].children() {
        if let Some(child_idx) = control::find_idx(controls, cid) {
            let b = controls[child_idx].base();
            let abs_x = parent_abs_x + b.x;
            let abs_y = parent_abs_y + b.y;
            if b.dirty && rects_intersect(abs_x, abs_y, b.w, b.h, rect.0, rect.1, rect.2, rect.3) {
                return true;
            }
            let (child_abs_x, child_abs_y) = match controls[child_idx].kind() {
                ControlKind::ScrollView => {
                    let (sx, sy) = crate::controls::scroll_view::scroll_offsets(
                        controls,
                        controls[child_idx].id(),
                    );
                    (abs_x - sx, abs_y - sy)
                }
                ControlKind::Expander => (
                    abs_x,
                    abs_y + crate::controls::expander::HEADER_HEIGHT as i32,
                ),
                _ => (abs_x, abs_y),
            };
            if subtree_has_dirty_in_rect(controls, cid, child_abs_x, child_abs_y, rect) {
                return true;
            }
        }
    }
    false
}

fn scroll_exposed_rects(d: ScrollBlitDamage) -> [Option<(i32, i32, u32, u32)>; 2] {
    let mut rects = [None, None];
    if d.dy > 0 {
        let h = (d.dy as u32).min(d.view_h);
        rects[0] = Some((d.abs_x, d.abs_y + d.view_h as i32 - h as i32, d.view_w, h));
    } else if d.dy < 0 {
        let h = ((-d.dy) as u32).min(d.view_h);
        rects[0] = Some((d.abs_x, d.abs_y, d.view_w, h));
    }
    if d.dx > 0 {
        let w = (d.dx as u32).min(d.view_w);
        rects[1] = Some((d.abs_x + d.view_w as i32 - w as i32, d.abs_y, w, d.view_h));
    } else if d.dx < 0 {
        let w = ((-d.dx) as u32).min(d.view_w);
        rects[1] = Some((d.abs_x, d.abs_y, w, d.view_h));
    }
    rects
}

fn scroll_chrome_rects(
    controls: &[Box<dyn Control>],
    d: ScrollBlitDamage,
) -> [Option<(i32, i32, u32, u32)>; 2] {
    let mut rects = [None, None];
    let Some(idx) = control::find_idx(controls, d.id) else {
        return rects;
    };
    let Some(sv) = control::cast_ref::<crate::controls::scroll_view::ScrollView>(
        &controls[idx],
        ControlKind::ScrollView,
    ) else {
        return rects;
    };
    if let Some((x, y, w, h)) = sv.vertical_bar_rect() {
        rects[0] = Some((d.abs_x + x, d.abs_y + y, w, h));
    }
    if let Some((x, y, w, h)) = sv.horizontal_bar_rect() {
        rects[1] = Some((d.abs_x + x, d.abs_y + y, w, h));
    }
    rects
}

fn blit_back_buffer_scroll(back_buf: *mut u32, stride: u32, height: u32, d: ScrollBlitDamage) {
    if back_buf.is_null() || stride == 0 || height == 0 {
        return;
    }

    let vx = crate::theme::scale_i32(d.abs_x).max(0);
    let vy = crate::theme::scale_i32(d.abs_y).max(0);
    let vw = crate::theme::scale(d.view_w).min(stride.saturating_sub(vx as u32));
    let vh = crate::theme::scale(d.view_h).min(height.saturating_sub(vy as u32));
    let dx = scale_delta_i32(d.dx);
    let dy = scale_delta_i32(d.dy);
    let abs_dx = dx.abs() as u32;
    let abs_dy = dy.abs() as u32;
    if vw == 0 || vh == 0 || abs_dx >= vw || abs_dy >= vh {
        return;
    }

    let copy_w = vw - abs_dx;
    let copy_h = vh - abs_dy;
    let (src_x, dst_x) = if dx > 0 { (vx + dx, vx) } else { (vx, vx - dx) };
    let (src_y, dst_y) = if dy > 0 { (vy + dy, vy) } else { (vy, vy - dy) };

    unsafe {
        if dst_y > src_y {
            for row in (0..copy_h).rev() {
                let src = (src_y as u32 + row) as usize * stride as usize + src_x as usize;
                let dst = (dst_y as u32 + row) as usize * stride as usize + dst_x as usize;
                core::ptr::copy(back_buf.add(src), back_buf.add(dst), copy_w as usize);
            }
        } else {
            for row in 0..copy_h {
                let src = (src_y as u32 + row) as usize * stride as usize + src_x as usize;
                let dst = (dst_y as u32 + row) as usize * stride as usize + dst_x as usize;
                core::ptr::copy(back_buf.add(src), back_buf.add(dst), copy_w as usize);
            }
        }
    }
}

fn render_scrollview_chrome(
    controls: &[Box<dyn Control>],
    d: ScrollBlitDamage,
    surface: &crate::draw::Surface,
) {
    let Some(idx) = control::find_idx(controls, d.id) else {
        return;
    };
    controls[idx].render(surface, d.parent_abs_x, d.parent_abs_y);
}

fn sync_rendered_scroll_offsets(controls: &mut [Box<dyn Control>], id: ControlId) {
    let Some(idx) = control::find_idx(controls, id) else {
        return;
    };
    if controls[idx].kind() == ControlKind::ScrollView {
        if let Some(sv) = control::cast_mut::<crate::controls::scroll_view::ScrollView>(
            &mut controls[idx],
            ControlKind::ScrollView,
        ) {
            sv.sync_rendered_scroll_offsets();
        }
    }
    let children: Vec<u32> = controls[idx].children().to_vec();
    for &cid in &children {
        sync_rendered_scroll_offsets(controls, cid);
    }
}

/// Walk the control tree, compute absolute positions, and union dirty controls'
/// bounding rects into `cw.dirty_rect`. If the root Window control itself is dirty,
/// forces a full-window redraw (dirty_rect = None).
fn collect_dirty_rects(
    controls: &[Box<dyn Control>],
    id: ControlId,
    parent_abs_x: i32,
    parent_abs_y: i32,
    cw: &mut crate::CompWindow,
) {
    let idx = match control::find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };
    if !controls[idx].visible() {
        // Even when invisible, if the control is dirty its area must be included
        // in the dirty rect so the background behind it gets repainted.
        // Without this, hiding a control (e.g. context menu) leaves stale pixels.
        let b = controls[idx].base();
        if b.dirty {
            cw.dirty = true;
            let abs_x = parent_abs_x + b.x;
            let abs_y = parent_abs_y + b.y;
            cw.dirty_rect = Some(union_rect(cw.dirty_rect, abs_x, abs_y, b.w, b.h));
        }
        return;
    }

    let b = controls[idx].base();
    let abs_x = parent_abs_x + b.x;
    let abs_y = parent_abs_y + b.y;

    if b.dirty {
        cw.dirty = true;

        // If the top-level Window control itself is dirty, force full redraw
        // (covers resize, theme changes, initial render).
        if controls[idx].kind() == ControlKind::Window {
            cw.dirty_rect = None;
            return; // No need to recurse — full render
        }

        // Union current bounds with dirty rect
        cw.dirty_rect = Some(union_rect(cw.dirty_rect, abs_x, abs_y, b.w, b.h));

        // If position or size changed, also union the old bounds to repaint the vacated area.
        if b.prev_x != b.x || b.prev_y != b.y || b.prev_w != b.w || b.prev_h != b.h {
            let prev_abs_x = parent_abs_x + b.prev_x;
            let prev_abs_y = parent_abs_y + b.prev_y;
            cw.dirty_rect = Some(union_rect(
                cw.dirty_rect,
                prev_abs_x,
                prev_abs_y,
                b.prev_w,
                b.prev_h,
            ));
        }
    }

    let children: Vec<u32> = controls[idx].children().to_vec();

    // Handle ScrollView offset for child absolute positions
    let (child_abs_x, child_abs_y) = match controls[idx].kind() {
        ControlKind::ScrollView => {
            let (sx, sy) =
                crate::controls::scroll_view::scroll_offsets(controls, controls[idx].id());
            (abs_x - sx, abs_y - sy)
        }
        ControlKind::Expander => (
            abs_x,
            abs_y + crate::controls::expander::HEADER_HEIGHT as i32,
        ),
        _ => (abs_x, abs_y),
    };

    for &cid in &children {
        collect_dirty_rects(controls, cid, child_abs_x, child_abs_y, cw);
    }
}

fn clear_back_buffer_rect(
    back_buf: *mut u32,
    stride: u32,
    height: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) {
    if back_buf.is_null() || w == 0 || h == 0 || stride == 0 || height == 0 {
        return;
    }
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w as i32).max(0) as u32;
    let y1 = (y + h as i32).max(0) as u32;
    let x1 = x1.min(stride);
    let y1 = y1.min(height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for row in y0..y1 {
        let off = row as usize * stride as usize + x0 as usize;
        let count = (x1 - x0) as usize;
        unsafe {
            core::ptr::write_bytes(back_buf.add(off), 0, count);
        }
    }
}

// ── Tree rendering ──────────────────────────────────────────────────

/// Render the control tree, optionally skipping controls outside `dirty_rect`.
/// When `dirty_rect` is `Some`, only controls whose bounds intersect the dirty
/// region are rendered — all other controls retain their pixels from the
/// previous frame in the persistent back buffer.
fn render_tree(
    controls: &[Box<dyn Control>],
    id: ControlId,
    surface: &crate::draw::Surface,
    parent_abs_x: i32,
    parent_abs_y: i32,
    dirty_rect: Option<(i32, i32, u32, u32)>,
) {
    let idx = match control::find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };

    if !controls[idx].visible() {
        return;
    }

    let (cx, cy) = controls[idx].position();
    let abs_x = parent_abs_x + cx;
    let abs_y = parent_abs_y + cy;
    let (cw, ch) = controls[idx].size();

    // Skip entire subtree if this control doesn't intersect the dirty rect.
    // The back buffer retains pixels from the previous frame, so unchanged
    // controls don't need to be redrawn.
    if let Some((dx, dy, dw, dh)) = dirty_rect {
        if !rects_intersect(abs_x, abs_y, cw, ch, dx, dy, dw, dh) {
            return;
        }
    }

    if controls[idx].kind() == ControlKind::AntiAliasFilterContainer {
        render_antialias_filter_container(
            controls,
            idx,
            surface,
            parent_abs_x,
            parent_abs_y,
            dirty_rect,
        );
        return;
    }

    // ScrollView: skip initial render — scrollbar is drawn after children
    // so it isn't painted over by content.
    if controls[idx].kind() != ControlKind::ScrollView {
        controls[idx].render(surface, parent_abs_x, parent_abs_y);
    }

    let child_abs_x = abs_x;
    let child_abs_y = abs_y;

    let children: Vec<u32> = controls[idx].children().to_vec();
    // Skip children if this is a collapsed Expander
    if controls[idx].kind() == ControlKind::Expander && controls[idx].base().state == 0 {
        return;
    }
    // ScrollView: offset children by -scroll_y and clip to viewport
    // Expander: offset children by +HEADER_HEIGHT (below header)
    let (child_abs_x, child_abs_y, child_surface, sv_cull) = match controls[idx].kind() {
        ControlKind::ScrollView => {
            // Logical coords for the ScrollView viewport
            let sv_x = parent_abs_x + controls[idx].base().x;
            let sv_y = parent_abs_y + controls[idx].base().y;
            let (sv_w, sv_h) = crate::controls::scroll_view::viewport_size(controls, id);
            let (sx, sy) = crate::controls::scroll_view::scroll_offsets(controls, id);
            // Scale to physical for the Surface clip rect
            let p = crate::draw::scale_bounds(0, 0, sv_x, sv_y, sv_w, sv_h);
            (
                child_abs_x - sx,
                child_abs_y - sy,
                surface.with_clip(p.x, p.y, p.w, p.h),
                Some((sv_x, sv_y, sv_w as i32, sv_h as i32)),
            )
        }
        ControlKind::Expander => (
            child_abs_x,
            child_abs_y + crate::controls::expander::HEADER_HEIGHT as i32,
            *surface,
            None,
        ),
        _ => (child_abs_x, child_abs_y, *surface, None),
    };
    for &cid in &children {
        // Viewport culling: skip children completely outside the ScrollView viewport.
        if let Some((vis_left, vis_top, vis_w, vis_h)) = sv_cull {
            if let Some(ci) = control::find_idx(controls, cid) {
                let (cx, cy) = controls[ci].position();
                let (c_w, c_h) = controls[ci].size();
                let child_left = child_abs_x + cx;
                let child_right = child_left + c_w as i32;
                let child_top = child_abs_y + cy;
                let child_bottom = child_top + c_h as i32;
                let vis_right = vis_left + vis_w;
                let vis_bottom = vis_top + vis_h;
                if child_right < vis_left
                    || child_left > vis_right
                    || child_bottom < vis_top
                    || child_top > vis_bottom
                {
                    continue;
                }
            }
        }
        render_tree(
            controls,
            cid,
            &child_surface,
            child_abs_x,
            child_abs_y,
            dirty_rect,
        );
    }

    // ScrollView: render scrollbar AFTER children so it isn't painted over.
    // Clip to the ScrollView's own physical bounds so the scrollbar never
    // bleeds outside the container (e.g. over DOCK_BOTTOM siblings).
    if controls[idx].kind() == ControlKind::ScrollView {
        let (cx, cy) = controls[idx].position();
        let (cw, ch) = controls[idx].size();
        let p = crate::draw::scale_bounds(parent_abs_x, parent_abs_y, cx, cy, cw, ch);
        let sv_surface = surface.with_clip(p.x, p.y, p.w, p.h);
        controls[idx].render(&sv_surface, parent_abs_x, parent_abs_y);
    }
}

fn render_antialias_filter_container(
    controls: &[Box<dyn Control>],
    idx: usize,
    surface: &crate::draw::Surface,
    parent_abs_x: i32,
    parent_abs_y: i32,
    _dirty_rect: Option<(i32, i32, u32, u32)>,
) {
    let b = controls[idx].base();
    if b.w == 0 || b.h == 0 {
        return;
    }

    let p = crate::draw::scale_bounds(parent_abs_x, parent_abs_y, b.x, b.y, b.w, b.h);
    if p.w == 0 || p.h == 0 {
        return;
    }

    let pixel_count = (p.w as usize).saturating_mul(p.h as usize);
    if pixel_count == 0 || pixel_count > 16 * 1024 * 1024 {
        return;
    }

    let mut pixels = alloc::vec![0u32; pixel_count];
    let offscreen = crate::draw::Surface::new(pixels.as_mut_ptr(), p.w, p.h);

    // Render the container itself with its logical origin mapped to (0, 0).
    controls[idx].render(&offscreen, -b.x, -b.y);

    let children: Vec<u32> = controls[idx].children().to_vec();
    for &cid in &children {
        // The offscreen subtree must be complete; the outer dirty rect only
        // decides whether this container is rendered at all.
        render_tree(controls, cid, &offscreen, 0, 0, None);
    }

    let strength = if b.style.filter_strength == 0 {
        96
    } else {
        b.style.filter_strength.clamp(0, 255)
    };
    let quality = b.style.filter_quality.clamp(1, 3);
    if strength > 0 {
        for _ in 0..quality {
            antialias_edge_pass(&mut pixels, p.w as usize, p.h as usize, strength);
        }
    }

    crate::draw::blit_argb(surface, p.x, p.y, p.w, p.h, &pixels);
}

fn antialias_edge_pass(pixels: &mut [u32], w: usize, h: usize, strength: u32) {
    if w < 3 || h < 3 || pixels.len() < w.saturating_mul(h) {
        return;
    }

    let src = pixels.to_vec();
    let mix = strength.min(255);
    let keep = 255 - mix;

    for y in 1..h - 1 {
        let row = y * w;
        for x in 1..w - 1 {
            let i = row + x;
            let c = src[i];
            let a = (c >> 24) & 0xFF;
            let n0 = src[i - 1];
            let n1 = src[i + 1];
            let n2 = src[i - w];
            let n3 = src[i + w];
            let a0 = (n0 >> 24) & 0xFF;
            let a1 = (n1 >> 24) & 0xFF;
            let a2 = (n2 >> 24) & 0xFF;
            let a3 = (n3 >> 24) & 0xFF;
            let min_a = a.min(a0).min(a1).min(a2).min(a3);
            let max_a = a.max(a0).max(a1).max(a2).max(a3);

            if max_a - min_a < 96 {
                continue;
            }

            let avg_a = (a0 + a1 + a2 + a3) / 4;
            let avg_r = (((n0 >> 16) & 0xFF)
                + ((n1 >> 16) & 0xFF)
                + ((n2 >> 16) & 0xFF)
                + ((n3 >> 16) & 0xFF))
                / 4;
            let avg_g =
                (((n0 >> 8) & 0xFF) + ((n1 >> 8) & 0xFF) + ((n2 >> 8) & 0xFF) + ((n3 >> 8) & 0xFF))
                    / 4;
            let avg_b = ((n0 & 0xFF) + (n1 & 0xFF) + (n2 & 0xFF) + (n3 & 0xFF)) / 4;

            let r = ((c >> 16) & 0xFF) * keep + avg_r * mix;
            let g = ((c >> 8) & 0xFF) * keep + avg_g * mix;
            let b = (c & 0xFF) * keep + avg_b * mix;
            let out_a = a * keep + avg_a * mix;

            pixels[i] = ((out_a / 255) << 24) | ((r / 255) << 16) | ((g / 255) << 8) | (b / 255);
        }
    }
}

// ── Theme-change repaint helper ─────────────────────────────────────

/// Recursively mark a control and all its descendants as dirty.
fn mark_tree_dirty(controls: &mut [Box<dyn Control>], idx: usize) {
    controls[idx].base_mut().mark_dirty();
    let children: Vec<u32> = controls[idx].children().to_vec();
    for &cid in &children {
        if let Some(ci) = control::find_idx(controls, cid) {
            mark_tree_dirty(controls, ci);
        }
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

fn collect_descendants(controls: &[Box<dyn Control>], id: ControlId, out: &mut Vec<ControlId>) {
    if let Some(idx) = control::find_idx(controls, id) {
        let children: Vec<ControlId> = controls[idx].children().to_vec();
        for &child in &children {
            out.push(child);
            collect_descendants(controls, child, out);
        }
    }
}
