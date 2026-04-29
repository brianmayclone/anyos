//! Accessibility / UI-Automation server for libanyui.
//!
//! Each app that uses libanyui exposes a named pipe `"uiacc-<pid>"`.
//! The external `uictl` tool writes text commands to this pipe; responses
//! are written back to a reply pipe whose name the caller registers with
//! the HELLO command.
//!
//! # Protocol (text / UTF-8, newline-terminated lines)
//!
//! ## Commands  (uictl → app pipe `uiacc-<pid>`)
//! ```
//! HELLO <rsp_pipe_name>          register reply pipe; app responds READY
//! TREE                           dump full control tree (all CTRL lines + TREE_END)
//! PROPS <id>                     properties of one control
//! CLICK <id>                     fire click event on control
//! SET_TEXT <id> <text...>        set text content of a control
//! GET_TEXT <id>                  get text content  → TEXT <text>
//! SET_STATE <id> <value>         set state (checkbox 0/1, slider 0-100, …)
//! GET_STATE <id>                 get state value   → STATE <value>
//! SUBMIT <id>                    fire Enter/submit event on a control
//! TYPE_TEXT <text>               inject text as keystrokes into focused control
//! RESIZE <w> <h>                 resize window
//! MOVE <x> <y>                   move window on screen
//! FOCUS                          bring window to foreground
//! FIND_TEXT <text>               find controls whose text contains <text>
//! BYE                            close session (app closes rsp pipe)
//! ```
//!
//! ## Responses  (app → reply pipe)
//! ```
//! READY <win_title> <w> <h>              answer to HELLO
//! OK                                     generic success
//! ERR <reason>                           error
//! CTRL <id> <parent> <kind> <x> <y> <w> <h> <vis> <dis> <state> <text>
//! TREE_END                               end of TREE dump
//! TEXT <text>                            answer to GET_TEXT
//! STATE <value>                          answer to GET_STATE
//! MATCH <id> <kind> <text>               answer to FIND_TEXT (one per match)
//! MATCH_END                              end of FIND_TEXT results
//! ```
//!
//! Fields are separated by a single tab `\t`; `<text>` is the last field and
//! may contain spaces.  A tab inside text is escaped as `\t` (two chars).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::control::{self, ControlId, ControlKind};

// ── Named pipe wrappers (syscalls 45-49, matching anyos_std::ipc) ────────────

mod ipc {
    use libsyscall::{syscall1, syscall3};

    const SYS_PIPE_CREATE: u32 = 45;
    const SYS_PIPE_READ: u32 = 46;
    const SYS_PIPE_CLOSE: u32 = 47;
    const SYS_PIPE_WRITE: u32 = 48;
    const SYS_PIPE_OPEN: u32 = 49;

    pub fn pipe_create(name: &str) -> u32 {
        let mut buf = [0u8; 65];
        let len = name.len().min(64);
        buf[..len].copy_from_slice(&name.as_bytes()[..len]);
        buf[len] = 0;
        syscall1(SYS_PIPE_CREATE, buf.as_ptr() as u64) as u32
    }

    pub fn pipe_open(name: &str) -> u32 {
        let mut buf = [0u8; 65];
        let len = name.len().min(64);
        buf[..len].copy_from_slice(&name.as_bytes()[..len]);
        buf[len] = 0;
        syscall1(SYS_PIPE_OPEN, buf.as_ptr() as u64) as u32
    }

    pub fn pipe_read(pipe_id: u32, buf: &mut [u8]) -> u32 {
        syscall3(
            SYS_PIPE_READ,
            pipe_id as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        ) as u32
    }

    pub fn pipe_write(pipe_id: u32, data: &[u8]) -> u32 {
        syscall3(
            SYS_PIPE_WRITE,
            pipe_id as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        ) as u32
    }

    pub fn pipe_close(pipe_id: u32) -> u32 {
        syscall1(SYS_PIPE_CLOSE, pipe_id as u64) as u32
    }
}

// ── Pipe names ───────────────────────────────────────────────────────────────

pub fn req_pipe_name(pid: u32) -> String {
    format!("uiacc-{}", pid)
}

// ── State kept in AnyuiState ─────────────────────────────────────────────────

pub struct AccState {
    pub req_pipe_id: u32, // created at init, app reads commands from here
    pub rsp_pipe_id: u32, // 0 = no session open
}

