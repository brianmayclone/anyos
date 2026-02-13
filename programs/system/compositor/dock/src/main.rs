#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::anim::{AnimSet, Easing};
use anyos_std::process;
use anyos_std::fs;
use anyos_std::icons;

use libcompositor_client::{CompositorClient, WindowHandle};
use libimage_client;

anyos_std::entry!(main);

// ── Dock theme constants (match kernel theme) ──
const DOCK_HEIGHT: u32 = 64;
const DOCK_TOOLTIP_H: u32 = 28; // space above pill for tooltips
const DOCK_MARGIN: u32 = 8 + DOCK_TOOLTIP_H;
const DOCK_TOTAL_H: u32 = DOCK_HEIGHT + DOCK_MARGIN;
const DOCK_ICON_SIZE: u32 = 48;
const DOCK_ICON_SPACING: u32 = 6;
const DOCK_BORDER_RADIUS: i32 = 16;
const DOCK_H_PADDING: u32 = 12;

// Colors (ARGB u32)
const DOCK_BG: u32 = 0xC0303035;
const COLOR_WHITE: u32 = 0xFFFFFFFF;
const COLOR_TRANSPARENT: u32 = 0x00000000;
const COLOR_HIGHLIGHT: u32 = 0x19FFFFFF; // 10% white
const TOOLTIP_BG: u32 = 0xE6303035; // dark bg, ~90% opaque
const TOOLTIP_PAD: u32 = 8; // horizontal padding inside tooltip pill

// System font (sfpro.ttf, loaded by kernel at boot)
const FONT_ID: u16 = 0;
const FONT_SIZE: u16 = 12;

const CONFIG_PATH: &str = "/system/dock/programs.conf";

// ── Icon data ──
struct Icon {
    width: u32,
    height: u32,
    pixels: Vec<u32>, // ARGB
}

const CMD_FOCUS_BY_TID: u32 = 0x100A;
const STILL_RUNNING: u32 = u32::MAX - 1;

struct DockItem {
    name: String,
    bin_path: String,
    icon: Option<Icon>,
    running: bool,
    tid: u32, // TID of spawned process (0 = not running)
    pinned: bool, // true = from config, false = transient (running only)
}

