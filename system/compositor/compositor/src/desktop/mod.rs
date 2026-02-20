//! Desktop manager — coordinates window management, menubar, wallpaper, cursor, and input.

pub mod cursors;
pub mod drawing;
pub mod input;
pub mod ipc;
pub mod theme;
pub mod window;

// Re-export public API used by main.rs and other crates
pub use cursors::CursorShape;
pub use theme::set_theme;
pub use window::{
    copy_shm_to_pixels, pre_render_chrome_ex,
    MENUBAR_HEIGHT, TITLE_BAR_HEIGHT, WIN_FLAG_BORDERLESS,
};

use alloc::vec;
use alloc::vec::Vec;

use crate::compositor::{Compositor, Rect};
use crate::menu::MenuBar;

use cursors::CURSOR_H;
use cursors::CURSOR_W;
use theme::*;
use window::*;

// ── Desktop ────────────────────────────────────────────────────────────────

pub struct Desktop {
    pub compositor: Compositor,

    // Desktop layers
    pub(crate) bg_layer_id: u32,
    pub(crate) menubar_layer_id: u32,

    // Windows
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) focused_window: Option<u32>,
    pub(crate) next_window_id: u32,

    // Mouse state
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub(crate) mouse_buttons: u32,

    // SW cursor
    pub(crate) cursor_save: Vec<u32>,
    pub(crate) cursor_drawn: bool,
    pub(crate) prev_cursor_x: i32,
    pub(crate) prev_cursor_y: i32,

    // Interaction
    pub(crate) dragging: Option<DragState>,
    pub(crate) resizing: Option<ResizeState>,

    // Screen
    pub screen_width: u32,
    pub screen_height: u32,

    // Menubar clock
    pub(crate) last_clock_min: u32,

    // HW cursor
    pub(crate) current_cursor: CursorShape,
    pub(crate) hw_arrow_pixels: Vec<u32>,

    // Menu bar system
    pub(crate) menu_bar: MenuBar,

    // Button hover/press animation
    pub(crate) btn_hover: Option<(u32, u8)>,
    pub(crate) btn_pressed: Option<(u32, u8)>,
    pub(crate) btn_anims: anyos_std::anim::AnimSet,

    /// Whether GPU acceleration is available (cached at init).
    pub(crate) has_gpu_accel: bool,
    /// Whether GPU hardware cursor is available (cached at init).
    pub(crate) has_hw_cursor: bool,
    /// Per-app subscription IDs for targeted event delivery: (tid, sub_id).
    pub(crate) app_subs: Vec<(u32, u32)>,
    /// Deferred wallpaper reload after resolution change.
    pub(crate) wallpaper_pending: bool,
    /// Tray icon events for windowless apps.
    pub(crate) tray_ipc_events: Vec<(Option<u32>, [u32; 5])>,
    /// Current wallpaper path (for reload on resolution change).
    pub(crate) wallpaper_path: [u8; 128],
    pub(crate) wallpaper_path_len: usize,
}

impl Desktop {
    pub fn new(fb_ptr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        let mut compositor = Compositor::new(fb_ptr, width, height, pitch);

        // Background layer (bottom)
        let bg_id = compositor.add_layer(0, 0, width, height, true);

        // Menubar layer (always on top, above all windows)
        let mb_id = compositor.add_layer(0, 0, width, MENUBAR_HEIGHT + 1, false);
        if let Some(layer) = compositor.get_layer_mut(mb_id) {
            layer.has_shadow = true;
        }

        // Convert arrow cursor bitmap to ARGB
        let arrow_pixels = Self::create_arrow_pixels();

        let mut desktop = Desktop {
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
            btn_hover: None,
            btn_pressed: None,
            btn_anims: anyos_std::anim::AnimSet::new(),
            has_gpu_accel: anyos_std::ui::window::gpu_has_accel(),
            has_hw_cursor: anyos_std::ui::window::gpu_has_hw_cursor(),
            app_subs: Vec::with_capacity(16),
            wallpaper_pending: false,
            tray_ipc_events: Vec::new(),
            wallpaper_path: [0u8; 128],
            wallpaper_path_len: 0,
        };

        if desktop.has_gpu_accel {
            desktop.compositor.enable_gpu_accel();
            // Initialize VRAM allocator if enough off-screen VRAM is available
            let vram_total = anyos_std::ipc::gpu_vram_size();
            if vram_total > 0 {
                desktop.compositor.init_vram_allocator(vram_total);
            }
        }

        desktop
    }

    // ── Initialization ─────────────────────────────────────────────────

    /// Whether GPU 2D acceleration is available.
    pub fn has_gpu_accel(&self) -> bool {
        self.has_gpu_accel
    }

