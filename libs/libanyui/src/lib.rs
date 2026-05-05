//! libanyui — Object-oriented UI framework for anyOS.
//!
//! All widgets derive from the `Control` trait (virtual base class), which
//! provides common properties and virtual methods. Concrete types
//! (Label, Button, TextField, etc.) live in `controls/`, each in its own file.
//!
//! # Architecture
//! - Global `AnyuiState` owns all controls as `Vec<Box<dyn Control>>`.
//! - Each control has an ID (`ControlId`), parent, children, and a `ControlKind`.
//! - Window management goes through libcompositor.dlib (user-space compositor),
//!   NOT kernel syscalls. Windows have shared memory (SHM) pixel surfaces.
//! - The event loop polls compositor events, dispatches via hit-testing and
//!   virtual method calls, and invokes registered callbacks.
//! - Rendering uses draw.rs (Surface-based) with librender/libfont DLLs
//!   for themed control drawing and direct SHM surface writes.
//!
//! # Event Model
//! Base events are fired for ALL controls automatically:
//! - MouseEnter / MouseLeave — hover tracking
//! - MouseDown / MouseUp — raw pointer press/release
//! - Click — synthesized from down+up on same control
//! - DoubleClick — two clicks within 400ms
//! - Focus / Blur — keyboard focus changes
//! - KeyDown, Scroll, Drag
//!
//! # Export Convention
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.

#![no_std]
#![no_main]

extern crate alloc;

/// Debug logging macro — writes to serial via SYS_WRITE(fd=1).
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = $crate::LogBuf([0u8; 256], 0);
        let _ = core::write!(&mut buf, $($arg)*);
        buf.0[buf.1.min(255)] = b'\n';
        let len = (buf.1 + 1).min(256);
        $crate::syscall::write(1, &buf.0[..len]);
    }};
}

/// Small stack buffer for log formatting.
pub struct LogBuf(pub [u8; 256], pub usize);

impl core::fmt::Write for LogBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.1 < 255 {
                self.0[self.1] = b;
                self.1 += 1;
            }
        }
        Ok(())
    }
}

pub(crate) mod accessibility;
mod compositor;
mod control;
mod controls;
mod dialogs;
pub mod dnd;
pub mod dnd_ipc;
pub mod draw;
mod event_loop;
pub mod font_bitmap;
pub mod icons;
mod layout;
mod marshal;
pub mod syscall;
pub mod theme;
mod timer;

use alloc::boxed::Box;
use alloc::vec::Vec;
use control::{Callback, Control, ControlId, ControlKind, DockStyle, Orientation};

// ── Compositor window handle ─────────────────────────────────────────

/// Per-window compositor state (SHM surface + IDs).
///
/// `width`/`height` are **physical** pixel dimensions (used for SHM surface
/// allocation and back-buffer sizing). `logical_width`/`logical_height` are
/// what the application (control tree) sees — they equal `physical / scale`.
pub(crate) struct CompWindow {
    pub window_id: u32,
    pub shm_id: u32,
    pub surface: *mut u32,
    /// Physical width (= logical * scale_factor / 100). Used for SHM and back-buffer.
    pub width: u32,
    /// Physical height (= logical * scale_factor / 100). Used for SHM and back-buffer.
    pub height: u32,
    /// Logical width — what the control tree and application see.
    pub logical_width: u32,
    /// Logical height — what the control tree and application see.
    pub logical_height: u32,
    /// Back-pressure: true after present(), cleared on EVT_FRAME_ACK from compositor.
    pub frame_presented: bool,
    /// A fully rendered frame is staged in `back_buffer` and still needs upload
    /// to the SHM surface once the compositor has ACKed the previous frame.
    pub present_pending: bool,
    /// Pending physical dirty rect to upload from `back_buffer` into SHM.
    /// `None` means the next upload must present the full window.
    pub pending_present_rect: Option<(i32, i32, u32, u32)>,
    /// Timestamp of last present() call (for safety timeout).
    pub last_present_ms: u32,
    /// Window-level dirty flag: true if any control in this window's subtree is dirty.
    /// Computed in a flat O(n) scan, replacing the O(n²) recursive any_dirty() tree walk.
    pub dirty: bool,
    /// Accumulated dirty region (union of all dirty controls' bounding rects).
    /// `None` means full-window redraw (first frame, resize, etc.).
    /// `Some((x, y, w, h))` in window-local coordinates for partial redraw.
    pub dirty_rect: Option<(i32, i32, u32, u32)>,
    /// Local back buffer for flicker-free rendering. All drawing goes here first,
    /// then a single memcpy to SHM before present() — the compositor never sees
    /// a half-rendered frame (no background flash, no partial content).
    pub back_buffer: Vec<u32>,
    /// Saved logical size before fullscreen (for restore on exit).
    pub saved_logical_size_fs: Option<(u32, u32)>,
    /// Cached compositor-owned window-frame x in physical pixels.
    /// Updated on EVT_WINDOW_MOVED; queried by Window::get_position().
    /// Stored in *physical* coords because the underlying compositor
    /// API (`compositor::get_window_position`) and existing callers in
    /// event_loop already work in physical, and the libanyui_client
    /// `get_position` FFI returns physical too — converting on read
    /// would break those call sites.
    pub frame_x: i32,
    pub frame_y: i32,
}

// ── Context menu popup window ─────────────────────────────────────────

/// State for a context menu rendered as a separate compositor popup window.
/// The popup is borderless, always-on-top, and destroyed when the menu is dismissed.
pub(crate) struct PopupInfo {
    pub window_id: u32,
    pub shm_id: u32,
    pub surface: *mut u32,
    pub width: u32,
    pub height: u32,
    pub back_buffer: Vec<u32>,
    /// The context menu control being displayed in the popup.
    pub menu_id: ControlId,
    /// Index into st.windows for the parent window that owns the menu.
    #[allow(dead_code)]
    pub owner_win_idx: usize,
    /// Shadow margin (pixels of padding around the menu for shadow rendering).
    pub margin: i32,
    /// Whether the popup needs to be re-rendered (independent of control dirty flag,
    /// because Phase 4's clear_dirty clears the menu control's flag before popup render).
    pub dirty: bool,
    /// If this popup was opened by a DropDown, its control ID.
    /// When the popup item is selected, the DropDown's state is updated.
    pub owner_dropdown: Option<ControlId>,
    /// If this popup was opened by a ComboBox, its control ID.
    /// When the popup item is selected, the ComboBox's state is updated.
    pub owner_combobox: Option<ControlId>,
    /// If this popup was opened by an AutoCompleteTextField, its control ID.
    /// When the popup item is selected, the TextField's text is updated.
    pub owner_autocomplete: Option<ControlId>,
    /// If this popup is the built-in Cut/Copy/Paste/Select All menu shown
    /// on right-click in a text-input control, the target control's ID.
    pub owner_text_edit: Option<ControlId>,
}

/// Source-side payload SHM tied to a `DragSession`. Allocated lazily when
/// the source first sets a payload, kept alive until the compositor sends
/// `EVT_DRAG_END`, then unmapped + destroyed.
pub(crate) struct DragBridge {
    pub shm_id: u32,
    pub shm_addr: u32,
    pub shm_cap: u32,
    /// Compositor window-id of the source window (needed for CMD_DRAG_*).
    pub source_window_id: u32,
    /// True after we've sent CMD_DRAG_BEGIN to the compositor.
    pub announced: bool,
    /// Drag-image SHM allocated by `drag_set_image` (0 = no image).
    pub image_shm_id: u32,
    /// Local mapping of the drag-image SHM (0 = not mapped).
    pub image_addr: u32,
}

/// Target-side state for an incoming cross-process drag. Created on
/// `EVT_DRAG_ENTER` for one of our windows, updated on `EVT_DRAG_OVER`,
/// torn down on `EVT_DRAG_LEAVE` / `EVT_DROP`.
pub(crate) struct IncomingDrag {
    /// The window-id this drag is currently over (one of our windows).
    pub window_id: ControlId,
    /// Compositor window-id (the cross-process equivalent).
    pub comp_window_id: u32,
    /// Payload SHM id received from the source via the compositor.
    pub payload_shm_id: u32,
    /// Local mapping of the payload SHM (0 if not yet mapped).
    pub payload_addr: u32,
    /// Total payload length.
    pub payload_len: u32,
    /// Payload format (`dnd::DND_FORMAT_*`).
    pub format: u32,
    /// Effects the source allows.
    pub allowed_effects: u32,
    /// Source process TID.
    pub source_tid: u32,

    /// Currently hovered control inside this window, if any.
    pub target_control: Option<ControlId>,
    /// Whether the current control accepted via `anyui_drag_accept`.
    pub target_accepted: bool,
    /// Negotiated effect (after target accept).
    pub negotiated_effect: u32,

    /// Last reported pointer position (window-local logical pixels).
    pub pointer_x: i32,
    pub pointer_y: i32,
    /// Last reported modifier bits (Ctrl=1, Shift=2).
    pub modifiers: u32,
    /// Modifier bits at the last accept; when current `modifiers` differs,
    /// acceptance is reset so the target re-evaluates on EVT_DRAG_OVER.
    pub accept_modifiers: u32,
}

/// Active drag-and-drop session owned by the framework.
pub(crate) struct DragSession {
    /// Control that initiated the drag.
    pub source_id: ControlId,
    /// Current active drop target under the pointer, if any.
    pub target_id: Option<ControlId>,
    /// Opaque payload bytes set by the source (any `dnd::DND_FORMAT_*`).
    pub data: Vec<u8>,
    /// Payload format identifier (see `dnd::DND_FORMAT_*`).
    pub format: u32,
    /// Bitmask of effects the source is willing to allow (`dnd::DND_EFFECT_*`).
    pub allowed_effects: u32,
    /// Effect the current target accepted; `DND_EFFECT_NONE` until the target
    /// calls `anyui_drag_accept`. Persists across over-events on the same
    /// target as long as modifiers stay unchanged; reset on target change or
    /// modifier change so apps can re-negotiate.
    pub negotiated_effect: u32,
    /// True after the current target called `anyui_drag_accept`. Persists
    /// across over-events on the same target as long as modifiers stay
    /// unchanged. Drops are silently rejected if this is false.
    pub target_accepted: bool,
    /// Current pointer position in logical window-relative pixels.
    pub pointer_x: i32,
    pub pointer_y: i32,
    /// Ctrl/Shift modifier bits from the latest pointer event (bit 0 = Ctrl,
    /// bit 1 = Shift). Used by `dnd::negotiate_effect`.
    pub modifiers: u32,
    /// Source-side cross-process bridge: the payload SHM allocated by us
    /// and shared with the compositor + target processes. `None` until the
    /// drag has been announced to the compositor (after the DRAG_START
    /// callback set the payload).
    pub bridge: Option<DragBridge>,
    /// Modifier bits captured the last time the target accepted. When
    /// `modifiers` differs from this, acceptance is reset so the target's
    /// `EVENT_DRAG` handler can re-evaluate based on the new modifiers.
    pub accept_modifiers: u32,
}

// ── Modal dialog tracking ─────────────────────────────────────────────

/// An active modal context. Tracks the relationship between a modal dialog
/// and its blocked owner window.
pub(crate) struct ModalEntry {
    /// ControlId of the modal window (for separate-window modals like VMManager Settings).
    /// 0 for in-window overlays (MessageBox, FileDialog).
    pub modal_win_id: ControlId,
    /// ControlId of the blocked owner window.
    pub owner_win_id: ControlId,
    /// ControlId of the overlay control (for in-window modals). 0 for separate-window modals.
    pub overlay_id: ControlId,
}

// ── Global state (per-process, lives in .data/.bss of the .so) ───────

pub(crate) struct AnyuiState {
    pub controls: Vec<Box<dyn Control>>,
    pub next_id: ControlId,
    /// Top-level window ControlIds.
    pub windows: Vec<ControlId>,
    /// Compositor window handles, parallel to `windows`.
    pub comp_windows: Vec<CompWindow>,
    /// Set to true when anyui_quit() is called.
    pub quit_requested: bool,

    // ── Compositor connection ────────────────────────────────────────
    pub channel_id: u32,
    pub reply_channel_id: u32,
    pub sub_id: u32,

    // ── Event tracking ──────────────────────────────────────────────
    /// Currently focused control (receives keyboard events).
    pub focused: Option<ControlId>,
    /// Currently pressed control (mouse button held down).
    pub pressed: Option<ControlId>,
    /// Currently hovered control (mouse cursor is over).
    pub hovered: Option<ControlId>,
    /// Control ID of the last click (for multi-click detection).
    pub last_click_id: Option<ControlId>,
    /// Timestamp (ms) of the last click (for multi-click detection).
    pub last_click_tick: u32,
    /// Click count (1=single, 2=double, 3=triple) for multi-click detection.
    pub click_count: u32,
    /// Which mouse button was pressed (for right-click detection).
    pub pressed_button: u32,
    /// Last known mouse position (logical pixels, window-relative).
    pub last_mouse_x: i32,
    /// Last known mouse position (logical pixels, window-relative).
    pub last_mouse_y: i32,
    /// Pointer location where the current press began.
    pub press_mouse_x: i32,
    /// Pointer location where the current press began.
    pub press_mouse_y: i32,

    // ── Tooltip ──────────────────────────────────────────────────────
    /// Framework-managed tooltip control ID (created lazily on first use).
    pub active_tooltip: Option<ControlId>,
    /// Control ID waiting for tooltip display (pending hover delay).
    pub tooltip_pending_id: Option<ControlId>,
    /// Timestamp (ms) when hover started on the pending tooltip control.
    pub tooltip_hover_start: u32,

    // ── Context menu popup ──────────────────────────────────────────
    /// Active popup window for context menus (at most one at a time).
    pub popup: Option<PopupInfo>,
    /// Active drag-and-drop session, if a drag gesture is in progress.
    pub drag: Option<DragSession>,
    /// Set when the drag has ended (mouse-up) but the DROP / DRAG_END
    /// callbacks are still queued. The session is kept alive across the
    /// callback dispatch so handlers can still call `anyui_drag_get_payload`
    /// and friends; cleared at the end of the event-loop iteration.
    pub drag_release_pending: bool,
    /// Incoming cross-process drag — present whenever the cursor (or a
    /// drag pointer) is over one of our windows during an active global
    /// drag. Created on `EVT_DRAG_ENTER`, updated on `EVT_DRAG_OVER`,
    /// torn down on `EVT_DRAG_LEAVE` / `EVT_DROP`.
    pub incoming_drag: Option<IncomingDrag>,
    /// Set when a DROP has fired and the SHM mapping should be released
    /// after the post-Phase-3 cleanup pass (so DROP callbacks can still
    /// read the payload via `anyui_drag_get_payload`).
    pub incoming_release_pending: bool,

    // ── Timers ───────────────────────────────────────────────────────
    pub timers: timer::TimerState,

    // ── Dirty tracking (push-based, avoids per-frame O(n) scans) ─────
    /// True when at least one control has been marked dirty since last render.
    pub needs_repaint: bool,
    /// True when layout-affecting properties changed since last layout pass.
    pub needs_layout: bool,

