// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Developer Tools window — Firefox-style multi-tab inspector.
//!
//! Tabs (left to right):
//!   0 — "Auswählen" (element picker toggle, no own panel)
//!   1 — Inspektor   (DOM tree + computed styles)
//!   2 — Konsole     (JS console output + REPL eval)
//!   3 — Debugger    (placeholder, see C3 plan)
//!   4 — Netzwerkanalyse (request log)

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libanyui_client as ui_lib;
use ui_lib::Widget;

use crate::http::{RequestTiming, Url};

const COLOR_BG: u32 = 0xFF1E1E1E;
const COLOR_PANE: u32 = 0xFF252526;
const COLOR_TEXT: u32 = 0xFFCCCCCC;
const COLOR_DIM: u32 = 0xFF888888;
const COLOR_FILTER_ACTIVE: u32 = 0xFF0E639C;
const COLOR_FILTER_INACTIVE: u32 = 0xFF333333;

const NET_COLS: u32 = 20;

const STATUS_OK: u32 = 0xFF6BB36B; // green
const STATUS_REDIRECT: u32 = 0xFFD7BA7D; // yellow
const STATUS_ERROR: u32 = 0xFFE06C75; // red
const STATUS_PENDING: u32 = 0xFF888888; // dim

/// Status of a network request entry. Includes pending / error states so the
/// row colour can be picked without parsing strings later.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NetStatus {
    Pending,
    Ok(u16),
    Redirect(u16),
    ClientError(u16),
    ServerError(u16),
    Blocked,
}

