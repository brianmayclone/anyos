//! Menubar rendering — system logo, active app name, clock.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libanyui_client as anyui;
use libfont_client as font;

pub const MENUBAR_HEIGHT: u32 = 24;

const FONT_SIZE: u16 = 13;
const TEXT_COLOR_DARK: u32 = 0xFFE0E0E0;
const TEXT_COLOR_LIGHT: u32 = 0xFF1D1D1F;
const MENUBAR_BG_DARK: u32 = 0xE0282828;
const MENUBAR_BG_LIGHT: u32 = 0xE0F6F6F6;

pub struct Menubar {
    pub win: anyui::Window,
    pub canvas: anyui::Canvas,
    pub pixels: Vec<u32>,
    pub needs_redraw: bool,
    pub screen_width: u32,
}

impl Menubar {
    pub fn new(screen_width: u32) -> Self {
        let flags = anyui::WIN_FLAG_BORDERLESS
            | anyui::WIN_FLAG_NOT_RESIZABLE
            | anyui::WIN_FLAG_ALWAYS_ON_TOP;
        let win = anyui::Window::new_with_flags(
            "Menubar", 0, 0, screen_width, MENUBAR_HEIGHT, flags,
        );
        let canvas = anyui::Canvas::new(screen_width, MENUBAR_HEIGHT);
        canvas.set_dock(anyui::DOCK_FILL);
        canvas.set_interactive(true);
        win.add(&canvas);

        let pixels = vec![0u32; (screen_width * MENUBAR_HEIGHT) as usize];

        Menubar {
            win,
            canvas,
            pixels,
            needs_redraw: true,
            screen_width,
        }
    }

    pub fn resize(&mut self, new_width: u32) {
        self.screen_width = new_width;
        self.win.resize(new_width, MENUBAR_HEIGHT);
        self.canvas.set_size(new_width, MENUBAR_HEIGHT);
        self.pixels = vec![0u32; (new_width * MENUBAR_HEIGHT) as usize];
        self.needs_redraw = true;
    }

    pub fn render(&mut self, app_name: &str, is_light: bool) {
        let bg = if is_light { MENUBAR_BG_LIGHT } else { MENUBAR_BG_DARK };
        let text_color = if is_light { TEXT_COLOR_LIGHT } else { TEXT_COLOR_DARK };
        let w = self.screen_width;
        let h = MENUBAR_HEIGHT;

        self.pixels.fill(bg);
        let buf_ptr = self.pixels.as_mut_ptr();

        // Left side: system logo placeholder + app name
        let mut x: i32 = 12;

        // System logo placeholder (bullet)
        font::draw_string_buf(buf_ptr, w, h, x, 4, text_color, 1, FONT_SIZE, "\u{25CF}");
        x += 20;

        // Focused app name (bold)
        if !app_name.is_empty() {
            font::draw_string_buf(buf_ptr, w, h, x, 4, text_color, 1, FONT_SIZE, app_name);
        }

        // Right side: clock
        let clock_str = format_clock();
        let (clock_w, _) = font::measure(0, FONT_SIZE, &clock_str);
        let clock_x = w as i32 - clock_w as i32 - 12;
        font::draw_string_buf(buf_ptr, w, h, clock_x, 4, text_color, 0, FONT_SIZE, &clock_str);

        // Bottom border
        let border_color = if is_light { 0x40000000u32 } else { 0x40FFFFFFu32 };
        let bottom_row = ((h - 1) * w) as usize;
        if bottom_row + w as usize <= self.pixels.len() {
            for i in 0..w as usize {
                self.pixels[bottom_row + i] = border_color;
            }
        }

        self.canvas.copy_pixels_from(&self.pixels);
        self.needs_redraw = false;
    }
}

/// Format the current time as HH:MM for the menubar clock.
fn format_clock() -> String {
    let ms = anyos_std::sys::uptime_ms();
    let total_sec = ms / 1000;
    let hours = (total_sec / 3600) % 24;
    let minutes = (total_sec / 60) % 60;
    alloc::format!("{:02}:{:02}", hours, minutes)
}
