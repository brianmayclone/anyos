#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;
use anyos_std::process;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::ui::window;
use alloc::string::ToString;

anyos_std::entry!(main);

// ─── Constants ───────────────────────────────────────────────────────────────
const CELL_W: u16 = 8;
const CELL_H: u16 = 16;
const MAX_SCROLLBACK: usize = 500;

// Colors (ARGB)
const COLOR_BG: u32 = 0xFF1E1E28;
const COLOR_FG: u32 = 0xFFCCCCCC;
const COLOR_PROMPT: u32 = 0xFF64FF64;
const COLOR_TITLE: u32 = 0xFF00C8FF;
const COLOR_DIM: u32 = 0xFF969696;

// Key codes from kernel (must match desktop.rs encode_key)
const KEY_ENTER: u32 = 0x100;
const KEY_BACKSPACE: u32 = 0x101;
const KEY_TAB: u32 = 0x102;
const KEY_UP: u32 = 0x105;
const KEY_DOWN: u32 = 0x106;
const KEY_LEFT: u32 = 0x107;
const KEY_RIGHT: u32 = 0x108;
const KEY_DELETE: u32 = 0x120;
const KEY_HOME: u32 = 0x121;
const KEY_END: u32 = 0x122;

// Event types
const EVENT_KEY_DOWN: u32 = 1;
const EVENT_RESIZE: u32 = 3;
const EVENT_MOUSE_SCROLL: u32 = 7;
const EVENT_WINDOW_CLOSE: u32 = 8;

// Modifier flags
const MOD_CTRL: u32 = 2;

// ─── Terminal Buffer ─────────────────────────────────────────────────────────

struct TerminalBuffer {
    lines: Vec<Vec<(char, u32)>>, // (character, color)
    cols: usize,
    visible_rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_offset: usize,
    current_color: u32,
}

impl TerminalBuffer {
    fn new(cols: usize, rows: usize) -> Self {
        let mut lines = Vec::new();
        lines.push(Vec::new());
        TerminalBuffer {
            lines,
            cols,
            visible_rows: rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            current_color: COLOR_FG,
        }
    }

    fn ensure_line(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    fn write_char(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.cursor_row += 1;
                self.cursor_col = 0;
                self.ensure_line(self.cursor_row);
                if self.cursor_row >= self.scroll_offset + self.visible_rows {
                    self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                }
                if self.lines.len() > MAX_SCROLLBACK {
                    let excess = self.lines.len() - MAX_SCROLLBACK;
                    self.lines.drain(0..excess);
                    self.cursor_row = self.cursor_row.saturating_sub(excess);
                    self.scroll_offset = self.scroll_offset.saturating_sub(excess);
                }
            }
            '\r' => {
                self.cursor_col = 0;
            }
            _ => {
                self.ensure_line(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                while line.len() <= self.cursor_col {
                    line.push((' ', COLOR_BG));
                }
                line[self.cursor_col] = (ch, self.current_color);
                self.cursor_col += 1;
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                    self.ensure_line(self.cursor_row);
                    if self.cursor_row >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                    }
                }
            }
        }
    }

    fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            if self.cursor_row < self.lines.len() {
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col < line.len() {
                    line.remove(self.cursor_col);
                }
            }
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        let max_offset = self.lines.len().saturating_sub(self.visible_rows);
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
    }
}

// ─── Foreground process tracker ──────────────────────────────────────────────

struct ForegroundProcess {
    tid: u32,
    pipe_id: u32,
}

// ─── Shell ───────────────────────────────────────────────────────────────────

struct Shell {
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    cwd: String,
}

impl Shell {
    fn new() -> Self {
        Shell {
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            cwd: String::from("/"),
        }
    }

    fn prompt(&self) -> String {
        format!("{}> ", self.cwd)
    }