impl NetStatus {
    pub fn from_http(code: u32) -> Self {
        match code {
            0 => Self::Blocked,
            100..=199 => Self::Ok(code as u16),
            200..=299 => Self::Ok(code as u16),
            300..=399 => Self::Redirect(code as u16),
            400..=499 => Self::ClientError(code as u16),
            500..=599 => Self::ServerError(code as u16),
            _ => Self::Blocked,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Pending => String::from("…"),
            Self::Ok(c) | Self::Redirect(c) | Self::ClientError(c) | Self::ServerError(c) => {
                format!("{}", c)
            }
            Self::Blocked => String::from("⊘"),
        }
    }
    pub fn color(&self) -> u32 {
        match self {
            Self::Pending => STATUS_PENDING,
            Self::Ok(_) => STATUS_OK,
            Self::Redirect(_) => STATUS_REDIRECT,
            Self::ClientError(_) | Self::ServerError(_) | Self::Blocked => STATUS_ERROR,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct NetPhases {
    pub queue_ms: u32,
    pub dns_ms: u32,
    pub connect_ms: u32,
    pub tls_ms: u32,
    pub send_ms: u32,
    pub wait_ms: u32,
    pub body_ms: u32,
    pub decode_ms: u32,
    pub enqueue_ms: u32,
    pub ui_ms: u32,
    pub reused_connection: bool,
}

/// One row of the network panel.
pub struct NetEntry {
    pub id: u32,
    pub status: NetStatus,
    pub method: String,
    pub host: String,
    pub file: String, // basename (after last '/')
    pub path: String, // full path (used for matching)
    pub initiator: String,
    pub kind: String,     // "html" | "css" | "js" | "img" | "font" | "xhr" | …
    pub size: u64,        // body length
    pub transferred: u64, // wire bytes (currently == size)
    pub start_ms: u32,
    pub end_ms: u32,
    pub phases: NetPhases,
}

pub struct DevTools {
    pub win: ui_lib::Window,
    pub open: bool,

    pub tab_bar: ui_lib::TabBar,
    /// Top-level panel views in tab order. Visibility is toggled in
    /// `on_active_changed` (we cannot use `tab_bar.connect_panels` because the
    /// underlying libanyui `on_change` callback only supports a single
    /// registration and we already need it for picker-mode handling).
    pub panels: [u32; 5],

    /// Maps a TreeView index to the libwebview DOM node id.
    pub dom_tree: ui_lib::TreeView,
    pub style_pane: ui_lib::TextArea,
    pub tree_to_dom: Vec<usize>,

    pub console_output: ui_lib::TextArea,
    pub console_input: ui_lib::TextField,

    // ── Netzwerkanalyse ─────────────────────────────────────────────────────
    pub net_grid: ui_lib::DataGrid,
    pub net_entries: Vec<NetEntry>,
    pub net_search: ui_lib::TextField,
    pub net_filter_kind: String, // "" | html | css | js | xhr | font | img | media | ws | other
    pub net_paused: bool,
    pub net_pause_btn: ui_lib::IconButton,
    pub net_clear_btn: ui_lib::IconButton,
    pub net_block_btn: ui_lib::IconButton,
    pub net_disable_cache: ui_lib::Checkbox,
    pub net_filter_btns: Vec<(String, ui_lib::Button)>, // kind id → button
    pub net_status_label: ui_lib::Label,
    pub nav_start_ms: u32, // for the timeline column
    pub net_next_request_id: u32,

    /// `true` while the user is in element-picker mode — set by the toolbar
    /// button and consumed on the next click in the webview.
    pub picker_active: bool,
}

pub fn build() -> DevTools {
    let win = ui_lib::Window::new("Werkzeuge für Webentwickler", 9999, 9999, 1100, 560);
    win.set_visible(false);

    let tab_bar = ui_lib::TabBar::new("Auswählen|Inspektor|Konsole|Debugger|Netzwerkanalyse");
    tab_bar.set_dock(ui_lib::DOCK_TOP);
    tab_bar.set_size(0, 30);
    win.add(&tab_bar);

    // ── Picker placeholder panel ────────────────────────────────────────────
    let picker_panel = ui_lib::View::new();
    picker_panel.set_color(COLOR_BG);
    picker_panel.set_padding(16, 16, 16, 16);
    let picker_lbl = ui_lib::Label::new(
        "Element-Auswahl aktiv — klicke ein Element im Webview an, um es im Inspektor zu öffnen. Erneuter Klick auf den Tab beendet den Modus.",
    );
    picker_lbl.set_dock(ui_lib::DOCK_FILL);
    picker_lbl.set_text_color(COLOR_TEXT);
    picker_panel.add(&picker_lbl);

    // ── Inspector panel ─────────────────────────────────────────────────────
    let insp_panel = ui_lib::View::new();
    insp_panel.set_color(COLOR_BG);

    let dom_tree = ui_lib::TreeView::new(450, 460);
    dom_tree.set_dock(ui_lib::DOCK_LEFT);
    dom_tree.set_size(450, 460);
    dom_tree.set_indent_width(14);
    dom_tree.set_row_height(20);
    insp_panel.add(&dom_tree);

    let style_pane = ui_lib::TextArea::new();
    style_pane.set_dock(ui_lib::DOCK_FILL);
    style_pane.set_read_only(true);
    style_pane.set_color(COLOR_PANE);
    style_pane.set_text_color(COLOR_TEXT);
    style_pane.set_padding(8, 8, 8, 8);
    style_pane.set_text("(kein Element ausgewählt)");
    insp_panel.add(&style_pane);

    // ── Console panel ───────────────────────────────────────────────────────
    let console_panel = ui_lib::View::new();
    console_panel.set_color(COLOR_BG);

    let input_bar = ui_lib::View::new();
    input_bar.set_dock(ui_lib::DOCK_BOTTOM);
    input_bar.set_size(0, 32);
    input_bar.set_color(COLOR_PANE);
    input_bar.set_padding(4, 4, 4, 4);
    let console_input = ui_lib::TextField::new();
    console_input.set_dock(ui_lib::DOCK_FILL);
    console_input.set_placeholder(">  JavaScript-Ausdruck — Enter zum Auswerten");
    input_bar.add(&console_input);
    console_panel.add(&input_bar);

    let console_output = ui_lib::TextArea::new();
    console_output.set_dock(ui_lib::DOCK_FILL);
    console_output.set_read_only(true);
    console_output.set_color(COLOR_PANE);
    console_output.set_text_color(COLOR_TEXT);
    console_output.set_padding(8, 8, 8, 8);
    console_panel.add(&console_output);

    // ── Debugger placeholder ────────────────────────────────────────────────
    let dbg_panel = ui_lib::View::new();
    dbg_panel.set_color(COLOR_BG);
    dbg_panel.set_padding(16, 16, 16, 16);
    let dbg_lbl = ui_lib::Label::new(
        "Debugger — folgt in einer späteren Phase. Geplant: Source-Liste, Pause, Schrittweise-Ausführung, Breakpoints, Watch-Expressions (benötigt libjs-Hooks).",
    );
    dbg_lbl.set_dock(ui_lib::DOCK_FILL);
    dbg_lbl.set_text_color(COLOR_DIM);
    dbg_panel.add(&dbg_lbl);

    // ── Network panel ───────────────────────────────────────────────────────
    let net_panel = ui_lib::View::new();
    net_panel.set_color(COLOR_BG);

    // Toolbar row: clear, search, pause, block, cache toggle.
    let toolbar = ui_lib::View::new();
    toolbar.set_dock(ui_lib::DOCK_TOP);
    toolbar.set_size(0, 32);
    toolbar.set_color(COLOR_PANE);
    toolbar.set_padding(4, 4, 4, 4);

    let net_clear_btn = ui_lib::IconButton::new("");
    net_clear_btn.set_dock(ui_lib::DOCK_LEFT);
    net_clear_btn.set_size(28, 24);
    net_clear_btn.set_system_icon("trash", ui_lib::IconType::Outline, COLOR_TEXT, 16);
    net_clear_btn.set_tooltip("Liste leeren");
    toolbar.add(&net_clear_btn);

    let net_search = ui_lib::TextField::new();
    net_search.set_dock(ui_lib::DOCK_LEFT);
    net_search.set_size(280, 24);
    net_search.set_placeholder("Adressen durchsuchen");
    toolbar.add(&net_search);

    let net_pause_btn = ui_lib::IconButton::new("");
    net_pause_btn.set_dock(ui_lib::DOCK_LEFT);
    net_pause_btn.set_size(28, 24);
    net_pause_btn.set_system_icon("pause", ui_lib::IconType::Outline, COLOR_TEXT, 16);
    net_pause_btn.set_tooltip("Aufzeichnung pausieren");
    toolbar.add(&net_pause_btn);

    let net_block_btn = ui_lib::IconButton::new("");
    net_block_btn.set_dock(ui_lib::DOCK_LEFT);
    net_block_btn.set_size(28, 24);
    net_block_btn.set_system_icon("block", ui_lib::IconType::Outline, COLOR_TEXT, 16);
    net_block_btn.set_tooltip("Blockierte Anfragen werden in der Liste markiert");
    toolbar.add(&net_block_btn);

    let net_disable_cache = ui_lib::Checkbox::new("Cache deaktivieren");
    net_disable_cache.set_dock(ui_lib::DOCK_RIGHT);
    net_disable_cache.set_size(160, 24);
    toolbar.add(&net_disable_cache);

    net_panel.add(&toolbar);

    // Filter row: Alles, HTML, CSS, JS, XHR, Schriften, Grafiken, Medien, WebSockets, Sonstiges.
    let filter_row = ui_lib::View::new();
    filter_row.set_dock(ui_lib::DOCK_TOP);
    filter_row.set_size(0, 30);
    filter_row.set_color(COLOR_PANE);
    filter_row.set_padding(4, 2, 4, 2);

    let mut net_filter_btns: Vec<(String, ui_lib::Button)> = Vec::new();
    let kinds: &[(&str, &str)] = &[
        ("", "Alles"),
        ("html", "HTML"),
        ("css", "CSS"),
        ("js", "JS"),
        ("xhr", "XHR"),
        ("font", "Schriften"),
        ("img", "Grafiken"),
        ("media", "Medien"),
        ("ws", "WebSockets"),
        ("other", "Sonstiges"),
    ];
    for (id, label) in kinds {
        let b = ui_lib::Button::new(label);
        b.set_dock(ui_lib::DOCK_LEFT);
        let w = (label.len() as u32 * 9 + 24).max(50);
        b.set_size(w, 24);
        b.set_text_color(if id.is_empty() { COLOR_TEXT } else { COLOR_DIM });
        b.set_color(if id.is_empty() {
            COLOR_FILTER_ACTIVE
        } else {
            COLOR_FILTER_INACTIVE
        });
        filter_row.add(&b);
        net_filter_btns.push((id.to_string(), b));
    }
    net_panel.add(&filter_row);

    // Status bar at the bottom: "N Anfragen, X übertragen, beendet: Y s".
    let net_status_label = ui_lib::Label::new("");
    net_status_label.set_dock(ui_lib::DOCK_BOTTOM);
    net_status_label.set_size(0, 22);
    net_status_label.set_color(COLOR_PANE);
    net_status_label.set_text_color(COLOR_DIM);
    net_status_label.set_font_size(11);
    net_status_label.set_padding(8, 4, 0, 0);
    net_panel.add(&net_status_label);

    // Grid takes the rest.
    let net_grid = ui_lib::DataGrid::new(1100, 460);
    net_grid.set_dock(ui_lib::DOCK_FILL);
    net_grid.set_columns(&[
        ui_lib::ColumnDef::new("Status")
            .width(70)
            .align(ui_lib::ALIGN_CENTER),
        ui_lib::ColumnDef::new("Methode").width(70),
        ui_lib::ColumnDef::new("Host").width(180),
        ui_lib::ColumnDef::new("Datei").width(280),
        ui_lib::ColumnDef::new("Initiator").width(160),
        ui_lib::ColumnDef::new("Typ").width(70),
        ui_lib::ColumnDef::new("Übertragen")
            .width(90)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Größe")
            .width(80)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Dauer (ms)")
            .width(90)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Queue")
            .width(70)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("DNS")
            .width(60)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("TCP")
            .width(60)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("TLS")
            .width(60)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Senden")
            .width(70)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Warten")
            .width(75)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Body")
            .width(65)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Dec")
            .width(55)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Enq")
            .width(55)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("UI")
            .width(50)
            .align(ui_lib::ALIGN_RIGHT)
            .numeric(),
        ui_lib::ColumnDef::new("Conn").width(60),
    ]);
    net_panel.add(&net_grid);

    picker_panel.set_dock(ui_lib::DOCK_FILL);
    insp_panel.set_dock(ui_lib::DOCK_FILL);
    console_panel.set_dock(ui_lib::DOCK_FILL);
    dbg_panel.set_dock(ui_lib::DOCK_FILL);
    net_panel.set_dock(ui_lib::DOCK_FILL);

    win.add(&picker_panel);
    win.add(&insp_panel);
    win.add(&console_panel);
    win.add(&dbg_panel);
    win.add(&net_panel);

    // Initial visibility: only the first tab's panel is visible. Switching is
    // handled in main.rs via `tab_bar.on_active_changed`.
    insp_panel.set_visible(false);
    console_panel.set_visible(false);
    dbg_panel.set_visible(false);
    net_panel.set_visible(false);

    let panels = [
        picker_panel.id(),
        insp_panel.id(),
        console_panel.id(),
        dbg_panel.id(),
        net_panel.id(),
    ];

    DevTools {
        win,
        open: false,
        tab_bar,
        panels,
        dom_tree,
        style_pane,
        tree_to_dom: Vec::new(),
        console_output,
        console_input,
        net_grid,
        net_entries: Vec::new(),
        net_search,
        net_filter_kind: String::new(),
        net_paused: false,
        net_pause_btn,
        net_clear_btn,
        net_block_btn,
        net_disable_cache,
        net_filter_btns,
        net_status_label,
        nav_start_ms: 0,
        net_next_request_id: 1,
        picker_active: false,
    }
}

/// Clear every panel — called from `tab::navigate*` so each page load gets a
/// fresh slate, regardless of whether the DevTools window is currently
/// visible. Network recording starts from zero on the new navigation.
pub fn reset_for_navigation() {
    let st = crate::state();
    st.devtools.net_entries.clear();
    st.devtools.net_next_request_id = 1;
    st.devtools.nav_start_ms = anyos_std::sys::uptime_ms();
    if st.devtools.open {
        st.devtools.net_grid.set_row_count(0);
        st.devtools.net_grid.set_data_raw(&[]);
        st.devtools.net_grid.set_cell_colors(&[]);
        st.devtools.dom_tree.clear();
        st.devtools.tree_to_dom.clear();
        st.devtools.style_pane.set_text("(kein Element ausgewählt)");
        st.devtools.console_output.set_text("");
        update_net_status_label();
    }
}

/// Toggle the DevTools window open/closed; refresh on open.
pub fn toggle() {
    let st = crate::state();
    st.devtools.open = !st.devtools.open;
    if st.devtools.open {
        st.devtools.win.move_to(120, 120);
        refresh_all();
    }
    st.devtools.win.set_visible(st.devtools.open);
}

/// Refresh every panel — called when the window opens or after navigation.
pub fn refresh_all() {
    refresh_inspector();
    refresh_console();
    refresh_network();
}

/// Rebuild the DOM tree for the active tab.
pub fn refresh_inspector() {
    let st = crate::state();
    if !st.devtools.open {
        return;
    }
    st.devtools.dom_tree.clear();
    st.devtools.tree_to_dom.clear();

    if st.active_tab >= st.tabs.len() {
        return;
    }
    let dom = match st.tabs[st.active_tab].webview.dom() {
        Some(d) => d,
        None => return,
    };

    fn walk(
        dom: &libwebview::dom::Dom,
        node_id: usize,
        parent_tree: u32,
        tree: &ui_lib::TreeView,
        map: &mut Vec<usize>,
        depth: u32,
    ) {
        if node_id >= dom.nodes.len() || depth > 64 {
            return;
        }
        let node = &dom.nodes[node_id];
        let label = match &node.node_type {
            libwebview::dom::NodeType::Element { tag, attrs } => {
                let mut s = format!("<{}", tag.tag_name());
                for a in attrs.iter().take(3) {
                    s.push(' ');
                    s.push_str(&a.name);
                    s.push_str("=\"");
                    let val: String = a.value.chars().take(40).collect();
                    s.push_str(&val);
                    s.push('"');
                }
                if attrs.len() > 3 {
                    s.push_str(" …");
                }
                s.push('>');
                s
            }
            libwebview::dom::NodeType::Text(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return;
                }
                let preview: String = trimmed.chars().take(60).collect();
                format!("\"{}\"", preview)
            }
        };
        let idx = if parent_tree == u32::MAX {
            tree.add_root(&label)
        } else {
            tree.add_child(parent_tree, &label)
        };
        map.push(node_id);
        debug_assert_eq!(map.len() as u32, idx + 1);
        if depth < 3 {
            tree.set_expanded(idx, true);
        }
        for &child in &node.children {
            walk(dom, child, idx, tree, map, depth + 1);
        }
    }

    walk(
        dom,
        0,
        u32::MAX,
        &st.devtools.dom_tree,
        &mut st.devtools.tree_to_dom,
        0,
    );
}

/// Push the active webview's JS console buffer into the console output area.
pub fn refresh_console() {
    let st = crate::state();
    if !st.devtools.open {
        return;
    }
    if st.active_tab >= st.tabs.len() {
        return;
    }
    let lines = st.tabs[st.active_tab].webview.js_console();
    let mut text = String::new();
    let start = if lines.len() > 500 {
        lines.len() - 500
    } else {
        0
    };
    for line in &lines[start..] {
        text.push_str(line);
        text.push('\n');
    }
    st.devtools.console_output.set_text(&text);
}

/// Repaint the network grid from `net_entries` honouring the active filter.
pub fn refresh_network() {
    let st = crate::state();
    if !st.devtools.open {
        return;
    }
    let filter_kind = st.devtools.net_filter_kind.clone();
    let mut search = [0u8; 256];
    let n = st.devtools.net_search.get_text(&mut search);
    let search_str = core::str::from_utf8(&search[..n as usize])
        .unwrap_or("")
        .to_ascii_lowercase();
    let search_active = !search_str.is_empty();

    let mut buf: Vec<u8> = Vec::new();
    let mut cell_colors: Vec<u32> = Vec::new();
    let mut shown = 0u32;

    for e in st.devtools.net_entries.iter() {
        if !filter_kind.is_empty() && !kind_matches(&filter_kind, &e.kind) {
            continue;
        }
        if search_active {
            let host_l = e.host.to_ascii_lowercase();
            let path_l = e.path.to_ascii_lowercase();
            if !host_l.contains(&search_str) && !path_l.contains(&search_str) {
                continue;
            }
        }
        if shown > 0 {
            buf.push(0x1E);
        }
        let dur_ms = e.end_ms.wrapping_sub(e.start_ms);
        let cells: [String; 20] = [
            e.status.label(),
            e.method.clone(),
            e.host.clone(),
            e.file.clone(),
            e.initiator.clone(),
            e.kind.clone(),
            humanize_size(e.transferred),
            humanize_size(e.size),
            format!("{}", dur_ms),
            phase_label(e.phases.queue_ms),
            phase_label(e.phases.dns_ms),
            phase_label(e.phases.connect_ms),
            phase_label(e.phases.tls_ms),
            phase_label(e.phases.send_ms),
            phase_label(e.phases.wait_ms),
            phase_label(e.phases.body_ms),
            phase_label(e.phases.decode_ms),
            phase_label(e.phases.enqueue_ms),
            phase_label(e.phases.ui_ms),
            if e.phases.reused_connection {
                String::from("reuse")
            } else {
                String::from("neu")
            },
        ];
        for (j, c) in cells.iter().enumerate() {
            if j > 0 {
                buf.push(0x1F);
            }
            buf.extend_from_slice(c.as_bytes());
        }
        // Per-cell text colours: only the Status cell gets a coloured tint;
        // other cells default to 0 (theme default).
        cell_colors.push(e.status.color());
        for _ in 1..NET_COLS {
            cell_colors.push(0);
        }
        shown += 1;
    }
    st.devtools.net_grid.set_row_count(shown);
    st.devtools.net_grid.set_data_raw(&buf);
    st.devtools.net_grid.set_cell_colors(&cell_colors);
    update_net_status_label();
}

fn kind_matches(filter: &str, kind: &str) -> bool {
    if filter == kind {
        return true;
    }
    // Group "media" → audio/video/image; "img" → img only; "other" → anything
    // not matching one of the well-known groups.
    match filter {
        "media" => matches!(kind, "audio" | "video" | "media"),
        "other" => !matches!(
            kind,
            "html" | "css" | "js" | "xhr" | "font" | "img" | "audio" | "video" | "media" | "ws"
        ),
        _ => false,
    }
}

fn update_net_status_label() {
    let st = crate::state();
    let total = st.devtools.net_entries.len();
    let mut total_bytes: u64 = 0;
    let mut last_end_ms = st.devtools.nav_start_ms;
    for e in &st.devtools.net_entries {
        total_bytes += e.transferred;
        if matches!(e.status, NetStatus::Pending) {
            continue;
        }
        if e.end_ms.wrapping_sub(st.devtools.nav_start_ms)
            > last_end_ms.wrapping_sub(st.devtools.nav_start_ms)
        {
            last_end_ms = e.end_ms;
        }
    }
    let elapsed = last_end_ms.wrapping_sub(st.devtools.nav_start_ms);
    let txt = format!(
        "{} Anfragen   {} übertragen   Beendet: {} ms",
        total,
        humanize_size(total_bytes),
        elapsed,
    );
    st.devtools.net_status_label.set_text(&txt);
}

fn humanize_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::from("—");
    }
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} kB", bytes as f64 / 1024.0);
    }
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn phase_label(ms: u32) -> String {
    if ms == 0 {
        String::from("—")
    } else {
        format!("{}", ms)
    }
}

