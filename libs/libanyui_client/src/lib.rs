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

    /// Register a callback for a specific event type.
    pub fn on_event(&self, event_type: u32, cb: Callback, userdata: u64) {
        (lib().on_event_fn)(self.0, event_type, cb, userdata);
    }

    /// Register a click callback (convenience).
    pub fn on_click(&self, cb: Callback, userdata: u64) {
        (lib().on_click_fn)(self.0, cb, userdata);
    }

    /// Register a change callback (convenience).
    pub fn on_change(&self, cb: Callback, userdata: u64) {
        (lib().on_change_fn)(self.0, cb, userdata);
    }

    /// Register a mouse enter callback.
    pub fn on_mouse_enter(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_ENTER, cb, userdata);
    }

    /// Register a mouse leave callback.
    pub fn on_mouse_leave(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_LEAVE, cb, userdata);
    }

    /// Register a double-click callback.
    pub fn on_double_click(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_DOUBLE_CLICK, cb, userdata);
    }

    /// Register a focus callback.
    pub fn on_focus(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_FOCUS, cb, userdata);
    }

    /// Register a blur callback.
    pub fn on_blur(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_BLUR, cb, userdata);
    }

    /// Register a scroll callback.
    pub fn on_scroll(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_SCROLL, cb, userdata);
    }

    /// Register a key down callback.
    pub fn on_key_down(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_KEY, cb, userdata);
    }

    /// Register a mouse down callback.
    pub fn on_mouse_down(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_DOWN, cb, userdata);
    }

    /// Register a mouse up callback.
    pub fn on_mouse_up(&self, cb: Callback, userdata: u64) {
        self.on_event(EVENT_MOUSE_UP, cb, userdata);
    }

    pub fn remove(&self) {
        (lib().remove_fn)(self.0);
    }
}
