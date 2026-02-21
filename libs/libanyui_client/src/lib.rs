//! libanyui_client — Windows Forms-style OO wrapper for libanyui.so.
//!
//! Provides typed control structs with proper inheritance hierarchy:
//!
//! - `Control` — base type with common properties (position, size, color, events)
//! - `Container` — extends Control, adds generic `add()` for child controls
//! - Typed leaf controls (Label, Button, Slider, etc.) — Deref to Control
//! - Typed container controls (View, Card, SplitView, etc.) — Deref to Container
//!
//! # Usage
//! ```rust
//! use libanyui_client as ui;
//!
//! ui::init();
//! let win = ui::Window::new("Demo", 400, 300);
//! let label = ui::Label::new("Hello World");
//! label.set_position(20, 20);
//! win.add(&label);
//! let slider = ui::Slider::new(50);
//! let progress = ui::ProgressBar::new(50);
//! slider.on_value_changed(|e| {
//!     ui::Control::from_id(e.id).set_state(e.value);
//! });
//! win.add(&slider);
//! win.add(&progress);
//! ui::run();
//! ```

#![no_std]

extern crate alloc;

mod events;
pub use events::*;

pub mod icon;
pub use icon::Icon;

use dynlink::{DlHandle, dl_open, dl_sym};

// ── Control kind constants (match libanyui's ControlKind enum) ───────

pub const KIND_WINDOW: u32 = 0;
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
pub const KIND_EXPANDER: u32 = 37;
pub const KIND_DATA_GRID: u32 = 38;
pub const KIND_TEXT_EDITOR: u32 = 39;
pub const KIND_TREE_VIEW: u32 = 40;

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

// ══════════════════════════════════════════════════════════════════════
//  Internal: cached function pointers from libanyui.so
// ══════════════════════════════════════════════════════════════════════

