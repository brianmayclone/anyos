//! Safe Rust client API for the userspace compositor (via libcompositor.dlib).
//!
//! # Usage
//! ```rust,ignore
//! let mut client = libcompositor_client::CompositorClient::init().unwrap();
//! let win = client.create_window(-1, -1, 400, 300, 0).unwrap();
//!
//! // Draw to the window surface (raw pixel buffer)
//! let surface = win.surface();
//! for i in 0..(400 * 300) {
//!     unsafe { *surface.add(i) = 0xFF2E2E2E; }
//! }
//!
//! // Signal compositor to composite
//! client.present(&win);
//!
//! // Poll events
//! if let Some(event) = client.poll_event(&win) {
//!     // handle event
//! }
//! ```

#![no_std]

pub mod raw;

// IPC event types (from compositor's ipc_protocol.rs)
pub const EVT_KEY_DOWN: u32 = 0x3001;
pub const EVT_KEY_UP: u32 = 0x3002;
pub const EVT_MOUSE_DOWN: u32 = 0x3003;
pub const EVT_MOUSE_UP: u32 = 0x3004;
pub const EVT_MOUSE_SCROLL: u32 = 0x3005;
pub const EVT_RESIZE: u32 = 0x3006;
pub const EVT_WINDOW_CLOSE: u32 = 0x3007;
pub const EVT_MENU_ITEM: u32 = 0x3008;
pub const EVT_STATUS_ICON_CLICK: u32 = 0x3009;
pub const EVT_MOUSE_MOVE: u32 = 0x300A;

/// A handle to a compositor window.
pub struct WindowHandle {
    pub id: u32,
    pub shm_id: u32,
    surface_ptr: *mut u32,
    pub width: u32,
    pub height: u32,
}

impl WindowHandle {
    /// Get a raw pointer to the window's pixel surface.
    /// The surface is `width * height` u32 pixels in ARGB8888 format.
    pub fn surface(&self) -> *mut u32 {
        self.surface_ptr
    }

    /// Get a mutable slice of the surface pixels.
    pub unsafe fn surface_slice(&self) -> &mut [u32] {
        let count = (self.width * self.height) as usize;
        core::slice::from_raw_parts_mut(self.surface_ptr, count)
    }
}

/// A handle to a VRAM-direct window (app renders directly to GPU VRAM).
/// The surface pointer points to mapped VRAM memory. The row stride is
/// `stride` pixels (not `width`), because VRAM rows are pitch-aligned.
pub struct VramWindowHandle {
    pub id: u32,
    surface_ptr: *mut u32,
    pub width: u32,
    pub height: u32,
    /// Row stride in pixels (fb_pitch / 4). Use this instead of `width` for row offsets.
    pub stride: u32,
}

impl VramWindowHandle {
    /// Get a raw pointer to the VRAM surface.
    /// Pixels are ARGB8888. Row stride is `self.stride` pixels (NOT `self.width`).
    pub fn surface(&self) -> *mut u32 {
        self.surface_ptr
    }

    /// Get a mutable slice of the VRAM surface.
    /// Note: length = stride * height (covers gaps between rows).
    pub unsafe fn surface_slice(&self) -> &mut [u32] {
        let count = (self.stride * self.height) as usize;
        core::slice::from_raw_parts_mut(self.surface_ptr, count)
    }

    /// Write a pixel at (x, y) using the correct stride.
    pub unsafe fn put_pixel(&self, x: u32, y: u32, color: u32) {
        let off = y * self.stride + x;
        *self.surface_ptr.add(off as usize) = color;
    }
}

/// An event received from the compositor.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub event_type: u32,
    pub window_id: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg3: u32,
}

/// Compositor client connection.
pub struct CompositorClient {
    pub channel_id: u32,
    sub_id: u32,
}

impl CompositorClient {
    /// Connect to the compositor.
    /// Returns None if the compositor is not running or channel creation fails.
    pub fn init() -> Option<Self> {
        let mut sub_id: u32 = 0;
        let channel_id = (raw::exports().init)(&mut sub_id);
        if channel_id == 0 {
            return None;
        }
        Some(CompositorClient { channel_id, sub_id })
    }

