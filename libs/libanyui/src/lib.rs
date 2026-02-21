//! libanyui — Object-oriented UI framework for anyOS.
//!
//! All widgets derive from the `Control` trait (virtual base class), which
//! provides common properties and virtual methods. Concrete types
//! (Label, Button, TextField, etc.) live in `controls/`, each in its own file.
//!
//! # Architecture
//! - Global `AnyuiState` owns all controls as `Vec<Box<dyn Control>>`.
//! - Each control has an ID (`ControlId`), parent, children, and a `ControlKind`.
//! - The event loop polls compositor events, dispatches via hit-testing and
//!   virtual method calls, and invokes registered callbacks.
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

mod control;
mod controls;
mod event_loop;
mod syscall;
pub mod uisys;

use alloc::boxed::Box;
use alloc::vec::Vec;
use control::{Control, ControlId, ControlKind, Callback};

// ── Global state (per-process, lives in .data/.bss of the .so) ───────

pub(crate) struct AnyuiState {
    pub controls: Vec<Box<dyn Control>>,
    pub next_id: ControlId,
    /// Top-level window ControlIds.
    pub windows: Vec<ControlId>,
    /// Compositor window handles, parallel to `windows`.
    pub comp_wins: Vec<u32>,
    /// Set to true when anyui_quit() is called.
    pub quit_requested: bool,

    // ── Event tracking ──────────────────────────────────────────────
    /// Currently focused control (receives keyboard events).
    pub focused: Option<ControlId>,
    /// Currently pressed control (mouse button held down).
    pub pressed: Option<ControlId>,
    /// Currently hovered control (mouse cursor is over).
    pub hovered: Option<ControlId>,
    /// Control ID of the last click (for double-click detection).
    pub last_click_id: Option<ControlId>,
    /// Tick count of the last click (for double-click detection).
    pub last_click_tick: u64,
}

static mut STATE: Option<AnyuiState> = None;

pub(crate) fn state() -> &'static mut AnyuiState {
    unsafe { STATE.as_mut().expect("anyui not initialized") }
}

// ── Allocator (bump allocator via sbrk) ──────────────────────────────

mod allocator {
    use core::alloc::{GlobalAlloc, Layout};

    struct BumpAllocator;

    unsafe impl GlobalAlloc for BumpAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let align = layout.align();
            let size = layout.size();

            let brk = crate::syscall::sbrk(0) as usize;
            let aligned = (brk + align - 1) & !(align - 1);
            let needed = aligned - brk + size;

            let new_brk = crate::syscall::sbrk(needed as u32) as usize;
            if new_brk == 0 || new_brk == usize::MAX {
                return core::ptr::null_mut();
            }
            aligned as *mut u8
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // Bump allocator never frees
        }
    }

    #[global_allocator]
    static ALLOCATOR: BumpAllocator = BumpAllocator;
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
/// Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_init() -> u32 {
    unsafe {
        STATE = Some(AnyuiState {
            controls: Vec::new(),
            next_id: 1,
            windows: Vec::new(),
            comp_wins: Vec::new(),
            quit_requested: false,
            focused: None,
            pressed: None,
            hovered: None,
            last_click_id: None,
            last_click_tick: 0,
        });
    }
    1
}

/// Shut down the anyui framework and destroy all windows.
#[no_mangle]
pub extern "C" fn anyui_shutdown() {
    let st = state();
    for &win in &st.comp_wins {
        syscall::win_destroy(win);
    }
    unsafe { STATE = None; }
}

// ── Control creation ─────────────────────────────────────────────────

/// Create a top-level window. Returns a ControlId.
#[no_mangle]
pub extern "C" fn anyui_create_window(
    title: *const u8,
    title_len: u32,
    w: u32,
    h: u32,
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

    // Create compositor window
    let comp_win = syscall::win_create(&title_buf[..len], 100, 100, w, h);

    let ctrl = controls::create_control(ControlKind::Window, id, 0, 0, 0, w, h, &title_buf[..len]);
    st.controls.push(ctrl);
    st.windows.push(id);
    st.comp_wins.push(comp_win);
    id
}

/// Add a control as a child of `parent`. Returns the new ControlId.
///
/// `kind` selects the control type (see `ControlKind` values 0-32).
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
}

#[no_mangle]
pub extern "C" fn anyui_set_visible(id: ControlId, visible: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_visible(visible != 0);
    }
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

#[no_mangle]
pub extern "C" fn anyui_destroy_window(win_id: ControlId) {
    let st = state();

    if let Some(idx) = st.windows.iter().position(|&w| w == win_id) {
        let comp_win = st.comp_wins[idx];
        syscall::win_destroy(comp_win);
        st.windows.remove(idx);
        st.comp_wins.remove(idx);
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
