//! AC — Anywhere Commander
//!
//! A full-featured two-panel file manager for anyOS, modelled closely after
//! Midnight Commander (mc) and Norton Commander.
//!
//! Module layout:
//!   main.rs   — entry point, main event loop, action dispatch
//!   term.rs   — terminal abstraction (ANSI escapes, con_set_mode, TermBuf)
//!   panel.rs  — panel state (path, entries, cursor, marks, sort)
//!   fs.rs     — filesystem helpers (readdir, copy, delete, rename, stat)
//!   render.rs — full-screen renderer (panels, menubar, fnbar, cmdline)
//!   dialog.rs — modal dialogs (confirm, input, message, help, sort menu)
//!   input.rs  — keyboard input decoding (Key enum, poll/read_key)

#![no_std]
#![no_main]

anyos_std::entry!(main);

mod term;
mod panel;
mod fs;
mod render;
mod dialog;
mod input;

use term::TermBuf;
use panel::Panel;
use input::Key;
use dialog::input_buf_as_str;

// ─── Application state ───────────────────────────────────────────────────────

/// Which panel currently has keyboard focus.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivePanel { Left, Right }

/// Command-line buffer (mini-shell line at the bottom).
const CMDLINE_MAX: usize = 512;

struct AppState {
    left:         Panel,
    right:        Panel,
    active:       ActivePanel,
    cmdline:      [u8; CMDLINE_MAX],
    cmdline_len:  usize,
    cols:         u32,
    rows:         u32,
    /// Set to true when screen needs a full redraw.
    dirty:        bool,
    /// Set to false to exit the main loop.
    running:      bool,
}

impl AppState {
    fn new() -> Self {
        let (cols, rows) = term::get_size();
        let left  = Panel::new("/", true);
        let right = Panel::new("/", false);
        Self {
            left,
            right,
            active:      ActivePanel::Left,
            cmdline:     [0u8; CMDLINE_MAX],
            cmdline_len: 0,
            cols,
            rows,
            dirty:   true,
            running: true,
        }
    }

    fn active_panel(&self) -> &Panel {
        match self.active { ActivePanel::Left => &self.left, ActivePanel::Right => &self.right }
    }

    fn active_panel_mut(&mut self) -> &mut Panel {
        match self.active { ActivePanel::Left => &mut self.left, ActivePanel::Right => &mut self.right }
    }

    fn inactive_panel(&self) -> &Panel {
        match self.active { ActivePanel::Left => &self.right, ActivePanel::Right => &self.left }
    }

    fn inactive_panel_mut(&mut self) -> &mut Panel {
        match self.active { ActivePanel::Left => &mut self.right, ActivePanel::Right => &mut self.left }
    }

    fn panel_visible_rows(&self) -> usize {
        // panel_rows = rows - 3 (menu + cmdline + fnbar), then minus 4 (top border + header + status + bottom border)
        let pr = self.rows.saturating_sub(3);
        pr.saturating_sub(4) as usize
    }

    fn switch_panel(&mut self) {
        self.left.focused  = self.active == ActivePanel::Right;
        self.right.focused = self.active == ActivePanel::Left;
        self.active = match self.active {
            ActivePanel::Left  => ActivePanel::Right,
            ActivePanel::Right => ActivePanel::Left,
        };
        self.dirty = true;
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("ac - Two-panel file manager\n\nUsage: ac");
        return;
    }

    term::enter_tui_mode();

    let mut state = AppState::new();
    let mut buf   = TermBuf::new();

    // Initial screen size check — re-read in case env is set after entry
    let (c, r) = term::get_size();
    state.cols = c.max(40);
    state.rows = r.max(10);

    loop {
        if !state.running { break; }

        // Sync visible_rows into panels before rendering
        let vr = state.panel_visible_rows();
        state.left.ensure_visible_with(vr);
        state.right.ensure_visible_with(vr);

        // Render if dirty
        if state.dirty {
            render::render_all(
                &mut buf,
                state.cols,
                state.rows,
                &state.left,
                &state.right,
                &state.cmdline,
                state.cmdline_len,
            );
            buf.flush();
            state.dirty = false;
        }

        // Blocking read: wait for the next key without burning CPU.
        // input::read_key() yields internally between polls.
        let key = input::read_key();
        handle_key(&mut state, &mut buf, key);
    }

