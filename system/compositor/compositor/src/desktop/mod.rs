//! Desktop manager — coordinates window management, menubar, wallpaper, cursor, and input.

pub mod crash_dialog;
pub mod cursors;
pub mod desktop_icons;
pub mod drawing;
pub mod input;
pub mod ipc;
pub mod theme;
pub mod volume_hud;
pub mod window;

// Re-export public API used by main.rs and other crates
pub use cursors::CursorShape;
pub use theme::{set_theme, set_font_smoothing};
pub use window::{
    copy_shm_to_pixels, pre_render_chrome_ex,
    menubar_height, title_bar_height, WIN_FLAG_BORDERLESS,
};

use alloc::vec;
use alloc::vec::Vec;

use crate::compositor::{Compositor, Rect};
use crate::menu::MenuBar;

use cursors::CURSOR_H;
use cursors::CURSOR_W;
use theme::*;
use window::*;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Format a wallpaper preference entry: "UID:PATH\n"
/// Returns (buffer, length).
fn format_wallpaper_entry(uid: u32, path: &[u8]) -> ([u8; 160], usize) {
    let mut buf = [0u8; 160];
    let mut pos = 0;

    // Write UID digits
    if uid == 0 {
        buf[pos] = b'0';
        pos += 1;
    } else {
        let mut digits = [0u8; 10];
        let mut d = 0;
        let mut n = uid;
        while n > 0 {
            digits[d] = b'0' + (n % 10) as u8;
            d += 1;
            n /= 10;
        }
        while d > 0 {
            d -= 1;
            buf[pos] = digits[d];
            pos += 1;
        }
    }

    buf[pos] = b':';
    pos += 1;

    let plen = path.len().min(buf.len() - pos - 1);
    buf[pos..pos + plen].copy_from_slice(&path[..plen]);
    pos += plen;

    buf[pos] = b'\n';
    pos += 1;

    (buf, pos)
}

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
    /// Current keyboard modifier state (Shift=1, Ctrl=2, Alt=4), updated on key events.
    pub(crate) current_modifiers: u32,

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
    /// Clipboard contents (stored as raw bytes).
    pub(crate) clipboard_data: Vec<u8>,
    /// Clipboard format: 0 = text/plain, 1 = text/uri-list.
    pub(crate) clipboard_format: u32,
    /// Active crash dialogs (internal windows showing crash info).
    pub(crate) crash_dialogs: Vec<crash_dialog::CrashDialog>,
    /// Volume HUD overlay (centered-bottom).
    pub(crate) volume_hud: volume_hud::VolumeHud,
    /// Cascading auto-placement state for new windows.
    pub(crate) cascade_x: i32,
    pub(crate) cascade_y: i32,
    /// Frame ACK queue: (sub_id, window_id) pairs to emit after compose.
    /// Populated during compose(), drained by render thread via evt_chan_emit_to.
    pub(crate) frame_ack_queue: Vec<(u32, u32)>,

    /// Set to true when the user selects "Log Out" from the system menu.
    /// The management loop checks this flag and initiates the logout sequence.
    pub(crate) logout_requested: bool,

    /// Shutdown/restart mode: 0 = none, 1 = power off, 2 = reboot.
    /// Set by the system menu, consumed by the management loop.
    pub(crate) shutdown_mode: u8,

    /// Menu logo (white variant for dark mode, ARGB pixels).
    pub(crate) logo_white: Vec<u32>,
    /// Menu logo (black variant for light mode, ARGB pixels).
    pub(crate) logo_black: Vec<u32>,
    /// Logo image dimensions (both variants have same size).
    pub(crate) logo_w: u32,
    pub(crate) logo_h: u32,

    /// Desktop drive icons manager (macOS-style mounted volume icons).
    pub(crate) desktop_icons: desktop_icons::DesktopIconManager,
    /// Cached clean wallpaper pixels (without icons), to avoid reloading from disk.
    pub(crate) wallpaper_pixel_cache: Vec<u32>,
    /// Double-click detection: timestamp of last click (uptime_ms).
    pub(crate) last_click_time: u32,
    /// Double-click detection: x position of last click.
    pub(crate) last_click_x: i32,
    /// Double-click detection: y position of last click.
    pub(crate) last_click_y: i32,

    /// VNC injection: previous pointer button mask (RFB convention).
    /// Tracks which buttons were pressed on the last `inject_pointer_event` call
    /// so we can synthesise press/release pairs for changed bits only.
    pub(crate) vnc_buttons: u8,
}

