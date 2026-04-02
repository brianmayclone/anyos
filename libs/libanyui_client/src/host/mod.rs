//! Host-mode stub for libanyui_client.
//!
//! Provides minimal implementations of the UI controls that libwebview uses.
//! Canvas stores pixels in a Vec<u32>, other controls are no-ops.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

// ── ID generation ────────────────────────────────────────────────────────────

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
fn next_id() -> u32 { NEXT_ID.fetch_add(1, Ordering::Relaxed) }

// ── Callback type ────────────────────────────────────────────────────────────

pub type Callback = extern "C" fn(u32, u32, u64);

// ── Widget trait ─────────────────────────────────────────────────────────────

pub trait Widget {
    fn id(&self) -> u32;
}

// ── Constants ────────────────────────────────────────────────────────────────

pub const DOCK_NONE: u32 = 0;
pub const DOCK_TOP: u32 = 1;
pub const DOCK_BOTTOM: u32 = 2;
pub const DOCK_LEFT: u32 = 3;
pub const DOCK_RIGHT: u32 = 4;
pub const DOCK_FILL: u32 = 5;

pub const EVENT_CLICK: u32 = 1;
pub const EVENT_CHANGE: u32 = 2;
pub const EVENT_KEY: u32 = 3;
pub const EVENT_SCROLL: u32 = 8;
pub const EVENT_SUBMIT: u32 = 17;
pub const EVENT_MOUSE_DOWN: u32 = 14;
pub const EVENT_MOUSE_UP: u32 = 15;
pub const EVENT_MOUSE_MOVE: u32 = 16;

pub const KEY_ENTER: u32 = 0x100;
pub const KEY_BACKSPACE: u32 = 0x101;
pub const KEY_TAB: u32 = 0x102;
pub const KEY_ESCAPE: u32 = 0x103;
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_ALT: u32 = 4;

// ── Canvas storage (global) ──────────────────────────────────────────────────

struct CanvasData {
    id: u32,
    width: u32,
    height: u32,
    pos_x: i32,
    pos_y: i32,
    pixels: Vec<u32>,
}

static mut CANVASES: Vec<CanvasData> = Vec::new();

fn find_canvas(id: u32) -> Option<&'static mut CanvasData> {
    unsafe { CANVASES.iter_mut().find(|c| c.id == id) }
}

// ── Control text storage (global) ────────────────────────────────────────────
// Allows host-mode form controls (TextField, TextArea) to store/retrieve text.

struct ControlText {
    id: u32,
    text: Vec<u8>,
    state: u32, // for checkboxes / radio buttons
}

static mut CONTROL_TEXTS: Vec<ControlText> = Vec::new();

fn get_control_text(id: u32) -> Option<&'static ControlText> {
    unsafe { CONTROL_TEXTS.iter().find(|c| c.id == id) }
}

fn get_control_text_mut(id: u32) -> &'static mut ControlText {
    unsafe {
        if let Some(pos) = CONTROL_TEXTS.iter().position(|c| c.id == id) {
            return &mut CONTROL_TEXTS[pos];
        }
        CONTROL_TEXTS.push(ControlText { id, text: Vec::new(), state: 0 });
        let last = CONTROL_TEXTS.len() - 1;
        &mut CONTROL_TEXTS[last]
    }
}

// ── Control ──────────────────────────────────────────────────────────────────

pub struct Control {
    id: u32,
}

impl Widget for Control {
    fn id(&self) -> u32 { self.id }
}

