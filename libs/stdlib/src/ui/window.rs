//! Window manager — create, draw, events, present.
//!
//! Uses the userspace compositor via libcompositor.dlib (at 0x0438_0000).
//! Drawing functions operate directly on shared-memory pixel buffers.
//!
//! The external "window ID" returned by `create()` is actually a pointer to a
//! `WinSurface` struct, which is compatible with the uisys DLL's surface-mode
//! rendering (win >= 0x0100_0000 → direct pixel writes via librender).

use crate::raw::*;
use alloc::boxed::Box;
use alloc::vec::Vec;

// ── Public constants (unchanged API) ────────────────────────────────────────

pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;
pub const EVENT_MOUSE_SCROLL: u32 = 7;
pub const EVENT_WINDOW_CLOSE: u32 = 8;
pub const EVENT_MENU_ITEM: u32 = 9;

pub const WIN_FLAG_BORDERLESS: u32 = 0x01;
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;
pub const WIN_FLAG_NO_CLOSE: u32 = 0x08;
pub const WIN_FLAG_NO_MINIMIZE: u32 = 0x10;
pub const WIN_FLAG_NO_MAXIMIZE: u32 = 0x20;

/// System font — regular weight (SF Pro).
pub const FONT_REGULAR: u16 = 0;
/// System font — bold weight.
pub const FONT_BOLD: u16 = 1;
/// System font — thin weight.
pub const FONT_THIN: u16 = 2;
/// System font — italic.
pub const FONT_ITALIC: u16 = 3;

// ── Compositor DLIB bindings (libcompositor.dlib at 0x0438_0000) ─────────────

const LIBCOMPOSITOR_BASE: usize = 0x0438_0000;

#[repr(C)]
struct DllExports {
    magic: [u8; 4],
    version: u32,
    num_exports: u32,
    _pad: u32,
    init: extern "C" fn(out_sub_id: *mut u32) -> u32,
    create_window: extern "C" fn(
        channel_id: u32,
        sub_id: u32,
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
}

#[inline(always)]
fn dll() -> &'static DllExports {
    unsafe { &*(LIBCOMPOSITOR_BASE as *const DllExports) }
}

// ── Internal state ──────────────────────────────────────────────────────────

/// Surface descriptor matching uisys DLL's `draw::WinSurface` layout.
/// When uisys receives the pointer to this struct as a "window ID", it
/// detects `win >= 0x0100_0000` and uses librender for direct rendering.
#[repr(C)]
pub struct WinSurface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

struct WinInfo {
    comp_id: u32,
    shm_id: u32,
    surface: WinSurface,
    ext_id: u32, // = &self.surface as u32 — external "window ID"
}

struct CompState {
    channel_id: u32,
    sub_id: u32,
    windows: Vec<Box<WinInfo>>,
}

// Single-threaded user programs — static mut is safe.
static mut COMP: Option<CompState> = None;

fn ensure_init() -> bool {
    unsafe {
        if COMP.is_some() {
            return true;
        }
        let mut sub_id: u32 = 0;
        let channel_id = (dll().init)(&mut sub_id);
        if channel_id == 0 {
            return false;
        }
        COMP = Some(CompState {
            channel_id,
            sub_id,
            windows: Vec::new(),
        });
        true
    }
}

#[inline(always)]
fn state() -> &'static mut CompState {
    unsafe { COMP.as_mut().unwrap() }
}

fn find_win(ext_id: u32) -> Option<&'static WinInfo> {
    let st = unsafe { COMP.as_ref()? };
    st.windows.iter().find(|w| w.ext_id == ext_id).map(|b| &**b)
}

fn find_win_mut(ext_id: u32) -> Option<&'static mut WinInfo> {
    let st = unsafe { COMP.as_mut()? };
    st.windows
        .iter_mut()
        .find(|w| w.ext_id == ext_id)
        .map(|b| &mut **b)
}

// ── libfont DLL access (at fixed address 0x04200000) ────────────────────────

const LIBFONT_BASE: usize = 0x0420_0000;

