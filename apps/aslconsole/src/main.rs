#![no_std]
#![no_main]

mod asld;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{ipc, process, sys};
use libanyui_client as anyui;

const WIN_W: u32 = 900;
const WIN_H: u32 = 560;
const COLS: usize = 80;
const ROWS: usize = 25;
const PADDING_X: i32 = 12;
const PADDING_Y: i32 = 10;
const FONT_ID_MONO: u32 = 4;
const FONT_SIZE: u16 = 14;
const BG: u32 = 0xFF05070A;
const FG: u32 = 0xFFE9EDF1;
const MUTED: u32 = 0xFF9AA6B2;
const CURSOR: u32 = 0xFF00A8A8;

anyos_std::entry!(main);

struct AppState {
    distro: String,
    input_pipe_name: String,
    canvas: anyui::Canvas,
    status: anyui::Label,
    rows: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
    frame_active: bool,
    frame_width: usize,
    frame_height: usize,
    framebuffer: Vec<u32>,
    diagnostic_rows: Vec<String>,
    next_diagnostic_ms: u32,
    last_error: String,
}

anyos_std::global_app_state!(AppState);

fn main() {
    if !anyui::init() {
        anyos_std::println!("[ASL Console] failed to init libanyui");
        return;
    }

    let distro = selected_distro();
    let win = anyui::Window::new(&format!("ASL Console - {}", distro), -1, -1, WIN_W, WIN_H);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 36);
    toolbar.set_color(0xFF121820);
    toolbar.set_padding(6, 4, 6, 4);
    let icon = toolbar.add_icon_button("");
    icon.set_system_icon("terminal", anyui::IconType::Outline, CURSOR, 20);
    icon.set_enabled(false);
    let title = toolbar.add_label(&format!("ASL Console: {}", distro));
    title.set_font(1);
    title.set_font_size(13);
    title.set_text_color(FG);
    win.add(&toolbar);

    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(0xFF10151C);
    let status = anyui::Label::new("connecting");
    status.set_dock(anyui::DOCK_FILL);
    status.set_font_size(11);
    status.set_text_color(MUTED);
    status.set_color(0xFF10151C);
    status_bar.add(&status);
    win.add(&status_bar);

    let canvas = anyui::Canvas::new(640, 400);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_interactive(true);
    canvas.clear(BG);
    win.add(&canvas);

    unsafe {
        APP = Some(AppState {
            input_pipe_name: format!("asl-input-{}", distro),
            distro,
            canvas,
            status,
            rows: blank_rows(),
            cursor_x: 0,
            cursor_y: 0,
            frame_active: false,
            frame_width: 0,
            frame_height: 0,
            framebuffer: Vec::new(),
            diagnostic_rows: Vec::new(),
            next_diagnostic_ms: 0,
            last_error: String::new(),
        });
    }

    redraw();
    poll_console();

    app().canvas.on_mouse_down(|x, y, button| {
        send_mouse(x, y, button, true);
    });
    app().canvas.on_mouse_up(|x, y, button| {
        send_mouse(x, y, button, false);
    });
    app().canvas.on_mouse_move(|x, y| {
        let (_, _, button) = app().canvas.get_mouse();
        if button != 0 {
            send_mouse_drag(x, y, button);
        }
    });
    app().canvas.on_wheel(|delta| {
        if delta > 0 {
            send_bytes(b"\x1b[<64;1;1M");
        } else {
            send_bytes(b"\x1b[<65;1;1M");
        }
    });
    win.on_key_down(|ke| {
        send_key(ke);
    });
    win.on_close(|_| anyui::quit());

    let _timer_id = anyui::set_timer(120, || {
        poll_console();
    });
    anyui::run();
}

fn selected_distro() -> String {
    let mut args_buf = [0u8; 128];
    let raw = process::args(&mut args_buf).trim();
    let name = raw.split_whitespace().next().unwrap_or("debian");
    if name.is_empty() {
        String::from("debian")
    } else {
        String::from(name)
    }
}