impl Control {
    pub fn from_id(id: u32) -> Self { Control { id } }
    pub fn set_position(&self, x: i32, y: i32) {
        // Store position for canvases (needed for tile compositing)
        if let Some(c) = find_canvas(self.id) {
            c.pos_x = x;
            c.pos_y = y;
        }
    }
    pub fn set_size(&self, _w: u32, _h: u32) {}
    pub fn get_size(&self) -> (u32, u32) { (0, 0) }
    pub fn get_position(&self) -> (i32, i32) { (0, 0) }
    pub fn set_visible(&self, _visible: bool) {}
    pub fn set_color(&self, _color: u32) {}
    pub fn set_text(&self, text: &str) {
        let entry = get_control_text_mut(self.id);
        entry.text.clear();
        entry.text.extend_from_slice(text.as_bytes());
    }
    pub fn get_text(&self, buf: &mut [u8]) -> u32 {
        if let Some(entry) = get_control_text(self.id) {
            let len = entry.text.len().min(buf.len());
            buf[..len].copy_from_slice(&entry.text[..len]);
            len as u32
        } else {
            0
        }
    }
    pub fn set_state(&self, value: u32) {
        get_control_text_mut(self.id).state = value;
    }
    pub fn get_state(&self) -> u32 {
        get_control_text(self.id).map(|e| e.state).unwrap_or(0)
    }
    pub fn add_child(&self, _child_id: u32) {}
    pub fn set_padding(&self, _l: i32, _t: i32, _r: i32, _b: i32) {}
    pub fn set_margin(&self, _l: i32, _t: i32, _r: i32, _b: i32) {}
    pub fn set_dock(&self, _dock: u32) {}
    pub fn set_enabled(&self, _enabled: bool) {}
    pub fn set_auto_size(&self, _enabled: bool) {}
    pub fn set_min_size(&self, _w: u32, _h: u32) {}
    pub fn set_max_size(&self, _w: u32, _h: u32) {}
    pub fn set_font_size(&self, _size: u32) {}
    pub fn get_font_size(&self) -> u32 { 14 }
    pub fn set_font(&self, _font_id: u32) {}
    pub fn set_text_color(&self, _color: u32) {}
    pub fn set_tooltip(&self, _text: &str) {}
    pub fn set_tab_index(&self, _index: u32) {}
    pub fn focus(&self) {}
    pub fn bring_to_front(&self) {}
    pub fn remove(&self) {}
    pub fn on_event_raw(&self, _event_type: u32, _cb: Callback, _userdata: u64) {}
    pub fn on_click_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_change_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_submit_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_key_down_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_mouse_down_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_mouse_up_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn on_scroll_raw(&self, _cb: Callback, _userdata: u64) {}
    pub fn set_password_mode(&self, _enabled: bool) {}
}

// ── Container ────────────────────────────────────────────────────────────────

pub struct Container {
    ctrl: Control,
}

impl core::ops::Deref for Container {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Widget for Container {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl Container {
    pub fn add(&self, _child: &impl Widget) {}
    pub fn remove_child(&self, _child: &impl Widget) {}
    pub fn clear(&self) {}
}

// ── View ─────────────────────────────────────────────────────────────────────

pub struct View {
    container: Container,
}

impl core::ops::Deref for View {
    type Target = Container;
    fn deref(&self) -> &Container { &self.container }
}

impl Widget for View {
    fn id(&self) -> u32 { self.container.ctrl.id }
}

impl View {
    pub fn new() -> Self {
        View { container: Container { ctrl: Control { id: next_id() } } }
    }
}

// ── ScrollView ───────────────────────────────────────────────────────────────

pub struct ScrollView {
    container: Container,
}

impl core::ops::Deref for ScrollView {
    type Target = Container;
    fn deref(&self) -> &Container { &self.container }
}

impl Widget for ScrollView {
    fn id(&self) -> u32 { self.container.ctrl.id }
}

impl ScrollView {
    pub fn new() -> Self {
        ScrollView { container: Container { ctrl: Control { id: next_id() } } }
    }
    pub fn on_scroll(&self, _f: impl FnMut(&ScrollChangedEvent) + 'static) {}
}

// ── Canvas ───────────────────────────────────────────────────────────────────

pub struct Canvas {
    id: u32,
}

impl Widget for Canvas {
    fn id(&self) -> u32 { self.id }
}

impl core::ops::Deref for Canvas {
    type Target = Control;
    fn deref(&self) -> &Control {
        // Safety: Control is a zero-cost wrapper around u32
        unsafe { &*((&self.id) as *const u32 as *const Control) }
    }
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        let id = next_id();
        let pixels = vec![0u32; (w * h) as usize];
        unsafe {
            CANVASES.push(CanvasData { id, width: w, height: h, pos_x: 0, pos_y: 0, pixels });
        }
        Canvas { id }
    }

