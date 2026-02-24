#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use libanyui_client as anyui;

anyos_std::entry!(main);

// ── Font IDs ──────────────────────────────────────────────────────────────────
const FONT_REGULAR: u32 = 0;
const FONT_BOLD: u32 = 1;
const FONT_MONO: u32 = 4;

// ── Colors (dark theme) ──────────────────────────────────────────────────────
const COLOR_BG: u32 = 0xFF1E1E1E;
const COLOR_TOOLBAR: u32 = 0xFF252526;
const COLOR_STATUS: u32 = 0xFF252525;
const COLOR_TAB_BAR: u32 = 0xFF2D2D2D;

const TEXT_HEADING: u32 = 0xFFE0E0E0;
const TEXT_BODY: u32 = 0xFFCCCCCC;
const TEXT_CODE: u32 = 0xFFD4D4D4;
const TEXT_QUOTE: u32 = 0xFF9E9E9E;
const TEXT_LINK: u32 = 0xFF569CD6;
const TEXT_STATUS: u32 = 0xFF969696;
const TEXT_LIST_BULLET: u32 = 0xFF569CD6;

const BG_CODE_BLOCK: u32 = 0xFF0D0D0D;
const BG_CODE_INLINE: u32 = 0xFF2A2A2A;
const BG_QUOTE: u32 = 0xFF252525;

// ── Heading sizes ────────────────────────────────────────────────────────────
const H_SIZES: [u32; 6] = [28, 24, 20, 18, 16, 15];

// ── Wrap width (characters) ──────────────────────────────────────────────────
const WRAP_WIDTH: usize = 110;

// ── Data structures ──────────────────────────────────────────────────────────

struct OpenFile {
    path: String,
    panel: anyui::StackPanel,
}

