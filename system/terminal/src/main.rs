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
const TEXT_PAD: u16 = 4;
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
    /// Intermediate pipe IDs from a pipeline (cmd1 | cmd2 | cmd3).
    /// Closed when the pipeline exits.
    extra_pipes: Vec<u32>,
}

// ─── Environment / PATH helpers ──────────────────────────────────────────────

/// Read a file into a buffer. Returns the number of bytes read.
fn read_file_to_buf(path: &str, buf: &mut [u8]) -> usize {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return 0;
    }
    let mut total = 0usize;
    loop {
        let n = fs::read(fd, &mut buf[total..]);
        if n == 0 || n == u32::MAX {
            break;
        }
        total += n as usize;
        if total >= buf.len() {
            break;
        }
    }
    fs::close(fd);
    total
}

/// Source an env file — supports:
///   KEY=VALUE
///   export KEY=VALUE
///   source /path/to/file
///   # comments
/// `depth` prevents infinite recursion.
fn source_env_file(path: &str, depth: u32) {
    if depth > 4 {
        return; // prevent infinite source loops
    }
    let mut data = [0u8; 2048];
    let total = read_file_to_buf(path, &mut data);
    if total == 0 {
        return;
    }

    let text = match core::str::from_utf8(&data[..total]) {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle 'source /path/to/file'
        if line.starts_with("source ") {
            let target = line[7..].trim();
            if !target.is_empty() {
                source_env_file(target, depth + 1);
            }
            continue;
        }

        // Strip optional 'export ' prefix
        let assignment = if line.starts_with("export ") {
            line[7..].trim()
        } else {
            line
        };

        if let Some(eq) = assignment.find('=') {
            let key = assignment[..eq].trim();
            let val = assignment[eq + 1..].trim();
            if !key.is_empty() {
                anyos_std::env::set(key, val);
            }
        }
    }
}

/// Load system and user env files.
fn load_dotenv() {
    // 1. System environment
    source_env_file("/System/env", 0);

    // 2. User environment — determine username from uid
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 32];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            if username != "root" {
                let user_env = format!("/Users/{}/env", username);
                source_env_file(&user_env, 0);
                // Update HOME and USER based on actual identity
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
                anyos_std::env::set("USER", username);
            }
        }
    }
}

