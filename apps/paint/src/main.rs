//! Paint — Microsoft Paint-style drawing application for anyOS.
//!
//! Provides freehand drawing (pencil, brush, eraser), shape tools
//! (line, rectangle, ellipse), flood fill, color picker, and BMP save/load.

#![no_std]
#![no_main]

use anyos_std::{Vec, vec};
use libanyui_client as ui;

anyos_std::entry!(main);

// ── Color palette (18 colors, 6x3 grid) ─────────────────────────────

const PALETTE: [u32; 18] = [
    0xFF000000, // Black
    0xFFFFFFFF, // White
    0xFFFF0000, // Red
    0xFF00B050, // Green
    0xFF0070C0, // Blue
    0xFFFFD700, // Yellow
    0xFF404040, // Dark Gray
    0xFFC0C0C0, // Light Gray
    0xFF800000, // Dark Red
    0xFF006400, // Dark Green
    0xFF00008B, // Dark Blue
    0xFFFF8C00, // Orange
    0xFF8B4513, // Brown
    0xFFFF69B4, // Pink
    0xFF00CED1, // Cyan
    0xFFFF00FF, // Magenta
    0xFF800080, // Purple
    0xFF008080, // Teal
];

// ── Tool types ───────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pencil,
    Brush,
    Eraser,
    Line,
    Rect,
    Ellipse,
    Fill,
    Picker,
}

const TOOL_NAMES: [&str; 8] = [
    "Pencil", "Brush", "Eraser", "Line", "Rect", "Ellipse", "Fill", "Picker",
];

// ── Canvas dimensions ────────────────────────────────────────────────

const CANVAS_W: u32 = 640;
const CANVAS_H: u32 = 480;
const SIDEBAR_W: u32 = 80;

// ── Application state (global, accessed from callbacks) ──────────────

static mut APP: Option<PaintApp> = None;

fn app() -> &'static mut PaintApp {
    unsafe { APP.as_mut().unwrap() }
}

struct PaintApp {
    canvas: ui::Canvas,
    tool: Tool,
    brush_size: u32,
    fg_color: u32,
    bg_color: u32,
    is_drawing: bool,
    start_x: i32,
    start_y: i32,
    last_x: i32,
    last_y: i32,
    /// Pixel snapshot for shape preview (save/restore during drag).
    snapshot: Vec<u32>,
    status_pos: ui::Label,
    status_tool: ui::Label,
    fg_swatch: ui::View,
    bg_swatch: ui::View,
    tool_buttons: [ui::Button; 8],
    size_label: ui::Label,
}

