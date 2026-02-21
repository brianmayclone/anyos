//! Control — base trait for all UI widgets (OOP base class).
//!
//! Every widget in anyui implements the `Control` trait, which provides
//! common properties (position, size, visibility, parent/child relationships)
//! and virtual methods that each control type overrides.
//!
//! Concrete control types live in `controls/`, each in its own file.
//! They embed a `ControlBase` struct for shared state and implement
//! `Control` to provide their specific rendering and event handling.

use alloc::boxed::Box;
use alloc::vec::Vec;

/// Unique identifier for a control in the widget tree.
pub type ControlId = u32;

/// Compositor event types (from window event buffer).
pub const COMP_EVENT_MOUSE_DOWN: u32 = 1;
pub const COMP_EVENT_MOUSE_UP: u32 = 2;
pub const COMP_EVENT_MOUSE_MOVE: u32 = 3;
pub const COMP_EVENT_KEY_DOWN: u32 = 4;
pub const COMP_EVENT_KEY_UP: u32 = 5;
pub const COMP_EVENT_WINDOW_CLOSE: u32 = 6;
pub const COMP_EVENT_MOUSE_SCROLL: u32 = 7;
pub const COMP_EVENT_WINDOW_RESIZE: u32 = 8;

/// Callback event types (passed to user callbacks).
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

/// Number of callback slots (EVENT_CLICK=1 .. EVENT_MOUSE_MOVE=16, index 0 unused).
const NUM_CALLBACK_SLOTS: usize = 17;

// ── Layout types (Windows Forms-inspired) ────────────────────────────

/// Inner spacing (space reserved inside a control for its children).
#[derive(Clone, Copy, Default)]
pub struct Padding {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Padding {
    pub const fn all(v: i32) -> Self { Self { left: v, top: v, right: v, bottom: v } }
}

/// Outer spacing (space reserved around a control, between it and siblings/parent).
#[derive(Clone, Copy, Default)]
pub struct Margin {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Margin {
    pub const fn all(v: i32) -> Self { Self { left: v, top: v, right: v, bottom: v } }
}

/// Dock style — how a control docks within its parent's client area.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum DockStyle {
    /// Manual positioning (x, y are used as-is).
    #[default]
    None = 0,
    /// Dock to parent's top edge, full width.
    Top = 1,
    /// Dock to parent's bottom edge, full width.
    Bottom = 2,
    /// Dock to parent's left edge, full height.
    Left = 3,
    /// Dock to parent's right edge, full height.
    Right = 4,
    /// Fill remaining space after other docked controls.
    Fill = 5,
}

impl DockStyle {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Top,
            2 => Self::Bottom,
            3 => Self::Left,
            4 => Self::Right,
            5 => Self::Fill,
            _ => Self::None,
        }
    }
}

/// Text styling properties shared by all text-displaying controls.
#[derive(Clone, Copy)]
pub struct TextStyle {
    /// Font size in pixels. Default: 14.
    pub font_size: u16,
    /// Font ID (0 = system default).
    pub font_id: u16,
    /// Text color override (0 = use theme default).
    pub text_color: u32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self { font_size: 14, font_id: 0, text_color: 0 }
    }
}

/// Orientation for layout containers (StackPanel).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Orientation {
    Vertical = 0,
    Horizontal = 1,
}

impl Orientation {
    pub fn from_u32(v: u32) -> Self {
        if v == 1 { Self::Horizontal } else { Self::Vertical }
    }
}

/// Callback function pointer type.
/// Parameters: (control_id, event_type, userdata)
pub type Callback = extern "C" fn(ControlId, u32, u64);