    // ── Last key event (queryable by callbacks) ──────────────────────
    /// Keycode from the most recent KEY_DOWN event.
    pub last_keycode: u32,
    /// Character code from the most recent KEY_DOWN event.
    pub last_char_code: u32,
    /// Modifier flags from the most recent KEY_DOWN event.
    pub last_modifiers: u32,

    // ── Tracked modifier state (safety net for modifier loss) ──────
    /// Modifier bits tracked independently from KEY_DOWN/KEY_UP keycode
    /// observations.  Merged (OR'd) with each event's own `modifiers`
    /// field before dispatching to controls.
    pub tracked_modifiers: u32,
    /// Current cursor shape sent to compositor (0=Arrow, 1=ResizeEW, 2=ResizeNS).
    /// Tracked to avoid redundant CMD_SET_CURSOR messages.
    pub current_cursor: u32,

    // ── Window lifecycle callbacks (for dock/system integration) ──────
    /// Callback for EVT_WINDOW_OPENED (0x0060). Called with (app_tid, 0x0060, userdata).
    pub on_window_opened: Option<(Callback, u64)>,
    /// Callback for EVT_WINDOW_CLOSED (0x0061). Called with (app_tid, 0x0061, userdata).
    pub on_window_closed: Option<(Callback, u64)>,
    /// Callback for EVT_WINDOW_LIST_END (0x0065). Called once per request_window_list()
    /// with (count, 0x0065, userdata). The collected TIDs are in `window_list_buffer`.
    pub on_window_list: Option<(Callback, u64)>,
    /// Buffer for accumulating EVT_WINDOW_LIST_ENTRY TIDs between request and end marker.
    pub window_list_buffer: Vec<u32>,

    // ── Modal dialog stack ──────────────────────────────────────────
    /// Stack of active modal contexts. Last entry = topmost modal.
    /// When non-empty, only the topmost modal's window receives input events.
    pub modal_stack: Vec<ModalEntry>,

    // ── Tray icon callbacks ─────────────────────────────────────────
    /// Registered tray icon click callbacks: (icon_id, callback, userdata).
    pub tray_callbacks: Vec<(u32, Callback, u64)>,

    // ── Menu item callbacks ─────────────────────────────────────────
    /// Registered menu item callbacks: (window_control_id, callback, userdata).
    /// Callback receives (item_id, EVT_MENU_ITEM, userdata).
    pub menu_callbacks: Vec<(u32, Callback, u64)>,

    // ── Accessibility / UI-Automation ───────────────────────────────
    /// Accessibility pipe server state (None on host build / before init).
    pub acc: Option<crate::accessibility::AccState>,
}

/// Signal that at least one control needs repainting.
/// Called from `ControlBase::mark_dirty()` — avoids per-frame O(n) dirty scan.
pub(crate) fn mark_needs_repaint() {
    unsafe {
        if let Some(ref mut st) = STATE {
            st.needs_repaint = true;
        }
    }
}

/// Signal that layout needs to be recalculated.
/// Called when dock, padding, margin, visibility, or children change.
pub(crate) fn mark_needs_layout() {
    unsafe {
        if let Some(ref mut st) = STATE {
            st.needs_layout = true;
        }
    }
}

static mut STATE: Option<AnyuiState> = None;

pub(crate) fn state() -> &'static mut AnyuiState {
    unsafe {
        (*core::ptr::addr_of_mut!(STATE))
            .as_mut()
            .expect("anyui not initialized")
    }
}

/// Fullscreen info: low 32 bits = (width<<16)|height, high 32 bits = stride.
pub(crate) static FULLSCREEN_INFO: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Fullscreen direct framebuffer pointer (0 if not in fullscreen or SHM mode).
pub(crate) static FULLSCREEN_FB_PTR: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

fn make_compositor_reply_channel_name(tid: u32) -> ([u8; 32], u32) {
    let prefix = b"compositor.reply.";
    let mut name = [0u8; 32];
    let mut pos = 0usize;
    while pos < prefix.len() {
        name[pos] = prefix[pos];
        pos += 1;
    }
    if tid == 0 {
        name[pos] = b'0';
        pos += 1;
    } else {
        let mut digits = [0u8; 10];
        let mut n = tid;
        let mut d = 0usize;
        while n > 0 {
            digits[d] = b'0' + (n % 10) as u8;
            n /= 10;
            d += 1;
        }
        while d > 0 {
            d -= 1;
            name[pos] = digits[d];
            pos += 1;
        }
    }
    (name, pos as u32)
}

// ══════════════════════════════════════════════════════════════════════
//  Exported C API
// ══════════════════════════════════════════════════════════════════════

/// Initialize the anyui framework. Must be called before any other function.
/// Connects to the compositor via libcompositor.dlib. Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_init() -> u32 {
    let already_initialized = unsafe { (*core::ptr::addr_of!(STATE)).is_some() };
    if already_initialized {
        return 1;
    }

    let mut sub_id: u32 = 0;
    let channel_id = compositor::init(&mut sub_id);
    if channel_id == 0 {
        return 0;
    }
    let (reply_name, reply_len) = make_compositor_reply_channel_name(syscall::get_tid());
    let reply_channel_id = syscall::evt_chan_create(reply_name.as_ptr(), reply_len);
    if reply_channel_id == 0 {
        return 0;
    }

    // Load theme palettes from /System/compositor/themes/{dark,light}.conf.
    // Falls back to built-in defaults for missing files / keys.
    theme::load_from_disk();

    // Read the current DPI scale factor from the shared page so that
    // scale()/scale_i32() return correct values from the very first call
    // (before the event loop starts refreshing the cache every frame).
    theme::refresh_scale_cache();

    unsafe {
        STATE = Some(AnyuiState {
            controls: Vec::new(),
            next_id: 1,
            windows: Vec::new(),
            comp_windows: Vec::new(),
            quit_requested: false,
            channel_id,
            reply_channel_id,
            sub_id,
            focused: None,
            pressed: None,
            hovered: None,
            last_click_id: None,
            last_click_tick: 0,
            click_count: 0,
            pressed_button: 0,
            last_mouse_x: 0,
            last_mouse_y: 0,
            press_mouse_x: 0,
            press_mouse_y: 0,
            active_tooltip: None,
            tooltip_pending_id: None,
            tooltip_hover_start: 0,
            popup: None,
            drag: None,
            drag_release_pending: false,
            incoming_drag: None,
            incoming_release_pending: false,
            timers: timer::TimerState::new(),
            needs_repaint: true,
            needs_layout: true,
            last_keycode: 0,
            last_char_code: 0,
            last_modifiers: 0,
            tracked_modifiers: 0,
            current_cursor: 0,
            on_window_opened: None,
            on_window_closed: None,
            on_window_list: None,
            window_list_buffer: Vec::new(),
            modal_stack: Vec::new(),
            tray_callbacks: Vec::new(),
            menu_callbacks: Vec::new(),
            acc: {
                let pid = libsyscall::get_tid();
                if pid != 0 {
                    Some(crate::accessibility::AccState::new(pid))
                } else {
                    None
                }
            },
        });
    }
    1
}

/// Shut down the anyui framework and destroy all windows.
#[no_mangle]
pub extern "C" fn anyui_shutdown() {
    let st = state();
    let channel_id = st.channel_id;
    // Destroy popup window if active
    if let Some(popup) = st.popup.take() {
        compositor::destroy_window(channel_id, popup.window_id, popup.shm_id);
    }
    for cw in &st.comp_windows {
        compositor::destroy_window(channel_id, cw.window_id, cw.shm_id);
    }
    unsafe {
        STATE = None;
    }
}

// ── Control creation ─────────────────────────────────────────────────

/// Create a top-level window at position (x, y). Returns a ControlId (0 on failure).
/// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
/// flags: window flags (borderless, shadow, etc.) — 0 for default decorated window.
#[no_mangle]
pub extern "C" fn anyui_create_window(
    title: *const u8,
    title_len: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    flags: u32,
) -> ControlId {
    let st = state();
    let id = st.next_id;
    st.next_id += 1;

    // Copy title
    let len = (title_len as usize).min(63);
    let mut title_buf = [0u8; 64];
    if !title.is_null() && len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(title, title_buf.as_mut_ptr(), len);
        }
    }

    // Ensure we have the latest scale factor from the shared page before
    // computing physical dimensions (the event loop hasn't started yet on
    // the first window creation).
    crate::theme::refresh_scale_cache();

    // All coordinates from the app are LOGICAL pixels. Scale to physical.
    // x/y == -1 is the auto-placement sentinel (CW_USEDEFAULT = 0xFFFF) —
    // pass it through unscaled so the compositor detects it correctly.
    let phys_x = if x == -1 {
        -1
    } else {
        crate::theme::scale_i32(x)
    };
    let phys_y = if y == -1 {
        -1
    } else {
        crate::theme::scale_i32(y)
    };
    let phys_w = crate::theme::scale(w);
    let phys_h = crate::theme::scale(h);

    // Create compositor window via DLL — physical pixel dimensions.
    // Set WIN_FLAG_DPI_AWARE (0x200) so the compositor knows this window
    // renders at physical resolution and does not need content upscaling.
    let dpi_flags = flags | 0x200;
    let (window_id, shm_id, surface) = match compositor::create_window(
        st.channel_id,
        st.sub_id,
        phys_x,
        phys_y,
        phys_w,
        phys_h,
        dpi_flags,
    ) {
        Some(result) => result,
        None => return 0,
    };

    // Set title
    compositor::set_title(st.channel_id, window_id, &title_buf[..len]);

    // The Window control keeps logical dimensions (w, h) — the control tree
    // always works in logical coordinates.
    let ctrl = controls::create_control(ControlKind::Window, id, 0, 0, 0, w, h, &title_buf[..len]);
    st.controls.push(ctrl);
    st.windows.push(id);
    let pixel_count = (phys_w as usize)
        .saturating_mul(phys_h as usize)
        .min(16384 * 16384);
    // Resolve the spawn-time frame position. -1 sentinels mean the
    // compositor auto-placed the window; ask it where. The compositor
    // returns *content-area* coords (physical pixels including title
    // bar offset), which is exactly what `Window::get_position()` has
    // historically exposed via this same call from popup-positioning
    // call sites — keep the convention identical so existing callers
    // continue to work unchanged.
    let (initial_frame_x, initial_frame_y) =
        compositor::get_window_position(st.channel_id, st.sub_id, window_id);
    st.comp_windows.push(CompWindow {
        window_id,
        shm_id,
        surface,
        width: phys_w,
        height: phys_h,
        logical_width: w,
        logical_height: h,
        frame_presented: false,
        present_pending: false,
        pending_present_rect: None,
        last_present_ms: 0,
        dirty: true,
        dirty_rect: None,
        back_buffer: alloc::vec![0u32; pixel_count],
        saved_logical_size_fs: None,
        frame_x: initial_frame_x,
        frame_y: initial_frame_y,
    });
    id
}

/// Add a control as a child of `parent`. Returns the new ControlId.
///
/// `kind` selects the control type (see `ControlKind` values 0-36).
/// `text` + `text_len` provide initial text content.
#[no_mangle]
pub extern "C" fn anyui_add_control(
    parent: ControlId,
    kind: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    text: *const u8,
    text_len: u32,
) -> ControlId {
    let st = state();
    let id = st.next_id;
    st.next_id += 1;

    let ck = ControlKind::from_u32(kind);

    let text_slice = if !text.is_null() && text_len > 0 {
        unsafe { core::slice::from_raw_parts(text, text_len as usize) }
    } else {
        &[]
    };

    let ctrl = controls::create_control(ck, id, parent, x, y, w, h, text_slice);
    st.controls.push(ctrl);

    // Add to parent's children
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent) {
        p.add_child(id);
    }

    id
}

/// Create a standalone control (not yet parented). Returns the new ControlId.
///
/// The control is created with position (0,0) and default size for its kind.
/// Use `anyui_set_position()`, `anyui_set_size()` to configure, then
/// `anyui_add_child()` to attach it to a parent container.
#[no_mangle]
pub extern "C" fn anyui_create_control(kind: u32, text: *const u8, text_len: u32) -> ControlId {
    let st = state();
    let id = st.next_id;
    st.next_id += 1;

    let ck = ControlKind::from_u32(kind);
    let (dw, dh) = ck.default_size();

    let text_slice = if !text.is_null() && text_len > 0 {
        unsafe { core::slice::from_raw_parts(text, text_len as usize) }
    } else {
        &[]
    };

    let ctrl = controls::create_control(ck, id, 0, 0, 0, dw, dh, text_slice);
    st.controls.push(ctrl);
    id
}

/// Attach a child control to a parent container.
///
/// Sets the child's parent and adds it to the parent's children list.
#[no_mangle]
pub extern "C" fn anyui_add_child(parent: ControlId, child: ControlId) {
    let st = state();
    // Set parent on child
    if let Some(c) = st.controls.iter_mut().find(|c| c.id() == child) {
        c.set_parent(parent);
    }
    // Add to parent's children list
    let parent_is_radio_group = st
        .controls
        .iter()
        .find(|c| c.id() == parent)
        .map(|c| c.kind() == control::ControlKind::RadioGroup)
        .unwrap_or(false);
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent) {
        p.add_child(child);
    }
    // If parent is a RadioGroup, set group pointer on the child RadioButton
    if parent_is_radio_group {
        if let Some(c) = st.controls.iter_mut().find(|c| c.id() == child) {
            c.set_radio_group(parent);
        }
    }
    mark_needs_layout();
}