    pub fn get_buffer(&self) -> *mut u32 {
        if let Some(c) = find_canvas(self.id) {
            c.pixels.as_mut_ptr()
        } else {
            core::ptr::null_mut()
        }
    }

    pub fn get_stride(&self) -> u32 {
        if let Some(c) = find_canvas(self.id) { c.width } else { 0 }
    }

    pub fn get_height(&self) -> u32 {
        if let Some(c) = find_canvas(self.id) { c.height } else { 0 }
    }

    pub fn clear(&self, color: u32) {
        if let Some(c) = find_canvas(self.id) {
            c.pixels.fill(color);
        }
    }

    pub fn set_pixel(&self, x: i32, y: i32, color: u32) {
        if let Some(c) = find_canvas(self.id) {
            if x >= 0 && y >= 0 && (x as u32) < c.width && (y as u32) < c.height {
                c.pixels[(y as u32 * c.width + x as u32) as usize] = color;
            }
        }
    }

    pub fn get_pixel(&self, x: i32, y: i32) -> u32 {
        if let Some(c) = find_canvas(self.id) {
            if x >= 0 && y >= 0 && (x as u32) < c.width && (y as u32) < c.height {
                return c.pixels[(y as u32 * c.width + x as u32) as usize];
            }
        }
        0
    }

    pub fn copy_pixels_from(&self, src: &[u32]) {
        if let Some(c) = find_canvas(self.id) {
            let len = src.len().min(c.pixels.len());
            c.pixels[..len].copy_from_slice(&src[..len]);
        }
    }

    pub fn copy_pixels_to(&self, dst: &mut [u32]) -> usize {
        if let Some(c) = find_canvas(self.id) {
            let len = dst.len().min(c.pixels.len());
            dst[..len].copy_from_slice(&c.pixels[..len]);
            len
        } else {
            0
        }
    }

