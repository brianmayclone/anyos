//! libanyui_client — Ergonomic Rust wrapper for libanyui.so.
//!
//! Loads libanyui.so via dynlink (dlopen/dlsym) and provides a safe Rust API.
//! User programs depend on this crate, NOT on libanyui directly.
//!
//! # Usage
//! ```rust
//! use libanyui_client as ui;
//!
//! ui::init();
//! let win = ui::Window::new("Demo", 300, 200);
//! let btn = win.add_button("Click Me", 20, 60, 120, 32);
//! btn.on_click(my_callback, 0);
//! ui::run();
//! ```

#![no_std]

extern crate alloc;

use dynlink::{DlHandle, dl_open, dl_sym};

// ── Control kind constants (match libanyui's ControlKind enum) ───────

pub const KIND_VIEW: u32 = 1;
pub const KIND_LABEL: u32 = 2;
pub const KIND_BUTTON: u32 = 3;
pub const KIND_TEXTFIELD: u32 = 4;
pub const KIND_TOGGLE: u32 = 5;
pub const KIND_CHECKBOX: u32 = 6;
pub const KIND_SLIDER: u32 = 7;
pub const KIND_RADIO_BUTTON: u32 = 8;
pub const KIND_PROGRESS_BAR: u32 = 9;
pub const KIND_STEPPER: u32 = 10;
pub const KIND_SEGMENTED: u32 = 11;
pub const KIND_TABLE_VIEW: u32 = 12;
pub const KIND_SCROLL_VIEW: u32 = 13;
pub const KIND_SIDEBAR: u32 = 14;
pub const KIND_NAVIGATION_BAR: u32 = 15;
pub const KIND_TAB_BAR: u32 = 16;
pub const KIND_TOOLBAR: u32 = 17;
pub const KIND_CARD: u32 = 18;
pub const KIND_GROUP_BOX: u32 = 19;
pub const KIND_SPLIT_VIEW: u32 = 20;
pub const KIND_DIVIDER: u32 = 21;
pub const KIND_ALERT: u32 = 22;
pub const KIND_CONTEXT_MENU: u32 = 23;
pub const KIND_TOOLTIP: u32 = 24;
pub const KIND_IMAGE_VIEW: u32 = 25;
pub const KIND_STATUS_INDICATOR: u32 = 26;
pub const KIND_COLOR_WELL: u32 = 27;
pub const KIND_SEARCH_FIELD: u32 = 28;
pub const KIND_TEXT_AREA: u32 = 29;
pub const KIND_ICON_BUTTON: u32 = 30;
pub const KIND_BADGE: u32 = 31;
pub const KIND_TAG: u32 = 32;
pub const KIND_STACK_PANEL: u32 = 33;
pub const KIND_FLOW_PANEL: u32 = 34;
pub const KIND_TABLE_LAYOUT: u32 = 35;
pub const KIND_CANVAS: u32 = 36;

// ── DockStyle constants ─────────────────────────────────────────────

pub const DOCK_NONE: u32 = 0;
pub const DOCK_TOP: u32 = 1;
pub const DOCK_BOTTOM: u32 = 2;
pub const DOCK_LEFT: u32 = 3;
pub const DOCK_RIGHT: u32 = 4;
pub const DOCK_FILL: u32 = 5;

// ── Orientation constants ───────────────────────────────────────────

pub const ORIENTATION_VERTICAL: u32 = 0;
pub const ORIENTATION_HORIZONTAL: u32 = 1;

// ── Event type constants ────────────────────────────────────────────

pub const EVENT_CLICK: u32 = 1;
pub const EVENT_CHANGE: u32 = 2;
pub const EVENT_KEY: u32 = 3;
pub const EVENT_FOCUS: u32 = 4;
pub const EVENT_BLUR: u32 = 5;
pub const EVENT_CLOSE: u32 = 6;
pub const EVENT_RESIZE: u32 = 7;
pub const EVENT_SCROLL: u32 = 8;
pub const EVENT_DRAG: u32 = 9;
pub const EVENT_CONTEXT_MENU: u32 = 10;
pub const EVENT_DOUBLE_CLICK: u32 = 11;
pub const EVENT_MOUSE_ENTER: u32 = 12;
pub const EVENT_MOUSE_LEAVE: u32 = 13;
pub const EVENT_MOUSE_DOWN: u32 = 14;
pub const EVENT_MOUSE_UP: u32 = 15;
pub const EVENT_MOUSE_MOVE: u32 = 16;

