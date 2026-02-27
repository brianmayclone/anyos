// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Video Player — plays MJV (Motion JPEG Video) files.

#![no_std]
#![no_main]

use anyos_std::{Vec, vec};
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

const BG: u32 = 0xFF1E1E1E;
const CONTROLS_H: u32 = 32;
const KEY_SPACE: u32 = 0x104;

struct AppState {
    canvas: anyui::Canvas,
    data: Vec<u8>,
    num_frames: u32,
    fps: u32,
    vid_w: usize,
    vid_h: usize,
    pixels: Vec<u32>,
    scratch: Vec<u8>,
    render_buf: Vec<u32>,
    current_frame: u32,
    playing: bool,
    looping: bool,
    start_tick: u32,
    pause_elapsed: u32,
    tick_hz: u32,
    needs_redraw: bool,
    lbl_status: anyui::Label,
    lbl_time: anyui::Label,
    progress: anyui::ProgressBar,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

fn main() {
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("videoplayer: no file specified");
        anyos_std::println!("Usage: videoplayer <file.mjv>");
        return;
    }

    let data = match read_file(path) {
        Some(d) => d,
        None => {
            anyos_std::println!("videoplayer: cannot open '{}'", path);
            return;
        }
    };

    let info = match libimage_client::video_probe(&data) {
        Some(i) => i,
        None => {
            anyos_std::println!("videoplayer: unsupported format or corrupt file");
            return;
        }
    };

    let vid_w = info.width as usize;
    let vid_h = info.height as usize;
    let pixels = vec![0u32; vid_w * vid_h];
    let scratch = vec![0u8; info.scratch_needed as usize];
    let num_frames = info.num_frames;
    let fps = info.fps;

    let filename = path.rsplit('/').next().unwrap_or(path);

    if !anyui::init() { return; }

    let (scr_w, scr_h) = anyui::screen_size();
    let (scr_w, scr_h) = if scr_w == 0 || scr_h == 0 { (1024, 768) } else { (scr_w, scr_h) };
    let win_w = (vid_w as u32 + 2).min(scr_w).max(200);
    let win_h = (vid_h as u32 + CONTROLS_H + 2).min(scr_h).max(150);

    let mut title_buf = [0u8; 128];
    let title = format_title(&mut title_buf, filename, vid_w, vid_h, fps);

    let win = anyui::Window::new(title, -1, -1, win_w, win_h);

    // Bottom controls bar (DOCK_BOTTOM, add before DOCK_FILL)
    let controls = anyui::View::new();
    controls.set_dock(anyui::DOCK_BOTTOM);
    controls.set_size(win_w, CONTROLS_H);
    controls.set_color(0xFF2A2A2A);

    let lbl_status = anyui::Label::new("||");
    lbl_status.set_position(8, 8);
    lbl_status.set_size(20, 16);
    lbl_status.set_text_color(0xFFE6E6E6);
    controls.add(&lbl_status);

    let lbl_time = anyui::Label::new("0:00 / 0:00");
    lbl_time.set_position(28, 8);
    lbl_time.set_size(80, 16);
    lbl_time.set_text_color(0xFF888888);
    controls.add(&lbl_time);

    let progress = anyui::ProgressBar::new(0);
    progress.set_position(110, 12);
    progress.set_size(win_w.saturating_sub(120), 6);
    controls.add(&progress);

    win.add(&controls);

    // Canvas for video frames (DOCK_FILL, add last)
    let canvas = anyui::Canvas::new(win_w, win_h.saturating_sub(CONTROLS_H));
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_color(BG);
    win.add(&canvas);

    let tick_hz = anyos_std::sys::tick_hz();

    unsafe {
        APP = Some(AppState {
            canvas,
            data,
            num_frames,
            fps,
            vid_w,
            vid_h,
            pixels,
            scratch,
            render_buf: Vec::new(),
            current_frame: 0,
            playing: true,
            looping: true,
            start_tick: anyos_std::sys::uptime(),
            pause_elapsed: 0,
            tick_hz,
            needs_redraw: true,
            lbl_status,
            lbl_time,
            progress,
        });
    }

    // Keyboard events
    win.on_key_down(|ke| {
        match ke.keycode {
            anyui::KEY_ESCAPE => anyui::quit(),
            KEY_SPACE => {
                let a = app();
                if a.playing {
                    a.pause_elapsed = anyos_std::sys::uptime().wrapping_sub(a.start_tick);
                    a.playing = false;
                    a.lbl_status.set_text(">");
                } else {
                    a.start_tick = anyos_std::sys::uptime().wrapping_sub(a.pause_elapsed);
                    a.playing = true;
                    a.lbl_status.set_text("||");
                }
            }
            anyui::KEY_LEFT => {
                let a = app();
                let back = a.fps;
                a.current_frame = a.current_frame.saturating_sub(back);
                seek_to_current();
            }
            anyui::KEY_RIGHT => {
                let a = app();
                let fwd = a.fps;
                a.current_frame = (a.current_frame + fwd).min(a.num_frames - 1);
                seek_to_current();
            }
            _ => {}
        }
    });

    win.on_close(|_| anyui::quit());

