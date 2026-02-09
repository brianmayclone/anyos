// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Image Viewer — displays BMP, PNG, JPEG, and GIF images.

#![no_std]
#![no_main]

use anyos_std::{Vec, vec};
use anyos_std::ui::window;
use uisys_client::*;

anyos_std::entry!(main);

const NAVBAR_H: i32 = 44;
const EVENT_MOUSE_SCROLL: u32 = 7;

// Dark theme background
const BG: u32 = 0xFF1E1E1E;
const CHECKER_A: u32 = 0xFF3C3C3C;
const CHECKER_B: u32 = 0xFF2C2C2C;
const CHECKER_SIZE: i32 = 8;

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

    // Allocate pixel + scratch buffers
    let mut pixels = vec![0u32; img_w * img_h];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    // Decode
    if let Err(e) = libimage_client::decode(&data, &mut pixels, &mut scratch) {
        anyos_std::println!("imgview: decode error: {:?}", e);
        return;
    }

    // Free scratch (no longer needed)
    drop(scratch);
    drop(data);

    // Extract filename
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Window dimensions: fit image + navbar, capped to screen
    let (scr_w, scr_h) = window::screen_size();
    let (scr_w, scr_h) = if scr_w == 0 || scr_h == 0 { (1024, 768) } else { (scr_w, scr_h) };
    let max_w = scr_w.min(1024);
    let max_h = scr_h.min(768);

    let win_w = (img_w as u32 + 2).min(max_w).max(200);
    let win_h = (img_h as u32 + NAVBAR_H as u32 + 2).min(max_h).max(150);

    // Center on screen
    let wx = (scr_w.saturating_sub(win_w)) / 2;
    let wy = (scr_h.saturating_sub(win_h)) / 2;

    // Build title
    let mut title_buf = [0u8; 128];
    let title = format_title(&mut title_buf, filename, fmt_name, img_w, img_h);

    let win = window::create_ex(title, wx as u16, wy as u16, win_w as u16, win_h as u16, 0);
    if win == u32::MAX {
        anyos_std::println!("imgview: failed to create window");
        return;
    }

    let (mut cur_w, mut cur_h) = window::get_size(win).unwrap_or((win_w, win_h));

    // Scroll state
    let mut scroll_x: i32 = 0;
    let mut scroll_y: i32 = 0;

    // Drag state for panning
    let mut dragging = false;
    let mut drag_start_x: i32 = 0;
    let mut drag_start_y: i32 = 0;
    let mut drag_scroll_x: i32 = 0;
    let mut drag_scroll_y: i32 = 0;

    let mut needs_redraw = true;

    loop {
        let mut event_raw = [0u32; 5];
        while window::get_event(win, &mut event_raw) != 0 {
            let ev = UiEvent::from_raw(&event_raw);

            match ev.event_type {
                EVENT_RESIZE => {
                    let new_w = ev.p1;
                    let new_h = ev.p2;
                    if new_w != cur_w || new_h != cur_h {
                        cur_w = new_w;
                        cur_h = new_h;
                        clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                        needs_redraw = true;
                    }
                }
                EVENT_MOUSE_DOWN => {
                    let (mx, my) = ev.mouse_pos();
                    if my > NAVBAR_H {
                        dragging = true;
                        drag_start_x = mx;
                        drag_start_y = my;
                        drag_scroll_x = scroll_x;
                        drag_scroll_y = scroll_y;
                    }
                }
                EVENT_MOUSE_UP => {
                    dragging = false;
                }
                EVENT_MOUSE_MOVE => {
                    if dragging {
                        let (mx, my) = ev.mouse_pos();
                        scroll_x = drag_scroll_x - (mx - drag_start_x);
                        scroll_y = drag_scroll_y - (my - drag_start_y);
                        clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                        needs_redraw = true;
                    }
                }
                EVENT_MOUSE_SCROLL => {
                    let dz = ev.p1 as i32;
                    scroll_y += dz * 32;
                    clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                    needs_redraw = true;
                }
                EVENT_KEY_DOWN => {
                    let key = ev.key_code();
                    match key {
                        KEY_ESCAPE => {
                            window::destroy(win);
                            return;
                        }
                        KEY_UP => {
                            scroll_y -= 32;
                            clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                            needs_redraw = true;
                        }
                        KEY_DOWN => {
                            scroll_y += 32;
                            clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                            needs_redraw = true;
                        }
                        KEY_LEFT => {
                            scroll_x -= 32;
                            clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                            needs_redraw = true;
                        }
                        KEY_RIGHT => {
                            scroll_x += 32;
                            clamp_scroll(&mut scroll_x, &mut scroll_y, img_w, img_h, cur_w, cur_h);
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
                EVENT_WINDOW_CLOSE => {
                    window::destroy(win);
                    return;
                }
                _ => {}
            }
        }

        if needs_redraw {
            render(win, cur_w, cur_h, &pixels, img_w, img_h, scroll_x, scroll_y);
            needs_redraw = false;
        }

        anyos_std::process::yield_cpu();
    }
}

fn clamp_scroll(sx: &mut i32, sy: &mut i32, img_w: usize, img_h: usize, win_w: u32, win_h: u32) {
    let view_w = win_w as i32;
    let view_h = (win_h as i32 - NAVBAR_H).max(0);

    let max_x = (img_w as i32 - view_w).max(0);
    let max_y = (img_h as i32 - view_h).max(0);

    if *sx < 0 { *sx = 0; }
    if *sx > max_x { *sx = max_x; }
    if *sy < 0 { *sy = 0; }
    if *sy > max_y { *sy = max_y; }
}

fn render(
    win: u32,
    win_w: u32,
    win_h: u32,
    pixels: &[u32],
    img_w: usize,
    img_h: usize,
    scroll_x: i32,
    scroll_y: i32,
) {
    // Clear background
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, BG);

    // Draw navbar
    let nav = UiNavbar::new(0, 0, win_w, false);
    let mut title_buf = [0u8; 64];
    let dims = format_dims(&mut title_buf, img_w, img_h);
    nav.render(win, dims);

    let view_h = (win_h as i32 - NAVBAR_H).max(0);
    let view_w = win_w as i32;

    // Draw checkerboard behind image (shows through transparent areas)
    let img_x = if (img_w as i32) < view_w { (view_w - img_w as i32) / 2 } else { -scroll_x };
    let img_y = if (img_h as i32) < view_h { NAVBAR_H + (view_h - img_h as i32) / 2 } else { NAVBAR_H - scroll_y };

    // Visible region of the image
    let src_x = scroll_x.max(0) as usize;
    let src_y = scroll_y.max(0) as usize;
    let dst_x = img_x.max(0);
    let dst_y = img_y.max(NAVBAR_H);

    let vis_w = (img_w as i32 - src_x as i32).min(view_w).max(0) as usize;
    let vis_h = (img_h as i32 - src_y as i32).min(view_h).max(0) as usize;

    if vis_w == 0 || vis_h == 0 {
        window::present(win);
        return;
    }

    // Draw checkerboard for the visible image area
    draw_checkerboard(win, dst_x, dst_y, vis_w as u32, vis_h as u32);

    // Blit image in row strips (blit has a max buffer, so do rows)
    // For efficiency, blit multiple rows at once
    let strip_h = 64usize; // rows per blit call
    let mut y = 0usize;
    while y < vis_h {
        let rows = strip_h.min(vis_h - y);
        let mut strip = vec![0u32; vis_w * rows];

        for row in 0..rows {
            let sy = src_y + y + row;
            if sy >= img_h { break; }
            let src_row = &pixels[sy * img_w + src_x..sy * img_w + src_x + vis_w];
            let dst_row = &mut strip[row * vis_w..(row + 1) * vis_w];
            dst_row.copy_from_slice(src_row);
        }

        window::blit(
            win,
            dst_x as i16,
            (dst_y + y as i32) as i16,
            vis_w as u16,
            rows as u16,
            &strip,
        );

        y += rows;
    }

    window::present(win);
}

fn draw_checkerboard(win: u32, x: i32, y: i32, w: u32, h: u32) {
    let mut cy = 0i32;
    while cy < h as i32 {
        let rh = CHECKER_SIZE.min(h as i32 - cy) as u16;
        let mut cx = 0i32;
        while cx < w as i32 {
            let rw = CHECKER_SIZE.min(w as i32 - cx) as u16;
            let color = if ((cx / CHECKER_SIZE) + (cy / CHECKER_SIZE)) % 2 == 0 {
                CHECKER_A
            } else {
                CHECKER_B
            };
            window::fill_rect(win, (x + cx) as i16, (y + cy) as i16, rw, rh, color);
            cx += CHECKER_SIZE;
        }
        cy += CHECKER_SIZE;
    }
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut content = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

fn format_title<'a>(buf: &'a mut [u8; 128], filename: &str, fmt: &str, w: usize, h: usize) -> &'a str {
    let mut pos = 0;
    pos = write_str(buf, pos, filename.as_bytes());
    pos = write_str(buf, pos, b" (");
    pos = write_str(buf, pos, fmt.as_bytes());
    pos = write_str(buf, pos, b" ");
    pos = write_num(buf, pos, w);
    pos = write_str(buf, pos, b"x");
    pos = write_num(buf, pos, h);
    pos = write_str(buf, pos, b") - Image Viewer");
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
}