// ── Local framebuffer ──
struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Framebuffer {
    fn new(width: u32, height: u32) -> Self {
        Framebuffer {
            width,
            height,
            pixels: vec![COLOR_TRANSPARENT; (width * height) as usize],
        }
    }

    fn clear(&mut self) {
        for p in &mut self.pixels {
            *p = COLOR_TRANSPARENT;
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let a = (color >> 24) & 0xFF;
        for row in 0..h as i32 {
            let py = y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..w as i32 {
                let px = x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[idx] = color;
                } else if a > 0 {
                    self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
                }
            }
        }
    }

    fn fill_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, r: i32, color: u32) {
        let ru = r as u32;
        if ru == 0 || w < ru * 2 || h < ru * 2 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        // 3 non-overlapping rects to avoid double-alpha-blending
        if h > ru * 2 {
            self.fill_rect(x, y + r, w, h - ru * 2, color);
        }
        if w > ru * 2 {
            self.fill_rect(x + r, y, w - ru * 2, ru, color);
            self.fill_rect(x + r, y + h as i32 - r, w - ru * 2, ru, color);
        }
        // Corner fills using pixel-center test for accuracy
        // Write pixels directly (no alpha_blend) to avoid stacking on already-transparent bg
        let r2x4 = (2 * r) * (2 * r);
        for dy in 0..ru {
            let cy = 2 * dy as i32 + 1 - 2 * r;
            let cy2 = cy * cy;
            let mut fill_start = ru;
            for dx in 0..ru {
                let cx = 2 * dx as i32 + 1 - 2 * r;
                if cx * cx + cy2 <= r2x4 {
                    fill_start = dx;
                    break;
                }
            }
            let fill_width = ru - fill_start;
            if fill_width > 0 {
                let fs = fill_start as i32;
                // Top-left: arc curves from top-edge to left-edge
                self.fill_rect(x + fs, y + dy as i32, fill_width, 1, color);
                // Top-right: mirror horizontally — fill from left side of quadrant
                self.fill_rect(x + (w - ru) as i32, y + dy as i32, fill_width, 1, color);
                // Bottom-left: mirror vertically — reverse row order
                self.fill_rect(x + fs, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
                // Bottom-right: mirror both axes
                self.fill_rect(x + (w - ru) as i32, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
            }
        }
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: u32) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < self.width as i32 && py >= 0 && py < self.height as i32 {
                        let idx = (py as u32 * self.width + px as u32) as usize;
                        self.pixels[idx] = color;
                    }
                }
            }
        }
    }

    fn blit_icon(&mut self, icon: &Icon, dst_x: i32, dst_y: i32) {
        for row in 0..icon.height as i32 {
            let py = dst_y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..icon.width as i32 {
                let px = dst_x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let src_idx = (row as u32 * icon.width + col as u32) as usize;
                let src_pixel = icon.pixels[src_idx];
                let a = (src_pixel >> 24) & 0xFF;
                if a == 0 { continue; }
                let dst_idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[dst_idx] = src_pixel;
                } else {
                    self.pixels[dst_idx] = alpha_blend(src_pixel, self.pixels[dst_idx]);
                }
            }
        }
    }

    /// Draw TTF text directly into the framebuffer using the system font.
    fn draw_text(&mut self, x: i32, y: i32, text: &str, color: u32) {
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, &mut self.pixels,
            self.width, self.height, x, y, color, text,
        );
    }

    /// Blit an icon scaled to dst_w x dst_h using nearest-neighbor sampling.
    fn blit_icon_scaled(&mut self, icon: &Icon, dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32) {
        let src_w = icon.width;
        let src_h = icon.height;
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 { return; }
        for dy in 0..dst_h as i32 {
            let py = dst_y + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            let sy = (dy as u32 * src_h / dst_h) as usize;
            for dx in 0..dst_w as i32 {
                let px = dst_x + dx;
                if px < 0 || px >= self.width as i32 { continue; }
                let sx = (dx as u32 * src_w / dst_w) as usize;
                let src_idx = sy * src_w as usize + sx;
                let src_pixel = icon.pixels[src_idx];
                let a = (src_pixel >> 24) & 0xFF;
                if a == 0 { continue; }
                let dst_idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[dst_idx] = src_pixel;
                } else {
                    self.pixels[dst_idx] = alpha_blend(src_pixel, self.pixels[dst_idx]);
                }
            }
        }
    }
}

fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let inv = 255 - sa;
    let or = (sr * sa + dr * inv) / 255;
    let og = (sg * sa + dg * inv) / 255;
    let ob = (sb * sa + db * inv) / 255;
    let oa = sa + ((dst >> 24) & 0xFF) * inv / 255;
    (oa << 24) | (or << 16) | (og << 8) | ob
}

fn load_ico_icon(path: &str) -> Option<Icon> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    if data.is_empty() {
        return None;
    }

    // Use size-aware ICO probe/decode to pick the best entry for dock size
    let info = match libimage_client::probe_ico_size(&data, DOCK_ICON_SIZE) {
        Some(i) => i,
        None => match libimage_client::probe(&data) {
            Some(i) => i,
            None => return None,
        },
    };

    let src_w = info.width;
    let src_h = info.height;
    let src_pixels = (src_w as usize) * (src_h as usize);

    let mut pixels: Vec<u32> = Vec::new();
    pixels.resize(src_pixels, 0);
    let mut scratch: Vec<u8> = Vec::new();
    scratch.resize(info.scratch_needed as usize, 0);

    let decode_ok = if info.format == libimage_client::FMT_ICO {
        libimage_client::decode_ico_size(&data, DOCK_ICON_SIZE, &mut pixels, &mut scratch).is_ok()
    } else {
        libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok()
    };
    if !decode_ok {
        return None;
    }

    // Scale to dock icon size
    if src_w == DOCK_ICON_SIZE && src_h == DOCK_ICON_SIZE {
        return Some(Icon { width: DOCK_ICON_SIZE, height: DOCK_ICON_SIZE, pixels });
    }

    let dst_count = (DOCK_ICON_SIZE * DOCK_ICON_SIZE) as usize;
    let mut dst_pixels = vec![0u32; dst_count];
    libimage_client::scale_image(
        &pixels, src_w, src_h,
        &mut dst_pixels, DOCK_ICON_SIZE, DOCK_ICON_SIZE,
        libimage_client::MODE_SCALE,
    );

    Some(Icon { width: DOCK_ICON_SIZE, height: DOCK_ICON_SIZE, pixels: dst_pixels })
}