    // Playback timer (16ms ~ 60fps)
    anyui::set_timer(16, || {
        let a = app();
        if a.playing {
            let elapsed = anyos_std::sys::uptime().wrapping_sub(a.start_tick);
            let target_frame = (elapsed as u64 * a.fps as u64 / a.tick_hz as u64) as u32;
            let target_frame = target_frame.min(a.num_frames - 1);

            if target_frame != a.current_frame {
                a.current_frame = target_frame;
                a.needs_redraw = true;
            }

            if a.current_frame >= a.num_frames - 1 {
                if a.looping {
                    a.current_frame = 0;
                    a.start_tick = anyos_std::sys::uptime();
                    a.needs_redraw = true;
                } else {
                    a.playing = false;
                    a.lbl_status.set_text(">");
                }
            }
        }

        if a.needs_redraw {
            a.needs_redraw = false;
            redraw();
            update_controls();
        }
    });

    anyui::run();
}

fn seek_to_current() {
    let a = app();
    let frame_ticks = if a.fps > 0 {
        (a.current_frame as u64 * a.tick_hz as u64 / a.fps as u64) as u32
    } else { 0 };
    if a.playing {
        a.start_tick = anyos_std::sys::uptime().wrapping_sub(frame_ticks);
    } else {
        a.pause_elapsed = frame_ticks;
    }
    a.needs_redraw = true;
}

fn redraw() {
    let a = app();

    // Decode frame
    let _ = libimage_client::video_decode_frame(
        &a.data, a.num_frames, a.current_frame, &mut a.pixels, &mut a.scratch,
    );

    let w = a.canvas.get_stride() as usize;
    let h = a.canvas.get_height() as usize;
    if w == 0 || h == 0 { return; }

    let needed = w * h;
    if a.render_buf.len() != needed {
        a.render_buf.resize(needed, BG);
    }

    for p in a.render_buf.iter_mut() { *p = BG; }

    // Center video in canvas
    let offset_x = if a.vid_w < w { (w - a.vid_w) / 2 } else { 0 };
    let offset_y = if a.vid_h < h { (h - a.vid_h) / 2 } else { 0 };
    let vis_w = a.vid_w.min(w);
    let vis_h = a.vid_h.min(h);

    for row in 0..vis_h {
        let src_start = row * a.vid_w;
        let dst_start = (offset_y + row) * w + offset_x;
        a.render_buf[dst_start..dst_start + vis_w]
            .copy_from_slice(&a.pixels[src_start..src_start + vis_w]);
    }

    a.canvas.copy_pixels_from(&a.render_buf);
}

fn update_controls() {
    let a = app();
    let current_sec = if a.fps > 0 { a.current_frame / a.fps } else { 0 };
    let total_sec = if a.fps > 0 { a.num_frames / a.fps } else { 0 };
    let mut time_buf = [0u8; 32];
    let time_str = format_time(&mut time_buf, current_sec, total_sec);
    a.lbl_time.set_text(time_str);

    if a.num_frames > 0 {
        let pct = ((a.current_frame as u64 + 1) * 100 / a.num_frames as u64) as u32;
        a.progress.set_state(pct.min(100));
    }
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

fn format_title<'a>(buf: &'a mut [u8; 128], filename: &str, w: usize, h: usize, fps: u32) -> &'a str {
    let mut pos = 0;
    for &b in filename.as_bytes() { if pos < 127 { buf[pos] = b; pos += 1; } }
    for &b in b" (" { if pos < 127 { buf[pos] = b; pos += 1; } }
    pos = write_num(buf, pos, w);
    if pos < 127 { buf[pos] = b'x'; pos += 1; }
    pos = write_num(buf, pos, h);
    for &b in b" @ " { if pos < 127 { buf[pos] = b; pos += 1; } }
    pos = write_num(buf, pos, fps as usize);
    for &b in b" fps) - Video Player" { if pos < 127 { buf[pos] = b; pos += 1; } }
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
}

fn format_time<'a>(buf: &'a mut [u8; 32], current: u32, total: u32) -> &'a str {
    let mut pos = 0usize;
    pos = write_num_small(buf, pos, (current / 60) as usize);
    buf[pos] = b':'; pos += 1;
    let sec = (current % 60) as usize;
    if sec < 10 { buf[pos] = b'0'; pos += 1; }
    pos = write_num_small(buf, pos, sec);
    buf[pos] = b' '; pos += 1;
    buf[pos] = b'/'; pos += 1;
    buf[pos] = b' '; pos += 1;
    pos = write_num_small(buf, pos, (total / 60) as usize);
    buf[pos] = b':'; pos += 1;
    let sec = (total % 60) as usize;
    if sec < 10 { buf[pos] = b'0'; pos += 1; }
    pos = write_num_small(buf, pos, sec);
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
}

fn write_num(buf: &mut [u8; 128], pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    let mut p = pos;
    for &b in &tmp[..len] { if p < 127 { buf[p] = b; p += 1; } }
    p
}

fn write_num_small(buf: &mut [u8; 32], mut pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    for &b in &tmp[..len] { if pos < 31 { buf[pos] = b; pos += 1; } }
    pos
}

fn fmt_usize(mut n: usize, buf: &mut [u8; 10]) -> usize {
    if n == 0 { buf[0] = b'0'; return 1; }
    let mut pos = 10;
    while n > 0 && pos > 0 { pos -= 1; buf[pos] = b'0' + (n % 10) as u8; n /= 10; }
    let len = 10 - pos;
    buf.copy_within(pos..10, 0);
    len
}
