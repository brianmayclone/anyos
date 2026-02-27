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

mod compositor;
mod control;
mod controls;
pub mod draw;
mod event_loop;
pub mod font_bitmap;
mod layout;
mod marshal;
pub mod syscall;
mod timer;
mod dialogs;
pub mod icons;
pub mod theme;

use alloc::boxed::Box;
use alloc::vec::Vec;
use control::{Control, ControlId, ControlKind, Callback, DockStyle, Orientation};

// ── Compositor window handle ─────────────────────────────────────────

/// Per-window compositor state (SHM surface + IDs).
pub(crate) struct CompWindow {
    pub window_id: u32,
    pub shm_id: u32,
    pub surface: *mut u32,
    pub width: u32,
    pub height: u32,
    /// Back-pressure: true after present(), cleared on EVT_FRAME_ACK from compositor.
    pub frame_presented: bool,
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
    pub owner_win_idx: usize,
    /// Shadow margin (pixels of padding around the menu for shadow rendering).
    pub margin: i32,
    /// Whether the popup needs to be re-rendered (independent of control dirty flag,
    /// because Phase 4's clear_dirty clears the menu control's flag before popup render).
    pub dirty: bool,
    /// If this popup was opened by a DropDown, its control ID.
    /// When the popup item is selected, the DropDown's state is updated.
    pub owner_dropdown: Option<ControlId>,
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

    // ── Tooltip ──────────────────────────────────────────────────────
    /// Framework-managed tooltip control ID (created lazily on first use).
    pub active_tooltip: Option<ControlId>,

    // ── Context menu popup ──────────────────────────────────────────
    /// Active popup window for context menus (at most one at a time).
    pub popup: Option<PopupInfo>,

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

    // ── Window lifecycle callbacks (for dock/system integration) ──────
    /// Callback for EVT_WINDOW_OPENED (0x0060). Called with (app_tid, 0x0060, userdata).
    pub on_window_opened: Option<(Callback, u64)>,
    /// Callback for EVT_WINDOW_CLOSED (0x0061). Called with (app_tid, 0x0061, userdata).
    pub on_window_closed: Option<(Callback, u64)>,
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
    unsafe { STATE.as_mut().expect("anyui not initialized") }
}

// ── Allocator (free-list + sbrk per-allocation for DLL coexistence) ──
//
// DLL allocators share the sbrk address space with stdlib. We MUST call
// sbrk(0) + sbrk(n) for each new allocation to get fresh addresses that
// don't overlap with stdlib. Freed blocks go into a free list for reuse
// (via libheap).

mod allocator {
    use core::alloc::{GlobalAlloc, Layout};
    use core::ptr;
    use libheap::{FreeBlock, block_size, free_list_alloc, free_list_dealloc};

    struct DllFreeListAlloc;

    static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

    unsafe impl GlobalAlloc for DllFreeListAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = block_size(layout);

            // 1) Search free list for first fit (reuse freed memory)
            let ptr = free_list_alloc(&mut FREE_LIST, size);
            if !ptr.is_null() { return ptr; }

            // 2) No free block — get fresh memory from sbrk.
            //    Must call sbrk(0) each time to get the CURRENT break,
            //    since stdlib's allocator may have moved it.
            let brk = crate::syscall::sbrk(0);
            if brk == u64::MAX { return ptr::null_mut(); }
            let align = layout.align().max(16) as u64;
            let aligned = (brk + align - 1) & !(align - 1);
            let needed = (aligned - brk + size as u64) as u32;
            let result = crate::syscall::sbrk(needed);
            if result == u64::MAX { return ptr::null_mut(); }
            aligned as *mut u8
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            free_list_dealloc(&mut FREE_LIST, ptr, block_size(layout));
        }
    }

    #[global_allocator]
    static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;
}

// ── Panic handler ────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ══════════════════════════════════════════════════════════════════════
//  Exported C API
// ══════════════════════════════════════════════════════════════════════

