//! IPC protocol definitions for compositor ↔ app communication.
//!
//! Uses the anyOS event channel system (5 × u32 per event).
//! The compositor creates a channel named "compositor" and polls for commands.
//! Apps subscribe to the same channel to receive responses and input events.

// ── App → Compositor Commands ────────────────────────────────────────────────

/// Create a window: [CMD, app_tid, width, height, (shm_id << 16) | flags]
/// Compositor responds with RESP_WINDOW_CREATED.
pub const CMD_CREATE_WINDOW: u32 = 0x1001;

/// Destroy a window: [CMD, window_id, 0, 0, 0]
pub const CMD_DESTROY_WINDOW: u32 = 0x1002;

/// Present (window content updated): [CMD, window_id, shm_id, 0, 0]
/// Compositor marks the window layer dirty for recomposition.
pub const CMD_PRESENT: u32 = 0x1003;

/// Set window title (packed ASCII): [CMD, window_id, chars[0..3], chars[4..7], chars[8..11]]
/// Up to 12 ASCII characters, null-terminated.
pub const CMD_SET_TITLE: u32 = 0x1004;

/// Move a window: [CMD, window_id, x (as i32), y (as i32), 0]
pub const CMD_MOVE_WINDOW: u32 = 0x1005;

// ── Compositor → App Responses ───────────────────────────────────────────────

/// Window created: [RESP, window_id, shm_id, app_tid, 0]
pub const RESP_WINDOW_CREATED: u32 = 0x2001;

/// Window destroyed: [RESP, window_id, app_tid, 0, 0]
pub const RESP_WINDOW_DESTROYED: u32 = 0x2002;

// ── Compositor → App Input Events ────────────────────────────────────────────

/// Key down: [EVT, window_id, scancode, char_code, modifiers]
pub const EVT_KEY_DOWN: u32 = 0x3001;

/// Key up: [EVT, window_id, scancode, char_code, modifiers]
pub const EVT_KEY_UP: u32 = 0x3002;

/// Mouse down: [EVT, window_id, local_x, local_y, buttons]
pub const EVT_MOUSE_DOWN: u32 = 0x3003;

/// Mouse up: [EVT, window_id, local_x, local_y, 0]
pub const EVT_MOUSE_UP: u32 = 0x3004;

/// Mouse scroll: [EVT, window_id, dz (signed), 0, 0]
pub const EVT_MOUSE_SCROLL: u32 = 0x3005;

/// Resize: [EVT, window_id, new_width, new_height, 0]
pub const EVT_RESIZE: u32 = 0x3006;

/// Window close requested: [EVT, window_id, 0, 0, 0]
pub const EVT_WINDOW_CLOSE: u32 = 0x3007;

// ── App → Compositor: Menu & Status Icon Commands ───────────────────────────

/// Set a window's menu bar definition via SHM.
/// [CMD, window_id, shm_id, 0, 0]
/// The SHM region contains a flat MenuBarDef structure (see menu.rs).
/// Compositor reads and parses the SHM, then unmaps it. App can free SHM after.
pub const CMD_SET_MENU: u32 = 0x1006;

/// Register a status icon in the menubar.
/// [CMD, app_tid, icon_id, shm_id, 0]
/// SHM contains 16x16 ARGB pixel data (16*16*4 = 1024 bytes).
pub const CMD_ADD_STATUS_ICON: u32 = 0x1007;

/// Remove a status icon from the menubar.
/// [CMD, app_tid, icon_id, 0, 0]
pub const CMD_REMOVE_STATUS_ICON: u32 = 0x1008;

/// Update a menu item's flags (enable/disable/check).
/// [CMD, window_id, item_id, new_flags, 0]
/// No SHM required — fits in 5 words.
pub const CMD_UPDATE_MENU_ITEM: u32 = 0x1009;

/// Focus a window by its owner TID.
/// [CMD, owner_tid, 0, 0, 0]
/// If a window with the given owner TID exists, focus and raise it.
pub const CMD_FOCUS_BY_TID: u32 = 0x100A;

/// Notify compositor of a new (resized) SHM for a window.
/// [CMD, window_id, new_shm_id, new_width, new_height]
/// App creates the new SHM, compositor maps it and unmaps the old one.
pub const CMD_RESIZE_SHM: u32 = 0x100B;

// ── Compositor → App: Menu & Status Icon Events ─────────────────────────────

/// Menu item selected: [EVT, window_id, menu_index, item_id, 0]
pub const EVT_MENU_ITEM: u32 = 0x3008;

/// Status icon clicked: [EVT, 0, app_tid, icon_id, 0]
pub const EVT_STATUS_ICON_CLICK: u32 = 0x3009;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Pack up to 12 ASCII characters into 3 u32 words.
pub fn pack_title(title: &str) -> [u32; 3] {
    let bytes = title.as_bytes();
    let mut packed = [0u32; 3];
    for i in 0..12.min(bytes.len()) {
        packed[i / 4] |= (bytes[i] as u32) << ((i % 4) * 8);
    }
    packed
}

/// Unpack 3 u32 words into a title string (up to 12 chars).
pub fn unpack_title(words: [u32; 3]) -> [u8; 12] {
    let mut buf = [0u8; 12];
    for i in 0..12 {
        buf[i] = ((words[i / 4] >> ((i % 4) * 8)) & 0xFF) as u8;
    }
    buf
}