#[repr(C)]
struct LibfontExportsPartial {
    _magic: [u8; 4],
    _version: u32,
    _num_exports: u32,
    _pad: u32,
    // offset 16
    _init: usize,
    // offset 24
    load_font: extern "C" fn(*const u8, u32) -> u32,
    // offset 32
    unload_font: extern "C" fn(u32),
    // offset 40
    measure_string: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32),
    // offset 48
    draw_string_buf: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u16, *const u8, u32),
    // offset 56
    _line_height: usize,
    // offset 64
    _set_subpixel: usize,
}

#[inline(always)]
fn libfont() -> &'static LibfontExportsPartial {
    unsafe { &*(LIBFONT_BASE as *const LibfontExportsPartial) }
}

// ── 8x16 bitmap font for draw_text / draw_text_mono ────────────────────────

const FONT_W: u32 = 8;
const FONT_H: u32 = 16;
static MONO_FONT: &[u8] = include_bytes!("font_8x16.bin");

// ── Public API ──────────────────────────────────────────────────────────────

/// Create a window. Returns a window handle (surface pointer as u32), or u32::MAX on failure.
pub fn create(title: &str, x: u16, y: u16, w: u16, h: u16) -> u32 {
    create_ex(title, x, y, w, h, 0)
}

/// Create a window with flags. Returns a window handle or u32::MAX.
pub fn create_ex(title: &str, x: u16, y: u16, w: u16, h: u16, flags: u32) -> u32 {
    if !ensure_init() {
        return u32::MAX;
    }
    let st = state();
    let mut shm_id: u32 = 0;
    let mut surface_ptr: *mut u32 = core::ptr::null_mut();
    let comp_id = (dll().create_window)(
        st.channel_id,
        st.sub_id,
        w as u32,
        h as u32,
        flags,
        &mut shm_id,
        &mut surface_ptr,
    );
    if comp_id == 0 || surface_ptr.is_null() {
        return u32::MAX;
    }

    // Set title
    let bytes = title.as_bytes();
    (dll().set_title)(st.channel_id, comp_id, bytes.as_ptr(), bytes.len() as u32);

    // Move to requested position
    (dll().move_window)(st.channel_id, comp_id, x as i32, y as i32);

    // Allocate stable WinInfo on heap (Box ensures address doesn't move)
    let mut info = Box::new(WinInfo {
        comp_id,
        shm_id,
        surface: WinSurface {
            pixels: surface_ptr,
            width: w as u32,
            height: h as u32,
        },
        ext_id: 0,
    });
    let ext_id = &info.surface as *const WinSurface as u32;
    info.ext_id = ext_id;
    st.windows.push(info);
    ext_id
}

/// Destroy a window.
pub fn destroy(window_id: u32) -> u32 {
    if unsafe { COMP.is_none() } {
        return u32::MAX;
    }
    let st = state();
    if let Some(idx) = st.windows.iter().position(|w| w.ext_id == window_id) {
        let win = &st.windows[idx];
        (dll().destroy_window)(st.channel_id, win.comp_id, win.shm_id);
        st.windows.swap_remove(idx);
        0
    } else {
        u32::MAX
    }
}

/// Set window title.
pub fn set_title(window_id: u32, title: &str) -> u32 {
    if unsafe { COMP.is_none() } {
        return u32::MAX;
    }
    let st = state();
    if let Some(win) = st.windows.iter().find(|w| w.ext_id == window_id) {
        let bytes = title.as_bytes();
        (dll().set_title)(st.channel_id, win.comp_id, bytes.as_ptr(), bytes.len() as u32);
        0
    } else {
        u32::MAX
    }
}