fn blank_rows() -> Vec<String> {
    let mut rows = Vec::new();
    for _ in 0..ROWS {
        rows.push(String::new());
    }
    rows
}

fn poll_console() {
    let command = format!("CONSOLE_CANVAS {}", app().distro);
    match asld::request(&command) {
        Ok(resp) if resp.ok => {
            apply_canvas_lines(&resp.lines);
            apply_diagnostic_fallback();
            app().status.set_text("connected");
            app().last_error.clear();
            redraw();
        }
        Ok(resp) => show_error(&resp.message),
        Err(err) => show_error(err),
    }
}

fn apply_canvas_lines(lines: &[String]) {
    let a = app();
    a.rows = blank_rows();
    a.frame_active = false;
    for line in lines {
        let mut parts = line.splitn(3, '\t');
        match parts.next().unwrap_or("") {
            "kind" => {
                if parts.next().unwrap_or("") == "framebuffer" {
                    a.frame_active = true;
                }
            }
            "row" => {
                let row = parse_usize(parts.next().unwrap_or(""));
                let text = parts.next().unwrap_or("");
                if row < ROWS {
                    a.rows[row] = String::from(text);
                }
            }
            "cursor_x" => a.cursor_x = parse_usize(parts.next().unwrap_or("0")).min(COLS - 1),
            "cursor_y" => a.cursor_y = parse_usize(parts.next().unwrap_or("0")).min(ROWS - 1),
            "fb_width" => a.frame_width = parse_usize(parts.next().unwrap_or("0")),
            "fb_height" => {
                a.frame_height = parse_usize(parts.next().unwrap_or("0"));
                let len = a.frame_width.saturating_mul(a.frame_height);
                if len > 0 && a.framebuffer.len() != len {
                    a.framebuffer = alloc::vec![BG; len];
                }
            }
            "fb_row" => {
                let row = parse_usize(parts.next().unwrap_or(""));
                let data = parts.next().unwrap_or("");
                apply_framebuffer_row(a, row, data);
            }
            _ => {}
        }
    }
}

fn apply_diagnostic_fallback() {
    if canvas_has_output() {
        return;
    }

    let now = sys::uptime_ms();
    if now >= app().next_diagnostic_ms {
        app().next_diagnostic_ms = now.wrapping_add(1000);
        refresh_diagnostic_rows();
    }
    if !app().diagnostic_rows.is_empty() {
        let rows = app().diagnostic_rows.clone();
        let row_count = rows.len();
        app().rows = rows;
        app().cursor_x = 0;
        app().cursor_y = row_count.min(ROWS).saturating_sub(1);
    }
}

fn canvas_has_output() -> bool {
    let a = app();
    a.frame_active || a.rows.iter().any(|row| !row.trim().is_empty())
}

fn refresh_diagnostic_rows() {
    let distro = app().distro.clone();
    let command = format!("VM_STATUS {}", distro);
    match asld::request(&command) {
        Ok(resp) if resp.ok => {
            let backend = asld::field_value(&resp.lines, "backend").unwrap_or("-");
            let state = asld::field_value(&resp.lines, "run_state").unwrap_or("-");
            let memory = asld::field_value(&resp.lines, "guest_memory_mb").unwrap_or("-");
            let exits = asld::field_value(&resp.lines, "total_exits").unwrap_or("0");
            let recent = asld::field_value(&resp.lines, "recent_exit_count").unwrap_or("0");
            let boot = asld::field_value(&resp.lines, "boot_summary").unwrap_or("-");
            let mut rows = blank_rows();
            rows[0] = String::from("ASL Console: no guest framebuffer or serial output yet");
            rows[2] = format!("Distro: {}", distro);
            rows[3] = format!("Backend: {}", backend);
            rows[4] = format!("State: {}", state);
            rows[5] = format!("Memory: {} MiB", memory);
            rows[6] = format!("VM exits: {} total, {} recent", exits, recent);
            rows[7] = format!("Boot: {}", boot);
            app().diagnostic_rows = rows;
        }
        Ok(resp) => {
            let mut rows = blank_rows();
            rows[0] = String::from("ASL Console: VM diagnostics unavailable");
            rows[2] = resp.message;
            app().diagnostic_rows = rows;
        }
        Err(err) => {
            let mut rows = blank_rows();
            rows[0] = String::from("ASL Console: VM diagnostics unavailable");
            rows[2] = String::from(err);
            app().diagnostic_rows = rows;
        }
    }
}