/// Initialize the anyui framework. Must be called before any other function.
/// Connects to the compositor via libcompositor.dlib. Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_init() -> u32 {
    let mut sub_id: u32 = 0;
    let channel_id = compositor::init(&mut sub_id);
    if channel_id == 0 {
        return 0;
    }

    // Load theme palettes from /System/compositor/themes/{dark,light}.conf.
    // Falls back to built-in defaults for missing files / keys.
    theme::load_from_disk();

    unsafe {
        STATE = Some(AnyuiState {
            controls: Vec::new(),
            next_id: 1,
            windows: Vec::new(),
            comp_windows: Vec::new(),
            quit_requested: false,
            channel_id,
            sub_id,
            focused: None,
            pressed: None,
            hovered: None,
            last_click_id: None,
            last_click_tick: 0,
            click_count: 0,
            pressed_button: 0,
            active_tooltip: None,
            popup: None,
            timers: timer::TimerState::new(),
            needs_repaint: true,
            needs_layout: true,
            last_keycode: 0,
            last_char_code: 0,
            last_modifiers: 0,
            on_window_opened: None,
            on_window_closed: None,
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
    unsafe { STATE = None; }
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

    // Create compositor window via DLL
    let (window_id, shm_id, surface) =
        match compositor::create_window(st.channel_id, st.sub_id, x, y, w, h, flags) {
            Some(result) => result,
            None => return 0,
        };

    // Set title
    compositor::set_title(st.channel_id, window_id, &title_buf[..len]);

    let ctrl = controls::create_control(ControlKind::Window, id, 0, 0, 0, w, h, &title_buf[..len]);
    st.controls.push(ctrl);
    st.windows.push(id);
    let pixel_count = (w as usize) * (h as usize);
    st.comp_windows.push(CompWindow {
        window_id,
        shm_id,
        surface,
        width: w,
        height: h,
        frame_presented: false,
        last_present_ms: 0,
        dirty: true,
        dirty_rect: None,
        back_buffer: alloc::vec![0u32; pixel_count],
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
pub extern "C" fn anyui_create_control(
    kind: u32,
    text: *const u8,
    text_len: u32,
) -> ControlId {
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
    let parent_is_radio_group = st.controls.iter()
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
pub extern "C" fn anyui_set_state(id: ControlId, value: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_state(value);
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_state(id: ControlId) -> u32 {
    let st = state();
    st.controls.iter().find(|c| c.id() == id).map_or(0, |c| c.state_val())
}

// ── Layout properties ────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_padding(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().padding = control::Padding { left, top, right, bottom };
        ctrl.base_mut().mark_dirty();
    }
    mark_needs_layout();
}

#[no_mangle]
pub extern "C" fn anyui_set_margin(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().margin = control::Margin { left, top, right, bottom };
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
    st.controls.iter().find(|c| c.id() == id)
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
                let raw: *mut dyn Control = &mut **ctrl;
                let sp = unsafe { &mut *(raw as *mut controls::stack_panel::StackPanel) };
                sp.orientation = Orientation::from_u32(orientation);
            }
            ControlKind::SplitView => {
                let raw: *mut dyn Control = &mut **ctrl;
                let sv = unsafe { &mut *(raw as *mut controls::split_view::SplitView) };
                let new_orient = Orientation::from_u32(orientation);
                if sv.orientation != new_orient {
                    sv.orientation = new_orient;
                    sv.sync_divider();
                    sv.base.mark_dirty();
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
        if ctrl.kind() == ControlKind::TableLayout {
            let raw: *mut dyn Control = &mut **ctrl;
            let tl = unsafe { &mut *(raw as *mut controls::table_layout::TableLayout) };
            tl.columns = columns;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_row_height(id: ControlId, row_height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::TableLayout {
            let raw: *mut dyn Control = &mut **ctrl;
            let tl = unsafe { &mut *(raw as *mut controls::table_layout::TableLayout) };
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
        if ctrl.kind() == ControlKind::TableLayout {
            let raw: *mut dyn Control = &mut **ctrl;
            let tl = unsafe { &mut *(raw as *mut controls::table_layout::TableLayout) };
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
    if ctrl.kind() == ControlKind::SplitView {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::split_view::SplitView) })
    } else {
        None
    }
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

// ── TextField properties ─────────────────────────────────────────────

/// Helper to downcast a control to TextField.
fn as_textfield(ctrl: &mut Box<dyn Control>) -> Option<&mut controls::textfield::TextField> {
    if ctrl.kind() == ControlKind::TextField {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::textfield::TextField) })
    } else {
        None
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_prefix(id: ControlId, icon_code: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            let new_val = if icon_code == 0 { None } else { Some(icon_code) };
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
            let new_val = if icon_code == 0 { None } else { Some(icon_code) };
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

// ── Canvas operations ────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_canvas_set_pixel(id: ControlId, x: i32, y: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.set_pixel(x, y, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_clear(id: ControlId, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.clear(color);
            canvas.base.mark_dirty();
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_fill_rect(id: ControlId, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.fill_rect(x, y, w, h, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_line(id: ControlId, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.draw_line(x0, y0, x1, y1, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_rect(id: ControlId, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.draw_rect(x, y, w, h, color, thickness);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_draw_circle(id: ControlId, cx: i32, cy: i32, radius: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.draw_circle(cx, cy, radius, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_fill_circle(id: ControlId, cx: i32, cy: i32, radius: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
            canvas.fill_circle(cx, cy, radius, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_canvas_get_buffer(id: ControlId) -> *mut u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::Canvas {
            let raw: *mut dyn Control = &mut **ctrl;
            let canvas = unsafe { &mut *(raw as *mut controls::canvas::Canvas) };
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
    if ctrl.kind() == ControlKind::Canvas {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::canvas::Canvas) })
    } else {
        None
    }
}

fn as_canvas_ref(ctrl: &Box<dyn Control>) -> Option<&controls::canvas::Canvas> {
    if ctrl.kind() == ControlKind::Canvas {
        let raw: *const dyn Control = &**ctrl;
        Some(unsafe { &*(raw as *const controls::canvas::Canvas) })
    } else {
        None
    }
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
pub extern "C" fn anyui_canvas_get_mouse(id: ControlId, out_x: *mut i32, out_y: *mut i32, out_button: *mut u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas_ref(ctrl) {
            if !out_x.is_null() { unsafe { *out_x = cv.last_mouse_x; } }
            if !out_y.is_null() { unsafe { *out_y = cv.last_mouse_y; } }
            if !out_button.is_null() { unsafe { *out_button = cv.mouse_button; } }
        }
    }
}

/// Draw a filled ellipse.
#[no_mangle]
pub extern "C" fn anyui_canvas_fill_ellipse(id: ControlId, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(cv) = as_canvas(ctrl) {
            cv.fill_ellipse(cx, cy, rx, ry, color);
        }
    }
}

/// Draw an ellipse outline.
#[no_mangle]
pub extern "C" fn anyui_canvas_draw_ellipse(id: ControlId, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
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
pub extern "C" fn anyui_canvas_draw_thick_line(id: ControlId, x0: i32, y0: i32, x1: i32, y1: i32, color: u32, thickness: u32) {
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

// ── ImageView ────────────────────────────────────────────────────────

/// Set pixel data for an ImageView from a decoded ARGB buffer.
#[no_mangle]
pub extern "C" fn anyui_imageview_set_pixels(id: ControlId, data: *const u32, w: u32, h: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::ImageView {
            let count = (w as usize) * (h as usize);
            if !data.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, count) };
                let raw: *mut dyn Control = &mut **ctrl;
                let iv = unsafe { &mut *(raw as *mut controls::image_view::ImageView) };
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
        if ctrl.kind() == ControlKind::ImageView {
            let raw: *mut dyn Control = &mut **ctrl;
            let iv = unsafe { &mut *(raw as *mut controls::image_view::ImageView) };
            if iv.scale_mode != mode {
                iv.scale_mode = mode;
                iv.base.mark_dirty();
            }
        }
    }
}

/// Get the original image dimensions. Returns via out pointers. Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_imageview_get_image_size(id: ControlId, out_w: *mut u32, out_h: *mut u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if ctrl.kind() == ControlKind::ImageView {
            let raw: *const dyn Control = &**ctrl;
            let iv = unsafe { &*(raw as *const controls::image_view::ImageView) };
            if !out_w.is_null() { unsafe { *out_w = iv.img_w; } }
            if !out_h.is_null() { unsafe { *out_h = iv.img_h; } }
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
        if ctrl.kind() == ControlKind::ImageView {
            let raw: *mut dyn Control = &mut **ctrl;
            let iv = unsafe { &mut *(raw as *mut controls::image_view::ImageView) };
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
        if ctrl.kind() == ControlKind::IconButton {
            let count = (w as usize) * (h as usize);
            if !data.is_null() && count > 0 {
                let slice = unsafe { core::slice::from_raw_parts(data, count) };
                let raw: *mut dyn Control = &mut **ctrl;
                let ib = unsafe { &mut *(raw as *mut controls::icon_button::IconButton) };
                ib.set_icon_pixels(slice, w, h);
            }
        }
    }
}

// ── DataGrid ─────────────────────────────────────────────────────────

fn as_data_grid(ctrl: &mut alloc::boxed::Box<dyn Control>) -> Option<&mut controls::data_grid::DataGrid> {
    if ctrl.kind() == ControlKind::DataGrid {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::data_grid::DataGrid) })
    } else {
        None
    }
}

fn as_data_grid_ref(ctrl: &alloc::boxed::Box<dyn Control>) -> Option<&controls::data_grid::DataGrid> {
    if ctrl.kind() == ControlKind::DataGrid {
        let raw: *const dyn Control = &**ctrl;
        Some(unsafe { &*(raw as *const controls::data_grid::DataGrid) })
    } else {
        None
    }
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
pub extern "C" fn anyui_datagrid_set_column_sort_type(id: ControlId, col_index: u32, sort_type: u32) {
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
pub extern "C" fn anyui_datagrid_set_cell(id: ControlId, row: u32, col: u32, text: *const u8, text_len: u32) {
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
pub extern "C" fn anyui_datagrid_get_cell(id: ControlId, row: u32, col: u32, buf: *mut u8, max_len: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid_ref(ctrl) {
            let text = dg.get_cell(row as usize, col as usize);
            let copy_len = text.len().min(max_len as usize);
            if !buf.is_null() && copy_len > 0 {
                unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buf, copy_len); }
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
                let bytes = unsafe { core::slice::from_raw_parts(data, count as usize * entry_size) };
                for i in 0..count as usize {
                    let off = i * entry_size;
                    if off + entry_size > bytes.len() { break; }
                    let start = u32::from_le_bytes([bytes[off], bytes[off+1], bytes[off+2], bytes[off+3]]) as usize;
                    let end = u32::from_le_bytes([bytes[off+4], bytes[off+5], bytes[off+6], bytes[off+7]]) as usize;
                    let color = u32::from_le_bytes([bytes[off+8], bytes[off+9], bytes[off+10], bytes[off+11]]);
                    let filled = bytes[off+12] != 0;
                    lines.push(controls::data_grid::ConnectorLine { start_row: start, end_row: end, color, filled });
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

fn as_text_editor(ctrl: &mut alloc::boxed::Box<dyn Control>) -> Option<&mut controls::text_editor::TextEditor> {
    if ctrl.kind() == ControlKind::TextEditor {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::text_editor::TextEditor) })
    } else {
        None
    }
}

fn as_text_editor_ref(ctrl: &alloc::boxed::Box<dyn Control>) -> Option<&controls::text_editor::TextEditor> {
    if ctrl.kind() == ControlKind::TextEditor {
        let raw: *const dyn Control = &**ctrl;
        Some(unsafe { &*(raw as *const controls::text_editor::TextEditor) })
    } else {
        None
    }
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
                unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buf, copy_len); }
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
            if !out_row.is_null() { unsafe { *out_row = r as u32; } }
            if !out_col.is_null() { unsafe { *out_col = c as u32; } }
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

// ── TreeView ──────────────────────────────────────────────────────────

fn as_tree_view(ctrl: &mut alloc::boxed::Box<dyn Control>) -> Option<&mut controls::tree_view::TreeView> {
    if ctrl.kind() == ControlKind::TreeView {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::tree_view::TreeView) })
    } else {
        None
    }
}

fn as_tree_view_ref(ctrl: &alloc::boxed::Box<dyn Control>) -> Option<&controls::tree_view::TreeView> {
    if ctrl.kind() == ControlKind::TreeView {
        let raw: *const dyn Control = &**ctrl;
        Some(unsafe { &*(raw as *const controls::tree_view::TreeView) })
    } else {
        None
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_add_node(id: ControlId, parent_index: u32, text: *const u8, text_len: u32) -> u32 {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            let parent = if parent_index == u32::MAX { None } else { Some(parent_index as usize) };
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
pub extern "C" fn anyui_treeview_set_node_text(id: ControlId, index: u32, text: *const u8, text_len: u32) {
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
pub extern "C" fn anyui_treeview_set_node_icon(id: ControlId, index: u32, pixels: *const u32, w: u32, h: u32) {
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
/// EVENT_MOUSE_UP=15, EVENT_MOUSE_MOVE=16
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
        ctrl.base_mut().tooltip_text = bytes;
    }
}

// ── MessageBox ───────────────────────────────────────────────────────

static mut MSGBOX_DISMISSED: bool = false;

extern "C" fn msgbox_ok_clicked(_id: u32, _event_type: u32, _userdata: u64) {
    unsafe { MSGBOX_DISMISSED = true; }
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
    if st.windows.is_empty() { return; }

    let win_id = st.windows[0];
    let (win_w, win_h) = {
        let ctrl = st.controls.iter().find(|c| c.id() == win_id);
        match ctrl {
            Some(c) => (c.base().w, c.base().h),
            None => return,
        }
    };

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
    let (icon_char, icon_color) = match msg_type {
        0 => (b"!" as &[u8], 0xFFFF3B30u32),  // alert — red
        1 => (b"i" as &[u8], 0xFF007AFFu32),   // info — blue
        _ => (b"!" as &[u8], 0xFFFFD60Au32),   // warning — yellow
    };

    let card_w = 320u32;
    let card_h = 160u32;
    let card_x = ((win_w as i32) - (card_w as i32)) / 2;
    let card_y = ((win_h as i32) - (card_h as i32)) / 2;

    // Allocate IDs
    let overlay_id = st.next_id; st.next_id += 1;
    let card_id = st.next_id; st.next_id += 1;
    let icon_id = st.next_id; st.next_id += 1;
    let msg_id = st.next_id; st.next_id += 1;
    let btn_id = st.next_id; st.next_id += 1;

    // Create overlay (full-window view, dark background)
    let mut overlay = controls::create_control(
        ControlKind::View, overlay_id, win_id, 0, 0, win_w, win_h, &[],
    );
    overlay.set_color(0xAA000000);
    st.controls.push(overlay);
    if let Some(w) = st.controls.iter_mut().find(|c| c.id() == win_id) {
        w.add_child(overlay_id);
    }

    // Create card
    let card = controls::create_control(
        ControlKind::Card, card_id, overlay_id, card_x, card_y, card_w, card_h, &[],
    );
    st.controls.push(card);
    if let Some(o) = st.controls.iter_mut().find(|c| c.id() == overlay_id) {
        o.add_child(card_id);
    }

    // Icon label
    let mut icon = controls::create_control(
        ControlKind::Label, icon_id, card_id, 20, 16, 24, 24, icon_char,
    );
    icon.set_color(icon_color);
    st.controls.push(icon);
    if let Some(c) = st.controls.iter_mut().find(|c| c.id() == card_id) {
        c.add_child(icon_id);
    }

    // Message label
    let msg = controls::create_control(
        ControlKind::Label, msg_id, card_id, 52, 16, card_w - 72, 80, text_slice,
    );
    st.controls.push(msg);
    if let Some(c) = st.controls.iter_mut().find(|c| c.id() == card_id) {
        c.add_child(msg_id);
    }

    // OK button
    let btn = controls::create_control(
        ControlKind::Button, btn_id, card_id,
        ((card_w as i32) - 80) / 2, (card_h as i32) - 48, 80, 32,
        btn_slice,
    );
    st.controls.push(btn);
    if let Some(c) = st.controls.iter_mut().find(|c| c.id() == card_id) {
        c.add_child(btn_id);
    }

    // Register click handler on the button
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == btn_id) {
        b.set_event_callback(control::EVENT_CLICK, msgbox_ok_clicked, 0);
    }

    // Mini event loop — block until dismissed
    unsafe { MSGBOX_DISMISSED = false; }
    while !unsafe { MSGBOX_DISMISSED } {
        let t0 = syscall::uptime_ms();
        if event_loop::run_once() == 0 { break; }
        let elapsed = syscall::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { syscall::sleep(16 - elapsed); }
    }

    // Clean up — remove overlay and all descendants
    anyui_remove(overlay_id);
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
pub extern "C" fn anyui_set_timer(
    interval_ms: u32,
    cb: control::Callback,
    userdata: u64,
) -> u32 {
    state().timers.set_timer(interval_ms, cb, userdata)
}

/// Remove a timer by ID. No-op if the timer ID is invalid.
#[no_mangle]
pub extern "C" fn anyui_kill_timer(timer_id: u32) {
    state().timers.kill_timer(timer_id);
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
        if st.focused == Some(rid) { st.focused = None; }
        if st.pressed == Some(rid) { st.pressed = None; }
        if st.hovered == Some(rid) { st.hovered = None; }
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

/// Programmatically resize a window (SHM buffer, back buffer, control size).
/// Used by the dock (and similar borderless windows) to react to resolution changes.
#[no_mangle]
pub extern "C" fn anyui_resize_window(win_id: ControlId, new_w: u32, new_h: u32) {
    let st = state();
    if let Some(wi) = st.windows.iter().position(|&w| w == win_id) {
        let cw = &mut st.comp_windows[wi];
        if cw.width == new_w && cw.height == new_h {
            return;
        }
        if let Some((new_shm_id, new_surface)) = compositor::resize_shm(
            st.channel_id, cw.window_id, cw.shm_id, new_w, new_h,
        ) {
            cw.shm_id = new_shm_id;
            cw.surface = new_surface;
        }
        cw.width = new_w;
        cw.height = new_h;
        let new_count = (new_w as usize) * (new_h as usize);
        cw.back_buffer.resize(new_count, 0);
        cw.dirty = true;
        cw.dirty_rect = None; // full redraw
    }
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == win_id) {
        ctrl.set_size(new_w, new_h);
    }
    mark_needs_layout();
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
        compositor::move_window(st.channel_id, comp_win_id, x, y);
    }
}

#[no_mangle]
pub extern "C" fn anyui_destroy_window(win_id: ControlId) {
    let st = state();

    if let Some(idx) = st.windows.iter().position(|&w| w == win_id) {
        let cw = &st.comp_windows[idx];
        compositor::destroy_window(st.channel_id, cw.window_id, cw.shm_id);
        st.comp_windows.remove(idx);
        st.windows.remove(idx);
    }

    anyui_remove(win_id);
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
        compositor::set_blur_behind(
            st.channel_id,
            st.comp_windows[idx].window_id,
            radius,
        );
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
    if !out_w.is_null() { unsafe { *out_w = w; } }
    if !out_h.is_null() { unsafe { *out_h = h; } }
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
    title_ptr: *const u8, title_len: u32,
    msg_ptr: *const u8, msg_len: u32,
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
    if !out_keycode.is_null() { unsafe { *out_keycode = st.last_keycode; } }
    if !out_char_code.is_null() { unsafe { *out_char_code = st.last_char_code; } }
    if !out_modifiers.is_null() { unsafe { *out_modifiers = st.last_modifiers; } }
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
        crate::log!("[anyui_clipboard_get] got {} bytes: '{}'", copy_len, preview);
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
        if !out_w.is_null() { unsafe { *out_w = ctrl.base().w; } }
        if !out_h.is_null() { unsafe { *out_h = ctrl.base().h; } }
    }
}

/// Get the position of a control. Returns via out pointers.
#[no_mangle]
pub extern "C" fn anyui_get_position(id: ControlId, out_x: *mut i32, out_y: *mut i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == id) {
        if !out_x.is_null() { unsafe { *out_x = ctrl.base().x; } }
        if !out_y.is_null() { unsafe { *out_y = ctrl.base().y; } }
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

// ── Focus by task ID ────────────────────────────────────────────────

/// Send CMD_FOCUS_BY_TID to the compositor to bring a window to the front.
#[no_mangle]
pub extern "C" fn anyui_focus_by_tid(tid: u32) {
    let channel_id = state().channel_id;
    if channel_id == 0 { return; }
    let cmd: [u32; 5] = [0x100A, tid, 0, 0, 0]; // CMD_FOCUS_BY_TID
    syscall::evt_chan_emit(channel_id, &cmd);
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
