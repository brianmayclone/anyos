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

mod compositor;
mod control;
mod controls;
pub mod draw;
mod event_loop;
pub mod font_bitmap;
mod layout;
mod marshal;
mod syscall;
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
    /// Control ID of the last click (for double-click detection).
    pub last_click_id: Option<ControlId>,
    /// Tick count of the last click (for double-click detection).
    pub last_click_tick: u64,
    /// Which mouse button was pressed (for right-click detection).
    pub pressed_button: u32,

    // ── Timers ───────────────────────────────────────────────────────
    pub timers: timer::TimerState,
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
/// Connects to the compositor via libcompositor.dlib. Returns 1 on success.
#[no_mangle]
pub extern "C" fn anyui_init() -> u32 {
    let mut sub_id: u32 = 0;
    let channel_id = compositor::init(&mut sub_id);
    if channel_id == 0 {
        return 0;
    }

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
            pressed_button: 0,
            timers: timer::TimerState::new(),
        });
    }
    1
}

/// Shut down the anyui framework and destroy all windows.
#[no_mangle]
pub extern "C" fn anyui_shutdown() {
    let st = state();
    let channel_id = st.channel_id;
    for cw in &st.comp_windows {
        compositor::destroy_window(channel_id, cw.window_id, cw.shm_id);
    }
    unsafe { STATE = None; }
}

// ── Control creation ─────────────────────────────────────────────────

/// Create a top-level window at position (x, y). Returns a ControlId (0 on failure).
/// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
#[no_mangle]
pub extern "C" fn anyui_create_window(
    title: *const u8,
    title_len: u32,
    x: i32,
    y: i32,
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

    // Create compositor window via DLL
    let (window_id, shm_id, surface) =
        match compositor::create_window(st.channel_id, st.sub_id, x, y, w, h, 0) {
            Some(result) => result,
            None => return 0,
        };

    // Set title
    compositor::set_title(st.channel_id, window_id, &title_buf[..len]);

    let ctrl = controls::create_control(ControlKind::Window, id, 0, 0, 0, w, h, &title_buf[..len]);
    st.controls.push(ctrl);
    st.windows.push(id);
    st.comp_windows.push(CompWindow {
        window_id,
        shm_id,
        surface,
        width: w,
        height: h,
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
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent) {
        p.add_child(child);
    }
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

// ── Layout properties ────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn anyui_set_padding(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().padding = control::Padding { left, top, right, bottom };
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_margin(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().margin = control::Margin { left, top, right, bottom };
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_dock(id: ControlId, dock_style: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().dock = DockStyle::from_u32(dock_style);
    }
}

#[no_mangle]
pub extern "C" fn anyui_set_auto_size(id: ControlId, enabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().auto_size = enabled != 0;
    }
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
        if let Some(tb) = ctrl.text_base_mut() {
            tb.text_style.font_size = size as u16;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_get_font_size(id: ControlId) -> u32 {
    let st = state();
    st.controls.iter().find(|c| c.id() == id)
        .and_then(|c| c.text_base())
        .map_or(14, |tb| tb.text_style.font_size as u32)
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
                sv.orientation = Orientation::from_u32(orientation);
                sv.sync_divider();
                sv.base.dirty = true;
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
            sv.split_ratio = r;
            sv.sync_divider();
            sv.base.state = r;
            sv.base.dirty = true;
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
            tf.prefix_icon = if icon_code == 0 { None } else { Some(icon_code) };
            tf.text_base.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_postfix(id: ControlId, icon_code: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.postfix_icon = if icon_code == 0 { None } else { Some(icon_code) };
            tf.text_base.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_password(id: ControlId, enabled: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.password_mode = enabled != 0;
            tf.text_base.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_textfield_set_placeholder(id: ControlId, text: *const u8, len: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tf) = as_textfield(ctrl) {
            tf.placeholder.clear();
            if !text.is_null() && len > 0 {
                let slice = unsafe { core::slice::from_raw_parts(text, len as usize) };
                tf.placeholder.extend_from_slice(slice);
            }
            tf.text_base.base.dirty = true;
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
            iv.scale_mode = mode;
            iv.base.dirty = true;
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
            dg.base.dirty = true;
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
            dg.row_height = height.max(16);
            dg.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_datagrid_set_header_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(dg) = as_data_grid(ctrl) {
            dg.header_height = height.max(16);
            dg.base.dirty = true;
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
            te.line_height = height.max(12);
            te.base.dirty = true;
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
            te.show_line_numbers = show != 0;
            te.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_texteditor_set_font(id: ControlId, font_id: u32, font_size: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(te) = as_text_editor(ctrl) {
            te.font_id = font_id as u16;
            te.font_size = font_size as u16;
            let (cw, _) = crate::draw::measure_text_ex(b"M", te.font_id, te.font_size);
            te.char_width = if cw > 0 { cw } else { 8 };
            te.base.dirty = true;
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
            tv.indent_width = width.max(8);
            tv.base.dirty = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn anyui_treeview_set_row_height(id: ControlId, height: u32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        if let Some(tv) = as_tree_view(ctrl) {
            tv.row_height = height.max(16);
            tv.base.dirty = true;
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
pub extern "C" fn anyui_set_context_menu(id: ControlId, menu_id: ControlId) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().context_menu = Some(menu_id);
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