impl AccState {
    pub fn new(pid: u32) -> Self {
        let name = req_pipe_name(pid);
        let id = ipc::pipe_create(&name);
        AccState {
            req_pipe_id: id,
            rsp_pipe_id: 0,
        }
    }
}

// ── Poll & dispatch ───────────────────────────────────────────────────────────

/// Called once per event-loop frame.  Reads all available command lines from
/// the accessibility request pipe and executes them.  Returns a list of
/// `(control_id, event_type)` pairs that the event loop should fire as
/// normal callbacks (e.g. CLICK → EVENT_CLICK, SUBMIT → EVENT_SUBMIT).
pub fn poll_and_handle(
    acc: &mut AccState,
    st: &mut crate::AnyuiState,
    pending_events: &mut Vec<(ControlId, u32)>,
) {
    if acc.req_pipe_id == 0 {
        return;
    }

    // Drain the pipe: read up to 4 KB per frame to avoid stalling.
    let mut raw = [0u8; 4096];
    let n = ipc::pipe_read(acc.req_pipe_id, &mut raw);
    if n == u32::MAX {
        // The pipe was destroyed (e.g. the kernel freed it when the last external
        // write-handle was closed by uictl).  Recreate it so subsequent uictl
        // calls can reconnect without restarting the app.
        let pid = libsyscall::get_tid();
        let name = req_pipe_name(pid);
        acc.req_pipe_id = ipc::pipe_create(&name);
        if acc.rsp_pipe_id != 0 {
            ipc::pipe_close(acc.rsp_pipe_id);
            acc.rsp_pipe_id = 0;
        }
        return;
    }
    if n == 0 {
        return;
    }

    let data = &raw[..n as usize];

    // The pipe may contain multiple newline-terminated commands.
    let mut start = 0usize;
    for i in 0..n as usize {
        if data[i] == b'\n' {
            let line = core::str::from_utf8(&data[start..i])
                .unwrap_or("")
                .trim_end_matches('\r');
            if !line.is_empty() {
                handle_line(acc, st, line, pending_events);
            }
            start = i + 1;
        }
    }
    // Partial line at end — ignore (next frame will get more)
}

