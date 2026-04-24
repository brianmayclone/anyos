#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(target_os = "linux")]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, ipc, println, process, sys};
use libconf_schema::{default_bool, default_int, manifest, RegistryScope, ServiceSchema};
use libsvc::ServiceLifecycle;

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(main);

const PIPE_NAME: &str = "aslconsoled";
const STATUS_PATH: &str = "/System/var/asl/aslconsoled.status";
const DEFAULT_CANVAS_WIDTH: usize = 80;
const DEFAULT_CANVAS_HEIGHT: usize = 25;
const MAX_CANVAS_WIDTH: usize = 240;
const MAX_CANVAS_HEIGHT: usize = 100;

const DIRS: &[&str] = &["config"];
const DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/max_sessions", 128),
    default_bool("config/fallback_console_enabled", true),
];
const MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslconsoled",
    RegistryScope::System,
    1,
    DIRS,
    DEFAULTS,
    &[],
);
const SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslconsoled", &MANIFEST);

#[derive(Clone)]
struct ConsoleSession {
    distro: String,
    session_id: String,
    mode: String,
    console_pipe: String,
    stdin_pipe: String,
}

struct ConsoleState {
    sessions: Vec<ConsoleSession>,
    canvases: Vec<TextCanvas>,
    attach_count: u32,
    rejected_count: u32,
    write_count: u32,
    canvas_update_count: u32,
    output_bytes: u64,
    last_attach_ms: u32,
    last_write_ms: u32,
}

#[derive(Clone)]
struct TextCanvas {
    distro: String,
    width: usize,
    height: usize,
    cursor_x: usize,
    cursor_y: usize,
    cells: Vec<u8>,
    ansi: AnsiState,
}

#[derive(Clone)]
struct AnsiState {
    in_escape: bool,
    in_csi: bool,
    params: Vec<u16>,
    current: u16,
    has_current: bool,
}

fn main() {
    println!("aslconsoled: starting");
    let _ = SCHEMA.register();
    let mut lifecycle = ServiceLifecycle::connect("aslconsoled").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }
    let old_pipe = ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        ipc::pipe_close(old_pipe);
    }
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }
    let mut state = ConsoleState {
        sessions: Vec::new(),
        canvases: Vec::new(),
        attach_count: 0,
        rejected_count: 0,
        write_count: 0,
        canvas_update_count: 0,
        output_bytes: 0,
        last_attach_ms: 0,
        last_write_ms: 0,
    };
    write_status(&state);
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslconsoled: ready (pipe='{}')", PIPE_NAME);

    let mut pending = String::new();
    let mut buf = [0u8; 1024];
    loop {
        if handle_requests(pipe_id, &mut pending, &mut state, &mut buf) {
            process::sleep(20);
        } else {
            process::sleep(100);
        }
    }
}

fn handle_requests(
    pipe_id: u32,
    pending: &mut String,
    state: &mut ConsoleState,
    buf: &mut [u8],
) -> bool {
    let n = ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    let Ok(text) = core::str::from_utf8(&buf[..n as usize]) else {
        return true;
    };
    pending.push_str(text);
    while let Some(pos) = pending.find('\n') {
        let line = pending[..pos].trim().to_string();
        pending.drain(..=pos);
        if !line.is_empty() {
            handle_line(state, &line);
        }
    }
    true
}

