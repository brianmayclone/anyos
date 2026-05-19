#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{ipc, process};
use libanyui_client as anyui;
use libanyui_client::{
    KeyEvent, KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_ESCAPE, KEY_HOME,
    KEY_LEFT, KEY_PAGE_DOWN, KEY_PAGE_UP, KEY_RIGHT, KEY_TAB, KEY_UP,
};
use liblxecore::config::LxeConfig;
use liblxecore::readiness::{shell_availability, LxeShellAvailability};

anyos_std::entry!(main);

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const FONT_SIZE: u16 = 14;
const LINE_H: i32 = 18;
const PAD_X: i32 = 10;
const PAD_Y: i32 = 8;
const BG: u32 = 0xFF171821;
const FG: u32 = 0xFFE8E8EA;
const DIM: u32 = 0xFF8B8D98;
const ERROR: u32 = 0xFFFF6B6B;
const CURSOR: u32 = 0xFFE8E8EA;
const MAX_LINES: usize = 2000;

struct LxeShellApp {
    win: anyui::Window,
    canvas: anyui::Canvas,
    stdin_pipe: u32,
    stdout_pipe: u32,
    child_tid: u32,
    lines: Vec<String>,
    cursor_col: usize,
    in_escape: bool,
    escape: Vec<u8>,
    pending_echo: Vec<u8>,
    skip_erase_echo: u8,
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
            cursor_col: 0,
            in_escape: false,
            escape: Vec::new(),
            pending_echo: Vec::new(),
            skip_erase_echo: 0,
            running: false,
            dirty: true,
        });
    }

    app().lines.push(String::new());
    start_lxe_shell();
    render();
    app().canvas.focus();

    app().win.on_key_down(|ke| {
        handle_key(ke);
    });

    app().canvas.on_mouse_down(|_, _, _| {
        app().canvas.focus();
    });

    app().win.on_resize(|_| {
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

fn handle_key(ke: &KeyEvent) {
    if app().stdin_pipe == 0 || !app().running {
        return;
    }

    let mut tmp = [0u8; 8];
    let data = key_bytes(ke, &mut tmp);
    if !data.is_empty() {
        local_echo(data);
        let _ = ipc::pipe_write(app().stdin_pipe, data);
    }
}

fn local_echo(data: &[u8]) {
    let mut changed = false;
    for &b in data {
        match b {
            0x20..=0x7e => {
                insert_char(b as char);
                app().pending_echo.push(b);
                changed = true;
            }
            b'\x7f' | b'\x08' => {
                backspace();
                app().skip_erase_echo = 3;
                changed = true;
            }
            _ => {}
        }
    }
    if changed {
        app().dirty = true;
        render();
        app().dirty = false;
    }
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
        KEY_TAB => b"\t",
        KEY_ESCAPE => b"\x1b",
        KEY_UP => b"\x1b[A",
        KEY_DOWN => b"\x1b[B",
        KEY_RIGHT => b"\x1b[C",
        KEY_LEFT => b"\x1b[D",
        KEY_HOME => b"\x1b[H",
        KEY_END => b"\x1b[F",
        KEY_DELETE => b"\x1b[3~",
        KEY_PAGE_UP => b"\x1b[5~",
        KEY_PAGE_DOWN => b"\x1b[6~",
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
        a.lines.push(String::new());
    }
    a.lines.push(String::from(text));
    trim_lines(a);
    a.cursor_col = a.lines.last().map(|s| s.len()).unwrap_or(0);
    a.dirty = true;
}

fn feed_output_byte(b: u8) {
    if consume_local_echo(b) {
        return;
    }

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
        b'\x08' => backspace(),
        b'\t' => {
            for _ in 0..4 {
                put_char(' ');
            }
        }
        0x20..=0x7e => put_char(b as char),
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
        b']' => b == 0x07,
        _ => true,
    }
}

fn consume_local_echo(b: u8) -> bool {
    if !app().pending_echo.is_empty() && app().pending_echo[0] == b {
        app().pending_echo.remove(0);
        return true;
    }

    if app().skip_erase_echo > 0 && (b == b'\x08' || b == b'\x7f' || b == b' ') {
        app().skip_erase_echo -= 1;
        return true;
    }

    false
}

fn finish_escape() {
    if app().escape.is_empty() {
        app().in_escape = false;
        return;
    }

    if app().escape[0] != b'[' {
        app().in_escape = false;
        app().escape.clear();
        app().dirty = true;
        return;
    }

    let final_byte = *app().escape.last().unwrap_or(&0);
    match final_byte {
        b'K' => erase_line(csi_first_number().unwrap_or(0)),
        b'J' => {
            if csi_first_number().unwrap_or(0) == 2 {
                app().lines.clear();
                app().lines.push(String::new());
                app().cursor_col = 0;
            }
        }
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
            let col = csi_number_at(1).unwrap_or(1);
            app().cursor_col = col.saturating_sub(1);
        }
        b'm' | b'h' | b'l' | b's' | b'u' => {}
        _ => {}
    }
    app().in_escape = false;
    app().escape.clear();
    app().dirty = true;
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

fn newline() {
    let a = app();
    a.lines.push(String::new());
    a.cursor_col = 0;
    trim_lines(a);
}

fn insert_char(ch: char) {
    let a = app();
    if a.lines.is_empty() {
        a.lines.push(String::new());
    }
    let line = a.lines.last_mut().unwrap();
    let pos = a.cursor_col.min(line.len());
    line.insert(pos, ch);
    a.cursor_col = pos + ch.len_utf8();
}

fn put_char(ch: char) {
    let a = app();
    if a.lines.is_empty() {
        a.lines.push(String::new());
    }
    let line = a.lines.last_mut().unwrap();
    while a.cursor_col > line.len() {
        line.push(' ');
    }
    let pos = a.cursor_col.min(line.len());
    if pos < line.len() {
        line.remove(pos);
    }
    line.insert(pos, ch);
    a.cursor_col = pos + ch.len_utf8();
}

fn backspace() {
    let a = app();
    if a.lines.is_empty() || a.cursor_col == 0 {
        return;
    }
    let line = a.lines.last_mut().unwrap();
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
    let line = a.lines.last_mut().unwrap();
    let pos = a.cursor_col.min(line.len());
    match mode {
        1 => {
            let end = pos.min(line.len());
            line.replace_range(0..end, &" ".repeat(end));
        }
        2 => line.clear(),
        _ => line.truncate(pos),
    }
}

fn trim_lines(a: &mut LxeShellApp) {
    while a.lines.len() > MAX_LINES {
        a.lines.remove(0);
    }
}

fn render() {
    let a = app();
    let canvas = a.canvas;
    canvas.clear(BG);

    let h = canvas.get_height() as i32;
    let rows = ((h - PAD_Y * 2).max(LINE_H) / LINE_H) as usize;
    let start = a.lines.len().saturating_sub(rows);
    let mut y = PAD_Y;
    for line in a.lines.iter().skip(start) {
        canvas.draw_text(PAD_X, y, FG, 4, FONT_SIZE, line);
        y += LINE_H;
    }

    if a.running {
        let visible_cursor_line = a.lines.len().saturating_sub(1) >= start;
        if visible_cursor_line {
            let cursor_row = a.lines.len().saturating_sub(1).saturating_sub(start);
            let x = PAD_X + (a.cursor_col as i32 * 8);
            let y = PAD_Y + cursor_row as i32 * LINE_H + LINE_H - 3;
            canvas.fill_rect(x, y, 8, 2, CURSOR);
        }
    }
}