// ── Properties ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_text(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if !text.is_null() && len > 0 {
            let slice = unsafe { core::slice::from_raw_parts(text, len as usize) };
            ctrl.set_text(slice);
        } else {
            ctrl.set_text(&[]);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_text(id: ControlId, buf: *mut u8, max_len: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let t = ctrl.text();
        let copy_len = t.len().min(max_len as usize);
        if !buf.is_null() && copy_len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(t.as_ptr(), buf, copy_len);
            }
        }
        return copy_len as u32;
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_set_position(id: ControlId, x: i32, y: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_position(x, y);
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_size(id: ControlId, w: u32, h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_size(w, h);
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_visible(id: ControlId, visible: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_visible(visible != 0);
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_color(id: ControlId, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_color(color);
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_style(id: ControlId, key: u32, value: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().style.set(key, value);
        ctrl.base_mut().mark_dirty();
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_state(id: ControlId, value: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_state(value);
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_state(id: ControlId) -> u32 {
    let st = state();
    st.controls
        .iter()
        .find(|c| c.id() == id)
        .map_or(0, |c| c.state_val())
}

// ── Layout properties ────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_padding(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().padding = control::Padding {
            left,
            top,
            right,
            bottom,
        };
        ctrl.base_mut().mark_dirty();
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_margin(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().margin = control::Margin {
            left,
            top,
            right,
            bottom,
        };
        ctrl.base_mut().mark_dirty();
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_dock(id: ControlId, dock_style: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().dock = DockStyle::from_u32(dock_style);
        ctrl.base_mut().mark_dirty();
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_disabled(id: ControlId, disabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let b = ctrl.base_mut();
        let new_val = disabled != 0;
        if b.disabled != new_val {
            b.disabled = new_val;
            b.mark_dirty();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_auto_size(id: ControlId, enabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().auto_size = enabled != 0;
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_min_size(id: ControlId, min_w: u32, min_h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let b = ctrl.base_mut();
        b.min_w = min_w;
        b.min_h = min_h;
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_max_size(id: ControlId, max_w: u32, max_h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let b = ctrl.base_mut();
        b.max_w = max_w;
        b.max_h = max_h;
    }
}

// ── Text styling ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_font_size(id: ControlId, size: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_font_size(size as u16);
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_font_size(id: ControlId) -> u32 {
    let st = state();
    st.controls
        .iter()
        .find(|c| c.id() == id)
        .map_or(14, |c| c.get_font_size() as u32)
}

#[no_mangle]
pub extern "C" fn anyui_set_font(id: ControlId, font_id: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tb) = ctrl.text_base_mut() {
            tb.text_style.font_id = font_id as u16;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_text_color(id: ControlId, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tb) = ctrl.text_base_mut() {
            tb.text_style.text_color = color;
        }
    }
}

// ── Layout orientation ───────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_orientation(id: ControlId, orientation: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        match ctrl.kind() {
            ControlKind::StackPanel => {
                if let Some(sp) = control::cast_mut::<controls::stack_panel::StackPanel>(
                    ctrl,
                    ControlKind::StackPanel,
                ) {
                    sp.orientation = Orientation::from_u32(orientation);
                }
            }
            ControlKind::SplitView => {
                if let Some(sv) = control::cast_mut::<controls::split_view::SplitView>(
                    ctrl,
                    ControlKind::SplitView,
                ) {
                    let new_orient = Orientation::from_u32(orientation);
                    if sv.orientation != new_orient {
                        sv.orientation = new_orient;
                        sv.sync_divider();
                        sv.base.mark_dirty();
                    }
                }
            }
            _ => {}
        }
    }
}

// ── TableLayout properties ───────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_columns(id: ControlId, columns: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tl) =
            control::cast_mut::<controls::table_layout::TableLayout>(ctrl, ControlKind::TableLayout)
        {
            tl.columns = columns;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_row_height(id: ControlId, row_height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tl) =
            control::cast_mut::<controls::table_layout::TableLayout>(ctrl, ControlKind::TableLayout)
        {
            tl.row_height = row_height;
        }
    }
}

/// Set per-column pixel widths for a TableLayout.
///
/// `widths` is a pointer to an array of `len` u32 values. The first N-1
/// entries are used as-is; the last column receives the remaining available
/// width. Passing `len=0` resets to equal-distribution (default).
#[no_mangle]
pub extern "C" fn anyui_set_column_widths(id: ControlId, widths: *const u32, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tl) =
            control::cast_mut::<controls::table_layout::TableLayout>(ctrl, ControlKind::TableLayout)
        {
            if len == 0 || widths.is_null() {
                tl.col_widths.clear();
            } else {
                let slice = unsafe { core::slice::from_raw_parts(widths, len as usize) };
                tl.col_widths = slice.to_vec();
            }
        }
    }
}

// ── SplitView properties ─────────────────────────────────────────────

/// Helper to downcast a control to SplitView.
fn as_split_view(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::split_view::SplitView> {
    control::cast_mut(ctrl, ControlKind::SplitView)
}

#[no_mangle]
pub extern "C" fn anyui_set_split_ratio(id: ControlId, ratio: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(sv) = as_split_view(ctrl) {
            let r = ratio.min(100);
            if sv.split_ratio != r {
                sv.split_ratio = r;
                sv.sync_divider();
                sv.base.state = r;
                sv.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_min_split(id: ControlId, min_ratio: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(sv) = as_split_view(ctrl) {
            sv.min_ratio = min_ratio.min(100);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_max_split(id: ControlId, max_ratio: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(sv) = as_split_view(ctrl) {
            sv.max_ratio = max_ratio.min(100);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_splitview_set_resizable(id: ControlId, resizable: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(sv) = as_split_view(ctrl) {
            sv.resizable = resizable != 0;
        }
    }
}

// ── TextField properties ─────────────────────────────────────────────

/// Helper to downcast a control to TextField.
fn as_textfield(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::textfield::TextField> {
    control::cast_mut(ctrl, ControlKind::TextField)
}

fn as_textarea(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::textarea::TextArea> {
    control::cast_mut(ctrl, ControlKind::TextArea)
}

fn as_autocomplete_textfield(
    ctrl: &mut Box<dyn Control>,
) -> Option<&mut controls::autocomplete_textfield::AutoCompleteTextField> {
    control::cast_mut(ctrl, ControlKind::AutoCompleteTextField)
}

fn as_combobox(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::combobox::ComboBox> {
    control::cast_mut(ctrl, ControlKind::ComboBox)
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_prefix(id: ControlId, icon_code: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_val = if icon_code == 0 {
                None
            } else {
                Some(icon_code)
            };
            if tf.prefix_icon != new_val {
                tf.prefix_icon = new_val;
                tf.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_postfix(id: ControlId, icon_code: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_val = if icon_code == 0 {
                None
            } else {
                Some(icon_code)
            };
            if tf.postfix_icon != new_val {
                tf.postfix_icon = new_val;
                tf.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_password(id: ControlId, enabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_val = enabled != 0;
            if tf.password_mode != new_val {
                tf.password_mode = new_val;
                tf.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_placeholder(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_text = if !text.is_null() && len > 0 {
                unsafe { core::slice::from_raw_parts(text, len as usize) }
            } else {
                &[]
            };
            if tf.placeholder.as_slice() != new_text {
                tf.placeholder.clear();
                tf.placeholder.extend_from_slice(new_text);
                tf.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_select_all(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.select_all();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_read_only(id: ControlId, read_only: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_val = read_only != 0;
            if tf.read_only != new_val {
                tf.read_only = new_val;
                tf.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_cursor(id: ControlId, pos: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.set_cursor_pos(pos as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_get_cursor(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            return tf.cursor_pos() as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_selection(id: ControlId, start: u32, end: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.set_selection(start as usize, end as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_get_selection(
    id: ControlId,
    out_start: *mut u32,
    out_end: *mut u32,
) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let (start, end) = tf.selection();
            if !out_start.is_null() {
                unsafe { *out_start = start as u32 };
            }
            if !out_end.is_null() {
                unsafe { *out_end = end as u32 };
            }
            return 1;
        }
    }
    0
}

/// Set the maximum text length for a TextField (0 = unlimited).
#[no_mangle]
pub extern "C" fn anyui_textfield_set_max_length(id: ControlId, max_len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.max_length = max_len as usize;
        }
    }
}

/// Set the maximum text length for a TextArea (0 = unlimited).
#[no_mangle]
pub extern "C" fn anyui_textarea_set_max_length(id: ControlId, max_len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            ta.max_length = max_len as usize;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textarea_set_read_only(id: ControlId, read_only: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            let new_val = read_only != 0;
            if ta.read_only != new_val {
                ta.read_only = new_val;
                ta.text_base.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textarea_select_all(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            ta.select_all();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textarea_set_cursor(id: ControlId, pos: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            ta.set_cursor_pos(pos as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textarea_get_cursor(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            return ta.cursor_pos() as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_textarea_set_selection(id: ControlId, start: u32, end: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            ta.set_selection(start as usize, end as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textarea_get_selection(
    id: ControlId,
    out_start: *mut u32,
    out_end: *mut u32,
) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ta) = as_textarea(ctrl) {
            let (start, end) = ta.selection();
            if !out_start.is_null() {
                unsafe { *out_start = start as u32 };
            }
            if !out_end.is_null() {
                unsafe { *out_end = end as u32 };
            }
            return 1;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_autocomplete_set_suggestions(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(actf) = as_autocomplete_textfield(ctrl) {
            let new_text = if !text.is_null() && len > 0 {
                unsafe { core::slice::from_raw_parts(text, len as usize) }
            } else {
                &[]
            };
            actf.suggestions.clear();
            actf.suggestions.extend_from_slice(new_text);
            actf.text_base.base.mark_dirty();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_combobox_set_items(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            let data = if !text.is_null() && len > 0 {
                unsafe { core::slice::from_raw_parts(text, len as usize) }
            } else {
                &[]
            };
            cb.set_items(data);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_combobox_set_placeholder(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            let data = if !text.is_null() && len > 0 {
                unsafe { core::slice::from_raw_parts(text, len as usize) }
            } else {
                &[]
            };
            cb.set_placeholder(data);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_combobox_set_editable(id: ControlId, editable: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            cb.set_editable(editable != 0);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_combobox_get_editable(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            return cb.editable as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_combobox_set_selected_index(id: ControlId, index: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            if index == u32::MAX {
                cb.base_mut().state = u32::MAX;
                cb.set_text(&[]);
                cb.base_mut().mark_dirty();
            } else {
                cb.apply_selected_index(index as usize);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_combobox_get_selected_index(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cb) = as_combobox(ctrl) {
            return cb.base().state;
        }
    }
    u32::MAX
}

// ── Canvas operations ────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_canvas_set_pixel(id: ControlId, x: i32, y: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.set_pixel(x, y, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_clear(id: ControlId, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.clear(color);
            canvas.base.mark_dirty();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_fill_rect(
    id: ControlId,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.fill_rect(x, y, w, h, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_line(
    id: ControlId,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.draw_line(x0, y0, x1, y1, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_rect(
    id: ControlId,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
    thickness: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.draw_rect(x, y, w, h, color, thickness);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_circle(
    id: ControlId,
    cx: i32,
    cy: i32,
    radius: i32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.draw_circle(cx, cy, radius, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_fill_circle(
    id: ControlId,
    cx: i32,
    cy: i32,
    radius: i32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            canvas.fill_circle(cx, cy, radius, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_get_buffer(id: ControlId) -> *mut u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(canvas) =
            control::cast_mut::<controls::canvas::Canvas>(ctrl, ControlKind::Canvas)
        {
            return canvas.pixels.as_mut_ptr();
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn anyui_canvas_get_stride(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            return ctrl.base().w;
        }
    }
    0
}

/// Get the canvas height (in pixels).
#[no_mangle]
pub extern "C" fn anyui_canvas_get_height(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            return ctrl.base().h;
        }
    }
    0
}

// ── Canvas extensions (interactive, drawing primitives) ──────────────

fn as_canvas(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::canvas::Canvas> {
    control::cast_mut(ctrl, ControlKind::Canvas)
}

fn as_canvas_ref(ctrl: &Box<dyn Control>) -> Option<&controls::canvas::Canvas> {
    control::cast_ref(ctrl, ControlKind::Canvas)
}

/// Enable or disable interactive mode (mouse move fires EVENT_CHANGE).
#[no_mangle]
pub extern "C" fn anyui_canvas_set_interactive(id: ControlId, enabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.interactive = enabled != 0;
        }
    }
}

/// Get last mouse position and button state. Returns via out pointers.
#[no_mangle]
pub extern "C" fn anyui_canvas_get_mouse(
    id: ControlId,
    out_x: *mut i32,
    out_y: *mut i32,
    out_button: *mut u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas_ref(ctrl) {
            if !out_x.is_null() {
                unsafe {
                    *out_x = cv.last_mouse_x;
                }
            }
            if !out_y.is_null() {
                unsafe {
                    *out_y = cv.last_mouse_y;
                }
            }
            if !out_button.is_null() {
                unsafe {
                    *out_button = cv.mouse_button;
                }
            }
        }
    }
}

/// Get the most recent wheel-scroll delta on this canvas. Positive
/// values are wheel-up, negative wheel-down. Returns 0 when no scroll
/// event has been received yet, or the canvas id is invalid. Apps
/// typically read this from an EVENT_SCROLL handler registered via
/// `Control::on_scroll_raw` to react to scroll direction.
#[no_mangle]
pub extern "C" fn anyui_canvas_get_wheel(id: ControlId) -> i32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas_ref(ctrl) {
            return cv.last_wheel_dz;
        }
    }
    0
}

/// Draw a filled ellipse.
#[no_mangle]
pub extern "C" fn anyui_canvas_fill_ellipse(
    id: ControlId,
    cx: i32,
    cy: i32,
    rx: i32,
    ry: i32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.fill_ellipse(cx, cy, rx, ry, color);
        }
    }
}

/// Draw an ellipse outline.
#[no_mangle]
pub extern "C" fn anyui_canvas_draw_ellipse(
    id: ControlId,
    cx: i32,
    cy: i32,
    rx: i32,
    ry: i32,
    color: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.draw_ellipse(cx, cy, rx, ry, color);
        }
    }
}

/// Flood fill starting from (x, y) with the given color.
#[no_mangle]
pub extern "C" fn anyui_canvas_flood_fill(id: ControlId, x: i32, y: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.flood_fill(x, y, color);
        }
    }
}

/// Draw a thick line (filled circles at each Bresenham step).
#[no_mangle]
pub extern "C" fn anyui_canvas_draw_thick_line(
    id: ControlId,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
    thickness: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.draw_thick_line(x0, y0, x1, y1, color, thickness);
        }
    }
}

/// Read a single pixel value. Returns 0 if out of bounds.
#[no_mangle]
pub extern "C" fn anyui_canvas_get_pixel(id: ControlId, x: i32, y: i32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas_ref(ctrl) {
            return cv.get_pixel(x, y);
        }
    }
    0
}

/// Copy pixels from a source buffer into the canvas.
#[no_mangle]
pub extern "C" fn anyui_canvas_copy_from(id: ControlId, src: *const u32, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            if !src.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(src, len as usize) };
                cv.copy_pixels_from(slice);
            }
        }
    }
}

/// Copy canvas pixels into a destination buffer. Returns number of pixels copied.
#[no_mangle]
pub extern "C" fn anyui_canvas_copy_to(id: ControlId, dst: *mut u32, len: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas_ref(ctrl) {
            if !dst.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts_mut(dst, len as usize) };
                return cv.copy_pixels_to(slice) as u32;
            }
        }
    }
    0
}

/// Draw text into a Canvas pixel buffer using the system font engine (libfont).
///
/// - `font_id`: 0 = system, 1 = bold, 2 = thin, 3 = italic, 4 = mono (Andale Mono)
/// - `size`: font size in pixels
#[no_mangle]
pub extern "C" fn anyui_canvas_draw_text(
    id: ControlId,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    text_len: u32,
) {
    if text_ptr.is_null() || text_len == 0 {
        return;
    }
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            let w = cv.base.w;
            let h = cv.base.h;
            let ptr = cv.pixels.as_mut_ptr();
            draw::ensure_libfont();
            let text = unsafe { core::slice::from_raw_parts(text_ptr, text_len as usize) };
            draw::draw_text_to_buf(ptr, w, h, x, y, color, font_id, size, text);
            cv.base.mark_dirty();
        }
    }
}

// ── ImageView ────────────────────────────────────────────────────────

/// Set pixel data for an ImageView from a decoded ARGB buffer.
#[no_mangle]
pub extern "C" fn anyui_imageview_set_pixels(id: ControlId, data: *const u32, w: u32, h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(iv) =
            control::cast_mut::<controls::image_view::ImageView>(ctrl, ControlKind::ImageView)
        {
            let count = (w as usize) * (h as usize);
            if !data.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, count) };
                iv.set_pixels(slice, w, h);
            }
        }
    }
}

/// Set the scale mode for an ImageView: 0=None, 1=Fit, 2=Fill, 3=Stretch.
#[no_mangle]
pub extern "C" fn anyui_imageview_set_scale_mode(id: ControlId, mode: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(iv) =
            control::cast_mut::<controls::image_view::ImageView>(ctrl, ControlKind::ImageView)
        {
            if iv.scale_mode != mode {
                iv.scale_mode = mode;
                iv.base.mark_dirty();
            }
        }
    }
}

/// Get the original image dimensions. Returns via out pointers. Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_imageview_get_image_size(
    id: ControlId,
    out_w: *mut u32,
    out_h: *mut u32,
) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(iv) =
            control::cast_ref::<controls::image_view::ImageView>(ctrl, ControlKind::ImageView)
        {
            if !out_w.is_null() {
                unsafe {
                    *out_w = iv.img_w;
                }
            }
            if !out_h.is_null() {
                unsafe {
                    *out_h = iv.img_h;
                }
            }
            return 1;
        }
    }
    0
}

/// Clear pixel data from an ImageView.
#[no_mangle]
pub extern "C" fn anyui_imageview_clear(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(iv) =
            control::cast_mut::<controls::image_view::ImageView>(ctrl, ControlKind::ImageView)
        {
            iv.clear();
        }
    }
}

// ── IconButton ───────────────────────────────────────────────────────

/// Set pre-rendered icon pixel data for an IconButton.
#[no_mangle]
pub extern "C" fn anyui_iconbutton_set_pixels(id: ControlId, data: *const u32, w: u32, h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let count = (w as usize) * (h as usize);
        if data.is_null() || count == 0 {
            return;
        }
        let slice = unsafe { core::slice::from_raw_parts(data, count) };
        match ctrl.kind() {
            ControlKind::IconButton => {
                if let Some(ib) = control::cast_mut::<controls::icon_button::IconButton>(
                    ctrl,
                    ControlKind::IconButton,
                ) {
                    ib.set_icon_pixels(slice, w, h);
                }
            }
            ControlKind::PlainButton => {
                if let Some(pb) = control::cast_mut::<controls::plain_button::PlainButton>(
                    ctrl,
                    ControlKind::PlainButton,
                ) {
                    pb.set_icon_pixels(slice, w, h);
                }
            }
            _ => {}
        }
    }
}

/// Enable or disable flat toolbar-style rendering for an IconButton.
#[no_mangle]
pub extern "C" fn anyui_iconbutton_set_flat(id: ControlId, flat: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(ib) =
            control::cast_mut::<controls::icon_button::IconButton>(ctrl, ControlKind::IconButton)
        {
            ib.flat = flat != 0;
            ib.base_mut().mark_dirty();
        }
    }
}

// ── DataGrid ─────────────────────────────────────────────────────────

fn as_data_grid(
    ctrl: &mut alloc::boxed::Box<dyn Control>,
) -> Option<&mut controls::data_grid::DataGrid> {
    control::cast_mut(ctrl, ControlKind::DataGrid)
}

fn as_data_grid_ref(
    ctrl: &alloc::boxed::Box<dyn Control>,
) -> Option<&controls::data_grid::DataGrid> {
    control::cast_ref(ctrl, ControlKind::DataGrid)
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_columns(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !data.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
                dg.set_columns_from_data(slice);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_get_column_count(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            return dg.column_count() as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_column_width(id: ControlId, col_index: u32, width: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_column_width(col_index as usize, width);
        }
    }
}

/// Set the sort comparison type for a column (0 = string, 1 = numeric).
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_column_sort_type(
    id: ControlId,
    col_index: u32,
    sort_type: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_column_sort_type(
                col_index as usize,
                controls::data_grid::SortType::from_u8(sort_type as u8),
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_data(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !data.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
                dg.set_data_from_encoded(slice);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_cell(
    id: ControlId,
    row: u32,
    col: u32,
    text: *const u8,
    text_len: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !text.is_null() && text_len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
                dg.set_cell(row as usize, col as usize, slice);
            } else {
                dg.set_cell(row as usize, col as usize, &[]);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_get_cell(
    id: ControlId,
    row: u32,
    col: u32,
    buf: *mut u8,
    max_len: u32,
) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            let text = dg.get_cell(row as usize, col as usize);
            let copy_len = text.len().min(max_len as usize);
            if !buf.is_null() && copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(text.as_ptr(), buf, copy_len);
                }
            }
            return copy_len as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_cell_colors(id: ControlId, colors: *const u32, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !colors.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(colors, count as usize) };
                dg.set_cell_colors(slice);
            } else {
                dg.set_cell_colors(&[]);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_cell_bg_colors(id: ControlId, colors: *const u32, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !colors.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(colors, count as usize) };
                dg.set_cell_bg_colors(slice);
            } else {
                dg.set_cell_bg_colors(&[]);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_row_count(id: ControlId, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_row_count(count as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_get_row_count(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            return dg.row_count as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_selection_mode(id: ControlId, mode: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_selection_mode(if mode == 1 {
                controls::data_grid::SelectionMode::Multi
            } else {
                controls::data_grid::SelectionMode::Single
            });
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_editable_columns(id: ControlId, mask: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_editable_columns(mask);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_row_editor_kinds(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let slice = if len == 0 || data.is_null() {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(data, len as usize) }
            };
            dg.set_row_editor_kinds_from_encoded(slice);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_row_editor_options(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let slice = if len == 0 || data.is_null() {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(data, len as usize) }
            };
            dg.set_row_editor_options_from_encoded(slice);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_get_selected_row(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::DataGrid {
            return ctrl.base().state;
        }
    }
    u32::MAX
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_selected_row(id: ControlId, row: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.clear_selection();
            dg.set_row_selected(row as usize, true);
            dg.base.state = row;
            dg.scroll_to_row(row as usize);
            dg.base.mark_dirty();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_is_row_selected(id: ControlId, row: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            return dg.is_row_selected(row as usize) as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_sort(id: ControlId, column: u32, direction: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let dir = match direction {
                1 => controls::data_grid::SortDirection::Ascending,
                2 => controls::data_grid::SortDirection::Descending,
                _ => controls::data_grid::SortDirection::None,
            };
            dg.sort_by(column as usize, dir);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_row_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let h = height.max(16);
            if dg.row_height != h {
                dg.row_height = h;
                dg.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_header_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let h = height.max(16);
            if dg.header_height != h {
                dg.header_height = h;
                dg.base.mark_dirty();
            }
        }
    }
}

/// Set per-character text colors for grid cells.
/// `char_colors`/`char_colors_len`: flat array of ARGB colors, one per character.
/// `offsets`/`offsets_len`: one u32 per cell — index into char_colors (u32::MAX = none).
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_char_colors(
    id: ControlId,
    char_colors: *const u32,
    char_colors_len: u32,
    offsets: *const u32,
    offsets_len: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let cc = if !char_colors.is_null() && char_colors_len > 0 {
                unsafe { core::slice::from_raw_parts(char_colors, char_colors_len as usize) }
            } else {
                &[]
            };
            let off = if !offsets.is_null() && offsets_len > 0 {
                unsafe { core::slice::from_raw_parts(offsets, offsets_len as usize) }
            } else {
                &[]
            };
            dg.set_char_colors(cc, off);
        }
    }
}

/// Set an icon (ARGB pixels) for a specific cell. The icon is drawn before the text.
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_cell_icon(
    id: ControlId,
    row: u32,
    col: u32,
    pixels: *const u32,
    w: u32,
    h: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !pixels.is_null() && w > 0 && h > 0 {
                let count = (w * h) as usize;
                let slice = unsafe { core::slice::from_raw_parts(pixels, count) };
                dg.set_cell_icon(row as usize, col as usize, slice, w as u16, h as u16);
            }
        }
    }
}

/// Set per-row minimap colors (drawn in scrollbar track).
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_minimap(id: ControlId, colors: *const u32, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !colors.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(colors, count as usize) };
                dg.set_minimap_colors(slice);
            } else {
                dg.set_minimap_colors(&[]);
            }
        }
    }
}

/// Set per-row left indent in pixels (first column only).
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_row_indents(id: ControlId, indents: *const u16, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            if !indents.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(indents, count as usize) };
                dg.set_row_indents(slice);
            } else {
                dg.set_row_indents(&[]);
            }
        }
    }
}

/// Set which logical column receives per-row indentation.
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_indent_column(id: ControlId, col: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_indent_column(col as usize);
        }
    }
}

/// Get the display column index of the last click (-1 if none).
#[no_mangle]
pub extern "C" fn anyui_datagrid_get_click_col(id: ControlId) -> i32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            return dg.last_click_col();
        }
    }
    -1
}

/// Set connector lines for the DataGrid (drawn over a column).
/// Data format per entry: start_row:u32, end_row:u32, color:u32, filled:u8 (+ 3 pad bytes) = 16 bytes each.
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_connectors(id: ControlId, data: *const u8, count: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            let mut lines = alloc::vec::Vec::new();
            if !data.is_null() && count > 0 {
                let entry_size = 16usize;
                let bytes =
                    unsafe { core::slice::from_raw_parts(data, count as usize * entry_size) };
                for i in 0..count as usize {
                    let off = i * entry_size;
                    if off + entry_size > bytes.len() {
                        break;
                    }
                    let start = u32::from_le_bytes([
                        bytes[off],
                        bytes[off + 1],
                        bytes[off + 2],
                        bytes[off + 3],
                    ]) as usize;
                    let end = u32::from_le_bytes([
                        bytes[off + 4],
                        bytes[off + 5],
                        bytes[off + 6],
                        bytes[off + 7],
                    ]) as usize;
                    let color = u32::from_le_bytes([
                        bytes[off + 8],
                        bytes[off + 9],
                        bytes[off + 10],
                        bytes[off + 11],
                    ]);
                    let filled = bytes[off + 12] != 0;
                    lines.push(controls::data_grid::ConnectorLine {
                        start_row: start,
                        end_row: end,
                        color,
                        filled,
                    });
                }
            }
            dg.set_connector_lines(lines);
        }
    }
}

/// Set which display column connector lines are drawn in.
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_connector_column(id: ControlId, col: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.set_connector_column(col as usize);
        }
    }
}

// ── TextEditor ────────────────────────────────────────────────────────

fn as_text_editor(
    ctrl: &mut alloc::boxed::Box<dyn Control>,
) -> Option<&mut controls::text_editor::TextEditor> {
    control::cast_mut(ctrl, ControlKind::TextEditor)
}

fn as_text_editor_ref(
    ctrl: &alloc::boxed::Box<dyn Control>,
) -> Option<&controls::text_editor::TextEditor> {
    control::cast_ref(ctrl, ControlKind::TextEditor)
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_text(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            if !data.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
                te.set_text(slice);
            } else {
                te.set_text(&[]);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_get_text(id: ControlId, buf: *mut u8, max_len: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor_ref(ctrl) {
            let text = te.get_text();
            let copy_len = text.len().min(max_len as usize);
            if !buf.is_null() && copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(text.as_ptr(), buf, copy_len);
                }
            }
            return copy_len as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_syntax(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            if !data.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
                te.set_syntax(slice);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_cursor(id: ControlId, row: u32, col: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.set_cursor(row as usize, col as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_get_cursor(id: ControlId, out_row: *mut u32, out_col: *mut u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor_ref(ctrl) {
            let (r, c) = te.cursor();
            if !out_row.is_null() {
                unsafe {
                    *out_row = r as u32;
                }
            }
            if !out_col.is_null() {
                unsafe {
                    *out_col = c as u32;
                }
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_line_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            let h = height.max(12);
            if te.line_height != h {
                te.line_height = h;
                te.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_tab_width(id: ControlId, width: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.tab_width = width.max(1);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_show_line_numbers(id: ControlId, show: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            let new_val = show != 0;
            if te.show_line_numbers != new_val {
                te.show_line_numbers = new_val;
                te.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_font(id: ControlId, font_id: u32, font_size: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            let fid = font_id as u16;
            let fsz = font_size as u16;
            if te.font_id != fid || te.font_size != fsz {
                te.font_id = fid;
                te.font_size = fsz;
                let (cw, _) = crate::draw::measure_text_ex(b"M", te.font_id, te.font_size);
                te.char_width = if cw > 0 { cw } else { 8 };
                te.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_insert_text(id: ControlId, data: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            if !data.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
                te.push_undo();
                te.insert_text_at_cursor(slice);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_get_line_count(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor_ref(ctrl) {
            return te.line_count() as u32;
        }
    }
    0
}

/// Copy selected text to system clipboard. Returns 1 if text was copied, 0 if no selection.
#[no_mangle]
pub extern "C" fn anyui_texteditor_copy(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor_ref(ctrl) {
            if let Some(text) = te.extract_selected_text() {
                compositor::clipboard_set(&text);
                return 1;
            }
        }
    }
    0
}

/// Cut selected text to system clipboard. Returns 1 if text was cut, 0 if no selection.
#[no_mangle]
pub extern "C" fn anyui_texteditor_cut(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            if let Some(text) = te.extract_selected_text() {
                compositor::clipboard_set(&text);
                te.delete_selection();
                te.base_mut().mark_dirty();
                return 1;
            }
        }
    }
    0
}

/// Paste from system clipboard at cursor position.
#[no_mangle]
pub extern "C" fn anyui_texteditor_paste(id: ControlId) -> u32 {
    if let Some(data) = compositor::clipboard_get() {
        let st = state();
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
            if let Some(te) = as_text_editor(ctrl) {
                te.delete_selection();
                te.clamp_cursor();
                te.insert_text_at_cursor(&data);
                te.base_mut().mark_dirty();
                return data.len() as u32;
            }
        }
    }
    0
}

/// Select all text in the editor.
#[no_mangle]
pub extern "C" fn anyui_texteditor_select_all(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.select_all();
            te.base_mut().mark_dirty();
        }
    }
}

/// Highlight a line with a background color. Multiple calls add more highlights.
#[no_mangle]
pub extern "C" fn anyui_texteditor_highlight_line(id: ControlId, line: u32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.highlight_line(line, color);
        }
    }
}

/// Remove all line highlights.
#[no_mangle]
pub extern "C" fn anyui_texteditor_clear_highlights(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.clear_highlights();
        }
    }
}

/// Add a diagnostic underline/gutter marker. Coordinates are zero-based.
#[no_mangle]
pub extern "C" fn anyui_texteditor_add_diagnostic(
    id: ControlId,
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
    severity: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.add_diagnostic(line, column, end_line, end_column, severity);
        }
    }
}

/// Remove all diagnostic underlines/gutter markers.
#[no_mangle]
pub extern "C" fn anyui_texteditor_clear_diagnostics(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.clear_diagnostics();
        }
    }
}

/// Set read-only mode (1 = read-only, 0 = editable).
#[no_mangle]
pub extern "C" fn anyui_texteditor_set_read_only(id: ControlId, read_only: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.read_only = read_only != 0;
        }
    }
}