fn handle_line(state: &mut ConsoleState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let response = dispatch(state, line[tab_pos + 1..].trim());
    let reply_pipe = ipc::pipe_open(&format!("aslconsoled-{}", tid));
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut ConsoleState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => ok_lines(alloc::vec![
            format!("sessions\t{}", state.sessions.len()),
            format!("canvases\t{}", state.canvases.len()),
            format!("attach_count\t{}", state.attach_count),
            format!("rejected_count\t{}", state.rejected_count),
            format!("write_count\t{}", state.write_count),
            format!("canvas_update_count\t{}", state.canvas_update_count),
            format!("output_bytes\t{}", state.output_bytes),
            format!("last_attach_ms\t{}", state.last_attach_ms),
            format!("last_write_ms\t{}", state.last_write_ms),
            format!(
                "last_session\t{}",
                state
                    .sessions
                    .last()
                    .map(format_session)
                    .unwrap_or_else(|| String::from("-"))
            ),
        ]),
        "ATTACH" | "attach" => {
            let Some(session) = parse_attach(rest) else {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_session");
            };
            state.sessions.retain(|existing| {
                !(existing.distro == session.distro && existing.session_id == session.session_id)
            });
            state.sessions.push(session);
            state.attach_count = state.attach_count.wrapping_add(1);
            state.last_attach_ms = sys::uptime_ms();
            write_status(state);
            ok_lines(alloc::vec![String::from("attached\ttrue")])
        }
        "WRITE" | "write" => {
            let Some((distro, bytes)) = parse_write(rest) else {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_write");
            };
            let delivered = deliver_console_bytes(state, distro, &bytes);
            canvas_for_distro(state, distro).write_bytes(&bytes);
            state.write_count = state.write_count.wrapping_add(1);
            state.canvas_update_count = state.canvas_update_count.wrapping_add(1);
            state.output_bytes = state.output_bytes.wrapping_add(bytes.len() as u64);
            state.last_write_ms = sys::uptime_ms();
            write_status(state);
            ok_lines(alloc::vec![
                format!("delivered\t{}", delivered),
                format!("bytes\t{}", bytes.len()),
            ])
        }
        "CANVAS" | "canvas" => {
            let distro = rest.trim();
            if !valid_token(distro) {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_canvas");
            }
            ok_lines(canvas_for_distro(state, distro).snapshot_lines())
        }
        "RESIZE" | "resize" => {
            let Some((distro, width, height)) = parse_resize(rest) else {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_resize");
            };
            canvas_for_distro(state, distro).resize(width, height);
            state.canvas_update_count = state.canvas_update_count.wrapping_add(1);
            write_status(state);
            ok_lines(alloc::vec![
                format!("width\t{}", width),
                format!("height\t{}", height),
            ])
        }
        "CLEAR" | "clear" => {
            let distro = rest.trim();
            state.sessions.retain(|session| session.distro != distro);
            state.canvases.retain(|canvas| canvas.distro != distro);
            write_status(state);
            ok_lines(alloc::vec![format!("sessions\t{}", state.sessions.len())])
        }
        _ => err("unknown_command"),
    }
}

fn write_status(state: &ConsoleState) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!(
        "health=ready\nsessions={}\ncanvases={}\nattach_count={}\nrejected_count={}\nwrite_count={}\ncanvas_update_count={}\noutput_bytes={}\nlast_attach_ms={}\nlast_write_ms={}\n",
        state.sessions.len(),
        state.canvases.len(),
        state.attach_count,
        state.rejected_count,
        state.write_count,
        state.canvas_update_count,
        state.output_bytes,
        state.last_attach_ms,
        state.last_write_ms
    );
    if let Some(session) = state.sessions.last() {
        text.push_str(&format!("last_session={}\n", format_session(session)));
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn parse_write(rest: &str) -> Option<(&str, Vec<u8>)> {
    let fields = split_tab_fields(rest);
    if fields.len() != 2 || !valid_token(fields[0]) {
        return None;
    }
    Some((fields[0], decode_hex(fields[1])?))
}

fn parse_resize(rest: &str) -> Option<(&str, usize, usize)> {
    let fields = split_tab_fields(rest);
    if fields.len() != 3 || !valid_token(fields[0]) {
        return None;
    }
    let width = parse_usize(fields[1])?;
    let height = parse_usize(fields[2])?;
    if !(1..=MAX_CANVAS_WIDTH).contains(&width) || !(1..=MAX_CANVAS_HEIGHT).contains(&height) {
        return None;
    }
    Some((fields[0], width, height))
}

fn canvas_for_distro<'a>(state: &'a mut ConsoleState, distro: &str) -> &'a mut TextCanvas {
    if let Some(index) = state
        .canvases
        .iter()
        .position(|canvas| canvas.distro == distro)
    {
        return &mut state.canvases[index];
    }
    state.canvases.push(TextCanvas::new(distro));
    let last = state.canvases.len().saturating_sub(1);
    &mut state.canvases[last]
}

fn deliver_console_bytes(state: &ConsoleState, distro: &str, bytes: &[u8]) -> u32 {
    let mut delivered = 0u32;
    for session in state
        .sessions
        .iter()
        .filter(|session| session.distro == distro)
    {
        if session.console_pipe == "-" {
            continue;
        }
        let pipe = ipc::pipe_open(&session.console_pipe);
        if pipe == 0 || pipe == u32::MAX {
            continue;
        }
        let written = ipc::pipe_write(pipe, bytes);
        ipc::pipe_close(pipe);
        if written != 0 && written != u32::MAX {
            delivered = delivered.wrapping_add(1);
        }
    }
    delivered
}

