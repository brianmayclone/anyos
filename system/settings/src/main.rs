#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;
use ui::Widget;

mod types;
mod module_loader;
mod layout;
mod page_dashboard;
mod page_general;
mod page_display;
mod page_dock;
mod page_apps;
mod page_devices;
mod page_network;
mod page_update;

use types::*;

anyos_std::entry!(main);

/// Get the app's bundle directory (CWD).
pub(crate) fn bundle_dir() -> String {
    let mut buf = [0u8; 256];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len != u32::MAX && len > 0 {
        String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or("/"))
    } else {
        String::from("/")
    }
}

/// Build the full path to a resource icon file.
pub(crate) fn icon_path(filename: &str) -> String {
    format!("{}/resources/icons/{}", bundle_dir(), filename)
}

// ── Global state ────────────────────────────────────────────────────────────

struct AppState {
    pages: Vec<PageEntry>,
    sidebar_ids: Vec<u32>,
    active_idx: usize,
    content_scroll: ui::ScrollView,
    sidebar_scroll_id: u32,
    sidebar_panel_id: u32,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        return;
    }

    let pages = module_loader::builtin_pages();

    let win = ui::Window::new("Settings", -1, -1, 900, 600);

    // ── SplitView (DOCK_FILL) ───────────────────────────────────────
    let split = ui::SplitView::new();
    split.set_dock(ui::DOCK_FILL);
    split.set_orientation(ui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(24);
    split.set_min_split(18);
    split.set_max_split(35);
    win.add(&split);

    // ── Left: Sidebar ───────────────────────────────────────────────
    let tc = ui::theme::colors();
    let sidebar_scroll = ui::ScrollView::new();
    sidebar_scroll.set_dock(ui::DOCK_FILL);
    sidebar_scroll.set_color(tc.sidebar_bg);

    let sidebar_panel = ui::View::new();
    sidebar_panel.set_dock(ui::DOCK_FILL);
    sidebar_panel.set_color(tc.sidebar_bg);

    // Title area
    let title_area = ui::View::new();
    title_area.set_dock(ui::DOCK_TOP);
    title_area.set_size(220, 52);

    let title_lbl = ui::Label::new("Settings");
    title_lbl.set_position(16, 16);
    title_lbl.set_size(180, 28);
    title_lbl.set_font_size(18);
    title_lbl.set_text_color(tc.text);
    title_area.add(&title_lbl);

    sidebar_panel.add(&title_area);

    // Search field
    let search = ui::SearchField::new();
    search.set_dock(ui::DOCK_TOP);
    search.set_size(188, 28);
    search.set_margin(16, 4, 16, 12);
    search.set_placeholder("Search settings");
    sidebar_panel.add(&search);

    // Page items
    let mut sidebar_ids: Vec<u32> = Vec::new();
    let mut last_category = String::new();

    for (i, page) in pages.iter().enumerate() {
        if page.category != last_category {
            if !last_category.is_empty() {
                let spacer = ui::View::new();
                spacer.set_dock(ui::DOCK_TOP);
                spacer.set_size(220, 8);
                sidebar_panel.add(&spacer);
            }
            if page.category != "System" {
                let cat_label = ui::Label::new(&page.category);
                cat_label.set_dock(ui::DOCK_TOP);
                cat_label.set_size(220, 22);
                cat_label.set_font_size(11);
                cat_label.set_text_color(tc.text_disabled);
                cat_label.set_margin(16, 4, 16, 2);
                sidebar_panel.add(&cat_label);
            }
            last_category = page.category.clone();
        }

        // Row container for icon + label
        let item = ui::View::new();
        item.set_dock(ui::DOCK_TOP);
        item.set_size(220, 34);
        item.set_margin(4, 1, 4, 1);
        if i == 0 {
            item.set_color(tc.selection);
        }

        // Icon (24x24, loaded from bundle resources)
        if !page.icon.is_empty() {
            let path = icon_path(&page.icon);
            if let Some(icon) = ui::Icon::load(&path, 24) {
                let iv = icon.into_image_view(24, 24);
                iv.set_dock(ui::DOCK_LEFT);
                iv.set_margin(12, 5, 4, 5);
                item.add(&iv);
            }
        }

        // Label
        let lbl = ui::Label::new(&page.name);
        lbl.set_dock(ui::DOCK_FILL);
        lbl.set_font_size(13);
        lbl.set_text_color(if i == 0 { tc.text } else { tc.text });
        lbl.set_padding(4, 8, 8, 8);
        item.add(&lbl);

        item.on_click_raw(sidebar_click_handler, i as u64);
        sidebar_ids.push(item.id());
        sidebar_panel.add(&item);
    }

    sidebar_scroll.add(&sidebar_panel);
    split.add(&sidebar_scroll);

    // ── Right: Content ScrollView ───────────────────────────────────
    let content_scroll = ui::ScrollView::new();
    content_scroll.set_dock(ui::DOCK_FILL);
    content_scroll.set_color(tc.window_bg);

    split.add(&content_scroll);

    // ── Initialize state ────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            pages,
            sidebar_ids,
            active_idx: 0,
            content_scroll,
            sidebar_scroll_id: sidebar_scroll.id(),
            sidebar_panel_id: sidebar_panel.id(),
        });
    }

    // Build the first page (Dashboard)
    build_page(0);

    // ── Keyboard shortcuts ──────────────────────────────────────────
    win.on_key_down(|e| {
        if e.keycode == ui::KEY_ESCAPE {
            ui::quit();
        }
    });

    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}

