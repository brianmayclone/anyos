//! Desktop manager — window management, menubar, wallpaper, cursor, input handling.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::compositor::{alpha_blend, Compositor, Rect};
use crate::ipc_protocol as proto;
use crate::keys::encode_scancode;
use crate::menu::{MenuBar, MenuBarDef, MenuBarHit};

// ── Theme Constants (macOS dark) ────────────────────────────────────────────

const COLOR_DESKTOP_BG: u32 = 0xFF1E1E1E;
const COLOR_MENUBAR_BG: u32 = 0xE6303035;
const COLOR_MENUBAR_BORDER: u32 = 0xFF404045;
const COLOR_MENUBAR_TEXT: u32 = 0xFFE0E0E0;

const COLOR_TITLEBAR_FOCUSED: u32 = 0xFF3C3C3C;
const COLOR_TITLEBAR_UNFOCUSED: u32 = 0xFF2A2A2A;
const COLOR_TITLEBAR_TEXT: u32 = 0xFFE0E0E0;
const COLOR_WINDOW_BG: u32 = 0xFF1E1E1E;
const COLOR_WINDOW_BORDER: u32 = 0xFF4A4A4E;

const COLOR_CLOSE_BTN: u32 = 0xFFFF5F56;
const COLOR_MIN_BTN: u32 = 0xFFFEBD2E;
const COLOR_MAX_BTN: u32 = 0xFF27C93F;
const COLOR_BTN_UNFOCUSED: u32 = 0xFF5A5A5E;

pub const MENUBAR_HEIGHT: u32 = 24;
const TITLE_BAR_HEIGHT: u32 = 28;

// ── System Font ────────────────────────────────────────────────────────────
const FONT_ID: u16 = 0;   // System font (sfpro.ttf), loaded by kernel at boot
const FONT_SIZE: u16 = 13;
const TITLE_BTN_SIZE: u32 = 12;
const TITLE_BTN_Y: u32 = 8;
const TITLE_BTN_SPACING: u32 = 20;

// ── Window Flags ────────────────────────────────────────────────────────────

pub const WIN_FLAG_BORDERLESS: u32 = 0x01;
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;

// ── Event Types ─────────────────────────────────────────────────────────────

pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;
pub const EVENT_MOUSE_SCROLL: u32 = 7;
pub const EVENT_WINDOW_CLOSE: u32 = 8;
pub const EVENT_MENU_ITEM: u32 = 9;
pub const EVENT_STATUS_ICON_CLICK: u32 = 10;

// ── Input Constants ─────────────────────────────────────────────────────────

const INPUT_KEY_DOWN: u32 = 1;
const INPUT_KEY_UP: u32 = 2;
const INPUT_MOUSE_MOVE: u32 = 3;
const INPUT_MOUSE_BUTTON: u32 = 4;
const INPUT_MOUSE_SCROLL: u32 = 5;

// ── SW Cursor ───────────────────────────────────────────────────────────────

const CURSOR_W: u32 = 12;
const CURSOR_H: u32 = 18;
#[rustfmt::skip]
const CURSOR_BITMAP: [u8; (CURSOR_W * CURSOR_H) as usize] = [
    2,0,0,0,0,0,0,0,0,0,0,0,
    2,2,0,0,0,0,0,0,0,0,0,0,
    2,1,2,0,0,0,0,0,0,0,0,0,
    2,1,1,2,0,0,0,0,0,0,0,0,
    2,1,1,1,2,0,0,0,0,0,0,0,
    2,1,1,1,1,2,0,0,0,0,0,0,
    2,1,1,1,1,1,2,0,0,0,0,0,
    2,1,1,1,1,1,1,2,0,0,0,0,
    2,1,1,1,1,1,1,1,2,0,0,0,
    2,1,1,1,1,1,1,1,1,2,0,0,
    2,1,1,1,1,1,1,1,1,1,2,0,
    2,1,1,1,1,1,2,2,2,2,2,0,
    2,1,1,1,1,2,0,0,0,0,0,0,
    2,1,1,2,1,1,2,0,0,0,0,0,
    2,1,2,0,2,1,1,2,0,0,0,0,
    2,2,0,0,2,1,1,2,0,0,0,0,
    2,0,0,0,0,2,1,2,0,0,0,0,
    0,0,0,0,0,2,2,2,0,0,0,0,
];

// ── HW Cursor Shapes (ARGB8888) ────────────────────────────────────────────

const W: u32 = 0xFFFFFFFF; // white (fill)
const B: u32 = 0xFF000000; // black (outline)
const T: u32 = 0x00000000; // transparent

/// Cursor shape identifiers
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Arrow,
    ResizeNS,
    ResizeEW,
    ResizeNWSE,
    ResizeNESW,
    Move,
}

/// Arrow cursor (12x18, hotspot 0,0) — matches the SW cursor bitmap
const HW_ARROW_W: u32 = 12;
const HW_ARROW_H: u32 = 18;
const HW_ARROW_HOT_X: u32 = 0;
const HW_ARROW_HOT_Y: u32 = 0;

fn bitmap_to_argb(bitmap: &[u8], out: &mut [u32]) {
    for (i, &val) in bitmap.iter().enumerate() {
        out[i] = match val {
            1 => W,
            2 => B,
            _ => T,
        };
    }
}

/// N-S resize cursor (11x16, hotspot 5,8)
const HW_RESIZE_NS_W: u32 = 11;
const HW_RESIZE_NS_H: u32 = 16;
const HW_RESIZE_NS_HOT_X: u32 = 5;
const HW_RESIZE_NS_HOT_Y: u32 = 8;
#[rustfmt::skip]
static HW_RESIZE_NS: [u32; (11 * 16) as usize] = [
    T,T,T,T,B,B,B,T,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,B,W,W,W,W,W,B,T,T,
    T,B,W,W,W,W,W,W,W,B,T,
    B,B,B,B,W,W,W,B,B,B,B,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    B,B,B,B,W,W,W,B,B,B,B,
    T,B,W,W,W,W,W,W,W,B,T,
    T,T,B,W,W,W,W,W,B,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,T,B,B,B,T,T,T,T,
];

/// E-W resize cursor (16x11, hotspot 8,5)
const HW_RESIZE_EW_W: u32 = 16;
const HW_RESIZE_EW_H: u32 = 11;
const HW_RESIZE_EW_HOT_X: u32 = 8;
const HW_RESIZE_EW_HOT_Y: u32 = 5;
#[rustfmt::skip]
static HW_RESIZE_EW: [u32; (16 * 11) as usize] = [
    T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,
    T,T,T,B,T,T,T,T,T,T,T,T,B,T,T,T,
    T,T,B,W,B,T,T,T,T,T,T,B,W,B,T,T,
    T,B,W,W,B,T,T,T,T,T,T,B,W,W,B,T,
    B,W,W,W,B,B,B,B,B,B,B,B,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,B,B,B,B,B,B,B,B,W,W,W,B,
    T,B,W,W,B,T,T,T,T,T,T,B,W,W,B,T,
    T,T,B,W,B,T,T,T,T,T,T,B,W,B,T,T,
    T,T,T,B,T,T,T,T,T,T,T,T,B,T,T,T,
    T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,
];

/// NW-SE resize cursor (14x14, hotspot 7,7)
const HW_RESIZE_NWSE_W: u32 = 14;
const HW_RESIZE_NWSE_H: u32 = 14;
const HW_RESIZE_NWSE_HOT_X: u32 = 7;
const HW_RESIZE_NWSE_HOT_Y: u32 = 7;
#[rustfmt::skip]
static HW_RESIZE_NWSE: [u32; (14 * 14) as usize] = [
    B,B,B,B,B,B,B,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,W,W,B,T,T,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,B,W,W,W,B,T,T,T,T,T,T,T,
    B,B,T,B,W,W,W,B,T,T,T,T,T,T,
    B,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,B,
    T,T,T,T,T,T,B,W,W,W,B,T,B,B,
    T,T,T,T,T,T,T,B,W,W,W,B,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,T,T,B,W,W,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,B,B,B,B,B,B,B,
];