// ── Config file parsing ──

/// Load dock items from /system/dock/programs.conf
/// Format: one item per line: name|path
/// Lines starting with '#' are comments, empty lines are skipped.
fn load_dock_config() -> Vec<DockItem> {
    let mut items = Vec::new();

    let mut stat_buf = [0u32; 2];
    if fs::stat(CONFIG_PATH, &mut stat_buf) != 0 {
        return items;
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 4096 {
        return items;
    }

    let fd = fs::open(CONFIG_PATH, 0);
    if fd == u32::MAX {
        return items;
    }

    let mut data = vec![0u8; file_size];
    let bytes_read = fs::read(fd, &mut data) as usize;
    fs::close(fd);

    if bytes_read == 0 {
        return items;
    }

    let text = match core::str::from_utf8(&data[..bytes_read]) {
        Ok(s) => s,
        Err(_) => return items,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split by '|': name|path
        let mut parts = line.splitn(2, '|');
        let name = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let path = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };

        if name.is_empty() || path.is_empty() {
            continue;
        }

        items.push(DockItem {
            name: String::from(name),
            bin_path: String::from(path),
            icon: None,
            running: false,
            tid: 0,
            pinned: true,
        });
    }

    items
}

// ── System event name unpacking ──

/// System thread names that should NOT appear in the dock.
const SYSTEM_NAMES: &[&str] = &["compositor", "dock", "cpu_monitor", "init"];

/// Unpack a process name from EVT_PROCESS_SPAWNED words[2..5] (3 u32 LE packed).
fn unpack_event_name(w2: u32, w3: u32, w4: u32) -> String {
    let mut buf = [0u8; 12];
    for i in 0..4 { buf[i] = ((w2 >> (i * 8)) & 0xFF) as u8; }
    for i in 0..4 { buf[4 + i] = ((w3 >> (i * 8)) & 0xFF) as u8; }
    for i in 0..4 { buf[8 + i] = ((w4 >> (i * 8)) & 0xFF) as u8; }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(12);
    String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
}

// ── Rendering ──

/// Compute bounce Y-offset for an icon being launched.
/// 3 bounces over 2000ms with decreasing amplitude (16, 10, 5 pixels).
/// Uses a sine-approximation curve for smooth, natural motion.
fn bounce_offset(elapsed_ms: u32) -> i32 {
    if elapsed_ms >= 2000 { return 0; }
    let bounce_dur = 667u32;
    let bounce_idx = (elapsed_ms / bounce_dur).min(2);
    let t_in_bounce = elapsed_ms - bounce_idx * bounce_dur;
    let peak: i32 = match bounce_idx { 0 => 16, 1 => 10, _ => 5 };
    // Sine approximation: map t_in_bounce [0..bounce_dur] to a half-sine [0..pi]
    // sin(pi * t / dur) ≈ parabolic: 4t(dur-t) / dur^2 (smooth at 0 and peak)
    let t = t_in_bounce as i64;
    let d = bounce_dur as i64;
    let sine_approx = (4 * t * (d - t) * 1000 / (d * d)) as i32; // 0..1000
    peak * sine_approx / 1000
}

struct RenderState<'a> {
    hover_idx: Option<usize>,
    anims: &'a AnimSet,
    bounce_items: &'a [(usize, u32)],
    now: u32,
}