/// Resolve a bare command name via PATH env var.
/// Returns the full path if found, None otherwise.
fn resolve_from_path(cmd: &str) -> Option<String> {
    let mut path_buf = [0u8; 256];
    let len = anyos_std::env::get("PATH", &mut path_buf);
    if len == u32::MAX {
        return None;
    }
    let path_str = match core::str::from_utf8(&path_buf[..len as usize]) {
        Ok(s) => s,
        Err(_) => return None,
    };
    let mut stat_buf = [0u32; 6];
    for dir in path_str.split(':') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        let candidate = format!("{}/{}", dir, cmd);
        if fs::stat(&candidate, &mut stat_buf) == 0 && stat_buf[0] == 0 {
            return Some(candidate);
        }
    }
    None
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

    fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    fn cursor_end(&mut self) {
        self.cursor = self.input.len();
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
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
                buf.write_str(".anyOS v0.1 x86_64\n");
            }
            "cd" => self.cmd_cd(args, buf),
            "pwd" => {
                buf.current_color = COLOR_FG;
                buf.write_str(&self.cwd);
                buf.write_char('\n');
            }
            "set" => self.cmd_set(args, buf),
            "export" => self.cmd_export(args, buf),
            "unset" => self.cmd_unset(args, buf),
            "source" | "." => self.cmd_source(args, buf),
            "su" => self.cmd_su(args, buf),
            "exit" => return (false, None),
            "reboot" => {
                buf.current_color = COLOR_FG;
                buf.write_str("Rebooting...\n");
                // Reboot via keyboard controller
                // (this is a syscall-less hack — TODO: add reboot syscall)
                process::exit(0);
            }
            _ => {
                // Check for pipeline: "cmd1 | cmd2 | cmd3"
                if line.contains('|') {
                    if let Some(fp) = self.execute_pipeline(&line, buf) {
                        return (true, Some(fp));
                    }
                    return (true, None);
                }

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

                // Pass arguments as-is — programs resolve relative paths
                // via PWD env var. Only special-case: ls defaults to cwd.
                let bg_args = if raw_args.is_empty() {
                    match bg_cmd {
                        "ls" => self.cwd.as_str(),
                        _ => "",
                    }
                } else {
                    raw_args
                };

                // Resolve command path:
                // - Absolute paths (/foo/bar) used as-is
                // - Relative paths (./foo, ../foo) resolved against cwd
                // - Bare names resolved via PATH
                let path = if bg_cmd.starts_with('/') {
                    String::from(bg_cmd)
                } else if bg_cmd.starts_with("./") || bg_cmd.starts_with("../") {
                    if self.cwd == "/" {
                        format!("/{}", bg_cmd.trim_start_matches("./"))
                    } else {
                        format!("{}/{}", self.cwd, bg_cmd)
                    }
                } else {
                    match resolve_from_path(bg_cmd) {
                        Some(p) => p,
                        None => format!("/System/bin/{}", bg_cmd),
                    }
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
                        return (true, Some(ForegroundProcess { tid, pipe_id, extra_pipes: Vec::new() }));
                    }
                }
            }
        }

        (true, None)
    }

    /// Execute a pipeline (cmd1 | cmd2 | cmd3).
    /// Returns a ForegroundProcess tracking the last command + display pipe.
    fn execute_pipeline(&mut self, line: &str, buf: &mut TerminalBuffer) -> Option<ForegroundProcess> {
        let segments: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if segments.len() < 2 {
            return None;
        }

        let n = segments.len();
        let mut pipes = Vec::new();

        // Create N pipes: pipes[0..n-2] are intermediate, pipes[n-1] is the display pipe
        for i in 0..n {
            let name = format!("term:pipe:{}", i);
            let pipe_id = ipc::pipe_create(&name);
            pipes.push(pipe_id);
        }

        let display_pipe = pipes[n - 1];
        let mut last_tid = 0u32;

        for (i, segment) in segments.iter().enumerate() {
            let mut parts = segment.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("").trim();
            let raw_args = parts.next().unwrap_or("").trim();

            if cmd.is_empty() {
                continue;
            }

            // Default args for specific commands
            let effective_args = if raw_args.is_empty() {
                match cmd {
                    "ls" => self.cwd.as_str(),
                    _ => "",
                }
            } else {
                raw_args
            };

            // Resolve command path
            let path = if cmd.starts_with('/') {
                String::from(cmd)
            } else if cmd.starts_with("./") || cmd.starts_with("../") {
                if self.cwd == "/" {
                    format!("/{}", cmd.trim_start_matches("./"))
                } else {
                    format!("{}/{}", self.cwd, cmd)
                }
            } else {
                match resolve_from_path(cmd) {
                    Some(p) => p,
                    None => format!("/System/bin/{}", cmd),
                }
            };

            // Build full args with program name as argv[0]
            let full_args = if effective_args.is_empty() {
                String::from(cmd)
            } else {
                format!("{} {}", cmd, effective_args)
            };

            let stdin_pipe = if i > 0 { pipes[i - 1] } else { 0 };
            let stdout_pipe = pipes[i];

            let tid = process::spawn_piped_full(&path, &full_args, stdout_pipe, stdin_pipe);
            if tid == u32::MAX {
                buf.current_color = COLOR_FG;
                buf.write_str("pipe: unknown command: ");
                buf.write_str(cmd);
                buf.write_char('\n');
                // Clean up all pipes
                for &p in &pipes {
                    ipc::pipe_close(p);
                }
                return None;
            }
            last_tid = tid;
        }

        // Intermediate pipes (not the display pipe) — cleaned up on exit
        let extra_pipes: Vec<u32> = pipes[..n - 1].to_vec();

        Some(ForegroundProcess {
            tid: last_tid,
            pipe_id: display_pipe,
            extra_pipes,
        })
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
        buf.write_str("    set      Set environment variable\n");
        buf.write_str("    export   Export environment variable\n");
        buf.write_str("    unset    Remove environment variable\n");
        buf.write_str("    uname    System identification\n");
        buf.write_str("    exit     Exit terminal\n");
        buf.write_str("\n");
        buf.write_str("  Programs (in PATH):\n");
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
        buf.write_str("  Tip: use | to pipe output: ls | cat\n");
    }

    fn cmd_set(&self, args: &str, buf: &mut TerminalBuffer) {
        let args = args.trim();
        if args.is_empty() {
            // List all variables
            let mut env_buf = [0u8; 4096];
            let total = anyos_std::env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            buf.current_color = COLOR_FG;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    buf.write_str(entry);
                    buf.write_char('\n');
                }
                offset += end + 1;
            }
            return;
        }
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if key.is_empty() {
                buf.current_color = COLOR_FG;
                buf.write_str("set: invalid variable name\n");
                return;
            }
            anyos_std::env::set(key, value);
        } else {
            // Show value of a single variable
            let mut val_buf = [0u8; 256];
            let len = anyos_std::env::get(args, &mut val_buf);
            buf.current_color = COLOR_FG;
            if len != u32::MAX {
                let val = core::str::from_utf8(&val_buf[..len as usize]).unwrap_or("");
                buf.write_str(args);
                buf.write_char('=');
                buf.write_str(val);
                buf.write_char('\n');
            } else {
                buf.write_str("set: '");
                buf.write_str(args);
                buf.write_str("' not set\n");
            }
        }
    }

    fn cmd_export(&self, args: &str, buf: &mut TerminalBuffer) {
        let args = args.trim();
        if args.is_empty() {
            // List all with "export" prefix
            let mut env_buf = [0u8; 4096];
            let total = anyos_std::env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            buf.current_color = COLOR_FG;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    buf.write_str("export ");
                    buf.write_str(entry);
                    buf.write_char('\n');
                }
                offset += end + 1;
            }
            return;
        }
        // Same as set — all env vars are "exported" (inherited by child processes)
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if !key.is_empty() {
                anyos_std::env::set(key, value);
            }
        } else {
            // Mark existing var as exported (no-op since all are exported)
            let mut val_buf = [0u8; 256];
            let len = anyos_std::env::get(args, &mut val_buf);
            if len == u32::MAX {
                anyos_std::env::set(args, "");
            }
        }
    }

    fn cmd_unset(&self, args: &str, buf: &mut TerminalBuffer) {
        let key = args.trim();
        if key.is_empty() {
            buf.current_color = COLOR_FG;
            buf.write_str("Usage: unset VARIABLE\n");
            return;
        }
        anyos_std::env::unset(key);
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
        let mut stat_buf = [0u32; 6];
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
        anyos_std::env::set("PWD", &self.cwd);
        fs::chdir(&self.cwd);
    }

    fn cmd_source(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let path = args.trim();
        if path.is_empty() {
            buf.current_color = COLOR_FG;
            buf.write_str("usage: source <file>\n");
            return;
        }

        let mut data = [0u8; 4096];
        let total = read_file_to_buf(path, &mut data);
        if total == 0 {
            buf.current_color = COLOR_FG;
            buf.write_str("source: cannot read '");
            buf.write_str(path);
            buf.write_str("'\n");
            return;
        }

        let text = match core::str::from_utf8(&data[..total]) {
            Ok(s) => s,
            Err(_) => {
                buf.current_color = COLOR_FG;
                buf.write_str("source: invalid UTF-8\n");
                return;
            }
        };

        for line in text.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse command and args
            let mut parts = line.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let cmd_args = parts.next().unwrap_or("");

            match cmd {
                "export" => self.cmd_export(cmd_args, buf),
                "set" => self.cmd_set(cmd_args, buf),
                "unset" => self.cmd_unset(cmd_args, buf),
                "cd" => self.cmd_cd(cmd_args, buf),
                "echo" => {
                    buf.current_color = COLOR_FG;
                    buf.write_str(cmd_args);
                    buf.write_char('\n');
                }
                "source" | "." => self.cmd_source(cmd_args, buf),
                _ => {
                    // Handle KEY=VALUE assignment (no command prefix)
                    if let Some(eq) = line.find('=') {
                        if !line[..eq].contains(' ') {
                            let key = line[..eq].trim();
                            let val = line[eq + 1..].trim();
                            if !key.is_empty() {
                                anyos_std::env::set(key, val);
                            }
                            continue;
                        }
                    }

                    // External command — resolve path, spawn, and wait
                    let resolved = if cmd.starts_with('/') {
                        String::from(cmd)
                    } else if cmd.starts_with("./") || cmd.starts_with("../") {
                        if self.cwd == "/" {
                            format!("/{}", cmd.trim_start_matches("./"))
                        } else {
                            format!("{}/{}", self.cwd, cmd)
                        }
                    } else {
                        match resolve_from_path(cmd) {
                            Some(p) => p,
                            None => format!("/System/bin/{}", cmd),
                        }
                    };

                    let full_args = if cmd_args.is_empty() {
                        String::from(cmd)
                    } else {
                        format!("{} {}", cmd, cmd_args)
                    };

                    let tid = process::spawn(&resolved, &full_args);
                    if tid != u32::MAX {
                        process::waitpid(tid);
                    }
                }
            }
        }
    }

    fn cmd_su(&self, args: &str, buf: &mut TerminalBuffer) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let username = if parts.is_empty() { "root" } else { parts[0] };
        let password = if parts.len() > 1 { parts[1] } else { "" };

        buf.current_color = COLOR_FG;
        if process::authenticate(username, password) {
            // Update environment to reflect new identity
            anyos_std::env::set("USER", username);
            let uid = process::getuid();
            if uid == 0 {
                anyos_std::env::set("HOME", "/");
            } else {
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
            }
            buf.write_str("Switched to user '");
            buf.write_str(username);
            buf.write_str("'.\n");
        } else {
            buf.write_str("su: authentication failed for '");
            buf.write_str(username);
            buf.write_str("'\n");
        }
    }
}