/// Poll window event. Returns 1 if event received, 0 if none.
/// Event format: [type:u32, p1:u32, p2:u32, p3:u32, p4:u32]
pub fn get_event(window_id: u32, event: &mut [u32; 5]) -> u32 {
    if unsafe { COMP.is_none() } {
        return 0;
    }
    let st = state();
    let comp_id = match st.windows.iter().find(|w| w.ext_id == window_id) {
        Some(w) => w.comp_id,
        None => return 0,
    };
    let mut buf = [0u32; 5];
    let ok = (dll().poll_event)(st.channel_id, st.sub_id, comp_id, &mut buf);
    if ok != 0 {
        // Translate: compositor [type, win_id, a1, a2, a3] → old [type, a1, a2, a3, 0]
        event[0] = match buf[0] {
            0x3001 => EVENT_KEY_DOWN,
            0x3002 => EVENT_KEY_UP,
            0x3003 => EVENT_MOUSE_DOWN,
            0x3004 => EVENT_MOUSE_UP,
            0x3005 => EVENT_MOUSE_SCROLL,
            0x3006 => EVENT_RESIZE,
            0x3007 => EVENT_WINDOW_CLOSE,
            0x3008 => EVENT_MENU_ITEM,
            _ => buf[0],
        };
        event[1] = buf[2];
        event[2] = buf[3];
        event[3] = buf[4];
        event[4] = 0;

        // Update stored dimensions on resize — must reallocate SHM
        if buf[0] == 0x3006 {
            let new_w = buf[2];
            let new_h = buf[3];
            if let Some(win) = find_win_mut(window_id) {
                // Only reallocate if dimensions actually changed
                if new_w != win.surface.width || new_h != win.surface.height {
                    let mut new_shm_id: u32 = 0;
                    let new_ptr = (dll().resize_shm)(
                        st.channel_id,
                        win.comp_id,
                        win.shm_id,
                        new_w,
                        new_h,
                        &mut new_shm_id,
                    );
                    if !new_ptr.is_null() && new_shm_id != 0 {
                        win.shm_id = new_shm_id;
                        win.surface.pixels = new_ptr;
                    }
                }
                win.surface.width = new_w;
                win.surface.height = new_h;
            }
        }
        1
    } else {
        0
    }
}

/// Fill a rectangle in a window.
pub fn fill_rect(window_id: u32, x: i16, y: i16, w: u16, h: u16, color: u32) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let sw = win.surface.width as i32;
    let sh = win.surface.height as i32;
    let x0 = (x as i32).max(0) as u32;
    let y0 = (y as i32).max(0) as u32;
    let x1 = (x as i32 + w as i32).max(0).min(sw) as u32;
    let y1 = (y as i32 + h as i32).max(0).min(sh) as u32;
    if x0 >= x1 || y0 >= y1 {
        return 0;
    }
    let stride = win.surface.width;
    for row in y0..y1 {
        let start = (row * stride + x0) as usize;
        let count = (x1 - x0) as usize;
        unsafe {
            let slice = core::slice::from_raw_parts_mut(win.surface.pixels.add(start), count);
            slice.fill(color);
        }
    }
    0
}

/// Draw text using the system TTF font (sfpro.ttf at 13px).
pub fn draw_text(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let pixel_count = (win.surface.width as usize) * (win.surface.height as usize);
    let buf = unsafe { core::slice::from_raw_parts_mut(win.surface.pixels, pixel_count) };
    font_render_buf(
        0, // system font
        13, // default size
        buf,
        win.surface.width,
        win.surface.height,
        x as i32,
        y as i32,
        color,
        text,
    )
}

/// Draw text using the 8x16 monospace bitmap font.
pub fn draw_text_mono(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let sw = win.surface.width as i32;
    let sh = win.surface.height as i32;
    let stride = win.surface.width;
    let mut cx = x as i32;
    let cy = y as i32;
    for &ch in text.as_bytes() {
        let c = ch as u32;
        if c >= 32 && c <= 126 {
            let idx = (c - 32) as usize;
            let glyph_off = idx * FONT_H as usize;
            for row in 0..FONT_H as i32 {
                let py = cy + row;
                if py < 0 || py >= sh {
                    continue;
                }
                let byte = MONO_FONT[glyph_off + row as usize];
                for col in 0..FONT_W as i32 {
                    if byte & (0x80 >> col) != 0 {
                        let px = cx + col;
                        if px >= 0 && px < sw {
                            unsafe {
                                *win.surface
                                    .pixels
                                    .add((py as u32 * stride + px as u32) as usize) = color;
                            }
                        }
                    }
                }
            }
        }
        cx += FONT_W as i32;
    }
    0
}

/// Present (flush) window drawing to compositor.
pub fn present(window_id: u32) -> u32 {
    if unsafe { COMP.is_none() } {
        return u32::MAX;
    }
    let st = state();
    if let Some(win) = st.windows.iter().find(|w| w.ext_id == window_id) {
        (dll().present)(st.channel_id, win.comp_id, win.shm_id);
        0
    } else {
        u32::MAX
    }
}

