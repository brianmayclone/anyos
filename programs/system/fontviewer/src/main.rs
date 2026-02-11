#![no_std]
#![no_main]

use anyos_std::ui::window;
use anyos_std::ui::filedialog;
use anyos_std::String;
use anyos_std::format;

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
struct State {
    font_id: u16,
    font_path: String,
    font_name: String,
    scroll_y: i32,
}

fn basename(path: &str) -> &str {
    if let Some(pos) = path.rfind('/') {
        &path[pos + 1..]
    } else {
        path
    }
}

fn render(win: u32, st: &State) {
    let (w, h) = match window::get_size(win) {
        Some(s) => s,
        None => return,
    };
    let ww = w as i16;
    let wh = h as i16;

    // Background
    window::fill_rect(win, 0, 0, w as u16, h as u16, BG);

    let mut y = 12 - st.scroll_y as i16;

    // Font name header
    let name = if st.font_name.is_empty() { "System Font (SF Pro)" } else { &st.font_name };
    window::draw_text_ex(win, 16, y, TEXT, window::FONT_BOLD, 18, name);
    y += 28;

    if !st.font_path.is_empty() {
        window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, &st.font_path);
        y += 20;
    }

    // Divider
    window::fill_rect(win, 16, y, (ww - 32) as u16, 1, DIVIDER);
    y += 12;

    // Sample text at different sizes
    for &sz in &SIZES {
        let label = format!("{}px", sz);
        window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 11, &label);
        y += 14;

        // Clip: only render if visible
        if y > -60 && y < wh + 60 {
            window::draw_text_ex(win, 16, y, TEXT, st.font_id, sz, SAMPLE);
        }
        let (_, th) = window::font_measure(st.font_id, sz, SAMPLE);
        y += th as i16 + 16;
    }

    // Divider
    window::fill_rect(win, 16, y, (ww - 32) as u16, 1, DIVIDER);
    y += 16;

    // Character sets
    window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, "Uppercase");
    y += 18;
    if y > -40 && y < wh + 40 {
        window::draw_text_ex(win, 16, y, TEXT, st.font_id, 24, SAMPLE_UPPER);
    }
    y += 34;

    window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, "Lowercase");
    y += 18;
    if y > -40 && y < wh + 40 {
        window::draw_text_ex(win, 16, y, TEXT, st.font_id, 24, SAMPLE_LOWER);
    }
    y += 34;

    window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, "Digits");
    y += 18;
    if y > -40 && y < wh + 40 {
        window::draw_text_ex(win, 16, y, TEXT, st.font_id, 24, SAMPLE_DIGITS);
    }
    y += 34;

    window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, "Symbols");
    y += 18;
    if y > -40 && y < wh + 40 {
        window::draw_text_ex(win, 16, y, TEXT, st.font_id, 24, SAMPLE_SYMBOLS);
    }
    y += 34;

    // Divider
    window::fill_rect(win, 16, y, (ww - 32) as u16, 1, DIVIDER);
    y += 16;

    // Character grid (ASCII 33-126)
    window::draw_text_ex(win, 16, y, TEXT_DIM, 0, 13, "Character Grid");
    y += 20;

    let cell_sz: i16 = 40;
    let cols = ((ww - 32) / (cell_sz + 4)).max(1);
    let mut col = 0i16;
    for ch in 33u8..=126 {
        let cx = 16 + col * (cell_sz + 4);
        let cy = y;

        if cy > -cell_sz && cy < wh + cell_sz {
            // Cell background
            window::fill_rounded_rect(win, cx, cy, cell_sz as u16, cell_sz as u16, 4, CELL_BG);

            // Character
            let mut buf = [0u8; 4];
            let s = (ch as char).encode_utf8(&mut buf);
            let (tw, th) = window::font_measure(st.font_id, 20, s);
            let tx = cx + (cell_sz - tw as i16) / 2;
            let ty = cy + (cell_sz - th as i16) / 2;
            window::draw_text_ex(win, tx, ty, TEXT, st.font_id, 20, s);
        }

        col += 1;
        if col >= cols {
            col = 0;
            y += cell_sz + 4;
        }
    }
    if col > 0 {
        y += cell_sz + 4;
    }
}

fn main() {
    let win = window::create("Font Viewer", 100, 60, 640, 500);
    if win == u32::MAX { return; }

    // Menu
    let mut mb = window::MenuBarBuilder::new()
        .menu("Font Viewer")
            .item(100, "About Font Viewer", 0)
            .separator()
            .item(199, "Quit", 0)
        .end_menu()
        .menu("File")
            .item(200, "Open Font...", 0)
        .end_menu();
    window::set_menu(win, mb.build());

    let mut st = State {
        font_id: 0,
        font_path: String::new(),
        font_name: String::new(),
        scroll_y: 0,
    };

    // Check if a font path was passed as argument
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    if !args.is_empty() {
        let path = args.trim();
        if !path.is_empty() {
            if let Some(fid) = window::font_load(path) {
                st.font_id = fid as u16;
                st.font_path = String::from(path);
                st.font_name = String::from(basename(path));
                window::set_title(win, &format!("Font Viewer - {}", basename(path)));
            }
        }
    }

    let mut event = [0u32; 5];
    let mut dirty = true;

    loop {
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_RESIZE => { dirty = true; }
                window::EVENT_MOUSE_SCROLL => {
                    let dy = event[1] as i32;
                    st.scroll_y = (st.scroll_y - dy * 30).max(0);
                    dirty = true;
                }
                window::EVENT_KEY_DOWN => {
                    match event[1] {
                        0x26 => { // Up
                            st.scroll_y = (st.scroll_y - 40).max(0);
                            dirty = true;
                        }
                        0x28 => { // Down
                            st.scroll_y += 40;
                            dirty = true;
                        }
                        _ => {}
                    }
                }
                window::EVENT_MENU_ITEM => {
                    match event[1] {
                        199 | 0xFFF2 => {
                            window::destroy(win);
                            return;
                        }
                        200 => {
                            // Open font file
                            if let filedialog::FileDialogResult::Selected(path) = filedialog::open_file("/system/fonts") {
                                if let Some(fid) = window::font_load(&path) {
                                    st.font_id = fid as u16;
                                    st.font_name = String::from(basename(&path));
                                    st.font_path = path;
                                    st.scroll_y = 0;
                                    window::set_title(win, &format!("Font Viewer - {}", &st.font_name));
                                    dirty = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                window::EVENT_WINDOW_CLOSE => {
                    window::destroy(win);
                    return;
                }
                _ => {}
            }
        }

        if dirty {
            render(win, &st);
            window::present(win);
            dirty = false;
        }
        anyos_std::process::sleep(16);
    }
}
