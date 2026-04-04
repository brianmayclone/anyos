# anyOS Compositor Library (libcompositor) API Reference

The **libcompositor** DLL provides IPC-based window management for GUI applications. Windows are backed by shared memory (SHM) pixel buffers and communicate with the compositor process via an event channel.

**DLL Address:** `0x04380000`
**Version:** 1
**Exports:** 28
**Client crate:** `libcompositor_client`

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libcompositor_client = { path = "../../libs/libcompositor_client" }
```

### Minimal Window Example

```rust
use libcompositor_client::CompositorClient;

let mut client = CompositorClient::init().expect("compositor not running");
let win = client.create_window(-1, -1, 400, 300, 0).expect("create failed");

// Get pixel buffer (ARGB8888, width * height u32s)
let surface = client.surface(win);

// Draw a red pixel at (10, 10)
surface[10 + 10 * 400] = 0xFFFF0000;

// Present to compositor
client.present(win);

// Event loop
loop {
    if let Some(event) = client.poll_event(win) {
        match event.event_type {
            0x3007 => break, // EVT_WINDOW_CLOSE
            _ => {}
        }
    }
    anyos_std::process::yield_cpu();
}

client.destroy_window(win);
```

---

## Architecture

```
+-------------------+        Event Channel        +-------------------+
|   Application     |  ←————————————————————————→  |   Compositor      |
|   (libcompositor) |                              |   (Ring 3)        |
+--------+----------+                              +--------+----------+
         |                                                  |
    SHM Pixel Buffer                                  Composites to
    (ARGB8888)                                        Framebuffer