/// Callback type: extern "C" fn(control_id: u32, event_type: u32, userdata: u64)
pub type Callback = extern "C" fn(u32, u32, u64);

// ── Internal: cached function pointers from libanyui.so ──────────────

struct AnyuiLib {
    _handle: DlHandle,
    // Core
    init: extern "C" fn() -> u32,
    shutdown: extern "C" fn(),
    create_window: extern "C" fn(*const u8, u32, u32, u32) -> u32,
    add_control: extern "C" fn(u32, u32, i32, i32, u32, u32, *const u8, u32) -> u32,
    set_text: extern "C" fn(u32, *const u8, u32),
    get_text: extern "C" fn(u32, *mut u8, u32) -> u32,
    set_position: extern "C" fn(u32, i32, i32),
    set_size: extern "C" fn(u32, u32, u32),
    set_visible: extern "C" fn(u32, u32),
    set_color: extern "C" fn(u32, u32),
    set_state: extern "C" fn(u32, u32),
    get_state: extern "C" fn(u32) -> u32,
    on_event_fn: extern "C" fn(u32, u32, Callback, u64),
    on_click_fn: extern "C" fn(u32, Callback, u64),
    on_change_fn: extern "C" fn(u32, Callback, u64),
    run_fn: extern "C" fn(),
    run_once_fn: extern "C" fn() -> u32,
    quit_fn: extern "C" fn(),
    remove_fn: extern "C" fn(u32),
    destroy_window: extern "C" fn(u32),
    // Layout
    set_padding: extern "C" fn(u32, i32, i32, i32, i32),
    set_margin: extern "C" fn(u32, i32, i32, i32, i32),
    set_dock: extern "C" fn(u32, u32),
    set_auto_size: extern "C" fn(u32, u32),
    set_min_size: extern "C" fn(u32, u32, u32),
    set_max_size: extern "C" fn(u32, u32, u32),
    // Text styling
    set_font_size: extern "C" fn(u32, u32),
    get_font_size: extern "C" fn(u32) -> u32,
    set_font: extern "C" fn(u32, u32),
    set_text_color: extern "C" fn(u32, u32),
    // Container properties
    set_orientation: extern "C" fn(u32, u32),
    set_columns: extern "C" fn(u32, u32),
    set_row_height: extern "C" fn(u32, u32),
    // Canvas
    canvas_set_pixel: extern "C" fn(u32, i32, i32, u32),
    canvas_clear: extern "C" fn(u32, u32),
    canvas_fill_rect: extern "C" fn(u32, i32, i32, u32, u32, u32),
    canvas_draw_line: extern "C" fn(u32, i32, i32, i32, i32, u32),
    canvas_draw_rect: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),
    canvas_draw_circle: extern "C" fn(u32, i32, i32, i32, u32),
    canvas_fill_circle: extern "C" fn(u32, i32, i32, i32, u32),
    canvas_get_buffer: extern "C" fn(u32) -> *mut u32,
    canvas_get_stride: extern "C" fn(u32) -> u32,
    // TextField-specific
    textfield_set_prefix: extern "C" fn(u32, u32),
    textfield_set_postfix: extern "C" fn(u32, u32),
    textfield_set_password: extern "C" fn(u32, u32),
    textfield_set_placeholder: extern "C" fn(u32, *const u8, u32),
    // Marshal (cross-thread)
    marshal_set_text: extern "C" fn(u32, *const u8, u32),
    marshal_set_color: extern "C" fn(u32, u32),
    marshal_set_state: extern "C" fn(u32, u32),
    marshal_set_visible: extern "C" fn(u32, u32),
    marshal_set_position: extern "C" fn(u32, i32, i32),
    marshal_set_size: extern "C" fn(u32, u32, u32),
    marshal_dispatch: extern "C" fn(extern "C" fn(u64), u64),
}

