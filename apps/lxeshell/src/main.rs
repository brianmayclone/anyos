#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{ipc, process};
use libanyui_client as anyui;
use libanyui_client::{
    KeyEvent, KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_ESCAPE, KEY_HOME,
    KEY_F1, KEY_F10, KEY_F11, KEY_F12, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8,
    KEY_F9, KEY_LEFT, KEY_PAGE_DOWN, KEY_PAGE_UP, KEY_RIGHT, KEY_TAB, KEY_UP,
};
use liblxecore::config::LxeConfig;
use liblxecore::readiness::{shell_availability, LxeShellAvailability};

anyos_std::entry!(main);

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const FONT_SIZE: u16 = 14;
const CELL_W: i32 = 8;
const LINE_H: i32 = 18;
const PAD_X: i32 = 10;
const PAD_Y: i32 = 8;
const BG: u32 = 0xFF171821;
const FG: u32 = 0xFFE8E8EA;
const DIM: u32 = 0xFF8B8D98;
const ERROR: u32 = 0xFFFF6B6B;
const CURSOR: u32 = 0xFFE8E8EA;
const SCROLL_TRACK: u32 = 0x332A2D38;
const SCROLL_THUMB: u32 = 0xAA8B8D98;
const MAX_LINES: usize = 2000;
const WHEEL_SCROLL_LINES: usize = 3;

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: u32,
    bg: u32,
    bold: bool,
}

impl Cell {
    fn blank(fg: u32, bg: u32, bold: bool) -> Self {
        Self {
            ch: ' ',
            fg,
            bg,
            bold,
        }
    }
}

struct LxeShellApp {
    win: anyui::Window,
    canvas: anyui::Canvas,
    stdin_pipe: u32,
    stdout_pipe: u32,
    child_tid: u32,
    lines: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    scrollback_lines: usize,
    in_escape: bool,
    escape: Vec<u8>,
    line_drawing: bool,
    app_cursor_keys: bool,
    current_fg: u32,
    current_bg: u32,
    current_bold: bool,
    running: bool,
    dirty: bool,
}

static mut APP: Option<LxeShellApp> = None;

fn app() -> &'static mut LxeShellApp {
    unsafe { APP.as_mut().unwrap() }
}

fn main() {
    if !anyui::init() {
        anyos_std::println!("lxeshell: failed to load libanyui.so");
        return;
    }

    let win = anyui::Window::new("LXE Shell", -1, -1, WIN_W, WIN_H);
    let canvas = anyui::Canvas::new(WIN_W, WIN_H);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.clear(BG);
    win.add(&canvas);

    unsafe {
        APP = Some(LxeShellApp {
            win,
            canvas,
            stdin_pipe: 0,
            stdout_pipe: 0,
            child_tid: 0,
            lines: Vec::new(),
            cursor_row: 0,
            cursor_col: 0,
            scrollback_lines: 0,
            in_escape: false,
            escape: Vec::new(),
            line_drawing: false,
            app_cursor_keys: false,
            current_fg: FG,
            current_bg: BG,
            current_bold: false,
            running: false,
            dirty: true,
        });
    }

    app().lines.push(Vec::new());
    start_lxe_shell();
    render();
    app().canvas.focus();

    app().win.on_key_down(|ke| {
        handle_key(ke);
    });

    app().canvas.on_mouse_down(|_, _, _| {
        app().canvas.focus();
    });

    app().canvas.on_wheel(|dz| {
        handle_wheel(dz);
    });

    app().win.on_resize(|_| {
        update_pty_size();
        app().dirty = true;
        render();
        app().canvas.focus();
    });

    app().win.on_close(|_| {
        stop_child();
        anyui::quit();
    });

    anyui::set_timer(30, || {
        poll_child();
        if app().dirty {
            render();
            app().dirty = false;
        }
    });

    anyui::run();
}