/// NE-SW resize cursor (14x14, hotspot 7,7)
const HW_RESIZE_NESW_W: u32 = 14;
const HW_RESIZE_NESW_H: u32 = 14;
const HW_RESIZE_NESW_HOT_X: u32 = 7;
const HW_RESIZE_NESW_HOT_Y: u32 = 7;
#[rustfmt::skip]
static HW_RESIZE_NESW: [u32; (14 * 14) as usize] = [
    T,T,T,T,T,T,T,B,B,B,B,B,B,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,T,T,B,W,W,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,B,W,W,W,B,W,B,
    T,T,T,T,T,T,B,W,W,W,B,T,B,B,
    T,T,T,T,T,B,W,W,W,B,T,T,T,B,
    B,T,T,T,B,W,W,W,B,T,T,T,T,T,
    B,B,T,B,W,W,W,B,T,T,T,T,T,T,
    B,W,B,W,W,W,B,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,W,W,B,T,T,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,B,B,B,B,B,B,T,T,T,T,T,T,T,
];

/// Move/grab cursor (15x15, hotspot 7,7)
const HW_MOVE_W: u32 = 15;
const HW_MOVE_H: u32 = 15;
const HW_MOVE_HOT_X: u32 = 7;
const HW_MOVE_HOT_Y: u32 = 7;
#[rustfmt::skip]
static HW_MOVE: [u32; (15 * 15) as usize] = [
    T,T,T,T,T,T,B,B,B,T,T,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,B,W,W,W,W,W,B,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,B,T,T,B,W,W,W,B,T,T,B,T,T,
    T,B,W,B,B,B,W,W,W,B,B,B,W,B,T,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    T,B,W,B,B,B,W,W,W,B,B,B,W,B,T,
    T,T,B,T,T,B,W,W,W,B,T,T,B,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,B,W,W,W,W,W,B,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,T,T,B,B,B,T,T,T,T,T,T,
];

// ── Hit Test ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum HitTest {
    None,
    TitleBar,
    CloseButton,
    MinButton,
    MaxButton,
    Content,
    ResizeTop,
    ResizeBottom,
    ResizeLeft,
    ResizeRight,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
    ResizeBottomRight,
}

// ── Window Info ─────────────────────────────────────────────────────────────

pub struct WindowInfo {
    pub id: u32,
    pub layer_id: u32,
    pub title: String,
    /// Position of the full window (including title bar)
    pub x: i32,
    pub y: i32,
    /// Content area dimensions (excluding title bar)
    pub content_width: u32,
    pub content_height: u32,
    pub flags: u32,
    pub owner_tid: u32,
    /// Event queue for this window
    pub events: VecDeque<[u32; 5]>,
    /// Whether this window has the focused title bar style
    pub focused: bool,
    /// Saved bounds for maximize toggle
    pub saved_bounds: Option<(i32, i32, u32, u32)>,
    /// Whether the window is maximized
    pub maximized: bool,
    /// SHM region ID (0 = local/compositor-owned window)
    pub shm_id: u32,
    /// SHM pixel pointer (null = local window)
    pub shm_ptr: *mut u32,
}

impl WindowInfo {
    pub fn is_borderless(&self) -> bool {
        self.flags & WIN_FLAG_BORDERLESS != 0
    }

    pub fn is_resizable(&self) -> bool {
        self.flags & WIN_FLAG_NOT_RESIZABLE == 0
    }

    pub fn is_always_on_top(&self) -> bool {
        self.flags & WIN_FLAG_ALWAYS_ON_TOP != 0
    }

    /// Full window width (same as content for borderless).
    pub fn full_width(&self) -> u32 {
        self.content_width
    }

    /// Full window height (content + title bar for decorated windows).
    pub fn full_height(&self) -> u32 {
        if self.is_borderless() {
            self.content_height
        } else {
            self.content_height + TITLE_BAR_HEIGHT
        }
    }

    /// Hit test a point (in screen coordinates) against the window.
    pub fn hit_test(&self, px: i32, py: i32) -> HitTest {
        let wx = px - self.x;
        let wy = py - self.y;
        let fw = self.full_width() as i32;
        let fh = self.full_height() as i32;

        if wx < 0 || wy < 0 || wx >= fw || wy >= fh {
            return HitTest::None;
        }

        if self.is_borderless() {
            return HitTest::Content;
        }

        // Resize edges (4px border)
        if self.is_resizable() && !self.maximized {
            let edge = 4;
            let top = wy < edge;
            let bottom = wy >= fh - edge;
            let left = wx < edge;
            let right = wx >= fw - edge;

            if top && left {
                return HitTest::ResizeTopLeft;
            }
            if top && right {
                return HitTest::ResizeTopRight;
            }
            if bottom && left {
                return HitTest::ResizeBottomLeft;
            }
            if bottom && right {
                return HitTest::ResizeBottomRight;
            }
            if top {
                return HitTest::ResizeTop;
            }
            if bottom {
                return HitTest::ResizeBottom;
            }
            if left {
                return HitTest::ResizeLeft;
            }
            if right {
                return HitTest::ResizeRight;
            }
        }

        // Title bar
        if wy < TITLE_BAR_HEIGHT as i32 {
            // Traffic light buttons
            let btn_y = TITLE_BTN_Y as i32;
            let btn_r = (TITLE_BTN_SIZE / 2) as i32;
            if wy >= btn_y && wy < btn_y + TITLE_BTN_SIZE as i32 {
                // Close button at x=8
                let cx = 8 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::CloseButton;
                }
                // Minimize at x=28
                let cx = 28 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::MinButton;
                }
                // Maximize at x=48
                let cx = 48 + btn_r;
                if (wx - cx).abs() <= btn_r && (wy - btn_y - btn_r).abs() <= btn_r {
                    return HitTest::MaxButton;
                }
            }
            return HitTest::TitleBar;
        }

        HitTest::Content
    }
}

// ── Interaction State ───────────────────────────────────────────────────────

struct DragState {
    window_id: u32,
    offset_x: i32,
    offset_y: i32,
}

struct ResizeState {
    window_id: u32,
    start_mouse_x: i32,
    start_mouse_y: i32,
    start_x: i32,
    start_y: i32,
    start_w: u32,
    start_h: u32,
    edge: HitTest,
}

// ── Desktop ─────────────────────────────────────────────────────────────────

pub struct Desktop {
    pub compositor: Compositor,

    // Desktop layers
    bg_layer_id: u32,
    menubar_layer_id: u32,

    // Windows
    windows: Vec<WindowInfo>,
    focused_window: Option<u32>,
    next_window_id: u32,

    // Mouse state
    pub mouse_x: i32,
    pub mouse_y: i32,
    mouse_buttons: u32,

    // SW cursor
    cursor_save: Vec<u32>,
    cursor_drawn: bool,
    prev_cursor_x: i32,
    prev_cursor_y: i32,

    // Interaction
    dragging: Option<DragState>,
    resizing: Option<ResizeState>,

    // Screen
    pub screen_width: u32,
    pub screen_height: u32,

    // Menubar clock
    last_clock_min: u32,

    // HW cursor
    current_cursor: CursorShape,
    /// ARGB pixels for the arrow cursor (converted from bitmap on init)
    hw_arrow_pixels: Vec<u32>,

    // Menu bar system
    menu_bar: MenuBar,
}

impl Desktop {
    pub fn new(fb_ptr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        let mut compositor = Compositor::new(fb_ptr, width, height, pitch);

        // Background layer (bottom)
        let bg_id = compositor.add_layer(0, 0, width, height, true);

        // Menubar layer (top of desktop, below windows)
        let mb_id = compositor.add_layer(0, 0, width, MENUBAR_HEIGHT + 1, false);

        // Convert arrow cursor bitmap to ARGB
        let mut arrow_pixels = vec![0u32; (HW_ARROW_W * HW_ARROW_H) as usize];
        bitmap_to_argb(&CURSOR_BITMAP, &mut arrow_pixels);

        Desktop {
            compositor,
            bg_layer_id: bg_id,
            menubar_layer_id: mb_id,
            windows: Vec::with_capacity(16),
            focused_window: None,
            next_window_id: 1,
            mouse_x: width as i32 / 2,
            mouse_y: height as i32 / 2,
            mouse_buttons: 0,
            cursor_save: vec![0u32; (CURSOR_W * CURSOR_H) as usize],
            cursor_drawn: false,
            prev_cursor_x: width as i32 / 2,
            prev_cursor_y: height as i32 / 2,
            dragging: None,
            resizing: None,
            screen_width: width,
            screen_height: height,
            last_clock_min: u32::MAX,
            current_cursor: CursorShape::Arrow,
            hw_arrow_pixels: arrow_pixels,
            menu_bar: MenuBar::new(),
        }
    }

    // ── Initialization ──────────────────────────────────────────────────

    /// Draw the initial desktop (background + menubar).
    pub fn init(&mut self) {
        // Load wallpaper (falls back to gradient)
        if !self.load_wallpaper("/media/wallpapers/default.png") {
            self.draw_gradient_background();
        }
        self.draw_menubar();
        self.compositor.damage_all();
    }

