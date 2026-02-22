//! Export table for libcompositor.dlib.
//!
//! Provides IPC-based window management via the compositor event channel.
//! Apps create windows backed by shared memory, draw directly to the surface,
//! and signal the compositor to composite via `present()`.

use crate::syscall;

// IPC protocol constants (must match compositor's ipc_protocol.rs)
const CMD_CREATE_WINDOW: u32 = 0x1001;
const CMD_DESTROY_WINDOW: u32 = 0x1002;
const CMD_PRESENT: u32 = 0x1003;
const CMD_SET_TITLE: u32 = 0x1004;
const CMD_MOVE_WINDOW: u32 = 0x1005;
const CMD_SET_MENU: u32 = 0x1006;
const CMD_ADD_STATUS_ICON: u32 = 0x1007;
const CMD_REMOVE_STATUS_ICON: u32 = 0x1008;
const CMD_UPDATE_MENU_ITEM: u32 = 0x1009;
const CMD_RESIZE_SHM: u32 = 0x100B;
const CMD_REGISTER_SUB: u32 = 0x100C;
const CMD_SET_BLUR_BEHIND: u32 = 0x100E;
const CMD_SET_WALLPAPER: u32 = 0x100F;
const CMD_CREATE_VRAM_WINDOW: u32 = 0x1010;
const RESP_WINDOW_CREATED: u32 = 0x2001;
const RESP_VRAM_WINDOW_CREATED: u32 = 0x2004;
const RESP_VRAM_WINDOW_FAILED: u32 = 0x2005;

const NUM_EXPORTS: u32 = 18;

#[repr(C)]
pub struct LibcompositorExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,

    /// Initialize compositor connection.
    /// Returns channel_id (0 on failure). Fills out_sub_id.
    pub init: extern "C" fn(out_sub_id: *mut u32) -> u32,

    /// Create a window.
    /// Returns window_id (0 on failure). Fills out_shm_id and out_surface.
    pub create_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        width: u32,
        height: u32,
        flags: u32,
        out_shm_id: *mut u32,
        out_surface: *mut *mut u32,
    ) -> u32,

    /// Destroy a window.
    pub destroy_window: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    /// Signal that window content has been updated.
    pub present: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    /// Poll for a window event. Returns 1 if event available, 0 if not.
    pub poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        window_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    /// Set window title (up to 12 ASCII chars).
    pub set_title:
        extern "C" fn(channel_id: u32, window_id: u32, title_ptr: *const u8, title_len: u32),

    /// Get screen dimensions.
    pub screen_size: extern "C" fn(out_w: *mut u32, out_h: *mut u32),

    /// Set desktop wallpaper by file path (via compositor IPC).
    pub set_wallpaper: extern "C" fn(channel_id: u32, path_ptr: *const u8, path_len: u32),

    /// Move a window to a new position.
    pub move_window: extern "C" fn(channel_id: u32, window_id: u32, x: i32, y: i32),

    /// Set a window's menu bar definition via SHM.
    /// menu_data points to a flat binary MenuBarDef, menu_len is its byte length.
    pub set_menu:
        extern "C" fn(channel_id: u32, window_id: u32, menu_data: *const u8, menu_len: u32),

    /// Register a 16x16 ARGB status icon in the menubar.
    pub add_status_icon: extern "C" fn(channel_id: u32, icon_id: u32, pixels: *const u32),

    /// Remove a status icon from the menubar.
    pub remove_status_icon: extern "C" fn(channel_id: u32, icon_id: u32),

    /// Update a menu item's flags (enable/disable/check).
    pub update_menu_item:
        extern "C" fn(channel_id: u32, window_id: u32, item_id: u32, new_flags: u32),

    /// Resize a window's SHM buffer (creates new SHM, notifies compositor).
    /// Returns the new SHM surface pointer (null on failure).
    pub resize_shm: extern "C" fn(
        channel_id: u32,
        window_id: u32,
        old_shm_id: u32,
        new_width: u32,
        new_height: u32,
        out_new_shm_id: *mut u32,
    ) -> *mut u32,

    /// Poll for any event (unfiltered — no window_id check).
    /// Returns 1 if event available, 0 if not.
    pub tray_poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    /// Enable/disable blur-behind on a window.
    /// radius=0 disables, radius>0 enables with given blur radius.
    pub set_blur_behind: extern "C" fn(channel_id: u32, window_id: u32, radius: u32),

    /// Create a VRAM-direct window (app renders directly to GPU VRAM).
    /// Returns window_id (0 on failure). Fills out_stride, out_surface, out_vram_offset.
    /// stride = pitch in pixels (not bytes) — use this as row stride.
    /// surface = pointer to mapped VRAM region (app writes pixels here).
    /// If GPU not available or VRAM exhausted, returns 0 (app should fall back to SHM).
    pub create_vram_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        width: u32,
        height: u32,
        flags: u32,
        out_stride: *mut u32,
        out_surface: *mut *mut u32,
    ) -> u32,

    /// Signal that a rectangular region of window content has been updated.
    /// Only the dirty rect (x, y, w, h) is copied to the compositor layer.
    pub present_rect: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32, x: u32, y: u32, w: u32, h: u32),
}