fn start_lxe_shell() {
    match shell_availability(&LxeConfig::load()) {
        LxeShellAvailability::Ready { rootfs, bash } => {
            push_line(&format!("LXE rootfs: {}", rootfs), DIM);
            push_line(&format!("bash: {}", bash), DIM);
        }
        LxeShellAvailability::MissingRootfs { rootfs } => {
            push_line("LXE Shell cannot start.", ERROR);
            push_line(&format!("LXE is not installed at {}", rootfs), ERROR);
            push_line("Run `lxe init` first.", DIM);
            return;
        }
        LxeShellAvailability::MissingBash { rootfs } => {
            push_line("LXE Shell cannot start.", ERROR);
            push_line(&format!("bash is not installed in {}", rootfs), ERROR);
            push_line(
                "Install bash with `lxe apt install bash` or re-run `lxe init`.",
                DIM,
            );
            return;
        }
    }

    let pid = process::getpid();
    let out_name = format!("lxeshell:o:{}", pid);
    let in_name = format!("lxeshell:i:{}", pid);
    let stdout_pipe = ipc::pipe_create(&out_name);
    let stdin_pipe = ipc::pipe_create(&in_name);
    if stdout_pipe == 0 || stdin_pipe == 0 {
        push_line("Failed to create LXE Shell pipes.", ERROR);
        return;
    }

    let tid = process::spawn_piped_full(
        "/System/bin/lxe",
        "lxe shell --pty-bridge",
        stdout_pipe,
        stdin_pipe,
    );
    if tid == u32::MAX {
        ipc::pipe_close(stdout_pipe);
        ipc::pipe_close(stdin_pipe);
        push_line("Failed to start /System/bin/lxe shell --pty-bridge.", ERROR);
        return;
    }

    app().stdout_pipe = stdout_pipe;
    app().stdin_pipe = stdin_pipe;
    app().child_tid = tid;
    app().running = true;
    update_pty_size();
}

fn stop_child() {
    let a = app();
    if a.child_tid != 0 && a.running {
        let _ = process::kill(a.child_tid);
    }
    if a.stdout_pipe != 0 {
        let _ = ipc::pipe_close(a.stdout_pipe);
        a.stdout_pipe = 0;
    }
    if a.stdin_pipe != 0 {
        let _ = ipc::pipe_close(a.stdin_pipe);
        a.stdin_pipe = 0;
    }
    a.running = false;
}

