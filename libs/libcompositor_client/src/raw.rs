//! Raw FFI bindings to libcompositor.dlib export table.
//!
//! libcompositor.dlib is loaded at virtual address 0x0438_0000.
//! The export struct starts at byte 0 of the DLIB image.

const LIBCOMPOSITOR_BASE: usize = 0x0438_0000;

#[repr(C)]
pub struct LibcompositorExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,

    pub init: extern "C" fn(out_sub_id: *mut u32) -> u32,

    pub create_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        width: u32,
        height: u32,
        flags: u32,
        out_shm_id: *mut u32,
        out_surface: *mut *mut u32,
    ) -> u32,

    pub destroy_window: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    pub present: extern "C" fn(channel_id: u32, window_id: u32, shm_id: u32),

    pub poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        window_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    pub set_title:
        extern "C" fn(channel_id: u32, window_id: u32, title_ptr: *const u8, title_len: u32),

    pub screen_size: extern "C" fn(out_w: *mut u32, out_h: *mut u32),

    pub set_wallpaper: extern "C" fn(pixels: *const u32, width: u32, height: u32),

    pub move_window: extern "C" fn(channel_id: u32, window_id: u32, x: i32, y: i32),

    pub set_menu:
        extern "C" fn(channel_id: u32, window_id: u32, menu_data: *const u8, menu_len: u32),

    pub add_status_icon: extern "C" fn(channel_id: u32, icon_id: u32, pixels: *const u32),

    pub remove_status_icon: extern "C" fn(channel_id: u32, icon_id: u32),

    pub update_menu_item:
        extern "C" fn(channel_id: u32, window_id: u32, item_id: u32, new_flags: u32),

    pub resize_shm: extern "C" fn(
        channel_id: u32,
        window_id: u32,
        old_shm_id: u32,
        new_width: u32,
        new_height: u32,
        out_new_shm_id: *mut u32,
    ) -> *mut u32,

    pub tray_poll_event: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
        buf: *mut [u32; 5],
    ) -> u32,

    pub set_blur_behind: extern "C" fn(channel_id: u32, window_id: u32, radius: u32),
}

/// Get a reference to the libcompositor export table.
pub fn exports() -> &'static LibcompositorExports {
    unsafe { &*(LIBCOMPOSITOR_BASE as *const LibcompositorExports) }
}