/// Scroll to make a specific line visible (centered).
#[no_mangle]
pub extern "C" fn anyui_texteditor_ensure_line_visible(id: ControlId, line: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.ensure_line_visible(line);
        }
    }
}

/// Toggle folding for the block around the current cursor line.
#[no_mangle]
pub extern "C" fn anyui_texteditor_toggle_fold_at_cursor(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.toggle_fold_at_cursor();
        }
    }
}

// ── TreeView ──────────────────────────────────────────────────────────

fn as_tree_view(
    ctrl: &mut alloc::boxed::Box<dyn Control>,
) -> Option<&mut controls::tree_view::TreeView> {
    control::cast_mut(ctrl, ControlKind::TreeView)
}

fn as_tree_view_ref(
    ctrl: &alloc::boxed::Box<dyn Control>,
) -> Option<&controls::tree_view::TreeView> {
    control::cast_ref(ctrl, ControlKind::TreeView)
}

#[no_mangle]
pub extern "C" fn anyui_treeview_add_node(
    id: ControlId,
    parent_index: u32,
    text: *const u8,
    text_len: u32,
) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            let parent = if parent_index == u32::MAX {
                None
            } else {
                Some(parent_index as usize)
            };
            let slice = if !text.is_null() && text_len > 0 {
                unsafe { core::slice::from_raw_parts(text, text_len as usize) }
            } else {
                &[]
            };
            return tv.add_node(parent, slice) as u32;
        }
    }
    u32::MAX
}