    /// Whether GPU hardware cursor is available.
    pub fn has_hw_cursor(&self) -> bool {
        self.has_hw_cursor
    }

    /// Set the initial cursor position.
    pub fn set_cursor_pos(&mut self, x: i32, y: i32) {
        self.mouse_x = x.clamp(0, self.screen_width as i32 - 1);
        self.mouse_y = y.clamp(0, self.screen_height as i32 - 1);
        self.prev_cursor_x = self.mouse_x;
        self.prev_cursor_y = self.mouse_y;
    }

    /// Draw the initial desktop (background + menubar).
    pub fn init(&mut self) {
        self.load_user_wallpaper();
        self.draw_menubar();
        self.compositor.damage_all();
    }

    // ── Wallpaper ──────────────────────────────────────────────────────

    /// Load wallpaper from per-user preferences file.
    fn load_user_wallpaper(&mut self) {
        use anyos_std::fs;
        use anyos_std::process;

        let uid = process::getuid() as u32;
        let mut path_found = false;

        let fd = fs::open("/System/users/wallpapers", 0);
        if fd != u32::MAX {
            let mut buf = [0u8; 512];
            let n = fs::read(fd, &mut buf) as usize;
            fs::close(fd);

            let data = &buf[..n];
            let mut pos = 0;
            while pos < data.len() {
                let line_end = data[pos..].iter().position(|&b| b == b'\n')
                    .map(|p| pos + p).unwrap_or(data.len());
                let line = &data[pos..line_end];
                pos = line_end + 1;

                if let Some(colon) = line.iter().position(|&b| b == b':') {
                    let uid_str = &line[..colon];
                    let mut parsed_uid: u32 = 0;
                    let mut valid = uid_str.len() > 0;
                    for &b in uid_str {
                        if b >= b'0' && b <= b'9' {
                            parsed_uid = parsed_uid * 10 + (b - b'0') as u32;
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    if valid && parsed_uid == uid {
                        let path_bytes = &line[colon + 1..];
                        let len = path_bytes.len().min(127);
                        self.wallpaper_path[..len].copy_from_slice(&path_bytes[..len]);
                        self.wallpaper_path_len = len;
                        path_found = true;
                        break;
                    }
                }
            }
        }

        if path_found {
            let mut tmp = [0u8; 128];
            let len = self.wallpaper_path_len;
            tmp[..len].copy_from_slice(&self.wallpaper_path[..len]);
            if let Ok(path) = core::str::from_utf8(&tmp[..len]) {
                if self.load_wallpaper(path) {
                    return;
                }
            }
        }

        let default_path = b"/media/wallpapers/default.png";
        self.wallpaper_path[..default_path.len()].copy_from_slice(default_path);
        self.wallpaper_path_len = default_path.len();
        if !self.load_wallpaper("/media/wallpapers/default.png") {
            self.draw_gradient_background();
        }
    }

    /// Process deferred wallpaper reload (after resolution change).
    pub fn process_deferred_wallpaper(&mut self) {
        if !self.wallpaper_pending {
            return;
        }
        self.wallpaper_pending = false;
        let path = core::str::from_utf8(&self.wallpaper_path[..self.wallpaper_path_len])
            .unwrap_or("/media/wallpapers/default.png");
        let mut path_buf = [0u8; 128];
        let len = path.len().min(127);
        path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
        let path_str = core::str::from_utf8(&path_buf[..len]).unwrap_or("");
        if self.load_wallpaper(path_str) {
            self.compositor.damage_all();
        }
    }

    /// Load a wallpaper image and draw it to the background layer.
    pub(crate) fn load_wallpaper(&mut self, path: &str) -> bool {
        use anyos_std::fs;

        anyos_std::println!("compositor: loading wallpaper from '{}'...", path);

        let fd = fs::open(path, 0);
        if fd == u32::MAX { return false; }

        let mut stat_buf = [0u32; 4];
        if fs::fstat(fd, &mut stat_buf) == u32::MAX {
            fs::close(fd);
            return false;
        }
        let file_size = stat_buf[1] as usize;
        if file_size == 0 || file_size > 8 * 1024 * 1024 {
            fs::close(fd);
            return false;
        }

        let mut data = vec![0u8; file_size];
        let bytes_read = fs::read(fd, &mut data) as usize;
        fs::close(fd);
        if bytes_read == 0 { return false; }

        let info = match libimage_client::probe(&data[..bytes_read]) {
            Some(i) => i,
            None => return false,
        };

        let pixel_count = (info.width * info.height) as usize;
        if pixel_count > 4 * 1024 * 1024 { return false; }

        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode(&data[..bytes_read], &mut pixels, &mut scratch).is_err() {
            return false;
        }
        drop(scratch);
        drop(data);

        let sw = self.screen_width;
        let sh = self.screen_height;

        if info.width == sw && info.height == sh {
            if let Some(bg_pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
                let copy_len = bg_pixels.len().min(pixel_count);
                bg_pixels[..copy_len].copy_from_slice(&pixels[..copy_len]);
            }
        } else {
            let dst_count = (sw * sh) as usize;
            let mut dst = vec![0u32; dst_count];
            if !libimage_client::scale_image(
                &pixels, info.width, info.height,
                &mut dst, sw, sh,
                libimage_client::MODE_COVER,
            ) {
                return false;
            }
            drop(pixels);

            if let Some(bg_pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
                let copy_len = bg_pixels.len().min(dst_count);
                bg_pixels[..copy_len].copy_from_slice(&dst[..copy_len]);
            }
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

    // ── Menubar ────────────────────────────────────────────────────────

    /// Draw the menubar layer.
    pub fn draw_menubar(&mut self) {
        let w = self.screen_width;
        let h = MENUBAR_HEIGHT + 1;
        if let Some(pixels) = self.compositor.layer_pixels(self.menubar_layer_id) {
            for y in 0..MENUBAR_HEIGHT {
                for x in 0..w {
                    pixels[(y * w + x) as usize] = color_menubar_bg();
                }
            }
            for x in 0..w {
                pixels[(MENUBAR_HEIGHT * w + x) as usize] = color_menubar_border();
            }

            let (_, fh) = anyos_std::ui::window::font_measure(FONT_ID_BOLD, FONT_SIZE, "anyOS");
            let fy = ((MENUBAR_HEIGHT as i32 - fh as i32) / 2).max(0);
            anyos_std::ui::window::font_render_buf(
                FONT_ID_BOLD, FONT_SIZE, pixels, w, h, 10, fy, color_menubar_text(), "anyOS",
            );

            self.menu_bar.render_titles(pixels, w, h);
            draw_clock_to_menubar(pixels, w);
            self.menu_bar.render_status_icons(pixels, w);
        }
        self.compositor.mark_layer_dirty(self.menubar_layer_id);
    }

    /// Show or hide the menubar layer.
    pub fn set_menubar_visible(&mut self, visible: bool) {
        self.compositor.set_layer_visible(self.menubar_layer_id, visible);
    }

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
            self.compositor.add_damage(Rect::new(
                (w as i32 - 60).max(0),
                0,
                60,
                MENUBAR_HEIGHT + 1,
            ));
        }
    }

    // ── Resolution ─────────────────────────────────────────────────────

    /// Handle a resolution change event.
    pub fn handle_resolution_change(&mut self, new_w: u32, new_h: u32) {
        if new_w == self.screen_width && new_h == self.screen_height {
            return;
        }
        anyos_std::println!(
            "compositor: resolution changed {}x{} -> {}x{}",
            self.screen_width, self.screen_height, new_w, new_h
        );

        let pitch = match anyos_std::ipc::map_framebuffer() {
            Some(info) => info.pitch,
            None => new_w * 4,
        };

        self.screen_width = new_w;
        self.screen_height = new_h;

        self.mouse_x = self.mouse_x.min(new_w as i32 - 1);
        self.mouse_y = self.mouse_y.min(new_h as i32 - 1);

        self.compositor.resize_fb(new_w, new_h, pitch);
        self.compositor.resize_layer(self.bg_layer_id, new_w, new_h);
        self.compositor
            .resize_layer(self.menubar_layer_id, new_w, MENUBAR_HEIGHT + 1);

        self.draw_gradient_background();
        self.wallpaper_pending = true;

        self.draw_menubar();
        self.compositor.damage_all();
    }

    // ── Compose ────────────────────────────────────────────────────────

    /// Run the full compose cycle (damage → composite → cursor → flush).
    pub fn compose(&mut self) {
        self.compositor.compose();

        if self.compositor.has_hw_cursor() {
            self.compositor.flush_gpu();
        } else {
            self.draw_sw_cursor();
            let rect = Rect::new(
                self.mouse_x,
                self.mouse_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            )
            .clip_to_screen(self.screen_width, self.screen_height);
            if !rect.is_empty() {
                self.compositor.flush_cursor_region(&rect);
            }
        }
    }

    // ── Accessors ──────────────────────────────────────────────────────

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn focused_window_id(&self) -> Option<u32> {
        self.focused_window
    }
}

// ── Helper: Menubar Clock ──────────────────────────────────────────────────

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

        for y in 0..MENUBAR_HEIGHT {
            for x in (tx - 4).max(0) as u32..stride {
                pixels[(y * stride + x) as usize] = color_menubar_bg();
            }
        }

        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, pixels, stride, h, tx, fy, color_menubar_text(), s,
        );
    }
}
