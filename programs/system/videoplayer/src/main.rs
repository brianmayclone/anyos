// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Video Player — plays MJV (Motion JPEG Video) files.

#![no_std]
#![no_main]

use anyos_std::{Vec, vec};
use anyos_std::ui::window;
use uisys_client::*;

anyos_std::entry!(main);

const NAVBAR_H: i32 = 44;
const PROGRESS_H: i32 = 32;

// Dark theme colors
const BG: u32 = 0xFF1E1E1E;
const PROGRESS_BG: u32 = 0xFF2A2A2A;
const PROGRESS_FG: u32 = 0xFF0A84FF; // macOS blue accent
const TEXT_COLOR: u32 = 0xFFE6E6E6;
const TEXT_DIM: u32 = 0xFF888888;

fn main() {
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("videoplayer: no file specified");
        anyos_std::println!("Usage: videoplayer <file.mjv>");
        return;
    }

    // Read video file
    let data = match read_file(path) {
        Some(d) => d,
        None => {
            anyos_std::println!("videoplayer: cannot open '{}'", path);
            return;
        }
    };

    // Probe video
    let info = match libimage_client::video_probe(&data) {
        Some(i) => i,
        None => {
            anyos_std::println!("videoplayer: unsupported format or corrupt file");
            return;
        }
    };

    let vid_w = info.width as usize;
    let vid_h = info.height as usize;

    // Allocate pixel + scratch buffers
    let mut pixels = vec![0u32; vid_w * vid_h];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    // Extract filename
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Window dimensions
    let (scr_w, scr_h) = window::screen_size();
    let (scr_w, scr_h) = if scr_w == 0 || scr_h == 0 { (1024, 768) } else { (scr_w, scr_h) };

    let win_w = (vid_w as u32 + 2).min(scr_w).max(200);
    let win_h = (vid_h as u32 + NAVBAR_H as u32 + PROGRESS_H as u32 + 2).min(scr_h).max(150);

    // Center on screen
    let wx = scr_w.saturating_sub(win_w) / 2;
    let wy = scr_h.saturating_sub(win_h) / 2;

    // Build title
    let mut title_buf = [0u8; 128];
    let title = format_title(&mut title_buf, filename, vid_w, vid_h, info.fps);

    let win = window::create_ex(title, wx as u16, wy as u16, win_w as u16, win_h as u16, 0);
    if win == u32::MAX {
        anyos_std::println!("videoplayer: failed to create window");
        return;
    }

    let (mut cur_w, mut cur_h) = window::get_size(win).unwrap_or((win_w, win_h));

    // Playback state
    let mut playing = true;
    let mut current_frame: u32 = 0;
    let tick_hz = anyos_std::sys::tick_hz();
    let mut start_tick = anyos_std::sys::uptime();
    let mut pause_elapsed: u32 = 0; // ticks elapsed when paused
    let mut looping = true;

    // Decode and display first frame
    decode_and_blit(win, &data, &info, 0, &mut pixels, &mut scratch, cur_w);
    draw_chrome(win, cur_w, cur_h, filename, &info, 0, playing);
    window::present(win);

    loop {
        let mut event_raw = [0u32; 5];
        while window::get_event(win, &mut event_raw) != 0 {
            let ev = UiEvent::from_raw(&event_raw);

            match ev.event_type {
                EVENT_KEY_DOWN => {
                    match ev.key_code() {
                        KEY_ESCAPE => {
                            window::destroy(win);
                            return;
                        }
                        KEY_SPACE => {
                            if playing {
                                // Pause: record elapsed ticks
                                pause_elapsed = anyos_std::sys::uptime().wrapping_sub(start_tick);
                                playing = false;
                            } else {
                                // Resume: adjust start_tick so elapsed stays continuous
                                start_tick = anyos_std::sys::uptime().wrapping_sub(pause_elapsed);
                                playing = true;
                            }
                            draw_chrome(win, cur_w, cur_h, filename, &info, current_frame, playing);
                            window::present(win);
                        }
                        KEY_LEFT => {
                            // Seek back 1 second
                            let frames_back = info.fps;
                            current_frame = current_frame.saturating_sub(frames_back);
                            seek_to(win, &data, &info, current_frame, &mut pixels, &mut scratch,
                                    cur_w, cur_h, filename, playing,
                                    &mut start_tick, &mut pause_elapsed, tick_hz);
                        }
                        KEY_RIGHT => {
                            // Seek forward 1 second
                            let frames_fwd = info.fps;
                            current_frame = (current_frame + frames_fwd).min(info.num_frames - 1);
                            seek_to(win, &data, &info, current_frame, &mut pixels, &mut scratch,
                                    cur_w, cur_h, filename, playing,
                                    &mut start_tick, &mut pause_elapsed, tick_hz);
                        }
                        _ => {}
                    }
                }
                EVENT_RESIZE => {
                    let new_w = ev.p1;
                    let new_h = ev.p2;
                    if new_w != cur_w || new_h != cur_h {
                        cur_w = new_w;
                        cur_h = new_h;
                        // Redraw current frame at new size
                        window::fill_rect(win, 0, 0, cur_w as u16, cur_h as u16, BG);
                        decode_and_blit(win, &data, &info, current_frame, &mut pixels, &mut scratch, cur_w);
                        draw_chrome(win, cur_w, cur_h, filename, &info, current_frame, playing);
                        window::present(win);
                    }
                }
                EVENT_WINDOW_CLOSE => {
                    window::destroy(win);
                    return;
                }
                _ => {}
            }
        }

        // Advance playback
        if playing {
            let elapsed = anyos_std::sys::uptime().wrapping_sub(start_tick);
            let target_frame = (elapsed as u64 * info.fps as u64 / tick_hz as u64) as u32;
            let target_frame = target_frame.min(info.num_frames - 1);

            if target_frame != current_frame {
                current_frame = target_frame;
                decode_and_blit(win, &data, &info, current_frame, &mut pixels, &mut scratch, cur_w);
                draw_chrome(win, cur_w, cur_h, filename, &info, current_frame, playing);
                window::present(win);
            }

            if current_frame >= info.num_frames - 1 {
                if looping {
                    // Loop: restart
                    current_frame = 0;
                    start_tick = anyos_std::sys::uptime();
                    decode_and_blit(win, &data, &info, 0, &mut pixels, &mut scratch, cur_w);
                    draw_chrome(win, cur_w, cur_h, filename, &info, 0, playing);
                    window::present(win);
                } else {
                    playing = false;
                    draw_chrome(win, cur_w, cur_h, filename, &info, current_frame, playing);
                    window::present(win);
                }
            }
        }

        anyos_std::process::yield_cpu();
    }
}

