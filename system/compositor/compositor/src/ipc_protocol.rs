//! IPC protocol definitions for compositor ↔ app communication.
//!
//! Uses the anyOS event channel system (5 × u32 per event).
//! The compositor creates a channel named "compositor" and polls for commands.
//! Apps subscribe to the same channel to receive responses and input events.

// ── App → Compositor Commands ────────────────────────────────────────────────

/// Create a window: [CMD, app_tid, (width << 16) | height, (x << 16) | y, (shm_id << 16) | flags]
/// Width/height and x/y are packed as u16 pairs.
/// x = 0xFFFF (CW_USEDEFAULT) → compositor auto-places via cascading algorithm.
/// y = 0xFFFF (CW_USEDEFAULT) → compositor auto-places via cascading algorithm.
/// Compositor responds with RESP_WINDOW_CREATED.
pub const CMD_CREATE_WINDOW: u32 = 0x1001;

/// Sentinel value for x/y: compositor decides the position (cascading auto-placement).
pub const CW_USEDEFAULT: u16 = 0xFFFF;

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

/// Menu set acknowledged: [RESP, window_id, 0, app_tid, 0]
/// Sent after the compositor has parsed the menu SHM, so the app can free it.
pub const RESP_MENU_SET: u32 = 0x2003;

/// VRAM window created: [RESP, window_id, stride_pixels, app_tid, surface_va]
/// stride_pixels = fb_pitch/4 (the row stride for the VRAM surface).
/// surface_va = user virtual address of the VRAM surface (ready to write pixels).
/// App writes ARGB pixels to surface_va using stride_pixels as row stride.
pub const RESP_VRAM_WINDOW_CREATED: u32 = 0x2004;

/// VRAM window creation failed: [RESP, 0, 0, app_tid, 0]
/// Not enough off-screen VRAM or GPU not available. App should fall back to SHM.
pub const RESP_VRAM_WINDOW_FAILED: u32 = 0x2005;

/// Clipboard data response: [RESP, shm_id, len, format, requester_tid]
/// Sent in response to CMD_GET_CLIPBOARD. len=0 means clipboard is empty.
pub const RESP_CLIPBOARD_DATA: u32 = 0x2010;

/// Window position response: [RESP, window_id, content_x (as u32), content_y (as u32), requester_tid]
/// content_x/content_y are the screen coordinates of the window's content area top-left.
pub const RESP_WINDOW_POS: u32 = 0x2006;

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

/// Register an app's subscription ID for targeted event delivery.
/// [CMD, app_tid, sub_id, 0, 0]
/// Sent by the DLL during init so the compositor can use unicast emit
/// instead of broadcasting window events to all subscribers.
pub const CMD_REGISTER_SUB: u32 = 0x100C;

/// Set system theme (dark/light).
/// [CMD, theme_value, 0, 0, 0]   theme_value: 0=dark, 1=light
/// Compositor writes to shared DLL page and triggers repaint.
pub const CMD_SET_THEME: u32 = 0x100D;

/// Enable/disable blur-behind on a window.
/// [CMD, window_id, radius, 0, 0]
/// radius=0 disables blur. radius>0 enables blur with given kernel radius.
/// The compositor blurs the composited background behind the window layer
/// before alpha-blending the window on top (frosted glass effect).
pub const CMD_SET_BLUR_BEHIND: u32 = 0x100E;

/// Create a VRAM-direct window (app renders directly to off-screen VRAM).
/// [CMD, app_tid, width, height, flags]
/// Compositor allocates off-screen VRAM, maps it into the app's address space,
/// and responds with RESP_VRAM_WINDOW_CREATED containing the VRAM surface info.
/// On failure, responds with RESP_VRAM_WINDOW_FAILED (app should fall back to SHM).
pub const CMD_CREATE_VRAM_WINDOW: u32 = 0x1010;

/// Set desktop wallpaper by file path.
/// [CMD, shm_id, 0, 0, 0]
/// SHM contains a null-terminated UTF-8 path string (e.g. "/media/wallpapers/mountains.jpg").
/// Compositor maps SHM, reads path, loads wallpaper, unmaps SHM, damages all.
pub const CMD_SET_WALLPAPER: u32 = 0x100F;