fn last_segment(path: &str) -> String {
    let p = path.split('?').next().unwrap_or(path);
    let trimmed = p.trim_end_matches('/');
    let seg = trimmed.rsplit('/').next().unwrap_or(p);
    if seg.is_empty() {
        String::from("/")
    } else {
        // Truncate huge query strings.
        let s: String = seg.chars().take(120).collect();
        s
    }
}

/// Hook called from net_worker when a request is submitted.
pub fn record_request_started(method: &str, kind: &str, url: &Url) -> u32 {
    let st = crate::state();
    if st.devtools.net_paused {
        return 0;
    }
    let now = anyos_std::sys::uptime_ms();
    let id = st.devtools.net_next_request_id;
    st.devtools.net_next_request_id = st.devtools.net_next_request_id.wrapping_add(1).max(1);
    let initiator = current_initiator();
    let file = last_segment(&url.path);
    st.devtools.net_entries.push(NetEntry {
        id,
        status: NetStatus::Pending,
        method: method.to_string(),
        host: url.host.clone(),
        file,
        path: url.path.clone(),
        initiator,
        kind: kind.to_string(),
        size: 0,
        transferred: 0,
        start_ms: now,
        end_ms: now,
        phases: NetPhases::default(),
    });
    if st.devtools.net_entries.len() > 1000 {
        let drop = st.devtools.net_entries.len() - 1000;
        st.devtools.net_entries.drain(0..drop);
    }
    if st.devtools.open {
        refresh_network();
    }
    id
}

