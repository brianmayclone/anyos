//! App Store — Browse, install, update, and remove packages from apkg repos.
//!
//! Uses libanyui for the GUI, reads `/System/etc/apkg/index.json` and
//! `installed.json` directly, and spawns the `apkg` CLI for actions.

#![no_std]
#![no_main]

anyos_std::entry!(main);

mod apkg;
mod ui;

use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as anyui;
use anyui::Widget;

use crate::apkg::{PackageInfo, InstalledEntry, PkgStatus};

// ─── Constants ─────────────────────────────────────────────────────

const WIN_W: u32 = 960;
const WIN_H: u32 = 640;

// ─── Tab Indices ───────────────────────────────────────────────────

const TAB_ALL: usize = 0;
const TAB_INSTALLED: usize = 1;
const TAB_UPDATES: usize = 2;

// ─── Global Application State ──────────────────────────────────────

struct AppState {
    packages: Vec<PackageInfo>,
    installed: Vec<InstalledEntry>,
    current_tab: usize,
    search_query: String,

    // UI handles
    flow_panel: anyui::FlowPanel,
    grid_scroll: anyui::ScrollView,
    detail_view: anyui::View,
    status_label: anyui::Label,
    search_field: anyui::SearchField,

    // Detail view state
    detail_pkg_index: Option<usize>,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("APP not initialized") }
}

// ─── Filtering ─────────────────────────────────────────────────────

/// Get the list of package indices matching the current tab + search filter.
fn filtered_indices(state: &AppState) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, pkg) in state.packages.iter().enumerate() {
        let status = apkg::get_status(pkg, &state.installed);

        // Tab filter
        match state.current_tab {
            TAB_INSTALLED => {
                if status == PkgStatus::Available {
                    continue;
                }
            }
            TAB_UPDATES => {
                if status != PkgStatus::Updatable {
                    continue;
                }
            }
            _ => {} // TAB_ALL: show everything
        }

        // Search filter
        if !state.search_query.is_empty() {
            if !apkg::matches_search(&pkg.name, &state.search_query)
                && !apkg::matches_search(&pkg.description, &state.search_query)
            {
                continue;
            }
        }

        out.push(i);
    }
    out
}

// ─── Rebuild Card Grid ─────────────────────────────────────────────

/// Clear and rebuild the card grid based on current filters.
fn rebuild_cards() {
    let state = app();

    // Clear all cards from the flow panel
    state.flow_panel.clear();

    let indices = filtered_indices(state);
    let count = indices.len();

    for &pkg_idx in &indices {
        let pkg = &state.packages[pkg_idx];
        let status = apkg::get_status(pkg, &state.installed);

        let action_btn = ui::create_card(&state.flow_panel, pkg, status, pkg_idx);

        // Wire action button click
        let idx = pkg_idx;
        action_btn.on_click(move |_| {
            handle_action(idx);
        });
    }

    // Update status bar
    let status_text = match state.current_tab {
        TAB_ALL => alloc::format!("  {} packages available", count),
        TAB_INSTALLED => alloc::format!("  {} packages installed", count),
        TAB_UPDATES => alloc::format!("  {} updates available", count),
        _ => alloc::format!("  {} packages", count),
    };
    state.status_label.set_text(&status_text);
}

// ─── Action Handlers ───────────────────────────────────────────────

/// Handle install/update action from a card button.
fn handle_action(pkg_idx: usize) {
    let state = app();
    let pkg = &state.packages[pkg_idx];
    let status = apkg::get_status(pkg, &state.installed);
    let name = pkg.name.clone();

    match status {
        PkgStatus::Available => {
            state.status_label.set_text(&alloc::format!("  Installing {}...", name));
            let code = apkg::install_package(&name);
            if code == 0 {
                state.status_label.set_text(&alloc::format!("  {} installed successfully", name));
            } else {
                state.status_label.set_text(&alloc::format!("  Failed to install {} (exit {})", name, code));
            }
        }
        PkgStatus::Updatable => {
            state.status_label.set_text(&alloc::format!("  Updating {}...", name));
            let code = apkg::upgrade_package(&name);
            if code == 0 {
                state.status_label.set_text(&alloc::format!("  {} updated successfully", name));
            } else {
                state.status_label.set_text(&alloc::format!("  Failed to update {} (exit {})", name, code));
            }
        }
        PkgStatus::Installed => {
            return;
        }
    }

    // Reload installed list and refresh
    state.installed = apkg::load_installed();
    rebuild_cards();
}

