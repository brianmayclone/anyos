# anyOS Compositor Library (libcompositor) API Reference

The **libcompositor** DLL provides IPC-based window management for GUI applications. Windows are backed by shared memory (SHM) pixel buffers and communicate with the compositor process via an event channel.

**DLL Address:** `0x04380000`
**Version:** 1
**Exports:** 16
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

let mut client = CompositorClient::connect().expect("compositor not running");
let win = client.create_window(400, 300, 0).expect("create failed");

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
5. Compositor composites all windows to framebuffer on next vsync
6. Events (keyboard, mouse, resize, close) flow back via channel

---

## Functions

### `init(out_sub_id) -> channel_id`

Connect to the compositor. Returns channel ID for all subsequent calls, fills `out_sub_id` with event subscription ID. Returns 0 on failure (compositor not running).

### `create_window(channel_id, sub_id, width, height, flags, out_shm_id, out_surface) -> window_id`

Create a new window.

| Parameter | Type | Description |
|-----------|------|-------------|
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

Set the window title bar text (max 64 characters).

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

Enable frosted-glass blur effect behind the window. `radius=0` disables.

---

## Event Types

Events are 5 u32s: `[event_type, p1, p2, p3, p4]`.

| Event | Code | p1 | p2 | p3 | p4 |
|-------|------|----|----|----|-----|
| `EVT_KEY_DOWN` | 0x3001 | keycode | char_value | modifiers | — |
| `EVT_KEY_UP` | 0x3002 | keycode | char_value | modifiers | — |
| `EVT_MOUSE_DOWN` | 0x3003 | x | y | button | — |
| `EVT_MOUSE_UP` | 0x3004 | x | y | button | — |
| `EVT_MOUSE_SCROLL` | 0x3005 | x | y | delta_y | — |
| `EVT_RESIZE` | 0x3006 | new_width | new_height | — | — |
| `EVT_WINDOW_CLOSE` | 0x3007 | — | — | — | — |
| `EVT_MENU_ITEM` | 0x3008 | item_id | — | — | — |
| `EVT_STATUS_ICON_CLICK` | 0x3009 | icon_id | — | — | — |
| `EVT_MOUSE_MOVE` | 0x300A | x | y | — | — |

Mouse coordinates are relative to the window client area.

---

## Client Wrappers

### `CompositorClient`

High-level wrapper for windowed applications:
- `connect() -> Option<Self>` — Initialize connection
- `create_window(w, h, flags) -> Option<WindowId>`
- `surface(win) -> &mut [u32]` — Get mutable pixel buffer
- `present(win)` — Submit frame
- `poll_event(win) -> Option<Event>`
- `set_title(win, title)`
- `destroy_window(win)`
- `resize(win, w, h) -> bool`
- `set_menu(win, menu)`
- `set_blur_behind(win, radius)`

### `TrayClient`

For windowless tray-icon applications:
- `connect() -> Option<Self>`
- `add_icon(id, pixels)`
- `remove_icon(id)`
- `poll_event() -> Option<Event>`

### `MenuBarBuilder`

Fluent API for building menu definitions:
```rust
let menu = MenuBarBuilder::new()
    .menu("File")
        .item(1, "New", "Cmd+N")
        .item(2, "Open...", "Cmd+O")
        .separator()
        .item(3, "Quit", "Cmd+Q")
    .menu("Edit")
        .item(10, "Cut", "Cmd+X")
        .item(11, "Copy", "Cmd+C")
        .item(12, "Paste", "Cmd+V")
    .build();
```