#[no_mangle]
pub extern "C" fn anyui_treeview_remove_node(id: ControlId, index: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.remove_node(index as usize);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_node_text(
    id: ControlId,
    index: u32,
    text: *const u8,
    text_len: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            let slice = if !text.is_null() && text_len > 0 {
                unsafe { core::slice::from_raw_parts(text, text_len as usize) }
            } else {
                &[]
            };
            tv.set_node_text(index as usize, slice);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_node_icon(
    id: ControlId,
    index: u32,
    pixels: *const u32,
    w: u32,
    h: u32,
) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            if !pixels.is_null() && w > 0 && h > 0 {
                let count = (w * h) as usize;
                let slice = unsafe { core::slice::from_raw_parts(pixels, count) };
                tv.set_node_icon(index as usize, slice, w as u16, h as u16);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_node_style(id: ControlId, index: u32, style: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.set_node_style(index as usize, style);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_node_text_color(id: ControlId, index: u32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.set_node_text_color(index as usize, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_expanded(id: ControlId, index: u32, expanded: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.set_expanded(index as usize, expanded != 0);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_get_expanded(id: ControlId, index: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            return tv.is_expanded(index as usize) as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_treeview_get_selected(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            return tv.selected().map_or(u32::MAX, |s| s as u32);
        }
    }
    u32::MAX
}

#[no_mangle]
pub extern "C" fn anyui_treeview_get_hovered(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            return tv.hovered().map_or(u32::MAX, |s| s as u32);
        }
    }
    u32::MAX
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_selected(id: ControlId, index: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            if index == u32::MAX {
                tv.set_selected(None);
            } else {
                tv.set_selected(Some(index as usize));
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_clear(id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.clear();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_get_node_count(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            return tv.node_count() as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_indent_width(id: ControlId, width: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            let w = width.max(8);
            if tv.indent_width != w {
                tv.indent_width = w;
                tv.base.mark_dirty();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_row_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            let h = height.max(16);
            if tv.row_height != h {
                tv.row_height = h;
                tv.base.mark_dirty();
            }
        }
    }
}

// ── Callbacks ────────────────────────────────────────────────────────

/// Register a callback for a specific event type on a control.
///
/// Event types: EVENT_CLICK=1, EVENT_CHANGE=2, EVENT_KEY=3, EVENT_FOCUS=4,
/// EVENT_BLUR=5, EVENT_CLOSE=6, EVENT_RESIZE=7, EVENT_SCROLL=8,
/// EVENT_DRAG=9, EVENT_CONTEXT_MENU=10, EVENT_DOUBLE_CLICK=11,
/// EVENT_MOUSE_ENTER=12, EVENT_MOUSE_LEAVE=13, EVENT_MOUSE_DOWN=14,
/// EVENT_MOUSE_UP=15, EVENT_MOUSE_MOVE=16, EVENT_DRAG_START=21,
/// EVENT_DRAG_ENTER=22, EVENT_DRAG_LEAVE=23, EVENT_DROP=24,
/// EVENT_DRAG_END=25
#[no_mangle]
pub extern "C" fn anyui_on_event(id: ControlId, event_type: u32, cb: Callback, userdata: u64) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_event_callback(event_type, cb, userdata);
    }
}

// Convenience wrappers (backward compat)
#[no_mangle]
pub extern "C" fn anyui_on_click(id: ControlId, cb: Callback, userdata: u64) {
    anyui_on_event(id, control::EVENT_CLICK, cb, userdata);
}

#[no_mangle]
pub extern "C" fn anyui_on_change(id: ControlId, cb: Callback, userdata: u64) {
    anyui_on_event(id, control::EVENT_CHANGE, cb, userdata);
}

#[no_mangle]
pub extern "C" fn anyui_on_submit(id: ControlId, cb: Callback, userdata: u64) {
    anyui_on_event(id, control::EVENT_SUBMIT, cb, userdata);
}

#[no_mangle]
pub extern "C" fn anyui_set_context_menu(id: ControlId, menu_id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().context_menu = Some(menu_id);
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_draggable(id: ControlId, draggable: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().draggable = draggable != 0;
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_drop_target(id: ControlId, drop_target: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let base = ctrl.base_mut();
        base.drop_target = drop_target != 0;
        if base.drop_target && base.drop_formats == 0 {
            // Default policy: newly marked drop targets accept any format.
            // Callers can narrow this via anyui_set_drop_formats.
            base.drop_formats = dnd::DND_FORMAT_ACCEPT_ANY;
        }
    }
}

/// Restrict which payload formats this drop target accepts. The `mask`
/// is built client-side via `dnd::format_mask()` (or `DND_FORMAT_ACCEPT_ANY`
/// for "accept anything"). Only targets whose mask contains the current
/// drag payload's format receive DRAG_ENTER/DROP events.
#[no_mangle]
pub extern "C" fn anyui_set_drop_formats(id: ControlId, mask: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().drop_formats = mask;
    }
}

/// Install a binary payload for the active drag. Safe to call only from a
/// DRAG_START callback on the source control. `allowed_effects` is a bitmask
/// of `dnd::DND_EFFECT_*` describing which semantics the source permits.
///
/// On the first call (per drag), this also allocates a small SHM region,
/// writes the payload into it, and announces the drag to the compositor
/// (CMD_DRAG_BEGIN) so the cursor can cross window boundaries. Subsequent
/// calls update the SHM in place but do not re-announce.
#[no_mangle]
pub extern "C" fn anyui_drag_set_payload(
    format: u32,
    data: *const u8,
    len: u32,
    allowed_effects: u32,
) {
    let st = state();
    let source_id = match st.drag.as_ref() {
        Some(d) => d.source_id,
        None => return,
    };
    if let Some(drag) = st.drag.as_mut() {
        drag.format = format;
        drag.allowed_effects = allowed_effects;
        drag.data.clear();
        if !data.is_null() && len > 0 {
            let src = unsafe { core::slice::from_raw_parts(data, len as usize) };
            drag.data.extend_from_slice(src);
        }
    }
    // Ensure the cross-window bridge exists, then push the payload to the
    // compositor.
    let comp_win = dnd_ipc::comp_window_for_control(st, source_id);
    if comp_win != 0 {
        if dnd_ipc::ensure_bridge_shm(st, comp_win) {
            dnd_ipc::announce_drag(st);
        }
    }
}

/// Read the current drag payload's raw bytes into `buf`. Returns the total
/// length (not the number copied — use `min(return, cap)` to know the
/// copied slice). Writes the payload's format to `*format_out` when
/// `format_out` is non-null.
///
/// Reads from the incoming-drag SHM mapping when called from a
/// DRAG_ENTER/DRAG/DROP callback (target side); falls back to the local
/// source payload when called from a DRAG_START callback (source side).
#[no_mangle]
pub extern "C" fn anyui_drag_get_payload(buf: *mut u8, cap: u32, format_out: *mut u32) -> u32 {
    let st = state();
    // Target-side read (cross-process payload SHM).
    if let Some(inc) = st.incoming_drag.as_ref() {
        if !format_out.is_null() {
            unsafe {
                *format_out = inc.format;
            }
        }
        let total = inc.payload_len as usize;
        let n = core::cmp::min(total, cap as usize);
        if !buf.is_null() && n > 0 && inc.payload_addr != 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(inc.payload_addr as *const u8, buf, n);
            }
        }
        return total as u32;
    }
    // Source-side read (local payload Vec, before any cross-process target
    // has mapped the SHM yet).
    let drag = match st.drag.as_ref() {
        Some(d) => d,
        None => return 0,
    };
    if !format_out.is_null() {
        unsafe {
            *format_out = drag.format;
        }
    }
    let n = core::cmp::min(drag.data.len(), cap as usize);
    if !buf.is_null() && n > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(drag.data.as_ptr(), buf, n);
        }
    }
    drag.data.len() as u32
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_format() -> u32 {
    state()
        .drag
        .as_ref()
        .map(|d| d.format)
        .unwrap_or(dnd::DND_FORMAT_NONE)
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_allowed_effects() -> u32 {
    state()
        .drag
        .as_ref()
        .map(|d| d.allowed_effects)
        .unwrap_or(dnd::DND_EFFECT_NONE)
}

/// Drop target opt-in. Call from a DRAG_ENTER or DRAG (over) callback.
/// `requested_effects` is a bitmask of effects this target is willing to
/// perform; the framework negotiates against the source's allowed effects
/// and current modifier keys (Ctrl = Copy, Shift = Move, Ctrl+Shift = Link).
/// Returns the negotiated effect (also stored for `anyui_drag_get_effect`),
/// or `DND_EFFECT_NONE` when no overlap exists (effectively a rejection).
#[no_mangle]
pub extern "C" fn anyui_drag_accept(requested_effects: u32) -> u32 {
    let st = state();
    // Target side: incoming cross-process drag.
    if let Some(inc) = st.incoming_drag.as_ref() {
        let allowed = inc.allowed_effects;
        let mods = inc.modifiers;
        let effect = dnd::negotiate_effect(allowed, requested_effects, mods);
        if let Some(inc) = st.incoming_drag.as_mut() {
            inc.negotiated_effect = effect;
            inc.target_accepted = effect != dnd::DND_EFFECT_NONE;
            inc.accept_modifiers = mods;
        }
        // Mirror to the compositor so the source learns about the
        // negotiated effect via EVT_DRAG_FEEDBACK.
        if effect != dnd::DND_EFFECT_NONE {
            dnd_ipc::send_accept(st, effect);
        } else {
            dnd_ipc::send_reject(st);
        }
        return effect;
    }
    // Source-only context (DRAG_START before payload has been announced):
    // no target to negotiate with yet.
    dnd::DND_EFFECT_NONE
}

/// Explicitly reject the current drag from a drop-target callback. Not
/// required (the default is rejected) but useful for clarity when a target
/// opts out after a conditional check.
#[no_mangle]
pub extern "C" fn anyui_drag_reject() {
    let st = state();
    if let Some(inc) = st.incoming_drag.as_mut() {
        inc.target_accepted = false;
        inc.negotiated_effect = dnd::DND_EFFECT_NONE;
    }
    dnd_ipc::send_reject(st);
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_effect() -> u32 {
    state()
        .drag
        .as_ref()
        .map(|d| d.negotiated_effect)
        .unwrap_or(dnd::DND_EFFECT_NONE)
}

/// Report the current pointer position (logical, window-relative) via out
/// parameters. Returns 1 when a drag is active and coords were written,
/// 0 otherwise.
#[no_mangle]
pub extern "C" fn anyui_drag_get_pos(x_out: *mut i32, y_out: *mut i32) -> u32 {
    let st = state();
    match st.drag.as_ref() {
        Some(d) => {
            if !x_out.is_null() {
                unsafe {
                    *x_out = d.pointer_x;
                }
            }
            if !y_out.is_null() {
                unsafe {
                    *y_out = d.pointer_y;
                }
            }
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn anyui_drag_is_active() -> u32 {
    if state().drag.is_some() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_source() -> u32 {
    state().drag.as_ref().map(|d| d.source_id).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_target() -> u32 {
    state().drag.as_ref().and_then(|d| d.target_id).unwrap_or(0)
}

/// Attach a drag-image (ghost) to the active drag. Pixels are ARGB8888,
/// `w` × `h`, top-left origin. The hot-spot is the offset within the image
/// that should track the cursor (e.g. (0,0) for "image follows cursor's
/// top-left", or `(w/2, h/2)` for "image centred on cursor").
///
/// Must be called after `anyui_drag_set_payload` (which announces the drag
/// to the compositor). Replaces any previously-set image.
#[no_mangle]
pub extern "C" fn anyui_drag_set_image(
    pixels: *const u32,
    w: u32,
    h: u32,
    hot_x: i32,
    hot_y: i32,
) -> u32 {
    if pixels.is_null() || w == 0 || h == 0 {
        return 0;
    }
    let total = (w as usize) * (h as usize);
    let st = state();
    let slice = unsafe { core::slice::from_raw_parts(pixels, total) };
    if dnd_ipc::install_drag_image(st, slice, w, h, hot_x, hot_y) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn anyui_drag_set_text(text: *const u8, len: u32) {
    let st = state();
    let source_id = match st.drag.as_ref() {
        Some(d) => d.source_id,
        None => return,
    };
    if let Some(drag) = st.drag.as_mut() {
        drag.format = dnd::DND_FORMAT_TEXT;
        drag.data.clear();
        if !text.is_null() && len > 0 {
            let src = unsafe { core::slice::from_raw_parts(text, len as usize) };
            drag.data.extend_from_slice(src);
        }
    }
    let comp_win = dnd_ipc::comp_window_for_control(st, source_id);
    if comp_win != 0 && dnd_ipc::ensure_bridge_shm(st, comp_win) {
        dnd_ipc::announce_drag(st);
    }
}

#[no_mangle]
pub extern "C" fn anyui_drag_get_text(buf: *mut u8, cap: u32) -> u32 {
    let st = state();
    let data = match st.drag.as_ref() {
        Some(d) => &d.data,
        None => return 0,
    };
    let n = core::cmp::min(data.len(), cap as usize);
    if !buf.is_null() && n > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), buf, n);
        }
    }
    data.len() as u32
}

/// Programmatically open a control's context menu below the control.
/// Used for hamburger menus that open on left-click instead of right-click.
#[no_mangle]
pub extern "C" fn anyui_open_popup(id: ControlId) {
    let st = state();

    // Find the control and its context menu.
    let ctrl_idx = match st.controls.iter().position(|c| c.id() == id) {
        Some(i) => i,
        None => return,
    };
    let menu_id = match st.controls[ctrl_idx].base().context_menu {
        Some(m) => m,
        None => return,
    };
    let mi = match st.controls.iter().position(|c| c.id() == menu_id) {
        Some(i) => i,
        None => return,
    };

    // Dismiss any existing popup first.
    event_loop::dismiss_popup_pub(st);

    // Compute absolute position by walking up the parent chain.
    let mut abs_x: i32 = 0;
    let mut abs_y: i32 = 0;
    let mut cur = ctrl_idx;
    let ctrl_h = st.controls[ctrl_idx].base().h;
    loop {
        let b = st.controls[cur].base();
        abs_x += b.x;
        abs_y += b.y;
        let pid = b.parent;
        if pid == 0 {
            break;
        }
        match st.controls.iter().position(|c| c.id() == pid) {
            Some(pi) => cur = pi,
            None => break,
        }
    }
    // Position the popup below the control.
    let popup_anchor_x = abs_x;
    let popup_anchor_y = abs_y + ctrl_h as i32;

    // Get menu dimensions.
    let menu_w = st.controls[mi].base().w;
    let menu_h = st.controls[mi].base().h;
    let margin: i32 = 16;
    let popup_w = menu_w + (margin as u32) * 2;
    let popup_h = menu_h + (margin as u32) * 2;
    let phys_popup_w = crate::theme::scale(popup_w);
    let phys_popup_h = crate::theme::scale(popup_h);

    // Find the owning window for this control.
    let win_ctrl_id: ControlId;
    let mut walk = ctrl_idx;
    loop {
        let b = st.controls[walk].base();
        if b.parent == 0 || st.controls[walk].kind() == control::ControlKind::Window {
            win_ctrl_id = st.controls[walk].id();
            break;
        }
        match st.controls.iter().position(|c| c.id() == b.parent) {
            Some(pi) => walk = pi,
            None => {
                win_ctrl_id = st.controls[walk].id();
                break;
            }
        }
    }
    let wi = match st.windows.iter().position(|&w| w == win_ctrl_id) {
        Some(i) => i,
        None => return,
    };
    let comp_window_id = if wi < st.comp_windows.len() {
        st.comp_windows[wi].window_id
    } else {
        return;
    };

    // Get parent window's screen position (physical).
    let (content_x, content_y) =
        compositor::get_window_position(st.channel_id, st.sub_id, comp_window_id);

    // Calculate popup screen position.
    let phys_margin = crate::theme::scale_i32(margin);
    let mut popup_x = content_x + crate::theme::scale_i32(popup_anchor_x) - phys_margin;
    let mut popup_y = content_y + crate::theme::scale_i32(popup_anchor_y) - phys_margin;

    // Clamp to screen bounds.
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

    // Create popup compositor window.
    let popup_flags: u32 = 0x01 | 0x02 | 0x04 | 0x100;
    if let Some((popup_win_id, shm_id, surface)) = compositor::create_window(
        st.channel_id,
        st.sub_id,
        popup_x,
        popup_y,
        phys_popup_w,
        phys_popup_h,
        popup_flags,
    ) {
        st.controls[mi].set_position(0, 0);
        st.controls[mi].base_mut().visible = false;

        let back_buffer = alloc::vec![0u32; (phys_popup_w * phys_popup_h) as usize];
        st.popup = Some(PopupInfo {
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
            owner_autocomplete: None,
            owner_text_edit: None,
        });
        let tid = libsyscall::get_tid();
        compositor::focus_by_tid(st.channel_id, tid);
    }
}

/// Set tooltip text for a control. Pass empty text (len=0) to remove.
#[no_mangle]
pub extern "C" fn anyui_set_tooltip(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    let bytes = if len > 0 && !text.is_null() {
        unsafe { core::slice::from_raw_parts(text, len as usize) }.to_vec()
    } else {
        Vec::new()
    };
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().tooltip_text = bytes.clone();
    }
    // If the control is currently hovered, update the tooltip display immediately
    // instead of waiting for the next MouseEnter (which will never fire while
    // the cursor stays on the same control).
    if st.hovered == Some(id) {
        if bytes.is_empty() {
            // Remove pending schedule and hide any visible tooltip.
            st.tooltip_pending_id = None;
            if let Some(tip_id) = st.active_tooltip {
                if let Some(ti) = crate::control::find_idx(&st.controls, tip_id) {
                    if st.controls[ti].base().visible {
                        st.controls[ti].base_mut().visible = false;
                        st.controls[ti].base_mut().mark_dirty();
                    }
                }
            }
        } else {
            // Schedule a quick re-show (50 ms) so the tooltip text updates
            // as the cursor moves over different segments / items.
            st.tooltip_pending_id = Some(id);
            // Pretend hover started 450 ms ago so the 500 ms delay fires in ~50 ms.
            st.tooltip_hover_start = crate::syscall::uptime_ms().wrapping_sub(450);
        }
    }
}

// ── MessageBox ───────────────────────────────────────────────────────

static mut MSGBOX_DISMISSED: bool = false;

extern "C" fn msgbox_ok_clicked(_id: u32, _event_type: u32, _userdata: u64) {
    unsafe {
        MSGBOX_DISMISSED = true;
    }
}

/// Compute logical (x, y) that centers a dialog of size (dlg_w, dlg_h)
/// on the screen position of `owner_win_id`.
pub(crate) fn center_on_owner(owner_win_id: ControlId, dlg_w: u32, dlg_h: u32) -> (i32, i32) {
    let st = state();
    if let Some(idx) = st.windows.iter().position(|&w| w == owner_win_id) {
        let cw = &st.comp_windows[idx];
        let (owner_px, owner_py) =
            compositor::get_window_position(st.channel_id, st.sub_id, cw.window_id);
        let owner_pw = cw.width as i32;
        let owner_ph = cw.height as i32;
        let phys_dlg_w = theme::scale(dlg_w) as i32;
        let phys_dlg_h = theme::scale(dlg_h) as i32;
        let cx = owner_px + (owner_pw - phys_dlg_w) / 2;
        let cy = owner_py + (owner_ph - phys_dlg_h) / 2;
        // Convert physical screen coordinates to logical for anyui_create_window
        (theme::unscale(cx), theme::unscale(cy))
    } else {
        (-1, -1) // fallback: auto-placement
    }
}

/// Show a modal message box. Blocks until the user dismisses it.
///
/// `msg_type`: 0 = alert (red), 1 = info (blue), 2 = warning (yellow).
/// `text/text_len`: the message string.
/// `btn_text/btn_text_len`: button label (e.g. "OK").
#[no_mangle]
pub extern "C" fn anyui_message_box(
    msg_type: u32,
    text: *const u8,
    text_len: u32,
    btn_text: *const u8,
    btn_text_len: u32,
) {
    let st = state();
    if st.windows.is_empty() {
        return;
    }

    let owner_win_id = *st.windows.last().unwrap();

    let text_slice = if !text.is_null() && text_len > 0 {
        unsafe { core::slice::from_raw_parts(text, text_len as usize) }
    } else {
        b"Message"
    };
    let btn_slice = if !btn_text.is_null() && btn_text_len > 0 {
        unsafe { core::slice::from_raw_parts(btn_text, btn_text_len as usize) }
    } else {
        b"OK"
    };

    // Icon and accent color based on type
    let (icon_char, icon_color, title) = match msg_type {
        0 => (b"!" as &[u8], 0xFFFF3B30u32, b"Alert" as &[u8]),
        1 => (b"i" as &[u8], 0xFF007AFFu32, b"Info" as &[u8]),
        _ => (b"!" as &[u8], 0xFFFFD60Au32, b"Warning" as &[u8]),
    };

    // Measure button text to determine button width (with 24px padding).
    // Use scaled font size so measurement matches what gets rendered.
    let scaled_fs = draw::scale_font(13);
    let (btn_tw, _) = draw::text_size_at(btn_slice, scaled_fs);
    let btn_w = (btn_tw + 24).max(80);

    // Measure message text to determine dialog width.
    // For single-line messages, expand the dialog so the text fits.
    let (msg_tw, _) = draw::text_size_at(text_slice, scaled_fs);
    let text_area_w = msg_tw + 20; // some breathing room
    let min_content = if text_area_w > btn_w {
        text_area_w
    } else {
        btn_w
    };
    // 72 = icon area (52) + right margin (20), scale limits for HiDPI
    let min_dlg = theme::scale(320);
    let max_dlg = theme::scale(520);
    let icon_margin = theme::scale(72);
    let dlg_w = (min_content + icon_margin).max(min_dlg).min(max_dlg);
    let msg_label_w = dlg_w - icon_margin;

    // Word-wrap: insert '\n' when the message text exceeds the label width.
    let mut wrapped = alloc::vec::Vec::new();
    {
        let max_w = msg_label_w - 8; // subtract a bit of padding
        let mut start = 0;
        while start < text_slice.len() {
            // Find explicit newline
            let chunk_end = text_slice[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| start + p)
                .unwrap_or(text_slice.len());
            let chunk = &text_slice[start..chunk_end];

            // Check if this chunk fits
            let (cw, _) = draw::text_size_at(chunk, scaled_fs);
            if cw <= max_w || chunk.len() <= 1 {
                // Fits — append as-is
                if !wrapped.is_empty() {
                    wrapped.push(b'\n');
                }
                wrapped.extend_from_slice(chunk);
            } else {
                // Too wide — break at word boundaries
                let mut line_start = 0;
                while line_start < chunk.len() {
                    // Try progressively longer prefixes, break at last space
                    let mut best_end = chunk.len().min(line_start + 1); // at least 1 char
                    for i in (line_start + 1)..=chunk.len() {
                        let (pw, _) = draw::text_size_at(&chunk[line_start..i], scaled_fs);
                        if pw > max_w {
                            break;
                        }
                        best_end = i;
                    }
                    // Try to break at a space
                    if best_end < chunk.len() {
                        if let Some(sp) =
                            chunk[line_start..best_end].iter().rposition(|&b| b == b' ')
                        {
                            if sp > 0 {
                                best_end = line_start + sp + 1;
                            }
                        }
                    }
                    if !wrapped.is_empty() {
                        wrapped.push(b'\n');
                    }
                    wrapped.extend_from_slice(&chunk[line_start..best_end]);
                    line_start = best_end;
                    // Skip leading space on next line
                    if line_start < chunk.len() && chunk[line_start] == b' ' {
                        line_start += 1;
                    }
                }
            }

            if chunk_end >= text_slice.len() {
                break;
            }
            start = chunk_end + 1; // skip '\n'
        }
    }
    let wrapped_slice = &wrapped;

    // Count lines for height calculation (scale line height)
    let line_count = wrapped_slice.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_h = theme::scale(18);
    let msg_h = ((line_count as u32) * line_h).max(theme::scale(40));
    let btn_h = theme::scale(32);
    let btn_margin = theme::scale(48);
    let dlg_h = (msg_h + theme::scale(80)).max(theme::scale(160));

    // Compute center position relative to the owner window
    let (dlg_x, dlg_y) = center_on_owner(owner_win_id, dlg_w, dlg_h);

    // Create standalone MessageBox window
    // Flags: NOT_RESIZABLE(0x02) | NO_MINIMIZE(0x10) | NO_MAXIMIZE(0x20)
    let dlg_win_id = anyui_create_window(
        title.as_ptr(),
        title.len() as u32,
        dlg_x,
        dlg_y,
        dlg_w,
        dlg_h,
        0x02 | 0x10 | 0x20,
    );
    if dlg_win_id == 0 {
        return;
    }

    // Make it modal to the owner window
    anyui_set_modal(dlg_win_id, owner_win_id);

    // Scaled positions for child controls
    let icon_x = theme::scale_i32(20);
    let icon_y = theme::scale_i32(16);
    let icon_sz = theme::scale(24);
    let msg_x = theme::scale_i32(52);

    // Allocate child IDs
    let st = state();
    let icon_id = st.next_id;
    st.next_id += 1;
    let msg_id = st.next_id;
    st.next_id += 1;
    let btn_id = st.next_id;
    st.next_id += 1;

    // Icon label (round colored circle with centered letter)
    let mut icon = controls::create_control(
        ControlKind::Label,
        icon_id,
        dlg_win_id,
        icon_x,
        icon_y,
        icon_sz,
        icon_sz,
        icon_char,
    );
    icon.set_color(icon_color);
    icon.base_mut().state = 1; // center text
    if let Some(tb) = icon.text_base_mut() {
        tb.text_style.text_color = 0xFFFFFFFF;
        tb.text_style.font_size = 14;
    }
    st.controls.push(icon);
    if let Some(w) = st.controls.iter_mut().find(|c| c.id() == dlg_win_id) {
        w.add_child(icon_id);
    }

    // Message label (word-wrapped text)
    let msg = controls::create_control(
        ControlKind::Label,
        msg_id,
        dlg_win_id,
        msg_x,
        icon_y,
        msg_label_w,
        msg_h,
        wrapped_slice,
    );
    st.controls.push(msg);
    if let Some(w) = st.controls.iter_mut().find(|c| c.id() == dlg_win_id) {
        w.add_child(msg_id);
    }

    // OK button (auto-sized)
    let btn = controls::create_control(
        ControlKind::Button,
        btn_id,
        dlg_win_id,
        ((dlg_w as i32) - btn_w as i32) / 2,
        (dlg_h as i32) - btn_margin as i32,
        btn_w,
        btn_h,
        btn_slice,
    );
    st.controls.push(btn);
    if let Some(w) = st.controls.iter_mut().find(|c| c.id() == dlg_win_id) {
        w.add_child(btn_id);
    }

    // Register click handler on the button
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == btn_id) {
        b.set_event_callback(control::EVENT_CLICK, msgbox_ok_clicked, 0);
    }
    // Window close button (X) → same as OK
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == dlg_win_id) {
        b.set_event_callback(control::EVENT_CLOSE, msgbox_ok_clicked, 0);
    }

    // Mini event loop — block until dismissed
    unsafe {
        MSGBOX_DISMISSED = false;
    }
    while !unsafe { MSGBOX_DISMISSED } {
        let t0 = syscall::uptime_ms();
        if event_loop::run_once() == 0 {
            break;
        }
        let elapsed = syscall::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 {
            syscall::sleep(16 - elapsed);
        }
    }

    // Destroy dialog window — auto-clears modal + removes all children
    anyui_destroy_window(dlg_win_id);
}

// ── File Dialogs ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_open_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    dialogs::open_folder(result_buf, buf_len)
}

#[no_mangle]
pub extern "C" fn anyui_open_file(result_buf: *mut u8, buf_len: u32) -> u32 {
    dialogs::open_file(result_buf, buf_len)
}

#[no_mangle]
pub extern "C" fn anyui_save_file(
    result_buf: *mut u8,
    buf_len: u32,
    default_name: *const u8,
    name_len: u32,
) -> u32 {
    let name = if !default_name.is_null() && name_len > 0 {
        unsafe { core::slice::from_raw_parts(default_name, name_len as usize) }
    } else {
        &[]
    };
    dialogs::save_file(result_buf, buf_len, name)
}

#[no_mangle]
pub extern "C" fn anyui_create_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    dialogs::create_folder(result_buf, buf_len)
}

// ── Event loop ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_run() {
    event_loop::run();
}

#[no_mangle]
pub extern "C" fn anyui_run_once() -> u32 {
    event_loop::run_once()
}

#[no_mangle]
pub extern "C" fn anyui_quit() {
    state().quit_requested = true;
}

// ── Timers ───────────────────────────────────────────────────────────

/// Register a periodic timer. Returns a timer ID (>0).
/// The callback fires on the UI thread during run_once(), at approximately
/// the given interval. Receives (timer_id, 0, userdata).
#[no_mangle]
pub extern "C" fn anyui_set_timer(interval_ms: u32, cb: control::Callback, userdata: u64) -> u32 {
    state().timers.set_timer(interval_ms, cb, userdata)
}

/// Remove a timer by ID. No-op if the timer ID is invalid.
#[no_mangle]
pub extern "C" fn anyui_kill_timer(timer_id: u32) {
    state().timers.kill_timer(timer_id);
}

// ── Z-order ─────────────────────────────────────────────────────────

/// Move a control to the end of its parent's child list (render on top).
#[no_mangle]
pub extern "C" fn anyui_bring_to_front(id: ControlId) {
    let st = state();
    let parent_id = match st.controls.iter().find(|c| c.id() == id) {
        Some(ctrl) => ctrl.parent_id(),
        None => return,
    };
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent_id) {
        p.remove_child(id);
        p.add_child(id);
        p.base_mut().mark_dirty();
    }
}