struct AppState {
    tab_bar: anyui::TabBar,
    scroll: anyui::ScrollView,
    status_label: anyui::Label,
    path_label: anyui::Label,
    files: Vec<OpenFile>,
    active: usize,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── Utility ──────────────────────────────────────────────────────────────────

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn tab_labels(files: &[OpenFile]) -> String {
    let mut s = String::new();
    for (i, f) in files.iter().enumerate() {
        if i > 0 { s.push('|'); }
        s.push_str(basename(&f.path));
    }
    s
}

// ── Word wrapping ────────────────────────────────────────────────────────────

fn word_wrap(text: &str, max_chars: usize) -> String {
    let mut result = String::new();
    let mut col = 0usize;

    for word in text.split(' ') {
        if word.is_empty() { continue; }
        let wlen = word.len();
        if col > 0 && col + 1 + wlen > max_chars {
            result.push('\n');
            col = 0;
        }
        if col > 0 {
            result.push(' ');
            col += 1;
        }
        result.push_str(word);
        col += wlen;
    }
    result
}

// ── Inline markdown stripping ────────────────────────────────────────────────

/// Strip common inline markers: **bold**, *italic*, `code`, [text](url)
fn strip_inline(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0;

    while i < len {
        // **bold** or __bold__
        if i + 1 < len && ((bytes[i] == b'*' && bytes[i+1] == b'*') || (bytes[i] == b'_' && bytes[i+1] == b'_')) {
            let marker = bytes[i];
            i += 2;
            while i + 1 < len && !(bytes[i] == marker && bytes[i+1] == marker) {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i + 1 < len { i += 2; }
            continue;
        }
        // *italic* or _italic_ (single)
        if (bytes[i] == b'*' || bytes[i] == b'_') && i + 1 < len && bytes[i+1] != b' ' {
            let marker = bytes[i];
            i += 1;
            while i < len && bytes[i] != marker {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < len { i += 1; }
            continue;
        }
        // `inline code`
        if bytes[i] == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < len { i += 1; }
            continue;
        }
        // [text](url) → just text
        if bytes[i] == b'[' {
            i += 1;
            let start = i;
            while i < len && bytes[i] != b']' {
                i += 1;
            }
            let link_text = &text[start..i];
            if i < len { i += 1; } // skip ]
            // skip (url) if present
            if i < len && bytes[i] == b'(' {
                i += 1;
                while i < len && bytes[i] != b')' { i += 1; }
                if i < len { i += 1; }
            }
            out.push_str(link_text);
            continue;
        }
        // ![alt](url) → [Image: alt]
        if bytes[i] == b'!' && i + 1 < len && bytes[i+1] == b'[' {
            i += 2;
            let start = i;
            while i < len && bytes[i] != b']' { i += 1; }
            let alt = &text[start..i];
            if i < len { i += 1; }
            if i < len && bytes[i] == b'(' {
                i += 1;
                while i < len && bytes[i] != b')' { i += 1; }
                if i < len { i += 1; }
            }
            out.push_str("[Image: ");
            out.push_str(alt);
            out.push(']');
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// ── Markdown renderer ────────────────────────────────────────────────────────

fn add_spacing(panel: &anyui::StackPanel, height: u32) {
    let spacer = anyui::Label::new("");
    spacer.set_size(10, height);
    panel.add(&spacer);
}

fn add_heading(panel: &anyui::StackPanel, text: &str, level: usize) {
    let lvl = if level > 6 { 5 } else { level - 1 };
    let size = H_SIZES[lvl];

    add_spacing(panel, if level <= 2 { 12 } else { 8 });

    let stripped = strip_inline(text);
    let label = anyui::Label::new(&stripped);
    label.set_font(FONT_BOLD);
    label.set_font_size(size);
    label.set_text_color(TEXT_HEADING);
    label.set_padding(16, 2, 16, 2);
    label.set_auto_size(true);
    panel.add(&label);

    // Underline for H1 and H2
    if level <= 2 {
        let div = anyui::Divider::new();
        div.set_size(800, 1);
        div.set_color(0xFF404040);
        div.set_margin(16, 4, 16, 4);
        panel.add(&div);
    }
}

fn add_paragraph(panel: &anyui::StackPanel, text: &str) {
    let stripped = strip_inline(text);
    let wrapped = word_wrap(&stripped, WRAP_WIDTH);
    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_BODY);
    label.set_padding(16, 4, 16, 4);
    label.set_auto_size(true);
    panel.add(&label);
}

fn add_code_block(panel: &anyui::StackPanel, lines: &[&str]) {
    let code_text = {
        let mut s = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 { s.push('\n'); }
            s.push_str(line);
        }
        s
    };

    let container = anyui::View::new();
    container.set_color(BG_CODE_BLOCK);
    container.set_margin(16, 4, 16, 4);
    container.set_padding(12, 8, 12, 8);
    container.set_auto_size(true);

    let label = anyui::Label::new(&code_text);
    label.set_font(FONT_MONO);
    label.set_font_size(13);
    label.set_text_color(TEXT_CODE);
    label.set_auto_size(true);
    container.add(&label);

    panel.add(&container);
}

fn add_list_item(panel: &anyui::StackPanel, text: &str, ordered: bool, number: usize) {
    let stripped = strip_inline(text);
    let prefixed = if ordered {
        anyos_std::format!("  {}. {}", number, stripped)
    } else {
        anyos_std::format!("  \u{2022} {}", stripped)
    };
    let wrapped = word_wrap(&prefixed, WRAP_WIDTH);
    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_BODY);
    label.set_padding(16, 1, 16, 1);
    label.set_auto_size(true);
    panel.add(&label);
}

fn add_blockquote(panel: &anyui::StackPanel, text: &str) {
    let stripped = strip_inline(text);
    let prefixed = anyos_std::format!("  \u{2502} {}", stripped);
    let wrapped = word_wrap(&prefixed, WRAP_WIDTH - 4);

    let container = anyui::View::new();
    container.set_color(BG_QUOTE);
    container.set_margin(16, 2, 16, 2);
    container.set_padding(8, 4, 8, 4);
    container.set_auto_size(true);

    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_QUOTE);
    label.set_auto_size(true);
    container.add(&label);

    panel.add(&container);
}

fn add_horizontal_rule(panel: &anyui::StackPanel) {
    add_spacing(panel, 6);
    let div = anyui::Divider::new();
    div.set_size(800, 1);
    div.set_color(0xFF505050);
    div.set_margin(16, 0, 16, 0);
    panel.add(&div);
    add_spacing(panel, 6);
}

fn is_hr_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 { return false; }
    let bytes = trimmed.as_bytes();
    let ch = bytes[0];
    if ch != b'-' && ch != b'*' && ch != b'_' { return false; }
    bytes.iter().all(|&b| b == ch || b == b' ')
}

