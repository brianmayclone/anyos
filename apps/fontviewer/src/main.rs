#![no_std]
#![no_main]

use anyos_std::{String, format};
use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

// ---- Colors ----
const BG: u32 = 0xFF1E1E1E;
const TEXT: u32 = 0xFFE0E0E0;
const TEXT_DIM: u32 = 0xFF808080;
const DIVIDER: u32 = 0xFF3A3A3C;
const CELL_BG: u32 = 0xFF2A2A2C;

// ---- Sample text ----
const SAMPLE: &str = "The quick brown fox jumps over the lazy dog";
const SAMPLE_UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const SAMPLE_LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const SAMPLE_DIGITS: &str = "0123456789";
const SAMPLE_SYMBOLS: &str = "!@#$%^&*()-=[]{}|;':\",./<>?";

// ---- Preview sizes ----
const SIZES: [u16; 5] = [48, 36, 24, 18, 13];

// ---- State ----
struct AppState {
    font_id: u32,
    font_path: String,
    font_name: String,
    scroll_y: i32,
    canvas: ui::Canvas,
    win: ui::Window,
    needs_redraw: bool,
}

anyos_std::global_app_state!(AppState);

fn basename(path: &str) -> &str {
    anyos_std::path::basename(path)
}

fn render() {
    let a = app();
    let canvas = &a.canvas;
    let font_id = a.font_id;
    let scroll_y = a.scroll_y;

    // Get canvas dimensions from the canvas height (which updates on dock resize)
    let canvas_h = canvas.get_height();
    let ww: i32 = 640; // We'll use a reasonable default; canvas width is set at creation
    let wh = canvas_h as i32;

    canvas.clear(BG);

    let mut y: i32 = 12 - scroll_y;

    // Font name header
    let name = if a.font_name.is_empty() { "System Font (SF Pro)" } else { &a.font_name };
    canvas.draw_text(16, y, TEXT, 1, 18, name); // font_id 1 = bold
    y += 28;

    if !a.font_path.is_empty() {
        canvas.draw_text(16, y, TEXT_DIM, 0, 13, &a.font_path);
        y += 20;
    }

    // Divider
    canvas.fill_rect(16, y, (ww - 32) as u32, 1, DIVIDER);
    y += 12;

    // Sample text at different sizes
    for &sz in &SIZES {
        let label = format!("{}px", sz);
        canvas.draw_text(16, y, TEXT_DIM, 0, 11, &label);
        y += 14;

        if y > -60 && y < wh + 60 {
            canvas.draw_text(16, y, TEXT, font_id, sz, SAMPLE);
        }
        let (_, th) = ui::measure_text(SAMPLE, font_id as u16, sz);
        y += th as i32 + 16;
    }

    // Divider
    canvas.fill_rect(16, y, (ww - 32) as u32, 1, DIVIDER);
    y += 16;

    // Character sets
    canvas.draw_text(16, y, TEXT_DIM, 0, 13, "Uppercase");
    y += 18;
    if y > -40 && y < wh + 40 {
        canvas.draw_text(16, y, TEXT, font_id, 24, SAMPLE_UPPER);
    }
    y += 34;

    canvas.draw_text(16, y, TEXT_DIM, 0, 13, "Lowercase");
    y += 18;
    if y > -40 && y < wh + 40 {
        canvas.draw_text(16, y, TEXT, font_id, 24, SAMPLE_LOWER);
    }
    y += 34;

    canvas.draw_text(16, y, TEXT_DIM, 0, 13, "Digits");
    y += 18;
    if y > -40 && y < wh + 40 {
        canvas.draw_text(16, y, TEXT, font_id, 24, SAMPLE_DIGITS);
    }
    y += 34;

    canvas.draw_text(16, y, TEXT_DIM, 0, 13, "Symbols");
    y += 18;
    if y > -40 && y < wh + 40 {
        canvas.draw_text(16, y, TEXT, font_id, 24, SAMPLE_SYMBOLS);
    }
    y += 34;

    // Divider
    canvas.fill_rect(16, y, (ww - 32) as u32, 1, DIVIDER);
    y += 16;

    // Character grid (ASCII 33-126)
    canvas.draw_text(16, y, TEXT_DIM, 0, 13, "Character Grid");
    y += 20;

    let cell_sz: i32 = 40;
    let cols = ((ww - 32) / (cell_sz + 4)).max(1);
    let mut col: i32 = 0;
    for ch in 33u8..=126 {
        let cx = 16 + col * (cell_sz + 4);
        let cy = y;

        if cy > -cell_sz && cy < wh + cell_sz {
            canvas.fill_rect(cx, cy, cell_sz as u32, cell_sz as u32, CELL_BG);

            let mut buf = [0u8; 4];
            let s = (ch as char).encode_utf8(&mut buf);
            let (tw, th) = ui::measure_text(s, font_id as u16, 20);
            let tx = cx + (cell_sz - tw as i32) / 2;
            let ty = cy + (cell_sz - th as i32) / 2;
            canvas.draw_text(tx, ty, TEXT, font_id, 20, s);
        }

        col += 1;
        if col >= cols {
            col = 0;
            y += cell_sz + 4;
        }
    }
}