    pub fn fill_rect(&self, _x: i32, _y: i32, _w: u32, _h: u32, _color: u32) {}
    pub fn draw_text(&self, _x: i32, _y: i32, _color: u32, _font_id: u32, _size: u16, _text: &str) {}
    pub fn get_mouse(&self) -> (i32, i32, u32) { (0, 0, 0) }
    pub fn set_interactive(&self, _enabled: bool) {}
    pub fn on_click(&self, _f: impl FnMut(&ClickEvent) + 'static) {}
    pub fn on_mouse_down(&self, _f: impl FnMut(i32, i32, u32) + 'static) {}
    pub fn on_mouse_up(&self, _f: impl FnMut(i32, i32, u32) + 'static) {}
    pub fn on_mouse_move(&self, _f: impl FnMut(i32, i32) + 'static) {}
}

/// Get the pixels of a canvas by ID (host-only helper for surf-host).
/// Returns (pixels, width, height, pos_x, pos_y).
pub fn host_get_canvas_pixels(canvas_id: u32) -> Option<(&'static [u32], u32, u32, i32, i32)> {
    find_canvas(canvas_id).map(|c| (c.pixels.as_slice(), c.width, c.height, c.pos_x, c.pos_y))
}

// ── Window ───────────────────────────────────────────────────────────────────

pub struct Window {
    container: Container,
}

impl core::ops::Deref for Window {
    type Target = Container;
    fn deref(&self) -> &Container { &self.container }
}

impl Widget for Window {
    fn id(&self) -> u32 { self.container.ctrl.id }
}

impl Window {
    pub fn new(_title: &str, _x: i32, _y: i32, _w: u32, _h: u32) -> Self {
        Window { container: Container { ctrl: Control { id: next_id() } } }
    }
    pub fn new_with_flags(_title: &str, _x: i32, _y: i32, _w: u32, _h: u32, _flags: u32) -> Self {
        Window { container: Container { ctrl: Control { id: next_id() } } }
    }
    pub fn set_title(&self, _title: &str) {}
    pub fn destroy(&self) {}
    pub fn resize(&self, _w: u32, _h: u32) {}
    pub fn on_close(&self, _f: impl FnMut(&EventArgs) + 'static) {}
    pub fn on_resize(&self, _f: impl FnMut(&EventArgs) + 'static) {}
    pub fn on_key_down(&self, _f: impl FnMut(&KeyEvent) + 'static) {}
    pub fn on_key_up(&self, _f: impl FnMut(&KeyEvent) + 'static) {}
    pub fn on_click(&self, _f: impl FnMut(&ClickEvent) + 'static) {}
}

pub const WIN_FLAG_BORDERLESS: u32 = 0x01;
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x02;

// ── TextField ────────────────────────────────────────────────────────────────

pub struct TextField {
    ctrl: Control,
}

impl core::ops::Deref for TextField {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Widget for TextField {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl TextField {
    pub fn new() -> Self { TextField { ctrl: Control { id: next_id() } } }
    pub fn set_placeholder(&self, _text: &str) {}
    pub fn select_all(&self) {}
    pub fn set_password_mode(&self, _enabled: bool) {}
    pub fn on_text_changed(&self, _f: impl FnMut(&TextChangedEvent) + 'static) {}
    pub fn on_submit(&self, _f: impl FnMut(&SubmitEvent) + 'static) {}
}

// ── Checkbox ─────────────────────────────────────────────────────────────────

pub struct Checkbox {
    ctrl: Control,
}

impl core::ops::Deref for Checkbox {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Widget for Checkbox {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl Checkbox {
    pub fn new(_label: &str) -> Self { Checkbox { ctrl: Control { id: next_id() } } }
    pub fn on_checked_changed(&self, _f: impl FnMut(&CheckedChangedEvent) + 'static) {}
}

// ── RadioButton ──────────────────────────────────────────────────────────────

pub struct RadioButton {
    ctrl: Control,
}

impl core::ops::Deref for RadioButton {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Widget for RadioButton {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl RadioButton {
    pub fn new(_label: &str) -> Self { RadioButton { ctrl: Control { id: next_id() } } }
    pub fn on_checked_changed(&self, _f: impl FnMut(&CheckedChangedEvent) + 'static) {}
}

// ── TextArea ─────────────────────────────────────────────────────────────────

pub struct TextArea {
    ctrl: Control,
}

impl core::ops::Deref for TextArea {
    type Target = Control;
    fn deref(&self) -> &Control { &self.ctrl }
}

impl Widget for TextArea {
    fn id(&self) -> u32 { self.ctrl.id }
}

impl TextArea {
    pub fn new() -> Self { TextArea { ctrl: Control { id: next_id() } } }
    pub fn on_text_changed(&self, _f: impl FnMut(&TextChangedEvent) + 'static) {}
}

// ── Additional controls (stubs for anything else) ────────────────────────────

pub struct Label { ctrl: Control }
impl core::ops::Deref for Label { type Target = Control; fn deref(&self) -> &Control { &self.ctrl } }
impl Widget for Label { fn id(&self) -> u32 { self.ctrl.id } }
impl Label { pub fn new(_text: &str) -> Self { Label { ctrl: Control { id: next_id() } } } }

pub struct Button { ctrl: Control }
impl core::ops::Deref for Button { type Target = Control; fn deref(&self) -> &Control { &self.ctrl } }
impl Widget for Button { fn id(&self) -> u32 { self.ctrl.id } }
impl Button {
    pub fn new(_text: &str) -> Self { Button { ctrl: Control { id: next_id() } } }
    pub fn on_click(&self, _f: impl FnMut(&ClickEvent) + 'static) {}
}

pub struct ProgressBar { ctrl: Control }
impl core::ops::Deref for ProgressBar { type Target = Control; fn deref(&self) -> &Control { &self.ctrl } }
impl Widget for ProgressBar { fn id(&self) -> u32 { self.ctrl.id } }
impl ProgressBar { pub fn new(_val: u32) -> Self { ProgressBar { ctrl: Control { id: next_id() } } } }

pub struct TabBar { container: Container }
impl core::ops::Deref for TabBar { type Target = Container; fn deref(&self) -> &Container { &self.container } }
impl Widget for TabBar { fn id(&self) -> u32 { self.container.ctrl.id } }
impl TabBar {
    pub fn new() -> Self { TabBar { container: Container { ctrl: Control { id: next_id() } } } }
    pub fn add_tab(&self, _title: &str) -> u32 { next_id() }
    pub fn remove_tab(&self, _index: u32) {}
    pub fn set_tab_title(&self, _index: u32, _title: &str) {}
    pub fn on_tab_changed(&self, _f: impl FnMut(u32) + 'static) {}
    pub fn on_tab_close(&self, _f: impl FnMut(u32) + 'static) {}
    pub fn set_closable(&self, _closable: bool) {}
    pub fn selected_index(&self) -> u32 { 0 }
    pub fn select_tab(&self, _index: u32) {}
}

pub struct Toolbar { container: Container }
impl core::ops::Deref for Toolbar { type Target = Container; fn deref(&self) -> &Container { &self.container } }
impl Widget for Toolbar { fn id(&self) -> u32 { self.container.ctrl.id } }
impl Toolbar { pub fn new() -> Self { Toolbar { container: Container { ctrl: Control { id: next_id() } } } } }

pub struct IconButton { ctrl: Control }
impl core::ops::Deref for IconButton { type Target = Control; fn deref(&self) -> &Control { &self.ctrl } }
impl Widget for IconButton { fn id(&self) -> u32 { self.ctrl.id } }
impl IconButton {
    pub fn new(_text: &str) -> Self { IconButton { ctrl: Control { id: next_id() } } }
    pub fn on_click(&self, _f: impl FnMut(&ClickEvent) + 'static) {}
}

// ── Event types ──────────────────────────────────────────────────────────────

pub struct EventArgs { pub id: u32 }
pub struct ClickEvent { pub id: u32 }
pub struct CheckedChangedEvent { pub id: u32, pub checked: bool }
pub struct ScrollChangedEvent { pub id: u32, pub offset: u32 }

pub struct TextChangedEvent { pub id: u32 }
impl TextChangedEvent {
    pub fn text(&self) -> alloc::string::String { alloc::string::String::new() }
}

pub struct SubmitEvent { pub id: u32 }
pub struct ValueChangedEvent { pub id: u32, pub value: u32 }

pub struct KeyEvent {
    pub keycode: u32,
    pub char_code: u32,
    pub modifiers: u32,
}
impl KeyEvent {
    pub fn shift(&self) -> bool { self.modifiers & MOD_SHIFT != 0 }
    pub fn ctrl(&self) -> bool { self.modifiers & MOD_CTRL != 0 }
    pub fn alt(&self) -> bool { self.modifiers & MOD_ALT != 0 }
}

// ── Global functions ─────────────────────────────────────────────────────────

pub fn init() -> bool { true }
pub fn shutdown() {}
pub fn run() {}
pub fn run_once() -> bool { false }
pub fn quit() {}

/// Measure text dimensions using libfont_client.
pub fn measure_text(text: &str, font_id: u16, font_size: u16) -> (u32, u32) {
    libfont_client::measure(font_id as u32, font_size, text)
}

pub fn clipboard_get(_buf: &mut [u8]) -> u32 { 0 }
pub fn clipboard_set(_text: &str) {}

pub fn get_key_info() -> KeyEvent {
    KeyEvent { keycode: 0, char_code: 0, modifiers: 0 }
}
pub fn get_modifiers() -> u32 { 0 }

pub fn marshal_set_text(_id: u32, _text: &str) {}
pub fn marshal_set_color(_id: u32, _color: u32) {}
pub fn marshal_set_state(_id: u32, _value: u32) {}
pub fn marshal_set_visible(_id: u32, _visible: bool) {}

// ── FileDialog (stub) ────────────────────────────────────────────────────────

pub struct FileDialog;
impl FileDialog {
    pub fn open_file() -> Option<alloc::string::String> { None }
    pub fn save_file(_default: &str) -> Option<alloc::string::String> { None }
}