#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static LIBCOMPOSITOR_EXPORTS: LibcompositorExports = LibcompositorExports {
    magic: *b"DLIB",
    version: 1,
    num_exports: NUM_EXPORTS,
    _pad: 0,
    init: export_init,
    create_window: export_create_window,
    destroy_window: export_destroy_window,
    present: export_present,
    poll_event: export_poll_event,
    set_title: export_set_title,
    screen_size: export_screen_size,
    set_wallpaper: export_set_wallpaper,
    move_window: export_move_window,
    set_menu: export_set_menu,
    add_status_icon: export_add_status_icon,
    remove_status_icon: export_remove_status_icon,
    update_menu_item: export_update_menu_item,
    resize_shm: export_resize_shm,
    tray_poll_event: export_tray_poll_event,
    set_blur_behind: export_set_blur_behind,
    create_vram_window: export_create_vram_window,
    present_rect: export_present_rect,
};

// ── Export Implementations ───────────────────────────────────────────────────

extern "C" fn export_init(out_sub_id: *mut u32) -> u32 {
    // Find/create the "compositor" event channel
    let name = b"compositor";
    let channel_id = syscall::evt_chan_create(name.as_ptr(), name.len() as u32);
    if channel_id == 0 {
        return 0;
    }
    // Subscribe to receive responses and events
    let sub_id = syscall::evt_chan_subscribe(channel_id, 0);
    unsafe {
        *out_sub_id = sub_id;
    }

    // Register our subscription ID with the compositor so it can use
    // targeted (unicast) event delivery instead of broadcasting to all apps.
    let tid = syscall::get_tid();
    let cmd: [u32; 5] = [CMD_REGISTER_SUB, tid, sub_id, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);

    channel_id
}

extern "C" fn export_create_window(
    channel_id: u32,
    sub_id: u32,
    width: u32,
    height: u32,
    flags: u32,
    out_shm_id: *mut u32,
    out_surface: *mut *mut u32,
) -> u32 {
    // Create SHM region for the window surface (content area only)
    let shm_size = width * height * 4;
    let shm_id = syscall::shm_create(shm_size);
    if shm_id == 0 {
        return 0;
    }

    // Map SHM into our address space
    let shm_addr = syscall::shm_map(shm_id);
    if shm_addr == 0 {
        syscall::shm_destroy(shm_id);
        return 0;
    }

    // Get our TID for the create command
    let tid = syscall::get_tid();

    // Send CREATE_WINDOW command to compositor
    // [CMD, app_tid, width, height, (shm_id << 16) | flags]
    let cmd: [u32; 5] = [
        CMD_CREATE_WINDOW,
        tid,
        width,
        height,
        (shm_id << 16) | (flags & 0xFFFF),
    ];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Poll for RESP_WINDOW_CREATED — drain ALL queued events per iteration
    // to avoid stalling behind mouse/keyboard events for other windows.
    let mut response = [0u32; 5];
    for _ in 0..100 {
        // Drain entire queue looking for our response
        while syscall::evt_chan_poll(channel_id, sub_id, &mut response) {
            if response[0] == RESP_WINDOW_CREATED && response[3] == tid {
                let window_id = response[1];
                unsafe {
                    *out_shm_id = shm_id;
                    *out_surface = shm_addr as *mut u32;
                }
                return window_id;
            }
            // Not our response — discard and try next
        }
        syscall::sleep(5);
    }

    // Timeout — clean up
    syscall::shm_unmap(shm_id);
    syscall::shm_destroy(shm_id);
    0
}

