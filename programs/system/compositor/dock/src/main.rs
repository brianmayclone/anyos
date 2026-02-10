#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::process;
use anyos_std::fs;

use libcompositor_client::{CompositorClient, WindowHandle};

anyos_std::entry!(main);

// ── Dock theme constants (match kernel theme) ──
const DOCK_HEIGHT: u32 = 64;
const DOCK_MARGIN: u32 = 8; // transparent margin above dock pill
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

const CONFIG_PATH: &str = "/system/dock/programs.conf";

// ── Icon data ──
struct Icon {
    width: u32,
    height: u32,
    pixels: Vec<u32>, // ARGB
}

struct DockItem {
    name: String,
    bin_path: String,
    icon_path: String,
    icon: Option<Icon>,
    running: bool,
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

fn load_icon(path: &str) -> Option<Icon> {
    // Get file size
    let mut stat_buf = [0u32; 2];
    if fs::stat(path, &mut stat_buf) != 0 {
        return None;
    }
    let file_size = stat_buf[1] as usize;
    if file_size < 8 {
        return None;
    }

    // Open and read file
    let fd = fs::open(path, 0); // read-only
    if fd == u32::MAX {
        return None;
    }

    let mut data = vec![0u8; file_size];
    let bytes_read = fs::read(fd, &mut data);
    fs::close(fd);

    if (bytes_read as usize) < 8 {
        return None;
    }

    // Parse header: [width:u32 LE][height:u32 LE][RGBA pixel data]
    let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let pixel_count = (width * height) as usize;
    let expected_size = 8 + pixel_count * 4;

    if bytes_read as usize != expected_size {
        return None;
    }

    // Convert RGBA bytes to ARGB u32
    let mut pixels = Vec::with_capacity(pixel_count);
    for i in 0..pixel_count {
        let off = 8 + i * 4;
        let r = data[off] as u32;
        let g = data[off + 1] as u32;
        let b = data[off + 2] as u32;
        let a = data[off + 3] as u32;
        pixels.push((a << 24) | (r << 16) | (g << 8) | b);
    }

    Some(Icon { width, height, pixels })
}

// ── Config file parsing ──

/// Load dock items from /system/dock/programs.conf
/// Format: one item per line: name|path|icon
/// Lines starting with '#' are comments, empty lines are skipped.
fn load_dock_config() -> Vec<DockItem> {
    let mut items = Vec::new();

    // Read config file
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

    // Parse line by line
    let text = match core::str::from_utf8(&data[..bytes_read]) {
        Ok(s) => s,
        Err(_) => return items,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split by '|': name|path|icon
        let mut parts = line.splitn(3, '|');
        let name = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let path = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let icon = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };

        if name.is_empty() || path.is_empty() {
            continue;
        }

        items.push(DockItem {
            name: String::from(name),
            bin_path: String::from(path),
            icon_path: String::from(icon),
            icon: None,
            running: false,
        });
    }

    items
}

// ── Rendering ──

fn render_dock(fb: &mut Framebuffer, items: &[DockItem], screen_width: u32) {
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
    let icon_y = dock_y + ((DOCK_HEIGHT as i32 - DOCK_ICON_SIZE as i32) / 2) - 2;
    let mut ix = dock_x + DOCK_H_PADDING as i32;

    for item in items {
        if let Some(ref icon) = item.icon {
            fb.blit_icon(icon, ix, icon_y);
        } else {
            // Fallback: rounded-rect placeholder with first letter
            fb.fill_rounded_rect(ix, icon_y, DOCK_ICON_SIZE, DOCK_ICON_SIZE, 10, 0xFF3C3C41);
        }

        // Running indicator dot
        if item.running {
            let dot_x = ix + DOCK_ICON_SIZE as i32 / 2;
            let dot_y = icon_y + DOCK_ICON_SIZE as i32 + 5;
            fb.fill_circle(dot_x, dot_y, 2, COLOR_WHITE);
        }

        ix += (DOCK_ICON_SIZE + DOCK_ICON_SPACING) as i32;
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

    let win = match client.create_window(screen_width, DOCK_TOTAL_H, flags) {
        Some(w) => w,
        None => return,
    };

    // Position at bottom of screen
    client.move_window(&win, 0, (screen_height - DOCK_TOTAL_H) as i32);
    client.set_title(&win, "Dock");

    // Load dock items from config file
    let items = load_dock_config();

    // Load icons
    let mut items = items;
    for item in &mut items {
        item.icon = load_icon(&item.icon_path);
    }

    // Create local framebuffer
    let mut fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);

    // Initial render
    render_dock(&mut fb, &items, screen_width);
    blit_to_surface(&fb, &win);
    client.present(&win);

    // Main loop
    loop {
        // Process window events
        while let Some(event) = client.poll_event(&win) {
            if event.event_type == libcompositor_client::EVT_MOUSE_DOWN {
                let lx = event.arg1 as i32;
                let ly = event.arg2 as i32;

                if let Some(idx) = dock_hit_test(lx, ly, screen_width, &items) {
                    if let Some(item) = items.get(idx) {
                        process::spawn(&item.bin_path, "");
                    }
                }
            }
        }

        process::sleep(50); // ~20 Hz
    }
}