```

1. Application calls `init()` — connects to compositor event channel
2. `create_window()` — allocates SHM buffer, compositor creates window layer
3. Application draws into SHM pixel buffer directly
4. `present()` — notifies compositor that buffer is updated
5. Compositor composites all dirty windows → `flush_gpu()` (VSync)
6. Compositor emits `EVT_FRAME_ACK` directly from render thread → app knows frame is on screen
7. Events (keyboard, mouse, resize, close, frame ACK) flow back via channel

---

## Functions

### `init(out_sub_id) -> channel_id`

Connect to the compositor. Returns channel ID for all subsequent calls, fills `out_sub_id` with event subscription ID. Returns 0 on failure (compositor not running).

### `create_window(channel_id, sub_id, x, y, width, height, flags, out_shm_id, out_surface) -> window_id`

Create a new window at position (x, y) with the given content dimensions.

| Parameter | Type | Description |
|-----------|------|-------------|
| x, y | `i32` | Window position in pixels, or -1 for compositor auto-placement (CW_USEDEFAULT) |
| width, height | `u32` | Window client area size in pixels |
| flags | `u32` | Reserved (pass 0) |
| out_shm_id | `*mut u32` | Receives SHM region ID |
| out_surface | `*mut *mut u32` | Receives pointer to ARGB8888 pixel buffer |
| **Returns** | `u32` | Window ID (>0) or 0 on failure |

The pixel buffer is `width * height` u32 values in ARGB8888 format. Draw directly into this buffer, then call `present()`.

### `destroy_window(channel_id, window_id, shm_id)`

Close window and release SHM resources.

### `present(channel_id, window_id, shm_id)`

Signal the compositor that the window's pixel buffer has been updated and should be recomposited.

### `poll_event(channel_id, sub_id, window_id, buf) -> u32`

Poll for window-specific events. Returns 1 if an event was written to buf (5 u32s), 0 if none.

### `set_title(channel_id, window_id, title_ptr, title_len)`

Set the window title bar text (max 12 ASCII characters — longer titles are truncated).

### `screen_size(out_w, out_h)`

Get the current screen resolution.

### `set_wallpaper(channel_id, path_ptr, path_len)`

Set the desktop wallpaper by file path (JPEG, PNG, BMP).

### `move_window(channel_id, window_id, x, y)`

Move window to absolute screen position.

### `set_menu(channel_id, window_id, menu_data, menu_len)`

Set the window's menu bar. Menu data is a binary `MenuBarDef` format built with `MenuBarBuilder`.

### `add_status_icon(channel_id, icon_id, pixels)`

Register a 16x16 ARGB8888 icon in the system menu bar tray area.

### `remove_status_icon(channel_id, icon_id)`

Remove a previously registered status icon.

### `update_menu_item(channel_id, window_id, item_id, new_flags)`

Update menu item state (enable/disable, checked/unchecked).

### `resize_shm(channel_id, window_id, old_shm_id, new_width, new_height, out_new_shm_id) -> *mut u32`

Resize the window's backing buffer. Returns new pixel buffer pointer, or null on failure. Old SHM is released.

### `tray_poll_event(channel_id, sub_id, buf) -> u32`

Poll events without window filtering — used by tray-icon applications that don't have a window.

### `set_blur_behind(channel_id, window_id, radius)`

Enable frosted-glass blur effect behind the window. `radius=0` disables. Typical values are 4-12.

### `set_clipboard(channel_id, data_ptr, data_len, format)`

Set the system clipboard contents.

| Parameter | Type | Description |
|-----------|------|-------------|
| data_ptr | `*const u8` | Pointer to clipboard data bytes |
| data_len | `u32` | Length of data in bytes |
| format | `u32` | Clipboard format: 0 = text/plain, 1 = text/uri-list |

### `get_clipboard(channel_id, sub_id, out_ptr, out_cap, out_format) -> u32`

Read the current clipboard contents. Returns the actual byte count written to `out_ptr` (0 if clipboard is empty).

| Parameter | Type | Description |
|-----------|------|-------------|
| out_ptr | `*mut u8` | Buffer to receive clipboard data |
| out_cap | `u32` | Capacity of the output buffer in bytes |
| out_format | `*mut u32` | Receives the clipboard format (0 = text/plain, 1 = text/uri-list) |
| **Returns** | `u32` | Actual byte count (0 if empty) |

### `get_window_position(channel_id, sub_id, window_id, out_x, out_y) -> u32`

Get a window's content area screen position. Returns 1 on success, 0 on failure/timeout.

| Parameter | Type | Description |
|-----------|------|-------------|
| out_x | `*mut i32` | Receives the window X position |
| out_y | `*mut i32` | Receives the window Y position |
| **Returns** | `u32` | 1 on success, 0 on failure/timeout |

### `minimize_window(channel_id, window_id)`

Minimize a window (move off-screen, save bounds for later restore).

### `set_modal_owner(channel_id, modal_window_id, owner_window_id)`

Set a window as a modal child of another window. The modal window will stay above its owner, and clicking the owner re-focuses the modal. Pass `owner_window_id=0` to clear the modal relationship.

### `set_fullscreen_capable(channel_id, window_id, auto_enter)`

Mark a window as fullscreen-capable. If `auto_enter=1`, the window immediately enters fullscreen mode.

### `request_fullscreen(channel_id, sub_id, window_id, want_direct_fb, out_width, out_height, out_stride, out_fb_ptr) -> u32`

Request fullscreen mode for a window. Returns 1 on success (response received), 0 on timeout.

| Parameter | Type | Description |
|-----------|------|-------------|
| want_direct_fb | `u32` | 1 = request direct framebuffer access (for LibGL apps), 0 = SHM compositing |
| out_width | `*mut u32` | Receives the fullscreen width |
| out_height | `*mut u32` | Receives the fullscreen height |
| out_stride | `*mut u32` | Receives the row stride in pixels |
| out_fb_ptr | `*mut usize` | Receives the direct framebuffer pointer (0 if SHM compositing mode) |
| **Returns** | `u32` | 1 on success, 0 on timeout |

### `exit_fullscreen(channel_id, window_id)`

Exit fullscreen mode for a window.

---

## Event Types

Events are 5 u32s: `[event_type, window_id, arg1, arg2, arg3]`.

The `Event` struct has the following fields:
```rust
pub struct Event {
    pub event_type: u32,
    pub window_id: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg3: u32,
}
```

| Event | Code | window_id | arg1 | arg2 | arg3 |
|-------|------|-----------|------|------|------|
| `EVT_KEY_DOWN` | 0x3001 | win | keycode | char_value | modifiers |
| `EVT_KEY_UP` | 0x3002 | win | keycode | char_value | modifiers |
| `EVT_MOUSE_DOWN` | 0x3003 | win | x | y | button |
| `EVT_MOUSE_UP` | 0x3004 | win | x | y | button |
| `EVT_MOUSE_SCROLL` | 0x3005 | win | x | y | delta_y |
| `EVT_RESIZE` | 0x3006 | win | new_width | new_height | — |
| `EVT_WINDOW_CLOSE` | 0x3007 | win | — | — | — |
| `EVT_MENU_ITEM` | 0x3008 | win | item_id | — | — |
| `EVT_STATUS_ICON_CLICK` | 0x3009 | — | icon_id | — | — |
| `EVT_MOUSE_MOVE` | 0x300A | win | x | y | — |
| `EVT_FRAME_ACK` | 0x300B | win | — | — | — |
| `EVT_FOCUS_LOST` | 0x300C | win | — | — | — |
| `EVT_FULLSCREEN_ENTER` | 0x300D | win | — | — | — |
| `EVT_FULLSCREEN_EXIT` | 0x300E | win | — | — | — |
| `EVT_NOTIFICATION_CLICK` | 0x3010 | — | notification_id | — | — |
| `EVT_NOTIFICATION_DISMISSED` | 0x3011 | — | notification_id | — | — |

Mouse coordinates are relative to the window client area.

### EVT_FRAME_ACK — VSync Callback

`EVT_FRAME_ACK` is emitted by the compositor's render thread immediately after a window's content has been composited to the display (i.e., after VSync — the VirtIO-GPU `RESOURCE_FLUSH` completion). This is the anyOS equivalent of Windows `DwmFlush` / macOS `CVDisplayLink` callbacks.

**Flow:**
1. App calls `present()` → compositor marks window dirty
2. Render thread composes all dirty windows → `flush_gpu()` (synchronous VirtIO RESOURCE_FLUSH)
3. Render thread emits `EVT_FRAME_ACK` directly to the app via `evt_chan_emit_to()`
4. App receives ACK → safe to prepare and present the next frame

**Back-pressure:** Apps should not present a new frame until they receive the ACK for the previous one. This prevents wasted rendering when the compositor hasn't caught up yet. Implement a safety timeout (e.g., 64ms) to handle rare cases where an ACK might be lost.

**Backward compatibility:** Apps that don't handle `EVT_FRAME_ACK` simply discard the event — no behavioral change. The event passes through the existing `poll_event()` filter (`event_type >= 0x3000`).

```rust
// Frame-paced rendering loop
let mut frame_pending = false;