fn seek_to(
    win: u32,
    data: &[u8],
    info: &libimage_client::VideoInfo,
    frame: u32,
    pixels: &mut [u32],
    scratch: &mut [u8],
    cur_w: u32,
    cur_h: u32,
    filename: &str,
    playing: bool,
    start_tick: &mut u32,
    pause_elapsed: &mut u32,
    tick_hz: u32,
) {
    // Recalculate timing for the new frame position
    let frame_ticks = (frame as u64 * tick_hz as u64 / info.fps as u64) as u32;
    if playing {
        *start_tick = anyos_std::sys::uptime().wrapping_sub(frame_ticks);
    } else {
        *pause_elapsed = frame_ticks;
    }

    decode_and_blit(win, data, info, frame, pixels, scratch, cur_w);
    draw_chrome(win, cur_w, cur_h, filename, info, frame, playing);
    window::present(win);
}

fn decode_and_blit(
    win: u32,
    data: &[u8],
    info: &libimage_client::VideoInfo,
    frame_idx: u32,
    pixels: &mut [u32],
    scratch: &mut [u8],
    win_w: u32,
) {
    // Decode frame
    let _ = libimage_client::video_decode_frame(
        data, info.num_frames, frame_idx, pixels, scratch,
    );

    let w = info.width as usize;
    let h = info.height as usize;

    // Center video horizontally
    let offset_x = if (w as u32) < win_w {
        ((win_w - w as u32) / 2) as i16
    } else {
        0
    };

    // Blit in 64-row strips (same pattern as imgview)
    let strip_h = 64usize;
    let mut y = 0usize;
    while y < h {
        let rows = strip_h.min(h - y);
        window::blit(
            win,
            offset_x,
            (NAVBAR_H as i16) + y as i16,
            w as u16,
            rows as u16,
            &pixels[y * w..(y + rows) * w],
        );
        y += rows;
    }
}