static mut LIB: Option<AnyuiLib> = None;

fn lib() -> &'static AnyuiLib {
    unsafe { LIB.as_ref().expect("libanyui not loaded") }
}

/// Resolve a function pointer from the loaded library, or panic.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name).expect("symbol not found in libanyui.so");
    core::mem::transmute_copy::<*const (), T>(&ptr)
}

// ── Public API ───────────────────────────────────────────────────────

/// Load and initialize libanyui.so. Call once at program start.
/// Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libanyui.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = AnyuiLib {
            // Core
            init: resolve(&handle, "anyui_init"),
            shutdown: resolve(&handle, "anyui_shutdown"),
            create_window: resolve(&handle, "anyui_create_window"),
            add_control: resolve(&handle, "anyui_add_control"),
            set_text: resolve(&handle, "anyui_set_text"),
            get_text: resolve(&handle, "anyui_get_text"),
            set_position: resolve(&handle, "anyui_set_position"),
            set_size: resolve(&handle, "anyui_set_size"),
            set_visible: resolve(&handle, "anyui_set_visible"),
            set_color: resolve(&handle, "anyui_set_color"),
            set_state: resolve(&handle, "anyui_set_state"),
            get_state: resolve(&handle, "anyui_get_state"),
            on_event_fn: resolve(&handle, "anyui_on_event"),
            on_click_fn: resolve(&handle, "anyui_on_click"),
            on_change_fn: resolve(&handle, "anyui_on_change"),
            run_fn: resolve(&handle, "anyui_run"),
            run_once_fn: resolve(&handle, "anyui_run_once"),
            quit_fn: resolve(&handle, "anyui_quit"),
            remove_fn: resolve(&handle, "anyui_remove"),
            destroy_window: resolve(&handle, "anyui_destroy_window"),
            // Layout
            set_padding: resolve(&handle, "anyui_set_padding"),
            set_margin: resolve(&handle, "anyui_set_margin"),
            set_dock: resolve(&handle, "anyui_set_dock"),
            set_auto_size: resolve(&handle, "anyui_set_auto_size"),
            set_min_size: resolve(&handle, "anyui_set_min_size"),
            set_max_size: resolve(&handle, "anyui_set_max_size"),
            // Text styling
            set_font_size: resolve(&handle, "anyui_set_font_size"),
            get_font_size: resolve(&handle, "anyui_get_font_size"),
            set_font: resolve(&handle, "anyui_set_font"),
            set_text_color: resolve(&handle, "anyui_set_text_color"),
            // Container properties
            set_orientation: resolve(&handle, "anyui_set_orientation"),
            set_columns: resolve(&handle, "anyui_set_columns"),
            set_row_height: resolve(&handle, "anyui_set_row_height"),
            // Canvas
            canvas_set_pixel: resolve(&handle, "anyui_canvas_set_pixel"),
            canvas_clear: resolve(&handle, "anyui_canvas_clear"),
            canvas_fill_rect: resolve(&handle, "anyui_canvas_fill_rect"),
            canvas_draw_line: resolve(&handle, "anyui_canvas_draw_line"),
            canvas_draw_rect: resolve(&handle, "anyui_canvas_draw_rect"),
            canvas_draw_circle: resolve(&handle, "anyui_canvas_draw_circle"),
            canvas_fill_circle: resolve(&handle, "anyui_canvas_fill_circle"),
            canvas_get_buffer: resolve(&handle, "anyui_canvas_get_buffer"),
            canvas_get_stride: resolve(&handle, "anyui_canvas_get_stride"),
            // TextField-specific
            textfield_set_prefix: resolve(&handle, "anyui_textfield_set_prefix"),
            textfield_set_postfix: resolve(&handle, "anyui_textfield_set_postfix"),
            textfield_set_password: resolve(&handle, "anyui_textfield_set_password"),
            textfield_set_placeholder: resolve(&handle, "anyui_textfield_set_placeholder"),
            // Marshal (cross-thread)
            marshal_set_text: resolve(&handle, "anyui_marshal_set_text"),
            marshal_set_color: resolve(&handle, "anyui_marshal_set_color"),
            marshal_set_state: resolve(&handle, "anyui_marshal_set_state"),
            marshal_set_visible: resolve(&handle, "anyui_marshal_set_visible"),
            marshal_set_position: resolve(&handle, "anyui_marshal_set_position"),
            marshal_set_size: resolve(&handle, "anyui_marshal_set_size"),
            marshal_dispatch: resolve(&handle, "anyui_marshal_dispatch"),
            _handle: handle,
        };
        (lib.init)();
        LIB = Some(lib);
    }

    true
}