    /// Load a wallpaper image and draw it to the background layer.
    fn load_wallpaper(&mut self, path: &str) -> bool {
        use anyos_std::fs;

        let fd = fs::open(path, 0);
        if fd == u32::MAX { return false; }

        // Get file size
        let mut stat_buf = [0u32; 3];
        if fs::fstat(fd, &mut stat_buf) == u32::MAX {
            fs::close(fd);
            return false;
        }
        let file_size = stat_buf[1] as usize;
        if file_size == 0 || file_size > 2 * 1024 * 1024 {
            fs::close(fd);
            return false;
        }

        // Read file
        let mut data = vec![0u8; file_size];
        let bytes_read = fs::read(fd, &mut data) as usize;
        fs::close(fd);
        if bytes_read == 0 { return false; }

        // Probe image
        let info = match libimage_client::probe(&data[..bytes_read]) {
            Some(i) => i,
            None => return false,
        };

        let pixel_count = (info.width * info.height) as usize;
        if pixel_count > 4 * 1024 * 1024 { return false; }

        // Decode
        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode(&data[..bytes_read], &mut pixels, &mut scratch).is_err() {
            return false;
        }
        // Free file data and scratch early
        drop(data);
        drop(scratch);

        let sw = self.screen_width;
        let sh = self.screen_height;

        // Scale to screen if needed
        let final_pixels = if info.width == sw && info.height == sh {
            pixels
        } else {
            let mut dst = vec![0u32; (sw * sh) as usize];
            if !libimage_client::scale_image(
                &pixels, info.width, info.height,
                &mut dst, sw, sh,
                libimage_client::MODE_COVER,
            ) {
                return false;
            }
            drop(pixels);
            dst
        };

        // Copy to background layer
        if let Some(bg_pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
            let copy_len = bg_pixels.len().min(final_pixels.len());
            bg_pixels[..copy_len].copy_from_slice(&final_pixels[..copy_len]);
        }

        anyos_std::println!("compositor: wallpaper loaded ({}x{} {})",
            info.width, info.height,
            libimage_client::format_name(info.format));
        true
    }

    /// Draw a gradient fallback background.
    fn draw_gradient_background(&mut self) {
        let w = self.screen_width;
        let h = self.screen_height;
        if let Some(pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
            for y in 0..h {
                let t = y * 255 / h.max(1);
                let r = 25 - (t * 10 / 255).min(10);
                let g = 25 - (t * 10 / 255).min(10);
                let b = 35 - (t * 10 / 255).min(10);
                let color = 0xFF000000 | (r << 16) | (g << 8) | b;
                for x in 0..w {
                    pixels[(y * w + x) as usize] = color;
                }
            }
        }
    }