fn draw_chrome(
    win: u32,
    win_w: u32,
    win_h: u32,
    filename: &str,
    info: &libimage_client::VideoInfo,
    current_frame: u32,
    playing: bool,
) {
    // Navbar
    let nav = UiNavbar::new(0, 0, win_w, false);
    nav.render(win, filename);

    // Progress bar area
    let prog_y = win_h as i16 - PROGRESS_H as i16;
    window::fill_rect(win, 0, prog_y, win_w as u16, PROGRESS_H as u16, PROGRESS_BG);

    // Play/Pause indicator
    let status = if playing { "||" } else { ">" };
    window::draw_text(win, 8, prog_y + 9, TEXT_COLOR, status);

    // Time display: current / total
    let current_sec = if info.fps > 0 { current_frame / info.fps } else { 0 };
    let total_sec = if info.fps > 0 { info.num_frames / info.fps } else { 0 };

    let mut time_buf = [0u8; 32];
    let time_str = format_time(&mut time_buf, current_sec, total_sec);
    window::draw_text(win, 28, prog_y + 9, TEXT_DIM, time_str);

    // Progress bar
    let bar_x: i16 = 100;
    let bar_w = (win_w as i16 - bar_x - 8).max(0) as u32;
    let bar_y = prog_y + 12;
    let bar_h: u16 = 6;

    // Background track
    window::fill_rect(win, bar_x, bar_y, bar_w as u16, bar_h, 0xFF444444);

    // Filled portion
    if info.num_frames > 0 && bar_w > 0 {
        let fill_w = ((current_frame as u64 + 1) * bar_w as u64 / info.num_frames as u64) as u16;
        if fill_w > 0 {
            window::fill_rect(win, bar_x, bar_y, fill_w, bar_h, PROGRESS_FG);
        }
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

fn format_title<'a>(buf: &'a mut [u8; 128], filename: &str, w: usize, h: usize, fps: u32) -> &'a str {
    let mut pos = 0;
    pos = write_str(buf, pos, filename.as_bytes());
    pos = write_str(buf, pos, b" (");
    pos = write_num(buf, pos, w);
    pos = write_str(buf, pos, b"x");
    pos = write_num(buf, pos, h);
    pos = write_str(buf, pos, b" @ ");
    pos = write_num(buf, pos, fps as usize);
    pos = write_str(buf, pos, b" fps) - Video Player");
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
}

fn format_time<'a>(buf: &'a mut [u8; 32], current: u32, total: u32) -> &'a str {
    let mut pos = 0usize;
    // current time
    pos = write_num_small(buf, pos, (current / 60) as usize);
    buf[pos] = b':';
    pos += 1;
    let sec = (current % 60) as usize;
    if sec < 10 {
        buf[pos] = b'0';
        pos += 1;
    }
    pos = write_num_small(buf, pos, sec);
    // separator
    buf[pos] = b' ';
    pos += 1;
    buf[pos] = b'/';
    pos += 1;
    buf[pos] = b' ';
    pos += 1;
    // total time
    pos = write_num_small(buf, pos, (total / 60) as usize);
    buf[pos] = b':';
    pos += 1;
    let sec = (total % 60) as usize;
    if sec < 10 {
        buf[pos] = b'0';
        pos += 1;
    }
    pos = write_num_small(buf, pos, sec);
    unsafe { core::str::from_utf8_unchecked(&buf[..pos]) }
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

fn write_num_small(buf: &mut [u8; 32], mut pos: usize, n: usize) -> usize {
    let mut tmp = [0u8; 10];
    let len = fmt_usize(n, &mut tmp);
    for &b in &tmp[..len] {
        if pos >= 31 { break; }
        buf[pos] = b;
        pos += 1;
    }
    pos
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