fn main() {
    if !ui::init() {
        anyos_std::println!("[Paint] Failed to init libanyui");
        return;
    }

    let win = ui::Window::new("Paint", -1, -1, SIDEBAR_W + CANVAS_W + 4, CANVAS_H + 42 + 24 + 4);
    let tc = ui::theme::colors();

    // ── Toolbar (top) ────────────────────────────────────────────────
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(SIDEBAR_W + CANVAS_W + 4, 42);
    toolbar.set_padding(4, 4, 4, 4);
    win.add(&toolbar);

    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_system_icon("file-plus", ui::IconType::Outline, tc.text, 24);
    let btn_open = toolbar.add_icon_button("");
    btn_open.set_size(34, 34);
    btn_open.set_system_icon("folder-open", ui::IconType::Outline, tc.text, 24);
    let btn_save = toolbar.add_icon_button("");
    btn_save.set_size(34, 34);
    btn_save.set_system_icon("device-floppy", ui::IconType::Outline, tc.text, 24);
    toolbar.add_separator();

    let size_label = ui::Label::new("Size: 3");
    size_label.set_size(52, 24);
    size_label.set_text_color(tc.text);
    toolbar.add(&size_label);

    let btn_size_down = toolbar.add_button("-");
    let btn_size_up = toolbar.add_button("+");

    // ── Sidebar (left) ───────────────────────────────────────────────
    let sidebar = ui::View::new();
    sidebar.set_dock(ui::DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, CANVAS_H + 24);
    sidebar.set_color(tc.window_bg);
    win.add(&sidebar);

    // Tool buttons
    let tool_labels = ["Pencil", "Brush", "Eraser", "Line", "Rect", "Ellipse", "Fill", "Picker"];
    let mut tool_buttons: [ui::Button; 8] = unsafe { core::mem::zeroed() };
    for (i, &label) in tool_labels.iter().enumerate() {
        let btn = ui::Button::new(label);
        btn.set_position(4, 4 + (i as i32) * 28);
        btn.set_size(SIDEBAR_W - 8, 24);
        sidebar.add(&btn);
        tool_buttons[i] = btn;
    }

    // Color palette (6x3 grid)
    let palette_y = 4 + 8 * 28 + 8;
    for (i, &color) in PALETTE.iter().enumerate() {
        let col = (i % 6) as i32;
        let row = (i / 6) as i32;
        let swatch = ui::View::new();
        swatch.set_position(4 + col * 12, palette_y + row * 12);
        swatch.set_size(11, 11);
        swatch.set_color(color);
        sidebar.add(&swatch);

        // Register click on each color swatch
        let c = color;
        swatch.on_click_raw(palette_click_handler, c as u64);
    }

    // FG/BG display
    let fg_label = ui::Label::new("FG");
    fg_label.set_position(4, palette_y + 44);
    fg_label.set_size(20, 16);
    fg_label.set_text_color(tc.text_secondary);
    fg_label.set_font_size(11);
    sidebar.add(&fg_label);

    let fg_swatch = ui::View::new();
    fg_swatch.set_position(26, palette_y + 42);
    fg_swatch.set_size(20, 20);
    fg_swatch.set_color(0xFF000000);
    sidebar.add(&fg_swatch);

    let bg_label = ui::Label::new("BG");
    bg_label.set_position(4, palette_y + 68);
    bg_label.set_size(20, 16);
    bg_label.set_text_color(tc.text_secondary);
    bg_label.set_font_size(11);
    sidebar.add(&bg_label);

    let bg_swatch = ui::View::new();
    bg_swatch.set_position(26, palette_y + 66);
    bg_swatch.set_size(20, 20);
    bg_swatch.set_color(0xFFFFFFFF);
    sidebar.add(&bg_swatch);

    // ── Canvas (center) ──────────────────────────────────────────────
    let canvas = ui::Canvas::new(CANVAS_W, CANVAS_H);
    canvas.set_dock(ui::DOCK_FILL);
    canvas.set_interactive(true);
    canvas.clear(0xFFFFFFFF);
    win.add(&canvas);

    // ── Status bar (bottom) ──────────────────────────────────────────
    let status = ui::View::new();
    status.set_dock(ui::DOCK_BOTTOM);
    status.set_size(SIDEBAR_W + CANVAS_W + 4, 22);
    status.set_color(tc.control_bg);
    win.add(&status);

    let status_pos = ui::Label::new("0, 0");
    status_pos.set_position(8, 3);
    status_pos.set_size(120, 16);
    status_pos.set_text_color(tc.text_secondary);
    status_pos.set_font_size(11);
    status.add(&status_pos);

    let status_tool = ui::Label::new("Pencil");
    status_tool.set_position(200, 3);
    status_tool.set_size(100, 16);
    status_tool.set_text_color(tc.text_secondary);
    status_tool.set_font_size(11);
    status.add(&status_tool);

    let status_dim = ui::Label::new("640 x 480");
    status_dim.set_position(340, 3);
    status_dim.set_size(100, 16);
    status_dim.set_text_color(tc.text_secondary);
    status_dim.set_font_size(11);
    status.add(&status_dim);

    // ── Initialize app state ─────────────────────────────────────────
    unsafe {
        APP = Some(PaintApp {
            canvas,
            tool: Tool::Pencil,
            brush_size: 3,
            fg_color: 0xFF000000,
            bg_color: 0xFFFFFFFF,
            is_drawing: false,
            start_x: 0,
            start_y: 0,
            last_x: 0,
            last_y: 0,
            snapshot: Vec::new(),
            status_pos,
            status_tool,
            fg_swatch,
            bg_swatch,
            tool_buttons,
            size_label,
        });
    }

    highlight_active_tool(0);

    // ── Register callbacks ───────────────────────────────────────────

    // Canvas events
    app().canvas.on_mouse_down(|x, y, button| {
        on_mouse_down(x, y, button);
    });
    app().canvas.on_draw(|x, y, _button| {
        on_mouse_move(x, y);
    });
    app().canvas.on_mouse_up(|x, y, _button| {
        on_mouse_up(x, y);
    });

    // Tool buttons
    for i in 0..8u32 {
        app().tool_buttons[i as usize].on_click_raw(tool_click_handler, i as u64);
    }

    // File operations
    btn_new.on_click_raw(new_click_handler, 0);
    btn_open.on_click_raw(open_click_handler, 0);
    btn_save.on_click_raw(save_click_handler, 0);

    // Brush size
    btn_size_down.on_click_raw(size_down_handler, 0);
    btn_size_up.on_click_raw(size_up_handler, 0);

    ui::run();
    ui::shutdown();
}