struct AnyuiLib {
    _handle: DlHandle,
    // Core
    init: extern "C" fn() -> u32,
    shutdown: extern "C" fn(),
    create_window: extern "C" fn(*const u8, u32, u32, u32) -> u32,
    add_control: extern "C" fn(u32, u32, i32, i32, u32, u32, *const u8, u32) -> u32,
    create_control: extern "C" fn(u32, *const u8, u32) -> u32,
    add_child: extern "C" fn(u32, u32),
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
    // SplitView properties
    set_split_ratio: extern "C" fn(u32, u32),
    set_min_split: extern "C" fn(u32, u32),
    set_max_split: extern "C" fn(u32, u32),
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
    // Context menu
    set_context_menu: extern "C" fn(u32, u32),
    // MessageBox
    message_box: extern "C" fn(u32, *const u8, u32, *const u8, u32),
    // ImageView
    imageview_set_pixels: extern "C" fn(u32, *const u32, u32, u32),
    imageview_set_scale_mode: extern "C" fn(u32, u32),
    imageview_get_image_size: extern "C" fn(u32, *mut u32, *mut u32) -> u32,
    imageview_clear: extern "C" fn(u32),
    // DataGrid
    datagrid_set_columns: extern "C" fn(u32, *const u8, u32),
    datagrid_get_column_count: extern "C" fn(u32) -> u32,
    datagrid_set_column_width: extern "C" fn(u32, u32, u32),
    datagrid_set_data: extern "C" fn(u32, *const u8, u32),
    datagrid_set_cell: extern "C" fn(u32, u32, u32, *const u8, u32),
    datagrid_get_cell: extern "C" fn(u32, u32, u32, *mut u8, u32) -> u32,
    datagrid_set_cell_colors: extern "C" fn(u32, *const u32, u32),
    datagrid_set_row_count: extern "C" fn(u32, u32),
    datagrid_get_row_count: extern "C" fn(u32) -> u32,
    datagrid_set_selection_mode: extern "C" fn(u32, u32),
    datagrid_get_selected_row: extern "C" fn(u32) -> u32,
    datagrid_set_selected_row: extern "C" fn(u32, u32),
    datagrid_is_row_selected: extern "C" fn(u32, u32) -> u32,
    datagrid_sort: extern "C" fn(u32, u32, u32),
    datagrid_set_row_height: extern "C" fn(u32, u32),
    datagrid_set_header_height: extern "C" fn(u32, u32),
    // TextEditor
    texteditor_set_text: extern "C" fn(u32, *const u8, u32),
    texteditor_get_text: extern "C" fn(u32, *mut u8, u32) -> u32,
    texteditor_set_syntax: extern "C" fn(u32, *const u8, u32),
    texteditor_set_cursor: extern "C" fn(u32, u32, u32),
    texteditor_get_cursor: extern "C" fn(u32, *mut u32, *mut u32),
    texteditor_set_line_height: extern "C" fn(u32, u32),
    texteditor_set_tab_width: extern "C" fn(u32, u32),
    texteditor_set_show_line_numbers: extern "C" fn(u32, u32),
    texteditor_set_font: extern "C" fn(u32, u32, u32),
    texteditor_insert_text: extern "C" fn(u32, *const u8, u32),
    texteditor_get_line_count: extern "C" fn(u32) -> u32,
    // TreeView
    treeview_add_node: extern "C" fn(u32, u32, *const u8, u32) -> u32,
    treeview_remove_node: extern "C" fn(u32, u32),
    treeview_set_node_text: extern "C" fn(u32, u32, *const u8, u32),
    treeview_set_node_icon: extern "C" fn(u32, u32, *const u32, u32, u32),
    treeview_set_node_style: extern "C" fn(u32, u32, u32),
    treeview_set_node_text_color: extern "C" fn(u32, u32, u32),
    treeview_set_expanded: extern "C" fn(u32, u32, u32),
    treeview_get_expanded: extern "C" fn(u32, u32) -> u32,
    treeview_get_selected: extern "C" fn(u32) -> u32,
    treeview_set_selected: extern "C" fn(u32, u32),
    treeview_clear: extern "C" fn(u32),
    treeview_get_node_count: extern "C" fn(u32) -> u32,
    treeview_set_indent_width: extern "C" fn(u32, u32),
    treeview_set_row_height: extern "C" fn(u32, u32),
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

// ══════════════════════════════════════════════════════════════════════
//  Public API — init / shutdown / run
// ══════════════════════════════════════════════════════════════════════

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
            create_control: resolve(&handle, "anyui_create_control"),
            add_child: resolve(&handle, "anyui_add_child"),
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
            // SplitView properties
            set_split_ratio: resolve(&handle, "anyui_set_split_ratio"),
            set_min_split: resolve(&handle, "anyui_set_min_split"),
            set_max_split: resolve(&handle, "anyui_set_max_split"),
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
            // Context menu
            set_context_menu: resolve(&handle, "anyui_set_context_menu"),
            // MessageBox
            message_box: resolve(&handle, "anyui_message_box"),
            // ImageView
            imageview_set_pixels: resolve(&handle, "anyui_imageview_set_pixels"),
            imageview_set_scale_mode: resolve(&handle, "anyui_imageview_set_scale_mode"),
            imageview_get_image_size: resolve(&handle, "anyui_imageview_get_image_size"),
            imageview_clear: resolve(&handle, "anyui_imageview_clear"),
            // DataGrid
            datagrid_set_columns: resolve(&handle, "anyui_datagrid_set_columns"),
            datagrid_get_column_count: resolve(&handle, "anyui_datagrid_get_column_count"),
            datagrid_set_column_width: resolve(&handle, "anyui_datagrid_set_column_width"),
            datagrid_set_data: resolve(&handle, "anyui_datagrid_set_data"),
            datagrid_set_cell: resolve(&handle, "anyui_datagrid_set_cell"),
            datagrid_get_cell: resolve(&handle, "anyui_datagrid_get_cell"),
            datagrid_set_cell_colors: resolve(&handle, "anyui_datagrid_set_cell_colors"),
            datagrid_set_row_count: resolve(&handle, "anyui_datagrid_set_row_count"),
            datagrid_get_row_count: resolve(&handle, "anyui_datagrid_get_row_count"),
            datagrid_set_selection_mode: resolve(&handle, "anyui_datagrid_set_selection_mode"),
            datagrid_get_selected_row: resolve(&handle, "anyui_datagrid_get_selected_row"),
            datagrid_set_selected_row: resolve(&handle, "anyui_datagrid_set_selected_row"),
            datagrid_is_row_selected: resolve(&handle, "anyui_datagrid_is_row_selected"),
            datagrid_sort: resolve(&handle, "anyui_datagrid_sort"),
            datagrid_set_row_height: resolve(&handle, "anyui_datagrid_set_row_height"),
            datagrid_set_header_height: resolve(&handle, "anyui_datagrid_set_header_height"),
            // TextEditor
            texteditor_set_text: resolve(&handle, "anyui_texteditor_set_text"),
            texteditor_get_text: resolve(&handle, "anyui_texteditor_get_text"),
            texteditor_set_syntax: resolve(&handle, "anyui_texteditor_set_syntax"),
            texteditor_set_cursor: resolve(&handle, "anyui_texteditor_set_cursor"),
            texteditor_get_cursor: resolve(&handle, "anyui_texteditor_get_cursor"),
            texteditor_set_line_height: resolve(&handle, "anyui_texteditor_set_line_height"),
            texteditor_set_tab_width: resolve(&handle, "anyui_texteditor_set_tab_width"),
            texteditor_set_show_line_numbers: resolve(&handle, "anyui_texteditor_set_show_line_numbers"),
            texteditor_set_font: resolve(&handle, "anyui_texteditor_set_font"),
            texteditor_insert_text: resolve(&handle, "anyui_texteditor_insert_text"),
            texteditor_get_line_count: resolve(&handle, "anyui_texteditor_get_line_count"),
            // TreeView
            treeview_add_node: resolve(&handle, "anyui_treeview_add_node"),
            treeview_remove_node: resolve(&handle, "anyui_treeview_remove_node"),
            treeview_set_node_text: resolve(&handle, "anyui_treeview_set_node_text"),
            treeview_set_node_icon: resolve(&handle, "anyui_treeview_set_node_icon"),
            treeview_set_node_style: resolve(&handle, "anyui_treeview_set_node_style"),
            treeview_set_node_text_color: resolve(&handle, "anyui_treeview_set_node_text_color"),
            treeview_set_expanded: resolve(&handle, "anyui_treeview_set_expanded"),
            treeview_get_expanded: resolve(&handle, "anyui_treeview_get_expanded"),
            treeview_get_selected: resolve(&handle, "anyui_treeview_get_selected"),
            treeview_set_selected: resolve(&handle, "anyui_treeview_set_selected"),
            treeview_clear: resolve(&handle, "anyui_treeview_clear"),
            treeview_get_node_count: resolve(&handle, "anyui_treeview_get_node_count"),
            treeview_set_indent_width: resolve(&handle, "anyui_treeview_set_indent_width"),
            treeview_set_row_height: resolve(&handle, "anyui_treeview_set_row_height"),
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

// ══════════════════════════════════════════════════════════════════════
//  Widget trait — implemented by all control types
// ══════════════════════════════════════════════════════════════════════

/// Base trait for all UI controls. Provides the control's unique ID.
pub trait Widget {
    fn id(&self) -> u32;
}

// ══════════════════════════════════════════════════════════════════════
//  Control — base type with common properties
// ══════════════════════════════════════════════════════════════════════

/// Base control handle. All typed controls Deref to this.
///
/// Provides common properties: position, size, visibility, color, state,
/// layout (padding, margin, dock), text styling, and event callbacks.
#[derive(Clone, Copy)]
pub struct Control {
    id: u32,
}

impl Widget for Control {
    fn id(&self) -> u32 { self.id }
}

impl Control {
    /// Wrap a raw control ID.
    pub fn from_id(id: u32) -> Self { Self { id } }