fn show_error(message: &str) {
    if app().last_error != message {
        app().last_error = String::from(message);
        app().status.set_text(message);
    }
}

fn redraw() {
    let a = app();
    a.canvas.clear(BG);
    if a.frame_active && a.frame_width > 0 && a.frame_height > 0 {
        draw_framebuffer(a);
        return;
    }
    let cell_w = cell_width().max(1);
    let cell_h = cell_height().max(1);
    for row in 0..ROWS {
        let y = PADDING_Y + (row as i32 * cell_h);
        if let Some(text) = a.rows.get(row) {
            a.canvas
                .draw_text(PADDING_X, y, FG, FONT_ID_MONO, FONT_SIZE, text);
        }
    }
    let cx = PADDING_X + (a.cursor_x as i32 * cell_w);
    let cy = PADDING_Y + (a.cursor_y as i32 * cell_h) + cell_h - 3;
    a.canvas.fill_rect(cx, cy, cell_w as u32, 2, CURSOR);
}

fn apply_framebuffer_row(a: &mut AppState, row: usize, data: &str) {
    if row >= a.frame_height || a.frame_width == 0 {
        return;
    }
    let start = row.saturating_mul(a.frame_width);
    let end = start.saturating_add(a.frame_width);
    if end > a.framebuffer.len() || data.len() < a.frame_width.saturating_mul(4) {
        return;
    }
    let bytes = data.as_bytes();
    for col in 0..a.frame_width {
        let index = col * 4;
        let Some(pixel) = parse_rgb565(&bytes[index..index + 4]) else {
            return;
        };
        a.framebuffer[start + col] = rgb565_to_argb(pixel);
    }
}

fn draw_framebuffer(a: &AppState) {
    let ptr = a.canvas.get_buffer();
    if ptr.is_null() {
        return;
    }
    let cw = a.canvas.get_stride() as usize;
    let ch = a.canvas.get_height() as usize;
    if cw == 0 || ch == 0 || a.framebuffer.is_empty() {
        return;
    }
    let (rw, rh) = scaled_size(cw, ch, a.frame_width, a.frame_height);
    let ox = (cw.saturating_sub(rw)) / 2;
    let oy = (ch.saturating_sub(rh)) / 2;
    unsafe {
        for y in 0..rh {
            let sy = y * a.frame_height / rh;
            for x in 0..rw {
                let sx = x * a.frame_width / rw;
                let dst = (oy + y) * cw + ox + x;
                let src = sy * a.frame_width + sx;
                *ptr.add(dst) = a.framebuffer[src];
            }
        }
    }
}

fn scaled_size(cw: usize, ch: usize, sw: usize, sh: usize) -> (usize, usize) {
    if sw == 0 || sh == 0 {
        return (0, 0);
    }
    if cw.saturating_mul(sh) <= ch.saturating_mul(sw) {
        (cw, (cw.saturating_mul(sh) / sw).max(1))
    } else {
        ((ch.saturating_mul(sw) / sh).max(1), ch)
    }
}

fn send_key(ke: &anyui::KeyEvent) {
    if ke.ctrl() {
        if let Some(ctrl) = ctrl_code(ke.char_code) {
            send_bytes(&[ctrl]);
            return;
        }
    }

    match ke.keycode {
        anyui::KEY_ENTER => send_bytes(b"\n"),
        anyui::KEY_BACKSPACE => send_bytes(&[0x7f]),
        anyui::KEY_TAB => send_bytes(b"\t"),
        anyui::KEY_ESCAPE => send_bytes(&[0x1b]),
        anyui::KEY_UP => send_bytes(b"\x1b[A"),
        anyui::KEY_DOWN => send_bytes(b"\x1b[B"),
        anyui::KEY_RIGHT => send_bytes(b"\x1b[C"),
        anyui::KEY_LEFT => send_bytes(b"\x1b[D"),
        anyui::KEY_DELETE => send_bytes(b"\x1b[3~"),
        anyui::KEY_HOME => send_bytes(b"\x1b[H"),
        anyui::KEY_END => send_bytes(b"\x1b[F"),
        anyui::KEY_PAGE_UP => send_bytes(b"\x1b[5~"),
        anyui::KEY_PAGE_DOWN => send_bytes(b"\x1b[6~"),
        _ => send_char(ke.char_code),
    }
}