// ── Sidebar click handler ───────────────────────────────────────────────────

extern "C" fn sidebar_click_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    switch_page(userdata as usize);
}

fn switch_page(idx: usize) {
    let s = app();
    if idx >= s.pages.len() || idx == s.active_idx {
        return;
    }

    let old_idx = s.active_idx;
    s.active_idx = idx;

    // Update sidebar highlight
    let tc = ui::theme::colors();
    if old_idx < s.sidebar_ids.len() {
        let old_lbl = ui::Control::from_id(s.sidebar_ids[old_idx]);
        old_lbl.set_color(0x00000000);
        old_lbl.set_text_color(tc.text);
    }
    if idx < s.sidebar_ids.len() {
        let new_lbl = ui::Control::from_id(s.sidebar_ids[idx]);
        new_lbl.set_color(tc.selection);
        new_lbl.set_text_color(tc.text);
    }

    // Hide old panel
    if s.pages[old_idx].panel_id != 0 {
        let old_panel = ui::Control::from_id(s.pages[old_idx].panel_id);
        old_panel.set_visible(false);
    }

    // Show/build new panel
    build_page(idx);
}

fn build_page(idx: usize) {
    let s = app();
    if idx >= s.pages.len() {
        return;
    }

    if s.pages[idx].panel_id != 0 {
        let panel = ui::Control::from_id(s.pages[idx].panel_id);
        panel.set_visible(true);
        return;
    }

    let scroll = &s.content_scroll;
    let panel_id = match s.pages[idx].id {
        BuiltinId::Dashboard => page_dashboard::build(scroll),
        BuiltinId::General => page_general::build(scroll),
        BuiltinId::Display => page_display::build(scroll),
        BuiltinId::Dock => page_dock::build(scroll),
        BuiltinId::Apps => page_apps::build(scroll),
        BuiltinId::Devices => page_devices::build(scroll),
        BuiltinId::Network => page_network::build(scroll),
        BuiltinId::Update => page_update::build(scroll),
    };
    s.pages[idx].panel_id = panel_id;
}

/// Destroy all cached pages and rebuild the active one.
///
/// Called after a theme change so that every page picks up the new palette.
pub(crate) fn invalidate_all_pages() {
    let s = app();
    let tc = ui::theme::colors();

    // Update sidebar colours
    ui::Control::from_id(s.sidebar_scroll_id).set_color(tc.sidebar_bg);
    ui::Control::from_id(s.sidebar_panel_id).set_color(tc.sidebar_bg);
    s.content_scroll.set_color(tc.window_bg);

    // Update sidebar item highlights
    for (i, &sid) in s.sidebar_ids.iter().enumerate() {
        let item = ui::Control::from_id(sid);
        if i == s.active_idx {
            item.set_color(tc.selection);
        } else {
            item.set_color(0x00000000);
        }
    }

    // Kill any running wallpaper loading timer before destroying pages
    unsafe {
        if page_display::WP_TIMER != 0 {
            ui::kill_timer(page_display::WP_TIMER);
            page_display::WP_TIMER = 0;
        }
    }

    // Destroy all cached pages and rebuild active
    let active = s.active_idx;
    for page in &mut s.pages {
        if page.panel_id != 0 {
            ui::Control::from_id(page.panel_id).remove();
            page.panel_id = 0;
        }
    }
    build_page(active);
}