fn poll_child() {
    let stdout_pipe = app().stdout_pipe;
    if stdout_pipe != 0 {
        let mut buf = [0u8; 2048];
        loop {
            let n = ipc::pipe_read(stdout_pipe, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            for &b in &buf[..n as usize] {
                feed_output_byte(b);
            }
        }
    }

    let child_tid = app().child_tid;
    if app().running && child_tid != 0 {
        let status = process::try_waitpid(child_tid);
        if status != process::STILL_RUNNING {
            let a = app();
            a.running = false;
            if a.stdout_pipe != 0 {
                let _ = ipc::pipe_close(a.stdout_pipe);
                a.stdout_pipe = 0;
            }
            if a.stdin_pipe != 0 {
                let _ = ipc::pipe_close(a.stdin_pipe);
                a.stdin_pipe = 0;
            }
            push_line("", DIM);
            push_line(&format!("[lxe shell exited: {}]", status), DIM);
        }
    }
}

fn update_pty_size() {
    let stdin_pipe = app().stdin_pipe;
    if stdin_pipe == 0 {
        return;
    }
    let cols = terminal_cols().clamp(20, u16::MAX as usize) as u16;
    let rows = visible_rows().clamp(5, u16::MAX as usize) as u16;
    let _ = ipc::pty_set_winsize(stdin_pipe, cols, rows);
}

fn handle_key(ke: &KeyEvent) {
    if handle_scrollback_key(ke) {
        return;
    }

    if app().stdin_pipe == 0 || !app().running {
        return;
    }

    let mut tmp = [0u8; 8];
    let data = key_bytes(ke, &mut tmp);
    if !data.is_empty() {
        app().scrollback_lines = 0;
        let _ = ipc::pipe_write(app().stdin_pipe, data);
    }
}

fn handle_scrollback_key(ke: &KeyEvent) -> bool {
    if !ke.shift() {
        return false;
    }

    match ke.keycode {
        KEY_PAGE_UP => scroll_by(visible_rows().max(1) as isize),
        KEY_PAGE_DOWN => scroll_by(-(visible_rows().max(1) as isize)),
        KEY_HOME => scroll_to_top(),
        KEY_END => scroll_to_bottom(),
        _ => return false,
    }
    true
}

fn handle_wheel(dz: i32) {
    if dz == 0 {
        return;
    }
    let delta = dz as isize * WHEEL_SCROLL_LINES as isize;
    scroll_by(delta);
}

fn key_bytes<'a>(ke: &KeyEvent, tmp: &'a mut [u8; 8]) -> &'a [u8] {
    if ke.ctrl() {
        if let Some(ch) = core::char::from_u32(ke.char_code) {
            let lower = ch.to_ascii_lowercase();
            if lower >= 'a' && lower <= 'z' {
                tmp[0] = (lower as u8) & 0x1f;
                return &tmp[..1];
            }
        }
    }

    match ke.keycode {
        KEY_ENTER => b"\n",
        KEY_BACKSPACE => b"\x7f",
        KEY_TAB => {
            if ke.shift() {
                b"\x1b[Z"
            } else {
                b"\t"
            }
        }
        KEY_ESCAPE => b"\x1b",
        KEY_UP => {
            if app().app_cursor_keys {
                b"\x1bOA"
            } else {
                b"\x1b[A"
            }
        }
        KEY_DOWN => {
            if app().app_cursor_keys {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            }
        }
        KEY_RIGHT => {
            if app().app_cursor_keys {
                b"\x1bOC"
            } else {
                b"\x1b[C"
            }
        }
        KEY_LEFT => {
            if app().app_cursor_keys {
                b"\x1bOD"
            } else {
                b"\x1b[D"
            }
        }
        KEY_HOME => b"\x1b[H",
        KEY_END => b"\x1b[F",
        KEY_DELETE => b"\x1b[3~",
        KEY_PAGE_UP => b"\x1b[5~",
        KEY_PAGE_DOWN => b"\x1b[6~",
        KEY_F1 => b"\x1bOP",
        KEY_F2 => b"\x1bOQ",
        KEY_F3 => b"\x1bOR",
        KEY_F4 => b"\x1bOS",
        KEY_F5 => b"\x1b[15~",
        KEY_F6 => b"\x1b[17~",
        KEY_F7 => b"\x1b[18~",
        KEY_F8 => b"\x1b[19~",
        KEY_F9 => b"\x1b[20~",
        KEY_F10 => b"\x1b[21~",
        KEY_F11 => b"\x1b[23~",
        KEY_F12 => b"\x1b[24~",
        _ => {
            if ke.char_code == 0 || ke.alt() {
                return &[];
            }
            if let Some(ch) = core::char::from_u32(ke.char_code) {
                let s = ch.encode_utf8(tmp);
                s.as_bytes()
            } else {
                &[]
            }
        }
    }
}

fn push_line(text: &str, _color: u32) {
    let a = app();
    if a.lines.is_empty() {
        a.lines.push(Vec::new());
    }
    let keep_view = a.scrollback_lines > 0;
    let mut line = Vec::new();
    for ch in text.chars() {
        line.push(Cell {
            ch,
            fg: _color,
            bg: BG,
            bold: false,
        });
    }
    a.lines.push(line);
    if keep_view {
        a.scrollback_lines = a.scrollback_lines.saturating_add(1);
    }
    trim_lines(a);
    a.cursor_row = a.lines.len().saturating_sub(1);
    a.cursor_col = a.lines.last().map(|s| s.len()).unwrap_or(0);
    clamp_scrollback(a);
    a.dirty = true;
}

fn feed_output_byte(b: u8) {
    if app().in_escape {
        app().escape.push(b);
        if escape_sequence_complete(b) {
            finish_escape();
        } else if app().escape.len() > 64 {
            app().in_escape = false;
            app().escape.clear();
        }
        return;
    }

    match b {
        b'\x1b' => {
            app().in_escape = true;
            app().escape.clear();
        }
        b'\r' => app().cursor_col = 0,
        b'\n' => newline(),
        0x0e => app().line_drawing = true,
        0x0f => app().line_drawing = false,
        b'\x08' => backspace(),
        b'\t' => {
            let next_tab = ((app().cursor_col / 8) + 1) * 8;
            while app().cursor_col < next_tab {
                put_char(' ');
            }
        }
        0x20..=0x7e => {
            let ch = if app().line_drawing {
                acs_char(b)
            } else {
                b as char
            };
            put_char(ch);
        }
        _ => {}
    }
    app().dirty = true;
}