fn send_char(code: u32) {
    let Some(ch) = core::char::from_u32(code) else {
        return;
    };
    if ch == '\0' || ch.is_control() {
        return;
    }
    let mut buf = [0u8; 4];
    let text = ch.encode_utf8(&mut buf);
    send_bytes(text.as_bytes());
}

fn ctrl_code(code: u32) -> Option<u8> {
    let lower = if (b'a' as u32..=b'z' as u32).contains(&code) {
        code as u8
    } else if (b'A' as u32..=b'Z' as u32).contains(&code) {
        (code as u8) + 32
    } else {
        return None;
    };
    Some(lower - b'a' + 1)
}

fn send_mouse(x: i32, y: i32, button: u32, down: bool) {
    let code = match button {
        1 => 0,
        2 => 1,
        3 => 2,
        _ => 0,
    };
    send_mouse_sgr(code, x, y, down);
}

fn send_mouse_drag(x: i32, y: i32, button: u32) {
    let base = match button {
        1 => 32,
        2 => 33,
        3 => 34,
        _ => 32,
    };
    send_mouse_sgr(base, x, y, true);
}

fn send_mouse_sgr(code: u32, x: i32, y: i32, press: bool) {
    let col = pixel_to_col(x);
    let row = pixel_to_row(y);
    let suffix = if press { "M" } else { "m" };
    let seq = format!("\x1b[<{};{};{}{}", code, col, row, suffix);
    send_bytes(seq.as_bytes());
}

fn send_bytes(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let pipe = ipc::pipe_open(&app().input_pipe_name);
    if pipe == 0 || pipe == u32::MAX {
        app().status.set_text("input pipe unavailable");
        return;
    }
    let written = ipc::pipe_write(pipe, bytes);
    let _ = ipc::pipe_close(pipe);
    if written == u32::MAX {
        app().status.set_text("input write failed");
    }
}

fn cell_width() -> i32 {
    let width = app().canvas.get_stride() as i32;
    ((width - PADDING_X * 2) / COLS as i32).max(1)
}

fn cell_height() -> i32 {
    let height = app().canvas.get_height() as i32;
    ((height - PADDING_Y * 2) / ROWS as i32).max(1)
}

fn pixel_to_col(x: i32) -> u32 {
    (((x - PADDING_X).max(0) / cell_width()) as u32 + 1).min(COLS as u32)
}

fn pixel_to_row(y: i32) -> u32 {
    (((y - PADDING_Y).max(0) / cell_height()) as u32 + 1).min(ROWS as u32)
}

fn parse_usize(text: &str) -> usize {
    let mut n = 0usize;
    for b in text.bytes() {
        if !b.is_ascii_digit() {
            return 0;
        }
        n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
    }
    n
}

fn parse_rgb565(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 4 {
        return None;
    }
    let mut value = 0u16;
    for &byte in bytes {
        value = (value << 4) | hex_nibble(byte)? as u16;
    }
    Some(value)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn rgb565_to_argb(pixel: u16) -> u32 {
    let r = ((pixel >> 11) & 0x1f) as u32;
    let g = ((pixel >> 5) & 0x3f) as u32;
    let b = (pixel & 0x1f) as u32;
    let r8 = (r << 3) | (r >> 2);
    let g8 = (g << 2) | (g >> 4);
    let b8 = (b << 3) | (b >> 2);
    0xff00_0000 | (r8 << 16) | (g8 << 8) | b8
}