impl Desktop {
    pub fn new(fb_ptr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        let mut compositor = Compositor::new(fb_ptr, width, height, pitch);

        // Background layer (bottom)
        let bg_id = compositor.add_layer(0, 0, width, height, true);

        // Menubar layer (always on top, above all windows)
        let mb_id = compositor.add_layer(0, 0, width, menubar_height() + 1, false);
        if let Some(layer) = compositor.get_layer_mut(mb_id) {
            layer.has_shadow = true;
        }

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
            current_modifiers: 0,
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
            clipboard_data: Vec::new(),
            clipboard_format: 0,
            crash_dialogs: Vec::new(),
            volume_hud: volume_hud::VolumeHud::new(),
            cascade_x: 120,
            cascade_y: menubar_height() as i32 + 50,
            frame_ack_queue: Vec::new(),
            logout_requested: false,
            shutdown_mode: 0,
            logo_white: Vec::new(),
            logo_black: Vec::new(),
            logo_w: 0,
            logo_h: 0,
            desktop_icons: desktop_icons::DesktopIconManager::new(),
            wallpaper_pixel_cache: Vec::new(),
            last_click_time: 0,
            last_click_x: 0,
            last_click_y: 0,
            vnc_buttons: 0,
        };

        if desktop.has_gpu_accel {
            desktop.compositor.enable_gpu_accel();
            // Try to register back buffer as GPU GMR for DMA transfers.
            // If successful, flush_region() skips CPU memcpy to VRAM.
            desktop.compositor.try_enable_gmr();
            // Initialize VRAM allocator if enough off-screen VRAM is available
            if !desktop.compositor.gmr_active {
                let vram_total = anyos_std::ipc::gpu_vram_size();
                if vram_total > 0 {
                    desktop.compositor.init_vram_allocator(vram_total);
                }
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

    /// Draw the initial desktop (background + menubar). Desktop icons are
    /// initialized separately after login via `init_desktop_icons()`.
    pub fn init(&mut self) {
        self.load_menu_logos();
        self.load_user_wallpaper();
        self.draw_menubar();
        self.compositor.damage_all();
    }

    /// Initialize desktop drive icons. Call once after login succeeds.
    pub fn init_desktop_icons(&mut self) {
        let sw = self.screen_width;
        self.desktop_icons.init(sw);
        self.render_desktop_icons();
        self.compositor.damage_all();
    }

    /// Render desktop icons onto the background layer.
    pub(crate) fn render_desktop_icons(&mut self) {
        let sw = self.screen_width;
        let sh = self.screen_height;
        let bg_id = self.bg_layer_id;
        if let Some(bg_pixels) = self.compositor.layer_pixels(bg_id) {
            self.desktop_icons.render_icons(bg_pixels, sw, sh);
        }
        self.compositor.mark_layer_dirty(bg_id);
    }

    /// Poll mounts and update desktop icons. Returns true if icons changed.
    pub fn poll_desktop_icons(&mut self) -> bool {
        let sw = self.screen_width;
        if self.desktop_icons.poll_mounts(sw) {
            // Mounts changed — reload wallpaper region and re-render icons
            self.reload_wallpaper_and_icons();
            true
        } else {
            false
        }
    }

    /// Reload wallpaper then render icons on top (used after icon changes).
    pub(crate) fn reload_wallpaper_and_icons(&mut self) {
        // Restore clean wallpaper from in-memory cache (no disk I/O!)
        if !self.wallpaper_pixel_cache.is_empty() {
            if let Some(bg_pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
                let copy_len = bg_pixels.len().min(self.wallpaper_pixel_cache.len());
                bg_pixels[..copy_len].copy_from_slice(&self.wallpaper_pixel_cache[..copy_len]);
            }
        } else {
            self.draw_gradient_background();
        }
        // Re-render icons on fresh wallpaper
        self.render_desktop_icons();
        self.compositor.damage_all();
    }

    /// Load menu logo PNGs from /System/media/ and scale to fit the menubar.
    fn load_menu_logos(&mut self) {
        self.load_one_logo("/System/media/menulogo_white.png", true);
        self.load_one_logo("/System/media/menulogo_black.png", false);
    }

    fn load_one_logo(&mut self, path: &str, is_white: bool) {
        use anyos_std::fs;

        let fd = fs::open(path, 0);
        if fd == u32::MAX { return; }
        let mut stat_buf = [0u32; 4];
        if fs::fstat(fd, &mut stat_buf) == u32::MAX {
            fs::close(fd);
            return;
        }
        let file_size = stat_buf[1] as usize;
        if file_size == 0 || file_size > 32 * 1024 {
            fs::close(fd);
            return;
        }

        let mut data = vec![0u8; file_size];
        let n = fs::read(fd, &mut data) as usize;
        fs::close(fd);
        if n == 0 { return; }

        let info = match libimage_client::probe(&data[..n]) {
            Some(i) => i,
            None => return,
        };
        let src_w = info.width;
        let src_h = info.height;
        let pixel_count = (src_w * src_h) as usize;
        if pixel_count == 0 || pixel_count > 4096 { return; }

        let mut pixels = vec![0u32; pixel_count];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        if libimage_client::decode(&data[..n], &mut pixels, &mut scratch).is_err() {
            return;
        }

        // Scale to fit menubar (14px height — compact, like macOS Apple logo)
        let target_h: u32 = 14;
        let target_w = (src_w * target_h + src_h / 2) / src_h.max(1);
        if target_w == 0 { return; }

        let mut scaled = vec![0u32; (target_w * target_h) as usize];
        // Use libimage's area-averaging scaler for proper antialiasing
        if !libimage_client::scale_image(
            &pixels, src_w, src_h,
            &mut scaled, target_w, target_h,
            libimage_client::MODE_SCALE,
        ) {
            return;
        }

        if is_white {
            self.logo_white = scaled;
        } else {
            self.logo_black = scaled;
        }
        self.logo_w = target_w;
        self.logo_h = target_h;

        anyos_std::println!("compositor: loaded menu logo '{}' ({}x{} → {}x{})",
            path, src_w, src_h, target_w, target_h);
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

    /// Save the current wallpaper path to the per-user preferences file.
    fn save_user_wallpaper(&self) {
        use anyos_std::fs;
        use anyos_std::process;

        if self.wallpaper_path_len == 0 {
            return;
        }

        let uid = process::getuid() as u32;
        let wp_path = &self.wallpaper_path[..self.wallpaper_path_len];

        // Read existing file to preserve other users' entries
        let mut existing = [0u8; 512];
        let mut existing_len = 0usize;
        let fd = fs::open("/System/users/wallpapers", 0);
        if fd != u32::MAX {
            let n = fs::read(fd, &mut existing);
            if (n as i32) > 0 {
                existing_len = n as usize;
            }
            fs::close(fd);
        }

        // Build new file content: keep lines for other UIDs, replace/add ours
        let mut out = [0u8; 512];
        let mut out_pos = 0usize;
        let mut replaced = false;

        if existing_len > 0 {
            let data = &existing[..existing_len];
            let mut pos = 0;
            while pos < data.len() {
                let line_end = data[pos..].iter().position(|&b| b == b'\n')
                    .map(|p| pos + p).unwrap_or(data.len());
                let line = &data[pos..line_end];
                pos = line_end + 1;

                if line.is_empty() {
                    continue;
                }

                // Check if this line is for our UID
                let is_our_uid = if let Some(colon) = line.iter().position(|&b| b == b':') {
                    let uid_str = &line[..colon];
                    let mut parsed_uid: u32 = 0;
                    let mut valid = !uid_str.is_empty();
                    for &b in uid_str {
                        if b >= b'0' && b <= b'9' {
                            parsed_uid = parsed_uid * 10 + (b - b'0') as u32;
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    valid && parsed_uid == uid
                } else {
                    false
                };

                if is_our_uid {
                    // Replace with our new entry
                    let (entry, elen) = format_wallpaper_entry(uid, wp_path);
                    let elen = elen.min(out.len() - out_pos);
                    out[out_pos..out_pos + elen].copy_from_slice(&entry[..elen]);
                    out_pos += elen;
                    replaced = true;
                } else {
                    // Keep other user's line
                    let llen = line.len().min(out.len() - out_pos - 1);
                    out[out_pos..out_pos + llen].copy_from_slice(&line[..llen]);
                    out_pos += llen;
                    if out_pos < out.len() {
                        out[out_pos] = b'\n';
                        out_pos += 1;
                    }
                }
            }
        }

        if !replaced {
            let (entry, elen) = format_wallpaper_entry(uid, wp_path);
            let elen = elen.min(out.len() - out_pos);
            out[out_pos..out_pos + elen].copy_from_slice(&entry[..elen]);
            out_pos += elen;
        }

        // Write back
        let wfd = fs::open(
            "/System/users/wallpapers",
            fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC,
        );
        if wfd != u32::MAX {
            fs::write(wfd, &out[..out_pos]);
            fs::close(wfd);
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
            // Re-render desktop icons on top of the fresh wallpaper
            self.render_desktop_icons();
            self.compositor.damage_all();
        }
    }

    /// Load a wallpaper image and draw it to the background layer.
    pub(crate) fn load_wallpaper(&mut self, path: &str) -> bool {
        use anyos_std::fs;

        anyos_std::println!("compositor: loading wallpaper from '{}'...", path);

        // Free old wallpaper cache first to reduce heap pressure before
        // allocating large temporary buffers for the new image.
        let had_cache = !self.wallpaper_pixel_cache.is_empty();
        if had_cache {
            self.wallpaper_pixel_cache = Vec::new();
        }

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

        // Cache the clean wallpaper pixels (without icons) for fast restore
        if let Some(bg_pixels) = self.compositor.layer_pixels(self.bg_layer_id) {
            self.wallpaper_pixel_cache = bg_pixels.to_vec();
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
            // Cache the clean gradient pixels
            self.wallpaper_pixel_cache = pixels.to_vec();
        }
    }

    // ── Menubar ────────────────────────────────────────────────────────

    /// Draw the menubar layer.
    pub fn draw_menubar(&mut self) {
        let w = self.screen_width;
        let mb_h = menubar_height();
        let h = mb_h + 1;
        if let Some(pixels) = self.compositor.layer_pixels(self.menubar_layer_id) {
            for y in 0..mb_h {
                for x in 0..w {
                    pixels[(y * w + x) as usize] = color_menubar_bg();
                }
            }
            for x in 0..w {
                pixels[(mb_h * w + x) as usize] = color_menubar_border();
            }

            // Render logo (white for dark mode, black for light mode)
            let logo = if is_light() { &self.logo_black } else { &self.logo_white };
            if !logo.is_empty() && self.logo_w > 0 && self.logo_h > 0 {
                let lx = 10i32;
                let ly = ((mb_h as i32 - self.logo_h as i32) / 2).max(0);
                for row in 0..self.logo_h {
                    let py = ly + row as i32;
                    if py < 0 || py >= mb_h as i32 { continue; }
                    for col in 0..self.logo_w {
                        let px = lx + col as i32;
                        if px < 0 || px >= w as i32 { continue; }
                        let src = logo[(row * self.logo_w + col) as usize];
                        let a = (src >> 24) & 0xFF;
                        if a == 0 { continue; }
                        let didx = (py as u32 * w + px as u32) as usize;
                        if didx < pixels.len() {
                            if a >= 255 {
                                pixels[didx] = src;
                            } else {
                                pixels[didx] = crate::compositor::alpha_blend(src, pixels[didx]);
                            }
                        }
                    }
                }
            } else {
                // Fallback: render "anyOS" text if logos not loaded
                let fs = scaled_font_size();
                let (_, fh) = anyos_std::ui::window::font_measure(FONT_ID_BOLD, fs, "anyOS");
                let fy = ((mb_h as i32 - fh as i32) / 2).max(0);
                anyos_std::ui::window::font_render_buf(
                    FONT_ID_BOLD, fs, pixels, w, h, 10, fy, color_menubar_text(), "anyOS",
                );
            }

            // Render system menu highlight if the system dropdown is open
            if self.menu_bar.system_menu_open {
                let highlight_w = crate::menu::types::SYSTEM_MENU_WIDTH as u32;
                for y in 0..mb_h {
                    for x in 0..highlight_w.min(w) {
                        let idx = (y * w + x) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = crate::compositor::alpha_blend(
                                crate::menu::types::color_menubar_highlight(), pixels[idx],
                            );
                        }
                    }
                }
            }

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

    /// Update the clock display. Returns `true` if the minute changed and
    /// damage was added (caller should compose). Returns `false` if no change.
    pub fn update_clock(&mut self) -> bool {
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
                menubar_height() + 1,
            ));
            true
        } else {
            false
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
            .resize_layer(self.menubar_layer_id, new_w, menubar_height() + 1);

        self.draw_gradient_background();
        self.wallpaper_pending = true;

        self.draw_menubar();
        self.compositor.damage_all();
    }

    /// Handle a runtime DPI scale factor change.
    ///
    /// Resizes the menubar layer (its pixel buffer must match the new scaled
    /// height) and all window layers (title bar height changes), then forces
    /// a full redraw.
    pub fn handle_scale_change(&mut self) {
        // Resize menubar layer to new scaled height.
        let sw = self.screen_width;
        self.compositor
            .resize_layer(self.menubar_layer_id, sw, menubar_height() + 1);

        // Resize every window layer — title bar height changed, so the
        // compositor-owned pixel buffer for each window needs to grow/shrink.
        for win in &self.windows {
            let new_full_h = win.full_height();
            self.compositor
                .resize_layer(win.layer_id, win.content_width, new_full_h);
        }

        // Invalidate all shadow caches (spread/offset changed).
        for layer in self.compositor.layers.iter_mut() {
            layer.shadow_cache = None;
        }

        self.draw_menubar();
        self.compositor.damage_all();
    }

    // ── Compose ────────────────────────────────────────────────────────

    /// Run the full compose cycle (damage → composite → cursor → flush → collect ACKs).
    /// Returns `true` if actual damage was composited (pixels changed on screen).
    pub fn compose(&mut self) -> bool {
        let had_damage = self.compositor.compose();

        if self.compositor.has_hw_cursor() {
            self.compositor.flush_gpu();
        } else {
            // Only redraw + flush SW cursor if something changed:
            // cursor moved, or compositing updated the back buffer
            let cursor_moved = self.mouse_x != self.prev_cursor_x
                || self.mouse_y != self.prev_cursor_y;

            if cursor_moved || had_damage {
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

        // Collect frame ACKs for windows that presented content in this frame.
        // The render thread emits these via evt_chan_emit_to after releasing the lock.
        if had_damage {
            for win in &mut self.windows {
                if win.needs_frame_ack && win.owner_tid != 0 {
                    if let Some((_, sub_id)) = self.app_subs.iter().find(|(t, _)| *t == win.owner_tid) {
                        self.frame_ack_queue.push((*sub_id, win.id));
                    }
                    win.needs_frame_ack = false;
                }
            }
        }

        had_damage
    }

    // ── Accessors ──────────────────────────────────────────────────────

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn focused_window_id(&self) -> Option<u32> {
        self.focused_window
    }

    // ── Crash Dialogs ──────────────────────────────────────────────────

    /// Show a crash dialog for a terminated thread.
    /// `report_buf` contains the raw CrashReport bytes from the kernel.
    pub fn show_crash_dialog(&mut self, tid: u32, _exit_code: u32, report_buf: &[u8]) {
        let report = unsafe {
            &*(report_buf.as_ptr() as *const crash_dialog::CrashReport)
        };

        let content_h = crash_dialog::CrashDialog::from_report(0, report).content_height();
        let flags = WIN_FLAG_NO_MINIMIZE | WIN_FLAG_NO_MAXIMIZE | WIN_FLAG_ALWAYS_ON_TOP;

        // Center on screen
        let x = (self.screen_width as i32 - 420) / 2;
        let y = (self.screen_height as i32 - (content_h + title_bar_height()) as i32) / 2;

        let win_id = self.create_window("Application Crashed", x, y, 420, content_h, flags, 0);

        let dialog = crash_dialog::CrashDialog::from_report(win_id, report);

        // Render the dialog content
        let layer_id = self.windows.iter().find(|w| w.id == win_id).map(|w| w.layer_id).unwrap_or(0);
        let full_h = content_h + title_bar_height();
        if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
            dialog.render(pixels, 420, full_h);
        }
        self.compositor.mark_layer_dirty(layer_id);

        self.crash_dialogs.push(dialog);
    }

    /// Check if a window click is on a crash dialog and handle it.
    /// Returns true if the click was consumed by a crash dialog.
    pub fn handle_crash_dialog_click(&mut self, window_id: u32, wx: i32, wy: i32) -> bool {
        let dialog_idx = match self.crash_dialogs.iter().position(|d| d.window_id == window_id) {
            Some(i) => i,
            None => return false,
        };

        let action = self.crash_dialogs[dialog_idx].handle_click(wx, wy);
        match action {
            crash_dialog::CrashDialogAction::Dismiss => {
                self.crash_dialogs.remove(dialog_idx);
                self.destroy_window(window_id);
            }
            crash_dialog::CrashDialogAction::ToggleDetails => {
                let new_h = self.crash_dialogs[dialog_idx].content_height();
                // Resize the window
                if let Some(win) = self.windows.iter_mut().find(|w| w.id == window_id) {
                    win.content_height = new_h;
                    let full_h = new_h + title_bar_height();
                    let layer_id = win.layer_id;
                    let cw = win.content_width;
                    self.compositor.resize_layer(layer_id, cw, full_h);

                    // Re-render chrome + dialog content
                    if let Some(pixels) = self.compositor.layer_pixels(layer_id) {
                        window::pre_render_chrome_ex(
                            pixels, cw, full_h, "Application Crashed", true,
                            WIN_FLAG_NO_MINIMIZE | WIN_FLAG_NO_MAXIMIZE,
                        );
                        self.crash_dialogs[dialog_idx].render(pixels, cw, full_h);
                    }
                    self.compositor.mark_layer_dirty(layer_id);
                }
            }
            crash_dialog::CrashDialogAction::None => {}
        }
        true
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
        let mb_h = menubar_height();
        let h = mb_h + 1;
        let fs = scaled_font_size();
        let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, fs, s);
        let tx = stride as i32 - tw as i32 - 10;
        let fy = ((mb_h as i32 - th as i32) / 2).max(0);

        for y in 0..mb_h {
            for x in (tx - 4).max(0) as u32..stride {
                pixels[(y * stride + x) as usize] = color_menubar_bg();
            }
        }

        anyos_std::ui::window::font_render_buf(
            FONT_ID, fs, pixels, stride, h, tx, fy, color_menubar_text(), s,
        );
    }
}