fn open_font() {
    if let Some(path) = ui::FileDialog::open_file() {
        if let Some(fid) = libfont_client::load(&path) {
            let a = app();
            a.font_id = fid;
            a.font_name = String::from(basename(&path));
            a.scroll_y = 0;
            a.win.set_title(&format!("Font Viewer - {}", &a.font_name));
            a.font_path = path;
            a.needs_redraw = true;
        }
    }
}

fn main() {
    if !ui::init() { return; }

    let win = ui::Window::new("Font Viewer", -1, -1, 640, 500);
    win.set_color(BG);

    // Menu
    let mut mb = ui::MenuBarBuilder::new()
        .menu("Font Viewer")
            .item(100, "About Font Viewer", 0)
            .separator()
            .item(199, "Quit", 0)
        .end_menu()
        .menu("File")
            .item(200, "Open Font...", 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = ui::MenuBar::set(win.id(), menu_data);
    menu.on_item(|e| {
        match e.item_id {
            199 => ui::quit(),
            200 => open_font(),
            _ => {}
        }
    });

    // Canvas (DOCK_FILL)
    let canvas = ui::Canvas::new(640, 500);
    canvas.set_dock(ui::DOCK_FILL);
    win.add(&canvas);

    // Check if a font path was passed as argument
    let mut font_id: u32 = 0;
    let mut font_path = String::new();
    let mut font_name = String::new();

    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    if !args.is_empty() {
        let path = args.trim();
        if !path.is_empty() {
            if let Some(fid) = libfont_client::load(path) {
                font_id = fid;
                font_path = String::from(path);
                font_name = String::from(basename(path));
                win.set_title(&format!("Font Viewer - {}", basename(path)));
            }
        }
    }

    // Initialize global state
    unsafe {
        APP = Some(AppState {
            font_id,
            font_path,
            font_name,
            scroll_y: 0,
            canvas,
            win,
            needs_redraw: true,
        });
    }

    // Initial render
    render();

    // Keyboard events for scrolling
    app().win.on_key_down(|ke| {
        match ke.keycode {
            ui::KEY_UP => {
                app().scroll_y = (app().scroll_y - 40).max(0);
                app().needs_redraw = true;
            }
            ui::KEY_DOWN => {
                app().scroll_y += 40;
                app().needs_redraw = true;
            }
            ui::KEY_PAGE_UP => {
                app().scroll_y = (app().scroll_y - 200).max(0);
                app().needs_redraw = true;
            }
            ui::KEY_PAGE_DOWN => {
                app().scroll_y += 200;
                app().needs_redraw = true;
            }
            ui::KEY_HOME => {
                app().scroll_y = 0;
                app().needs_redraw = true;
            }
            _ => {}
        }
    });

    // Resize handler
    app().win.on_resize(|_| {
        app().needs_redraw = true;
    });

    // Redraw timer (check if needs_redraw, render if so)
    ui::set_timer(32, || {
        if app().needs_redraw {
            app().needs_redraw = false;
            render();
        }
    });

    app().win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}