fn format_dims<'a>(buf: &'a mut [u8; 64], w: usize, h: usize) -> &'a str {
    let mut tmp = [0u8; 64];
    let mut p = 0usize;
    p = write_num_small(&mut tmp, p, w);
    p = write_str_small(&mut tmp, p, b"x");
    p = write_num_small(&mut tmp, p, h);
    buf[..p].copy_from_slice(&tmp[..p]);
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn write_str(buf: &mut [u8; 128], mut pos: usize, s: &[u8]) -> usize {
    for &b in s {
        if pos >= 127 { break; }
        buf[pos] = b;
        pos += 1;
    }
    pos
}

fn write_num(buf: &mut [u8; 128], pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    write_str(buf, pos, &tmp[..len])
}

fn write_str_small(buf: &mut [u8; 64], mut pos: usize, s: &[u8]) -> usize {
    for &b in s {
        if pos >= 63 { break; }
        buf[pos] = b;
        pos += 1;
    }
    pos
}

fn write_num_small(buf: &mut [u8; 64], pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    write_str_small(buf, pos, &tmp[..len])
}

fn fmt_usize(mut n: usize, buf: &mut [u8; 10]) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut pos = 10;
    while n > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let len = 10 - pos;
    buf.copy_within(pos..10, 0);
    len
}