// ── Control removal ──────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_remove(id: ControlId) {
    let st = state();

    // Collect all descendants
    let mut to_remove = Vec::new();
    collect_descendants(st, id, &mut to_remove);
    to_remove.push(id);

    // Clear tracking for removed controls
    for &rid in &to_remove {
        if st.focused == Some(rid) {
            st.focused = None;
        }
        if st.pressed == Some(rid) {
            st.pressed = None;
        }
        if st.hovered == Some(rid) {
            st.hovered = None;
        }
    }

    // Remove from parent's children
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let parent = ctrl.parent_id();
        if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent) {
            p.remove_child(id);
        }
    }

    st.controls.retain(|c| !to_remove.contains(&c.id()));
}

/// Remove a specific child from a parent container and destroy it.
///
/// Equivalent to `anyui_remove(child)` but expressed as a parent operation.
#[no_mangle]
pub extern "C" fn anyui_remove_child(parent: ControlId, child: ControlId) {
    // Verify the child actually belongs to this parent
    let st = state();
    let is_child = st
        .controls
        .iter()
        .find(|c| c.id() == child)
        .map(|c| c.parent_id() == parent)
        .unwrap_or(false);
    if is_child {
        anyui_remove(child);
    }
}

/// Remove and destroy ALL children of a container.
///
/// Collects all direct children, then removes each (including their
/// descendants). The container itself is preserved.
#[no_mangle]
pub extern "C" fn anyui_clear_children(parent: ControlId) {
    let st = state();

    // Collect direct children IDs
    let children: Vec<ControlId> = match st.controls.iter().find(|c| c.id() == parent) {
        Some(p) => p.children().to_vec(),
        None => return,
    };

    // Collect all descendants of each child
    let mut to_remove = Vec::new();
    for &child in &children {
        to_remove.push(child);
        collect_descendants(st, child, &mut to_remove);
    }

    // Clear tracking for removed controls
    for &rid in &to_remove {
        if st.focused == Some(rid) {
            st.focused = None;
        }
        if st.pressed == Some(rid) {
            st.pressed = None;
        }
        if st.hovered == Some(rid) {
            st.hovered = None;
        }
    }

    // Clear parent's children list
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent) {
        p.base_mut().children.clear();
    }

    // Remove all collected controls
    st.controls.retain(|c| !to_remove.contains(&c.id()));
}