    /// Create a new window at position (x, y) with the given content dimensions.
    /// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
    /// Returns a WindowHandle on success, None on failure.
    pub fn create_window(&self, x: i32, y: i32, width: u32, height: u32, flags: u32) -> Option<WindowHandle> {
        let mut shm_id: u32 = 0;
        let mut surface: *mut u32 = core::ptr::null_mut();
        let id = (raw::exports().create_window)(
            self.channel_id,
            self.sub_id,
            x,
            y,
            width,
            height,
            flags,
            &mut shm_id,
            &mut surface,
        );
        if id == 0 || surface.is_null() {
            return None;
        }
        Some(WindowHandle {
            id,
            shm_id,
            surface_ptr: surface,
            width,
            height,
        })
    }

    /// Create a VRAM-direct window (app renders directly to GPU VRAM).
    /// Returns a VramWindowHandle on success, None if GPU not available or VRAM exhausted.
    /// The app should fall back to `create_window()` (SHM path) on failure.
    ///
    /// IMPORTANT: The surface stride is `handle.stride` pixels (not `width`).
    /// Use `y * stride + x` to address pixels, not `y * width + x`.
    pub fn create_vram_window(&self, x: i32, y: i32, width: u32, height: u32, flags: u32) -> Option<VramWindowHandle> {
        let mut stride: u32 = 0;
        let mut surface: *mut u32 = core::ptr::null_mut();
        let id = (raw::exports().create_vram_window)(
            self.channel_id,
            self.sub_id,
            x,
            y,
            width,
            height,
            flags,
            &mut stride,
            &mut surface,
        );
        if id == 0 || surface.is_null() {
            return None;
        }
        Some(VramWindowHandle {
            id,
            surface_ptr: surface,
            width,
            height,
            stride,
        })
    }

    /// Destroy a window.
    pub fn destroy_window(&self, handle: &WindowHandle) {
        (raw::exports().destroy_window)(self.channel_id, handle.id, handle.shm_id);
    }

    /// Destroy a VRAM-direct window.
    pub fn destroy_vram_window(&self, handle: &VramWindowHandle) {
        (raw::exports().destroy_window)(self.channel_id, handle.id, 0);
    }

    /// Signal the compositor that window content has been updated.
    pub fn present(&self, handle: &WindowHandle) {
        (raw::exports().present)(self.channel_id, handle.id, handle.shm_id);
    }

    /// Signal the compositor that a rectangular region of window content has been updated.
    /// Only the dirty rect is copied to the compositor layer (much faster than full present).
    pub fn present_rect(&self, handle: &WindowHandle, x: u32, y: u32, w: u32, h: u32) {
        (raw::exports().present_rect)(self.channel_id, handle.id, handle.shm_id, x, y, w, h);
    }

    /// Signal the compositor that VRAM window content has been updated.
    pub fn present_vram(&self, handle: &VramWindowHandle) {
        (raw::exports().present)(self.channel_id, handle.id, 0);
    }

    /// Poll for the next event for a specific window.
    pub fn poll_event(&self, handle: &WindowHandle) -> Option<Event> {
        let mut buf = [0u32; 5];
        let ok = (raw::exports().poll_event)(
            self.channel_id,
            self.sub_id,
            handle.id,
            &mut buf,
        );
        if ok != 0 {
            Some(Event {
                event_type: buf[0],
                window_id: buf[1],
                arg1: buf[2],
                arg2: buf[3],
                arg3: buf[4],
            })
        } else {
            None
        }
    }

    /// Poll for the next event for a VRAM window.
    pub fn poll_event_vram(&self, handle: &VramWindowHandle) -> Option<Event> {
        let mut buf = [0u32; 5];
        let ok = (raw::exports().poll_event)(
            self.channel_id,
            self.sub_id,
            handle.id,
            &mut buf,
        );
        if ok != 0 {
            Some(Event {
                event_type: buf[0],
                window_id: buf[1],
                arg1: buf[2],
                arg2: buf[3],
                arg3: buf[4],
            })
        } else {
            None
        }
    }

