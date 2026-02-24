//! Compositor access layer — reads libcompositor.dlib export table at 0x04380000.
//!
//! libanyui is a shared library (.so) that cannot link against libcompositor_client.
//! Instead, it reads the libcompositor.dlib export struct directly from its fixed
//! virtual address, just like uisys.rs reads from 0x04000000.

const LIBCOMPOSITOR_BASE: usize = 0x0438_0000;

/// Export table layout for libcompositor.dlib (matches libcompositor_client::raw).
#[repr(C)]
struct LibcompositorExports {
    magic: [u8; 4],
    version: u32,
    num_exports: u32,
    _pad: u32,

    init: extern "C" fn(out_sub_id: *mut u32) -> u32,

    create_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        flags: u32,
        out_shm_id: *mut u32,
        out_surface: *mut *mut u32,
    ) -> u32,

    destroy_window: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    present: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        window_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    set_title:
        extern "C" fn(channel_id: u32, window_id: u32, title_ptr: *const u8, title_len: u32),

    screen_size: extern "C" fn(out_w: *mut u32, out_h: *mut u32),

    set_wallpaper: extern "C" fn(channel_id: u32, path_ptr: *const u8, path_len: u32),

    move_window: extern "C" fn(channel_id: u32, window_id: u32, x: i32, y: i32),

    set_menu:
        extern "C" fn(channel_id: u32, window_id: u32, menu_data: *const u8, menu_len: u32),

    add_status_icon: extern "C" fn(channel_id: u32, icon_id: u32, pixels: *const u32),

    remove_status_icon: extern "C" fn(channel_id: u32, icon_id: u32),

    update_menu_item:
        extern "C" fn(channel_id: u32, window_id: u32, item_id: u32, new_flags: u32),

    resize_shm: extern "C" fn(
        channel_id: u32,
        window_id: u32,
        old_shm_id: u32,
        new_width: u32,
        new_height: u32,
        out_new_shm_id: *mut u32,
    ) -> *mut u32,

    tray_poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    set_blur_behind: extern "C" fn(channel_id: u32, window_id: u32, radius: u32),

    create_vram_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        flags: u32,
        out_stride: *mut u32,
        out_surface: *mut *mut u32,
    ) -> u32,

    present_rect: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32, x: u32, y: u32, w: u32, h: u32),

    set_clipboard: extern "C" fn(channel_id: u32, data_ptr: *const u8, data_len: u32, format: u32),

    get_clipboard: extern "C" fn(channel_id: u32, sub_id: u32, out_ptr: *mut u8, out_cap: u32, out_format: *mut u32) -> u32,

    show_notification: extern "C" fn(
        channel_id: u32,
        title_ptr: *const u8, title_len: u32,
        msg_ptr: *const u8, msg_len: u32,
        icon_ptr: *const u32,
        timeout_ms: u32, flags: u32,
    ),

    dismiss_notification: extern "C" fn(channel_id: u32, notification_id: u32),
}

fn exports() -> &'static LibcompositorExports {
    unsafe { &*(LIBCOMPOSITOR_BASE as *const LibcompositorExports) }
}

// ── Compositor event types (IPC protocol) ────────────────────────────

pub const EVT_KEY_DOWN: u32 = 0x3001;
pub const EVT_KEY_UP: u32 = 0x3002;
pub const EVT_MOUSE_DOWN: u32 = 0x3003;
pub const EVT_MOUSE_UP: u32 = 0x3004;
pub const EVT_MOUSE_SCROLL: u32 = 0x3005;
pub const EVT_RESIZE: u32 = 0x3006;
pub const EVT_WINDOW_CLOSE: u32 = 0x3007;
pub const EVT_MOUSE_MOVE: u32 = 0x300A;
pub const EVT_FRAME_ACK: u32 = 0x300B;

// ── High-level wrappers ──────────────────────────────────────────────

/// Initialize compositor connection. Returns channel_id (0 = failure).
pub fn init(out_sub_id: &mut u32) -> u32 {
    (exports().init)(out_sub_id)
}

/// Create a compositor window at position (x, y).
/// x/y: pixel coordinates, or -1 for compositor auto-placement (CW_USEDEFAULT).
/// Returns (window_id, shm_id, surface_ptr) or None.
pub fn create_window(
    channel_id: u32,
    sub_id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    flags: u32,
) -> Option<(u32, u32, *mut u32)> {
    let mut shm_id: u32 = 0;
    let mut surface: *mut u32 = core::ptr::null_mut();
    let window_id = (exports().create_window)(
        channel_id,
        sub_id,
        x,
        y,
        width,
        height,
        flags,
        &mut shm_id,
        &mut surface,
    );
    if window_id == 0 || surface.is_null() {
        None
    } else {
        Some((window_id, shm_id, surface))
    }
}

