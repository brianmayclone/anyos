//! Desktop rendering — wallpaper and desktop icons.

use alloc::vec;

use libanyui_client as anyui;

const DESKTOP_BG_DARK: u32 = 0xFF1A1A2E;
const DESKTOP_BG_LIGHT: u32 = 0xFF4A90D9;

pub struct Desktop {
    pub win: anyui::Window,
    pub canvas: anyui::Canvas,
    pub screen_width: u32,
    pub screen_height: u32,
    pub needs_redraw: bool,
}

impl Desktop {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        let flags = anyui::WIN_FLAG_BORDERLESS | anyui::WIN_FLAG_NOT_RESIZABLE;
        let win = anyui::Window::new_with_flags(
            "Desktop", 0, 0, screen_width, screen_height, flags,
        );
        let canvas = anyui::Canvas::new(screen_width, screen_height);
        canvas.set_dock(anyui::DOCK_FILL);
        win.add(&canvas);

        Desktop {
            win,
            canvas,
            screen_width,
            screen_height,
            needs_redraw: true,
        }
    }

    pub fn resize(&mut self, new_w: u32, new_h: u32) {
        self.screen_width = new_w;
        self.screen_height = new_h;
        self.win.resize(new_w, new_h);
        self.canvas.set_size(new_w, new_h);
        self.needs_redraw = true;
    }

    pub fn render(&mut self, is_light: bool) {
        let bg = if is_light { DESKTOP_BG_LIGHT } else { DESKTOP_BG_DARK };
        let pixels = vec![bg; (self.screen_width as usize) * (self.screen_height as usize)];
        self.canvas.copy_pixels_from(&pixels);
        self.needs_redraw = false;
    }
}
