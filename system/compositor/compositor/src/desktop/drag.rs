//! Cross-window drag-and-drop coordinator.
//!
//! At most one global drag session is active at a time. It is started by a
//! source app via `CMD_DRAG_BEGIN`, runs across the rest of the desktop, and
//! ends with either a drop on a target window or a cancel (ESC, source
//! window closed, etc.). The compositor's job is purely routing — payload
//! interpretation lives in the source/target apps.
//!
//! Payload transport: the source allocates an SHM region, writes payload
//! bytes, and passes the SHM id with `CMD_DRAG_BEGIN`. Targets receive the
//! same id via `EVT_DRAG_ENTER` and `shm_map` it themselves.

use crate::ipc_protocol as proto;

use super::window::{
    HitTest, EVENT_DRAG_END, EVENT_DRAG_ENTER, EVENT_DRAG_FEEDBACK, EVENT_DRAG_LEAVE,
    EVENT_DRAG_OVER, EVENT_DROP,
};
use super::Desktop;

/// Live cross-window drag session tracked by the compositor.
#[derive(Clone)]
pub(crate) struct GlobalDrag {
    /// TID of the source process.
    pub source_tid: u32,
    /// Window the drag originated from.
    pub source_window_id: u32,
    /// Payload format identifier (mirrors libanyui::dnd::DND_FORMAT_*).
    pub format: u32,
    /// SHM region holding the payload. Read-only for targets.
    pub payload_shm_id: u32,
    /// Length of the payload in bytes.
    pub payload_len: u32,
    /// Bitmask of effects the source permits (COPY|MOVE|LINK).
    pub allowed_effects: u32,

    /// Window currently under the cursor that has accepted ENTER, if any.
    /// `None` while the cursor is over chrome / no window / a non-focusable area.
    pub current_target_window: Option<u32>,
    /// True after the current target most recently called CMD_DRAG_ACCEPT.
    pub target_accepted: bool,
    /// Effect the current target negotiated; 0 when no acceptance.
    pub negotiated_effect: u32,
}

impl Desktop {
    // ── Source lifecycle ────────────────────────────────────────────────

    /// Start a global drag. Returns true if accepted; false if a drag is
    /// already in progress or the source window is not owned by `source_tid`.
    pub(crate) fn drag_begin(
        &mut self,
        source_tid: u32,
        source_window_id: u32,
        format: u32,
        payload_shm_id: u32,
        payload_len: u32,
        allowed_effects: u32,
    ) -> bool {
        if self.global_drag.is_some() {
            return false;
        }
        if !self
            .windows
            .iter()
            .any(|w| w.id == source_window_id && w.owner_tid == source_tid)
        {
            return false;
        }
        self.global_drag = Some(GlobalDrag {
            source_tid,
            source_window_id,
            format,
            payload_shm_id,
            payload_len,
            allowed_effects: allowed_effects & 0x07,
            current_target_window: None,
            target_accepted: false,
            negotiated_effect: 0,
        });
        // Immediately route to whatever's under the cursor — the source's
        // own window counts as a valid target if the cursor is still inside
        // it (the common case at drag-start).
        let mx = self.mouse_x;
        let my = self.mouse_y;
        self.drag_update_target(mx, my);
        true
    }

    /// Source attaches an optional drag-image (ghost) to the active drag.
    /// Replaces any previously-set image. Returns true on success.
    pub(crate) fn drag_set_image(
        &mut self,
        source_tid: u32,
        source_window_id: u32,
        shm_id: u32,
        w: u32,
        h: u32,
        hot_x: i32,
        hot_y: i32,
    ) -> bool {
        let matches = matches!(
            self.global_drag.as_ref(),
            Some(d) if d.source_tid == source_tid && d.source_window_id == source_window_id
        );
        if !matches {
            return false;
        }
        if w == 0 || h == 0 || w > 1024 || h > 1024 {
            return false;
        }
        self.drag_clear_image();
        let addr = anyos_std::ipc::shm_map(shm_id);
        if addr == 0 {
            return false;
        }
        self.compositor.drag_image = Some(crate::compositor::DragImageOverlay {
            shm_id,
            pixels: addr as *const u32,
            image_w: w,
            image_h: h,
            hot_x,
            hot_y,
            last_x: 0,
            last_y: 0,
            last_drawn: false,
        });
        true
    }

    /// Release the compositor's mapping of the current drag image (if any)
    /// and damage the last-drawn rect so the next compose erases it.
    pub(crate) fn drag_clear_image(&mut self) {
        if let Some(img) = self.compositor.drag_image.take() {
            if img.last_drawn {
                let r =
                    crate::compositor::Rect::new(img.last_x, img.last_y, img.image_w, img.image_h);
                self.compositor.add_damage(r);
            }
            anyos_std::ipc::shm_unmap(img.shm_id);
        }
    }

