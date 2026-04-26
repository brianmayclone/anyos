//! Cross-window drag-and-drop bridge to the compositor.
//!
//! The libanyui drag flow is local-first: drag-source detection, threshold,
//! and DRAG_START callbacks all run inside the source app. Once the
//! application has set a payload via `anyui_drag_set_payload`, this module
//! announces the drag to the compositor (CMD_DRAG_BEGIN) so the cursor can
//! cross window boundaries and visit other apps' drop targets.
//!
//! Payload transport: the source allocates a small SHM region, writes the
//! payload bytes, and passes the SHM id with `CMD_DRAG_BEGIN`. The compositor
//! relays the id to whichever target is under the pointer; targets map the
//! SHM read-only via `shm_map`. The source keeps the SHM alive until it
//! receives `EVT_DRAG_END`, then unmaps and destroys it.

use crate::control::ControlId;
use crate::syscall;
use crate::AnyuiState;

/// Maximum payload size for a cross-window drag. Most payloads (paths,
/// short text snippets, custom struct blobs) fit comfortably; larger drags
/// are uncommon and would require a different transport.
pub const MAX_DRAG_PAYLOAD_BYTES: u32 = 64 * 1024;

// IPC command IDs (mirror system/compositor/.../ipc_protocol.rs).
pub const CMD_DRAG_BEGIN: u32 = 0x1040;
pub const CMD_DRAG_ACCEPT: u32 = 0x1041;
pub const CMD_DRAG_REJECT: u32 = 0x1042;
pub const CMD_DRAG_CANCEL: u32 = 0x1043;
pub const CMD_DRAG_SET_IMAGE: u32 = 0x1044;

// Compositor-emitted events.
pub const EVT_DRAG_ENTER: u32 = 0x3020;
pub const EVT_DRAG_OVER: u32 = 0x3021;
pub const EVT_DRAG_LEAVE: u32 = 0x3022;
pub const EVT_DROP: u32 = 0x3023;
pub const EVT_DRAG_FEEDBACK: u32 = 0x3024;
pub const EVT_DRAG_END: u32 = 0x3025;

/// Walk the parent chain from `id` to its window root, then look up the
/// matching `comp_window.window_id`. Returns 0 if the control isn't owned
/// by any registered window (shouldn't happen for a drag source).
pub fn comp_window_for_control(st: &AnyuiState, id: ControlId) -> u32 {
    let mut cur = id;
    loop {
        let parent = match st.controls.iter().find(|c| c.id() == cur) {
            Some(c) => c.base().parent,
            None => return 0,
        };
        if parent == 0 {
            break;
        }
        cur = parent;
    }
    // `cur` is now a window-root control id. Find its index in `st.windows`.
    if let Some(idx) = st.windows.iter().position(|&w| w == cur) {
        if idx < st.comp_windows.len() {
            return st.comp_windows[idx].window_id;
        }
    }
    0
}

/// Allocate the source-side payload SHM if not already done. Returns true on
/// success. The capacity is fixed at `MAX_DRAG_PAYLOAD_BYTES`; payloads
/// larger than that are silently truncated when written.
pub fn ensure_bridge_shm(st: &mut AnyuiState, source_window_id: u32) -> bool {
    let drag = match st.drag.as_mut() {
        Some(d) => d,
        None => return false,
    };
    if let Some(b) = drag.bridge.as_ref() {
        // Already allocated. If the source window changed (shouldn't), keep
        // the existing one — its window_id is what we announced.
        let _ = b;
        return true;
    }
    let shm_id = syscall::shm_create(MAX_DRAG_PAYLOAD_BYTES);
    if shm_id == 0 {
        return false;
    }
    let addr = syscall::shm_map(shm_id);
    if addr == 0 {
        syscall::shm_destroy(shm_id);
        return false;
    }
    drag.bridge = Some(crate::DragBridge {
        shm_id,
        shm_addr: addr as u32,
        shm_cap: MAX_DRAG_PAYLOAD_BYTES,
        source_window_id,
        announced: false,
        image_shm_id: 0,
        image_addr: 0,
    });
    true
}

/// Allocate an SHM for the drag-image, copy the caller-provided ARGB
/// pixels into it, and send `CMD_DRAG_SET_IMAGE`. Replaces any previous
/// drag-image. Returns true on success.
pub fn install_drag_image(
    st: &mut crate::AnyuiState,
    pixels: &[u32],
    w: u32,
    h: u32,
    hot_x: i32,
    hot_y: i32,
) -> bool {
    let needed = (w as usize) * (h as usize) * 4;
    if needed == 0 || w > 1024 || h > 1024 || pixels.len() < (w * h) as usize {
        return false;
    }
    let source_window_id = match st.drag.as_ref().and_then(|d| d.bridge.as_ref()) {
        Some(b) => b.source_window_id,
        None => return false,
    };
    // Free a previous drag-image, if any.
    if let Some(bridge) = st.drag.as_mut().and_then(|d| d.bridge.as_mut()) {
        if bridge.image_shm_id != 0 {
            syscall::shm_unmap(bridge.image_shm_id);
            syscall::shm_destroy(bridge.image_shm_id);
            bridge.image_shm_id = 0;
            bridge.image_addr = 0;
        }
    }
    let shm_id = syscall::shm_create(needed as u32);
    if shm_id == 0 {
        return false;
    }
    let addr = syscall::shm_map(shm_id);
    if addr == 0 {
        syscall::shm_destroy(shm_id);
        return false;
    }
    // Copy pixels into SHM.
    unsafe {
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr(),
            addr as *mut u32,
            (w * h) as usize,
        );
    }
    if let Some(bridge) = st.drag.as_mut().and_then(|d| d.bridge.as_mut()) {
        bridge.image_shm_id = shm_id;
        bridge.image_addr = addr as u32;
    }
    let packed_size = (w << 16) | (h & 0xFFFF);
    let packed_hot = ((hot_x.max(0) as u32) << 16) | ((hot_y.max(0) as u32) & 0xFFFF);
    let cmd: [u32; 5] = [
        CMD_DRAG_SET_IMAGE,
        source_window_id,
        shm_id,
        packed_size,
        packed_hot,
    ];
    syscall::evt_chan_emit(st.channel_id, &cmd);
    true
}