    fn insert_char(&mut self, c: char) {
        if self.cursor >= self.input.len() {
            self.input.push(c);
        } else {
            self.input.insert(self.cursor, c);
        }
        self.cursor += 1;
        self.history_index = None;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    fn history_down(&mut self) {
        match self.history_index {
            None => return,
            Some(i) => {
                if i + 1 >= self.history.len() {
                    self.history_index = None;
                    self.input.clear();
                    self.cursor = 0;
                } else {
                    self.history_index = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                    self.cursor = self.input.len();
                }
            }
        }
    }

    /// Execute command. Returns (should_continue, optional foreground process).
    fn submit(&mut self, buf: &mut TerminalBuffer) -> (bool, Option<ForegroundProcess>) {
        let line = self.input.trim_matches(|c: char| c == ' ').to_string();
        buf.write_char('\n');

        if !line.is_empty() {
            if self.history.last().map_or(true, |last| *last != line) {
                self.history.push(line.clone());
                if self.history.len() > 64 {
                    self.history.remove(0);
                }
            }
        }

        self.input.clear();
        self.cursor = 0;
        self.history_index = None;

        if line.is_empty() {
            return (true, None);
        }

        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let args = parts.next().unwrap_or("");

        match cmd {
            "help" => self.cmd_help(buf),
            "echo" => {
                buf.current_color = COLOR_FG;
                buf.write_str(args);
                buf.write_char('\n');
            }
            "clear" => buf.clear(),
            "uname" => {
                buf.current_color = COLOR_FG;
                buf.write_str(".anyOS v0.1 i686\n");
            }
            "cd" => self.cmd_cd(args, buf),
            "pwd" => {
                buf.current_color = COLOR_FG;
                buf.write_str(&self.cwd);
                buf.write_char('\n');
            }
            "exit" => return (false, None),
            "reboot" => {
                buf.current_color = COLOR_FG;
                buf.write_str("Rebooting...\n");
                // Reboot via keyboard controller
                // (this is a syscall-less hack — TODO: add reboot syscall)
                process::exit(0);
            }
            _ => {
                // Check for background suffix: "cmd &"
                let (cmd_line, background) = if line.ends_with(" &") || line.ends_with("\t&") {
                    (&line[..line.len() - 2], true)
                } else if line.ends_with('&') && line.len() > 1 {
                    (&line[..line.len() - 1], true)
                } else {
                    (line.as_str(), false)
                };

                // Re-parse cmd and args from the (possibly trimmed) line
                let mut bg_parts = cmd_line.splitn(2, ' ');
                let bg_cmd = bg_parts.next().unwrap_or("");
                let raw_args = bg_parts.next().unwrap_or("");

                // Resolve arguments relative to cwd
                let resolved;
                let bg_args = if raw_args.is_empty() {
                    // Commands that default to cwd when no args given
                    match bg_cmd {
                        "ls" => self.cwd.as_str(),
                        _ => "",
                    }
                } else if !raw_args.starts_with('/') {
                    // Resolve relative path against cwd
                    resolved = if self.cwd == "/" {
                        format!("/{}", raw_args)
                    } else {
                        format!("{}/{}", self.cwd, raw_args)
                    };
                    &resolved
                } else {
                    raw_args
                };

                // Resolve command path:
                // - Absolute paths (/foo/bar) used as-is
                // - Relative paths (./foo, ../foo) resolved against cwd
                // - Bare names looked up in /bin/
                let path = if bg_cmd.starts_with('/') {
                    String::from(bg_cmd)
                } else if bg_cmd.starts_with("./") || bg_cmd.starts_with("../") {
                    if self.cwd == "/" {
                        format!("/{}", bg_cmd.trim_start_matches("./"))
                    } else {
                        format!("{}/{}", self.cwd, bg_cmd)
                    }
                } else {
                    format!("/bin/{}", bg_cmd)
                };

                // Build full args string with program name as argv[0]
                let full_args_buf;
                let full_args = if bg_args.is_empty() {
                    bg_cmd
                } else {
                    full_args_buf = format!("{} {}", bg_cmd, bg_args);
                    &full_args_buf
                };

                if background {
                    // Background: spawn without pipe or waiting
                    let tid = process::spawn(&path, full_args);
                    if tid == u32::MAX {
                        buf.current_color = COLOR_FG;
                        buf.write_str("Unknown command: ");
                        buf.write_str(bg_cmd);
                        buf.write_str("\n");
                    } else {
                        buf.current_color = COLOR_DIM;
                        let msg = format!("[{}] started in background\n", tid);
                        buf.write_str(&msg);
                    }
                } else {
                    // Foreground: capture output via pipe, poll in main loop
                    let pipe_name = format!("term:stdout:{}", bg_cmd);
                    let pipe_id = ipc::pipe_create(&pipe_name);

                    let tid = process::spawn_piped(&path, full_args, pipe_id);
                    if tid == u32::MAX {
                        ipc::pipe_close(pipe_id);
                        buf.current_color = COLOR_FG;
                        buf.write_str("Unknown command: ");
                        buf.write_str(bg_cmd);
                        buf.write_str("\nType 'help' for available commands.\n");
                    } else {
                        return (true, Some(ForegroundProcess { tid, pipe_id }));
                    }
                }
            }
        }

        (true, None)
    }

    fn cmd_help(&self, buf: &mut TerminalBuffer) {
        buf.current_color = COLOR_TITLE;
        buf.write_str(".anyOS Terminal - Commands:\n");
        buf.current_color = COLOR_FG;
        buf.write_str("\n");
        buf.write_str("  Built-in:\n");
        buf.write_str("    help     Show this help\n");
        buf.write_str("    echo     Print text\n");
        buf.write_str("    clear    Clear screen\n");
        buf.write_str("    cd       Change directory\n");
        buf.write_str("    pwd      Print working directory\n");
        buf.write_str("    uname    System identification\n");
        buf.write_str("    exit     Exit terminal\n");
        buf.write_str("\n");
        buf.write_str("  Programs (in /bin):\n");
        buf.write_str("    ls       List directory contents\n");
        buf.write_str("    cat      Show file contents\n");
        buf.write_str("    ping     Ping an IP address\n");
        buf.write_str("    dhcp     Request IP via DHCP\n");
        buf.write_str("    dns      Resolve hostname\n");
        buf.write_str("    ifconfig Network configuration\n");
        buf.write_str("    arp      Show ARP table\n");
        buf.write_str("    sysinfo  System information\n");
        buf.write_str("    dmesg    Kernel boot log\n");
        buf.write_str("\n");
        buf.write_str("  Tip: append & to run in background\n");
    }

    fn cmd_cd(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let target = args.trim();
        if target.is_empty() || target == "/" {
            self.cwd = String::from("/");
            return;
        }

        // Resolve path relative to cwd
        let new_path = if target.starts_with('/') {
            String::from(target)
        } else if target == ".." {
            // Go up one level
            if self.cwd == "/" {
                return;
            }
            let trimmed = self.cwd.trim_end_matches('/');
            match trimmed.rfind('/') {
                Some(0) => String::from("/"),
                Some(pos) => String::from(&trimmed[..pos]),
                None => String::from("/"),
            }
        } else {
            if self.cwd == "/" {
                format!("/{}", target)
            } else {
                format!("{}/{}", self.cwd, target)
            }
        };

        // Verify directory exists via stat
        let mut stat_buf = [0u32; 2];
        let ret = fs::stat(&new_path, &mut stat_buf);
        if ret != 0 {
            buf.current_color = COLOR_FG;
            buf.write_str("cd: ");
            buf.write_str(&new_path);
            buf.write_str(": No such directory\n");
            return;
        }
        // stat_buf[0] = type: 0=regular file, 1=directory
        if stat_buf[0] != 1 {
            buf.current_color = COLOR_FG;
            buf.write_str("cd: ");
            buf.write_str(&new_path);
            buf.write_str(": Not a directory\n");
            return;
        }

        self.cwd = new_path;
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render_terminal(win_id: u32, buf: &TerminalBuffer, win_w: u32, win_h: u32) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, COLOR_BG);

    let start_row = buf.scroll_offset;
    let end_row = (start_row + buf.visible_rows).min(buf.lines.len());

    // Build text line by line and draw
    for (screen_y, line_idx) in (start_row..end_row).enumerate() {
        let line = &buf.lines[line_idx];
        let py = (screen_y as u16) * CELL_H;

        // Group characters by color for efficient drawing
        let mut run_start = 0;
        let mut run_color = if !line.is_empty() { line[0].1 } else { COLOR_FG };
        let mut text_buf = String::new();

        for (col, &(ch, color)) in line.iter().enumerate() {
            if col >= buf.cols {
                break;
            }
            if color != run_color && !text_buf.is_empty() {
                let px = (run_start as u16) * CELL_W;
                window::draw_text_mono(win_id, px as i16, py as i16, run_color, &text_buf);
                text_buf.clear();
                run_start = col;
                run_color = color;
            }
            if text_buf.is_empty() {
                run_start = col;
                run_color = color;
            }
            text_buf.push(ch);
        }
        if !text_buf.is_empty() {
            let px = (run_start as u16) * CELL_W;
            window::draw_text_mono(win_id, px as i16, py as i16, run_color, &text_buf);
        }
    }

    // Draw cursor (inverted block)
    let cursor_screen_row = buf.cursor_row as i32 - buf.scroll_offset as i32;
    if cursor_screen_row >= 0 && (cursor_screen_row as usize) < buf.visible_rows {
        let cx = (buf.cursor_col as u16) * CELL_W;
        let cy = (cursor_screen_row as u16) * CELL_H;
        // Draw a white cursor block
        window::fill_rect(win_id, cx as i16, cy as i16, CELL_W, CELL_H, 0xFFCCCCCC);
    }

    window::present(win_id);
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // Create terminal window
    let win_id = window::create("Terminal", 50, 60, 640, 400);
    if win_id == u32::MAX {
        anyos_std::println!("terminal: failed to create window");
        return;
    }

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((640, 400));
    let cols = (win_w / CELL_W as u32) as usize;
    let rows = (win_h / CELL_H as u32) as usize;

    let mut buf = TerminalBuffer::new(cols, rows);
    let mut shell = Shell::new();

    // Welcome message
    buf.current_color = COLOR_TITLE;
    buf.write_str(".anyOS Terminal v0.1\n");
    buf.current_color = COLOR_DIM;
    buf.write_str("Type 'help' for available commands.\n\n");

    // Initial prompt
    buf.current_color = COLOR_PROMPT;
    let prompt = shell.prompt();
    buf.write_str(&prompt);
    buf.current_color = COLOR_FG;

    // Initial render
    render_terminal(win_id, &buf, win_w, win_h);

    let mut dirty = false;
    let mut event = [0u32; 5];
    let mut fg_proc: Option<ForegroundProcess> = None;

    loop {
        // Poll foreground process pipe for real-time output
        if let Some(ref fp) = fg_proc {
            let mut read_buf = [0u8; 512];
            loop {
                let n = ipc::pipe_read(fp.pipe_id, &mut read_buf);
                if n == 0 || n == u32::MAX {
                    break;
                }
                buf.current_color = COLOR_FG;
                if let Ok(s) = core::str::from_utf8(&read_buf[..n as usize]) {
                    buf.write_str(s);
                }
                dirty = true;
            }

            // Check if process exited (non-blocking)
            let status = process::try_waitpid(fp.tid);
            if status != process::STILL_RUNNING {
                // Drain remaining pipe data
                loop {
                    let n = ipc::pipe_read(fp.pipe_id, &mut read_buf);
                    if n == 0 || n == u32::MAX {
                        break;
                    }
                    buf.current_color = COLOR_FG;
                    if let Ok(s) = core::str::from_utf8(&read_buf[..n as usize]) {
                        buf.write_str(s);
                    }
                }
                let pipe_id = fp.pipe_id;
                let exit_code = status;
                fg_proc = None;
                ipc::pipe_close(pipe_id);

                if exit_code != 0 && exit_code != u32::MAX {
                    buf.current_color = COLOR_DIM;
                    let msg = format!("Process exited with code {}\n", exit_code);
                    buf.write_str(&msg);
                }

                // Show prompt again
                buf.current_color = COLOR_PROMPT;
                let prompt = shell.prompt();
                buf.write_str(&prompt);
                buf.current_color = COLOR_FG;
                dirty = true;
            }
        }

        // Poll events
        let got = window::get_event(win_id, &mut event);
        if got == 1 {
            if event[0] == EVENT_WINDOW_CLOSE {
                window::destroy(win_id);
                return;
            } else if event[0] == EVENT_RESIZE {
                win_w = event[1];
                win_h = event[2];
                let new_cols = (win_w / CELL_W as u32) as usize;
                let new_rows = (win_h / CELL_H as u32) as usize;
                buf.cols = new_cols;
                buf.visible_rows = new_rows;
                dirty = true;
            } else if event[0] == EVENT_MOUSE_SCROLL {
                let dz = event[1] as i32;
                if dz < 0 {
                    buf.scroll_up(3);
                } else if dz > 0 {
                    buf.scroll_down(3);
                }
                dirty = true;
            } else if event[0] == EVENT_KEY_DOWN && fg_proc.is_none() {
                let key_code = event[1];
                let char_val = event[2];
                let mods = event[3];

                match key_code {
                    KEY_ENTER => {
                        let (should_continue, new_fg) = shell.submit(&mut buf);
                        if !should_continue {
                            break;
                        }
                        if let Some(fp) = new_fg {
                            fg_proc = Some(fp);
                        } else {
                            buf.current_color = COLOR_PROMPT;
                            let prompt = shell.prompt();
                            buf.write_str(&prompt);
                            buf.current_color = COLOR_FG;
                        }
                        dirty = true;
                    }
                    KEY_BACKSPACE => {
                        if shell.cursor > 0 {
                            shell.backspace();
                            buf.backspace();
                            dirty = true;
                        }
                    }
                    KEY_UP => {
                        let old_len = shell.input.len();
                        shell.history_up();
                        // Erase old input from display
                        for _ in 0..old_len {
                            buf.backspace();
                        }
                        buf.write_str(&shell.input);
                        dirty = true;
                    }
                    KEY_DOWN => {
                        let old_len = shell.input.len();
                        shell.history_down();
                        for _ in 0..old_len {
                            buf.backspace();
                        }
                        buf.write_str(&shell.input);
                        dirty = true;
                    }
                    _ => {
                        // Check for Ctrl+C
                        if (mods & MOD_CTRL) != 0 && char_val == 'c' as u32 {
                            buf.write_str("^C\n");
                            shell.input.clear();
                            shell.cursor = 0;
                            buf.current_color = COLOR_PROMPT;
                            let prompt = shell.prompt();
                            buf.write_str(&prompt);
                            buf.current_color = COLOR_FG;
                            dirty = true;
                        } else if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                            // Printable character
                            let c = char_val as u8 as char;
                            if c >= ' ' {
                                shell.insert_char(c);
                                buf.write_char(c);
                                dirty = true;
                            }
                        }
                    }
                }
            }
        } else {
            // No event — render if dirty, then yield
            if dirty {
                render_terminal(win_id, &buf, win_w, win_h);
                dirty = false;
            }
            process::yield_cpu();
        }
    }

    window::destroy(win_id);
}