    /// Draw the menubar layer.
    pub fn draw_menubar(&mut self) {
        let w = self.screen_width;
        let h = MENUBAR_HEIGHT + 1;
        if let Some(pixels) = self.compositor.layer_pixels(self.menubar_layer_id) {
            // Fill background
            for y in 0..MENUBAR_HEIGHT {
                for x in 0..w {
                    pixels[(y * w + x) as usize] = COLOR_MENUBAR_BG;
                }
            }
            // Bottom border
            for x in 0..w {
                pixels[(MENUBAR_HEIGHT * w + x) as usize] = COLOR_MENUBAR_BORDER;
            }

            // "anyOS" brand text (left) — TTF system font
            let (_, fh) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, "anyOS");
            let fy = ((MENUBAR_HEIGHT as i32 - fh as i32) / 2).max(0);
            anyos_std::ui::window::font_render_buf(
                FONT_ID, FONT_SIZE, pixels, w, h, 10, fy, COLOR_MENUBAR_TEXT, "anyOS",
            );

            // App menu titles (after "anyOS")
            self.menu_bar.render_titles(pixels, w, h);

            // Clock (right side)
            draw_clock_to_menubar(pixels, w);

            // Status icons (between menus and clock)
            self.menu_bar.render_status_icons(pixels, w);
        }
        self.compositor.mark_layer_dirty(self.menubar_layer_id);
    }

    // draw_clock_to_menubar is a free function below (avoids borrow checker issues)

    /// Update the clock display (call periodically).
    pub fn update_clock(&mut self) {
        let mut time_buf = [0u8; 8];
        anyos_std::sys::time(&mut time_buf);
        let min = time_buf[5] as u32;
        if min != self.last_clock_min {
            self.last_clock_min = min;
            let w = self.screen_width;
            if let Some(pixels) = self.compositor.layer_pixels(self.menubar_layer_id) {
                draw_clock_to_menubar(pixels, w);
            }
            self.compositor.mark_layer_dirty(self.menubar_layer_id);
            // Add damage for the right portion of menubar
            self.compositor.add_damage(Rect::new(
                (w as i32 - 60).max(0),
                0,
                60,
                MENUBAR_HEIGHT + 1,
            ));
        }
    }

    // ── Window Management ───────────────────────────────────────────────

    /// Create a new window.
    pub fn create_window(
        &mut self,
        title: &str,
        x: i32,
        y: i32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        owner_tid: u32,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let borderless = flags & WIN_FLAG_BORDERLESS != 0;
        let full_h = if borderless {
            content_h
        } else {
            content_h + TITLE_BAR_HEIGHT
        };

        // Create compositor layer for this window
        let opaque = borderless; // Decorated windows have transparent rounded corners
        let layer_id = self.compositor.add_layer(x, y, content_w, full_h, opaque);

        // No shadow for windows
        let _ = borderless;

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from(title),
            x,
            y,
            content_width: content_w,
            content_height: content_h,
            flags,
            owner_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id: 0,
            shm_ptr: core::ptr::null_mut(),
        };

        self.windows.push(win);
        self.render_window(id);
        self.focus_window(id);

        id
    }

    /// Destroy a window.
    pub fn destroy_window(&mut self, id: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            let layer_id = self.windows[idx].layer_id;
            self.compositor.remove_layer(layer_id);
            self.windows.remove(idx);

            // Clean up menus for this window
            self.menu_bar.remove_menu(id);

            // Focus the next top window
            if self.focused_window == Some(id) {
                self.focused_window = None;
                if let Some(last) = self.windows.last() {
                    let next_id = last.id;
                    self.focus_window(next_id);
                } else {
                    // No windows left, clear menubar
                    if self.menu_bar.on_focus_change(None) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
            }
        }
    }

    /// Destroy all windows owned by a given thread (process exit cleanup).
    pub fn on_process_exit(&mut self, tid: u32) {
        // Collect window IDs owned by this TID
        let window_ids: Vec<u32> = self.windows.iter()
            .filter(|w| w.owner_tid == tid)
            .map(|w| w.id)
            .collect();
        // Destroy each window
        for id in window_ids {
            self.destroy_window(id);
        }
    }

    /// Focus a window (bring to front and set focused style).
    pub fn focus_window(&mut self, id: u32) {
        // Unfocus previous
        if let Some(old_id) = self.focused_window {
            if old_id != id {
                if let Some(idx) = self.windows.iter().position(|w| w.id == old_id) {
                    self.windows[idx].focused = false;
                    let win_id = self.windows[idx].id;
                    self.render_window(win_id);
                }
            }
        }

        if let Some(idx) = self.windows.iter().position(|w| w.id == id) {
            self.windows[idx].focused = true;
            self.focused_window = Some(id);
            let layer_id = self.windows[idx].layer_id;
            self.compositor.raise_layer(layer_id);
            self.render_window(id);

            // Update menu bar for the newly focused window
            if self.menu_bar.on_focus_change(Some(id)) {
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }
        }
    }

    /// Get a window's event queue.
    pub fn poll_event(&mut self, window_id: u32) -> Option<[u32; 5]> {
        self.windows
            .iter_mut()
            .find(|w| w.id == window_id)
            .and_then(|w| w.events.pop_front())
    }

    /// Push an event to a window's queue.
    fn push_event(&mut self, window_id: u32, event: [u32; 5]) {
        if let Some(win) = self.windows.iter_mut().find(|w| w.id == window_id) {
            if win.events.len() < 256 {
                win.events.push_back(event);
            }
        }
    }

    // ── Window Rendering ────────────────────────────────────────────────

    /// Render a window's surface (decorations + content area).
    fn render_window(&mut self, window_id: u32) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        let layer_id = self.windows[win_idx].layer_id;
        let cw = self.windows[win_idx].content_width;
        let _ch = self.windows[win_idx].content_height;
        let borderless = self.windows[win_idx].is_borderless();
        let focused = self.windows[win_idx].focused;
        let title_clone = self.windows[win_idx].title.clone();
        let full_h = self.windows[win_idx].full_height();

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;

            if borderless {
                // Borderless: just fill with window bg
                for p in pixels.iter_mut() {
                    *p = COLOR_WINDOW_BG;
                }
            } else {
                // Clear to transparent (for rounded corners)
                for p in pixels.iter_mut() {
                    *p = 0x00000000;
                }

                // Draw window body (rounded rect)
                fill_rounded_rect(
                    pixels, stride, full_h, 0, 0, cw, full_h, 8, COLOR_WINDOW_BG,
                );

                // Title bar (rounded top corners)
                let tb_color = if focused {
                    COLOR_TITLEBAR_FOCUSED
                } else {
                    COLOR_TITLEBAR_UNFOCUSED
                };
                fill_rounded_rect_top(pixels, stride, 0, 0, cw, TITLE_BAR_HEIGHT, 8, tb_color);

                // Title bar bottom border
                let border_y = TITLE_BAR_HEIGHT - 1;
                for x in 0..cw {
                    let idx = (border_y * stride + x) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = COLOR_WINDOW_BORDER;
                    }
                }

                // Traffic light buttons
                let btn_colors = if focused {
                    [COLOR_CLOSE_BTN, COLOR_MIN_BTN, COLOR_MAX_BTN]
                } else {
                    [COLOR_BTN_UNFOCUSED, COLOR_BTN_UNFOCUSED, COLOR_BTN_UNFOCUSED]
                };
                for (i, &color) in btn_colors.iter().enumerate() {
                    let cx = 8 + i as i32 * TITLE_BTN_SPACING as i32 + TITLE_BTN_SIZE as i32 / 2;
                    let cy = TITLE_BTN_Y as i32 + TITLE_BTN_SIZE as i32 / 2;
                    fill_circle(pixels, stride, full_h, cx, cy, (TITLE_BTN_SIZE / 2) as i32, color);
                }

                // Title text (centered) — TTF system font
                let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &title_clone);
                let tx = (cw as i32 - tw as i32) / 2;
                let ty = ((TITLE_BAR_HEIGHT as i32 - th as i32) / 2).max(0);
                anyos_std::ui::window::font_render_buf(
                    FONT_ID, FONT_SIZE, pixels, stride, full_h, tx, ty,
                    COLOR_TITLEBAR_TEXT, &title_clone,
                );

                // Content area fill
                let content_y = TITLE_BAR_HEIGHT;
                for y in content_y..full_h {
                    for x in 0..cw {
                        let idx = (y * stride + x) as usize;
                        if idx < pixels.len() && pixels[idx] == COLOR_WINDOW_BG {
                            // Already filled by rounded rect — keep it
                        }
                    }
                }
            }
        }

        self.compositor.mark_layer_dirty(layer_id);

        // Restore SHM content for IPC windows (render_window fills the entire
        // layer including the content area with COLOR_WINDOW_BG; for app windows
        // we must re-apply the latest SHM content over the content area).
        if self.windows[win_idx].owner_tid != 0 && !self.windows[win_idx].shm_ptr.is_null() {
            self.present_ipc_window(window_id);
        }
    }

    // ── Input Handling ──────────────────────────────────────────────────

    /// Process a batch of raw input events. Returns true if a compose is needed.
    pub fn process_input(&mut self, events: &[[u32; 5]], count: usize) -> bool {
        let mut needs_compose = false;
        let mut cursor_moved = false;
        let mut last_dx: i32 = 0;
        let mut last_dy: i32 = 0;

        // Batch mouse moves — only process the final position
        for i in 0..count {
            let evt = events[i];
            match evt[0] {
                INPUT_MOUSE_MOVE => {
                    let dx = evt[1] as i32;
                    let dy = evt[2] as i32;
                    last_dx += dx;
                    last_dy += dy;
                    cursor_moved = true;
                }
                INPUT_MOUSE_BUTTON => {
                    // Apply any pending move first
                    if cursor_moved {
                        self.apply_mouse_move(last_dx, last_dy);
                        last_dx = 0;
                        last_dy = 0;
                        cursor_moved = false;
                    }
                    // Also accumulate dx/dy from button event
                    let dx = evt[3] as i32;
                    let dy = evt[4] as i32;
                    if dx != 0 || dy != 0 {
                        self.apply_mouse_move(dx, dy);
                    }
                    let buttons = evt[1];
                    let down = evt[2] != 0;
                    self.handle_mouse_button(buttons, down);
                    needs_compose = true;
                }
                INPUT_MOUSE_SCROLL => {
                    let dz = evt[1] as i32;
                    self.handle_scroll(dz);
                }
                INPUT_KEY_DOWN => {
                    self.handle_key(evt[1], evt[2], evt[3], true);
                }
                INPUT_KEY_UP => {
                    self.handle_key(evt[1], evt[2], evt[3], false);
                }
                _ => {}
            }
        }

        // Apply any remaining batched mouse move
        if cursor_moved {
            self.apply_mouse_move(last_dx, last_dy);
            needs_compose = true;
        }

        needs_compose
    }

    fn apply_mouse_move(&mut self, dx: i32, dy: i32) {
        self.mouse_x = (self.mouse_x + dx).clamp(0, self.screen_width as i32 - 1);
        self.mouse_y = (self.mouse_y + dy).clamp(0, self.screen_height as i32 - 1);

        // Handle drag
        if let Some(ref drag) = self.dragging {
            let win_id = drag.window_id;
            let new_x = self.mouse_x - drag.offset_x;
            let new_y = self.mouse_y - drag.offset_y;
            if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                let layer_id = self.windows[idx].layer_id;
                self.windows[idx].x = new_x;
                self.windows[idx].y = new_y;
                self.compositor.move_layer(layer_id, new_x, new_y);
            }
        }

        // Handle resize (show outline)
        if let Some(ref resize) = self.resizing {
            let rdx = self.mouse_x - resize.start_mouse_x;
            let rdy = self.mouse_y - resize.start_mouse_y;
            let (ox, oy, ow, oh) = compute_resize(
                resize.edge,
                resize.start_x,
                resize.start_y,
                resize.start_w,
                resize.start_h,
                rdx,
                rdy,
            );
            // Update resize outline
            let old_outline = self.compositor.resize_outline;
            self.compositor.resize_outline = Some(Rect::new(ox, oy, ow, oh));
            if let Some(old) = old_outline {
                self.compositor.add_damage(old.expand(2));
            }
            self.compositor
                .add_damage(Rect::new(ox, oy, ow, oh).expand(2));
        }

        // Update dropdown hover state and menu-slide
        if self.menu_bar.is_dropdown_open() {
            if self.menu_bar.update_hover(self.mouse_x, self.mouse_y) {
                self.menu_bar.render_dropdown(&mut self.compositor);
            }

            // If hovering a different menu title while dropdown is open, switch menus
            if self.mouse_y < MENUBAR_HEIGHT as i32 {
                if let MenuBarHit::MenuTitle { menu_idx } =
                    self.menu_bar.hit_test_menubar(self.mouse_x, self.mouse_y)
                {
                    let current_idx = self.menu_bar.open_dropdown.as_ref().map(|d| d.menu_idx);
                    if current_idx != Some(menu_idx) {
                        let owner_wid = self.focused_window.unwrap_or(0);
                        self.menu_bar
                            .close_dropdown_with_compositor(&mut self.compositor);
                        self.menu_bar
                            .open_menu(menu_idx, owner_wid, &mut self.compositor);
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0,
                            0,
                            self.screen_width,
                            MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
            }
        }

        // Update HW cursor position
        self.compositor
            .move_hw_cursor(self.mouse_x, self.mouse_y);

        // Update cursor shape based on what's under it
        if self.dragging.is_some() {
            self.set_cursor_shape(CursorShape::Move);
        } else if self.resizing.is_some() {
            // Keep current resize cursor during resize
        } else {
            // Hit-test to determine cursor shape
            let mx = self.mouse_x;
            let my = self.mouse_y;
            let mut shape = CursorShape::Arrow;
            // Check windows in reverse z-order (topmost first)
            for w in self.windows.iter().rev() {
                let hit = w.hit_test(mx, my);
                if hit != HitTest::None {
                    shape = self.cursor_for_hit(hit);
                    break;
                }
            }
            self.set_cursor_shape(shape);
        }
    }

    fn handle_mouse_button(&mut self, buttons: u32, down: bool) {
        if down {
            self.mouse_buttons = buttons;

            // Check if clicking within open dropdown
            if self.menu_bar.is_dropdown_open() {
                if self.menu_bar.is_in_dropdown(self.mouse_x, self.mouse_y) {
                    if let Some(item_id) = self.menu_bar.hit_test_dropdown(self.mouse_x, self.mouse_y) {
                        if let Some(win_id) = self.focused_window {
                            match item_id {
                                menu::APP_MENU_QUIT => {
                                    // Send close event to the window
                                    self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                                }
                                menu::APP_MENU_HIDE => {
                                    // Hide: unfocus and move off-screen (simple hide)
                                    if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                                        let layer_id = self.windows[idx].layer_id;
                                        // Save current position, then move off-screen
                                        self.windows[idx].saved_bounds = Some(Rect::new(
                                            self.windows[idx].x,
                                            self.windows[idx].y,
                                            self.windows[idx].content_width,
                                            self.windows[idx].full_height(),
                                        ));
                                        self.compositor.move_layer(layer_id, -10000, -10000);
                                    }
                                    // Focus the next window
                                    let next = self.windows.iter().rev()
                                        .find(|w| w.id != win_id && w.x >= 0)
                                        .map(|w| w.id);
                                    if let Some(nid) = next {
                                        self.focus_window(nid);
                                    }
                                }
                                _ => {
                                    // Forward all other items (including APP_MENU_ABOUT) to the app
                                    let menu_idx = self.menu_bar.open_dropdown
                                        .as_ref()
                                        .map(|d| d.menu_idx as u32)
                                        .unwrap_or(0);
                                    self.push_event(win_id, [EVENT_MENU_ITEM, menu_idx, item_id, 0, 0]);
                                }
                            }
                        }
                    }
                    // Close dropdown after click
                    self.menu_bar.close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                    return;
                }

                // Clicked outside dropdown — close it (fall through to normal handling)
                self.menu_bar.close_dropdown_with_compositor(&mut self.compositor);
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                ));
            }

            // Check menubar click
            if self.mouse_y < MENUBAR_HEIGHT as i32 {
                self.handle_menubar_click();
                return;
            }

            // Check window hits (top to bottom in z-order = reverse of windows array)
            let mx = self.mouse_x;
            let my = self.mouse_y;

            // Find topmost window that contains the click
            let mut hit_win_id = None;
            let mut hit_test = HitTest::None;
            for win in self.windows.iter().rev() {
                let ht = win.hit_test(mx, my);
                if ht != HitTest::None {
                    hit_win_id = Some(win.id);
                    hit_test = ht;
                    break;
                }
            }

            if let Some(win_id) = hit_win_id {
                // Focus this window
                if self.focused_window != Some(win_id) {
                    self.focus_window(win_id);
                }

                match hit_test {
                    HitTest::CloseButton => {
                        self.push_event(win_id, [EVENT_WINDOW_CLOSE, 0, 0, 0, 0]);
                    }
                    HitTest::TitleBar => {
                        // Start drag
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            self.dragging = Some(DragState {
                                window_id: win_id,
                                offset_x: mx - self.windows[idx].x,
                                offset_y: my - self.windows[idx].y,
                            });
                        }
                    }
                    HitTest::MaxButton => {
                        self.toggle_maximize(win_id);
                    }
                    HitTest::Content => {
                        // Forward to window (local coordinates)
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            let lx = mx - self.windows[idx].x;
                            let mut ly = my - self.windows[idx].y;
                            if !self.windows[idx].is_borderless() {
                                ly -= TITLE_BAR_HEIGHT as i32;
                            }
                            self.push_event(
                                win_id,
                                [EVENT_MOUSE_DOWN, lx as u32, ly as u32, buttons, 0],
                            );
                        }
                    }
                    ht if is_resize_edge(ht) => {
                        // Start resize
                        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                            if self.windows[idx].is_resizable() {
                                self.resizing = Some(ResizeState {
                                    window_id: win_id,
                                    start_mouse_x: mx,
                                    start_mouse_y: my,
                                    start_x: self.windows[idx].x,
                                    start_y: self.windows[idx].y,
                                    start_w: self.windows[idx].content_width,
                                    start_h: self.windows[idx].full_height(),
                                    edge: ht,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            // Mouse up
            self.mouse_buttons = 0;

            // End drag
            if self.dragging.is_some() {
                self.set_cursor_shape(CursorShape::Arrow);
            }
            self.dragging = None;

            // End resize — apply final size
            if let Some(resize) = self.resizing.take() {
                let rdx = self.mouse_x - resize.start_mouse_x;
                let rdy = self.mouse_y - resize.start_mouse_y;
                let (nx, ny, nw, nh) = compute_resize(
                    resize.edge,
                    resize.start_x,
                    resize.start_y,
                    resize.start_w,
                    resize.start_h,
                    rdx,
                    rdy,
                );

                // Remove outline and reset cursor
                self.set_cursor_shape(CursorShape::Arrow);
                if let Some(outline) = self.compositor.resize_outline.take() {
                    self.compositor.add_damage(outline.expand(2));
                }

                // Apply resize
                if let Some(idx) = self.windows.iter().position(|w| w.id == resize.window_id) {
                    let borderless = self.windows[idx].is_borderless();
                    let content_h = if borderless {
                        nh
                    } else {
                        nh.saturating_sub(TITLE_BAR_HEIGHT)
                    };
                    self.windows[idx].x = nx;
                    self.windows[idx].y = ny;
                    self.windows[idx].content_width = nw;
                    self.windows[idx].content_height = content_h;

                    let layer_id = self.windows[idx].layer_id;
                    let full_h = self.windows[idx].full_height();
                    self.compositor.move_layer(layer_id, nx, ny);
                    self.compositor.resize_layer(layer_id, nw, full_h);

                    let win_id = resize.window_id;
                    self.render_window(win_id);

                    self.push_event(
                        win_id,
                        [EVENT_RESIZE, nw, content_h, 0, 0],
                    );
                }
            }

            // Forward mouse up to focused window
            if let Some(win_id) = self.focused_window {
                if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
                    let lx = self.mouse_x - self.windows[idx].x;
                    let mut ly = self.mouse_y - self.windows[idx].y;
                    if !self.windows[idx].is_borderless() {
                        ly -= TITLE_BAR_HEIGHT as i32;
                    }
                    self.push_event(win_id, [EVENT_MOUSE_UP, lx as u32, ly as u32, 0, 0]);
                }
            }
        }
    }

    fn handle_scroll(&mut self, dz: i32) {
        if let Some(win_id) = self.focused_window {
            self.push_event(win_id, [EVENT_MOUSE_SCROLL, dz as u32, 0, 0, 0]);
        }
    }

    fn handle_key(&mut self, scancode: u32, chr: u32, mods: u32, down: bool) {
        if let Some(win_id) = self.focused_window {
            let evt_type = if down { EVENT_KEY_DOWN } else { EVENT_KEY_UP };
            let key_code = encode_scancode(scancode);
            self.push_event(win_id, [evt_type, key_code, chr, mods, 0]);
        }
    }

    fn handle_menubar_click(&mut self) {
        let mx = self.mouse_x;
        let my = self.mouse_y;
        match self.menu_bar.hit_test_menubar(mx, my) {
            MenuBarHit::MenuTitle { menu_idx } => {
                // Toggle: if same menu is open, close it; otherwise open new
                let same = self
                    .menu_bar
                    .open_dropdown
                    .as_ref()
                    .map(|d| d.menu_idx == menu_idx)
                    .unwrap_or(false);
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                }
                if !same {
                    let owner_wid = self.focused_window.unwrap_or(0);
                    self.menu_bar
                        .open_menu(menu_idx, owner_wid, &mut self.compositor);
                }
                self.draw_menubar();
                self.compositor.add_damage(Rect::new(
                    0,
                    0,
                    self.screen_width,
                    MENUBAR_HEIGHT + 1,
                ));
            }
            MenuBarHit::StatusIcon {
                owner_tid,
                icon_id,
            } => {
                self.push_status_icon_event(owner_tid, icon_id);
            }
            MenuBarHit::None => {
                // Close any open dropdown
                if self.menu_bar.is_dropdown_open() {
                    self.menu_bar
                        .close_dropdown_with_compositor(&mut self.compositor);
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0,
                        0,
                        self.screen_width,
                        MENUBAR_HEIGHT + 1,
                    ));
                }
            }
        }
    }

    fn push_status_icon_event(&mut self, owner_tid: u32, icon_id: u32) {
        // Push event to the first window owned by this TID
        for win in &mut self.windows {
            if win.owner_tid == owner_tid {
                if win.events.len() < 256 {
                    win.events
                        .push_back([EVENT_STATUS_ICON_CLICK, icon_id, 0, 0, 0]);
                }
                break;
            }
        }
    }

    fn toggle_maximize(&mut self, win_id: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.id == win_id) {
            if self.windows[idx].maximized {
                // Restore
                if let Some((sx, sy, sw, sh)) = self.windows[idx].saved_bounds {
                    let layer_id = self.windows[idx].layer_id;
                    self.windows[idx].x = sx;
                    self.windows[idx].y = sy;
                    self.windows[idx].content_width = sw;
                    self.windows[idx].content_height = sh;
                    self.windows[idx].maximized = false;

                    let full_h = self.windows[idx].full_height();
                    self.compositor.move_layer(layer_id, sx, sy);
                    self.compositor.resize_layer(layer_id, sw, full_h);
                    self.render_window(win_id);
                }
            } else {
                // Maximize
                let x = self.windows[idx].x;
                let y = self.windows[idx].y;
                let cw = self.windows[idx].content_width;
                let ch = self.windows[idx].content_height;
                self.windows[idx].saved_bounds = Some((x, y, cw, ch));
                self.windows[idx].maximized = true;

                let new_x = 0i32;
                let new_y = MENUBAR_HEIGHT as i32 + 1;
                let new_w = self.screen_width;
                let new_ch = self.screen_height - MENUBAR_HEIGHT - 1 - TITLE_BAR_HEIGHT;

                let layer_id = self.windows[idx].layer_id;
                self.windows[idx].x = new_x;
                self.windows[idx].y = new_y;
                self.windows[idx].content_width = new_w;
                self.windows[idx].content_height = new_ch;

                let full_h = self.windows[idx].full_height();
                self.compositor.move_layer(layer_id, new_x, new_y);
                self.compositor.resize_layer(layer_id, new_w, full_h);
                self.render_window(win_id);
            }
        }
    }

    // ── Cursor ──────────────────────────────────────────────────────────

    /// Draw the software cursor onto the back buffer AFTER compositing.
    /// This should be called only if HW cursor is not available.
    pub fn draw_sw_cursor(&mut self) {
        if self.compositor.has_hw_cursor() {
            return;
        }

        let mx = self.mouse_x;
        let my = self.mouse_y;
        let stride = self.compositor.width() as usize;
        let bb = &mut self.compositor.back_buffer;

        // Save pixels under cursor
        for cy in 0..CURSOR_H as i32 {
            for cx in 0..CURSOR_W as i32 {
                let px = mx + cx;
                let py = my + cy;
                let idx = (cy * CURSOR_W as i32 + cx) as usize;
                if px >= 0
                    && py >= 0
                    && (px as u32) < self.screen_width
                    && (py as u32) < self.screen_height
                {
                    let di = py as usize * stride + px as usize;
                    self.cursor_save[idx] = bb[di];
                } else {
                    self.cursor_save[idx] = 0;
                }
            }
        }

        // Draw cursor
        for cy in 0..CURSOR_H as i32 {
            for cx in 0..CURSOR_W as i32 {
                let idx = (cy * CURSOR_W as i32 + cx) as usize;
                let pixel = CURSOR_BITMAP[idx];
                if pixel == 0 {
                    continue;
                }
                let color = if pixel == 1 { 0xFFFFFFFF } else { 0xFF000000 };
                let px = mx + cx;
                let py = my + cy;
                if px >= 0
                    && py >= 0
                    && (px as u32) < self.screen_width
                    && (py as u32) < self.screen_height
                {
                    let di = py as usize * stride + px as usize;
                    bb[di] = color;
                }
            }
        }

        self.cursor_drawn = true;
        self.prev_cursor_x = mx;
        self.prev_cursor_y = my;
    }

    /// Damage the cursor region (call before compositing to include cursor in flush).
    pub fn damage_cursor(&mut self) {
        if !self.compositor.has_hw_cursor() {
            // Damage old cursor position
            self.compositor.add_damage(Rect::new(
                self.prev_cursor_x,
                self.prev_cursor_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            ));
            // Damage new cursor position
            self.compositor.add_damage(Rect::new(
                self.mouse_x,
                self.mouse_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            ));
        }
    }

    /// Initialize the HW cursor: define arrow shape, show, and position.
    pub fn init_hw_cursor(&mut self) {
        self.compositor.define_hw_cursor(
            HW_ARROW_W,
            HW_ARROW_H,
            HW_ARROW_HOT_X,
            HW_ARROW_HOT_Y,
            &self.hw_arrow_pixels,
        );
        self.compositor.enable_hw_cursor();
        self.compositor
            .move_hw_cursor(self.mouse_x, self.mouse_y);
        self.compositor.flush_gpu();
        self.current_cursor = CursorShape::Arrow;
    }

    /// Change cursor shape if different from current. Defines new shape + flushes.
    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        if self.current_cursor == shape {
            return;
        }
        self.current_cursor = shape;
        if !self.compositor.has_hw_cursor() {
            return;
        }
        match shape {
            CursorShape::Arrow => {
                self.compositor.define_hw_cursor(
                    HW_ARROW_W,
                    HW_ARROW_H,
                    HW_ARROW_HOT_X,
                    HW_ARROW_HOT_Y,
                    &self.hw_arrow_pixels,
                );
            }
            CursorShape::ResizeNS => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NS_W,
                    HW_RESIZE_NS_H,
                    HW_RESIZE_NS_HOT_X,
                    HW_RESIZE_NS_HOT_Y,
                    &HW_RESIZE_NS,
                );
            }
            CursorShape::ResizeEW => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_EW_W,
                    HW_RESIZE_EW_H,
                    HW_RESIZE_EW_HOT_X,
                    HW_RESIZE_EW_HOT_Y,
                    &HW_RESIZE_EW,
                );
            }
            CursorShape::ResizeNWSE => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NWSE_W,
                    HW_RESIZE_NWSE_H,
                    HW_RESIZE_NWSE_HOT_X,
                    HW_RESIZE_NWSE_HOT_Y,
                    &HW_RESIZE_NWSE,
                );
            }
            CursorShape::ResizeNESW => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NESW_W,
                    HW_RESIZE_NESW_H,
                    HW_RESIZE_NESW_HOT_X,
                    HW_RESIZE_NESW_HOT_Y,
                    &HW_RESIZE_NESW,
                );
            }
            CursorShape::Move => {
                self.compositor.define_hw_cursor(
                    HW_MOVE_W,
                    HW_MOVE_H,
                    HW_MOVE_HOT_X,
                    HW_MOVE_HOT_Y,
                    &HW_MOVE,
                );
            }
        }
        self.compositor.flush_gpu();
    }

    /// Determine the correct cursor shape from a HitTest result.
    fn cursor_for_hit(&self, hit: HitTest) -> CursorShape {
        match hit {
            HitTest::ResizeTop | HitTest::ResizeBottom => CursorShape::ResizeNS,
            HitTest::ResizeLeft | HitTest::ResizeRight => CursorShape::ResizeEW,
            HitTest::ResizeTopLeft | HitTest::ResizeBottomRight => CursorShape::ResizeNWSE,
            HitTest::ResizeTopRight | HitTest::ResizeBottomLeft => CursorShape::ResizeNESW,
            HitTest::TitleBar => CursorShape::Arrow,
            _ => CursorShape::Arrow,
        }
    }

    // ── Compose ─────────────────────────────────────────────────────────

    /// Run the full compose cycle (damage → composite → cursor → flush).
    pub fn compose(&mut self) {
        // Composite layers into back buffer + flush to framebuffer
        self.compositor.compose();

        if self.compositor.has_hw_cursor() {
            // HW cursor: flush any queued GPU commands (e.g. CURSOR_MOVE)
            // that Compositor::compose() may have skipped due to empty damage.
            self.compositor.flush_gpu();
        } else {
            self.draw_sw_cursor();
            // Flush cursor region to FB
            let rect = Rect::new(
                self.mouse_x,
                self.mouse_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            )
            .clip_to_screen(self.screen_width, self.screen_height);
            if !rect.is_empty() {
                self.compositor.flush_region_pub(&rect);
                self.compositor.queue_gpu_update(
                    rect.x as u32,
                    rect.y as u32,
                    rect.width,
                    rect.height,
                );
                self.compositor.flush_gpu();
            }
        }
    }

    // ── IPC Window Operations ─────────────────────────────────────────────

    /// Create a window backed by a shared memory region.
    /// The app's surface pixels are read from the SHM during compositing.
    pub fn create_ipc_window(
        &mut self,
        app_tid: u32,
        content_w: u32,
        content_h: u32,
        flags: u32,
        shm_id: u32,
        shm_ptr: *mut u32,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let borderless = flags & WIN_FLAG_BORDERLESS != 0;

        // For SHM windows: the SHM holds the CONTENT area only.
        // The compositor creates a layer for the full window (title bar + content).
        // Title bar is drawn by compositor. Content is read from SHM.
        // To keep it simple: the layer is full-size, but we blit SHM content
        // into the layer's content area offset.
        let full_h = if borderless {
            content_h
        } else {
            content_h + TITLE_BAR_HEIGHT
        };

        // For IPC windows, we create a regular layer and store the SHM reference
        // in the WindowInfo. During present, we copy SHM content into the layer.
        // IPC windows are never opaque — apps may use alpha for transparency (e.g. dock).
        let layer_id = self.compositor.add_layer(
            100, // default position — apps can move later
            MENUBAR_HEIGHT as i32 + 40,
            content_w,
            full_h,
            false,
        );

        // No shadow for IPC windows
        let _ = borderless;

        let win = WindowInfo {
            id,
            layer_id,
            title: String::from("Window"),
            x: 100,
            y: MENUBAR_HEIGHT as i32 + 40,
            content_width: content_w,
            content_height: content_h,
            flags,
            owner_tid: app_tid,
            events: VecDeque::with_capacity(32),
            focused: false,
            saved_bounds: None,
            maximized: false,
            shm_id,
            shm_ptr,
        };

        self.windows.push(win);
        self.render_window(id);
        self.focus_window(id);

        id
    }

    /// Handle a "present" command for an IPC window.
    /// Copies the app's SHM content into the window layer's content area.
    pub fn present_ipc_window(&mut self, window_id: u32) {
        let win_idx = match self.windows.iter().position(|w| w.id == window_id) {
            Some(i) => i,
            None => return,
        };

        let shm_ptr = self.windows[win_idx].shm_ptr;
        if shm_ptr.is_null() {
            return;
        }

        let layer_id = self.windows[win_idx].layer_id;
        let cw = self.windows[win_idx].content_width;
        let ch = self.windows[win_idx].content_height;
        let borderless = self.windows[win_idx].is_borderless();
        let content_y = if borderless { 0 } else { TITLE_BAR_HEIGHT };

        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            let stride = cw;
            let src_count = (cw * ch) as usize;
            let src_slice = unsafe { core::slice::from_raw_parts(shm_ptr, src_count) };

            // Copy SHM content into the layer at content_y offset
            for row in 0..ch {
                let src_off = (row * cw) as usize;
                let dst_off = ((content_y + row) * stride) as usize;
                let w = cw as usize;
                let src_end = (src_off + w).min(src_slice.len());
                let dst_end = (dst_off + w).min(pixels.len());
                let copy_w = (src_end - src_off).min(dst_end - dst_off);
                if copy_w > 0 {
                    pixels[dst_off..dst_off + copy_w]
                        .copy_from_slice(&src_slice[src_off..src_off + copy_w]);
                }
            }
        }

        self.compositor.mark_layer_dirty(layer_id);

        // Add explicit damage for the window's layer bounds so the compositor
        // recomposes this region. mark_layer_dirty alone does NOT add damage.
        if let Some(layer) = self.compositor.get_layer(layer_id) {
            let bounds = layer.damage_bounds();
            self.compositor.add_damage(bounds);
        }
    }

    /// Forward all queued window events to apps via the event channel.
    /// Returns the events that need to be emitted (caller does the actual emit).
    pub fn drain_ipc_events(&mut self) -> Vec<[u32; 5]> {
        let mut out = Vec::new();
        for win in &mut self.windows {
            if win.owner_tid == 0 {
                continue; // local window, not IPC
            }
            while let Some(evt) = win.events.pop_front() {
                // Map internal event types to IPC protocol event types
                let ipc_type = match evt[0] {
                    EVENT_KEY_DOWN => proto::EVT_KEY_DOWN,
                    EVENT_KEY_UP => proto::EVT_KEY_UP,
                    EVENT_MOUSE_DOWN => proto::EVT_MOUSE_DOWN,
                    EVENT_MOUSE_UP => proto::EVT_MOUSE_UP,
                    EVENT_MOUSE_SCROLL => proto::EVT_MOUSE_SCROLL,
                    EVENT_RESIZE => proto::EVT_RESIZE,
                    EVENT_WINDOW_CLOSE => proto::EVT_WINDOW_CLOSE,
                    EVENT_MENU_ITEM => proto::EVT_MENU_ITEM,
                    EVENT_STATUS_ICON_CLICK => proto::EVT_STATUS_ICON_CLICK,
                    _ => continue,
                };
                out.push([ipc_type, win.id, evt[1], evt[2], evt[3]]);
            }
        }
        out
    }

    /// Process an IPC command from an app.
    /// Returns an optional response event to emit back.
    pub fn handle_ipc_command(&mut self, cmd: &[u32; 5]) -> Option<[u32; 5]> {
        match cmd[0] {
            proto::CMD_CREATE_WINDOW => {
                let app_tid = cmd[1];
                let width = cmd[2];
                let height = cmd[3];
                let shm_id_and_flags = cmd[4];
                let shm_id = shm_id_and_flags >> 16;
                let flags = shm_id_and_flags & 0xFFFF;

                if shm_id == 0 || width == 0 || height == 0 {
                    return None;
                }

                // Map the app's SHM into compositor's address space
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }

                let win_id = self.create_ipc_window(
                    app_tid,
                    width,
                    height,
                    flags,
                    shm_id,
                    shm_addr as *mut u32,
                );

                Some([proto::RESP_WINDOW_CREATED, win_id, shm_id, app_tid, 0])
            }
            proto::CMD_DESTROY_WINDOW => {
                let window_id = cmd[1];
                // Unmap SHM before destroying
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let shm_id = self.windows[idx].shm_id;
                    let app_tid = self.windows[idx].owner_tid;
                    if shm_id > 0 {
                        anyos_std::ipc::shm_unmap(shm_id);
                    }
                    self.destroy_window(window_id);
                    Some([proto::RESP_WINDOW_DESTROYED, window_id, app_tid, 0, 0])
                } else {
                    None
                }
            }
            proto::CMD_PRESENT => {
                let window_id = cmd[1];
                self.present_ipc_window(window_id);
                None
            }
            proto::CMD_SET_TITLE => {
                let window_id = cmd[1];
                let title_words = [cmd[2], cmd[3], cmd[4]];
                let title_bytes = proto::unpack_title(title_words);
                let len = title_bytes.iter().position(|&b| b == 0).unwrap_or(12);
                if let Ok(title) = core::str::from_utf8(&title_bytes[..len]) {
                    if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                        self.windows[idx].title = String::from(title);
                        self.render_window(window_id);
                    }
                }
                None
            }
            proto::CMD_MOVE_WINDOW => {
                let window_id = cmd[1];
                let x = cmd[2] as i32;
                let y = cmd[3] as i32;
                if let Some(idx) = self.windows.iter().position(|w| w.id == window_id) {
                    let layer_id = self.windows[idx].layer_id;
                    self.windows[idx].x = x;
                    self.windows[idx].y = y;
                    self.compositor.move_layer(layer_id, x, y);
                }
                None
            }
            proto::CMD_SET_MENU => {
                let window_id = cmd[1];
                let shm_id = cmd[2];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u8, 4096)
                };
                if let Some(def) = MenuBarDef::parse(data) {
                    // Get window title to use as the app-name menu
                    let app_name = self.windows.iter()
                        .find(|w| w.id == window_id)
                        .map(|w| w.title.as_str())
                        .unwrap_or("App");
                    let app_name_owned = String::from(app_name);
                    self.menu_bar.set_menu(window_id, def, &app_name_owned);
                    if self.focused_window == Some(window_id) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                }
                anyos_std::ipc::shm_unmap(shm_id);
                None
            }
            proto::CMD_ADD_STATUS_ICON => {
                let app_tid = cmd[1];
                let icon_id = cmd[2];
                let shm_id = cmd[3];
                if shm_id == 0 {
                    return None;
                }
                let shm_addr = anyos_std::ipc::shm_map(shm_id);
                if shm_addr == 0 {
                    return None;
                }
                let pixel_data = unsafe {
                    core::slice::from_raw_parts(shm_addr as *const u32, 256)
                };
                if self.menu_bar.add_status_icon(app_tid, icon_id, pixel_data, self.screen_width) {
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                }
                anyos_std::ipc::shm_unmap(shm_id);
                None
            }
            proto::CMD_REMOVE_STATUS_ICON => {
                let app_tid = cmd[1];
                let icon_id = cmd[2];
                if self.menu_bar.remove_status_icon(app_tid, icon_id, self.screen_width) {
                    self.draw_menubar();
                    self.compositor.add_damage(Rect::new(
                        0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                    ));
                }
                None
            }
            proto::CMD_UPDATE_MENU_ITEM => {
                let window_id = cmd[1];
                let item_id = cmd[2];
                let new_flags = cmd[3];
                if self.menu_bar.update_item_flags(window_id, item_id, new_flags) {
                    // If this window's menus are active, redraw the menubar
                    if self.focused_window == Some(window_id) {
                        self.draw_menubar();
                        self.compositor.add_damage(Rect::new(
                            0, 0, self.screen_width, MENUBAR_HEIGHT + 1,
                        ));
                    }
                    // If the dropdown is open for this window, re-render it
                    if let Some(ref dd) = self.menu_bar.open_dropdown {
                        if dd.owner_window_id == window_id {
                            self.menu_bar.render_dropdown(&mut self.compositor);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    // ── Accessors ───────────────────────────────────────────────────────

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn focused_window_id(&self) -> Option<u32> {
        self.focused_window
    }
}

// ── Helper: Menubar Clock ────────────────────────────────────────────────────

fn draw_clock_to_menubar(pixels: &mut [u32], stride: u32) {
    let mut time_buf = [0u8; 8];
    anyos_std::sys::time(&mut time_buf);
    let hour = time_buf[4];
    let minute = time_buf[5];

    let mut clock_str = [0u8; 5];
    clock_str[0] = b'0' + hour / 10;
    clock_str[1] = b'0' + hour % 10;
    clock_str[2] = b':';
    clock_str[3] = b'0' + minute / 10;
    clock_str[4] = b'0' + minute % 10;

    if let Ok(s) = core::str::from_utf8(&clock_str) {
        let h = MENUBAR_HEIGHT + 1;
        let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, s);
        let tx = stride as i32 - tw as i32 - 10;
        let fy = ((MENUBAR_HEIGHT as i32 - th as i32) / 2).max(0);

        // Clear clock area first
        for y in 0..MENUBAR_HEIGHT {
            for x in (tx - 4).max(0) as u32..stride {
                pixels[(y * stride + x) as usize] = COLOR_MENUBAR_BG;
            }
        }

        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, pixels, stride, h, tx, fy, COLOR_MENUBAR_TEXT, s,
        );
    }
}

// ── Helper: Rounded Rectangle Fill ──────────────────────────────────────────

fn fill_rounded_rect(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if r == 0 || w < r * 2 || h < r * 2 {
        fill_rect(pixels, stride, buf_h, x, y, w, h, color);
        return;
    }
    // Center body
    if h > r * 2 {
        fill_rect(pixels, stride, buf_h, x, y + r as i32, w, h - r * 2, color);
    }
    // Top and bottom bands
    if w > r * 2 {
        fill_rect(pixels, stride, buf_h, x + r as i32, y, w - r * 2, r, color);
        fill_rect(
            pixels,
            stride,
            buf_h,
            x + r as i32,
            y + h as i32 - r as i32,
            w - r * 2,
            r,
            color,
        );
    }
    // Corners
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            fill_rect(pixels, stride, buf_h, x + fs, y + dy as i32, fill_width, 1, color);
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + dy as i32,
                fill_width,
                1,
                color,
            );
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + fs,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
        }
    }
}