/// Shut down libanyui and destroy all windows.
pub fn shutdown() {
    (lib().shutdown)();
}

/// Run the event loop. Blocks until all windows are closed or `quit()` is called.
pub fn run() {
    (lib().run_fn)();
}

/// Process one frame. Returns true if windows remain open.
pub fn run_once() -> bool {
    (lib().run_once_fn)() != 0
}

/// Signal the event loop to exit.
pub fn quit() {
    (lib().quit_fn)();
}

// ── Window ───────────────────────────────────────────────────────────

/// A top-level window.
pub struct Window(pub u32);

impl Window {
    /// Create a new window with the given title and size.
    pub fn new(title: &str, w: u32, h: u32) -> Self {
        let l = lib();
        Window((l.create_window)(title.as_ptr(), title.len() as u32, w, h))
    }

    /// Destroy this window.
    pub fn destroy(&self) {
        (lib().destroy_window)(self.0);
    }

    /// Add a child control of any kind. Returns a generic `Control` handle.
    pub fn add_control(
        &self,
        kind: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        text: &str,
    ) -> Control {
        let l = lib();
        Control((l.add_control)(self.0, kind, x, y, w, h, text.as_ptr(), text.len() as u32))
    }

    // ── Convenience methods for common controls ──

    pub fn add_label(&self, text: &str, x: i32, y: i32) -> Control {
        self.add_control(KIND_LABEL, x, y, 0, 0, text)
    }

    pub fn add_button(&self, text: &str, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_BUTTON, x, y, w, h, text)
    }

    pub fn add_textfield(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_TEXTFIELD, x, y, w, h, "")
    }

    pub fn add_toggle(&self, x: i32, y: i32, on: bool) -> Control {
        let ctrl = self.add_control(KIND_TOGGLE, x, y, 0, 0, "");
        if on {
            ctrl.set_state(1);
        }
        ctrl
    }

    pub fn add_checkbox(&self, label: &str, x: i32, y: i32) -> Control {
        self.add_control(KIND_CHECKBOX, x, y, 0, 0, label)
    }

    pub fn add_slider(&self, x: i32, y: i32, w: u32, value: u32) -> Control {
        let ctrl = self.add_control(KIND_SLIDER, x, y, w, 20, "");
        ctrl.set_state(value);
        ctrl
    }

    pub fn add_progress_bar(&self, x: i32, y: i32, w: u32, value: u32) -> Control {
        let ctrl = self.add_control(KIND_PROGRESS_BAR, x, y, w, 8, "");
        ctrl.set_state(value);
        ctrl
    }

    pub fn add_view(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_VIEW, x, y, w, h, "")
    }

    pub fn add_card(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_CARD, x, y, w, h, "")
    }

    pub fn add_divider(&self, x: i32, y: i32, w: u32) -> Control {
        self.add_control(KIND_DIVIDER, x, y, w, 1, "")
    }

    pub fn add_stack_panel(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_STACK_PANEL, x, y, w, h, "")
    }

    pub fn add_flow_panel(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_FLOW_PANEL, x, y, w, h, "")
    }

    pub fn add_table_layout(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_TABLE_LAYOUT, x, y, w, h, "")
    }

    pub fn add_canvas(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_CANVAS, x, y, w, h, "")
    }
}

// ── Generic Control handle ───────────────────────────────────────────

/// A handle to any control in the tree.
#[derive(Clone, Copy)]
pub struct Control(pub u32);

