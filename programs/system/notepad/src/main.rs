#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::ui::window;
use uisys_client::*;

anyos_std::entry!(main);

// Layout constants
const NAVBAR_H: i32 = 44;
const GUTTER_W: i32 = 48;
const SCROLLBAR_W: u32 = 8;
const LINE_H: i32 = 16;
const CHAR_W: i32 = 8;
const PADDING_X: i32 = 6;

// Colors
const BG: u32 = 0xFF1E1E1E;
const GUTTER_BG: u32 = 0xFF252525;
const GUTTER_SEP: u32 = 0xFF3D3D3D;
const LINE_NUM_COLOR: u32 = 0xFF606060;
const TEXT_COLOR: u32 = 0xFFE6E6E6;

fn main() {
    // Get file path from arguments
    let mut args_buf = [0u8; 256];
    let args_len = anyos_std::process::getargs(&mut args_buf);
    let path = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("").trim();

    if path.is_empty() {
        anyos_std::println!("notepad: no file specified");
        return;
    }

    // Read file content
    let content = match read_file(path) {
        Some(data) => data,
        None => {
            anyos_std::println!("notepad: cannot open '{}'", path);
            return;
        }
    };

    // Parse into lines
    let text = core::str::from_utf8(&content).unwrap_or("<binary file>");
    let lines: Vec<&str> = text.split('\n').collect();
    let line_count = lines.len();

    // Extract filename for title
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut title = String::from(filename);
    title.push_str(" - Notepad");

    // Create window
    let win = window::create_ex(&title, 100, 60, 600, 400, 0);
    if win == u32::MAX {
        anyos_std::println!("notepad: failed to create window");
        return;
    }

    let (mut win_w, mut win_h) = window::get_size(win).unwrap_or((600, 400));

    let content_h = (line_count as u32) * (LINE_H as u32);
    let text_area_h = (win_h as i32 - NAVBAR_H).max(0) as u32;

    let nav = UiNavbar::new(0, 0, win_w, false);
    let mut sb = UiScrollbar::new(
        win_w as i32 - SCROLLBAR_W as i32,
        NAVBAR_H,
        SCROLLBAR_W,
        text_area_h,
        content_h,
    );

    let mut needs_redraw = true;

    loop {
        // Poll events
        let mut event_raw = [0u32; 5];
        while window::get_event(win, &mut event_raw) != 0 {
            let ev = UiEvent::from_raw(&event_raw);

            match ev.event_type {
                EVENT_RESIZE => {
                    let new_w = ev.p1;
                    let new_h = ev.p2;
                    if new_w != win_w || new_h != win_h {
                        win_w = new_w;
                        win_h = new_h;
                        let new_text_h = (win_h as i32 - NAVBAR_H).max(0) as u32;
                        sb.x = win_w as i32 - SCROLLBAR_W as i32;
                        sb.h = new_text_h;
                        sb.content_h = content_h;
                        if sb.scroll > sb.max_scroll() {
                            sb.scroll = sb.max_scroll();
                        }
                        needs_redraw = true;
                    }
                }
                EVENT_KEY_DOWN => {
                    let key = ev.key_code();
                    let page_lines = ((win_h as i32 - NAVBAR_H) / LINE_H).max(1) as u32;
                    match key {
                        KEY_UP => {
                            if sb.scroll >= LINE_H as u32 {
                                sb.scroll -= LINE_H as u32;
                                needs_redraw = true;
                            } else if sb.scroll > 0 {
                                sb.scroll = 0;
                                needs_redraw = true;
                            }
                        }
                        KEY_DOWN => {
                            let new = sb.scroll + LINE_H as u32;
                            if new <= sb.max_scroll() {
                                sb.scroll = new;
                                needs_redraw = true;
                            } else if sb.scroll < sb.max_scroll() {
                                sb.scroll = sb.max_scroll();
                                needs_redraw = true;
                            }
                        }
                        KEY_HOME => {
                            if sb.scroll != 0 {
                                sb.scroll = 0;
                                needs_redraw = true;
                            }
                        }
                        KEY_END => {
                            let max = sb.max_scroll();
                            if sb.scroll != max {
                                sb.scroll = max;
                                needs_redraw = true;
                            }
                        }
                        // Page Up / Page Down via char values (space = page down in many viewers)
                        _ => {
                            let ch = ev.char_val();
                            // Page Up (we use char 0 with specific key; or just treat unknown keys)
                            if key == 0x10B {
                                // PAGE_UP
                                let step = page_lines * LINE_H as u32;
                                sb.scroll = sb.scroll.saturating_sub(step);
                                needs_redraw = true;
                            } else if key == 0x10C {
                                // PAGE_DOWN
                                let step = page_lines * LINE_H as u32;
                                sb.scroll = (sb.scroll + step).min(sb.max_scroll());
                                needs_redraw = true;
                            } else if ch == b' ' as u32 {
                                // Space = page down
                                let step = page_lines * LINE_H as u32;
                                sb.scroll = (sb.scroll + step).min(sb.max_scroll());
                                needs_redraw = true;
                            }
                        }
                    }
                }
                EVENT_MOUSE_DOWN | EVENT_MOUSE_UP | EVENT_MOUSE_MOVE => {
                    if sb.handle_event(&ev).is_some() {
                        needs_redraw = true;
                    }
                }
                _ => {}
            }
        }

        if needs_redraw {
            render(win, win_w, win_h, &nav, &sb, &lines, filename);
            needs_redraw = false;
        }

        anyos_std::process::yield_cpu();
    }
}