// ─── Builtins & Completion ───────────────────────────────────────────────────

const BUILTINS: &[&str] = &[
    "help", "echo", "clear", "uname", "cd", "pwd",
    "set", "export", "unset", "source", "su", "exit", "reboot",
];

/// Erase the input portion of the current display line and rewrite it.
fn redraw_input_line(buf: &mut TerminalBuffer, shell: &Shell) {
    let prompt_len = shell.prompt().len();
    if buf.cursor_row < buf.lines.len() {
        buf.lines[buf.cursor_row].truncate(prompt_len);
    }
    buf.cursor_col = prompt_len;
    buf.current_color = COLOR_FG;
    buf.write_str(&shell.input);
    buf.cursor_col = prompt_len + shell.cursor;
}

/// List directory entries as (name, is_directory) pairs.
fn list_dir_entries(path: &str) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    let mut dir_buf = [0u8; 64 * 64];
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        return entries;
    }
    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > dir_buf.len() {
            break;
        }
        let entry_type = dir_buf[off];
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len.min(56)];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            entries.push((String::from(name), entry_type == 1));
        }
    }
    entries
}

/// Find the longest common prefix among a set of strings.
fn common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = &items[0];
    let mut len = first.len();
    for item in &items[1..] {
        len = len.min(item.len());
        for (i, (a, b)) in first.bytes().zip(item.bytes()).enumerate() {
            if i >= len { break; }
            if a != b {
                len = i;
                break;
            }
        }
    }
    String::from(&first[..len])
}