    // Restore terminal
    term::leave_tui_mode();
    let mut buf = TermBuf::new();
    term::clear_screen(&mut buf);
    term::home(&mut buf);
    term::reset(&mut buf);
    buf.flush();
}

// ─── Key dispatch ────────────────────────────────────────────────────────────

fn handle_key(state: &mut AppState, buf: &mut TermBuf, key: Key) {
    let vr = state.panel_visible_rows();

    match key {
        // ── Navigation ────────────────────────────────────────────────────────
        Key::Up    => { state.active_panel_mut().move_up(); state.dirty = true; }
        Key::Down  => { state.active_panel_mut().move_down(); state.dirty = true; }
        Key::Left  => {
            // Left arrow in inactive panel: navigate to parent
            if state.active == ActivePanel::Right {
                go_parent(state);
            } else {
                go_parent(state);
            }
        }
        Key::Right => {
            // Enter dir or do nothing for files
            if !state.active_panel_mut().enter_dir() {
                // If it's a file, we could open it — for now just redraw
            }
            state.dirty = true;
        }
        Key::PageUp   => { state.active_panel_mut().page_up(vr); state.dirty = true; }
        Key::PageDown => { state.active_panel_mut().page_down(vr); state.dirty = true; }
        Key::Home     => { state.active_panel_mut().move_home(); state.dirty = true; }
        Key::End      => { state.active_panel_mut().move_end(); state.dirty = true; }

        // ── Panel switch ──────────────────────────────────────────────────────
        Key::Tab => state.switch_panel(),

        // ── Mark / unmark ─────────────────────────────────────────────────────
        Key::Char(b' ') => {
            // Space marks and advances (like Norton Commander Insert key)
            state.active_panel_mut().toggle_mark_and_advance();
            state.dirty = true;
        }

        // ── Enter: run cmdline command or open dir / execute file ─────────────
        Key::Enter => {
            // Check cmdline for built-in commands first
            if state.cmdline_len > 0 {
                let cmd = core::str::from_utf8(&state.cmdline[..state.cmdline_len]).unwrap_or("");
                let cmd = cmd.trim();
                if cmd == "exit" || cmd == "quit" {
                    state.running = false;
                    return;
                }
                // Unknown command: clear cmdline and ignore
                state.cmdline_len = 0;
                state.dirty = true;
                return;
            }
            let entered = state.active_panel_mut().enter_dir();
            if !entered {
                // File: try to exec it
                if let Some(e) = state.active_panel().current() {
                    if !e.is_dir() {
                        let (pbuf, plen) = state.active_panel().current_path();
                        if let Ok(s) = core::str::from_utf8(&pbuf[..plen]) {
                            exec_file(state, buf, s);
                        }
                    }
                }
            }
            state.dirty = true;
        }

        // ── Backspace: delete from cmdline or go to parent ───────────────────
        Key::Backspace => {
            if state.cmdline_len > 0 {
                state.cmdline_len -= 1;
            } else {
                go_parent(state);
            }
            state.dirty = true;
        }

        // ── Command line input ────────────────────────────────────────────────
        Key::Char(c) if c >= 32 && c < 127 => {
            if state.cmdline_len < CMDLINE_MAX - 1 {
                state.cmdline[state.cmdline_len] = c;
                state.cmdline_len += 1;
                state.dirty = true;
            }
        }

        // ── Function keys ─────────────────────────────────────────────────────
        Key::F1  => { action_help(state, buf); }
        Key::F2  => { /* user menu — not implemented */ state.dirty = true; }
        Key::F3  => { action_view(state, buf); }
        Key::F4  => { action_edit(state, buf); }
        Key::F5  => { action_copy(state, buf); }
        Key::F6  => { action_move(state, buf); }
        Key::F7  => { action_mkdir(state, buf); }
        Key::F8  => { action_delete(state, buf); }
        Key::F9  => { /* menu bar — not implemented */ state.dirty = true; }
        Key::F10 => { state.running = false; }

        // ── Ctrl shortcuts ────────────────────────────────────────────────────
        Key::CtrlC | Key::CtrlD => { state.running = false; }

        Key::Ctrl(b'r') => {
            // Ctrl+R: reload panel
            state.active_panel_mut().reload();
            state.dirty = true;
        }
        Key::Ctrl(b's') => {
            // Ctrl+S: sort menu
            action_sort(state, buf);
        }
        Key::Ctrl(b'h') => {
            // Ctrl+H: TODO toggle hidden files
            state.dirty = true;
        }
        Key::Ctrl(b'q') => { state.running = false; }

        Key::Escape => {
            // Clear command line
            state.cmdline_len = 0;
            state.dirty = true;
        }

        _ => {}
    }
}

