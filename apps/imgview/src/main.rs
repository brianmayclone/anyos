// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Image Viewer — displays BMP, PNG, JPEG, and GIF images.

#![no_std]
#![no_main]

use anyos_std::{Vec, vec};
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

const BG: u32 = 0xFF1E1E1E;
const CHECKER_A: u32 = 0xFF3C3C3C;
const CHECKER_B: u32 = 0xFF2C2C2C;
const CHECKER_SIZE: i32 = 8;

struct AppState {
    canvas: anyui::Canvas,
    pixels: Vec<u32>,
    img_w: usize,
    img_h: usize,
    scroll_x: i32,
    scroll_y: i32,
    dragging: bool,
    drag_start_x: i32,
    drag_start_y: i32,
    drag_scroll_x: i32,
    drag_scroll_y: i32,
    render_buf: Vec<u32>,
    needs_redraw: bool,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

fn main() {
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("imgview: no file specified");
        anyos_std::println!("Usage: imgview <image_file>");
        return;
    }

    // Read image file
    let data = match read_file(path) {
        Some(d) => d,
        None => {
            anyos_std::println!("imgview: cannot open '{}'", path);
            return;
        }
    };

    // Probe format
    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => {
            anyos_std::println!("imgview: unsupported format");
            return;
        }
    };

    let img_w = info.width as usize;
    let img_h = info.height as usize;
    let fmt_name = libimage_client::format_name(info.format);

    // Decode
    let mut pixels = vec![0u32; img_w * img_h];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    if let Err(e) = libimage_client::decode(&data, &mut pixels, &mut scratch) {
        anyos_std::println!("imgview: decode error: {:?}", e);
        return;
    }
    drop(scratch);
    drop(data);

    if !anyui::init() { return; }

    let filename = path.rsplit('/').next().unwrap_or(path);

    // Window dimensions: fit image, capped to screen
    let (scr_w, scr_h) = anyui::screen_size();
    let (scr_w, scr_h) = if scr_w == 0 || scr_h == 0 { (1024, 768) } else { (scr_w, scr_h) };
    let win_w = (img_w as u32 + 2).min(scr_w.min(1024)).max(200);
    let win_h = (img_h as u32 + 2).min(scr_h.min(768)).max(150);

    let mut title_buf = [0u8; 128];
    let title = format_title(&mut title_buf, filename, fmt_name, img_w, img_h);

    let win = anyui::Window::new(title, -1, -1, win_w, win_h);

    // Canvas for image display
    let canvas = anyui::Canvas::new(win_w, win_h);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_interactive(true);
    win.add(&canvas);

    unsafe {
        APP = Some(AppState {
            canvas,
            pixels,
            img_w,
            img_h,
            scroll_x: 0,
            scroll_y: 0,
            dragging: false,
            drag_start_x: 0,
            drag_start_y: 0,
            drag_scroll_x: 0,
            drag_scroll_y: 0,
            render_buf: Vec::new(),
            needs_redraw: true,
        });
    }

    // Mouse events for panning
    app().canvas.on_mouse_down(|x, y, _btn| {
        let a = app();
        a.dragging = true;
        a.drag_start_x = x;
        a.drag_start_y = y;
        a.drag_scroll_x = a.scroll_x;
        a.drag_scroll_y = a.scroll_y;
    });

    app().canvas.on_mouse_up(|_x, _y, _btn| {
        app().dragging = false;
    });

    app().canvas.on_draw(|x, y, _btn| {
        if app().dragging {
            let a = app();
            a.scroll_x = a.drag_scroll_x - (x - a.drag_start_x);
            a.scroll_y = a.drag_scroll_y - (y - a.drag_start_y);
            clamp_scroll();
            a.needs_redraw = true;
        }
    });

    // Keyboard events
    win.on_key_down(|ke| {
        match ke.keycode {
            anyui::KEY_ESCAPE => anyui::quit(),
            anyui::KEY_UP    => { app().scroll_y -= 32; clamp_scroll(); app().needs_redraw = true; }
            anyui::KEY_DOWN  => { app().scroll_y += 32; clamp_scroll(); app().needs_redraw = true; }
            anyui::KEY_LEFT  => { app().scroll_x -= 32; clamp_scroll(); app().needs_redraw = true; }
            anyui::KEY_RIGHT => { app().scroll_x += 32; clamp_scroll(); app().needs_redraw = true; }
            _ => {}
        }
    });

    win.on_close(|_| anyui::quit());

    // Timer for rendering (handles initial layout + resize)
    anyui::set_timer(30, || {
        if app().needs_redraw {
            app().needs_redraw = false;
            redraw();
        }
    });

    anyui::run();
}

