use crate::drivers::input::keyboard::KeyEvent;

/// UI Event types
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Mouse moved to absolute position
    MouseMove { x: i32, y: i32 },
    /// Mouse button pressed
    MouseDown { x: i32, y: i32, button: MouseButton },
    /// Mouse button released
    MouseUp { x: i32, y: i32, button: MouseButton },
    /// Mouse dragged (button held + move)
    MouseDrag { x: i32, y: i32, button: MouseButton },
    /// Keyboard key event
    Key(KeyEvent),
    /// Window should redraw
    Paint,
    /// Window moved
    WindowMove { x: i32, y: i32 },
    /// Window resized
    WindowResize { width: u32, height: u32 },
    /// Window close requested
    WindowClose,
    /// Window gained focus
    FocusGained,
    /// Window lost focus
    FocusLost,
    /// Timer tick (for animations etc.)
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Hit-test result for window decorations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTest {
    /// Click was on the client area
    Client,
    /// Click was on the title bar (for dragging)
    TitleBar,
    /// Click was on the close button
    CloseButton,
    /// Click was on the minimize button
    MinimizeButton,
    /// Click was on the maximize button
    MaximizeButton,
    /// Click was on a resize edge/corner
    ResizeLeft,
    ResizeRight,
    ResizeBottom,
    ResizeBottomLeft,
    ResizeBottomRight,
    /// Click was outside the window
    None,
}
