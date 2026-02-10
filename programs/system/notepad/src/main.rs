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
const EVENT_MOUSE_SCROLL: u32 = 7;

// Colors
const BG: u32 = 0xFF1E1E1E;
const GUTTER_BG: u32 = 0xFF252525;
const GUTTER_SEP: u32 = 0xFF3D3D3D;
const LINE_NUM_COLOR: u32 = 0xFF606060;
const TEXT_COLOR: u32 = 0xFFE6E6E6;
const CURSOR_COLOR: u32 = 0xFFD4D4D4;

struct Editor {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    modified: bool,
    file_path: String,
}

impl Editor {
    fn new(content: &str, path: &str) -> Self {
        let mut lines: Vec<String> = content.split('\n').map(String::from).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Editor {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
            file_path: String::from(path),
        }
    }

    fn new_empty(path: &str) -> Self {
        let mut lines = Vec::new();
        lines.push(String::new());
        Editor {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
            file_path: String::from(path),
        }
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn insert_char(&mut self, ch: u8) {
        let line = &mut self.lines[self.cursor_line];
        if self.cursor_col >= line.len() {
            line.push(ch as char);
        } else {
            line.insert(self.cursor_col, ch as char);
        }
        self.cursor_col += 1;
        self.modified = true;
    }

    fn insert_newline(&mut self) {
        let line = &mut self.lines[self.cursor_line];
        let rest = String::from(&line[self.cursor_col..]);
        line.truncate(self.cursor_col);
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, rest);
        self.cursor_col = 0;
        self.modified = true;
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_line];
            line.remove(self.cursor_col - 1);
            self.cursor_col -= 1;
            self.modified = true;
        } else if self.cursor_line > 0 {
            // Merge with previous line
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
            self.modified = true;
        }
    }

    fn delete(&mut self) {
        let line_len = self.lines[self.cursor_line].len();
        if self.cursor_col < line_len {
            self.lines[self.cursor_line].remove(self.cursor_col);
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            // Merge next line into current
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
        }
    }

    fn insert_tab(&mut self) {
        // Insert 4 spaces
        for _ in 0..4 {
            self.insert_char(b' ');
        }
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
        }
    }

    fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_line].len();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            let line_len = self.lines[self.cursor_line].len();
            if self.cursor_col > line_len {
                self.cursor_col = line_len;
            }
        }
    }

    fn move_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            let line_len = self.lines[self.cursor_line].len();
            if self.cursor_col > line_len {
                self.cursor_col = line_len;
            }
        }
    }

    fn move_home(&mut self) {
        self.cursor_col = 0;
    }

    fn move_end(&mut self) {
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    fn save(&mut self) -> bool {
        use anyos_std::fs;
        // Truncate then write
        fs::truncate(&self.file_path);

        let fd = fs::open(&self.file_path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            return false;
        }

        for (i, line) in self.lines.iter().enumerate() {
            if !line.is_empty() {
                fs::write(fd, line.as_bytes());
            }
            if i + 1 < self.lines.len() {
                fs::write(fd, b"\n");
            }
        }
        fs::close(fd);
        self.modified = false;
        true
    }

}