/// Destroy a compositor window.
pub fn destroy_window(channel_id: u32, window_id: u32, shm_id: u32) {
    (exports().destroy_window)(channel_id, window_id, shm_id);
}

/// Signal the compositor that window content has been updated.
pub fn present(channel_id: u32, window_id: u32, shm_id: u32) {
    (exports().present)(channel_id, window_id, shm_id);
}

/// Signal the compositor that a specific region of the window content was updated.
/// The compositor will only copy and recomposite the dirty rectangle.
pub fn present_rect(channel_id: u32, window_id: u32, shm_id: u32, x: u32, y: u32, w: u32, h: u32) {
    (exports().present_rect)(channel_id, window_id, shm_id, x, y, w, h);
}

/// Poll for the next event. Returns true if an event was received.
/// Buffer layout: [event_type, window_id, arg1, arg2, arg3]
pub fn poll_event(
    channel_id: u32,
    sub_id: u32,
    window_id: u32,
    buf: &mut [u32; 5],
) -> bool {
    (exports().poll_event)(channel_id, sub_id, window_id, buf) != 0
}

/// Set window title.
pub fn set_title(channel_id: u32, window_id: u32, title: &[u8]) {
    (exports().set_title)(channel_id, window_id, title.as_ptr(), title.len() as u32);
}

/// Resize a window's SHM buffer. Returns new (shm_id, surface_ptr) or None.
pub fn resize_shm(
    channel_id: u32,
    window_id: u32,
    old_shm_id: u32,
    new_width: u32,
    new_height: u32,
) -> Option<(u32, *mut u32)> {
    let mut new_shm_id: u32 = 0;
    let surface = (exports().resize_shm)(
        channel_id,
        window_id,
        old_shm_id,
        new_width,
        new_height,
        &mut new_shm_id,
    );
    if surface.is_null() {
        None
    } else {
        Some((new_shm_id, surface))
    }
}

/// Enable or disable blur-behind on a compositor window.
/// radius=0 disables blur, radius>0 enables with given kernel radius.
pub fn set_blur_behind(channel_id: u32, window_id: u32, radius: u32) {
    (exports().set_blur_behind)(channel_id, window_id, radius);
}

/// Get screen dimensions.
pub fn screen_size() -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (exports().screen_size)(&mut w, &mut h);
    (w, h)
}

/// Show a notification banner via the compositor.
pub fn show_notification(
    channel_id: u32,
    title: &[u8],
    message: &[u8],
    icon: *const u32,
    timeout_ms: u32,
    flags: u32,
) {
    (exports().show_notification)(
        channel_id,
        title.as_ptr(), title.len() as u32,
        message.as_ptr(), message.len() as u32,
        icon,
        timeout_ms, flags,
    );
}

// ── Surface helpers ──────────────────────────────────────────────────

/// Fill a rectangle on a window's SHM surface.
pub fn fill_surface_rect(
    surface: *mut u32,
    sw: u32,
    sh: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x + w as i32) as u32).min(sw);
    let y1 = ((y + h as i32) as u32).min(sh);

    for row in y0..y1 {
        let start = (row * sw + x0) as usize;
        let count = (x1 - x0) as usize;
        unsafe {
            let slice = core::slice::from_raw_parts_mut(surface.add(start), count);
            slice.fill(color);
        }
    }
}

/// Blit ARGB pixel data to a window's SHM surface.
pub fn blit_to_surface(
    surface: *mut u32,
    sw: u32,
    sh: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    pixels: &[u32],
) {
    for row in 0..h {
        let dy = y + row as i32;
        if dy < 0 || dy >= sh as i32 {
            continue;
        }

        let src_start = (row * w) as usize;
        if src_start >= pixels.len() {
            continue;
        }

        let x0 = x.max(0);
        let x1 = (x + w as i32).min(sw as i32);
        if x0 >= x1 {
            continue;
        }

        let skip = (x0 - x) as usize;
        let count = (x1 - x0) as usize;
        if src_start + skip + count > pixels.len() {
            continue;
        }

        let dst_off = (dy as u32 * sw + x0 as u32) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(
                pixels.as_ptr().add(src_start + skip),
                surface.add(dst_off),
                count,
            );
        }
    }
}