fn escape_sequence_complete(b: u8) -> bool {
    let escape = &app().escape;
    if escape.is_empty() {
        return false;
    }

    match escape[0] {
        b'[' => escape.len() > 1 && (0x40..=0x7e).contains(&b),
        b'O' => escape.len() >= 2,
        b']' => b == 0x07,
        b'(' | b')' | b'*' | b'+' => escape.len() >= 2,
        _ => true,
    }
}

fn finish_escape() {
    if app().escape.is_empty() {
        app().in_escape = false;
        return;
    }

    if app().escape[0] != b'[' {
        match app().escape[0] {
            b'(' | b')' | b'*' | b'+' => match app().escape.get(1).copied().unwrap_or(b'B') {
                b'0' => app().line_drawing = true,
                b'B' | b'A' => app().line_drawing = false,
                _ => {}
            },
            b'=' => {}
            b'>' => {}
            _ => {}
        }
        app().in_escape = false;
        app().escape.clear();
        app().dirty = true;
        return;
    }

    let final_byte = *app().escape.last().unwrap_or(&0);
    match final_byte {
        b'A' => {
            let n = csi_first_number().unwrap_or(1);
            app().cursor_row = app().cursor_row.saturating_sub(n);
            ensure_cursor_line();
        }
        b'B' => {
            let n = csi_first_number().unwrap_or(1);
            app().cursor_row = app().cursor_row.saturating_add(n);
            ensure_cursor_line();
        }
        b'K' => erase_line(csi_first_number().unwrap_or(0)),
        b'J' => match csi_first_number().unwrap_or(0) {
            1 => erase_to_screen_start(),
            2 | 3 => clear_screen(),
            _ => erase_to_screen_end(),
        },
        b'C' => {
            let n = csi_first_number().unwrap_or(1);
            app().cursor_col = app().cursor_col.saturating_add(n);
        }
        b'D' => {
            let n = csi_first_number().unwrap_or(1);
            app().cursor_col = app().cursor_col.saturating_sub(n);
        }
        b'G' | b'`' => {
            let col = csi_first_number().unwrap_or(1);
            app().cursor_col = col.saturating_sub(1);
        }
        b'H' | b'f' => {
            let row = csi_number_at(0).unwrap_or(1);
            let col = csi_number_at(1).unwrap_or(1);
            app().cursor_row = row.saturating_sub(1);
            app().cursor_col = col.saturating_sub(1);
            ensure_cursor_line();
        }
        b'd' => {
            let row = csi_first_number().unwrap_or(1);
            app().cursor_row = row.saturating_sub(1);
            ensure_cursor_line();
        }
        b'h' | b'l' => {
            let set = final_byte == b'h';
            if csi_is_private() {
                if csi_has_number(1) {
                    app().app_cursor_keys = set;
                }
                if csi_has_number(1049) && set {
                    clear_screen();
                }
            }
        }
        b'm' => apply_sgr(),
        b's' | b'u' => {}
        _ => {}
    }
    app().in_escape = false;
    app().escape.clear();
    app().dirty = true;
}