    /// Set window title (up to 12 ASCII characters).
    pub fn set_title(&self, handle: &WindowHandle, title: &str) {
        let bytes = title.as_bytes();
        (raw::exports().set_title)(
            self.channel_id,
            handle.id,
            bytes.as_ptr(),
            bytes.len() as u32,
        );
    }

    /// Get screen dimensions.
    pub fn screen_size(&self) -> (u32, u32) {
        let mut w: u32 = 0;
        let mut h: u32 = 0;
        (raw::exports().screen_size)(&mut w, &mut h);
        (w, h)
    }

    /// Move a window to a new position.
    pub fn move_window(&self, handle: &WindowHandle, x: i32, y: i32) {
        (raw::exports().move_window)(self.channel_id, handle.id, x, y);
    }

    /// Set a window's menu bar definition.
    /// `menu_data` is a flat binary blob produced by `MenuBarBuilder::build()`.
    pub fn set_menu(&self, handle: &WindowHandle, menu_data: &[u8]) {
        (raw::exports().set_menu)(
            self.channel_id,
            handle.id,
            menu_data.as_ptr(),
            menu_data.len() as u32,
        );
    }

    /// Register a 16x16 ARGB status icon in the menubar.
    pub fn add_status_icon(&self, icon_id: u32, pixels: &[u32; 256]) {
        (raw::exports().add_status_icon)(self.channel_id, icon_id, pixels.as_ptr());
    }

    /// Remove a status icon from the menubar.
    pub fn remove_status_icon(&self, icon_id: u32) {
        (raw::exports().remove_status_icon)(self.channel_id, icon_id);
    }

    /// Update a menu item's flags (enable/disable/check).
    pub fn update_menu_item(&self, handle: &WindowHandle, item_id: u32, new_flags: u32) {
        (raw::exports().update_menu_item)(self.channel_id, handle.id, item_id, new_flags);
    }

    /// Enable blur-behind on a window (frosted glass effect).
    /// The compositor blurs the composited background behind the window layer
    /// before alpha-blending the window on top. The window must have semi-transparent
    /// pixels for the blur to be visible.
    /// `radius` controls blur strength (0 = disable, typically 4-12).
    pub fn set_blur_behind(&self, handle: &WindowHandle, radius: u32) {
        (raw::exports().set_blur_behind)(self.channel_id, handle.id, radius);
    }

    /// Set the desktop wallpaper by file path.
    /// The compositor loads the image, scales it to the screen, and recomposes.
    pub fn set_wallpaper(&self, path: &str) {
        let bytes = path.as_bytes();
        (raw::exports().set_wallpaper)(self.channel_id, bytes.as_ptr(), bytes.len() as u32);
    }

    /// Resize a window's shared memory surface to new dimensions.
    /// Updates the WindowHandle in-place with the new SHM id, surface pointer,
    /// and dimensions. Returns true on success.
    pub fn resize_window(&self, handle: &mut WindowHandle, new_width: u32, new_height: u32) -> bool {
        let mut new_shm_id: u32 = 0;
        let new_surface = (raw::exports().resize_shm)(
            self.channel_id,
            handle.id,
            handle.shm_id,
            new_width,
            new_height,
            &mut new_shm_id,
        );
        if new_surface.is_null() || new_shm_id == 0 {
            return false;
        }
        handle.shm_id = new_shm_id;
        handle.surface_ptr = new_surface;
        handle.width = new_width;
        handle.height = new_height;
        true
    }
}

/// Convenience: get screen size without a client connection.
pub fn screen_size() -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (raw::exports().screen_size)(&mut w, &mut h);
    (w, h)
}

// ── Tray Client ─────────────────────────────────────────────────────────────