fn parse_attach(rest: &str) -> Option<ConsoleSession> {
    let fields = split_tab_fields(rest);
    if fields.len() >= 5 {
        let session = ConsoleSession {
            distro: String::from(fields[0]),
            session_id: String::from(fields[1]),
            mode: String::from(fields[2]),
            console_pipe: String::from(fields[3]),
            stdin_pipe: String::from(fields[4]),
        };
        return valid_session(&session).then_some(session);
    }
    if rest.is_empty() || rest.contains("..") {
        return None;
    }
    let session = ConsoleSession {
        distro: String::from("unknown"),
        session_id: String::from(rest),
        mode: String::from("unknown"),
        console_pipe: String::from("-"),
        stdin_pipe: String::from("-"),
    };
    valid_session(&session).then_some(session)
}

fn valid_session(session: &ConsoleSession) -> bool {
    valid_token(&session.distro)
        && valid_token(&session.session_id)
        && matches!(
            session.mode.as_str(),
            "agent" | "fallback-console" | "canvas" | "unknown"
        )
        && valid_pipe(&session.console_pipe)
        && valid_pipe(&session.stdin_pipe)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_'))
}

fn valid_pipe(value: &str) -> bool {
    value == "-"
        || (!value.is_empty()
            && !value.contains("..")
            && value.bytes().all(|b| {
                matches!(
                    b,
                    b'a'..=b'z'
                        | b'A'..=b'Z'
                        | b'0'..=b'9'
                        | b'-'
                        | b'_'
                        | b'.'
                        | b':'
                )
            }))
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn format_session(session: &ConsoleSession) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        session.distro, session.session_id, session.mode, session.console_pipe, session.stdin_pipe
    )
}

impl TextCanvas {
    fn new(distro: &str) -> Self {
        let width = DEFAULT_CANVAS_WIDTH;
        let height = DEFAULT_CANVAS_HEIGHT;
        Self {
            distro: String::from(distro),
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            cells: alloc::vec![b' '; width * height],
            ansi: AnsiState::default(),
        }
    }

    fn resize(&mut self, width: usize, height: usize) {
        let mut cells = alloc::vec![b' '; width * height];
        let copy_width = core::cmp::min(self.width, width);
        let copy_height = core::cmp::min(self.height, height);
        for row in 0..copy_height {
            let old_start = row * self.width;
            let new_start = row * width;
            cells[new_start..new_start + copy_width]
                .copy_from_slice(&self.cells[old_start..old_start + copy_width]);
        }
        self.width = width;
        self.height = height;
        self.cursor_x = core::cmp::min(self.cursor_x, width.saturating_sub(1));
        self.cursor_y = core::cmp::min(self.cursor_y, height.saturating_sub(1));
        self.cells = cells;
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_byte(byte);
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if self.ansi.in_escape || self.ansi.in_csi {
            self.write_ansi_byte(byte);
            return;
        }

        match byte {
            0x1b => self.ansi.start_escape(),
            b'\r' => self.cursor_x = 0,
            b'\n' => self.newline(),
            0x08 => self.cursor_x = self.cursor_x.saturating_sub(1),
            b'\t' => {
                let next = ((self.cursor_x / 8) + 1) * 8;
                while self.cursor_x < next {
                    self.put_printable(b' ');
                }
            }
            0x20..=0x7e => self.put_printable(byte),
            _ => {}
        }
    }

    fn write_ansi_byte(&mut self, byte: u8) {
        if self.ansi.in_escape && !self.ansi.in_csi {
            if byte == b'[' {
                self.ansi.start_csi();
            } else {
                self.ansi.reset();
            }
            return;
        }

        match byte {
            b'0'..=b'9' => self.ansi.push_digit(byte - b'0'),
            b';' => self.ansi.finish_param(),
            final_byte => {
                let params = self.ansi.params_with_current();
                self.apply_csi(final_byte, &params);
                self.ansi.reset();
            }
        }
    }

    fn apply_csi(&mut self, final_byte: u8, params: &[u16]) {
        match final_byte {
            b'H' | b'f' => {
                let row = params.first().copied().unwrap_or(1).max(1) as usize;
                let col = params.get(1).copied().unwrap_or(1).max(1) as usize;
                self.cursor_y = core::cmp::min(row - 1, self.height.saturating_sub(1));
                self.cursor_x = core::cmp::min(col - 1, self.width.saturating_sub(1));
            }
            b'J' => {
                if params.first().copied().unwrap_or(0) == 2 {
                    self.clear();
                }
            }
            b'K' => self.clear_line_from_cursor(),
            b'A' => self.cursor_y = self.cursor_y.saturating_sub(param_or_one(params, 0)),
            b'B' => {
                self.cursor_y = core::cmp::min(
                    self.cursor_y.saturating_add(param_or_one(params, 0)),
                    self.height.saturating_sub(1),
                );
            }
            b'C' => {
                self.cursor_x = core::cmp::min(
                    self.cursor_x.saturating_add(param_or_one(params, 0)),
                    self.width.saturating_sub(1),
                );
            }
            b'D' => self.cursor_x = self.cursor_x.saturating_sub(param_or_one(params, 0)),
            b'm' => {}
            _ => {}
        }
    }