/// Programmatically resize a window (SHM buffer, back buffer, control size).
/// Used by the dock (and similar borderless windows) to react to resolution changes.
///
/// `new_w`/`new_h` are **logical** pixels (what the app sees). The SHM surface
/// and back-buffer are allocated at the corresponding physical dimensions.
#[no_mangle]
pub extern "C" fn anyui_resize_window(win_id: ControlId, new_w: u32, new_h: u32) {
    let st = state();
    // Convert logical → physical for the compositor surface.
    let phys_w = crate::theme::scale(new_w);
    let phys_h = crate::theme::scale(new_h);
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let cw = &mut st.comp_windows[wi];
        if cw.logical_width == new_w && cw.logical_height == new_h {
            return;
        }
        if let Some((new_shm_id, new_surface)) =
            compositor::resize_shm(st.channel_id, cw.window_id, cw.shm_id, phys_w, phys_h)
        {
            cw.shm_id = new_shm_id;
            cw.surface = new_surface;
        }
        cw.width = phys_w;
        cw.height = phys_h;
        cw.logical_width = new_w;
        cw.logical_height = new_h;
        let new_count = (phys_w as usize) * (phys_h as usize);
        cw.back_buffer.resize(new_count, 0);
        cw.present_pending = false;
        cw.pending_present_rect = None;
        cw.dirty = true;
        cw.dirty_rect = None; // full redraw
    }
    // Control tree uses logical dimensions.
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == win_id) {
        ctrl.set_size(new_w, new_h);
    }
    mark_needs_layout();
}

/// Mark a window as fullscreen-capable.
/// auto_enter: if non-zero, immediately enter fullscreen mode.
#[no_mangle]
pub extern "C" fn anyui_set_fullscreen_capable(win_id: ControlId, auto_enter: u32) {
    let st = state();
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let comp_win_id = st.comp_windows[wi].window_id;
        compositor::set_fullscreen_capable(st.channel_id, comp_win_id, auto_enter);
    }
}

/// Get fullscreen info from the last EVT_FULLSCREEN_ENTER event.
/// Returns: out[0] = width, out[1] = height, out[2] = stride, out[3] = fb_ptr.
/// Returns 1 if in fullscreen, 0 if not.
#[no_mangle]
pub extern "C" fn anyui_get_fullscreen_info(out: *mut u32) -> u32 {
    let info = FULLSCREEN_INFO.load(core::sync::atomic::Ordering::Relaxed);
    if info == 0 {
        return 0;
    }
    let packed_size = info as u32;
    let stride = (info >> 32) as u32;
    let fb_ptr = FULLSCREEN_FB_PTR.load(core::sync::atomic::Ordering::Relaxed);
    if !out.is_null() {
        unsafe {
            *out = packed_size >> 16; // width
            *out.add(1) = packed_size & 0xFFFF; // height
            *out.add(2) = stride;
            *out.add(3) = fb_ptr;
        }
    }
    1
}

/// Hide or show the mouse cursor for a window.
/// visible=0 hides the cursor, visible=1 shows it.
/// Uses cursor shape 0xFF (hidden) via CMD_SET_CURSOR.
#[no_mangle]
pub extern "C" fn anyui_set_cursor_visible(win_id: ControlId, visible: u32) {
    let st = state();
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let comp_win_id = st.comp_windows[wi].window_id;
        // 0xFF = hidden cursor shape, 0 = default arrow
        let shape = if visible != 0 { 0u32 } else { 0xFF };
        let cmd: [u32; 5] = [0x1018, comp_win_id, shape, 0, 0];
        crate::syscall::evt_chan_emit(st.channel_id, &cmd);
        st.current_cursor = shape;
    }
}

/// Flush a region of the display after direct framebuffer writes.
///
/// When a fullscreen app has been granted direct framebuffer access (fb_ptr != 0
/// in the FULLSCREEN_ENTER event), it writes pixels directly to GPU VRAM. The GPU
/// display controller (SVGA) requires an explicit update command to refresh the
/// changed region on screen. This function sends CMD_FLUSH_DISPLAY to the compositor,
/// which issues the GPU update command.
///
/// # Parameters
/// - `x`, `y`: Top-left corner of the dirty region (in pixels).
/// - `w`, `h`: Width and height of the dirty region (in pixels).
///
/// Pass (0, 0, screen_width, screen_height) to flush the entire screen.
/// Has no effect if the app does not have direct framebuffer access.
#[no_mangle]
pub extern "C" fn anyui_flush_display(x: u32, y: u32, w: u32, h: u32) {
    let st = state();
    let cmd: [u32; 5] = [0x1033, x, y, w, h];
    crate::syscall::evt_chan_emit(st.channel_id, &cmd);
}

/// Minimize a window (move off-screen, compositor saves bounds for later restore).
#[no_mangle]
pub extern "C" fn anyui_minimize_window(win_id: ControlId) {
    let st = state();
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let comp_win_id = st.comp_windows[wi].window_id;
        compositor::minimize_window(st.channel_id, comp_win_id);
    }
}

/// Move a window to a new screen position.
#[no_mangle]
pub extern "C" fn anyui_move_window(win_id: ControlId, x: i32, y: i32) {
    let st = state();
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let comp_win_id = st.comp_windows[wi].window_id;
        // Convert logical position to physical screen coordinates.
        let phys_x = crate::theme::scale_i32(x);
        let phys_y = crate::theme::scale_i32(y);
        compositor::move_window(st.channel_id, comp_win_id, phys_x, phys_y);
    }
}

#[no_mangle]
pub extern "C" fn anyui_destroy_window(win_id: ControlId) {
    // Clear modal relationship before destroying
    anyui_clear_modal(win_id);

    let st = state();

    if let Some(idx) = st.windows.iter().position(|&w| w == win_id) {
        let cw = &st.comp_windows[idx];
        compositor::destroy_window(st.channel_id, cw.window_id, cw.shm_id);
        st.comp_windows.remove(idx);
        st.windows.remove(idx);
    }

    anyui_remove(win_id);
}

// ── Modal dialog API ─────────────────────────────────────────────────

/// Mark a window as a modal child of another window.
/// The modal window will block input to the owner and stay on top.
#[no_mangle]
pub extern "C" fn anyui_set_modal(modal_id: ControlId, owner_id: ControlId) {
    let st = state();

    let modal_win_idx = st.windows.iter().position(|&w| w == modal_id);
    let owner_win_idx = st.windows.iter().position(|&w| w == owner_id);

    if let (Some(mi), Some(oi)) = (modal_win_idx, owner_win_idx) {
        let modal_comp_wid = st.comp_windows[mi].window_id;
        let owner_comp_wid = st.comp_windows[oi].window_id;

        // Tell compositor about the modal relationship
        compositor::set_modal_owner(st.channel_id, modal_comp_wid, owner_comp_wid);

        // Push onto modal stack
        st.modal_stack.push(ModalEntry {
            modal_win_id: modal_id,
            owner_win_id: owner_id,
            overlay_id: 0, // separate-window modal, no in-window overlay
        });
    }
}

/// Clear the modal relationship for a window.
/// Called automatically by anyui_destroy_window.
#[no_mangle]
pub extern "C" fn anyui_clear_modal(modal_id: ControlId) {
    let st = state();

    // Find and remove from modal stack
    if let Some(pos) = st
        .modal_stack
        .iter()
        .position(|e| e.modal_win_id == modal_id)
    {
        st.modal_stack.remove(pos);

        // Tell compositor to clear the relationship
        if let Some(idx) = st.windows.iter().position(|&w| w == modal_id) {
            let modal_comp_wid = st.comp_windows[idx].window_id;
            compositor::set_modal_owner(st.channel_id, modal_comp_wid, 0);
        }
    }
}

fn collect_descendants(st: &AnyuiState, id: ControlId, out: &mut Vec<ControlId>) {
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let children: Vec<ControlId> = ctrl.children().to_vec();
        for &child in &children {
            out.push(child);
            collect_descendants(st, child, out);
        }
    }
}

// ── Blur-behind ─────────────────────────────────────────────────────

/// Enable or disable blur-behind on a window.
/// radius=0 disables, radius>0 enables with given kernel radius.
#[no_mangle]
pub extern "C" fn anyui_set_blur_behind(id: ControlId, radius: u32) {
    let st = state();
    if let Some(idx) = st.windows.iter().position(|&w| w == id) {
        compositor::set_blur_behind(st.channel_id, st.comp_windows[idx].window_id, radius);
    }
}

// ── Focus management ────────────────────────────────────────────────

/// Programmatically set keyboard focus to a control.
#[no_mangle]
pub extern "C" fn anyui_set_focus(id: ControlId) {
    let st = state();
    // Blur the currently focused control
    if let Some(old_id) = st.focused {
        if old_id != id {
            if let Some(idx) = control::find_idx(&st.controls, old_id) {
                st.controls[idx].handle_blur();
            }
        }
    }
    // Focus the new control
    if let Some(idx) = control::find_idx(&st.controls, id) {
        st.controls[idx].handle_focus();
        st.focused = Some(id);
    }
}

/// Set the tab focus order index for a control.
/// Controls with lower tab_index receive focus first when Tab is pressed.
/// The index is cascaded: parent tab_index sorts first, then child tab_index.
/// Default is 0 (insertion order).
#[no_mangle]
pub extern "C" fn anyui_set_tab_index(id: ControlId, index: u32) {
    let st = state();
    if let Some(idx) = control::find_idx(&st.controls, id) {
        st.controls[idx].base_mut().tab_index = index;
    }
}

// ── Screen size ─────────────────────────────────────────────────────

/// Get screen dimensions. Returns (width, height) via out pointers.
#[no_mangle]
pub extern "C" fn anyui_screen_size(out_w: *mut u32, out_h: *mut u32) {
    let (w, h) = compositor::screen_size();
    // Return logical screen dimensions so apps work entirely in logical space.
    let lw = crate::theme::unscale_u32(w);
    let lh = crate::theme::unscale_u32(h);
    if !out_w.is_null() {
        unsafe {
            *out_w = lw;
        }
    }
    if !out_h.is_null() {
        unsafe {
            *out_h = lh;
        }
    }
}

// ── Notifications ───────────────────────────────────────────────────

/// Show a notification banner via the compositor.
///
/// `title_ptr`/`title_len`: notification title (UTF-8, max 64 bytes).
/// `msg_ptr`/`msg_len`: notification message (UTF-8, max 128 bytes).
/// `icon_ptr`: optional 16x16 ARGB pixel data (256 u32s), null for no icon.
/// `timeout_ms`: auto-dismiss timeout (0 = default 5s).
#[no_mangle]
pub extern "C" fn anyui_show_notification(
    title_ptr: *const u8,
    title_len: u32,
    msg_ptr: *const u8,
    msg_len: u32,
    icon_ptr: *const u32,
    timeout_ms: u32,
) {
    let st = state();
    let title = if !title_ptr.is_null() && title_len > 0 {
        unsafe { core::slice::from_raw_parts(title_ptr, title_len as usize) }
    } else {
        b""
    };
    let message = if !msg_ptr.is_null() && msg_len > 0 {
        unsafe { core::slice::from_raw_parts(msg_ptr, msg_len as usize) }
    } else {
        b""
    };
    compositor::show_notification(st.channel_id, title, message, icon_ptr, timeout_ms, 0);
}

// ── Theme ────────────────────────────────────────────────────────────