/// Compute an initiator label — the URL of the page that drove the request.
fn current_initiator() -> String {
    let st = crate::state();
    if st.active_tab >= st.tabs.len() {
        return String::new();
    }
    let tab = &st.tabs[st.active_tab];
    if let Some(url) = &tab.current_url {
        return format!("{}{}", url.host, url.path);
    }
    String::new()
}

/// Hook called when a request completes — matches by host+path of the most
/// recent entry without an end time.
pub fn record_request_done(host: &str, path: &str, status: u32, size: u64) {
    record_request_done_inner(host, path, status, size, None);
}

pub fn record_request_done_with_timing(
    host: &str,
    path: &str,
    status: u32,
    size: u64,
    timing: RequestTiming,
) {
    record_request_done_inner(host, path, status, size, Some(timing));
}

fn record_request_done_inner(
    host: &str,
    path: &str,
    status: u32,
    size: u64,
    timing: Option<RequestTiming>,
) {
    let st = crate::state();
    let now = anyos_std::sys::uptime_ms();
    let mut updated = false;
    for e in st.devtools.net_entries.iter_mut().rev() {
        let id_matches = timing
            .as_ref()
            .map(|t| t.request_id != 0 && e.id == t.request_id)
            .unwrap_or(false);
        let url_matches = matches!(e.status, NetStatus::Pending) && e.host == host && e.path == path;
        if matches!(e.status, NetStatus::Pending) && (id_matches || url_matches) {
            e.status = NetStatus::from_http(status);
            e.size = size;
            e.transferred = size;
            e.end_ms = now;
            if let Some(mut t) = timing {
                t.ui_done_ms = now;
                let submitted_ms = if t.submitted_ms != 0 {
                    t.submitted_ms
                } else {
                    e.start_ms
                };
                let dequeued_ms = if t.dequeued_ms != 0 {
                    t.dequeued_ms
                } else {
                    t.start_ms
                };
                let fetch_done_ms = if t.fetch_done_ms != 0 {
                    t.fetch_done_ms
                } else {
                    now
                };
                let result_enqueued_ms = if t.result_enqueued_ms != 0 {
                    t.result_enqueued_ms
                } else {
                    fetch_done_ms
                };
                e.phases = NetPhases {
                    queue_ms: dequeued_ms.wrapping_sub(submitted_ms),
                    dns_ms: t.dns_ms,
                    connect_ms: t.connect_ms,
                    tls_ms: t.tls_ms,
                    send_ms: t.send_ms,
                    wait_ms: t.wait_ms,
                    body_ms: t.body_ms,
                    decode_ms: t.decode_ms,
                    enqueue_ms: result_enqueued_ms.wrapping_sub(fetch_done_ms),
                    ui_ms: t.ui_done_ms.wrapping_sub(result_enqueued_ms),
                    reused_connection: t.reused_connection,
                };
            }
            updated = true;
            break;
        }
    }
    if updated && st.devtools.open {
        refresh_network();
    }
}

