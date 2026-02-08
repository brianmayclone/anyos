#![no_std]
#![no_main]

use alloc::vec;
use alloc::vec::Vec;

use anyos_std::ui::window;
use anyos_std::process;
use anyos_std::fs;

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

// ── Icon data ──
struct Icon {
    width: u32,
    height: u32,
    pixels: Vec<u32>, // ARGB
}

struct DockItem {
    name: &'static str,
    bin_path: &'static str,
    icon_path: &'static str,
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
        // Draw body
        self.fill_rect(x + r, y, w - 2 * r as u32, h, color);
        self.fill_rect(x, y + r, w, h - 2 * r as u32, color);

        // Draw corners
        for corner in 0..4 {
            let (cx, cy) = match corner {
                0 => (x + r, y + r),                         // top-left
                1 => (x + w as i32 - r - 1, y + r),          // top-right
                2 => (x + r, y + h as i32 - r - 1),          // bottom-left
                _ => (x + w as i32 - r - 1, y + h as i32 - r - 1), // bottom-right
            };
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy <= r * r {
                        let px = cx + dx;
                        let py = cy + dy;
                        if px >= 0 && px < self.width as i32 && py >= 0 && py < self.height as i32 {
                            let idx = (py as u32 * self.width + px as u32) as usize;
                            let a = (color >> 24) & 0xFF;
                            if a >= 255 {
                                self.pixels[idx] = color;
                            } else if a > 0 {
                                self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
                            }
                        }
                    }
                }
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

/// Parse window list buffer and check if a window with the given title exists.
/// Returns Some(window_id) if found, None otherwise.
fn find_window_by_title(title: &str, buf: &[u8], count: u32) -> Option<u32> {
    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > buf.len() { break; }
        let id = u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
        let tlen = u32::from_le_bytes([buf[off+4], buf[off+5], buf[off+6], buf[off+7]]) as usize;
        let tlen = tlen.min(56);
        if let Ok(win_title) = core::str::from_utf8(&buf[off+8..off+8+tlen]) {
            if win_title == title {
                return Some(id);
            }
        }
    }
    None
}

fn update_running_state(items: &mut [DockItem], win_buf: &[u8], win_count: u32) -> bool {
    let mut changed = false;
    for item in items.iter_mut() {
        let was_running = item.running;
        item.running = find_window_by_title(item.name, win_buf, win_count).is_some();
        if item.running != was_running {
            changed = true;
        }
    }
    changed
}

fn main() {
    // Get screen dimensions
    let (screen_width, screen_height) = window::screen_size();
    if screen_width == 0 || screen_height == 0 {
        return;
    }

    let win_y = (screen_height - DOCK_TOTAL_H) as u16;
    let flags = window::WIN_FLAG_BORDERLESS
              | window::WIN_FLAG_NOT_RESIZABLE
              | window::WIN_FLAG_ALWAYS_ON_TOP;

    let win = window::create_ex("Dock", 0, win_y, screen_width as u16, DOCK_TOTAL_H as u16, flags);
    if win == u32::MAX {
        return;
    }

    // Set up dock items
    let mut items: Vec<DockItem> = vec![
        DockItem {
            name: "Terminal",
            bin_path: "/system/terminal",
            icon_path: "/system/icons/terminal.icon",
            icon: None,
            running: false,
        },
        DockItem {
            name: "Activity Monitor",
            bin_path: "/system/taskmanager",
            icon_path: "/system/icons/taskmanager.icon",
            icon: None,
            running: false,
        },
        DockItem {
            name: "Settings",
            bin_path: "/system/settings",
            icon_path: "/system/icons/settings.icon",
            icon: None,
            running: false,
        },
    ];

    // Load icons
    for item in &mut items {
        item.icon = load_icon(item.icon_path);
    }

    // Create local framebuffer
    let mut fb = Framebuffer::new(screen_width, DOCK_TOTAL_H);

    // Initial render
    render_dock(&mut fb, &items, screen_width);
    window::blit(win, 0, 0, screen_width as u16, DOCK_TOTAL_H as u16, &fb.pixels);
    window::present(win);

    // Main loop
    let mut tick: u32 = 0;
    let mut win_buf = vec![0u8; 64 * 32]; // up to 32 windows

    loop {
        // Process events
        let mut event = [0u32; 5];
        while window::get_event(win, &mut event) == 1 {
            if event[0] == window::EVENT_MOUSE_DOWN {
                let lx = event[1] as i32;
                let ly = event[2] as i32;

                if let Some(idx) = dock_hit_test(lx, ly, screen_width, &items) {
                    if let Some(item) = items.get(idx) {
                        // Check if already running — focus it
                        let win_count = window::list_windows(&mut win_buf);
                        if let Some(wid) = find_window_by_title(item.name, &win_buf, win_count) {
                            window::focus(wid);
                        } else {
                            // Launch the program
                            process::spawn(item.bin_path, "");
                        }
                    }
                }
            }
        }

        // Periodically update running indicators (~every 1 second at 20Hz)
        tick = tick.wrapping_add(1);
        if tick % 20 == 0 {
            let win_count = window::list_windows(&mut win_buf);
            if update_running_state(&mut items, &win_buf, win_count) {
                render_dock(&mut fb, &items, screen_width);
                window::blit(win, 0, 0, screen_width as u16, DOCK_TOTAL_H as u16, &fb.pixels);
                window::present(win);
            }
        }

        process::sleep(50); // ~20 Hz
    }
}