loop {
    if let Some(event) = client.poll_event(win) {
        match event.event_type {
            0x300B => frame_pending = false,  // EVT_FRAME_ACK
            0x3007 => break,                   // EVT_WINDOW_CLOSE
            _ => { /* handle other events */ }
        }
    }

    if !frame_pending && needs_redraw {
        draw_frame(client.surface(win));
        client.present(win);
        frame_pending = true;
    }

    anyos_std::process::sleep(2);
}
```

---

## Client Wrappers

### `FullscreenInfo`

Information returned when entering fullscreen mode.

```rust
pub struct FullscreenInfo {
    pub width: u32,
    pub height: u32,
    /// Row stride in pixels.
    pub stride: u32,
    /// Direct framebuffer pointer (0 if SHM compositing mode).
    pub fb_ptr: usize,
}
```

### `CompositorClient`

High-level wrapper for windowed applications:
- `init() -> Option<Self>` — Initialize connection to compositor
- `create_window(x, y, w, h, flags) -> Option<WindowHandle>`
- `surface(win) -> &mut [u32]` — Get mutable pixel buffer
- `surface_slice(win) -> &mut [u32]` — Alias for surface
- `present(win)` — Submit full frame
- `present_rect(win, x, y, w, h)` — Submit dirty rectangle only (optimization)
- `poll_event(win) -> Option<Event>`
- `set_title(win, title)` — Max 12 ASCII characters
- `destroy_window(win)`
- `resize_window(win, new_w, new_h) -> bool` — Resize window and reallocate SHM
- `move_window(win, x, y)` — Move window to screen position
- `set_menu(win, menu)` — Set menu bar
- `set_blur_behind(win, radius)` — Frosted-glass effect
- `set_wallpaper(path)` — Set desktop wallpaper
- `show_notification(title, message, icon, timeout_ms)` — Show system notification
- `dismiss_notification(notification_id)` — Dismiss notification
- `screen_size() -> (u32, u32)` — Get screen resolution
- `set_fullscreen_capable(win, auto_enter)` — Mark window as fullscreen-capable
- `set_fullscreen_capable_vram(vram_win, auto_enter)` — Mark VRAM window as fullscreen-capable
- `request_fullscreen(win, want_direct_fb) -> Option<FullscreenInfo>` — Enter fullscreen mode
- `exit_fullscreen(win)` — Exit fullscreen mode
- `exit_fullscreen_vram(vram_win)` — Exit fullscreen mode for VRAM window

### `VramWindowHandle`

Direct VRAM-backed window for high-performance rendering:
- `id: u32` — Window ID
- `surface_ptr: *mut u32` — Raw VRAM surface pointer
- `width: u32, height: u32, stride: u32`
- `surface() -> *mut u32` — Get raw surface pointer
- `surface_slice() -> &mut [u32]` — Get surface as mutable slice
- `put_pixel(x, y, color)` — Set a single pixel

Created/destroyed via `CompositorClient`:
- `create_vram_window(x, y, w, h, flags) -> Option<VramWindowHandle>`
- `destroy_vram_window(handle)`
- `present_vram(handle)` — Present VRAM window
- `poll_event_vram(handle) -> Option<Event>`

### `TrayClient`

For windowless tray-icon applications:
- `init() -> Option<Self>` — Connect as tray client
- `add_icon(id, pixels)` — Register 16x16 tray icon
- `set_icon(id, pixels)` — Update existing icon
- `remove_icon(id)` — Remove tray icon
- `create_window(x, y, w, h, flags)` — Create popup window
- `destroy_window(handle)` — Destroy popup window
- `present(handle)` — Present popup
- `present_rect(handle, x, y, w, h)` — Present dirty rect
- `move_window(handle, x, y)` — Move popup
- `screen_size() -> (u32, u32)` — Get screen resolution
- `poll_event() -> Option<Event>`
- `show_notification(title, msg, icon, timeout_ms)` — Show notification
- `dismiss_notification(notification_id)` — Dismiss notification

### Menu Item Flags

```rust
pub const MENU_FLAG_DISABLED: u32 = 0x01;  // Greyed-out, non-clickable
pub const MENU_FLAG_SEPARATOR: u32 = 0x02; // Horizontal line separator
pub const MENU_FLAG_CHECKED: u32 = 0x04;   // Checkmark indicator
```

### `MenuBarBuilder`

Fluent API for building menu definitions:
```rust
let menu = MenuBarBuilder::new()
    .menu("File")
        .item(1, "New", 0)
        .item(2, "Open...", 0)
        .separator()
        .item(3, "Quit", 0)
    .end_menu()
    .menu("Edit")
        .item(10, "Cut", 0)
        .item(11, "Copy", 0)
        .item(12, "Paste", 0)
        .item(13, "Toggle", MENU_FLAG_CHECKED)  // checked item
    .end_menu()
    .build();
```