/// Fallback completion hook for results that don't carry a URL — finds the
/// oldest pending entry of `kind` and marks it done.
pub fn record_request_done_by_kind(kind: &str, status: u32, size: u64) {
    let st = crate::state();
    let now = anyos_std::sys::uptime_ms();
    let mut updated = false;
    for e in st.devtools.net_entries.iter_mut() {
        if matches!(e.status, NetStatus::Pending) && e.kind == kind {
            e.status = NetStatus::from_http(status);
            e.size = size;
            e.transferred = size;
            e.end_ms = now;
            updated = true;
            break;
        }
    }
    if updated && st.devtools.open {
        refresh_network();
    }
}

/// Compose the computed-style summary for the selected DOM node.
pub fn show_selected_node_styles() {
    let st = crate::state();
    if !st.devtools.open {
        return;
    }
    let sel = st.devtools.dom_tree.selected();
    if sel == u32::MAX || (sel as usize) >= st.devtools.tree_to_dom.len() {
        st.devtools.style_pane.set_text("(kein Element ausgewählt)");
        return;
    }
    let dom_id = st.devtools.tree_to_dom[sel as usize];
    if st.active_tab >= st.tabs.len() {
        return;
    }
    let webview = &st.tabs[st.active_tab].webview;
    let dom = match webview.dom() {
        Some(d) => d,
        None => return,
    };
    if dom_id >= dom.nodes.len() {
        return;
    }
    let mut out = String::new();
    let node = &dom.nodes[dom_id];
    match &node.node_type {
        libwebview::dom::NodeType::Element { tag, attrs } => {
            out.push_str(&format!("Element: <{}>\n", tag.tag_name()));
            if !attrs.is_empty() {
                out.push_str("\nAttribute:\n");
                for a in attrs {
                    out.push_str(&format!("  {} = \"{}\"\n", a.name, a.value));
                }
            }
        }
        libwebview::dom::NodeType::Text(t) => {
            out.push_str("Textknoten:\n");
            out.push('\n');
            out.push_str(t);
        }
    }
    if let Some(style) = webview.resolved_style_ref(dom_id) {
        out.push_str("\nComputed Style:\n");
        out.push_str(&format!("  color:           #{:08X}\n", style.color));
        out.push_str(&format!(
            "  background:      #{:08X}\n",
            style.background_color
        ));
        out.push_str(&format!("  font-size:       {}px\n", style.font_size));
        out.push_str(&format!("  display:         {:?}\n", style.display));
        let pos = match style.position {
            libwebview::style::Position::Static => "static",
            libwebview::style::Position::Relative => "relative",
            libwebview::style::Position::Absolute => "absolute",
            libwebview::style::Position::Fixed => "fixed",
            libwebview::style::Position::Sticky => "sticky",
        };
        out.push_str(&format!("  position:        {}\n", pos));
        out.push_str(&format!(
            "  margin:          {} {} {} {}\n",
            style.margin_top, style.margin_right, style.margin_bottom, style.margin_left
        ));
        out.push_str(&format!(
            "  padding:         {} {} {} {}\n",
            style.padding_top, style.padding_right, style.padding_bottom, style.padding_left
        ));
    }
    st.devtools.style_pane.set_text(&out);
}