fn render_dock(fb: &mut Framebuffer, items: &[DockItem], screen_width: u32, rs: &RenderState) {
    fb.clear();

    let item_count = items.len() as u32;
    if item_count == 0 { return; }

    let total_width = item_count * DOCK_ICON_SIZE
        + (item_count - 1) * DOCK_ICON_SPACING
        + DOCK_H_PADDING * 2;

    let dock_x = (screen_width as i32 - total_width as i32) / 2;
    let dock_y = DOCK_MARGIN as i32;

    // Glass pill background
    fb.fill_rounded_rect(dock_x, dock_y, total_width, DOCK_HEIGHT, DOCK_BORDER_RADIUS, DOCK_BG);

    // Top highlight line for depth
    fb.fill_rect(
        dock_x + DOCK_BORDER_RADIUS,
        dock_y,
        total_width - DOCK_BORDER_RADIUS as u32 * 2,
        1,
        COLOR_HIGHLIGHT,
    );

    // Draw each item
    let base_icon_y = dock_y + ((DOCK_HEIGHT as i32 - DOCK_ICON_SIZE as i32) / 2) - 2;
    let mut ix = dock_x + DOCK_H_PADDING as i32;

    for (i, item) in items.iter().enumerate() {
        // Scale animation: value 0..4000 → extra 0..4 pixels
        let extra = rs.anims.value_or(100 + i as u32, rs.now, 0).max(0) / 1000;
        let extra_u = extra as u32;
        let draw_w = DOCK_ICON_SIZE + extra_u;
        let draw_h = DOCK_ICON_SIZE + extra_u;
        let offset_x = -(extra / 2);
        let offset_y = -(extra / 2);

        // Bounce animation
        let bounce_y = rs.bounce_items.iter()
            .find(|(idx, _)| *idx == i)
            .map(|(_, start)| {
                let elapsed = rs.now.wrapping_sub(*start) * 1000 / anyos_std::sys::tick_hz().max(1);
                bounce_offset(elapsed)
            })
            .unwrap_or(0);

        let icon_x = ix + offset_x;
        let icon_y = base_icon_y + offset_y - bounce_y;

        if let Some(ref icon) = item.icon {
            if extra_u > 0 {
                fb.blit_icon_scaled(icon, icon_x, icon_y, draw_w, draw_h);
            } else {
                fb.blit_icon(icon, icon_x, icon_y);
            }
        } else {
            fb.fill_rounded_rect(icon_x, icon_y, draw_w, draw_h, 10, 0xFF3C3C41);
        }

        // Running indicator dot
        if item.running {
            let dot_x = ix + DOCK_ICON_SIZE as i32 / 2;
            let dot_y = base_icon_y + DOCK_ICON_SIZE as i32 + 5;
            fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
        }

        ix += (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
    }

    // Tooltip for hovered item
    if let Some(idx) = rs.hover_idx {
        if let Some(item) = items.get(idx) {
            let name = &item.name;
            let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, name);
            let pill_w = tw + TOOLTIP_PAD * 2;
            let pill_h = th + 8; // 4px padding top + bottom

            // Center tooltip above the hovered icon
            let item_stride = (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
            let icon_center_x = dock_x + DOCK_H_PADDING as i32
                + idx as i32 * item_stride
                + DOCK_ICON_SIZE as i32 / 2;
            let pill_x = icon_center_x - pill_w as i32 / 2;
            let pill_y = dock_y - pill_h as i32 - 4;

            fb.fill_rounded_rect(pill_x, pill_y, pill_w, pill_h, 6, TOOLTIP_BG);

            // Draw text centered in the pill
            let text_x = pill_x + TOOLTIP_PAD as i32;
            let text_y = pill_y + ((pill_h as i32 - th as i32) / 2);
            fb.draw_text(text_x, text_y, name, COLOR_WHITE);
        }
    }
}

fn dock_hit_test(x: i32, y: i32, screen_width: u32, items: &[DockItem]) -> Option<usize> {
    let item_count = items.len() as u32;
    if item_count == 0 { return None; }

    let total_width = item_count * DOCK_ICON_SIZE
        + (item_count - 1) * DOCK_ICON_SPACING
        + DOCK_H_PADDING * 2;

    let dock_x = (screen_width as i32 - total_width as i32) / 2;
    let dock_y = DOCK_MARGIN as i32;

    // Check if click is within dock bounds
    if x < dock_x || x >= dock_x + total_width as i32 { return None; }
    if y < dock_y || y >= dock_y + DOCK_HEIGHT as i32 { return None; }

    let local_x = x - dock_x - DOCK_H_PADDING as i32;
    if local_x < 0 { return None; }

    let item_stride = DOCK_ICON_SIZE + DOCK_ICON_SPACING;
    let idx = local_x as u32 / item_stride;
    if idx < item_count {
        Some(idx as usize)
    } else {
        None
    }
}

/// Copy local framebuffer pixels into the SHM window surface.
fn blit_to_surface(fb: &Framebuffer, win: &WindowHandle) {
    let count = (fb.width * fb.height) as usize;
    let surface = unsafe { core::slice::from_raw_parts_mut(win.surface(), count) };
    let copy_len = count.min(fb.pixels.len()).min(surface.len());
    surface[..copy_len].copy_from_slice(&fb.pixels[..copy_len]);
}

fn main() {
    // Connect to compositor
    let client = match CompositorClient::init() {
        Some(c) => c,
        None => return,
    };

    let (screen_width, screen_height) = client.screen_size();
    if screen_width == 0 || screen_height == 0 {
        return;
    }

    // Borderless, not resizable, always on top
    let flags: u32 = 0x01 | 0x02 | 0x04;

    let mut win = match client.create_window(screen_width, DOCK_TOTAL_H, flags) {
        Some(w) => w,
        None => return,
    };

    // Position at bottom of screen
    client.move_window(&win, 0, (screen_height - DOCK_TOTAL_H) as i32);
    client.set_title(&win, "Dock");

    // Subscribe to system events (process exit notifications)
    let sys_sub = anyos_std::ipc::evt_sys_subscribe(0);

    // Load dock items from config file
    let mut items = load_dock_config();

    // Load icons (derive path from binary name)
    for item in &mut items {
        let icon_path = icons::app_icon_path(&item.bin_path);
        item.icon = load_ico_icon(&icon_path);
    }

    // Mutable screen dimensions (updated on resolution change)
    let mut screen_width = screen_width;
    let mut screen_height = screen_height;

    // Create local framebuffer
    let mut fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);

    // Animation state
    let mut hovered_idx: Option<usize> = None;
    let mut anims = AnimSet::new();
    let mut bounce_items: Vec<(usize, u32)> = Vec::new();
    let has_gpu = anyos_std::ui::window::gpu_has_accel();

    // Initial render
    let rs = RenderState {
        hover_idx: hovered_idx,
        anims: &anims,
        bounce_items: &bounce_items,
        now: anyos_std::sys::uptime(),
    };
    render_dock(&mut fb, &items, screen_width, &rs);
    blit_to_surface(&fb, &win);
    client.present(&win);

    let mut needs_redraw = false;

    // Main loop
    loop {
        // Process window events
        while let Some(event) = client.poll_event(&win) {
            if event.event_type == libcompositor_client::EVT_MOUSE_MOVE {
                let lx = event.arg1 as i32;
                let ly = event.arg2 as i32;
                let new_hover = dock_hit_test(lx, ly, screen_width, &items);
                if new_hover != hovered_idx {
                    if has_gpu {
                        let now = anyos_std::sys::uptime();
                        // Animate old icon back to normal
                        if let Some(old) = hovered_idx {
                            anims.start_at(100 + old as u32, 4000, 0, 200, Easing::EaseOut, now);
                        }
                        // Animate new icon to enlarged size (+4px = 4000 fixed-point)
                        if let Some(new) = new_hover {
                            anims.start_at(100 + new as u32, 0, 4000, 200, Easing::EaseOut, now);
                        }
                    }
                    hovered_idx = new_hover;
                    needs_redraw = true;
                }
            } else if event.event_type == libcompositor_client::EVT_MOUSE_DOWN {
                let lx = event.arg1 as i32;
                let ly = event.arg2 as i32;

                if let Some(idx) = dock_hit_test(lx, ly, screen_width, &items) {
                    if let Some(item) = items.get_mut(idx) {
                        if item.tid != 0 {
                            // Check if still running
                            let status = process::try_waitpid(item.tid);
                            if status == STILL_RUNNING {
                                // App is running — tell compositor to focus its window
                                let cmd: [u32; 5] = [CMD_FOCUS_BY_TID, item.tid, 0, 0, 0];
                                anyos_std::ipc::evt_chan_emit(client.channel_id, &cmd);
                            } else {
                                // App exited — spawn new instance
                                item.running = false;
                                item.tid = 0;
                                let tid = process::spawn(&item.bin_path, "");
                                if tid != 0 {
                                    item.tid = tid;
                                    item.running = true;
                                    if has_gpu {
                                        bounce_items.push((idx, anyos_std::sys::uptime()));
                                    }
                                }
                                needs_redraw = true;
                            }
                        } else {
                            // Not running — spawn it
                            let tid = process::spawn(&item.bin_path, "");
                            if tid != 0 {
                                item.tid = tid;
                                item.running = true;
                                if has_gpu {
                                    bounce_items.push((idx, anyos_std::sys::uptime()));
                                }
                                needs_redraw = true;
                            }
                        }
                    }
                }
            }
        }

        // Poll system events for process spawns and exits
        let mut sys_buf = [0u32; 5];
        while anyos_std::ipc::evt_sys_poll(sys_sub, &mut sys_buf) {
            if sys_buf[0] == 0x0020 { // EVT_PROCESS_SPAWNED
                let spawned_tid = sys_buf[1];
                let name = unpack_event_name(sys_buf[2], sys_buf[3], sys_buf[4]);

                // Skip system threads
                if name.is_empty() || SYSTEM_NAMES.iter().any(|&s| s == name.as_str()) {
                    continue;
                }

                // Check if this TID is already tracked by a pinned item
                let already_tracked = items.iter().any(|it| it.tid == spawned_tid);
                if already_tracked {
                    continue;
                }

                // Check if a pinned item matches by binary basename
                let mut matched_pinned = false;
                for item in &mut items {
                    if item.pinned {
                        let basename = item.bin_path.rsplit('/').next().unwrap_or("");
                        if basename == name.as_str() {
                            item.tid = spawned_tid;
                            item.running = true;
                            matched_pinned = true;
                            needs_redraw = true;
                            break;
                        }
                    }
                }

                // If no pinned item matched, create a transient dock item
                if !matched_pinned {
                    let bin_path = alloc::format!("/bin/{}", name);
                    let icon_path = icons::app_icon_path(&bin_path);
                    let icon = load_ico_icon(&icon_path);
                    items.push(DockItem {
                        name: name,
                        bin_path: bin_path,
                        icon,
                        running: true,
                        tid: spawned_tid,
                        pinned: false,
                    });
                    needs_redraw = true;
                }
            } else if sys_buf[0] == 0x0040 { // EVT_RESOLUTION_CHANGED
                let new_w = sys_buf[1];
                let new_h = sys_buf[2];
                if new_w != screen_width || new_h != screen_height {
                    screen_width = new_w;
                    screen_height = new_h;

                    // Resize the dock window to span the new screen width
                    if client.resize_window(&mut win, screen_width, DOCK_TOTAL_H) {
                        // Recreate local framebuffer at new width
                        fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);

                        // Reposition to bottom of new screen
                        client.move_window(&win, 0, (screen_height - DOCK_TOTAL_H) as i32);

                        needs_redraw = true;
                    }
                }
            } else if sys_buf[0] == 0x0021 { // EVT_PROCESS_EXITED
                let exited_tid = sys_buf[1];
                // For pinned items: clear running state. For transient: remove.
                let mut i = 0;
                while i < items.len() {
                    if items[i].tid == exited_tid {
                        if items[i].pinned {
                            items[i].running = false;
                            items[i].tid = 0;
                            i += 1;
                        } else {
                            items.remove(i);
                        }
                        needs_redraw = true;
                    } else {
                        i += 1;
                    }
                }
            }
        }

        // Tick animations — GC done, check if any are active
        let now = anyos_std::sys::uptime();
        let hz = anyos_std::sys::tick_hz().max(1);
        anims.remove_done(now);
        if anims.has_active(now) {
            needs_redraw = true;
        }

        // Clean up finished bounces (>2 seconds)
        bounce_items.retain(|(_, start)| {
            let elapsed_ms = now.wrapping_sub(*start) * 1000 / hz;
            elapsed_ms < 2000
        });
        if !bounce_items.is_empty() {
            needs_redraw = true;
        }

        if needs_redraw {
            let rs = RenderState {
                hover_idx: hovered_idx,
                anims: &anims,
                bounce_items: &bounce_items,
                now,
            };
            render_dock(&mut fb, &items, screen_width, &rs);
            blit_to_surface(&fb, &win);
            client.present(&win);
            needs_redraw = false;
        }

        // Faster polling during animations, moderate when idle
        let sleep_ms = if anims.has_active(now) || !bounce_items.is_empty() { 8 } else { 32 };
        process::sleep(sleep_ms);
    }
}