fn main() {
    // Get file path from arguments
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("notepad: no file specified");
        return;
    }

    // Read file content or start empty for new files
    let mut editor = match read_file(path) {
        Some(data) => {
            let text = core::str::from_utf8(&data).unwrap_or("");
            Editor::new(text, path)
        }
        None => {
            // New file
            Editor::new_empty(path)
        }
    };

    // Extract filename for title
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Create window
    let win = window::create_ex(&make_title(filename, editor.modified), 100, 60, 600, 400, 0);
    if win == u32::MAX {
        anyos_std::println!("notepad: failed to create window");
        return;
    }

    // Set up menu bar
    let mut mb = window::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Save", 0)
            .separator()
            .item(2, "Close", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win, data);

    let (mut win_w, mut win_h) = window::get_size(win).unwrap_or((600, 400));

    let content_h = (editor.line_count() as u32) * (LINE_H as u32);
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
    let mut was_modified = false;

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
                        update_scrollbar(&mut sb, &editor);
                        needs_redraw = true;
                    }
                }
                EVENT_KEY_DOWN => {
                    let key = ev.key_code();
                    let ch = ev.char_val();

                    match key {
                        KEY_UP => {
                            editor.move_up();
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_DOWN => {
                            editor.move_down();
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_LEFT => {
                            editor.move_left();
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_RIGHT => {
                            editor.move_right();
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_HOME => {
                            editor.move_home();
                            needs_redraw = true;
                        }
                        KEY_END => {
                            editor.move_end();
                            needs_redraw = true;
                        }
                        KEY_BACKSPACE => {
                            editor.backspace();
                            update_scrollbar(&mut sb, &editor);
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_DELETE => {
                            editor.delete();
                            update_scrollbar(&mut sb, &editor);
                            needs_redraw = true;
                        }
                        KEY_ENTER => {
                            editor.insert_newline();
                            update_scrollbar(&mut sb, &editor);
                            ensure_cursor_visible(&mut sb, &editor, win_h);
                            needs_redraw = true;
                        }
                        KEY_TAB => {
                            editor.insert_tab();
                            needs_redraw = true;
                        }
                        KEY_ESCAPE => {
                            window::destroy(win);
                            return;
                        }
                        _ => {
                            // Check for Ctrl+S (char 19 = 0x13)
                            if ch == 0x13 {
                                editor.save();
                                needs_redraw = true;
                            } else if key == KEY_PAGE_UP {
                                // Page Up
                                let page_lines = ((win_h as i32 - NAVBAR_H) / LINE_H).max(1) as usize;
                                for _ in 0..page_lines {
                                    editor.move_up();
                                }
                                ensure_cursor_visible(&mut sb, &editor, win_h);
                                needs_redraw = true;
                            } else if key == KEY_PAGE_DOWN {
                                // Page Down
                                let page_lines = ((win_h as i32 - NAVBAR_H) / LINE_H).max(1) as usize;
                                for _ in 0..page_lines {
                                    editor.move_down();
                                }
                                ensure_cursor_visible(&mut sb, &editor, win_h);
                                needs_redraw = true;
                            } else if ch >= 0x20 && ch < 0x7F {
                                // Printable ASCII
                                editor.insert_char(ch as u8);
                                ensure_cursor_visible(&mut sb, &editor, win_h);
                                needs_redraw = true;
                            }
                        }
                    }
                }
                EVENT_MOUSE_DOWN | EVENT_MOUSE_UP | EVENT_MOUSE_MOVE => {
                    // Scrollbar interaction
                    if sb.handle_event(&ev).is_some() {
                        needs_redraw = true;
                    } else if ev.event_type == EVENT_MOUSE_DOWN {
                        // Click to place cursor
                        let (mx, my) = ev.mouse_pos();
                        click_to_cursor(&mut editor, &sb, mx, my, win_w);
                        needs_redraw = true;
                    }
                }
                EVENT_MOUSE_SCROLL => {
                    let dz = ev.p1 as i32;
                    let step = (dz.unsigned_abs() as u32) * LINE_H as u32 * 3;
                    if dz < 0 {
                        sb.scroll = sb.scroll.saturating_sub(step);
                    } else {
                        sb.scroll = (sb.scroll + step).min(sb.max_scroll());
                    }
                    needs_redraw = true;
                }
                window::EVENT_MENU_ITEM => {
                    let item_id = ev.p2;
                    match item_id {
                        1 => { editor.save(); needs_redraw = true; } // Save
                        2 => { window::destroy(win); return; }       // Close
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

        // Update title if modified state changed
        if editor.modified != was_modified {
            was_modified = editor.modified;
            window::set_title(win, &make_title(filename, editor.modified));
        }

        if needs_redraw {
            render(win, win_w, win_h, &nav, &sb, &editor, filename);
            needs_redraw = false;
        }

        anyos_std::process::yield_cpu();
    }
}

fn make_title(filename: &str, modified: bool) -> String {
    let mut t = String::new();
    if modified {
        t.push_str("* ");
    }
    t.push_str(filename);
    t.push_str(" - Notepad");
    t
}

fn update_scrollbar(sb: &mut UiScrollbar, editor: &Editor) {
    sb.content_h = (editor.line_count() as u32) * (LINE_H as u32);
    if sb.scroll > sb.max_scroll() {
        sb.scroll = sb.max_scroll();
    }
}

fn ensure_cursor_visible(sb: &mut UiScrollbar, editor: &Editor, win_h: u32) {
    let cursor_y = (editor.cursor_line as u32) * (LINE_H as u32);
    let text_area_h = (win_h as i32 - NAVBAR_H).max(0) as u32;

    // Scroll up if cursor is above visible area
    if cursor_y < sb.scroll {
        sb.scroll = cursor_y;
    }
    // Scroll down if cursor is below visible area
    if cursor_y + LINE_H as u32 > sb.scroll + text_area_h {
        sb.scroll = (cursor_y + LINE_H as u32).saturating_sub(text_area_h);
    }
    if sb.scroll > sb.max_scroll() {
        sb.scroll = sb.max_scroll();
    }
}

fn click_to_cursor(editor: &mut Editor, sb: &UiScrollbar, mx: i32, my: i32, _win_w: u32) {
    let text_x = GUTTER_W + PADDING_X;
    if my < NAVBAR_H || mx < text_x {
        return;
    }

    let pixel_y = (my - NAVBAR_H) as u32 + sb.scroll;
    let line = (pixel_y / LINE_H as u32) as usize;
    let line = line.min(editor.line_count().saturating_sub(1));

    let col_offset = mx - text_x;
    let col = if col_offset > 0 {
        (col_offset / CHAR_W) as usize
    } else {
        0
    };
    let col = col.min(editor.lines[line].len());

    editor.cursor_line = line;
    editor.cursor_col = col;
}

fn render(
    win: u32,
    win_w: u32,
    win_h: u32,
    nav: &UiNavbar,
    sb: &UiScrollbar,
    editor: &Editor,
    filename: &str,
) {
    // Clear background
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, BG);

    // Navbar
    let title = make_title(filename, editor.modified);
    nav.render(win, &title);

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
        if line_idx >= editor.lines.len() {
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
        let line = &editor.lines[line_idx];
        if !line.is_empty() && max_chars > 0 {
            let display = if line.len() > max_chars { &line[..max_chars] } else { line.as_str() };
            window::draw_text_mono(win, text_x as i16, y as i16, TEXT_COLOR, display);
        }

        // Cursor on this line
        if line_idx == editor.cursor_line {
            let cursor_x = text_x + (editor.cursor_col as i32) * CHAR_W;
            // Draw cursor bar
            window::fill_rect(win, cursor_x as i16, y as i16, 2, LINE_H as u16, CURSOR_COLOR);
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