/// Write the current `drag.data` into the bridge SHM and (re-)send
/// CMD_DRAG_BEGIN. Idempotent: called every time `anyui_drag_set_payload`
/// updates the payload, but only the *first* call announces the drag —
/// subsequent calls just overwrite the SHM contents for any future readers.
pub fn announce_drag(st: &mut AnyuiState) {
    let (data_ptr, data_len, format, allowed_effects, source_window_id, shm_addr, shm_cap, shm_id, announced) = {
        let drag = match st.drag.as_ref() {
            Some(d) => d,
            None => return,
        };
        let bridge = match drag.bridge.as_ref() {
            Some(b) => b,
            None => return,
        };
        (
            drag.data.as_ptr(),
            drag.data.len(),
            drag.format,
            drag.allowed_effects,
            bridge.source_window_id,
            bridge.shm_addr,
            bridge.shm_cap,
            bridge.shm_id,
            bridge.announced,
        )
    };
    // Copy payload into the SHM (truncate if too large).
    let n = core::cmp::min(data_len, shm_cap as usize);
    if n > 0 && shm_addr != 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(data_ptr, shm_addr as *mut u8, n);
        }
    }
    if !announced {
        let payload_len = n as u32;
        let packed = (payload_len << 8) | (allowed_effects & 0xFF);
        let cmd: [u32; 5] = [CMD_DRAG_BEGIN, source_window_id, format, shm_id, packed];
        syscall::evt_chan_emit(st.channel_id, &cmd);
        if let Some(drag) = st.drag.as_mut() {
            if let Some(bridge) = drag.bridge.as_mut() {
                bridge.announced = true;
            }
        }
    }
}

/// Tear down the source-side bridge: unmap and destroy the SHM. Called
/// after EVT_DRAG_END (the compositor guarantees no further target reads
/// will happen for this session — pending mappings in target processes
/// keep the underlying frames alive).
pub fn release_bridge(st: &mut AnyuiState) {
    let bridge = match st.drag.as_mut().and_then(|d| d.bridge.take()) {
        Some(b) => b,
        None => return,
    };
    syscall::shm_unmap(bridge.shm_id);
    syscall::shm_destroy(bridge.shm_id);
    if bridge.image_shm_id != 0 {
        syscall::shm_unmap(bridge.image_shm_id);
        syscall::shm_destroy(bridge.image_shm_id);
    }
}

/// Send CMD_DRAG_CANCEL for the active drag, if any. Used on ESC and on
/// abnormal teardown (caller's responsibility to also tear down `st.drag`).
pub fn send_cancel(st: &AnyuiState) {
    if let Some(drag) = st.drag.as_ref() {
        if let Some(bridge) = drag.bridge.as_ref() {
            if bridge.announced {
                let cmd: [u32; 5] = [CMD_DRAG_CANCEL, bridge.source_window_id, 0, 0, 0];
                syscall::evt_chan_emit(st.channel_id, &cmd);
            }
        }
    }
}

/// Send CMD_DRAG_ACCEPT for the current incoming drag, if any. Returns the
/// effect that the compositor will negotiate (also stored in incoming state
/// once it sends `EVT_DRAG_FEEDBACK`); the caller still has to mirror the
/// negotiated effect locally so reading it back is fast.
pub fn send_accept(st: &AnyuiState, requested_effects: u32) -> u32 {
    let inc = match st.incoming_drag.as_ref() {
        Some(i) => i,
        None => return 0,
    };
    let cmd: [u32; 5] = [
        CMD_DRAG_ACCEPT,
        inc.comp_window_id,
        requested_effects & 0x07,
        0,
        0,
    ];
    syscall::evt_chan_emit(st.channel_id, &cmd);
    // Mirror the source-side preference order so the local
    // `anyui_drag_get_effect` returns immediately even before EVT_DRAG_FEEDBACK
    // arrives.
    let overlap = inc.allowed_effects & (requested_effects & 0x07);
    if overlap & 0x02 != 0 {
        0x02
    } else if overlap & 0x01 != 0 {
        0x01
    } else if overlap & 0x04 != 0 {
        0x04
    } else {
        0
    }
}

/// Send CMD_DRAG_REJECT for the current incoming drag, if any.
pub fn send_reject(st: &AnyuiState) {
    if let Some(inc) = st.incoming_drag.as_ref() {
        let cmd: [u32; 5] = [CMD_DRAG_REJECT, inc.comp_window_id, 0, 0, 0];
        syscall::evt_chan_emit(st.channel_id, &cmd);
    }
}