/// Control kind — discriminator for widget types.
///
/// Used via `anyui_add_control(parent, kind, ...)` where `kind` is one of these values.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ControlKind {
    Window = 0,
    View = 1,
    Label = 2,
    Button = 3,
    TextField = 4,
    Toggle = 5,
    Checkbox = 6,
    Slider = 7,
    RadioButton = 8,
    ProgressBar = 9,
    Stepper = 10,
    SegmentedControl = 11,
    TableView = 12,
    ScrollView = 13,
    Sidebar = 14,
    NavigationBar = 15,
    TabBar = 16,
    Toolbar = 17,
    Card = 18,
    GroupBox = 19,
    SplitView = 20,
    Divider = 21,
    Alert = 22,
    ContextMenu = 23,
    Tooltip = 24,
    ImageView = 25,
    StatusIndicator = 26,
    ColorWell = 27,
    SearchField = 28,
    TextArea = 29,
    IconButton = 30,
    Badge = 31,
    Tag = 32,
    StackPanel = 33,
    FlowPanel = 34,
    TableLayout = 35,
    Canvas = 36,
}

impl ControlKind {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Window,
            1 => Self::View,
            2 => Self::Label,
            3 => Self::Button,
            4 => Self::TextField,
            5 => Self::Toggle,
            6 => Self::Checkbox,
            7 => Self::Slider,
            8 => Self::RadioButton,
            9 => Self::ProgressBar,
            10 => Self::Stepper,
            11 => Self::SegmentedControl,
            12 => Self::TableView,
            13 => Self::ScrollView,
            14 => Self::Sidebar,
            15 => Self::NavigationBar,
            16 => Self::TabBar,
            17 => Self::Toolbar,
            18 => Self::Card,
            19 => Self::GroupBox,
            20 => Self::SplitView,
            21 => Self::Divider,
            22 => Self::Alert,
            23 => Self::ContextMenu,
            24 => Self::Tooltip,
            25 => Self::ImageView,
            26 => Self::StatusIndicator,
            27 => Self::ColorWell,
            28 => Self::SearchField,
            29 => Self::TextArea,
            30 => Self::IconButton,
            31 => Self::Badge,
            32 => Self::Tag,
            33 => Self::StackPanel,
            34 => Self::FlowPanel,
            35 => Self::TableLayout,
            36 => Self::Canvas,
            _ => Self::View,
        }
    }

    /// Default (width, height) for this control kind. (0, 0) = caller must provide.
    pub fn default_size(self) -> (u32, u32) {
        match self {
            Self::Label => (200, 20),
            Self::Button => (100, 32),
            Self::TextField | Self::SearchField => (200, 28),
            Self::Toggle => (44, 24),
            Self::Checkbox | Self::RadioButton => (20, 20),
            Self::Slider => (200, 20),
            Self::ProgressBar => (200, 8),
            Self::Stepper => (94, 28),
            Self::SegmentedControl => (200, 28),
            Self::Divider => (200, 1),
            Self::Badge | Self::StatusIndicator => (20, 20),
            Self::Tag => (80, 24),
            Self::TextArea => (300, 150),
            Self::IconButton | Self::ColorWell => (32, 32),
            Self::Tooltip => (150, 24),
            Self::Canvas => (200, 200),
            _ => (0, 0),
        }
    }
}

// ── ControlBase — shared state embedded in every concrete control ────

/// A single callback slot: function pointer + per-slot userdata.
#[derive(Clone, Copy)]
pub struct CallbackSlot {
    pub cb: Callback,
    pub userdata: u64,
}

/// Shared state for all controls (composition pattern for "base class" fields).
pub struct ControlBase {
    pub id: ControlId,
    pub parent: ControlId,
    pub children: Vec<ControlId>,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub visible: bool,
    pub text: Vec<u8>,
    pub color: u32,
    pub state: u32,

    // ── Layout properties (Windows Forms-style) ──
    pub padding: Padding,
    pub margin: Margin,
    pub dock: DockStyle,
    pub auto_size: bool,
    pub min_w: u32,
    pub min_h: u32,
    pub max_w: u32,
    pub max_h: u32,

    // ── Text styling ──
    pub text_style: TextStyle,