extern "C" fn export_destroy_window(channel_id: u32, window_id: u32, shm_id: u32) {
    let cmd: [u32; 5] = [CMD_DESTROY_WINDOW, window_id, 0, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Clean up SHM
    if shm_id > 0 {
        syscall::shm_unmap(shm_id);
        syscall::shm_destroy(shm_id);
    }
}

extern "C" fn export_present(channel_id: u32, window_id: u32, shm_id: u32) {
    let cmd: [u32; 5] = [CMD_PRESENT, window_id, shm_id, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_present_rect(channel_id: u32, window_id: u32, shm_id: u32, x: u32, y: u32, w: u32, h: u32) {
    // Pack dirty rect: cmd[3] = (x << 16) | y, cmd[4] = (w << 16) | h
    let cmd: [u32; 5] = [CMD_PRESENT, window_id, shm_id, (x << 16) | y, (w << 16) | h];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_poll_event(
    channel_id: u32,
    sub_id: u32,
    window_id: u32,
    buf: *mut [u32; 5],
) -> u32 {
    let mut tmp = [0u32; 5];
    // Drain the subscription queue until we find an event for our window
    // or a broadcast event (theme change, etc.).
    // Non-matching events (for other windows) are discarded from OUR copy
    // of the queue — each subscriber has an independent queue, so this is safe.
    while syscall::evt_chan_poll(channel_id, sub_id, &mut tmp) {
        if tmp[0] >= 0x3000 && tmp[1] == window_id {
            // Window-specific event (mouse, key, resize, etc.)
            unsafe { *buf = tmp; }
            return 1;
        } else if tmp[0] < 0x1000 {
            // Broadcast event (theme change, etc.) — no window_id filter
            unsafe { *buf = tmp; }
            return 1;
        }
        // Not for our window — discard and try the next event
    }
    0
}

extern "C" fn export_set_title(
    channel_id: u32,
    window_id: u32,
    title_ptr: *const u8,
    title_len: u32,
) {
    // Pack title bytes into 3 u32 words
    let mut packed = [0u32; 3];
    let len = (title_len as usize).min(12);
    for i in 0..len {
        let byte = unsafe { *title_ptr.add(i) };
        packed[i / 4] |= (byte as u32) << ((i % 4) * 8);
    }
    let cmd: [u32; 5] = [CMD_SET_TITLE, window_id, packed[0], packed[1], packed[2]];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_screen_size(out_w: *mut u32, out_h: *mut u32) {
    syscall::screen_size(out_w, out_h);
}

extern "C" fn export_set_wallpaper(channel_id: u32, path_ptr: *const u8, path_len: u32) {
    if path_ptr.is_null() || path_len == 0 || path_len > 255 {
        return;
    }

    // Create temp SHM and copy path as null-terminated string
    let shm_size = path_len + 1; // +1 for null terminator
    let shm_id = syscall::shm_create(shm_size);
    if shm_id == 0 {
        return;
    }
    let shm_addr = syscall::shm_map(shm_id);
    if shm_addr == 0 {
        syscall::shm_destroy(shm_id);
        return;
    }

    // Copy path into SHM with null terminator
    let dst = shm_addr as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(path_ptr, dst, path_len as usize);
        *dst.add(path_len as usize) = 0; // null terminator
    }

    // Send CMD_SET_WALLPAPER: [CMD, shm_id, 0, 0, 0]
    let cmd: [u32; 5] = [CMD_SET_WALLPAPER, shm_id, 0, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Wait for compositor to read the SHM, then free it
    syscall::sleep(32);
    syscall::shm_unmap(shm_id);
    syscall::shm_destroy(shm_id);
}

extern "C" fn export_move_window(channel_id: u32, window_id: u32, x: i32, y: i32) {
    let cmd: [u32; 5] = [CMD_MOVE_WINDOW, window_id, x as u32, y as u32, 0];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_set_menu(
    channel_id: u32,
    window_id: u32,
    menu_data: *const u8,
    menu_len: u32,
) {
    if menu_data.is_null() || menu_len == 0 || menu_len > 4096 {
        return;
    }

    // Create temp SHM and copy menu data into it
    let shm_id = syscall::shm_create(menu_len);
    if shm_id == 0 {
        return;
    }
    let shm_addr = syscall::shm_map(shm_id);
    if shm_addr == 0 {
        syscall::shm_destroy(shm_id);
        return;
    }

    // Copy menu data into SHM
    let dst = shm_addr as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(menu_data, dst, menu_len as usize);
    }

    // Send CMD_SET_MENU: [CMD, window_id, shm_id, 0, 0]
    let cmd: [u32; 5] = [CMD_SET_MENU, window_id, shm_id, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Wait for compositor to read the SHM, then free it.
    // 32ms ≈ 2 compositor frames — enough for the compositor to process.
    syscall::sleep(32);
    syscall::shm_unmap(shm_id);
    syscall::shm_destroy(shm_id);
}

extern "C" fn export_add_status_icon(channel_id: u32, icon_id: u32, pixels: *const u32) {
    if pixels.is_null() {
        return;
    }

    // Create temp SHM for 16x16 ARGB (256 pixels = 1024 bytes)
    let shm_id = syscall::shm_create(1024);
    if shm_id == 0 {
        return;
    }
    let shm_addr = syscall::shm_map(shm_id);
    if shm_addr == 0 {
        syscall::shm_destroy(shm_id);
        return;
    }

    // Copy pixel data into SHM
    let dst = shm_addr as *mut u32;
    unsafe {
        core::ptr::copy_nonoverlapping(pixels, dst, 256);
    }

    // Send CMD_ADD_STATUS_ICON: [CMD, app_tid, icon_id, shm_id, 0]
    let tid = syscall::get_tid();
    let cmd: [u32; 5] = [CMD_ADD_STATUS_ICON, tid, icon_id, shm_id, 0];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Wait for compositor to read the SHM, then free it
    syscall::sleep(32);
    syscall::shm_unmap(shm_id);
    syscall::shm_destroy(shm_id);
}

extern "C" fn export_remove_status_icon(channel_id: u32, icon_id: u32) {
    let tid = syscall::get_tid();
    let cmd: [u32; 5] = [CMD_REMOVE_STATUS_ICON, tid, icon_id, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_update_menu_item(
    channel_id: u32,
    window_id: u32,
    item_id: u32,
    new_flags: u32,
) {
    let cmd: [u32; 5] = [CMD_UPDATE_MENU_ITEM, window_id, item_id, new_flags, 0];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_resize_shm(
    channel_id: u32,
    window_id: u32,
    old_shm_id: u32,
    new_width: u32,
    new_height: u32,
    out_new_shm_id: *mut u32,
) -> *mut u32 {
    // Create new SHM region for the resized content area
    let shm_size = new_width * new_height * 4;
    let new_shm_id = syscall::shm_create(shm_size);
    if new_shm_id == 0 {
        return core::ptr::null_mut();
    }

    // Map new SHM
    let new_shm_addr = syscall::shm_map(new_shm_id);
    if new_shm_addr == 0 {
        syscall::shm_destroy(new_shm_id);
        return core::ptr::null_mut();
    }

    // Tell compositor about the new SHM
    let cmd: [u32; 5] = [CMD_RESIZE_SHM, window_id, new_shm_id, new_width, new_height];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Unmap and destroy old SHM
    if old_shm_id > 0 {
        syscall::shm_unmap(old_shm_id);
        syscall::shm_destroy(old_shm_id);
    }

    unsafe {
        *out_new_shm_id = new_shm_id;
    }
    new_shm_addr as *mut u32
}

extern "C" fn export_tray_poll_event(
    channel_id: u32,
    sub_id: u32,
    buf: *mut [u32; 5],
) -> u32 {
    let mut tmp = [0u32; 5];
    if syscall::evt_chan_poll(channel_id, sub_id, &mut tmp) {
        unsafe { *buf = tmp; }
        1
    } else {
        0
    }
}

extern "C" fn export_set_blur_behind(channel_id: u32, window_id: u32, radius: u32) {
    let cmd: [u32; 5] = [CMD_SET_BLUR_BEHIND, window_id, radius, 0, 0];
    syscall::evt_chan_emit(channel_id, &cmd);
}

extern "C" fn export_create_vram_window(
    channel_id: u32,
    sub_id: u32,
    width: u32,
    height: u32,
    flags: u32,
    out_stride: *mut u32,
    out_surface: *mut *mut u32,
) -> u32 {
    let tid = syscall::get_tid();

    // Send CMD_CREATE_VRAM_WINDOW to compositor
    let cmd: [u32; 5] = [CMD_CREATE_VRAM_WINDOW, tid, width, height, flags];
    syscall::evt_chan_emit(channel_id, &cmd);

    // Poll for response
    let mut response = [0u32; 5];
    for _ in 0..100 {
        while syscall::evt_chan_poll(channel_id, sub_id, &mut response) {
            if response[3] == tid {
                if response[0] == RESP_VRAM_WINDOW_CREATED {
                    let window_id = response[1];
                    let stride_pixels = response[2];
                    let surface_va = response[4];

                    // The compositor already mapped VRAM into our address space.
                    // surface_va is the ready-to-use virtual address.
                    let surface_ptr = surface_va as *mut u32;

                    unsafe {
                        *out_stride = stride_pixels;
                        *out_surface = surface_ptr;
                    }
                    return window_id;
                } else if response[0] == RESP_VRAM_WINDOW_FAILED {
                    return 0; // VRAM allocation failed
                }
            }
            // Not our response — discard
        }
        syscall::sleep(5);
    }

    0 // Timeout
}