/// Compositor client for tray-icon apps (windowless event delivery).
///
/// Unlike `CompositorClient`, `TrayClient` does not require a window for
/// event delivery.  `poll_event()` returns ALL events unfiltered — the app
/// routes by `event_type` and `window_id`.
pub struct TrayClient {
    pub channel_id: u32,
    sub_id: u32,
}

impl TrayClient {
    /// Connect to the compositor.
    pub fn init() -> Option<Self> {
        let mut sub_id: u32 = 0;
        let channel_id = (raw::exports().init)(&mut sub_id);
        if channel_id == 0 {
            return None;
        }
        Some(TrayClient { channel_id, sub_id })
    }

    /// Register or update a 16x16 ARGB tray icon (upsert).
    pub fn set_icon(&self, icon_id: u32, pixels: &[u32; 256]) {
        (raw::exports().add_status_icon)(self.channel_id, icon_id, pixels.as_ptr());
    }

    /// Remove a tray icon.
    pub fn remove_icon(&self, icon_id: u32) {
        (raw::exports().remove_status_icon)(self.channel_id, icon_id);
    }

    /// Poll for the next event (unfiltered — tray clicks, window events, broadcasts).
    pub fn poll_event(&self) -> Option<Event> {
        let mut buf = [0u32; 5];
        let ok = (raw::exports().tray_poll_event)(
            self.channel_id,
            self.sub_id,
            &mut buf,
        );
        if ok != 0 {
            Some(Event {
                event_type: buf[0],
                window_id: buf[1],
                arg1: buf[2],
                arg2: buf[3],
                arg3: buf[4],
            })
        } else {
            None
        }
    }

    /// Create a window at position (x, y) (for popups, settings panels, etc.).
    /// x/y: pixel coordinates, or -1 for compositor auto-placement.
    pub fn create_window(&self, x: i32, y: i32, width: u32, height: u32, flags: u32) -> Option<WindowHandle> {
        let mut shm_id: u32 = 0;
        let mut surface: *mut u32 = core::ptr::null_mut();
        let id = (raw::exports().create_window)(
            self.channel_id,
            self.sub_id,
            x,
            y,
            width,
            height,
            flags,
            &mut shm_id,
            &mut surface,
        );
        if id == 0 || surface.is_null() {
            return None;
        }
        Some(WindowHandle {
            id,
            shm_id,
            surface_ptr: surface,
            width,
            height,
        })
    }

    /// Destroy a window.
    pub fn destroy_window(&self, handle: &WindowHandle) {
        (raw::exports().destroy_window)(self.channel_id, handle.id, handle.shm_id);
    }

    /// Signal the compositor that window content has been updated.
    pub fn present(&self, handle: &WindowHandle) {
        (raw::exports().present)(self.channel_id, handle.id, handle.shm_id);
    }

    /// Signal the compositor that a rectangular region of window content has been updated.
    pub fn present_rect(&self, handle: &WindowHandle, x: u32, y: u32, w: u32, h: u32) {
        (raw::exports().present_rect)(self.channel_id, handle.id, handle.shm_id, x, y, w, h);
    }

    /// Move a window to a new position.
    pub fn move_window(&self, handle: &WindowHandle, x: i32, y: i32) {
        (raw::exports().move_window)(self.channel_id, handle.id, x, y);
    }

    /// Get screen dimensions.
    pub fn screen_size(&self) -> (u32, u32) {
        let mut w: u32 = 0;
        let mut h: u32 = 0;
        (raw::exports().screen_size)(&mut w, &mut h);
        (w, h)
    }
}

// ── Menu Bar Builder ────────────────────────────────────────────────────────

const MENU_MAGIC: u32 = 0x4D454E55; // 'MENU'

/// Item flags for menu items.
pub const MENU_FLAG_DISABLED: u32 = 0x01;
pub const MENU_FLAG_SEPARATOR: u32 = 0x02;
pub const MENU_FLAG_CHECKED: u32 = 0x04;