// ─── Actions ─────────────────────────────────────────────────────────────────

fn go_parent(state: &mut AppState) {
    state.active_panel_mut().go_to_parent();
    state.dirty = true;
}

fn action_help(state: &mut AppState, buf: &mut TermBuf) {
    dialog::show_help(buf, state.cols, state.rows);
    state.dirty = true;
}

fn action_view(state: &mut AppState, buf: &mut TermBuf) {
    let (pbuf, plen) = state.active_panel().current_path();
    let path = match core::str::from_utf8(&pbuf[..plen]) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Check it's a file
    match fs::stat(path) {
        Some(si) if si.is_dir => return,
        None => return,
        Some(_) => {}
    }

    view_file(state, buf, path);
    state.dirty = true;
}

fn view_file(state: &mut AppState, buf: &mut TermBuf, path: &str) {
    // Simple pager: read file and display with scrolling
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        dialog::message(buf, state.cols, state.rows, " Error ", "Cannot open file", true);
        return;
    }

    // Read up to 64 KB for viewing
    const VIEW_MAX: usize = 65536;
    const MAX_LINES: usize = 4096;

    // Static buffers to avoid large stack allocations (AC is single-threaded).
    static mut VIEW_BUF:        [u8;     VIEW_MAX]  = [0u8; VIEW_MAX];
    static mut LINE_STARTS_BUF: [usize;  MAX_LINES] = [0usize; MAX_LINES];
    static mut LINE_ENDS_BUF:   [usize;  MAX_LINES] = [0usize; MAX_LINES];

    let (raw, line_starts, line_ends) = unsafe {
        (&mut VIEW_BUF, &mut LINE_STARTS_BUF, &mut LINE_ENDS_BUF)
    };

    let n = anyos_std::fs::read(fd, raw);
    anyos_std::fs::close(fd);

    if n == u32::MAX || n == 0 {
        dialog::message(buf, state.cols, state.rows, " Empty ", "File is empty", false);
        return;
    }
    let content = &raw[..n as usize];

    // Split into lines (up to MAX_LINES) using static buffers
    let mut line_count = 0usize;
    let mut ls = 0usize;
    for (i, &b) in content.iter().enumerate() {
        if b == b'\n' {
            if line_count < MAX_LINES {
                line_starts[line_count] = ls;
                line_ends[line_count]   = i;
                line_count += 1;
            }
            ls = i + 1;
        }
    }
    if ls < content.len() && line_count < MAX_LINES {
        line_starts[line_count] = ls;
        line_ends[line_count]   = content.len();
        line_count += 1;
    }

    let visible = state.rows.saturating_sub(3) as usize;
    let mut scroll = 0usize;

    loop {
        // Render
        term::clear_screen(buf);
        term::home(buf);

        // Title bar
        term::goto(buf, 1, 1);
        term::fg16(buf, term::color::BLACK);
        term::bg16(buf, term::color::BRIGHT_WHITE);
        term::bold(buf);
        buf.push_str(" View: ");
        let bn = fs::path_basename(path);
        let bnl = bn.len().min(state.cols as usize - 20);
        buf.push_bytes(&bn.as_bytes()[..bnl]);
        term::erase_to_eol(buf);
        term::reset(buf);

        for vi in 0..visible {
            let li = scroll + vi;
            term::goto(buf, 1, 2 + vi as u32);
            term::fg16(buf, term::color::BRIGHT_WHITE);
            term::bg16(buf, 4);
            if li < line_count {
                let s = line_starts[li];
                let e = line_ends[li];
                let line = &content[s..e];
                let lw = (state.cols as usize).saturating_sub(1);
                let ll = line.len().min(lw);
                // Filter non-printable (keep newline-stripped line)
                for &b in &line[..ll] {
                    if b >= 32 || b == b'\t' { buf.push_byte(b); } else { buf.push_byte(b'.'); }
                }
            }
            term::erase_to_eol(buf);
        }

        // Status / help line
        term::goto(buf, 1, state.rows);
        term::fg16(buf, term::color::BLACK);
        term::bg16(buf, term::color::BRIGHT_CYAN);
        buf.push_str(" Up/Down/PgUp/PgDn: scroll   q/F3/Esc: exit ");
        term::erase_to_eol(buf);
        term::reset(buf);
        buf.flush();

        match input::read_key() {
            Key::Up | Key::Char(b'k') => { if scroll > 0 { scroll -= 1; } }
            Key::Down | Key::Char(b'j') => {
                if scroll + visible < line_count { scroll += 1; }
            }
            Key::PageUp => { scroll = scroll.saturating_sub(visible); }
            Key::PageDown => {
                let max_scroll = line_count.saturating_sub(visible);
                scroll = (scroll + visible).min(max_scroll);
            }
            Key::Home | Key::Char(b'g') => { scroll = 0; }
            Key::End  | Key::Char(b'G') => {
                scroll = line_count.saturating_sub(visible);
            }
            Key::Escape | Key::Char(b'q') | Key::F3 => break,
            _ => {}
        }
    }
}