/// Fill rounded rect with only the top corners rounded.
fn fill_rounded_rect_top(
    pixels: &mut [u32],
    stride: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if r == 0 || w < r * 2 {
        let buf_h = h;
        fill_rect(pixels, stride, buf_h + y as u32, x, y, w, h, color);
        return;
    }
    let buf_h = stride; // assume square-ish
    // Body below rounded top
    if h > r {
        fill_rect(pixels, stride, buf_h, x, y + r as i32, w, h - r, color);
    }
    // Top band
    if w > r * 2 {
        fill_rect(pixels, stride, buf_h, x + r as i32, y, w - r * 2, r, color);
    }
    // Top corners only
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            fill_rect(pixels, stride, buf_h, x + fs, y + dy as i32, fill_width, 1, color);
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + dy as i32,
                fill_width,
                1,
                color,
            );
        }
    }
}

fn fill_rect(pixels: &mut [u32], stride: u32, buf_h: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let a = (color >> 24) & 0xFF;
    for row in 0..h as i32 {
        let py = y + row;
        if py < 0 || py >= buf_h as i32 {
            continue;
        }
        for col in 0..w as i32 {
            let px = x + col;
            if px < 0 || px >= stride as i32 {
                continue;
            }
            let idx = (py as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                if a >= 255 {
                    pixels[idx] = color;
                } else if a > 0 {
                    pixels[idx] = alpha_blend(color, pixels[idx]);
                }
            }
        }
    }
}