/// Get window content size. Returns (width, height).
pub fn get_size(window_id: u32) -> Option<(u32, u32)> {
    find_win(window_id).map(|w| (w.surface.width, w.surface.height))
}

/// Blit ARGB pixel data to window content surface.
pub fn blit(window_id: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let sw = win.surface.width as i32;
    let sh = win.surface.height as i32;
    let stride = win.surface.width;
    for row in 0..h as i32 {
        let py = y as i32 + row;
        if py < 0 || py >= sh {
            continue;
        }
        let x0 = (x as i32).max(0);
        let x1 = (x as i32 + w as i32).min(sw);
        if x0 >= x1 {
            continue;
        }
        let src_off = (row * w as i32 + (x0 - x as i32)) as usize;
        let dst_off = (py as u32 * stride + x0 as u32) as usize;
        let count = (x1 - x0) as usize;
        if src_off + count <= data.len() {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(src_off),
                    win.surface.pixels.add(dst_off),
                    count,
                );
            }
        }
    }
    0
}

/// Alpha-blend ARGB pixel data onto window content surface.
/// Unlike `blit`, this properly handles transparent and semi-transparent pixels.
pub fn blit_alpha(window_id: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let sw = win.surface.width as i32;
    let sh = win.surface.height as i32;
    let stride = win.surface.width;
    for row in 0..h as i32 {
        let py = y as i32 + row;
        if py < 0 || py >= sh { continue; }
        let x0 = (x as i32).max(0);
        let x1 = (x as i32 + w as i32).min(sw);
        if x0 >= x1 { continue; }
        for col in x0..x1 {
            let src_idx = (row * w as i32 + (col - x as i32)) as usize;
            if src_idx >= data.len() { break; }
            let src = data[src_idx];
            let a = (src >> 24) & 0xFF;
            if a == 0 { continue; }
            let dst_idx = (py as u32 * stride + col as u32) as usize;
            unsafe {
                if a >= 255 {
                    *win.surface.pixels.add(dst_idx) = src;
                } else {
                    let dst = *win.surface.pixels.add(dst_idx);
                    let inv = 255 - a;
                    let r = (((src >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * inv) / 255;
                    let g = (((src >> 8) & 0xFF) * a + ((dst >> 8) & 0xFF) * inv) / 255;
                    let b = ((src & 0xFF) * a + (dst & 0xFF) * inv) / 255;
                    *win.surface.pixels.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
    0
}

/// List open windows. Returns number of windows.
pub fn list_windows(_buf: &mut [u8]) -> u32 {
    // TODO: implement via compositor IPC
    0
}

/// Focus/raise a window by ID.
pub fn focus(_window_id: u32) -> u32 {
    // TODO: implement via compositor IPC
    0
}

/// Get screen dimensions. Returns (width, height).
pub fn screen_size() -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (dll().screen_size)(&mut w, &mut h);
    (w, h)
}

/// Set display resolution. Returns true on success.
pub fn set_resolution(width: u32, height: u32) -> bool {
    syscall2(SYS_SET_RESOLUTION, width as u64, height as u64) == 0
}

/// List supported display resolutions.
pub fn list_resolutions() -> alloc::vec::Vec<(u32, u32)> {
    let mut buf = [0u32; 32];
    let count =
        syscall2(SYS_LIST_RESOLUTIONS, buf.as_mut_ptr() as u64, (buf.len() * 4) as u64);
    let mut modes = alloc::vec::Vec::new();
    for i in 0..count as usize {
        if i * 2 + 1 < buf.len() {
            modes.push((buf[i * 2], buf[i * 2 + 1]));
        }
    }
    modes
}

/// Get current theme. Returns 0 (dark) or 1 (light).
/// Reads from the shared uisys.dlib page (zero syscalls).
pub fn get_theme() -> u32 {
    unsafe { core::ptr::read_volatile(0x0400_000C as *const u32) }
}

/// Set the system theme. Sends CMD_SET_THEME (0x100D) to compositor.
/// theme: 0 = dark, 1 = light.
pub fn set_theme(theme: u32) {
    if !ensure_init() { return; }
    let st = state();
    crate::ipc::evt_chan_emit(st.channel_id, &[0x100D, theme, 0, 0, 0]);
}

/// Load a TTF font from a file path. Returns font_id or None.
pub fn font_load(path: &str) -> Option<u32> {
    let id = (libfont().load_font)(path.as_ptr(), path.len() as u32);
    if id == u32::MAX {
        None
    } else {
        Some(id)
    }
}

/// Unload a previously loaded font.
pub fn font_unload(font_id: u32) {
    (libfont().unload_font)(font_id);
}

/// Measure text dimensions with a specific font and size.
pub fn font_measure(font_id: u16, size: u16, text: &str) -> (u32, u32) {
    let mut out_w: u32 = 0;
    let mut out_h: u32 = 0;
    (libfont().measure_string)(
        font_id as u32, size,
        text.as_ptr(), text.len() as u32,
        &mut out_w, &mut out_h,
    );
    (out_w, out_h)
}

/// Draw text with explicit font_id and size (TTF rendering via libfont.dlib).
pub fn draw_text_ex(
    window_id: u32,
    x: i16,
    y: i16,
    color: u32,
    font_id: u16,
    size: u16,
    text: &str,
) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let pixel_count = (win.surface.width as usize) * (win.surface.height as usize);
    let buf = unsafe { core::slice::from_raw_parts_mut(win.surface.pixels, pixel_count) };
    font_render_buf(
        font_id,
        size,
        buf,
        win.surface.width,
        win.surface.height,
        x as i32,
        y as i32,
        color,
        text,
    )
}

/// Render TTF text directly into a raw ARGB pixel buffer via libfont.dlib.
pub fn font_render_buf(
    font_id: u16,
    size: u16,
    buf: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    text: &str,
) -> u32 {
    (libfont().draw_string_buf)(
        buf.as_mut_ptr(), buf_w, buf_h,
        x, y, color,
        font_id as u32, size,
        text.as_ptr(), text.len() as u32,
    );
    0
}

/// Fill a rounded rectangle.
pub fn fill_rounded_rect(
    window_id: u32,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    radius: u16,
    color: u32,
) -> u32 {
    let win = match find_win(window_id) {
        Some(w) => w,
        None => return u32::MAX,
    };
    let sw = win.surface.width as i32;
    let sh = win.surface.height as i32;
    let stride = win.surface.width;
    let r = radius as i32;
    let rx = x as i32;
    let ry = y as i32;
    let rw = w as i32;
    let rh = h as i32;

    for row in 0..rh {
        let py = ry + row;
        if py < 0 || py >= sh {
            continue;
        }

        let mut x_start = rx;
        let mut x_end = rx + rw;

        // Top corners
        if row < r {
            let dy = r - 1 - row;
            let dx = r - isqrt((r * r - dy * dy) as u32) as i32;
            x_start = rx + dx;
            x_end = rx + rw - dx;
        }
        // Bottom corners
        else if row >= rh - r {
            let dy = row - (rh - r);
            let dx = r - isqrt((r * r - dy * dy) as u32) as i32;
            x_start = rx + dx;
            x_end = rx + rw - dx;
        }

        x_start = x_start.max(0);
        x_end = x_end.min(sw);
        if x_start >= x_end {
            continue;
        }

        let start = (py as u32 * stride + x_start as u32) as usize;
        let count = (x_end - x_start) as usize;
        unsafe {
            let slice = core::slice::from_raw_parts_mut(win.surface.pixels.add(start), count);
            slice.fill(color);
        }
    }
    0
}

/// Query if GPU hardware acceleration is available.
pub fn gpu_has_accel() -> bool {
    syscall0(SYS_GPU_HAS_ACCEL) != 0
}

/// Set the desktop wallpaper by file path.
/// The compositor loads and scales the image to fit the screen.
pub fn set_wallpaper(path: &str) -> u32 {
    if !ensure_init() { return u32::MAX; }
    let st = state();
    let bytes = path.as_bytes();
    (dll().set_wallpaper)(st.channel_id, bytes.as_ptr(), bytes.len() as u32);
    0
}

/// Get GPU driver name.
pub fn gpu_name() -> alloc::string::String {
    let mut buf = [0u8; 64];
    let len = syscall2(SYS_GPU_INFO, buf.as_mut_ptr() as u64, buf.len() as u64);
    if len > 0 {
        let actual_len = (len as usize).min(63);
        alloc::string::String::from(core::str::from_utf8(&buf[..actual_len]).unwrap_or("Unknown"))
    } else {
        alloc::string::String::from("Software (VESA VBE)")
    }
}

/// Get the raw surface pointer for a window.
pub fn surface_ptr(window_id: u32) -> *mut u32 {
    find_win(window_id)
        .map(|w| w.surface.pixels)
        .unwrap_or(core::ptr::null_mut())
}

/// Get surface info (pointer, width, height) for direct rendering.
pub fn surface_info(window_id: u32) -> Option<(*mut u32, u32, u32)> {
    find_win(window_id).map(|w| (w.surface.pixels, w.surface.width, w.surface.height))
}

// ── Menu Bar ────────────────────────────────────────────────────────────────

/// Set a window's menu bar definition. `data` is a binary blob from `MenuBarBuilder::build()`.
pub fn set_menu(window_id: u32, data: &[u8]) {
    if unsafe { COMP.is_none() } {
        return;
    }
    let st = state();
    if let Some(win) = st.windows.iter().find(|w| w.ext_id == window_id) {
        (dll().set_menu)(st.channel_id, win.comp_id, data.as_ptr(), data.len() as u32);
    }
}

/// Update the flags of a single menu item (enable/disable/check).
pub fn update_menu_item(window_id: u32, item_id: u32, new_flags: u32) {
    if unsafe { COMP.is_none() } {
        return;
    }
    let st = state();
    if let Some(win) = st.windows.iter().find(|w| w.ext_id == window_id) {
        (dll().update_menu_item)(st.channel_id, win.comp_id, item_id, new_flags);
    }
}

/// Enable a menu item (clear the DISABLED flag).
pub fn enable_menu_item(window_id: u32, item_id: u32) {
    update_menu_item(window_id, item_id, 0);
}

/// Disable a menu item (set the DISABLED flag).
pub fn disable_menu_item(window_id: u32, item_id: u32) {
    update_menu_item(window_id, item_id, MENU_FLAG_DISABLED);
}

const MENU_MAGIC: u32 = 0x4D454E55; // 'MENU'

/// Item flags for menu items.
pub const MENU_FLAG_DISABLED: u32 = 0x01;
pub const MENU_FLAG_SEPARATOR: u32 = 0x02;
pub const MENU_FLAG_CHECKED: u32 = 0x04;

/// Well-known app menu item IDs (auto-generated by compositor).
pub const APP_MENU_ABOUT: u32 = 0xFFF0;
pub const APP_MENU_HIDE: u32 = 0xFFF1;
pub const APP_MENU_QUIT: u32 = 0xFFF2;

/// Builder for constructing a binary menu bar definition.
///
/// ```rust,ignore
/// let mut mb = MenuBarBuilder::new();
/// let data = mb
///     .menu("File").item(1, "New", 0).item(2, "Open", 0).separator().item(5, "Quit", 0).end_menu()
///     .menu("Edit").item(10, "Cut", 0).item(11, "Copy", 0).item(12, "Paste", 0).end_menu()
///     .build();
/// window::set_menu(win, data);
/// ```
pub struct MenuBarBuilder {
    buf: [u8; 4096],
    pos: usize,
    num_menus: usize,
    num_menus_offset: usize,
}

/// Intermediate builder for adding items to a single menu.
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
        b.write_u32(MENU_MAGIC);
        b.num_menus_offset = b.pos;
        b.write_u32(0); // placeholder
        b
    }

    /// Start a new top-level menu with the given title.
    pub fn menu(mut self, title: &str) -> MenuBuilder {
        let bytes = title.as_bytes();
        let len = bytes.len().min(64);
        self.write_u32(len as u32);
        self.write_bytes(&bytes[..len]);
        self.align4();
        let num_items_offset = self.pos;
        self.write_u32(0); // placeholder
        self.num_menus += 1;
        MenuBuilder {
            inner: self,
            num_items: 0,
            num_items_offset,
        }
    }

    /// Finalize and return the binary menu data slice.
    pub fn build(&mut self) -> &[u8] {
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
    /// Add a menu item.
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
        let ni = self.num_items as u32;
        self.inner.buf[self.num_items_offset..self.num_items_offset + 4]
            .copy_from_slice(&ni.to_le_bytes());
        self.inner
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn isqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