    // ── Position / Size ──

    pub fn set_position(&self, x: i32, y: i32) {
        (lib().set_position)(self.id, x, y);
    }

    pub fn set_size(&self, w: u32, h: u32) {
        (lib().set_size)(self.id, w, h);
    }

    // ── Visibility ──

    pub fn set_visible(&self, visible: bool) {
        (lib().set_visible)(self.id, visible as u32);
    }

    // ── Color ──

    pub fn set_color(&self, color: u32) {
        (lib().set_color)(self.id, color);
    }

    // ── Text ──

    pub fn set_text(&self, text: &str) {
        (lib().set_text)(self.id, text.as_ptr(), text.len() as u32);
    }

    pub fn get_text(&self, buf: &mut [u8]) -> u32 {
        (lib().get_text)(self.id, buf.as_mut_ptr(), buf.len() as u32)
    }

    // ── State (numeric value: slider position, toggle on/off, etc.) ──

    pub fn set_state(&self, value: u32) {
        (lib().set_state)(self.id, value);
    }

    pub fn get_state(&self) -> u32 {
        (lib().get_state)(self.id)
    }

    // ── Layout properties ──

    pub fn set_padding(&self, left: i32, top: i32, right: i32, bottom: i32) {
        (lib().set_padding)(self.id, left, top, right, bottom);
    }

    pub fn set_margin(&self, left: i32, top: i32, right: i32, bottom: i32) {
        (lib().set_margin)(self.id, left, top, right, bottom);
    }