/// Integer square root (no floating point needed).
fn isqrt(n: i32) -> i32 {
    if n <= 0 {
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

/// Anti-aliased filled circle. Fills interior rows and alpha-blends edge pixels.
fn fill_circle(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    cx: i32,
    cy: i32,
    r: i32,
    color: u32,
) {
    if r <= 0 {
        return;
    }
    let r_sq = r * r;
    let cr = (color >> 16) & 0xFF;
    let cg = (color >> 8) & 0xFF;
    let cb = color & 0xFF;
    let ca = (color >> 24) & 0xFF;

    for dy in -r..=r {
        let y = cy + dy;
        if y < 0 || y >= buf_h as i32 {
            continue;
        }
        let inner_sq = r_sq - dy * dy;
        let dx_max = isqrt(inner_sq);

        // Fill solid interior row
        if dx_max > 1 {
            fill_rect(
                pixels,
                stride,
                buf_h,
                cx - dx_max + 1,
                y,
                (dx_max * 2 - 1) as u32,
                1,
                color,
            );
        }

        // Alpha-blend edge pixels
        for &dx in &[-dx_max - 1, -dx_max, dx_max, dx_max + 1] {
            let px = cx + dx;
            if px < 0 || px >= stride as i32 {
                continue;
            }
            let dist_sq = dx * dx + dy * dy;
            if dist_sq >= (r + 1) * (r + 1) || dist_sq <= (r - 1) * (r - 1) {
                continue;
            }
            let alpha = 255 * (r_sq + r - dist_sq) / (2 * r);
            if alpha <= 0 {
                continue;
            }
            let a = (alpha.min(255) as u32 * ca / 255).min(255);
            let aa_color = (a << 24) | (cr << 16) | (cg << 8) | cb;
            let idx = (y as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                if a >= 255 {
                    pixels[idx] = aa_color;
                } else {
                    pixels[idx] = alpha_blend(aa_color, pixels[idx]);
                }
            }
        }
    }
}

fn is_resize_edge(ht: HitTest) -> bool {
    matches!(
        ht,
        HitTest::ResizeTop
            | HitTest::ResizeBottom
            | HitTest::ResizeLeft
            | HitTest::ResizeRight
            | HitTest::ResizeTopLeft
            | HitTest::ResizeTopRight
            | HitTest::ResizeBottomLeft
            | HitTest::ResizeBottomRight
    )
}

fn compute_resize(
    edge: HitTest,
    start_x: i32,
    start_y: i32,
    start_w: u32,
    start_h: u32,
    dx: i32,
    dy: i32,
) -> (i32, i32, u32, u32) {
    let min_w: u32 = 100;
    let min_h: u32 = 60;
    let mut x = start_x;
    let mut y = start_y;
    let mut w = start_w;
    let mut h = start_h;

    match edge {
        HitTest::ResizeRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
        }
        HitTest::ResizeBottom => {
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
        }
        HitTest::ResizeTop => {
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        HitTest::ResizeBottomRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeBottomLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
            h = (start_h as i32 + dy).max(min_h as i32) as u32;
        }
        HitTest::ResizeTopRight => {
            w = (start_w as i32 + dx).max(min_w as i32) as u32;
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        HitTest::ResizeTopLeft => {
            let new_w = (start_w as i32 - dx).max(min_w as i32) as u32;
            x = start_x + start_w as i32 - new_w as i32;
            w = new_w;
            let new_h = (start_h as i32 - dy).max(min_h as i32) as u32;
            y = start_y + start_h as i32 - new_h as i32;
            h = new_h;
        }
        _ => {}
    }

    (x, y, w, h)
}