// ── Callback handlers (extern "C") ───────────────────────────────────

extern "C" fn tool_click_handler(_id: u32, _ev: u32, userdata: u64) {
    let idx = userdata as usize;
    let tool = match idx {
        0 => Tool::Pencil,
        1 => Tool::Brush,
        2 => Tool::Eraser,
        3 => Tool::Line,
        4 => Tool::Rect,
        5 => Tool::Ellipse,
        6 => Tool::Fill,
        7 => Tool::Picker,
        _ => return,
    };
    app().tool = tool;
    app().status_tool.set_text(TOOL_NAMES[idx]);
    highlight_active_tool(idx);
}

extern "C" fn palette_click_handler(_id: u32, _ev: u32, userdata: u64) {
    let color = userdata as u32;
    app().fg_color = color;
    app().fg_swatch.set_color(color);
}

extern "C" fn new_click_handler(_id: u32, _ev: u32, _userdata: u64) {
    app().canvas.clear(0xFFFFFFFF);
}

extern "C" fn open_click_handler(_id: u32, _ev: u32, _userdata: u64) {
    open_file();
}

extern "C" fn save_click_handler(_id: u32, _ev: u32, _userdata: u64) {
    save_file();
}

extern "C" fn size_down_handler(_id: u32, _ev: u32, _userdata: u64) {
    let a = app();
    if a.brush_size > 1 {
        a.brush_size -= 1;
    }
    update_size_label();
}

extern "C" fn size_up_handler(_id: u32, _ev: u32, _userdata: u64) {
    let a = app();
    if a.brush_size < 20 {
        a.brush_size += 1;
    }
    update_size_label();
}

// ── UI helpers ───────────────────────────────────────────────────────

fn highlight_active_tool(active_idx: usize) {
    let a = app();
    let tc = ui::theme::colors();
    for (i, btn) in a.tool_buttons.iter().enumerate() {
        if i == active_idx {
            btn.set_color(tc.accent);
        } else {
            btn.set_color(tc.control_bg);
        }
    }
}

fn update_size_label() {
    let a = app();
    let mut buf = [0u8; 16];
    let len = fmt_size(a.brush_size, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf[..len]) {
        a.size_label.set_text(s);
    }
}

fn fmt_size(size: u32, buf: &mut [u8]) -> usize {
    let prefix = b"Size: ";
    let plen = prefix.len();
    buf[..plen].copy_from_slice(prefix);
    let mut n = size;
    if n == 0 {
        buf[plen] = b'0';
        return plen + 1;
    }
    let mut digits = [0u8; 5];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[plen + j] = digits[i - 1 - j];
    }
    plen + i
}

fn update_status_pos(x: i32, y: i32) {
    let mut buf = [0u8; 32];
    let len = fmt_pos(x, y, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf[..len]) {
        app().status_pos.set_text(s);
    }
}