    pub fn set_dock(&self, dock_style: u32) {
        (lib().set_dock)(self.id, dock_style);
    }

    pub fn set_auto_size(&self, enabled: bool) {
        (lib().set_auto_size)(self.id, enabled as u32);
    }

    pub fn set_min_size(&self, min_w: u32, min_h: u32) {
        (lib().set_min_size)(self.id, min_w, min_h);
    }

    pub fn set_max_size(&self, max_w: u32, max_h: u32) {
        (lib().set_max_size)(self.id, max_w, max_h);
    }

    // ── Text styling ──

    pub fn set_font_size(&self, size: u32) {
        (lib().set_font_size)(self.id, size);
    }

    pub fn get_font_size(&self) -> u32 {
        (lib().get_font_size)(self.id)
    }

    pub fn set_font(&self, font_id: u32) {
        (lib().set_font)(self.id, font_id);
    }

    pub fn set_text_color(&self, color: u32) {
        (lib().set_text_color)(self.id, color);
    }

    // ── Events / Callbacks (raw FFI) ──

    pub fn on_event_raw(&self, event_type: u32, cb: Callback, userdata: u64) {
        (lib().on_event_fn)(self.id, event_type, cb, userdata);
    }

    pub fn on_click_raw(&self, cb: Callback, userdata: u64) {
        (lib().on_click_fn)(self.id, cb, userdata);
    }

    pub fn on_change_raw(&self, cb: Callback, userdata: u64) {
        (lib().on_change_fn)(self.id, cb, userdata);
    }

    pub fn on_mouse_enter_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_MOUSE_ENTER, cb, userdata);
    }

    pub fn on_mouse_leave_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_MOUSE_LEAVE, cb, userdata);
    }

    pub fn on_double_click_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_DOUBLE_CLICK, cb, userdata);
    }

    pub fn on_focus_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_FOCUS, cb, userdata);
    }

    pub fn on_blur_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_BLUR, cb, userdata);
    }

    pub fn on_scroll_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_SCROLL, cb, userdata);
    }

    pub fn on_key_down_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_KEY, cb, userdata);
    }

    pub fn on_mouse_down_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_MOUSE_DOWN, cb, userdata);
    }

    pub fn on_mouse_up_raw(&self, cb: Callback, userdata: u64) {
        self.on_event_raw(EVENT_MOUSE_UP, cb, userdata);
    }

    // ── Context menu ──

    /// Attach a context menu to this control. Shown on right-click.
    pub fn set_context_menu(&self, menu: &impl Widget) {
        (lib().set_context_menu)(self.id, menu.id());
    }

    // ── Removal ──

    pub fn remove(&self) {
        (lib().remove_fn)(self.id);
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Container — extends Control, adds generic add()
// ══════════════════════════════════════════════════════════════════════

/// A control that can contain child controls.
///
/// Inherits all `Control` methods via Deref. Adds `add()` to attach children.
#[derive(Clone, Copy)]
pub struct Container {
    ctrl: Control,
}

impl Widget for Container {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl core::ops::Deref for Container {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Container {
    /// Attach a child widget to this container.
    pub fn add(&self, child: &impl Widget) {
        (lib().add_child)(self.ctrl.id, child.id());
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Macros for generating typed control structs
// ══════════════════════════════════════════════════════════════════════

/// Generate a leaf control (non-container). Derefs to Control.
macro_rules! leaf_control {
    ($name:ident, $kind:expr) => {
        #[derive(Clone, Copy)]
        pub struct $name { ctrl: Control }

        impl Widget for $name {
            fn id(&self) -> u32 { self.ctrl.id }
        }

        impl core::ops::Deref for $name {
            type Target = Control;
            fn deref(&self) -> &Control { &self.ctrl }
        }
    };
}

/// Generate a container control. Derefs to Container (which Derefs to Control).
macro_rules! container_control {
    ($name:ident, $kind:expr) => {
        #[derive(Clone, Copy)]
        pub struct $name { container: Container }

        impl Widget for $name {
            fn id(&self) -> u32 { self.container.ctrl.id }
        }

        impl core::ops::Deref for $name {
            type Target = Container;
            fn deref(&self) -> &Container { &self.container }
        }
    };
}

// ══════════════════════════════════════════════════════════════════════
//  Controls — each control type lives in its own file under controls/
// ══════════════════════════════════════════════════════════════════════

mod controls;
pub use controls::*;

// ══════════════════════════════════════════════════════════════════════
//  Marshal (cross-thread UI access)
// ══════════════════════════════════════════════════════════════════════

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