/// Handle remove action from detail view.
fn handle_remove(pkg_idx: usize) {
    let state = app();
    let name = state.packages[pkg_idx].name.clone();

    state.status_label.set_text(&alloc::format!("  Removing {}...", name));
    let code = apkg::remove_package(&name);
    if code == 0 {
        state.status_label.set_text(&alloc::format!("  {} removed successfully", name));
    } else {
        state.status_label.set_text(&alloc::format!("  Failed to remove {} (exit {})", name, code));
    }

    // Reload and refresh
    state.installed = apkg::load_installed();
    show_detail(pkg_idx);
}

// ─── Detail View Navigation ───────────────────────────────────────

/// Show the detail view for a given package index.
fn show_detail(pkg_idx: usize) {
    let state = app();
    let tc = anyui::theme::colors();
    state.detail_pkg_index = Some(pkg_idx);

    // Hide the grid, show the detail
    state.grid_scroll.set_visible(false);
    state.detail_view.clear();
    state.detail_view.set_visible(true);

    let pkg = &state.packages[pkg_idx];
    let status = apkg::get_status(pkg, &state.installed);

    // Back button
    let back_btn = anyui::IconButton::new("Back");
    back_btn.set_position(8, 8);
    back_btn.set_size(60, 32);
    back_btn.set_system_icon("arrow-left", anyui::IconType::Outline, tc.text, 20);
    state.detail_view.add(&back_btn);

    back_btn.on_click(|_| {
        hide_detail();
    });

    // Action button (top-right area)
    let (btn_text, btn_color) = match status {
        PkgStatus::Available => ("Install", tc.accent),
        PkgStatus::Installed => ("Installed", tc.success),
        PkgStatus::Updatable => ("Update", tc.warning),
    };
    let action_btn = anyui::Button::new(btn_text);
    action_btn.set_position(WIN_W as i32 - 200, 16);
    action_btn.set_size(84, 32);
    action_btn.set_color(btn_color);
    action_btn.set_text_color(0xFFFFFFFF);
    if status == PkgStatus::Installed {
        action_btn.set_enabled(false);
    }
    state.detail_view.add(&action_btn);

    let idx = pkg_idx;
    action_btn.on_click(move |_| {
        handle_action(idx);
        show_detail(idx);
    });

    // Remove button (only if installed)
    if status == PkgStatus::Installed || status == PkgStatus::Updatable {
        let remove_btn = anyui::Button::new("Remove");
        remove_btn.set_position(WIN_W as i32 - 108, 16);
        remove_btn.set_size(84, 32);
        remove_btn.set_color(tc.destructive);
        remove_btn.set_text_color(0xFFFFFFFF);
        state.detail_view.add(&remove_btn);

        let idx = pkg_idx;
        remove_btn.on_click(move |_| {
            handle_remove(idx);
        });
    }

    // Populate detail content
    ui::populate_detail(&state.detail_view, pkg, status);
}

/// Hide detail view and return to card grid.
fn hide_detail() {
    let state = app();
    state.detail_pkg_index = None;
    state.detail_view.set_visible(false);
    state.grid_scroll.set_visible(true);
}