fn heading_level(line: &str) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level >= 1 && level <= 6 && level < bytes.len() && bytes[level] == b' ' {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

fn list_item_text(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    // Unordered: - item, * item
    if trimmed.len() >= 2 && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
        return Some((false, &trimmed[2..]));
    }
    // Ordered: 1. item, 12. item
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return Some((true, &trimmed[i + 2..]));
    }
    None
}

fn blockquote_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("> ") {
        Some(&trimmed[2..])
    } else if trimmed == ">" {
        Some("")
    } else {
        None
    }
}

fn render_markdown(md: &str, panel: &anyui::StackPanel) {
    let lines: Vec<&str> = md.split('\n').collect();
    let total = lines.len();
    let mut i = 0;
    let mut para_buf = String::new();
    let mut ordered_counter = 0usize;

    while i < total {
        let line = lines[i];

        // Fenced code block
        if line.trim_start().starts_with("```") {
            // Flush paragraph
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            i += 1;
            let mut code_lines: Vec<&str> = Vec::new();
            while i < total && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < total { i += 1; } // skip closing ```
            add_code_block(panel, &code_lines);
            continue;
        }

        // Empty line → paragraph break
        if line.trim().is_empty() {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            ordered_counter = 0;
            add_spacing(panel, 4);
            i += 1;
            continue;
        }

        // Heading
        if let Some((level, text)) = heading_level(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_heading(panel, text, level);
            i += 1;
            continue;
        }

        // Horizontal rule
        if is_hr_line(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_horizontal_rule(panel);
            i += 1;
            continue;
        }

        // Blockquote
        if let Some(text) = blockquote_text(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_blockquote(panel, text);
            i += 1;
            continue;
        }

        // List item
        if let Some((ordered, text)) = list_item_text(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            if ordered {
                ordered_counter += 1;
            }
            add_list_item(panel, text, ordered, ordered_counter);
            i += 1;
            continue;
        }

        // Regular paragraph text — accumulate
        if !para_buf.is_empty() {
            para_buf.push(' ');
        }
        para_buf.push_str(line);
        i += 1;
    }

    // Flush remaining paragraph
    if !para_buf.is_empty() {
        add_paragraph(panel, &para_buf);
    }

    // Bottom spacing
    add_spacing(panel, 20);
}

// ── File operations ──────────────────────────────────────────────────────────

fn open_file(path: &str) {
    let s = app();

    // Check if already open → switch to it
    for (i, f) in s.files.iter().enumerate() {
        if f.path == path {
            switch_tab(i);
            return;
        }
    }

    // Read file
    let content = match anyos_std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            anyos_std::println!("mdview: failed to read {}", path);
            return;
        }
    };

    // Hide current active panel
    if !s.files.is_empty() && s.active < s.files.len() {
        s.files[s.active].panel.set_visible(false);
    }

    // Create content panel
    let panel = anyui::StackPanel::vertical();
    panel.set_dock(anyui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_padding(0, 8, 0, 8);

    // Render markdown into the panel
    render_markdown(&content, &panel);

    // Add panel to scroll view
    s.scroll.add(&panel);

    // Track the file
    let idx = s.files.len();
    s.files.push(OpenFile {
        path: String::from(path),
        panel,
    });
    s.active = idx;

    // Update UI
    let labels = tab_labels(&s.files);
    s.tab_bar.set_text(&labels);
    s.tab_bar.set_state(idx as u32);
    s.tab_bar.set_visible(true);

    update_status();
}