    /// Cancel the active drag (source-initiated, e.g. ESC).
    pub(crate) fn drag_cancel(&mut self, source_tid: u32, source_window_id: u32) {
        let active = match self.global_drag.as_ref() {
            Some(d) => d.source_tid == source_tid && d.source_window_id == source_window_id,
            None => false,
        };
        if active {
            self.drag_finish(false, 0);
        }
    }

    /// Cancel any drag whose source belongs to `tid` (used when the process
    /// dies). Quiet no-op when no such drag exists.
    pub(crate) fn drag_cancel_for_tid(&mut self, tid: u32) {
        let matches = matches!(self.global_drag.as_ref(), Some(d) if d.source_tid == tid);
        if matches {
            self.drag_finish(false, 0);
        }
    }

    /// Drop the active drag because the source window closed.
    pub(crate) fn drag_cancel_for_window(&mut self, window_id: u32) {
        let matches = matches!(
            self.global_drag.as_ref(),
            Some(d) if d.source_window_id == window_id
        );
        if matches {
            self.drag_finish(false, 0);
        }
    }

    // ── Target lifecycle ────────────────────────────────────────────────

    /// Target opt-in. Returns the negotiated effect (0 if the request didn't
    /// overlap the source's allowed effects).
    pub(crate) fn drag_accept(&mut self, target_window_id: u32, requested_effects: u32) -> u32 {
        // Compute the negotiated effect, then mutate, then notify source.
        let (effect, source_window_id, source_target) = {
            let drag = match self.global_drag.as_mut() {
                Some(d) => d,
                None => return 0,
            };
            if drag.current_target_window != Some(target_window_id) {
                return 0;
            }
            let overlap = drag.allowed_effects & (requested_effects & 0x07);
            // Pick a single bit from overlap. Modifier-aware preference is
            // up to the target's own libanyui logic; the compositor doesn't
            // know modifiers here (libanyui still passes them in EVT_DRAG_OVER).
            let effect = if overlap & 0x02 != 0 {
                0x02 // Move
            } else if overlap & 0x01 != 0 {
                0x01 // Copy
            } else if overlap & 0x04 != 0 {
                0x04 // Link
            } else {
                0
            };
            drag.target_accepted = effect != 0;
            drag.negotiated_effect = effect;
            (effect, drag.source_window_id, target_window_id)
        };
        let _ = source_target;
        self.push_event(source_window_id, [EVENT_DRAG_FEEDBACK, 1, effect, 0, 0]);
        effect
    }

    /// Target rejects the current drag.
    pub(crate) fn drag_reject(&mut self, target_window_id: u32) {
        let source_window_id = match self.global_drag.as_mut() {
            Some(d) if d.current_target_window == Some(target_window_id) => {
                d.target_accepted = false;
                d.negotiated_effect = 0;
                d.source_window_id
            }
            _ => return,
        };
        self.push_event(source_window_id, [EVENT_DRAG_FEEDBACK, 0, 0, 0, 0]);
    }

    // ── Routing (mouse_move / mouse_up) ────────────────────────────────

    /// Walk the window stack at the given screen coords and pick the new
    /// target window for the active drag. Fires DRAG_LEAVE on the previous
    /// target and DRAG_ENTER on the new one if they differ.
    pub(crate) fn drag_update_target(&mut self, mx: i32, my: i32) {
        if self.global_drag.is_none() {
            return;
        }
        // Hit-test for content area. Drags don't fire on chrome/menubar.
        let new_target = match self.topmost_window_hit(mx, my, true) {
            Some((wid, HitTest::Content)) => Some(wid),
            _ => None,
        };
        let (
            old_target,
            format,
            payload_shm_id,
            payload_len,
            allowed_effects,
            source_tid,
            source_window_id,
        ) = {
            let d = self.global_drag.as_ref().unwrap();
            (
                d.current_target_window,
                d.format,
                d.payload_shm_id,
                d.payload_len,
                d.allowed_effects,
                d.source_tid,
                d.source_window_id,
            )
        };

        // NB: drain_ipc_events translates `[event_type, a, b, c, _drop]` into
        // wire form `[ipc_type, win.id, a, b, c]`. Don't repeat win.id in the
        // pushed array — the slot is filled by the drain step. evt[4] is
        // dropped on the wire, so no payload there.
        if new_target == old_target {
            if let Some(target_id) = new_target {
                let modifiers = self.current_modifiers & 0xFF;
                let xy = drag_pack_xy(self, target_id, mx, my);
                let mod_eff = modifiers | ((allowed_effects & 0xFF) << 8);
                // Wire: [type, target_id, payload_len, xy, mod_eff]
                self.push_event(target_id, [EVENT_DRAG_OVER, payload_len, xy, mod_eff, 0]);
            }
            return;
        }

        // Target changed: leave old, enter new, reset acceptance.
        if let Some(old_id) = old_target {
            // Wire: [type, old_id, 0, 0, 0]
            self.push_event(old_id, [EVENT_DRAG_LEAVE, 0, 0, 0, 0]);
        }
        if let Some(d) = self.global_drag.as_mut() {
            d.current_target_window = new_target;
            d.target_accepted = false;
            d.negotiated_effect = 0;
        }
        if let Some(new_id) = new_target {
            let packed_meta = (allowed_effects & 0xFF) | ((source_tid & 0x00FF_FFFF) << 8);
            // Wire: [type, new_id, format, payload_shm_id, packed_meta]
            self.push_event(
                new_id,
                [EVENT_DRAG_ENTER, format, payload_shm_id, packed_meta, 0],
            );
            let modifiers = self.current_modifiers & 0xFF;
            let xy = drag_pack_xy(self, new_id, mx, my);
            let mod_eff = modifiers | ((allowed_effects & 0xFF) << 8);
            // Wire: [type, new_id, payload_len, xy, mod_eff]
            self.push_event(new_id, [EVENT_DRAG_OVER, payload_len, xy, mod_eff, 0]);
        }
        let target_present = if new_target.is_some() { 1 } else { 0 };
        // Wire: [type, source_window_id, target_present, 0, 0]
        self.push_event(
            source_window_id,
            [EVENT_DRAG_FEEDBACK, target_present, 0, 0, 0],
        );
    }