fn render(
    win: u32,
    win_w: u32,
    win_h: u32,
    nav: &UiNavbar,
    sb: &UiScrollbar,
    lines: &[&str],
    filename: &str,
) {
    // Clear background
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, BG);

    // Navbar
    nav.render(win, filename);

    let text_area_h = (win_h as i32 - NAVBAR_H).max(0);
    let visible_lines = (text_area_h / LINE_H) + 1;
    let first_line = (sb.scroll as i32 / LINE_H) as usize;
    let pixel_offset = sb.scroll as i32 % LINE_H;

    // Gutter background
    window::fill_rect(win, 0, NAVBAR_H as i16, GUTTER_W as u16, text_area_h as u16, GUTTER_BG);
    // Gutter separator
    window::fill_rect(win, (GUTTER_W - 1) as i16, NAVBAR_H as i16, 1, text_area_h as u16, GUTTER_SEP);

    // Draw visible lines
    let mut num_buf = [0u8; 8];
    for i in 0..visible_lines as usize {
        let line_idx = first_line + i;
        if line_idx >= lines.len() {
            break;
        }

        let y = NAVBAR_H + (i as i32 * LINE_H) - pixel_offset;
        if y + LINE_H <= NAVBAR_H || y >= win_h as i32 {
            continue;
        }

        // Line number (right-aligned in gutter)
        let num = line_idx + 1;
        let num_str = format_num(num, &mut num_buf);
        let num_w = num_str.len() as i32 * CHAR_W;
        let num_x = GUTTER_W - PADDING_X - num_w;
        window::draw_text_mono(win, num_x as i16, y as i16, LINE_NUM_COLOR, num_str);

        // Line text (clipped to window width)
        let text_x = GUTTER_W + PADDING_X;
        let max_chars = ((win_w as i32 - text_x - SCROLLBAR_W as i32) / CHAR_W).max(0) as usize;
        let line = lines[line_idx];
        if !line.is_empty() && max_chars > 0 {
            let display = if line.len() > max_chars { &line[..max_chars] } else { line };
            window::draw_text_mono(win, text_x as i16, y as i16, TEXT_COLOR, display);
        }
    }

    // Scrollbar
    if sb.content_h > sb.h {
        sb.render(win);
    }

    window::present(win);
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut content = Vec::new();
    let mut buf = [0u8; 512];
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

fn format_num(mut n: usize, buf: &mut [u8; 8]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut pos = 8;
    while n > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[pos..]) }
}