impl Control {
    /// Add a child control of any kind.
    pub fn add_control(
        &self,
        kind: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        text: &str,
    ) -> Control {
        let l = lib();
        Control((l.add_control)(self.0, kind, x, y, w, h, text.as_ptr(), text.len() as u32))
    }

    // ── Convenience methods for adding children ──

    pub fn add_label(&self, text: &str, x: i32, y: i32) -> Control {
        self.add_control(KIND_LABEL, x, y, 0, 0, text)
    }

    pub fn add_button(&self, text: &str, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_BUTTON, x, y, w, h, text)
    }

    pub fn add_textfield(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_TEXTFIELD, x, y, w, h, "")
    }

    pub fn add_stack_panel(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_STACK_PANEL, x, y, w, h, "")
    }

    pub fn add_flow_panel(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_FLOW_PANEL, x, y, w, h, "")
    }

    pub fn add_table_layout(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_TABLE_LAYOUT, x, y, w, h, "")
    }

    pub fn add_canvas(&self, x: i32, y: i32, w: u32, h: u32) -> Control {
        self.add_control(KIND_CANVAS, x, y, w, h, "")
    }

    // ── Core properties ──

    pub fn set_text(&self, text: &str) {
        (lib().set_text)(self.0, text.as_ptr(), text.len() as u32);
    }

    pub fn get_text(&self, buf: &mut [u8]) -> u32 {
        (lib().get_text)(self.0, buf.as_mut_ptr(), buf.len() as u32)
    }

    pub fn set_position(&self, x: i32, y: i32) {
        (lib().set_position)(self.0, x, y);
    }

    pub fn set_size(&self, w: u32, h: u32) {
        (lib().set_size)(self.0, w, h);
    }

    pub fn set_visible(&self, visible: bool) {
        (lib().set_visible)(self.0, visible as u32);
    }

    pub fn set_color(&self, color: u32) {
        (lib().set_color)(self.0, color);
    }

    pub fn set_state(&self, value: u32) {
        (lib().set_state)(self.0, value);
    }

    pub fn get_state(&self) -> u32 {
        (lib().get_state)(self.0)
    }

    // ── Layout properties ──

    pub fn set_padding(&self, left: i32, top: i32, right: i32, bottom: i32) {
        (lib().set_padding)(self.0, left, top, right, bottom);
    }

    pub fn set_margin(&self, left: i32, top: i32, right: i32, bottom: i32) {
        (lib().set_margin)(self.0, left, top, right, bottom);
    }

    pub fn set_dock(&self, dock_style: u32) {
        (lib().set_dock)(self.0, dock_style);
    }

    pub fn set_auto_size(&self, enabled: bool) {
        (lib().set_auto_size)(self.0, enabled as u32);
    }

    pub fn set_min_size(&self, min_w: u32, min_h: u32) {
        (lib().set_min_size)(self.0, min_w, min_h);
    }

    pub fn set_max_size(&self, max_w: u32, max_h: u32) {
        (lib().set_max_size)(self.0, max_w, max_h);
    }

    // ── Text styling ──

    pub fn set_font_size(&self, size: u32) {
        (lib().set_font_size)(self.0, size);
    }

    pub fn get_font_size(&self) -> u32 {
        (lib().get_font_size)(self.0)
    }

    pub fn set_font(&self, font_id: u32) {
        (lib().set_font)(self.0, font_id);
    }

    pub fn set_text_color(&self, color: u32) {
        (lib().set_text_color)(self.0, color);
    }

    // ── Container properties (StackPanel / TableLayout) ──

    pub fn set_orientation(&self, orientation: u32) {
        (lib().set_orientation)(self.0, orientation);
    }

    pub fn set_columns(&self, columns: u32) {
        (lib().set_columns)(self.0, columns);
    }

    pub fn set_row_height(&self, row_height: u32) {
        (lib().set_row_height)(self.0, row_height);
    }

    // ── Canvas operations ──

    pub fn canvas_set_pixel(&self, x: i32, y: i32, color: u32) {
        (lib().canvas_set_pixel)(self.0, x, y, color);
    }

    pub fn canvas_clear(&self, color: u32) {
        (lib().canvas_clear)(self.0, color);
    }

    pub fn canvas_fill_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        (lib().canvas_fill_rect)(self.0, x, y, w, h, color);
    }

    pub fn canvas_draw_line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        (lib().canvas_draw_line)(self.0, x0, y0, x1, y1, color);
    }

    pub fn canvas_draw_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32) {
        (lib().canvas_draw_rect)(self.0, x, y, w, h, color, thickness);
    }

    pub fn canvas_draw_circle(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        (lib().canvas_draw_circle)(self.0, cx, cy, radius, color);
    }

    pub fn canvas_fill_circle(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        (lib().canvas_fill_circle)(self.0, cx, cy, radius, color);
    }

    pub fn canvas_get_buffer(&self) -> *mut u32 {
        (lib().canvas_get_buffer)(self.0)
    }

    pub fn canvas_get_stride(&self) -> u32 {
        (lib().canvas_get_stride)(self.0)
    }

    // ── Callbacks ──

    /// Register a callback for a specific event type on a control.
    pub fn on_event(&self, event_type: u32, cb: Callback, userdata: u64) {
        (lib().on_event_fn)(self.0, event_type, cb, userdata);
    }

    pub fn on_click(&self, cb: Callback, userdata: u64) {
        (lib().on_click_fn)(self.0, cb, userdata);
    }

    pub fn on_change(&self, cb: Callback, userdata: u64) {
        (lib().on_change_fn)(self.0, cb, userdata);
    }

    pub fn on_mouse_enter(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_ENTER, cb, userdata);
    }

    pub fn on_mouse_leave(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_LEAVE, cb, userdata);
    }

    pub fn on_double_click(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_DOUBLE_CLICK, cb, userdata);
    }

    pub fn on_focus(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_FOCUS, cb, userdata);
    }

    pub fn on_blur(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_BLUR, cb, userdata);
    }

    pub fn on_scroll(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_SCROLL, cb, userdata);
    }

    pub fn on_key_down(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_KEY, cb, userdata);
    }

    pub fn on_mouse_down(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_DOWN, cb, userdata);
    }

    pub fn on_mouse_up(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_UP, cb, userdata);
    }

    // ── TextField-specific ──

    pub fn textfield_set_prefix(&self, icon_code: u32) {
        (lib().textfield_set_prefix)(self.0, icon_code);
    }

    pub fn textfield_set_postfix(&self, icon_code: u32) {
        (lib().textfield_set_postfix)(self.0, icon_code);
    }

    pub fn textfield_set_password(&self, enabled: bool) {
        (lib().textfield_set_password)(self.0, enabled as u32);
    }

    pub fn textfield_set_placeholder(&self, text: &str) {
        (lib().textfield_set_placeholder)(self.0, text.as_ptr(), text.len() as u32);
    }

    pub fn remove(&self) {
        (lib().remove_fn)(self.0);
    }
}