    fn put_printable(&mut self, byte: u8) {
        if self.cursor_x >= self.width {
            self.newline();
        }
        let offset = self.cursor_y * self.width + self.cursor_x;
        if let Some(cell) = self.cells.get_mut(offset) {
            *cell = byte;
        }
        self.cursor_x += 1;
        if self.cursor_x >= self.width {
            self.newline();
        }
    }

    fn newline(&mut self) {
        self.cursor_x = 0;
        if self.cursor_y + 1 >= self.height {
            self.scroll_up();
        } else {
            self.cursor_y += 1;
        }
    }

    fn scroll_up(&mut self) {
        if self.height <= 1 {
            self.clear();
            return;
        }
        let row_len = self.width;
        let total = self.cells.len();
        self.cells.copy_within(row_len..total, 0);
        let last_row = (self.height - 1) * row_len;
        for cell in &mut self.cells[last_row..last_row + row_len] {
            *cell = b' ';
        }
    }

    fn clear(&mut self) {
        self.cells.fill(b' ');
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    fn clear_line_from_cursor(&mut self) {
        let start = self.cursor_y * self.width + self.cursor_x;
        let end = (self.cursor_y + 1) * self.width;
        for cell in &mut self.cells[start..end] {
            *cell = b' ';
        }
    }

    fn snapshot_lines(&self) -> Vec<String> {
        let mut lines = alloc::vec![
            String::from("kind\ttext"),
            format!("width\t{}", self.width),
            format!("height\t{}", self.height),
            format!("cursor_x\t{}", self.cursor_x),
            format!("cursor_y\t{}", self.cursor_y),
        ];
        for row in 0..self.height {
            let start = row * self.width;
            let end = start + self.width;
            lines.push(format!(
                "row\t{}\t{}",
                row,
                row_string(&self.cells[start..end])
            ));
        }
        lines
    }
}

impl Default for AnsiState {
    fn default() -> Self {
        Self {
            in_escape: false,
            in_csi: false,
            params: Vec::new(),
            current: 0,
            has_current: false,
        }
    }
}

impl AnsiState {
    fn start_escape(&mut self) {
        self.reset();
        self.in_escape = true;
    }

    fn start_csi(&mut self) {
        self.in_csi = true;
        self.params.clear();
        self.current = 0;
        self.has_current = false;
    }

    fn push_digit(&mut self, digit: u8) {
        self.current = self.current.saturating_mul(10).saturating_add(digit as u16);
        self.has_current = true;
    }

    fn finish_param(&mut self) {
        self.params
            .push(if self.has_current { self.current } else { 0 });
        self.current = 0;
        self.has_current = false;
    }

    fn params_with_current(&self) -> Vec<u16> {
        let mut params = self.params.clone();
        if self.has_current || params.is_empty() {
            params.push(if self.has_current { self.current } else { 0 });
        }
        params
    }

    fn reset(&mut self) {
        self.in_escape = false;
        self.in_csi = false;
        self.params.clear();
        self.current = 0;
        self.has_current = false;
    }
}

fn param_or_one(params: &[u16], index: usize) -> usize {
    params
        .get(index)
        .copied()
        .filter(|value| *value != 0)
        .unwrap_or(1) as usize
}

fn row_string(row: &[u8]) -> String {
    let mut end = row.len();
    while end > 0 && row[end - 1] == b' ' {
        end -= 1;
    }
    let mut out = String::new();
    for &byte in &row[..end] {
        out.push(if byte.is_ascii_graphic() || byte == b' ' {
            byte as char
        } else {
            ' '
        });
    }
    out
}

fn ok_lines(lines: Vec<String>) -> String {
    let mut out = format!("OK\t{}\n", lines.len());
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn err(code: &str) -> String {
    format!("ERR\t{}\t{}\n\n", code, code)
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim();
    if let Some(pos) = trimmed.find(char::is_whitespace) {
        (&trimmed[..pos], trimmed[pos + 1..].trim())
    } else {
        (trimmed, "")
    }
}

fn split_tab_fields(rest: &str) -> Vec<&str> {
    rest.split('\t').collect()
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn parse_usize(s: &str) -> Option<usize> {
    let mut value = 0usize;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(value)
}