/// Set clipboard contents.
/// [CMD, shm_id, len, format, 0]
/// SHM contains raw clipboard data (text or binary). Compositor copies data internally.
/// format: 0 = text/plain, 1 = text/uri-list
pub const CMD_SET_CLIPBOARD: u32 = 0x1011;

/// Get clipboard contents.
/// [CMD, shm_id, capacity, 0, 0]
/// App creates empty SHM with `capacity` bytes. Compositor writes clipboard data into it
/// and responds with RESP_CLIPBOARD_DATA.
pub const CMD_GET_CLIPBOARD: u32 = 0x1012;

// ── App → Compositor: Notification Commands ──────────────────────────────

/// Show a notification banner.
/// [CMD, sender_tid, shm_id, timeout_ms, flags]
/// SHM layout: [title_len: u16, msg_len: u16, has_icon: u8, pad: 3 bytes,
///              title_bytes..., msg_bytes..., icon_pixels (16×16 ARGB if has_icon)]
pub const CMD_SHOW_NOTIFICATION: u32 = 0x1020;

/// Dismiss a notification by ID.
/// [CMD, notification_id, 0, 0, 0]
pub const CMD_DISMISS_NOTIFICATION: u32 = 0x1021;

/// Get a window's content area screen position.
/// [CMD, window_id, requester_tid, 0, 0]
/// Compositor responds with RESP_WINDOW_POS containing content_x, content_y.
pub const CMD_GET_WINDOW_POS: u32 = 0x1013;

/// Inject a synthetic key event into the focused window.
/// [CMD, scancode, char_val, is_down (1=down/0=up), modifiers]
/// vncd maps RFB KeySyms → (scancode, char_val) before emitting this command.
/// Sent by vncd after successful authentication to relay keyboard input.
pub const CMD_INJECT_KEY: u32 = 0x1022;

/// Inject a synthetic pointer (mouse) event and move the cursor.
/// [CMD, x (abs screen coords), y (abs screen coords), buttons_mask, 0]
/// buttons_mask: bit 0 = left, bit 1 = middle, bit 2 = right (RFB convention).
/// Sent by vncd to relay VNC client pointer events into the desktop.
pub const CMD_INJECT_POINTER: u32 = 0x1023;

// ── Compositor → App: Notification Events ────────────────────────────────

/// Notification clicked by user: [EVT, notification_id, sender_tid, 0, 0]
pub const EVT_NOTIFICATION_CLICK: u32 = 0x3010;

/// Notification dismissed: [EVT, notification_id, sender_tid, reason, 0]
/// reason: 0 = timeout, 1 = user click, 2 = programmatic dismiss
pub const EVT_NOTIFICATION_DISMISSED: u32 = 0x3011;

/// Theme changed notification (compositor → apps via channel).
/// [EVT, new_theme, old_theme, 0, 0]
pub const EVT_THEME_CHANGED: u32 = 0x0050;

// ── Compositor → App: Menu & Status Icon Events ─────────────────────────────

/// Menu item selected: [EVT, window_id, menu_index, item_id, 0]
pub const EVT_MENU_ITEM: u32 = 0x3008;

/// Status icon clicked: [EVT, 0, app_tid, icon_id, 0]
pub const EVT_STATUS_ICON_CLICK: u32 = 0x3009;

/// Mouse move: [EVT, window_id, local_x, local_y, 0]
pub const EVT_MOUSE_MOVE: u32 = 0x300A;

/// Frame acknowledgment: [EVT, window_id, 0, 0, 0]
/// Sent by the render thread after compositing a window's content to screen
/// (i.e. after VSync — RESOURCE_FLUSH completion on VirtIO-GPU).
/// Apps use this for back-pressure: don't present a new frame until ACK.
pub const EVT_FRAME_ACK: u32 = 0x300B;

/// Focus lost: [EVT, window_id, 0, 0, 0]
/// Sent when a window loses focus (another window was clicked or desktop background).
pub const EVT_FOCUS_LOST: u32 = 0x300C;

/// Window opened (broadcast): [EVT, app_tid, win_id, 0, 0]
/// Emitted when any app creates a window. Used by dock for filtering.
pub const EVT_WINDOW_OPENED: u32 = 0x0060;

/// Window closed (broadcast): [EVT, exited_tid, 0, 0, 0]
/// Emitted when a process with windows exits.
pub const EVT_WINDOW_CLOSED: u32 = 0x0061;

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
