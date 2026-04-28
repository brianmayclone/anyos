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

use crate::http::Url;

const COLOR_BG: u32 = 0xFF1E1E1E;
const COLOR_PANE: u32 = 0xFF252526;
const COLOR_TEXT: u32 = 0xFFCCCCCC;
const COLOR_DIM: u32 = 0xFF888888;

/// One row of the network panel.
pub struct NetEntry {
    pub method: String,
    pub status: String,
    pub host: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub start_ms: u32,
    pub end_ms: u32,
}

pub struct DevTools {
    pub win: ui_lib::Window,
    pub open: bool,

    pub tab_bar: ui_lib::TabBar,

    /// Maps a TreeView index to the libwebview DOM node id.
    pub dom_tree: ui_lib::TreeView,
    pub style_pane: ui_lib::TextArea,
    pub tree_to_dom: Vec<usize>,

    pub console_output: ui_lib::TextArea,
    pub console_input: ui_lib::TextField,

    pub net_grid: ui_lib::DataGrid,
    pub net_entries: Vec<NetEntry>,

    /// `true` while the user is in element-picker mode — set by the toolbar
    /// button and consumed on the next click in the webview.
    pub picker_active: bool,
}

pub fn build() -> DevTools {
    // Master window — hidden by default and parked off-screen so it doesn't
    // flash before the user opens it.
    let win = ui_lib::Window::new("Werkzeuge für Webentwickler", 9999, 9999, 900, 500);
    win.set_visible(false);

    // ── Tab bar (DOCK_TOP) ──────────────────────────────────────────────────
    let tab_bar = ui_lib::TabBar::new(
        "Auswählen|Inspektor|Konsole|Debugger|Netzwerkanalyse",
    );
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
    let net_grid = ui_lib::DataGrid::new(900, 460);
    net_grid.set_dock(ui_lib::DOCK_FILL);
    net_grid.set_columns(&[
        ui_lib::ColumnDef::new("Method").width(70),
        ui_lib::ColumnDef::new("Status").width(60),
        ui_lib::ColumnDef::new("Host").width(180),
        ui_lib::ColumnDef::new("Pfad").width(360),
        ui_lib::ColumnDef::new("Typ").width(70),
        ui_lib::ColumnDef::new("Größe").width(70).align(ui_lib::ALIGN_RIGHT).numeric(),
        ui_lib::ColumnDef::new("Dauer (ms)").width(80).align(ui_lib::ALIGN_RIGHT).numeric(),
    ]);
    net_panel.add(&net_grid);

    // Make all panels DOCK_FILL so they fill the area below the tab bar; the
    // TabBar's `connect_panels` shows exactly one at a time.
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

    tab_bar.connect_panels(&[
        &picker_panel,
        &insp_panel,
        &console_panel,
        &dbg_panel,
        &net_panel,
    ]);

    DevTools {
        win,
        open: false,
        tab_bar,
        dom_tree,
        style_pane,
        tree_to_dom: Vec::new(),
        console_output,
        console_input,
        net_grid,
        net_entries: Vec::new(),
        picker_active: false,
    }
}

/// Clear every panel — called from `tab::navigate*` so each page load gets a
/// fresh slate, regardless of whether the DevTools window is currently
/// visible. Network recording starts from zero on the new navigation.
pub fn reset_for_navigation() {
    let st = crate::state();
    st.devtools.net_entries.clear();
    if st.devtools.open {
        st.devtools.net_grid.set_row_count(0);
        st.devtools.net_grid.set_data_raw(&[]);
        st.devtools.dom_tree.clear();
        st.devtools.tree_to_dom.clear();
        st.devtools.style_pane.set_text("(kein Element ausgewählt)");
        st.devtools.console_output.set_text("");
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

    // BFS-ish build: walk from node 0 (root) and add every Element node.
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
    let start = if lines.len() > 500 { lines.len() - 500 } else { 0 };
    for line in &lines[start..] {
        text.push_str(line);
        text.push('\n');
    }
    st.devtools.console_output.set_text(&text);
}

/// Repaint the network grid from `net_entries`.
pub fn refresh_network() {
    let st = crate::state();
    if !st.devtools.open {
        return;
    }
    let count = st.devtools.net_entries.len() as u32;
    st.devtools.net_grid.set_row_count(count);
    let mut buf: Vec<u8> = Vec::new();
    for (i, e) in st.devtools.net_entries.iter().enumerate() {
        if i > 0 {
            buf.push(0x1E); // row sep
        }
        let dur = e.end_ms.wrapping_sub(e.start_ms);
        let cells: [&str; 7] = [
            &e.method,
            &e.status,
            &e.host,
            &e.path,
            &e.kind,
            &humanize_size(e.size),
            &alloc::format!("{}", dur),
        ];
        for (j, c) in cells.iter().enumerate() {
            if j > 0 {
                buf.push(0x1F);
            }
            buf.extend_from_slice(c.as_bytes());
        }
    }
    st.devtools.net_grid.set_data_raw(&buf);
}

fn humanize_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} kB", bytes as f64 / 1024.0);
    }
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Hook called from net_worker when a request is submitted.
pub fn record_request_started(method: &str, kind: &str, url: &Url) {
    let st = crate::state();
    let now = anyos_std::sys::uptime_ms();
    st.devtools.net_entries.push(NetEntry {
        method: method.to_string(),
        status: String::from("…"),
        host: url.host.clone(),
        path: url.path.clone(),
        kind: kind.to_string(),
        size: 0,
        start_ms: now,
        end_ms: now,
    });
    // Cap history to keep the grid responsive.
    if st.devtools.net_entries.len() > 500 {
        let drop = st.devtools.net_entries.len() - 500;
        st.devtools.net_entries.drain(0..drop);
    }
    if st.devtools.open {
        refresh_network();
    }
}

/// Hook called when a request completes — matches by host+path of the most
/// recent entry without an end time.
pub fn record_request_done(host: &str, path: &str, status: u32, size: u64) {
    let st = crate::state();
    let now = anyos_std::sys::uptime_ms();
    let mut updated = false;
    for e in st.devtools.net_entries.iter_mut().rev() {
        if e.status == "…" && e.host == host && e.path == path {
            e.status = alloc::format!("{}", status);
            e.size = size;
            e.end_ms = now;
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
        if e.status == "…" && e.kind == kind {
            e.status = alloc::format!("{}", status);
            e.size = size;
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
        out.push_str(&format!("  background:      #{:08X}\n", style.background_color));
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
    // Find the tree index that maps back to `dom_id`. The tree was built in
    // pre-order over Element nodes; non-Element parents may be missing.
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
        // Fallback: refresh the tree (DOM may have changed) and try again.
        refresh_inspector();
        for (i, &mapped) in st.devtools.tree_to_dom.iter().enumerate() {
            if mapped == dom_id {
                st.devtools.dom_tree.set_selected(i as u32);
                show_selected_node_styles();
                break;
            }
        }
    }
    // Switch to the Inspector tab (index 1) and exit picker mode.
    st.devtools.tab_bar.set_state(1);
    st.devtools.picker_active = false;
}