fn apply_sgr() {
    let params = csi_params();
    if params.is_empty() {
        reset_attrs();
        return;
    }

    let mut i = 0usize;
    while i < params.len() {
        let p = params[i].unwrap_or(0);
        match p {
            0 => reset_attrs(),
            1 => app().current_bold = true,
            2 => app().current_fg = DIM,
            22 => {
                app().current_bold = false;
                app().current_fg = FG;
            }
            30..=37 => app().current_fg = ansi_color(p - 30, false),
            39 => app().current_fg = FG,
            40..=47 => app().current_bg = ansi_color(p - 40, false),
            49 => app().current_bg = BG,
            90..=97 => app().current_fg = ansi_color(p - 90, true),
            100..=107 => app().current_bg = ansi_color(p - 100, true),
            38 | 48 => {
                if i + 2 < params.len() && params[i + 1] == Some(5) {
                    if let Some(idx) = params[i + 2] {
                        let color = ansi_256_color(idx);
                        if p == 38 {
                            app().current_fg = color;
                        } else {
                            app().current_bg = color;
                        }
                    }
                    i += 2;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn reset_attrs() {
    let a = app();
    a.current_fg = FG;
    a.current_bg = BG;
    a.current_bold = false;
}

fn ansi_color(idx: usize, bright: bool) -> u32 {
    const NORMAL: [u32; 8] = [
        0xFF0C0D12,
        0xFFCC5C5C,
        0xFF7BC67E,
        0xFFD7C46A,
        0xFF6FA8DC,
        0xFFC586C0,
        0xFF5FC8D7,
        0xFFE8E8EA,
    ];
    const BRIGHT: [u32; 8] = [
        0xFF555866,
        0xFFFF7A7A,
        0xFF9BEA9E,
        0xFFFFE082,
        0xFF8FCBFF,
        0xFFE4A5DF,
        0xFF83E7F3,
        0xFFFFFFFF,
    ];
    if bright {
        BRIGHT[idx.min(7)]
    } else {
        NORMAL[idx.min(7)]
    }
}

fn ansi_256_color(idx: usize) -> u32 {
    if idx < 8 {
        return ansi_color(idx, false);
    }
    if idx < 16 {
        return ansi_color(idx - 8, true);
    }
    if idx >= 232 {
        let v = 8 + ((idx.min(255) - 232) as u32 * 10);
        return 0xFF000000 | (v << 16) | (v << 8) | v;
    }
    let n = idx - 16;
    let r = color_cube_component(n / 36);
    let g = color_cube_component((n / 6) % 6);
    let b = color_cube_component(n % 6);
    0xFF000000 | (r << 16) | (g << 8) | b
}

fn color_cube_component(v: usize) -> u32 {
    if v == 0 {
        0
    } else {
        55 + (v.min(5) as u32 * 40)
    }
}

fn csi_params() -> Vec<Option<usize>> {
    let mut out = Vec::new();
    let mut n = 0usize;
    let mut seen = false;
    for &b in app().escape.iter().skip(1) {
        if b.is_ascii_digit() {
            seen = true;
            n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
        } else if b == b';' {
            out.push(if seen { Some(n) } else { None });
            n = 0;
            seen = false;
        } else if b == b'?' {
            n = 0;
            seen = false;
        } else {
            break;
        }
    }
    if seen {
        out.push(Some(n));
    } else if app().escape.contains(&b';') {
        out.push(None);
    }
    out
}

fn csi_first_number() -> Option<usize> {
    csi_number_at(0)
}

fn csi_number_at(wanted: usize) -> Option<usize> {
    let mut n = 0usize;
    let mut seen = false;
    let mut index = 0usize;
    for &b in &app().escape {
        if b.is_ascii_digit() {
            seen = true;
            n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
        } else if b == b';' {
            if index == wanted {
                return if seen { Some(n) } else { None };
            }
            index += 1;
            n = 0;
            seen = false;
        } else if seen || b == b'?' {
            if index == wanted && seen {
                return Some(n);
            }
            if b != b'?' {
                break;
            }
        }
    }
    if index == wanted && seen {
        Some(n)
    } else {
        None
    }
}

fn csi_is_private() -> bool {
    app().escape.get(1).copied() == Some(b'?')
}

fn csi_has_number(needle: usize) -> bool {
    let mut n = 0usize;
    let mut seen = false;
    for &b in &app().escape {
        if b.is_ascii_digit() {
            seen = true;
            n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
        } else {
            if seen && n == needle {
                return true;
            }
            if b == b';' || b == b'?' {
                n = 0;
                seen = false;
            } else if seen {
                break;
            }
        }
    }
    seen && n == needle
}

fn newline() {
    let a = app();
    let keep_view = a.scrollback_lines > 0;
    a.cursor_row = a.cursor_row.saturating_add(1);
    while a.lines.len() <= a.cursor_row {
        a.lines.push(Vec::new());
    }
    if keep_view {
        a.scrollback_lines = a.scrollback_lines.saturating_add(1);
    }
    a.cursor_col = 0;
    trim_lines(a);
    clamp_scrollback(a);
}

fn put_char(ch: char) {
    let a = app();
    ensure_cursor_line_for(a);
    let cell = Cell {
        ch,
        fg: a.current_fg,
        bg: a.current_bg,
        bold: a.current_bold,
    };
    let blank = Cell::blank(a.current_fg, a.current_bg, a.current_bold);
    let line = &mut a.lines[a.cursor_row];
    while a.cursor_col > line.len() {
        line.push(blank);
    }
    let pos = a.cursor_col.min(line.len());
    if pos < line.len() {
        line.remove(pos);
    }
    line.insert(pos, cell);
    a.cursor_col = pos + 1;
}

fn backspace() {
    let a = app();
    if a.lines.is_empty() || a.cursor_col == 0 {
        return;
    }
    ensure_cursor_line_for(a);
    let line = &mut a.lines[a.cursor_row];
    let pos = a.cursor_col.min(line.len());
    if pos > 0 {
        line.remove(pos - 1);
        a.cursor_col = pos - 1;
    }
}

fn erase_line(mode: usize) {
    let a = app();
    if a.lines.is_empty() {
        return;
    }
    ensure_cursor_line_for(a);
    let blank = Cell::blank(a.current_fg, a.current_bg, a.current_bold);
    let line = &mut a.lines[a.cursor_row];
    let pos = a.cursor_col.min(line.len());
    match mode {
        1 => {
            let end = pos.min(line.len());
            for cell in line.iter_mut().take(end) {
                *cell = blank;
            }
        }
        2 => line.clear(),
        _ => line.truncate(pos),
    }
}

fn clear_screen() {
    let a = app();
    a.lines.clear();
    a.lines.push(Vec::new());
    a.cursor_row = 0;
    a.cursor_col = 0;
    a.scrollback_lines = 0;
}

fn erase_to_screen_end() {
    let a = app();
    ensure_cursor_line_for(a);
    let pos = a.cursor_col.min(a.lines[a.cursor_row].len());
    a.lines[a.cursor_row].truncate(pos);
    let start = a.cursor_row.saturating_add(1);
    for line in a.lines.iter_mut().skip(start) {
        line.clear();
    }
}

fn erase_to_screen_start() {
    let a = app();
    ensure_cursor_line_for(a);
    for line in a.lines.iter_mut().take(a.cursor_row) {
        line.clear();
    }
    let pos = a.cursor_col.min(a.lines[a.cursor_row].len());
    let fg = a.current_fg;
    let bg = a.current_bg;
    let bold = a.current_bold;
    for cell in a.lines[a.cursor_row].iter_mut().take(pos) {
        *cell = Cell::blank(fg, bg, bold);
    }
}

fn ensure_cursor_line() {
    let a = app();
    ensure_cursor_line_for(a);
}

fn ensure_cursor_line_for(a: &mut LxeShellApp) {
    if a.lines.is_empty() {
        a.lines.push(Vec::new());
    }
    let max_row = a.cursor_row.min(MAX_LINES.saturating_sub(1));
    a.cursor_row = max_row;
    while a.lines.len() <= a.cursor_row {
        a.lines.push(Vec::new());
    }
}

fn acs_char(b: u8) -> char {
    match b {
        b'q' => '-',
        b'x' => '|',
        b'l' | b'k' | b'm' | b'j' | b't' | b'u' | b'v' | b'w' | b'n' => '+',
        b'a' | b'f' | b'g' | b'~' => '.',
        b'`' | b'\'' => '+',
        _ => b as char,
    }
}

fn trim_lines(a: &mut LxeShellApp) {
    while a.lines.len() > MAX_LINES {
        a.lines.remove(0);
        a.cursor_row = a.cursor_row.saturating_sub(1);
    }
    clamp_scrollback(a);
}

fn visible_rows() -> usize {
    let h = app().canvas.get_height() as i32;
    ((h - PAD_Y * 2).max(LINE_H) / LINE_H) as usize
}

fn terminal_cols() -> usize {
    let (w, _) = app().canvas.get_size();
    let w = w as i32;
    ((w - PAD_X * 2 - 8).max(CELL_W) / CELL_W) as usize
}

fn max_scrollback(a: &LxeShellApp) -> usize {
    a.lines.len().saturating_sub(visible_rows())
}

fn clamp_scrollback(a: &mut LxeShellApp) {
    let max = max_scrollback(a);
    if a.scrollback_lines > max {
        a.scrollback_lines = max;
    }
}

fn scroll_by(delta: isize) {
    let a = app();
    let max = max_scrollback(a);
    let next = if delta >= 0 {
        a.scrollback_lines.saturating_add(delta as usize)
    } else {
        a.scrollback_lines.saturating_sub((-delta) as usize)
    };
    a.scrollback_lines = next.min(max);
    a.dirty = true;
    render();
    a.dirty = false;
}

fn scroll_to_top() {
    let a = app();
    a.scrollback_lines = max_scrollback(a);
    a.dirty = true;
    render();
    a.dirty = false;
}

fn scroll_to_bottom() {
    let a = app();
    a.scrollback_lines = 0;
    a.dirty = true;
    render();
    a.dirty = false;
}

fn render() {
    let a = app();
    let canvas = a.canvas;
    canvas.clear(BG);

    clamp_scrollback(a);
    let (w, _) = canvas.get_size();
    let w = w as i32;
    let h = canvas.get_height() as i32;
    let rows = ((h - PAD_Y * 2).max(LINE_H) / LINE_H) as usize;
    let bottom = a.lines.len().saturating_sub(a.scrollback_lines);
    let start = bottom.saturating_sub(rows);
    let end = bottom.min(a.lines.len());
    let mut y = PAD_Y;
    for line in a.lines.iter().skip(start).take(end.saturating_sub(start)) {
        draw_terminal_line(canvas, PAD_X, y, line);
        y += LINE_H;
    }

    if a.lines.len() > rows {
        draw_scrollbar(canvas, w, h, rows, start);
    }

    if a.running && a.scrollback_lines == 0 {
        if a.cursor_row >= start && a.cursor_row < end {
            let cursor_row = a.cursor_row.saturating_sub(start);
            let x = PAD_X + (a.cursor_col as i32 * CELL_W);
            let y = PAD_Y + cursor_row as i32 * LINE_H + LINE_H - 3;
            canvas.fill_rect(x, y, CELL_W as u32, 2, CURSOR);
        }
    }
}

fn draw_terminal_line(canvas: anyui::Canvas, x: i32, y: i32, line: &[Cell]) {
    if line.is_empty() {
        return;
    }

    let mut col = 0usize;
    while col < line.len() {
        let bg = line[col].bg;
        if bg != BG {
            let start = col;
            while col < line.len() && line[col].bg == bg {
                col += 1;
            }
            canvas.fill_rect(
                x + (start as i32 * CELL_W),
                y,
                ((col - start) as i32 * CELL_W).max(CELL_W) as u32,
                LINE_H as u32,
                bg,
            );
        } else {
            col += 1;
        }
    }

    col = 0;
    while col < line.len() {
        let fg = line[col].fg;
        let bold = line[col].bold;
        let start = col;
        let mut text = String::new();
        while col < line.len() && line[col].fg == fg && line[col].bold == bold {
            text.push(line[col].ch);
            col += 1;
        }
        let _ = bold;
        canvas.draw_text(x + (start as i32 * CELL_W), y, fg, 4, FONT_SIZE, &text);
    }
}

fn draw_scrollbar(canvas: anyui::Canvas, w: i32, h: i32, rows: usize, start: usize) {
    let track_x = (w - 6).max(PAD_X);
    let track_y = PAD_Y;
    let track_h = (h - PAD_Y * 2).max(LINE_H);
    canvas.fill_rect(track_x, track_y, 3, track_h as u32, SCROLL_TRACK);

    let total = app().lines.len().max(1);
    let visible = rows.min(total).max(1);
    let thumb_h = ((track_h as usize * visible) / total)
        .max(18)
        .min(track_h as usize) as i32;
    let max_start = total.saturating_sub(visible).max(1);
    let travel = (track_h - thumb_h).max(0);
    let thumb_y = track_y + ((travel as usize * start.min(max_start)) / max_start) as i32;
    canvas.fill_rect(track_x - 1, thumb_y, 5, thumb_h as u32, SCROLL_THUMB);
}