/// Builder for constructing a binary menu bar definition.
///
/// ```rust,ignore
/// let data = MenuBarBuilder::new()
///     .menu("File")
///         .item(1, "New", 0)
///         .item(2, "Open...", 0)
///         .separator()
///         .item(5, "Quit", 0)
///     .end_menu()
///     .menu("Edit")
///         .item(10, "Cut", 0)
///         .item(11, "Copy", 0)
///         .item(12, "Paste", 0)
///     .end_menu()
///     .build();
/// client.set_menu(&win, &data);
/// ```
pub struct MenuBarBuilder {
    buf: [u8; 4096],
    pos: usize,
    num_menus: usize,
    num_menus_offset: usize,
}

/// Intermediate builder for adding items to a single menu.
/// Owns the parent `MenuBarBuilder` so the chain is fully by-value.
pub struct MenuBuilder {
    inner: MenuBarBuilder,
    num_items: usize,
    num_items_offset: usize,
}

impl MenuBarBuilder {
    pub fn new() -> Self {
        let mut b = MenuBarBuilder {
            buf: [0u8; 4096],
            pos: 0,
            num_menus: 0,
            num_menus_offset: 0,
        };
        // Write magic
        b.write_u32(MENU_MAGIC);
        // Placeholder for num_menus — patched in build()
        b.num_menus_offset = b.pos;
        b.write_u32(0);
        b
    }

    /// Start a new top-level menu with the given title.
    pub fn menu(mut self, title: &str) -> MenuBuilder {
        let bytes = title.as_bytes();
        let len = bytes.len().min(64);
        self.write_u32(len as u32);
        self.write_bytes(&bytes[..len]);
        self.align4();
        // Placeholder for num_items
        let num_items_offset = self.pos;
        self.write_u32(0);
        self.num_menus += 1;
        MenuBuilder {
            inner: self,
            num_items: 0,
            num_items_offset,
        }
    }

    /// Finalize and return the total byte length of the menu data.
    /// The data lives in the builder's internal buffer.
    pub fn build(&mut self) -> &[u8] {
        // Patch num_menus
        let nm = self.num_menus as u32;
        self.buf[self.num_menus_offset..self.num_menus_offset + 4]
            .copy_from_slice(&nm.to_le_bytes());
        &self.buf[..self.pos]
    }

    fn write_u32(&mut self, val: u32) {
        if self.pos + 4 <= self.buf.len() {
            self.buf[self.pos..self.pos + 4].copy_from_slice(&val.to_le_bytes());
            self.pos += 4;
        }
    }

    fn write_bytes(&mut self, data: &[u8]) {
        let end = (self.pos + data.len()).min(self.buf.len());
        let count = end - self.pos;
        self.buf[self.pos..self.pos + count].copy_from_slice(&data[..count]);
        self.pos += count;
    }

    fn align4(&mut self) {
        while self.pos % 4 != 0 && self.pos < self.buf.len() {
            self.buf[self.pos] = 0;
            self.pos += 1;
        }
    }
}

impl MenuBuilder {
    /// Add a menu item with an app-defined item_id, label, and optional flags.
    pub fn item(mut self, item_id: u32, label: &str, flags: u32) -> Self {
        self.inner.write_u32(item_id);
        self.inner.write_u32(flags);
        let bytes = label.as_bytes();
        let len = bytes.len().min(64);
        self.inner.write_u32(len as u32);
        self.inner.write_bytes(&bytes[..len]);
        self.inner.align4();
        self.num_items += 1;
        self
    }

    /// Add a separator line.
    pub fn separator(self) -> Self {
        self.item(0, "", MENU_FLAG_SEPARATOR)
    }

    /// Finish this menu and return to the MenuBarBuilder.
    pub fn end_menu(mut self) -> MenuBarBuilder {
        // Patch num_items at the saved offset
        let ni = self.num_items as u32;
        self.inner.buf[self.num_items_offset..self.num_items_offset + 4]
            .copy_from_slice(&ni.to_le_bytes());
        self.inner
    }
}