fn fmt_pos(x: i32, y: i32, buf: &mut [u8]) -> usize {
    let mut off = 0;
    off += fmt_i32(x, &mut buf[off..]);
    buf[off] = b',';
    buf[off + 1] = b' ';
    off += 2;
    off += fmt_i32(y, &mut buf[off..]);
    off
}

fn fmt_i32(val: i32, buf: &mut [u8]) -> usize {
    if val < 0 {
        buf[0] = b'-';
        return 1 + fmt_u32((-val) as u32, &mut buf[1..]);
    }
    fmt_u32(val as u32, buf)
}

fn fmt_u32(mut val: u32, buf: &mut [u8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 10];
    let mut i = 0;
    while val > 0 {
        digits[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    i
}

// ── Drawing logic ────────────────────────────────────────────────────

fn on_mouse_down(x: i32, y: i32, button: u32) {
    let a = app();
    a.is_drawing = true;
    a.start_x = x;
    a.start_y = y;
    a.last_x = x;
    a.last_y = y;

    // Right-click picks BG color for palette
    let color = if button == 2 { a.bg_color } else { a.fg_color };

    match a.tool {
        Tool::Pencil => {
            a.canvas.set_pixel(x, y, color);
        }
        Tool::Brush => {
            let sz = a.brush_size;
            a.canvas.fill_circle(x, y, (sz / 2) as i32, color);
        }
        Tool::Eraser => {
            let sz = a.brush_size;
            let bg = a.bg_color;
            a.canvas.fill_circle(x, y, (sz / 2) as i32, bg);
        }
        Tool::Line | Tool::Rect | Tool::Ellipse => {
            // Snapshot current pixels for preview
            let total = (CANVAS_W * CANVAS_H) as usize;
            a.snapshot.resize(total, 0);
            a.canvas.copy_pixels_to(&mut a.snapshot);
        }
        Tool::Fill => {
            a.canvas.flood_fill(x, y, color);
        }
        Tool::Picker => {
            let picked = a.canvas.get_pixel(x, y);
            if picked != 0 {
                a.fg_color = picked;
                a.fg_swatch.set_color(picked);
            }
        }
    }

    update_status_pos(x, y);
}

fn on_mouse_move(x: i32, y: i32) {
    let a = app();
    if !a.is_drawing { return; }

    let color = a.fg_color;

    match a.tool {
        Tool::Pencil => {
            a.canvas.draw_line(a.last_x, a.last_y, x, y, color);
        }
        Tool::Brush => {
            let sz = a.brush_size;
            a.canvas.draw_thick_line(a.last_x, a.last_y, x, y, color, sz);
        }
        Tool::Eraser => {
            let sz = a.brush_size;
            let bg = a.bg_color;
            a.canvas.draw_thick_line(a.last_x, a.last_y, x, y, bg, sz);
        }
        Tool::Line => {
            // Restore snapshot, then draw preview line
            a.canvas.copy_pixels_from(&a.snapshot);
            a.canvas.draw_line(a.start_x, a.start_y, x, y, color);
        }
        Tool::Rect => {
            a.canvas.copy_pixels_from(&a.snapshot);
            let (rx, ry, rw, rh) = rect_from_points(a.start_x, a.start_y, x, y);
            a.canvas.draw_rect(rx, ry, rw, rh, color, 1);
        }
        Tool::Ellipse => {
            a.canvas.copy_pixels_from(&a.snapshot);
            let (rx, ry, rw, rh) = rect_from_points(a.start_x, a.start_y, x, y);
            let cx = rx + (rw as i32) / 2;
            let cy = ry + (rh as i32) / 2;
            let rrx = (rw / 2) as i32;
            let rry = (rh / 2) as i32;
            a.canvas.draw_ellipse(cx, cy, rrx, rry, color);
        }
        _ => {}
    }

    a.last_x = x;
    a.last_y = y;
    update_status_pos(x, y);
}

fn on_mouse_up(x: i32, y: i32) {
    let a = app();
    if !a.is_drawing { return; }
    a.is_drawing = false;

    let color = a.fg_color;

    match a.tool {
        Tool::Line => {
            // Restore and draw final line
            a.canvas.copy_pixels_from(&a.snapshot);
            a.canvas.draw_line(a.start_x, a.start_y, x, y, color);
            a.snapshot.clear();
        }
        Tool::Rect => {
            a.canvas.copy_pixels_from(&a.snapshot);
            let (rx, ry, rw, rh) = rect_from_points(a.start_x, a.start_y, x, y);
            a.canvas.draw_rect(rx, ry, rw, rh, color, 1);
            a.snapshot.clear();
        }
        Tool::Ellipse => {
            a.canvas.copy_pixels_from(&a.snapshot);
            let (rx, ry, rw, rh) = rect_from_points(a.start_x, a.start_y, x, y);
            let cx = rx + (rw as i32) / 2;
            let cy = ry + (rh as i32) / 2;
            let rrx = (rw / 2) as i32;
            let rry = (rh / 2) as i32;
            a.canvas.draw_ellipse(cx, cy, rrx, rry, color);
            a.snapshot.clear();
        }
        _ => {}
    }

    update_status_pos(x, y);
}

/// Convert two corner points to (x, y, w, h) rectangle.
fn rect_from_points(x0: i32, y0: i32, x1: i32, y1: i32) -> (i32, i32, u32, u32) {
    let rx = x0.min(x1);
    let ry = y0.min(y1);
    let rw = (x1 - x0).unsigned_abs();
    let rh = (y1 - y0).unsigned_abs();
    (rx, ry, rw.max(1), rh.max(1))
}

// ── File operations ──────────────────────────────────────────────────

fn open_file() {
    let path = match ui::FileDialog::open_file() {
        Some(p) => p,
        None => return,
    };

    let data = match anyos_std::fs::read_to_vec(&path) {
        Ok(d) => d,
        Err(_) => return,
    };

    // Probe image
    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => return,
    };

    // Decode
    let pixel_count = (info.width as usize) * (info.height as usize);
    let mut pixels = vec![0u32; pixel_count];
    let scratch_size = info.scratch_needed as usize;
    let mut scratch = vec![0u8; if scratch_size > 0 { scratch_size } else { 1 }];

    if libimage_client::decode(&data, &mut pixels, &mut scratch).is_err() {
        return;
    }

    // If dimensions match canvas, load directly; otherwise scale to fit
    if info.width == CANVAS_W && info.height == CANVAS_H {
        app().canvas.copy_pixels_from(&pixels);
    } else {
        // Scale to canvas dimensions
        let mut scaled = vec![0u32; (CANVAS_W * CANVAS_H) as usize];
        if libimage_client::scale_image(
            &pixels, info.width, info.height,
            &mut scaled, CANVAS_W, CANVAS_H,
            libimage_client::MODE_CONTAIN,
        ) {
            // Fill with white first, then overlay
            app().canvas.clear(0xFFFFFFFF);
            app().canvas.copy_pixels_from(&scaled);
        } else {
            // Fallback: just copy what fits
            app().canvas.copy_pixels_from(&pixels);
        }
    }
}

fn save_file() {
    let path = match ui::FileDialog::save_file("image.bmp") {
        Some(p) => p,
        None => return,
    };

    // Read pixels from canvas
    let total = (CANVAS_W * CANVAS_H) as usize;
    let mut pixels = vec![0u32; total];
    app().canvas.copy_pixels_to(&mut pixels);

    // Encode to BMP
    let bmp_size = 54 + total * 4;
    let mut bmp_buf = vec![0u8; bmp_size];
    match libimage_client::encode_bmp(&pixels, CANVAS_W, CANVAS_H, &mut bmp_buf) {
        Ok(written) => {
            let _ = anyos_std::fs::write_bytes(&path, &bmp_buf[..written]);
        }
        Err(_) => {}
    }
}