    /// Callback table indexed by event type (EVENT_CLICK=1 .. EVENT_MOUSE_MOVE=16).
    /// Index 0 is unused. Each slot has its own userdata.
    callbacks: [Option<CallbackSlot>; NUM_CALLBACK_SLOTS],
}

impl ControlBase {
    pub fn new(id: ControlId, parent: ControlId, x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            x,
            y,
            w,
            h,
            visible: true,
            text: Vec::new(),
            color: 0,
            state: 0,
            padding: Padding::default(),
            margin: Margin::default(),
            dock: DockStyle::None,
            auto_size: false,
            min_w: 0,
            min_h: 0,
            max_w: 0,
            max_h: 0,
            text_style: TextStyle::default(),
            callbacks: [None; NUM_CALLBACK_SLOTS],
        }
    }

    pub fn with_text(mut self, text: &[u8]) -> Self {
        self.text.extend_from_slice(text);
        self
    }

    pub fn with_color(mut self, color: u32) -> Self {
        self.color = color;
        self
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    /// Register a callback for the given event type.
    pub fn set_callback(&mut self, event_type: u32, cb: Callback, userdata: u64) {
        let idx = event_type as usize;
        if idx < NUM_CALLBACK_SLOTS {
            self.callbacks[idx] = Some(CallbackSlot { cb, userdata });
        }
    }

    /// Get the callback + userdata for the given event type.
    pub fn get_callback(&self, event_type: u32) -> Option<CallbackSlot> {
        let idx = event_type as usize;
        if idx < NUM_CALLBACK_SLOTS {
            self.callbacks[idx]
        } else {
            None
        }
    }
}

// ── EventResponse — return value from virtual event handlers ────────

/// Result of a virtual event handler call.
///
/// Controls return this to tell the event loop whether the event was consumed
/// and which additional callbacks to fire (beyond the base event callback).
#[derive(Clone, Copy)]
pub struct EventResponse {
    pub consumed: bool,
    pub fire_click: bool,
    pub fire_change: bool,
}

impl EventResponse {
    /// Event was ignored (not consumed).
    pub const IGNORED: Self = Self { consumed: false, fire_click: false, fire_change: false };
    /// Event was consumed, but no callback needed.
    pub const CONSUMED: Self = Self { consumed: true, fire_click: false, fire_change: false };
    /// Event consumed -> fire on_click callback.
    pub const CLICK: Self = Self { consumed: true, fire_click: true, fire_change: false };
    /// Event consumed -> fire on_change callback.
    pub const CHANGED: Self = Self { consumed: true, fire_click: false, fire_change: true };
    /// Event consumed -> fire both callbacks.
    pub const CLICK_AND_CHANGED: Self = Self { consumed: true, fire_click: true, fire_change: true };
}

// ── Control trait — virtual base class ──────────────────────────────

/// The base trait for all UI controls (virtual base class).
///
/// Every concrete control implements this trait. The event model provides
/// **base events** that are fired for ALL controls automatically by the event loop:
///
/// - MouseEnter / MouseLeave — hover tracking
/// - MouseDown / MouseUp — raw pointer press/release
/// - Click — mouse down + up on same control
/// - DoubleClick — two clicks within 400ms
/// - Focus / Blur — keyboard focus changes
/// - KeyDown — keyboard input to focused control
/// - Scroll — mouse wheel
///
/// Each control overrides the virtual methods relevant to its behavior.
/// Default implementations do nothing (return IGNORED).
pub trait Control {
    /// Access the shared base fields.
    fn base(&self) -> &ControlBase;
    /// Mutable access to the shared base fields.
    fn base_mut(&mut self) -> &mut ControlBase;
    /// The type discriminator of this control.
    fn kind(&self) -> ControlKind;

    /// Render this control. `parent_abs_x/y` is the parent's absolute position;
    /// the control adds its own (x, y) offset.
    ///
    /// **Override this in each concrete control type.**
    fn render(&self, win: u32, parent_abs_x: i32, parent_abs_y: i32);