/// Complete a command name (first word on the line).
fn complete_command(prefix: &str) -> Vec<String> {
    let mut matches = Vec::new();
    for &b in BUILTINS {
        if b.starts_with(prefix) {
            matches.push(String::from(b));
        }
    }
    let mut path_buf = [0u8; 256];
    let plen = anyos_std::env::get("PATH", &mut path_buf);
    if plen != u32::MAX {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..plen as usize]) {
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() { continue; }
                for (name, _) in list_dir_entries(dir) {
                    if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
                        matches.push(name);
                    }
                }
            }
        }
    }
    for (name, _) in list_dir_entries("/System/bin") {
        if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
            matches.push(name);
        }
    }
    matches.sort();
    matches
}

/// Complete a file or directory path (argument position).
fn complete_path(word: &str, cwd: &str) -> Vec<String> {
    let (dir_prefix, file_prefix) = if let Some(slash_pos) = word.rfind('/') {
        (&word[..slash_pos + 1], &word[slash_pos + 1..])
    } else {
        ("", word)
    };
    let search_dir = if dir_prefix.is_empty() {
        String::from(cwd)
    } else if dir_prefix.starts_with('/') {
        let p = dir_prefix.trim_end_matches('/');
        if p.is_empty() { String::from("/") } else { String::from(p) }
    } else {
        if cwd == "/" {
            format!("/{}", dir_prefix.trim_end_matches('/'))
        } else {
            format!("{}/{}", cwd, dir_prefix.trim_end_matches('/'))
        }
    };
    let entries = list_dir_entries(&search_dir);
    let mut matches = Vec::new();
    for (name, is_dir) in entries {
        if name.starts_with(file_prefix) {
            let completion = if is_dir {
                format!("{}{}/", dir_prefix, name)
            } else {
                format!("{}{}", dir_prefix, name)
            };
            matches.push(completion);
        }
    }
    matches.sort();
    matches
}