/// Set the system theme.
///
/// Updates the local fallback AND sends CMD_SET_THEME (0x100D) to the
/// compositor, which writes to the shared uisys DLIB page so all apps
/// pick up the new theme and persists the choice to `compositor.conf`.
#[no_mangle]
pub extern "C" fn anyui_set_theme(light: u32) {
    let val = light.min(1);
    theme::set_theme(val != 0);
    // Notify the compositor so it updates the shared page + persists.
    let channel_id = state().channel_id;
    if channel_id != 0 {
        let cmd: [u32; 5] = [0x100D, val, 0, 0, 0]; // CMD_SET_THEME
        syscall::evt_chan_emit(channel_id, &cmd);
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_theme() -> u32 {
    theme::get_theme()
}

/// Return a pointer to the live theme palette.
///
/// Client code in the same address space can dereference this directly
/// instead of duplicating the color data.
#[no_mangle]
pub extern "C" fn anyui_get_theme_colors_ptr() -> *const theme::ThemeColors {
    theme::colors_ptr()
}

/// Apply accent style overrides to both dark and light palettes.
#[no_mangle]
pub extern "C" fn anyui_apply_accent_style(
    dark_accent: u32,
    dark_hover: u32,
    light_accent: u32,
    light_hover: u32,
) {
    theme::apply_accent_style(dark_accent, dark_hover, light_accent, light_hover);
}

/// Set the font smoothing mode system-wide.
///
/// Sends CMD_SET_FONT_SMOOTHING (0x1016) to the compositor, which writes
/// to the shared uisys DLIB page and persists the choice to `compositor.conf`.
/// mode: 0 = no smoothing, 1 = greyscale AA, 2 = subpixel LCD.
#[no_mangle]
pub extern "C" fn anyui_set_font_smoothing(mode: u32) {
    let val = mode.min(2);
    let channel_id = state().channel_id;
    if channel_id != 0 {
        let cmd: [u32; 5] = [0x1016, val, 0, 0, 0]; // CMD_SET_FONT_SMOOTHING
        syscall::evt_chan_emit(channel_id, &cmd);
    }
}

/// Get the current font smoothing mode from the shared uisys page.
///
/// Returns: 0 = no smoothing, 1 = greyscale AA, 2 = subpixel LCD.
#[no_mangle]
pub extern "C" fn anyui_get_font_smoothing() -> u32 {
    unsafe { core::ptr::read_volatile(0x0400_0010 as *const u32) }
}

// ── DPI Scale Factor ────────────────────────────────────────────

/// Set the DPI scale factor system-wide (100–300 in 25% steps).
///
/// Sends CMD_SET_SCALE to the compositor, which writes to the shared page
/// and persists the setting.
#[no_mangle]
pub extern "C" fn anyui_set_scale_factor(percent: u32) {
    let clamped = percent.max(100).min(300);
    let rounded = ((clamped + 12) / 25) * 25;
    let channel_id = state().channel_id;
    if channel_id != 0 {
        let cmd: [u32; 5] = [0x1017, rounded, 0, 0, 0]; // CMD_SET_SCALE
        syscall::evt_chan_emit(channel_id, &cmd);
    }
}

/// Get the current DPI scale factor from the shared uisys page.
///
/// Returns: scale percentage (100 = 1x, 200 = 2x, etc.).
#[no_mangle]
pub extern "C" fn anyui_get_scale_factor() -> u32 {
    let v = unsafe { core::ptr::read_volatile(0x0400_0014 as *const u32) };
    if v >= 100 && v <= 300 {
        v
    } else {
        100
    }
}

// ── Window title (post-creation) ─────────────────────────────────

/// Set the title of a window after creation.
#[no_mangle]
pub extern "C" fn anyui_set_title(id: ControlId, title: *const u8, title_len: u32) {
    let st = state();
    if let Some(idx) = st.windows.iter().position(|&w| w == id) {
        let text = if !title.is_null() && title_len > 0 {
            unsafe { core::slice::from_raw_parts(title, title_len as usize) }
        } else {
            &[]
        };
        compositor::set_title(st.channel_id, st.comp_windows[idx].window_id, text);
        // Also update the control's text
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
            ctrl.set_text(text);
        }
    }
}

// ── Key event info ──────────────────────────────────────────────

/// Query the last key event info. Returns keycode, char_code, modifiers via out pointers.
/// Call this from inside a KEY event callback to get the key that was pressed.
#[no_mangle]
pub extern "C" fn anyui_get_key_info(
    out_keycode: *mut u32,
    out_char_code: *mut u32,
    out_modifiers: *mut u32,
) {
    let st = state();
    if !out_keycode.is_null() {
        unsafe {
            *out_keycode = st.last_keycode;
        }
    }
    if !out_char_code.is_null() {
        unsafe {
            *out_char_code = st.last_char_code;
        }
    }
    if !out_modifiers.is_null() {
        unsafe {
            *out_modifiers = st.last_modifiers;
        }
    }
}

// ── Clipboard ───────────────────────────────────────────────────

/// Copy text to the system clipboard.
#[no_mangle]
pub extern "C" fn anyui_clipboard_set(data: *const u8, len: u32) {
    if !data.is_null() && len > 0 {
        let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
        compositor::clipboard_set(slice);
    }
}

/// Get text from the system clipboard. Returns number of bytes written.
/// Returns 0 if clipboard is empty.
#[no_mangle]
pub extern "C" fn anyui_clipboard_get(out: *mut u8, capacity: u32) -> u32 {
    if let Some(data) = compositor::clipboard_get() {
        let copy_len = data.len().min(capacity as usize);
        if !out.is_null() && copy_len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr(), out, copy_len);
            }
        }
        let preview_len = copy_len.min(30);
        let preview = core::str::from_utf8(&data[..preview_len]).unwrap_or("?");
        crate::log!(
            "[anyui_clipboard_get] got {} bytes: '{}'",
            copy_len,
            preview
        );
        copy_len as u32
    } else {
        crate::log!("[anyui_clipboard_get] empty");
        0
    }
}

// ── Window size query ───────────────────────────────────────────

/// Get the size of a control. Returns via out pointers.
#[no_mangle]
pub extern "C" fn anyui_get_size(id: ControlId, out_w: *mut u32, out_h: *mut u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if !out_w.is_null() {
            unsafe {
                *out_w = ctrl.base().w;
            }
        }
        if !out_h.is_null() {
            unsafe {
                *out_h = ctrl.base().h;
            }
        }
    }
}

/// Get the position of a control. Returns via out pointers.
///
/// For Window controls, this returns the **compositor-owned frame
/// position** (physical pixels) cached in the matching `CompWindow`
/// rather than `controls[].base().x/y`. Reasons:
///
/// 1. The window-frame position is owned by the compositor (drag,
///    "move to other monitor", maximize/restore) and is communicated
///    back via EVT_WINDOW_MOVED — the cache is always live.
/// 2. `controls[Window].base().x/y` MUST stay (0,0) because
///    `abs_position` walks parents and adds those coords; a non-zero
///    value would shift every descendant control's hit-test and
///    layout origin by the same offset, mis-rendering everything in
///    the window.
///
/// For non-Window controls the historical control-tree-relative
/// position is returned as before.
#[no_mangle]
pub extern "C" fn anyui_get_position(id: ControlId, out_x: *mut i32, out_y: *mut i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        let (x, y) = if ctrl.kind() == control::ControlKind::Window {
            // st.windows[i] is the control id of the i-th window;
            // st.comp_windows[i] is the parallel CompWindow.
            st.windows
                .iter()
                .position(|&w| w == id)
                .and_then(|wi| st.comp_windows.get(wi))
                .map(|cw| (cw.frame_x, cw.frame_y))
                .unwrap_or((ctrl.base().x, ctrl.base().y))
        } else {
            (ctrl.base().x, ctrl.base().y)
        };
        if !out_x.is_null() {
            unsafe {
                *out_x = x;
            }
        }
        if !out_y.is_null() {
            unsafe {
                *out_y = y;
            }
        }
    }
}


/// Get the absolute position of a control in window coordinates. This mirrors
/// hit-testing offsets, including ScrollView scroll positions.
#[no_mangle]
pub extern "C" fn anyui_get_abs_position(id: ControlId, out_x: *mut i32, out_y: *mut i32) {
    let st = state();
    let (x, y) = control::abs_position(&st.controls, id);
    if !out_x.is_null() {
        unsafe {
            *out_x = x;
        }
    }
    if !out_y.is_null() {
        unsafe {
            *out_y = y;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_scrollview_get_offsets(id: ControlId, out_x: *mut i32, out_y: *mut i32) {
    let st = state();
    let (x, y) = controls::scroll_view::scroll_offsets(&st.controls, id);
    if !out_x.is_null() {
        unsafe {
            *out_x = x;
        }
    }
    if !out_y.is_null() {
        unsafe {
            *out_y = y;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_scrollview_set_offsets(id: ControlId, x: i32, y: i32) {
    let st = state();
    if let Some(idx) = control::find_idx(&st.controls, id) {
        if let Some(sv) = control::cast_mut::<controls::scroll_view::ScrollView>(
            &mut st.controls[idx],
            control::ControlKind::ScrollView,
        ) {
            sv.set_scroll_offsets(x, y);
        }
    }
}

// ── DataGrid scroll position ────────────────────────────────────

/// Get the current scroll Y position of a DataGrid (in pixels).
#[no_mangle]
pub extern "C" fn anyui_datagrid_get_scroll_offset(id: ControlId) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            return dg.scroll_y.max(0) as u32;
        }
    }
    0
}

/// Set the scroll Y position of a DataGrid (in pixels).
#[no_mangle]
pub extern "C" fn anyui_datagrid_set_scroll_offset(id: ControlId, offset: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.scroll_y = offset as i32;
            dg.base.mark_dirty();
        }
    }
}

// ── Compositor channel access ────────────────────────────────────

/// Return the compositor event channel ID for direct IPC commands.
#[no_mangle]
pub extern "C" fn anyui_get_compositor_channel() -> u32 {
    state().channel_id
}

// ── Window lifecycle callbacks ──────────────────────────────────

/// Register a callback for EVT_WINDOW_OPENED (0x0060).
/// Callback receives (app_tid, 0x0060, userdata).
#[no_mangle]
pub extern "C" fn anyui_on_window_opened(cb: Callback, userdata: u64) {
    state().on_window_opened = Some((cb, userdata));
}

/// Register a callback for EVT_WINDOW_CLOSED (0x0061).
/// Callback receives (app_tid, 0x0061, userdata).
#[no_mangle]
pub extern "C" fn anyui_on_window_closed(cb: Callback, userdata: u64) {
    state().on_window_closed = Some((cb, userdata));
}

/// Register a callback for the window list response.
/// Callback receives (count, 0x0065, userdata) when the list is complete.
/// Use `anyui_get_window_list_buffer()` to read the collected TIDs.
#[no_mangle]
pub extern "C" fn anyui_on_window_list(cb: Callback, userdata: u64) {
    state().on_window_list = Some((cb, userdata));
}

/// Send CMD_LIST_WINDOW_TIDS to the compositor.
/// The response arrives asynchronously via the on_window_list callback.
#[no_mangle]
pub extern "C" fn anyui_request_window_list() {
    let st = state();
    let channel_id = st.channel_id;
    if channel_id == 0 {
        return;
    }
    st.window_list_buffer.clear();
    let my_tid = syscall::get_tid();
    let cmd: [u32; 5] = [0x1034, my_tid, 0, 0, 0]; // CMD_LIST_WINDOW_TIDS
    syscall::evt_chan_emit(channel_id, &cmd);
}

/// Get a pointer to the window list buffer and its length.
/// Valid only inside the on_window_list callback. Returns (ptr, count).
#[no_mangle]
pub extern "C" fn anyui_get_window_list_buffer(out_count: *mut u32) -> *const u32 {
    let st = state();
    if !out_count.is_null() {
        unsafe {
            *out_count = st.window_list_buffer.len() as u32;
        }
    }
    st.window_list_buffer.as_ptr()
}

// ── Focus by task ID ────────────────────────────────────────────────

/// Send CMD_FOCUS_BY_TID to the compositor to bring a window to the front.
#[no_mangle]
pub extern "C" fn anyui_focus_by_tid(tid: u32) {
    let channel_id = state().channel_id;
    if channel_id == 0 {
        return;
    }
    let cmd: [u32; 5] = [0x100A, tid, 0, 0, 0]; // CMD_FOCUS_BY_TID
    syscall::evt_chan_emit(channel_id, &cmd);
}

// ── Tray icon management ────────────────────────────────────────────

/// Register or update a 16×16 ARGB tray icon via the compositor.
#[no_mangle]
pub extern "C" fn anyui_add_status_icon(icon_id: u32, pixels: *const u32) {
    let st = state();
    compositor::add_status_icon(st.channel_id, icon_id, pixels);
}

/// Remove a tray icon from the compositor.
#[no_mangle]
pub extern "C" fn anyui_remove_status_icon(icon_id: u32) {
    let st = state();
    compositor::remove_status_icon(st.channel_id, icon_id);
}

/// Register a callback for tray icon clicks.
/// Callback receives (icon_id, mouse_x, userdata).
#[no_mangle]
pub extern "C" fn anyui_on_tray_click(icon_id: u32, cb: Callback, userdata: u64) {
    let st = state();
    // Replace existing callback for this icon_id, or add new
    if let Some(entry) = st.tray_callbacks.iter_mut().find(|e| e.0 == icon_id) {
        entry.1 = cb;
        entry.2 = userdata;
    } else {
        st.tray_callbacks.push((icon_id, cb, userdata));
    }
}

// ── Menu bar management ─────────────────────────────────────────────

/// Helper: find compositor window_id for a given control window_id.
fn comp_window_id_for(st: &AnyuiState, win_ctrl_id: u32) -> Option<u32> {
    st.windows
        .iter()
        .position(|&w| w == win_ctrl_id)
        .map(|i| st.comp_windows[i].window_id)
}

/// Set a window's menu bar from binary data (MenuBarBuilder output).
/// `win_id` is the control window ID (from anyui_create_window).
#[no_mangle]
pub extern "C" fn anyui_set_menu(win_id: u32, menu_data: *const u8, menu_len: u32) {
    let st = state();
    if let Some(comp_wid) = comp_window_id_for(st, win_id) {
        compositor::set_menu(st.channel_id, comp_wid, menu_data, menu_len);
    }
}

/// Update a menu item's flags (enable/disable/check).
#[no_mangle]
pub extern "C" fn anyui_update_menu_item(win_id: u32, item_id: u32, new_flags: u32) {
    let st = state();
    if let Some(comp_wid) = comp_window_id_for(st, win_id) {
        compositor::update_menu_item(st.channel_id, comp_wid, item_id, new_flags);
    }
}

/// Register a callback for menu item clicks on a window.
/// Callback receives (item_id, EVT_MENU_ITEM, userdata).
#[no_mangle]
pub extern "C" fn anyui_on_menu_item(win_id: u32, cb: Callback, userdata: u64) {
    let st = state();
    if let Some(entry) = st.menu_callbacks.iter_mut().find(|e| e.0 == win_id) {
        entry.1 = cb;
        entry.2 = userdata;
    } else {
        st.menu_callbacks.push((win_id, cb, userdata));
    }
}

// ── Text measurement (for libwebview layout engine) ──────────────────

/// Measure a text string and return packed (width << 32 | height).
#[no_mangle]
pub extern "C" fn anyui_measure_text(
    text_ptr: *const u8,
    text_len: u32,
    font_id: u16,
    font_size: u16,
) -> u64 {
    let text = unsafe { core::slice::from_raw_parts(text_ptr, text_len as usize) };
    let (w, h) = draw::measure_text_ex(text, font_id, font_size);
    ((w as u64) << 32) | (h as u64)
}

// ── TabBar extensions ────────────────────────────────────────────────

fn as_tabbar(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::tabbar::TabBar> {
    control::cast_mut(ctrl, ControlKind::TabBar)
}

/// Show or hide the "+" (new-tab) button on a TabBar.
#[no_mangle]
pub extern "C" fn anyui_tabbar_show_plus(id: ControlId, show: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tb) = as_tabbar(ctrl) {
            tb.show_plus = show != 0;
            tb.text_base.base.mark_dirty();
        }
    }
}
