#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────────

const MAX_DISPLAY: usize = 60;
const CELL_W: u32 = 80;
const CELL_H: u32 = 72;
const ICON_SIZE: u32 = 32;
const ICO_PAK_PATH: &str = "/System/media/ico.pak";
const CELLS_PER_TICK: usize = 5;  // cells created per timer tick during init
const ICONS_PER_TICK: usize = 5;  // icons rendered per timer tick

// ── Data model ───────────────────────────────────────────────────────────────

struct UniqueIcon {
    name: String,
    has_filled: bool,
    has_outline: bool,
}

struct IconCell {
    container: anyui::View,
    image: anyui::ImageView,
    label: anyui::Label,
}

struct AppState {
    status_label: anyui::Label,
    flow: anyui::FlowPanel,

    all_icons: Vec<UniqueIcon>,
    cells: Vec<IconCell>,

    current_color: u32,
    icon_mode: u32, // 0=filled, 1=outline, 2=both
    search_query: String,

    selected_cell: Option<usize>,
    selected_icon_name: String,

    // Incremental rendering state
    pending_matches: Vec<(usize, u32)>,
    render_cursor: usize,
    render_timer: u32,

    // Deferred init state
    init_timer: u32,
    init_done: bool,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── ico.pak binary parser ────────────────────────────────────────────────────

fn u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn parse_icon_names(pak: &[u8]) -> Vec<UniqueIcon> {
    let mut icons: Vec<UniqueIcon> = Vec::new();
    if pak.len() < 20 || &pak[0..4] != b"IPAK" {
        return icons;
    }
    let version = u16_le(&pak[4..6]);
    if version != 2 {
        return icons;
    }

    let filled_count = u16_le(&pak[6..8]) as usize;
    let outline_count = u16_le(&pak[8..10]) as usize;
    let names_offset = u32_le(&pak[12..16]) as usize;
    let total = filled_count + outline_count;

    let index_base = 20usize;
    let entry_size = 16usize;

    for i in 0..total {
        let off = index_base + i * entry_size;
        if off + entry_size > pak.len() {
            break;
        }

        let name_off = u32_le(&pak[off..off + 4]) as usize;
        let name_len = u16_le(&pak[off + 4..off + 6]) as usize;
        let icon_type = pak[off + 6];

        let abs_name = names_offset + name_off;
        if abs_name + name_len > pak.len() {
            continue;
        }

        if let Ok(name_str) = core::str::from_utf8(&pak[abs_name..abs_name + name_len]) {
            let mut merged = false;
            if let Some(last) = icons.last_mut() {
                if last.name.as_str() == name_str {
                    if icon_type == 0 {
                        last.has_filled = true;
                    } else {
                        last.has_outline = true;
                    }
                    merged = true;
                }
            }
            if !merged {
                icons.push(UniqueIcon {
                    name: String::from(name_str),
                    has_filled: icon_type == 0,
                    has_outline: icon_type != 0,
                });
            }
        }
    }

    // Sort alphabetically (insertion sort)
    let len = icons.len();
    for i in 1..len {
        let mut j = i;
        while j > 0 && icons[j - 1].name.as_str() > icons[j].name.as_str() {
            icons.swap(j - 1, j);
            j -= 1;
        }
    }

    icons
}

// ── Fuzzy search ─────────────────────────────────────────────────────────────

fn fuzzy_score(query: &str, target: &str) -> u32 {
    if query.is_empty() {
        return 1;
    }

    let q = query.as_bytes();
    let t = target.as_bytes();

    let mut qi = 0usize;
    let mut score = 0u32;
    let mut prev_match: i32 = -1;
    let mut first_match: i32 = -1;

    for (ti, &tb) in t.iter().enumerate() {
        if qi < q.len() && tb.to_ascii_lowercase() == q[qi].to_ascii_lowercase() {
            if first_match == -1 {
                first_match = ti as i32;
            }
            if prev_match >= 0 && ti as i32 == prev_match + 1 {
                score += 10;
            }
            if ti == 0 || t[ti - 1] == b'-' {
                score += 15;
            }
            score += 5;
            prev_match = ti as i32;
            qi += 1;
        }
    }

    if qi < q.len() {
        return 0;
    }

    if first_match == 0 {
        score += 20;
    }
    if q.len() == t.len() {
        score += 50;
    }
    let spread = (prev_match - first_match + 1) as u32;
    if spread > q.len() as u32 {
        score = score.saturating_sub((spread - q.len() as u32) * 2);
    }

    score.max(1)
}

// ── Cell click callback ──────────────────────────────────────────────────────

extern "C" fn on_cell_clicked(_id: u32, _event_type: u32, userdata: u64) {
    select_cell(userdata as usize);
}

fn select_cell(cell_idx: usize) {
    let s = app();

    if let Some(prev) = s.selected_cell {
        if prev < s.cells.len() {
            s.cells[prev].container.set_color(0x00000000);
        }
    }

    if cell_idx >= s.cells.len() {
        return;
    }

    s.cells[cell_idx].container.set_color(0xFF3A3D41);
    s.selected_cell = Some(cell_idx);

    let matches = filter_icons(s);
    if cell_idx < matches.len() {
        let (idx, _) = matches[cell_idx];
        let name = s.all_icons[idx].name.clone();
        let total = s.all_icons.len();
        let matched = matches.len();
        s.status_label.set_text(&anyos_std::format!(
            "Selected: {} | {} of {} icons",
            name, matched, total
        ));
        s.selected_icon_name = name;
    }
}

// ── Filtering logic ──────────────────────────────────────────────────────────

fn filter_icons(s: &AppState) -> Vec<(usize, u32)> {
    let mut matches: Vec<(usize, u32)> = Vec::new();

    for (i, icon) in s.all_icons.iter().enumerate() {
        let type_ok = match s.icon_mode {
            0 => icon.has_filled,
            1 => icon.has_outline,
            _ => true,
        };
        if !type_ok {
            continue;
        }

        let score = fuzzy_score(s.search_query.as_str(), icon.name.as_str());
        if score > 0 {
            matches.push((i, score));
        }
    }

    // Sort by score descending (insertion sort)
    let len = matches.len();
    for i in 1..len {
        let mut j = i;
        while j > 0 && matches[j].1 > matches[j - 1].1 {
            matches.swap(j - 1, j);
            j -= 1;
        }
    }

    matches
}

// ── Deferred cell creation (runs inside event loop) ──────────────────────────

fn init_tick() {
    let s = app();
    if s.init_done {
        if s.init_timer != 0 {
            anyui::kill_timer(s.init_timer);
            s.init_timer = 0;
        }
        return;
    }

    let current = s.cells.len();
    let target = (current + CELLS_PER_TICK).min(MAX_DISPLAY);

    for i in current..target {
        let container = anyui::View::new();
        container.set_size(CELL_W, CELL_H);
        container.set_margin(2, 2, 2, 2);

        let image = anyui::ImageView::new(ICON_SIZE, ICON_SIZE);
        image.set_position(((CELL_W - ICON_SIZE) / 2) as i32, 4);
        container.add(&image);

        let label = anyui::Label::new("");
        label.set_size(CELL_W, 28);
        label.set_position(0, 40);
        label.set_text_color(0xFFA0A0A0);
        label.set_font_size(10);
        label.set_text_align(anyui::TEXT_ALIGN_CENTER);
        container.add(&label);

        anyui::Control::from_id(container.id()).on_click_raw(on_cell_clicked, i as u64);
        container.set_visible(false);

        s.flow.add(&container);

        s.cells.push(IconCell { container, image, label });
    }

    s.status_label.set_text(&anyos_std::format!(
        "Loading... {}/{}",
        s.cells.len(), MAX_DISPLAY
    ));

    if s.cells.len() >= MAX_DISPLAY {
        s.init_done = true;
        if s.init_timer != 0 {
            anyui::kill_timer(s.init_timer);
            s.init_timer = 0;
        }
        refresh_display();
    }
}

// ── Display refresh ──────────────────────────────────────────────────────────

fn refresh_display() {
    let s = app();
    if !s.init_done {
        return;
    }
    s.selected_cell = None;

    if s.render_timer != 0 {
        anyui::kill_timer(s.render_timer);
        s.render_timer = 0;
    }

    let matches = filter_icons(s);
    let display_count = matches.len().min(MAX_DISPLAY);

    for j in 0..s.cells.len() {
        if j < display_count {
            let (idx, _) = matches[j];
            let icon_name = s.all_icons[idx].name.as_str();

            let display_name = if icon_name.len() > 12 {
                let mut t = String::from(&icon_name[..11]);
                t.push_str("..");
                t
            } else {
                String::from(icon_name)
            };
            s.cells[j].label.set_text(display_name.as_str());
            s.cells[j].container.set_tooltip(icon_name);
            s.cells[j].container.set_color(0x00000000);
            s.cells[j].container.set_visible(true);
        } else {
            s.cells[j].container.set_visible(false);
        }
    }

    let cols = ((900 - 16) / (CELL_W + 4)).max(1);
    let rows = ((display_count as u32) + cols - 1) / cols;
    s.flow.set_size(900, rows * (CELL_H + 4) + 16);

    let total = s.all_icons.len();
    let matched = matches.len();
    let status = if matched <= display_count {
        anyos_std::format!("{} icons", matched)
    } else {
        anyos_std::format!(
            "Showing {} of {} matches ({} total)",
            display_count, matched, total
        )
    };
    s.status_label.set_text(&status);

    s.pending_matches = matches;
    s.render_cursor = 0;
    s.render_timer = anyui::set_timer(30, render_batch);
}

fn render_batch() {
    let s = app();
    let display_count = s.pending_matches.len().min(MAX_DISPLAY);
    let color = s.current_color;
    let icon_mode = s.icon_mode;

    let start = s.render_cursor;
    let end = (start + ICONS_PER_TICK).min(display_count);

    for j in start..end {
        let (idx, _) = s.pending_matches[j];
        let icon_name = s.all_icons[idx].name.as_str();

        let icon_type = match icon_mode {
            0 => anyui::IconType::Filled,
            1 => anyui::IconType::Outline,
            _ => {
                if s.all_icons[idx].has_filled {
                    anyui::IconType::Filled
                } else {
                    anyui::IconType::Outline
                }
            }
        };

        if let Some(icon) = anyui::Icon::system(icon_name, icon_type, color, ICON_SIZE) {
            icon.apply_to(&s.cells[j].image);
        }
    }

    s.render_cursor = end;

    if end >= display_count {
        if s.render_timer != 0 {
            anyui::kill_timer(s.render_timer);
            s.render_timer = 0;
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        return;
    }

    // Parse ico.pak to enumerate all icon names (pure CPU work, no IPC)
    let all_icons = match anyos_std::fs::read_to_vec(ICO_PAK_PATH) {
        Ok(pak_data) => parse_icon_names(&pak_data),
        Err(_) => {
            anyos_std::println!("[iconview] Failed to read ico.pak");
            Vec::new()
        }
    };

    // Build minimal UI shell (only ~10 widget creations)
    let win = anyui::Window::new("Icon Browser", -1, -1, 900, 600);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(900, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

    let search = anyui::SearchField::new();
    search.set_size(300, 28);
    search.set_placeholder("Search icons...");
    toolbar.add(&search);

    toolbar.add_separator();

    let seg = anyui::SegmentedControl::new("Filled|Outline|Both");
    seg.set_size(200, 28);
    seg.set_state(2);
    toolbar.add(&seg);

    toolbar.add_separator();

    let color_label = toolbar.add_label("Color:");
    color_label.set_size(42, 28);

    let color_well = anyui::ColorWell::new();
    color_well.set_size(28, 28);
    color_well.set_selected_color(0xFFE0E0E0);
    toolbar.add(&color_well);

    toolbar.add_separator();

    let btn_copy = toolbar.add_icon_button("Copy Name");
    btn_copy.set_size(88, 28);

    win.add(&toolbar);

    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(900, 24);
    status_bar.set_color(0xFF1E1E1E);

    let status_label = anyui::Label::new("Loading...");
    status_label.set_position(8, 4);
    status_label.set_size(800, 16);
    status_label.set_text_color(0xFF808080);
    status_bar.add(&status_label);

    win.add(&status_bar);

    let scroll = anyui::ScrollView::new();
    scroll.set_dock(anyui::DOCK_FILL);
    scroll.set_color(0xFF1E1E1E);

    let flow = anyui::FlowPanel::new();
    flow.set_size(900, 600);
    flow.set_padding(8, 8, 8, 8);
    scroll.add(&flow);

    win.add(&scroll);

    // NO cell creation here — deferred to after event loop starts

    // Initialize global state
    unsafe {
        APP = Some(AppState {
            status_label,
            flow,
            all_icons,
            cells: Vec::new(),
            current_color: 0xFFE0E0E0,
            icon_mode: 2,
            search_query: String::new(),
            selected_cell: None,
            selected_icon_name: String::new(),
            pending_matches: Vec::new(),
            render_cursor: 0,
            render_timer: 0,
            init_timer: 0,
            init_done: false,
        });
    }

    // Start deferred cell creation AFTER event loop is running
    app().init_timer = anyui::set_timer(50, init_tick);

    // Register callbacks
    search.on_text_changed(|e| {
        let text = e.text();
        app().search_query = String::from(text.as_str());
        refresh_display();
    });

    seg.on_active_changed(|e| {
        app().icon_mode = e.index;
        refresh_display();
    });

    color_well.on_color_selected(|e| {
        app().current_color = e.color;
        refresh_display();
    });

    btn_copy.on_click(|_| {
        let name = app().selected_icon_name.clone();
        if !name.is_empty() {
            anyui::clipboard_set(name.as_str());
            app().status_label.set_text(&anyos_std::format!("Copied: {}", name));
        }
    });

    win.on_close(|_| {
        anyui::quit();
    });

    anyui::run();
}