// ── Marshal (cross-thread UI access) ─────────────────────────────────

/// Set a control's text from a worker thread (deferred to UI thread).
pub fn marshal_set_text(id: u32, text: &str) {
    (lib().marshal_set_text)(id, text.as_ptr(), text.len() as u32);
}

/// Set a control's color from a worker thread.
pub fn marshal_set_color(id: u32, color: u32) {
    (lib().marshal_set_color)(id, color);
}

/// Set a control's state from a worker thread.
pub fn marshal_set_state(id: u32, value: u32) {
    (lib().marshal_set_state)(id, value);
}

/// Set a control's visibility from a worker thread.
pub fn marshal_set_visible(id: u32, visible: bool) {
    (lib().marshal_set_visible)(id, visible as u32);
}

/// Set a control's position from a worker thread.
pub fn marshal_set_position(id: u32, x: i32, y: i32) {
    (lib().marshal_set_position)(id, x, y);
}

/// Set a control's size from a worker thread.
pub fn marshal_set_size(id: u32, w: u32, h: u32) {
    (lib().marshal_set_size)(id, w, h);
}

/// Dispatch a callback to be executed on the UI thread.
pub fn marshal_dispatch(cb: extern "C" fn(u64), userdata: u64) {
    (lib().marshal_dispatch)(cb, userdata);
}