/// Evaluate the console input text as JavaScript in the active tab.
pub fn eval_console_input() {
    let st = crate::state();
    let mut buf = [0u8; 4096];
    let len = st.devtools.console_input.get_text(&mut buf);
    if len == 0 {
        return;
    }
    let expr = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(s) => s.to_string(),
        Err(_) => return,
    };
    if st.active_tab >= st.tabs.len() {
        return;
    }
    crate::surf_log!("[devtools] eval: {}", expr);
    let _ = st.tabs[st.active_tab]
        .webview
        .execute_js(&[alloc::format!("(function(){{return ({});}})()", expr)]);
    st.devtools.console_input.set_text("");
    refresh_console();
}

/// Toggle picker mode on/off — wired to the "Auswählen" tab activation.
pub fn set_picker_active(on: bool) {
    let st = crate::state();
    st.devtools.picker_active = on;
    crate::surf_log!("[devtools] picker_active = {}", on);
}

/// Switch the visible top-level panel inside the DevTools window.
///
/// Called from the `on_active_changed` handler — combines panel visibility
/// switching with picker-mode toggling because libanyui's `on_change`
/// callback only supports a single registration per control.
pub fn switch_panel(index: u32) {
    let st = crate::state();
    let n = st.devtools.panels.len() as u32;
    if index >= n {
        return;
    }
    for (i, &pid) in st.devtools.panels.iter().enumerate() {
        ui_lib::Control::from_id(pid).set_visible(i as u32 == index);
    }
    st.devtools.picker_active = index == 0;
    if index == 4 {
        // Repaint the network grid in case entries arrived while another tab
        // was active.
        refresh_network();
    } else if index == 1 {
        refresh_inspector();
    } else if index == 2 {
        refresh_console();
    }
}