/// Handle Tab key for autocompletion.
fn handle_tab(shell: &mut Shell, buf: &mut TerminalBuffer) {
    let before_cursor = &shell.input[..shell.cursor];
    let word_start = before_cursor.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let word = String::from(&before_cursor[word_start..]);
    let is_command = !before_cursor[..word_start].contains(|c: char| c != ' ');
    let completions = if is_command {
        complete_command(&word)
    } else {
        complete_path(&word, &shell.cwd)
    };

    if completions.is_empty() {
        return;
    }

    if completions.len() == 1 {
        let completion = &completions[0];
        if completion.len() > word.len() {
            let remaining = String::from(&completion[word.len()..]);
            for ch in remaining.chars() {
                shell.insert_char(ch);
            }
        }
        if !completion.ends_with('/') {
            shell.insert_char(' ');
        }
        redraw_input_line(buf, shell);
    } else {
        let common = common_prefix(&completions);
        if common.len() > word.len() {
            let remaining = String::from(&common[word.len()..]);
            for ch in remaining.chars() {
                shell.insert_char(ch);
            }
        }
        buf.write_char('\n');
        buf.current_color = COLOR_FG;
        for c in &completions {
            let display = c.rsplit('/').next().unwrap_or(c);
            buf.write_str(display);
            buf.write_str("  ");
        }
        buf.write_char('\n');
        buf.current_color = COLOR_PROMPT;
        let prompt = shell.prompt();
        buf.write_str(&prompt);
        buf.current_color = COLOR_FG;
        buf.write_str(&shell.input);
        let prompt_len = prompt.len();
        buf.cursor_col = prompt_len + shell.cursor;
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
        let py = TEXT_PAD + (screen_y as u16) * CELL_H;

        // Group characters by color for efficient drawing
        let mut run_start = 0;
        let mut run_color = if !line.is_empty() { line[0].1 } else { COLOR_FG };
        let mut text_buf = String::new();

        for (col, &(ch, color)) in line.iter().enumerate() {
            if col >= buf.cols {
                break;
            }
            if color != run_color && !text_buf.is_empty() {
                let px = TEXT_PAD + (run_start as u16) * CELL_W;
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
            let px = TEXT_PAD + (run_start as u16) * CELL_W;
            window::draw_text_mono(win_id, px as i16, py as i16, run_color, &text_buf);
        }
    }

    // Draw cursor (inverted block)
    let cursor_screen_row = buf.cursor_row as i32 - buf.scroll_offset as i32;
    if cursor_screen_row >= 0 && (cursor_screen_row as usize) < buf.visible_rows {
        let cx = TEXT_PAD + (buf.cursor_col as u16) * CELL_W;
        let cy = TEXT_PAD + (cursor_screen_row as u16) * CELL_H;
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

    // Set up menu bar
    let mut mb = window::MenuBarBuilder::new()
        .menu("Shell")
            .item(1, "Clear", 0)
            .item(2, "Help", 0)
            .separator()
            .item(3, "Close", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win_id, data);

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((640, 400));
    let cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / CELL_W as u32) as usize;
    let rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / CELL_H as u32) as usize;

    let mut buf = TerminalBuffer::new(cols, rows);
    let mut shell = Shell::new();

    // Load environment from /System/env
    load_dotenv();
    anyos_std::env::set("PWD", "/"); // PWD is dynamic, always set

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
                // Copy out pipe IDs before dropping fg_proc
                let pipe_id = fp.pipe_id;
                let extra_pipes: Vec<u32> = fp.extra_pipes.clone();
                let exit_code = status;
                fg_proc = None;
                ipc::pipe_close(pipe_id);
                // Close intermediate pipes from pipeline
                for &p in &extra_pipes {
                    ipc::pipe_close(p);
                }

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
            } else if event[0] == window::EVENT_MENU_ITEM {
                let item_id = event[2];
                match item_id {
                    1 => { // Clear
                        buf.clear();
                        buf.current_color = COLOR_PROMPT;
                        let prompt = shell.prompt();
                        buf.write_str(&prompt);
                        buf.current_color = COLOR_FG;
                        dirty = true;
                    }
                    2 => { // Help
                        shell.cmd_help(&mut buf);
                        buf.current_color = COLOR_PROMPT;
                        let prompt = shell.prompt();
                        buf.write_str(&prompt);
                        buf.current_color = COLOR_FG;
                        dirty = true;
                    }
                    3 => { // Close
                        window::destroy(win_id);
                        return;
                    }
                    _ => {}
                }
            } else if event[0] == EVENT_RESIZE {
                win_w = event[1];
                win_h = event[2];
                let new_cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / CELL_W as u32) as usize;
                let new_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / CELL_H as u32) as usize;
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
            } else if event[0] == EVENT_KEY_DOWN {
                let key_code = event[1];
                let char_val = event[2];
                let mods = event[3];

                // Ctrl+C: cancel foreground process or clear input
                if (mods & MOD_CTRL) != 0 && char_val == 'c' as u32 {
                    if let Some(fp) = fg_proc.take() {
                        process::kill(fp.tid);
                        let mut drain_buf = [0u8; 512];
                        loop {
                            let n = ipc::pipe_read(fp.pipe_id, &mut drain_buf);
                            if n == 0 || n == u32::MAX { break; }
                        }
                        ipc::pipe_close(fp.pipe_id);
                        for &p in &fp.extra_pipes {
                            ipc::pipe_close(p);
                        }
                    }
                    buf.write_str("^C\n");
                    shell.input.clear();
                    shell.cursor = 0;
                    buf.current_color = COLOR_PROMPT;
                    let prompt = shell.prompt();
                    buf.write_str(&prompt);
                    buf.current_color = COLOR_FG;
                    dirty = true;
                } else if fg_proc.is_none() {
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
                            shell.history_up();
                            redraw_input_line(&mut buf, &shell);
                            dirty = true;
                        }
                        KEY_DOWN => {
                            shell.history_down();
                            redraw_input_line(&mut buf, &shell);
                            dirty = true;
                        }
                        KEY_LEFT => {
                            if shell.cursor > 0 {
                                shell.cursor_left();
                                buf.cursor_col -= 1;
                                dirty = true;
                            }
                        }
                        KEY_RIGHT => {
                            if shell.cursor < shell.input.len() {
                                shell.cursor_right();
                                buf.cursor_col += 1;
                                dirty = true;
                            }
                        }
                        KEY_HOME => {
                            if shell.cursor > 0 {
                                buf.cursor_col -= shell.cursor;
                                shell.cursor_home();
                                dirty = true;
                            }
                        }
                        KEY_END => {
                            if shell.cursor < shell.input.len() {
                                buf.cursor_col += shell.input.len() - shell.cursor;
                                shell.cursor_end();
                                dirty = true;
                            }
                        }
                        KEY_DELETE => {
                            if shell.cursor < shell.input.len() {
                                shell.delete_at_cursor();
                                let row = buf.cursor_row;
                                let col = buf.cursor_col;
                                if row < buf.lines.len() && col < buf.lines[row].len() {
                                    buf.lines[row].remove(col);
                                }
                                dirty = true;
                            }
                        }
                        KEY_TAB => {
                            handle_tab(&mut shell, &mut buf);
                            dirty = true;
                        }
                        _ => {
                            if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                                let c = char_val as u8 as char;
                                if c >= ' ' {
                                    let at_end = shell.cursor >= shell.input.len();
                                    shell.insert_char(c);
                                    if at_end {
                                        buf.write_char(c);
                                    } else {
                                        buf.ensure_line(buf.cursor_row);
                                        let row = buf.cursor_row;
                                        let col = buf.cursor_col;
                                        let color = buf.current_color;
                                        buf.lines[row].insert(col, (c, color));
                                        buf.cursor_col += 1;
                                    }
                                    dirty = true;
                                }
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
            process::sleep(8); // ~125 Hz poll for pipe output
        }
    }

    window::destroy(win_id);
}
