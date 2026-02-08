//! Public types for uisys components.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Default = 0,
    Primary = 1,
    Destructive = 2,
    Plain = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal = 0,
    Hover = 1,
    Pressed = 2,
    Disabled = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CheckboxState {
    Unchecked = 0,
    Checked = 1,
    Indeterminate = 2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left = 0,
    Center = 1,
    Right = 2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontSize {
    Small = 13,
    Normal = 16,
    Large = 20,
    Title = 24,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Online = 0,
    Warning = 1,
    Error = 2,
    Offline = 3,
}

// ============================================================================
// UI Event (wraps raw [u32; 5] window events)
// ============================================================================

// Window event types
pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;
pub const EVENT_WINDOW_CLOSE: u32 = 8;

// Key codes (must match kernel encode_key in desktop.rs)
pub const KEY_ENTER: u32 = 0x100;
pub const KEY_BACKSPACE: u32 = 0x101;
pub const KEY_TAB: u32 = 0x102;
pub const KEY_ESCAPE: u32 = 0x103;
pub const KEY_SPACE: u32 = 0x104;
pub const KEY_UP: u32 = 0x105;
pub const KEY_DOWN: u32 = 0x106;
pub const KEY_LEFT: u32 = 0x107;
pub const KEY_RIGHT: u32 = 0x108;
pub const KEY_DELETE: u32 = 0x120;
pub const KEY_HOME: u32 = 0x121;
pub const KEY_END: u32 = 0x122;
pub const KEY_PAGE_UP: u32 = 0x123;
pub const KEY_PAGE_DOWN: u32 = 0x124;

#[derive(Clone, Copy)]
pub struct UiEvent {
    pub event_type: u32,
    pub p1: u32,
    pub p2: u32,
    pub p3: u32,
    pub p4: u32,
}

impl UiEvent {
    pub fn from_raw(e: &[u32; 5]) -> Self {
        UiEvent { event_type: e[0], p1: e[1], p2: e[2], p3: e[3], p4: e[4] }
    }
    pub fn is_mouse_down(&self) -> bool { self.event_type == EVENT_MOUSE_DOWN }
    pub fn is_mouse_up(&self) -> bool { self.event_type == EVENT_MOUSE_UP }
    pub fn is_key_down(&self) -> bool { self.event_type == EVENT_KEY_DOWN }
    pub fn mouse_pos(&self) -> (i32, i32) { (self.p1 as i32, self.p2 as i32) }
    pub fn key_code(&self) -> u32 { self.p1 }
    pub fn char_val(&self) -> u32 { self.p2 }
}

/// Theme color constants matching the DLL's built-in palette.
pub mod colors {
    pub const WINDOW_BG: u32       = 0xFF1E1E1E;
    pub const TEXT: u32             = 0xFFE6E6E6;
    pub const TEXT_SECONDARY: u32   = 0xFF969696;
    pub const TEXT_DISABLED: u32    = 0xFF5A5A5A;
    pub const ACCENT: u32          = 0xFF007AFF;
    pub const ACCENT_HOVER: u32    = 0xFF0A84FF;
    pub const DESTRUCTIVE: u32     = 0xFFFF3B30;
    pub const SUCCESS: u32         = 0xFF30D158;
    pub const WARNING: u32         = 0xFFFFD60A;
    pub const CONTROL_BG: u32      = 0xFF3C3C3C;
    pub const SEPARATOR: u32       = 0xFF3D3D3D;
    pub const SIDEBAR_BG: u32      = 0xFF252525;
    pub const CARD_BG: u32         = 0xFF2A2A2A;
}