/// Activate one of the kind filters by id (`""`, `"html"`, `"css"`, …) — also
/// updates the visual state of the filter buttons.
pub fn set_kind_filter(kind: &str) {
    let st = crate::state();
    st.devtools.net_filter_kind = kind.to_string();
    for (id, btn) in &st.devtools.net_filter_btns {
        let active = id == kind;
        btn.set_color(if active {
            COLOR_FILTER_ACTIVE
        } else {
            COLOR_FILTER_INACTIVE
        });
        btn.set_text_color(if active { COLOR_TEXT } else { COLOR_DIM });
    }
    if st.devtools.open {
        refresh_network();
    }
}

/// Toggle pause mode for new request recording.
pub fn toggle_pause() {
    let st = crate::state();
    st.devtools.net_paused = !st.devtools.net_paused;
    let icon = if st.devtools.net_paused {
        "play"
    } else {
        "pause"
    };
    st.devtools
        .net_pause_btn
        .set_system_icon(icon, ui_lib::IconType::Outline, COLOR_TEXT, 16);
}

/// Clear the recorded list (manual button).
pub fn clear_network() {
    let st = crate::state();
    st.devtools.net_entries.clear();
    st.devtools.net_next_request_id = 1;
    st.devtools.nav_start_ms = anyos_std::sys::uptime_ms();
    if st.devtools.open {
        refresh_network();
    }
}

/// Select the DOM node identified by `dom_id` in the inspector tree, switch
/// to the Inspektor tab and disable picker mode. Called from the click hook
/// in `callbacks::on_link_click` when picker mode is active.
pub fn select_dom_node(dom_id: usize) {
    let st = crate::state();
    if !st.devtools.open {
        st.devtools.open = true;
        st.devtools.win.move_to(120, 120);
        st.devtools.win.set_visible(true);
        refresh_all();
    }
    let mut found: Option<u32> = None;
    for (i, &mapped) in st.devtools.tree_to_dom.iter().enumerate() {
        if mapped == dom_id {
            found = Some(i as u32);
            break;
        }
    }
    if let Some(idx) = found {
        st.devtools.dom_tree.set_selected(idx);
        show_selected_node_styles();
    } else {
        refresh_inspector();
        for (i, &mapped) in st.devtools.tree_to_dom.iter().enumerate() {
            if mapped == dom_id {
                st.devtools.dom_tree.set_selected(i as u32);
                show_selected_node_styles();
                break;
            }
        }
    }
    st.devtools.tab_bar.set_state(1);
    st.devtools.picker_active = false;
}