fn action_edit(state: &mut AppState, buf: &mut TermBuf) {
    let (pbuf, plen) = state.active_panel().current_path();
    let path = match core::str::from_utf8(&pbuf[..plen]) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Restore terminal, spawn vi, then re-enter TUI mode
    term::leave_tui_mode();
    let mut rbuf = TermBuf::new();
    term::clear_screen(&mut rbuf);
    rbuf.flush();

    // Build args string: path is passed directly as the argument
    // anyos_std::process::spawn(exe, args: &str)
    spawn_and_wait("/System/bin/vi", path);

    term::enter_tui_mode();
    state.active_panel_mut().reload();
    state.dirty = true;
}

fn action_copy(state: &mut AppState, buf: &mut TermBuf) {
    // F5 Copy: copy selected entries from active to inactive panel
    let dest_path = {
        let p = state.inactive_panel().path();
        let b = p.as_bytes();
        let mut arr = [0u8; fs::MAX_PATH];
        let n = b.len().min(fs::MAX_PATH);
        arr[..n].copy_from_slice(&b[..n]);
        (arr, n)
    };
    let dest_str = core::str::from_utf8(&dest_path.0[..dest_path.1]).unwrap_or("/");

    let result = dialog::input_line(
        buf, state.cols, state.rows,
        " Copy ",
        "Copy to:",
        dest_str,
    );

    let dst_buf = match result {
        Some(b) => b,
        None    => return,
    };
    let dst = input_buf_as_str(&dst_buf);

    let (paths, n) = state.active_panel().selected_paths();
    let mut errors = 0u32;
    for i in 0..n {
        let (ref pb, pl) = paths[i];
        let src = match core::str::from_utf8(&pb[..pl]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let name = fs::path_basename(src);
        let (dst_full, dl) = fs::path_join(dst, name);
        let dst_str = fs::buf_to_str(&dst_full, dl);

        if !fs::copy_file(src, dst_str) { errors += 1; }

        // Show progress
        let pct = if n > 0 { ((i + 1) as u32 * 100) / n as u32 } else { 100 };
        dialog::progress(buf, state.cols, state.rows, " Copying ", name, pct);
    }

    state.inactive_panel_mut().reload();
    state.active_panel_mut().reload();

    if errors > 0 {
        dialog::message(buf, state.cols, state.rows, " Copy Error ", "Some files failed to copy", true);
    }
    state.dirty = true;
}

fn action_move(state: &mut AppState, buf: &mut TermBuf) {
    // F6 Move/Rename
    let (pbuf, plen) = state.active_panel().current_path();
    let src = match core::str::from_utf8(&pbuf[..plen]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let dest_path = {
        let p = state.inactive_panel().path();
        let b = p.as_bytes();
        let mut arr = [0u8; fs::MAX_PATH];
        let n = b.len().min(fs::MAX_PATH);
        arr[..n].copy_from_slice(&b[..n]);
        (arr, n)
    };
    let dest_str = core::str::from_utf8(&dest_path.0[..dest_path.1]).unwrap_or("/");

    let result = dialog::input_line(
        buf, state.cols, state.rows,
        " Move / Rename ",
        "Move to:",
        dest_str,
    );
    let dst_buf = match result {
        Some(b) => b,
        None    => return,
    };
    let dst = input_buf_as_str(&dst_buf);

    let name = fs::path_basename(src);
    let (dst_full, dl) = fs::path_join(dst, name);
    let dst_str = fs::buf_to_str(&dst_full, dl);

    if !fs::rename(src, dst_str) {
        dialog::message(buf, state.cols, state.rows, " Error ", "Move failed", true);
    }

    state.left.reload();
    state.right.reload();
    state.dirty = true;
}

fn action_mkdir(state: &mut AppState, buf: &mut TermBuf) {
    let result = dialog::input_line(
        buf, state.cols, state.rows,
        " Make Directory ",
        "Directory name:",
        "",
    );
    let name_buf = match result {
        Some(b) => b,
        None    => return,
    };
    let name = input_buf_as_str(&name_buf);
    if name.is_empty() { state.dirty = true; return; }

    let panel_path = {
        let p = state.active_panel().path();
        let b = p.as_bytes();
        let mut arr = [0u8; fs::MAX_PATH];
        let n = b.len().min(fs::MAX_PATH);
        arr[..n].copy_from_slice(&b[..n]);
        (arr, n)
    };
    let pstr = core::str::from_utf8(&panel_path.0[..panel_path.1]).unwrap_or("/");

    let (full, fl) = fs::path_join(pstr, name);
    let fullstr = fs::buf_to_str(&full, fl);

    if !fs::make_dir(fullstr) {
        dialog::message(buf, state.cols, state.rows, " Error ", "mkdir failed", true);
    }

    state.active_panel_mut().reload();
    state.dirty = true;
}

fn action_delete(state: &mut AppState, buf: &mut TermBuf) {
    let (paths, n) = state.active_panel().selected_paths();
    if n == 0 { state.dirty = true; return; }

    let first_name = {
        let (ref pb, pl) = paths[0];
        let s = core::str::from_utf8(&pb[..pl]).unwrap_or("?");
        fs::path_basename(s)
    };
    let confirmed = dialog::confirm(
        buf, state.cols, state.rows,
        " Delete ",
        if n == 1 { first_name } else { "Delete selected files?" },
    );
    if !confirmed { state.dirty = true; return; }

    let mut errors = 0u32;
    for i in 0..n {
        let (ref pb, pl) = paths[i];
        let path = match core::str::from_utf8(&pb[..pl]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !fs::delete_file(path) { errors += 1; }
    }

    state.active_panel_mut().reload();
    if errors > 0 {
        dialog::message(buf, state.cols, state.rows, " Error ", "Some files could not be deleted", true);
    }
    state.dirty = true;
}

fn action_sort(state: &mut AppState, buf: &mut TermBuf) {
    if let Some((mode, rev)) = dialog::sort_menu(buf, state.cols, state.rows) {
        state.active_panel_mut().set_sort(mode, rev);
    }
    state.dirty = true;
}

fn exec_file(state: &mut AppState, buf: &mut TermBuf, path: &str) {
    // Determine if it's executable by extension or permissions (simple heuristic)
    let ext = fs::path_basename(path);
    if ext.ends_with(".sh") || !ext.contains('.') {
        term::leave_tui_mode();
        let mut rbuf = TermBuf::new();
        term::clear_screen(&mut rbuf);
        rbuf.flush();

        spawn_and_wait(path, "");

        // Wait for key before returning
        anyos_std::sys::con_write("\r\n[Press any key to return to AC]");
        input::read_key();

        term::enter_tui_mode();
        state.dirty = true;
    }
}

// ─── Process spawn helper ─────────────────────────────────────────────────────

fn spawn_and_wait(exe: &str, args: &str) {
    let tid = anyos_std::process::spawn(exe, args);
    if tid != u32::MAX {
        anyos_std::process::waitpid(tid);
    }
}