    /// Whether this control accepts mouse/keyboard input.
    fn is_interactive(&self) -> bool {
        false
    }

    /// Whether this control can receive keyboard focus.
    fn accepts_focus(&self) -> bool {
        self.is_interactive()
    }

    /// Whether this control displays text (and supports TextStyle properties).
    fn is_text_control(&self) -> bool {
        false
    }

    /// Override for layout containers (StackPanel, FlowPanel, TableLayout).
    /// Called by the layout engine to position children according to the
    /// container's specific layout algorithm.
    /// Returns true if this control handled its children's layout.
    fn layout_children(&self, _controls: &mut [Box<dyn Control>]) -> bool {
        false
    }

    // ── Virtual event handlers (override in subclasses) ──────────────

    /// Called when mouse cursor enters this control's bounds.
    fn handle_mouse_enter(&mut self) {}

    /// Called when mouse cursor leaves this control's bounds.
    fn handle_mouse_leave(&mut self) {}

    /// Called when mouse button is pressed on this control.
    /// `local_x/y` are relative to this control's top-left corner.
    fn handle_mouse_down(&mut self, _local_x: i32, _local_y: i32, _button: u32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when mouse button is released on this control.
    fn handle_mouse_up(&mut self, _local_x: i32, _local_y: i32, _button: u32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when mouse moves while this control is pressed (drag).
    fn handle_mouse_move(&mut self, _local_x: i32, _local_y: i32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when mouse is clicked (down + up on same control).
    /// This is a higher-level event synthesized by the event loop.
    fn handle_click(&mut self, _local_x: i32, _local_y: i32, _button: u32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when mouse is double-clicked (two clicks within 400ms).
    fn handle_double_click(&mut self, _local_x: i32, _local_y: i32, _button: u32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when a key is pressed while this control has focus.
    /// `char_code` is the ASCII character (0 if non-printable).
    fn handle_key_down(&mut self, _keycode: u32, _char_code: u32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when mouse wheel scrolls over this control.
    fn handle_scroll(&mut self, _delta: i32) -> EventResponse {
        EventResponse::IGNORED
    }

    /// Called when this control receives keyboard focus.
    fn handle_focus(&mut self) {}

    /// Called when this control loses keyboard focus.
    fn handle_blur(&mut self) {}

    // ── Default property accessors (delegate to ControlBase) ────────

    fn id(&self) -> ControlId {
        self.base().id
    }
    fn parent_id(&self) -> ControlId {
        self.base().parent
    }
    fn set_parent(&mut self, p: ControlId) {
        self.base_mut().parent = p;
    }
    fn children(&self) -> &[ControlId] {
        &self.base().children
    }
    fn add_child(&mut self, c: ControlId) {
        self.base_mut().children.push(c);
    }
    fn remove_child(&mut self, c: ControlId) {
        self.base_mut().children.retain(|&x| x != c);
    }
    fn position(&self) -> (i32, i32) {
        (self.base().x, self.base().y)
    }
    fn set_position(&mut self, x: i32, y: i32) {
        let b = self.base_mut();
        b.x = x;
        b.y = y;
    }
    fn size(&self) -> (u32, u32) {
        (self.base().w, self.base().h)
    }
    fn set_size(&mut self, w: u32, h: u32) {
        let b = self.base_mut();
        b.w = w;
        b.h = h;
    }
    fn visible(&self) -> bool {
        self.base().visible
    }
    fn set_visible(&mut self, v: bool) {
        self.base_mut().visible = v;
    }
    fn text(&self) -> &[u8] {
        &self.base().text
    }
    fn set_text(&mut self, t: &[u8]) {
        let b = self.base_mut();
        b.text.clear();
        b.text.extend_from_slice(t);
    }
    fn color(&self) -> u32 {
        self.base().color
    }
    fn set_color(&mut self, c: u32) {
        self.base_mut().color = c;
    }
    fn state_val(&self) -> u32 {
        self.base().state
    }
    fn set_state(&mut self, s: u32) {
        self.base_mut().state = s;
    }

    // ── Callback accessors (generic, indexed by event type) ─────────

    fn set_event_callback(&mut self, event_type: u32, cb: Callback, userdata: u64) {
        self.base_mut().set_callback(event_type, cb, userdata);
    }

    fn get_event_callback(&self, event_type: u32) -> Option<CallbackSlot> {
        self.base().get_callback(event_type)
    }

    // Convenience aliases
    fn set_on_click(&mut self, cb: Callback, ud: u64) {
        self.base_mut().set_callback(EVENT_CLICK, cb, ud);
    }
    fn set_on_change(&mut self, cb: Callback, ud: u64) {
        self.base_mut().set_callback(EVENT_CHANGE, cb, ud);
    }
}

// ── Tree utilities ──────────────────────────────────────────────────

/// Find a control by ID. Returns index in the slice.
pub fn find_idx(controls: &[Box<dyn Control>], id: ControlId) -> Option<usize> {
    controls.iter().position(|c| c.id() == id)
}

/// Hit-test: find the deepest visible interactive control under (px, py).
/// Coordinates are in window-local space.
pub fn hit_test(
    controls: &[Box<dyn Control>],
    root: ControlId,
    px: i32,
    py: i32,
    parent_x: i32,
    parent_y: i32,
) -> Option<ControlId> {
    let idx = find_idx(controls, root)?;
    let b = controls[idx].base();

    if !b.visible {
        return None;
    }

    let abs_x = parent_x + b.x;
    let abs_y = parent_y + b.y;

    if px < abs_x || py < abs_y || px >= abs_x + b.w as i32 || py >= abs_y + b.h as i32 {
        return None;
    }

    // Check children in reverse order (topmost first)
    let children: Vec<ControlId> = b.children.to_vec();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test(controls, child_id, px, py, abs_x, abs_y) {
            return Some(hit);
        }
    }

    // This node is the target if interactive or has any relevant callback
    if controls[idx].is_interactive()
        || b.get_callback(EVENT_CLICK).is_some()
        || b.get_callback(EVENT_MOUSE_DOWN).is_some()
    {
        Some(root)
    } else {
        None
    }
}

/// Hit-test that returns ANY visible control (not just interactive ones).
/// Used for mouse enter/leave tracking on all controls.
pub fn hit_test_any(
    controls: &[Box<dyn Control>],
    root: ControlId,
    px: i32,
    py: i32,
    parent_x: i32,
    parent_y: i32,
) -> Option<ControlId> {
    let idx = find_idx(controls, root)?;
    let b = controls[idx].base();

    if !b.visible {
        return None;
    }

    let abs_x = parent_x + b.x;
    let abs_y = parent_y + b.y;

    if px < abs_x || py < abs_y || px >= abs_x + b.w as i32 || py >= abs_y + b.h as i32 {
        return None;
    }

    // Check children in reverse order (topmost first)
    let children: Vec<ControlId> = b.children.to_vec();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test_any(controls, child_id, px, py, abs_x, abs_y) {
            return Some(hit);
        }
    }

    Some(root)
}

/// Calculate the absolute position of a control by walking up the parent chain.
pub fn abs_position(controls: &[Box<dyn Control>], id: ControlId) -> (i32, i32) {
    let mut ax = 0i32;
    let mut ay = 0i32;
    let mut cur = id;
    loop {
        if let Some(idx) = find_idx(controls, cur) {
            let (x, y) = controls[idx].position();
            ax += x;
            ay += y;
            let parent = controls[idx].parent_id();
            if parent == 0 || parent == cur {
                break;
            }
            cur = parent;
        } else {
            break;
        }
    }
    (ax, ay)
}