fn handle_line(
    acc: &mut AccState,
    st: &mut crate::AnyuiState,
    line: &str,
    pending_events: &mut Vec<(ControlId, u32)>,
) {
    let (cmd, rest) = split_first(line);

    match cmd {
        "HELLO" => {
            // rest = reply pipe name
            let rsp_name = rest.trim();
            if !rsp_name.is_empty() {
                let id = ipc::pipe_open(rsp_name);
                if id == 0 {
                    // retry once — uictl may have just created the pipe
                    let id2 = ipc::pipe_open(rsp_name);
                    acc.rsp_pipe_id = id2;
                } else {
                    acc.rsp_pipe_id = id;
                }
            }
            // Send READY with window title + size
            let (title, w, h) = window_summary(st);
            send(acc, &format!("READY\t{}\t{}\t{}\n", title, w, h));
        }

        "TREE" => {
            dump_tree(acc, st);
        }

        "PROPS" => {
            if let Ok(id) = rest.trim().parse::<u32>() {
                if let Some(line) = ctrl_line(&st.controls, id, st) {
                    send(acc, &format!("{}\n", line));
                } else {
                    send(acc, "ERR\tnot found\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "CLICK" => {
            if let Ok(id) = rest.trim().parse::<u32>() {
                if control::find_idx(&st.controls, id).is_some() {
                    pending_events.push((id, crate::control::EVENT_CLICK));
                    send(acc, "OK\n");
                } else {
                    send(acc, "ERR\tnot found\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "SUBMIT" => {
            if let Ok(id) = rest.trim().parse::<u32>() {
                if control::find_idx(&st.controls, id).is_some() {
                    pending_events.push((id, crate::control::EVENT_SUBMIT));
                    send(acc, "OK\n");
                } else {
                    send(acc, "ERR\tnot found\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "SET_TEXT" => {
            // rest = "<id>\t<text...>"  or  "<id> <text...>"
            let (id_str, text) = split_first(rest);
            if let Ok(id) = id_str.parse::<u32>() {
                let text_bytes = unescape_tab(text).into_bytes();
                if set_text_by_id(&mut st.controls, id, text_bytes) {
                    send(acc, "OK\n");
                } else {
                    send(acc, "ERR\tnot found or not a text control\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "GET_TEXT" => {
            if let Ok(id) = rest.trim().parse::<u32>() {
                if let Some(text) = get_text_by_id(&st.controls, id) {
                    let escaped = escape_tab(&text);
                    send(acc, &format!("TEXT\t{}\n", escaped));
                } else {
                    send(acc, "ERR\tnot found or not a text control\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "SET_STATE" => {
            let (id_str, val_str) = split_first(rest);
            if let (Ok(id), Ok(val)) = (id_str.parse::<u32>(), val_str.trim().parse::<u32>()) {
                if let Some(idx) = control::find_idx(&st.controls, id) {
                    st.controls[idx].base_mut().state = val;
                    st.controls[idx].base_mut().mark_dirty();
                    send(acc, "OK\n");
                } else {
                    send(acc, "ERR\tnot found\n");
                }
            } else {
                send(acc, "ERR\tbad args\n");
            }
        }

        "GET_STATE" => {
            if let Ok(id) = rest.trim().parse::<u32>() {
                if let Some(idx) = control::find_idx(&st.controls, id) {
                    let val = st.controls[idx].base().state;
                    send(acc, &format!("STATE\t{}\n", val));
                } else {
                    send(acc, "ERR\tnot found\n");
                }
            } else {
                send(acc, "ERR\tbad id\n");
            }
        }

        "RESIZE" => {
            let (w_str, h_str) = split_first(rest);
            if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.trim().parse::<u32>()) {
                // Trigger a resize as if the user dragged the window chrome.
                // This sends EVT_RESIZE back to ourselves via the compositor.
                resize_window(st, w, h);
                send(acc, "OK\n");
            } else {
                send(acc, "ERR\tbad args\n");
            }
        }

        "MOVE" => {
            let (x_str, y_str) = split_first(rest);
            if let (Ok(x), Ok(y)) = (x_str.parse::<i32>(), y_str.trim().parse::<i32>()) {
                move_window(st, x, y);
                send(acc, "OK\n");
            } else {
                send(acc, "ERR\tbad args\n");
            }
        }

        "FOCUS" => {
            focus_window(st);
            send(acc, "OK\n");
        }

        "FIND_TEXT" => {
            let needle = rest.trim();
            find_by_text(acc, st, needle);
        }

        "TYPE_TEXT" => {
            // Inject each character of <text> as a key-down event into the
            // currently focused control.  Escape sequences in the text:
            //   \n  →  KEY_ENTER (0x100)
            //   \b  →  KEY_BACKSPACE (0x101)
            //   \t  →  KEY_TAB (0x102)
            //   \\  →  literal backslash
            // Regular bytes are sent as char_code = byte, keycode = 0.
            type_text_into_focused(st, rest, pending_events);
            send(acc, "OK\n");
        }

        "BYE" => {
            if acc.rsp_pipe_id != 0 {
                ipc::pipe_close(acc.rsp_pipe_id);
                acc.rsp_pipe_id = 0;
            }
        }

        _ => {
            send(acc, &format!("ERR\tunknown command: {}\n", cmd));
        }
    }
}

// ── Tree dump ─────────────────────────────────────────────────────────────────

fn dump_tree(acc: &mut AccState, st: &crate::AnyuiState) {
    // Walk all controls in insertion order (already depth-first by construction).
    let ids: Vec<ControlId> = st.controls.iter().map(|c| c.id()).collect();
    for id in ids {
        // Skip framework-internal controls (tooltips, context menu popups).
        if let Some(idx) = control::find_idx(&st.controls, id) {
            let kind = st.controls[idx].kind();
            if kind == ControlKind::Tooltip || kind == ControlKind::ContextMenu {
                continue;
            }
        }
        if let Some(line) = ctrl_line(&st.controls, id, st) {
            send(acc, &format!("{}\n", line));
        }
    }
    send(acc, "TREE_END\n");
}

/// Build one `CTRL\t…` line for a control.
fn ctrl_line(
    controls: &[alloc::boxed::Box<dyn crate::control::Control>],
    id: ControlId,
    st: &crate::AnyuiState,
) -> Option<String> {
    let idx = control::find_idx(controls, id)?;
    let base = controls[idx].base();
    let kind = controls[idx].kind();
    let (ax, ay) = control::abs_position(controls, id);
    let text = get_text_by_id(controls, id).unwrap_or_default();
    let escaped = escape_tab(&text);
    let vis = if base.visible { 1 } else { 0 };
    let dis = if base.disabled { 1 } else { 0 };
    let _ = st; // reserved for future use
    Some(format!(
        "CTRL\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        id,
        base.parent,
        kind_name(kind),
        ax,
        ay,
        base.w,
        base.h,
        vis,
        dis,
        base.state,
        escaped,
    ))
}

// ── Find by text ──────────────────────────────────────────────────────────────

fn find_by_text(acc: &mut AccState, st: &crate::AnyuiState, needle: &str) {
    let needle_lower = to_lowercase(needle);
    let ids: Vec<ControlId> = st.controls.iter().map(|c| c.id()).collect();
    for id in ids {
        let text = get_text_by_id(&st.controls, id).unwrap_or_default();
        if contains_ignore_case(&text, &needle_lower) {
            if let Some(idx) = control::find_idx(&st.controls, id) {
                let kind = st.controls[idx].kind();
                let escaped = escape_tab(&text);
                send(
                    acc,
                    &format!("MATCH\t{}\t{}\t{}\n", id, kind_name(kind), escaped),
                );
            }
        }
    }
    send(acc, "MATCH_END\n");
}

// ── Window actions ────────────────────────────────────────────────────────────

fn window_summary(st: &crate::AnyuiState) -> (String, u32, u32) {
    // Find the first Window control.
    if let Some(c) = st.controls.iter().find(|c| c.kind() == ControlKind::Window) {
        let b = c.base();
        let text = get_text_by_id(&st.controls, c.id()).unwrap_or_default();
        return (escape_tab(&text), b.w, b.h);
    }
    (String::from(""), 0, 0)
}

fn resize_window(st: &mut crate::AnyuiState, w: u32, h: u32) {
    // Delegate to the event-loop resize helper (same path as EVT_RESIZE).
    if let Some(cw) = st.comp_windows.first() {
        let window_id = cw.window_id;
        crate::event_loop::external_resize(st, window_id, w, h);
    }
}

fn move_window(st: &mut crate::AnyuiState, x: i32, y: i32) {
    if let Some(cw) = st.comp_windows.first() {
        let window_id = cw.window_id;
        let channel_id = st.channel_id;
        crate::compositor::move_window(channel_id, window_id, x, y);
    }
}

fn focus_window(st: &mut crate::AnyuiState) {
    let own_pid = libsyscall::get_tid();
    let channel_id = st.channel_id;
    crate::compositor::focus_by_tid(channel_id, own_pid);
}

// ── Text helpers ─────────────────────────────────────────────────────────────

fn get_text_by_id(
    controls: &[alloc::boxed::Box<dyn crate::control::Control>],
    id: ControlId,
) -> Option<String> {
    let idx = control::find_idx(controls, id)?;
    let tb = controls[idx].text_base()?;
    Some(String::from_utf8_lossy(&tb.text).into_owned())
}

fn set_text_by_id(
    controls: &mut [alloc::boxed::Box<dyn crate::control::Control>],
    id: ControlId,
    text: Vec<u8>,
) -> bool {
    if let Some(idx) = control::find_idx(controls, id) {
        if let Some(tb) = controls[idx].text_base_mut() {
            tb.text = text;
            controls[idx].base_mut().mark_dirty();
            return true;
        }
    }
    false
}

// ── String utilities ──────────────────────────────────────────────────────────

/// Split `"CMD REST"` → `("CMD", "REST")`.  Separator is first space or tab.
fn split_first(s: &str) -> (&str, &str) {
    for (i, c) in s.char_indices() {
        if c == ' ' || c == '\t' {
            return (&s[..i], s[i + 1..].trim_start_matches(' '));
        }
    }
    (s, "")
}

/// Escape a literal tab in text as `\\t` for transport.
fn escape_tab(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\t' {
            out.push('\\');
            out.push('t');
        } else if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Unescape `\\t` back to a literal tab.
fn unescape_tab(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars().peekable();
    while let Some(ch) = iter.next() {
        if ch == '\\' {
            if iter.peek() == Some(&'t') {
                iter.next();
                out.push('\t');
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn to_lowercase(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c >= 'A' && c <= 'Z' {
                (c as u8 + 32) as char
            } else {
                c
            }
        })
        .collect()
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let h = to_lowercase(haystack);
    h.contains(&needle_lower[..])
}

/// Human-readable name for a ControlKind.
fn kind_name(k: ControlKind) -> &'static str {
    match k {
        ControlKind::Window => "Window",
        ControlKind::View => "View",
        ControlKind::Label => "Label",
        ControlKind::LinkLabel => "LinkLabel",
        ControlKind::Button => "Button",
        ControlKind::TextField => "TextField",
        ControlKind::Toggle => "Toggle",
        ControlKind::Checkbox => "Checkbox",
        ControlKind::Slider => "Slider",
        ControlKind::RadioButton => "RadioButton",
        ControlKind::ProgressBar => "ProgressBar",
        ControlKind::Stepper => "Stepper",
        ControlKind::SegmentedControl => "SegmentedControl",
        ControlKind::TableView => "TableView",
        ControlKind::ScrollView => "ScrollView",
        ControlKind::Sidebar => "Sidebar",
        ControlKind::NavigationBar => "NavigationBar",
        ControlKind::TabBar => "TabBar",
        ControlKind::Toolbar => "Toolbar",
        ControlKind::Card => "Card",
        ControlKind::GroupBox => "GroupBox",
        ControlKind::SplitView => "SplitView",
        ControlKind::Divider => "Divider",
        ControlKind::Alert => "Alert",
        ControlKind::ContextMenu => "ContextMenu",
        ControlKind::Tooltip => "Tooltip",
        ControlKind::ImageView => "ImageView",
        ControlKind::StatusIndicator => "StatusIndicator",
        ControlKind::ColorWell => "ColorWell",
        ControlKind::SearchField => "SearchField",
        ControlKind::TextArea => "TextArea",
        ControlKind::IconButton => "IconButton",
        ControlKind::Badge => "Badge",
        ControlKind::Tag => "Tag",
        ControlKind::StackPanel => "StackPanel",
        ControlKind::FlowPanel => "FlowPanel",
        ControlKind::TableLayout => "TableLayout",
        ControlKind::Canvas => "Canvas",
        ControlKind::Expander => "Expander",
        ControlKind::DataGrid => "DataGrid",
        ControlKind::TextEditor => "TextEditor",
        ControlKind::TreeView => "TreeView",
        ControlKind::RadioGroup => "RadioGroup",
        ControlKind::DropDown => "DropDown",
        ControlKind::ComboBox => "ComboBox",
        ControlKind::AutoCompleteTextField => "AutoCompleteTextField",
        ControlKind::AntiAliasFilterContainer => "AntiAliasFilterContainer",
        ControlKind::Spinner => "Spinner",
        ControlKind::PlainButton => "PlainButton",
        ControlKind::DateTimePicker => "DateTimePicker",
        ControlKind::ListBox => "ListBox",
    }
}

// ── Type-text injection ───────────────────────────────────────────────────────

/// Inject `text` as individual key-down events into `st.focused`.
///
/// Escape sequences: `\n` → KEY_ENTER, `\b` → KEY_BACKSPACE, `\t` → KEY_TAB,
/// `\\` → literal `\`.  All other bytes are sent as regular character input
/// (keycode = 0, char_code = byte value).
fn type_text_into_focused(
    st: &mut crate::AnyuiState,
    text: &str,
    pending_events: &mut Vec<(ControlId, u32)>,
) {
    let focus_id = match st.focused {
        Some(id) => id,
        None => return,
    };
    let idx = match crate::control::find_idx(&st.controls, focus_id) {
        Some(i) => i,
        None => return,
    };

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let (keycode, char_code) = if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'n' => (crate::control::KEY_ENTER, 0u32),
                b'b' => (crate::control::KEY_BACKSPACE, 0u32),
                b't' => (crate::control::KEY_TAB, 0u32),
                b'\\' => (0u32, b'\\' as u32),
                other => (0u32, other as u32),
            }
        } else {
            (0u32, bytes[i] as u32)
        };
        i += 1;

        let resp = st.controls[idx].handle_key_down(keycode, char_code, 0);
        st.controls[idx].base_mut().mark_dirty();

        if resp.consumed {
            pending_events.push((focus_id, crate::control::EVENT_KEY));
        }
        if resp.fire_change {
            pending_events.push((focus_id, crate::control::EVENT_CHANGE));
        }
        if resp.fire_click {
            pending_events.push((focus_id, crate::control::EVENT_CLICK));
        }
        if resp.fire_submit {
            pending_events.push((focus_id, crate::control::EVENT_SUBMIT));
        }
    }
}

// ── Send helper ───────────────────────────────────────────────────────────────

fn send(acc: &AccState, msg: &str) {
    if acc.rsp_pipe_id == 0 {
        return;
    }
    ipc::pipe_write(acc.rsp_pipe_id, msg.as_bytes());
}