fn clamp_scroll() {
    let a = app();
    let view_w = a.canvas.get_stride() as i32;
    let view_h = a.canvas.get_height() as i32;
    let max_x = (a.img_w as i32 - view_w).max(0);
    let max_y = (a.img_h as i32 - view_h).max(0);
    a.scroll_x = a.scroll_x.max(0).min(max_x);
    a.scroll_y = a.scroll_y.max(0).min(max_y);
}

fn redraw() {
    let a = app();
    let w = a.canvas.get_stride() as usize;
    let h = a.canvas.get_height() as usize;
    if w == 0 || h == 0 { return; }

    let needed = w * h;
    if a.render_buf.len() != needed {
        a.render_buf.resize(needed, BG);
    }

    // Fill background
    for p in a.render_buf.iter_mut() { *p = BG; }

    // Calculate image position (centered if smaller than view)
    let img_x = if (a.img_w as i32) < w as i32 { (w as i32 - a.img_w as i32) / 2 } else { -a.scroll_x };
    let img_y = if (a.img_h as i32) < h as i32 { (h as i32 - a.img_h as i32) / 2 } else { -a.scroll_y };

    let src_x = a.scroll_x.max(0) as usize;
    let src_y = a.scroll_y.max(0) as usize;
    let dst_x = img_x.max(0) as usize;
    let dst_y = img_y.max(0) as usize;

    let vis_w = (a.img_w as i32 - src_x as i32).min(w as i32).max(0) as usize;
    let vis_h = (a.img_h as i32 - src_y as i32).min(h as i32).max(0) as usize;

    if vis_w > 0 && vis_h > 0 {
        // Draw checkerboard behind image (for transparency)
        for cy in 0..vis_h {
            for cx in 0..vis_w {
                let px = dst_x + cx;
                let py = dst_y + cy;
                if px < w && py < h {
                    let check = ((cx as i32 / CHECKER_SIZE) + (cy as i32 / CHECKER_SIZE)) % 2;
                    a.render_buf[py * w + px] = if check == 0 { CHECKER_A } else { CHECKER_B };
                }
            }
        }

        // Blit image pixels with alpha blending
        for row in 0..vis_h {
            let sy = src_y + row;
            if sy >= a.img_h { break; }
            for col in 0..vis_w {
                let sx = src_x + col;
                if sx >= a.img_w { break; }
                let px = dst_x + col;
                let py = dst_y + row;
                if px < w && py < h {
                    let src_pixel = a.pixels[sy * a.img_w + sx];
                    let alpha = (src_pixel >> 24) & 0xFF;
                    if alpha == 0xFF {
                        a.render_buf[py * w + px] = src_pixel;
                    } else if alpha > 0 {
                        a.render_buf[py * w + px] = blend(src_pixel, a.render_buf[py * w + px]);
                    }
                }
            }
        }
    }

    a.canvas.copy_pixels_from(&a.render_buf);
}

fn blend(src: u32, dst: u32) -> u32 {
    let sa = ((src >> 24) & 0xFF) as u32;
    let sr = ((src >> 16) & 0xFF) as u32;
    let sg = ((src >> 8) & 0xFF) as u32;
    let sb = (src & 0xFF) as u32;
    let da = 255 - sa;
    let dr = ((dst >> 16) & 0xFF) as u32;
    let dg = ((dst >> 8) & 0xFF) as u32;
    let db = (dst & 0xFF) as u32;
    let r = (sr * sa + dr * da) / 255;
    let g = (sg * sa + dg * da) / 255;
    let b = (sb * sa + db * da) / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX { return None; }
    let mut content = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

fn format_title<'a>(buf: &'a mut [u8; 128], filename: &str, fmt: &str, w: usize, h: usize) -> &'a str {
    let mut pos = 0;
    for &b in filename.as_bytes() { if pos < 127 { buf[pos] = b; pos += 1; } }
    for &b in b" (" { if pos < 127 { buf[pos] = b; pos += 1; } }
    for &b in fmt.as_bytes() { if pos < 127 { buf[pos] = b; pos += 1; } }
    if pos < 127 { buf[pos] = b' '; pos += 1; }
    pos = write_num(buf, pos, w);
    if pos < 127 { buf[pos] = b'x'; pos += 1; }
    pos = write_num(buf, pos, h);
    for &b in b") - Image Viewer" { if pos < 127 { buf[pos] = b; pos += 1; } }
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
}

fn write_num(buf: &mut [u8; 128], pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    let mut p = pos;
    for &b in &tmp[..len] { if p < 127 { buf[p] = b; p += 1; } }
    p
}

fn fmt_usize(mut n: usize, buf: &mut [u8; 10]) -> usize {
    if n == 0 { buf[0] = b'0'; return 1; }
    let mut pos = 10;
    while n > 0 && pos > 0 { pos -= 1; buf[pos] = b'0' + (n % 10) as u8; n /= 10; }
    let len = 10 - pos;
    buf.copy_within(pos..10, 0);
    len
}