fn close_tab(index: usize) {
    let s = app();
    if index >= s.files.len() { return; }

    // Remove panel from scroll view
    s.files[index].panel.remove();
    s.files.remove(index);

    if s.files.is_empty() {
        s.active = 0;
        s.tab_bar.set_visible(false);
        s.path_label.set_text("No file open");
        s.status_label.set_text("Ready");
        return;
    }

    // Adjust active index
    if s.active >= s.files.len() {
        s.active = s.files.len() - 1;
    }

    // Show the new active panel
    for (i, f) in s.files.iter().enumerate() {
        f.panel.set_visible(i == s.active);
    }

    let labels = tab_labels(&s.files);
    s.tab_bar.set_text(&labels);
    s.tab_bar.set_state(s.active as u32);
    update_status();
}

fn switch_tab(index: usize) {
    let s = app();
    if index >= s.files.len() { return; }

    // Hide all, show target
    for (i, f) in s.files.iter().enumerate() {
        f.panel.set_visible(i == index);
    }
    s.active = index;
    s.tab_bar.set_state(index as u32);
    update_status();
}

fn update_status() {
    let s = app();
    if s.files.is_empty() {
        s.path_label.set_text("No file open");
        s.status_label.set_text("Ready");
        return;
    }

    let file = &s.files[s.active];
    let name = basename(&file.path);
    s.path_label.set_text(&file.path);

    let status = anyos_std::format!("{} | {} file(s) open", name, s.files.len());
    s.status_label.set_text(&status);
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("mdview: failed to load libanyui.so");
        return;
    }

    // Parse command line argument
    let mut args_buf = [0u8; 256];
    let arg_path = anyos_std::process::args(&mut args_buf).trim();

    // Create window
    let win = anyui::Window::new("Markdown Viewer", -1, -1, 900, 600);

    // ── Toolbar ──
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(900, 36);
    toolbar.set_color(COLOR_TOOLBAR);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_open = toolbar.add_icon_button("Open");
    btn_open.set_size(60, 28);
    btn_open.set_icon(anyui::ICON_FOLDER_OPEN);

    toolbar.add_separator();

    let path_label = toolbar.add_label("No file open");
    path_label.set_text_color(TEXT_STATUS);

    win.add(&toolbar);

    // ── TabBar ──
    let tab_bar = anyui::TabBar::new("");
    tab_bar.set_dock(anyui::DOCK_TOP);
    tab_bar.set_size(900, 28);
    tab_bar.set_color(COLOR_TAB_BAR);
    tab_bar.set_visible(false);
    win.add(&tab_bar);

    // ── Status bar ──
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(900, 24);
    status_bar.set_color(COLOR_STATUS);

    let status_label = anyui::Label::new("Ready");
    status_label.set_position(8, 4);
    status_label.set_text_color(TEXT_STATUS);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Scroll view (DOCK_FILL) ──
    let scroll = anyui::ScrollView::new();
    scroll.set_dock(anyui::DOCK_FILL);
    scroll.set_color(COLOR_BG);
    win.add(&scroll);

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            tab_bar,
            scroll,
            status_label,
            path_label,
            files: Vec::new(),
            active: 0,
        });
    }

    // ── Wire events ──
    btn_open.on_click(|_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            open_file(&path);
        }
    });

    app().tab_bar.on_active_changed(|e| {
        switch_tab(e.index as usize);
    });

    app().tab_bar.on_tab_close(|e| {
        close_tab(e.index as usize);
    });

    win.on_close(|_| { anyui::quit(); });

    // ── Open file from command line ──
    if !arg_path.is_empty() {
        open_file(arg_path);
    }

    // ── Run event loop ──
    anyui::run();
}