    /// Pointer was released — finalize the drag with a drop on the current
    /// target (if any and accepted) or cancel.
    pub(crate) fn drag_finish_on_release(&mut self, mx: i32, my: i32) {
        if self.global_drag.is_none() {
            return;
        }
        let (target_id, target_accepted, negotiated_effect, source_tid) = {
            let d = self.global_drag.as_ref().unwrap();
            (
                d.current_target_window,
                d.target_accepted,
                d.negotiated_effect,
                d.source_tid,
            )
        };
        if let Some(target_id) = target_id {
            if target_accepted {
                let xy = drag_pack_xy(self, target_id, mx, my);
                // Wire: [type, target_id, xy, negotiated_effect, source_tid]
                self.push_event(
                    target_id,
                    [EVENT_DROP, xy, negotiated_effect, source_tid, 0],
                );
                self.drag_finish(true, negotiated_effect);
                return;
            }
            // Wire: [type, target_id, 0, 0, 0]
            self.push_event(target_id, [EVENT_DRAG_LEAVE, 0, 0, 0, 0]);
        }
        self.drag_finish(false, 0);
    }

    /// Tear down the drag session and tell the source how it ended.
    fn drag_finish(&mut self, completed: bool, negotiated_effect: u32) {
        let drag = match self.global_drag.take() {
            Some(d) => d,
            None => return,
        };
        // Drop any drag-image overlay first so the next compose erases it.
        self.drag_clear_image();
        // Make sure the target sees a LEAVE if we cancel while hovering it
        // and didn't go through release path.
        if !completed {
            if let Some(target_id) = drag.current_target_window {
                // Wire: [type, target_id, 0, 0, 0]
                self.push_event(target_id, [EVENT_DRAG_LEAVE, 0, 0, 0, 0]);
            }
        }
        let completed_u = if completed { 1 } else { 0 };
        // Wire: [type, source_window_id, completed, negotiated_effect, 0]
        self.push_event(
            drag.source_window_id,
            [EVENT_DRAG_END, completed_u, negotiated_effect, 0, 0],
        );
    }

    /// True while a global drag session is active. Used by mouse routing to
    /// suppress normal MOUSE_MOVE / MOUSE_UP forwarding (drags get their own
    /// EVT_DRAG_OVER / EVT_DROP path).
    pub(crate) fn drag_active(&self) -> bool {
        self.global_drag.is_some()
    }
}

/// Pack screen-coords mx/my into the (x_u16 << 16) | y_u16 form used by
/// EVT_DRAG_OVER / EVT_DROP, expressed in window-local content coordinates.
fn drag_pack_xy(desktop: &Desktop, window_id: u32, mx: i32, my: i32) -> u32 {
    if let Some(win) = desktop.windows.iter().find(|w| w.id == window_id) {
        let lx = mx - win.x;
        let mut ly = my - win.y;
        if !win.is_borderless() {
            ly -= super::title_bar_height() as i32;
        }
        let lx_u = lx.clamp(0, 0xFFFF) as u32;
        let ly_u = ly.clamp(0, 0xFFFF) as u32;
        (lx_u << 16) | ly_u
    } else {
        0
    }
}