// ─── Main ──────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("[App Store] Failed to init libanyui");
        return;
    }

    let tc = anyui::theme::colors();

    // ── Window ──
    let win = anyui::Window::new("App Store", -1, -1, WIN_W, WIN_H);

    // ═══════════════════════════════════════════════════════════════
    //  Toolbar (DOCK_TOP)
    // ═══════════════════════════════════════════════════════════════

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    win.add(&toolbar);

    // Store icon + title
    let store_icon = toolbar.add_icon_button("");
    store_icon.set_system_icon("building-store", anyui::IconType::Outline, tc.accent, 22);
    store_icon.set_enabled(false);

    let title_label = toolbar.add_label("App Store");
    title_label.set_font(1);
    title_label.set_font_size(14);
    title_label.set_text_color(tc.text);

    toolbar.add_separator();

    // Refresh button
    let btn_refresh = toolbar.add_icon_button("Refresh");
    btn_refresh.set_system_icon("refresh", anyui::IconType::Outline, tc.text, 20);

    toolbar.add_separator();

    // Search field
    let search_field = anyui::SearchField::new();
    search_field.set_size(220, 28);
    search_field.set_placeholder("Search packages...");
    toolbar.add(&search_field);

    // ═══════════════════════════════════════════════════════════════
    //  Tab bar (DOCK_TOP)
    // ═══════════════════════════════════════════════════════════════

    let tab_bar = anyui::View::new();
    tab_bar.set_dock(anyui::DOCK_TOP);
    tab_bar.set_size(WIN_W, 44);
    tab_bar.set_color(anyui::theme::darken(tc.window_bg, 3));
    win.add(&tab_bar);

    let segments = anyui::SegmentedControl::new("All Apps|Installed|Updates");
    segments.set_position((WIN_W as i32 - 340) / 2, 8);
    segments.set_size(340, 28);
    tab_bar.add(&segments);

    // ═══════════════════════════════════════════════════════════════
    //  Status bar (DOCK_BOTTOM)
    // ═══════════════════════════════════════════════════════════════

    let status_label = anyui::Label::new("  Loading packages...");
    status_label.set_dock(anyui::DOCK_BOTTOM);
    status_label.set_size(WIN_W, 26);
    status_label.set_color(anyui::theme::darken(tc.window_bg, 5));
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(11);
    win.add(&status_label);

    // ═══════════════════════════════════════════════════════════════
    //  Content area (DOCK_FILL)
    // ═══════════════════════════════════════════════════════════════

    // Card grid (ScrollView → FlowPanel)
    let grid_scroll = anyui::ScrollView::new();
    grid_scroll.set_dock(anyui::DOCK_FILL);
    win.add(&grid_scroll);

    let flow_panel = anyui::FlowPanel::new();
    flow_panel.set_dock(anyui::DOCK_FILL);
    flow_panel.set_padding(12, 12, 12, 12);
    grid_scroll.add(&flow_panel);

    // Detail view (initially hidden)
    let detail_view = anyui::View::new();
    detail_view.set_dock(anyui::DOCK_FILL);
    detail_view.set_color(tc.window_bg);
    detail_view.set_visible(false);
    win.add(&detail_view);

    // ═══════════════════════════════════════════════════════════════
    //  Load data and initialize state
    // ═══════════════════════════════════════════════════════════════

    let packages = apkg::load_index();
    let installed = apkg::load_installed();

    unsafe {
        APP = Some(AppState {
            packages,
            installed,
            current_tab: TAB_ALL,
            search_query: String::new(),
            flow_panel,
            grid_scroll,
            detail_view,
            status_label,
            search_field,
            detail_pkg_index: None,
        });
    }

    rebuild_cards();

    // ═══════════════════════════════════════════════════════════════
    //  Event Handlers
    // ═══════════════════════════════════════════════════════════════

    // Tab switching
    segments.on_active_changed(|e| {
        let state = app();
        state.current_tab = e.index as usize;
        if state.detail_pkg_index.is_some() {
            hide_detail();
        }
        rebuild_cards();
    });

    // Search
    app().search_field.on_text_changed(|e| {
        let state = app();
        let mut buf = [0u8; 256];
        let len = anyui::Control::from_id(e.id).get_text(&mut buf);
        let query = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        state.search_query = String::from(query);
        rebuild_cards();
    });

    // Refresh button — update index then reload
    btn_refresh.on_click(|_| {
        let state = app();
        state.status_label.set_text("  Updating package index...");
        let code = apkg::update_index();
        if code == 0 {
            state.packages = apkg::load_index();
            state.installed = apkg::load_installed();
            state.status_label.set_text("  Package index updated");
            rebuild_cards();
        } else {
            state.status_label.set_text("  Failed to update index");
        }
    });

    // ── Run event loop ──
    anyui::run();
}
