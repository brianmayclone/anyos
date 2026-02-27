#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libanyui_client as ui;
use ui::Widget;

mod types;
mod config_io;
mod module_loader;
mod form_builder;
mod layout;
mod registrar;
mod page_dashboard;
mod page_general;
mod page_display;
mod page_apps;
mod page_devices;
mod page_network;
mod page_update;

use types::*;

anyos_std::entry!(main);

// ── Global state ────────────────────────────────────────────────────────────

struct AppState {
    modules: Vec<ModuleEntry>,
    sidebar_ids: Vec<u32>,
    active_idx: usize,
    content_scroll: ui::ScrollView,
    edit_panel: ui::View,
    edit_fields: Vec<ui::TextField>,
    edit_grid_id: u32,
    edit_row: u32,
    edit_columns: Vec<ColumnSpec>,
    uid: u16,
    username: String,
    home: String,
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

    let (uid, username, home) = module_loader::resolve_user();
    let modules = module_loader::load_all_modules(uid, &username, &home);

    let win = ui::Window::new("Settings", -1, -1, 900, 600);

    // ── Edit panel (DOCK_BOTTOM, for external module table editing) ──
    let edit_panel = ui::View::new();
    edit_panel.set_dock(ui::DOCK_BOTTOM);
    edit_panel.set_size(900, 44);
    edit_panel.set_color(0xFF2D2D30);
    edit_panel.set_visible(false);
    win.add(&edit_panel);

    // ── SplitView (DOCK_FILL) ───────────────────────────────────────
    let split = ui::SplitView::new();
    split.set_dock(ui::DOCK_FILL);
    split.set_orientation(ui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(24);
    split.set_min_split(18);
    split.set_max_split(35);
    win.add(&split);

    // ── Left: Sidebar ───────────────────────────────────────────────
    let sidebar_scroll = ui::ScrollView::new();
    sidebar_scroll.set_dock(ui::DOCK_FILL);
    sidebar_scroll.set_color(0xFF202020);

    let sidebar_panel = ui::View::new();
    sidebar_panel.set_dock(ui::DOCK_FILL);
    sidebar_panel.set_color(0xFF202020);

    // Title area
    let title_area = ui::View::new();
    title_area.set_dock(ui::DOCK_TOP);
    title_area.set_size(220, 52);

    let title_lbl = ui::Label::new("Settings");
    title_lbl.set_position(16, 16);
    title_lbl.set_size(180, 28);
    title_lbl.set_font_size(18);
    title_lbl.set_text_color(0xFFFFFFFF);
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

    for (i, module) in modules.iter().enumerate() {
        // Category separator
        if module.category != last_category {
            if !last_category.is_empty() {
                let spacer = ui::View::new();
                spacer.set_dock(ui::DOCK_TOP);
                spacer.set_size(220, 8);
                sidebar_panel.add(&spacer);
            }
            // Show category headers for non-System categories
            if module.category != "System" {
                let cat_label = ui::Label::new(&module.category);
                cat_label.set_dock(ui::DOCK_TOP);
                cat_label.set_size(220, 22);
                cat_label.set_font_size(11);
                cat_label.set_text_color(0xFF888888);
                cat_label.set_margin(16, 4, 16, 2);
                sidebar_panel.add(&cat_label);
            }
            last_category = module.category.clone();
        }

        let item = ui::Label::new(&module.name);
        item.set_dock(ui::DOCK_TOP);
        item.set_size(220, 34);
        item.set_font_size(13);
        item.set_text_color(if i == 0 { 0xFFFFFFFF } else { 0xFFCCCCCC });
        item.set_padding(16, 8, 8, 8);
        item.set_margin(4, 1, 4, 1);
        if i == 0 {
            item.set_color(0xFF094771);
        }
        item.on_click_raw(sidebar_click_handler, i as u64);
        sidebar_ids.push(item.id());
        sidebar_panel.add(&item);
    }

    sidebar_scroll.add(&sidebar_panel);
    split.add(&sidebar_scroll);

    // ── Right: Content ScrollView ───────────────────────────────────
    let content_scroll = ui::ScrollView::new();
    content_scroll.set_dock(ui::DOCK_FILL);
    content_scroll.set_color(0xFF1E1E1E);

    split.add(&content_scroll);

    // ── Initialize state ────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            modules,
            sidebar_ids,
            active_idx: 0,
            content_scroll,
            edit_panel,
            edit_fields: Vec::new(),
            edit_grid_id: 0,
            edit_row: u32::MAX,
            edit_columns: Vec::new(),
            uid,
            username,
            home,
        });
    }

    // Build the first module panel (Dashboard)
    build_module_panel(0);

    // ── Keyboard shortcuts ──────────────────────────────────────────
    win.on_key_down(|e| {
        if e.keycode == ui::KEY_ESCAPE {
            hide_edit_panel();
        }
    });

    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}

// ── Sidebar click handler ───────────────────────────────────────────────────

extern "C" fn sidebar_click_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    switch_module(idx);
}

fn switch_module(idx: usize) {
    let s = app();
    if idx >= s.modules.len() || idx == s.active_idx {
        return;
    }

    let old_idx = s.active_idx;
    s.active_idx = idx;

    // Update sidebar highlight
    if old_idx < s.sidebar_ids.len() {
        let old_lbl = ui::Control::from_id(s.sidebar_ids[old_idx]);
        old_lbl.set_color(0x00000000);
        old_lbl.set_text_color(0xFFCCCCCC);
    }
    if idx < s.sidebar_ids.len() {
        let new_lbl = ui::Control::from_id(s.sidebar_ids[idx]);
        new_lbl.set_color(0xFF094771);
        new_lbl.set_text_color(0xFFFFFFFF);
    }

    // Hide old panel
    if s.modules[old_idx].panel_id != 0 {
        let old_panel = ui::Control::from_id(s.modules[old_idx].panel_id);
        old_panel.set_visible(false);
    }

    // Show/build new panel
    build_module_panel(idx);

    // Hide edit panel when switching
    hide_edit_panel();
}

fn build_module_panel(idx: usize) {
    let s = app();
    if idx >= s.modules.len() {
        return;
    }

    if s.modules[idx].panel_id != 0 {
        let panel = ui::Control::from_id(s.modules[idx].panel_id);
        panel.set_visible(true);
        return;
    }

    let scroll = &s.content_scroll;
    let panel_id = match &s.modules[idx].kind {
        ModuleKind::Builtin(id) => match id {
            BuiltinId::Dashboard => page_dashboard::build(scroll),
            BuiltinId::General => page_general::build(scroll),
            BuiltinId::Display => page_display::build(scroll),
            BuiltinId::Apps => page_apps::build(scroll),
            BuiltinId::Devices => page_devices::build(scroll),
            BuiltinId::Network => page_network::build(scroll),
            BuiltinId::Update => page_update::build(scroll),
        },
        ModuleKind::External(desc) => {
            let values = &s.modules[idx].values;
            let (pid, ctrl_ids) = form_builder::build_form(scroll, desc, values);
            s.modules[idx].field_ctrl_ids = ctrl_ids;
            pid
        }
    };
    s.modules[idx].panel_id = panel_id;
}

// ── External module: Apply / Reset ──────────────────────────────────────────

fn apply_settings() {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    let (is_external, path, format, columns, ipc_name, new_values) = {
        if let ModuleKind::External(desc) = &s.modules[idx].kind {
            let fields = &desc.fields;
            let ctrl_ids = &s.modules[idx].field_ctrl_ids;
            let table_values = &s.modules[idx].values;
            let new_values = form_builder::collect_values(fields, ctrl_ids, table_values);
            let columns = first_table_columns(fields);
            let path = desc.resolved_path.clone();
            let format = desc.config_format;
            let ipc_name = desc.on_apply_ipc.clone();
            (true, path, format, columns, ipc_name, new_values)
        } else {
            return;
        }
    };

    if is_external {
        if config_io::save_config(&path, format, &new_values, &columns) {
            s.modules[idx].values = new_values;
            s.modules[idx].dirty = false;

            if !ipc_name.is_empty() {
                let chan = anyos_std::ipc::evt_chan_create(&ipc_name);
                anyos_std::ipc::evt_chan_emit(chan, &[1, 0, 0, 0, 0]);
            }
        }
    }
}

fn reset_settings() {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    let reload_info = if let ModuleKind::External(desc) = &s.modules[idx].kind {
        let columns = first_table_columns(&desc.fields);
        let path = desc.resolved_path.clone();
        let format = desc.config_format;
        Some((path, format, columns))
    } else {
        None
    };

    if let Some((path, format, columns)) = reload_info {
        let values = config_io::load_config(&path, format, &columns);
        s.modules[idx].values = values;
        s.modules[idx].dirty = false;

        if s.modules[idx].panel_id != 0 {
            let old = ui::Control::from_id(s.modules[idx].panel_id);
            old.set_visible(false);
            s.modules[idx].panel_id = 0;
            build_module_panel(idx);
        }
    }
}

// ── Table editing (for external modules with table fields) ──────────────────

pub fn open_table_edit(grid_id: u32, row: u32) {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    let columns = if let ModuleKind::External(desc) = &s.modules[idx].kind {
        let mut cols = Vec::new();
        for f in &desc.fields {
            if let FieldType::Table { columns } = &f.field_type {
                for c in columns {
                    cols.push(ColumnSpec {
                        key: c.key.clone(),
                        label: c.label.clone(),
                        placeholder: c.placeholder.clone(),
                        width: c.width,
                    });
                }
                break;
            }
        }
        cols
    } else {
        return;
    };

    if columns.is_empty() {
        return;
    }

    s.edit_panel.set_visible(true);
    s.edit_panel.set_size(900, 44);
    s.edit_fields.clear();

    let mut x = 8i32;
    for col in &columns {
        let tf = ui::TextField::new();
        tf.set_position(x, 8);
        let w = (col.width as i32).max(60);
        tf.set_size(w as u32, 28);
        tf.set_placeholder(&col.placeholder);

        if row != u32::MAX {
            let val = get_table_cell_value(idx, &col.key, row as usize);
            if !val.is_empty() {
                tf.set_text(&val);
            }
        }

        s.edit_panel.add(&tf);
        s.edit_fields.push(tf);
        x += w + 4;
    }

    let btn_ok = ui::IconButton::new("OK");
    btn_ok.set_position(x, 8);
    btn_ok.set_size(40, 28);
    btn_ok.on_click(|_| {
        confirm_table_edit();
    });
    s.edit_panel.add(&btn_ok);

    let btn_cancel = ui::IconButton::new("X");
    btn_cancel.set_position(x + 44, 8);
    btn_cancel.set_size(28, 28);
    btn_cancel.on_click(|_| {
        hide_edit_panel();
    });
    s.edit_panel.add(&btn_cancel);

    s.edit_grid_id = grid_id;
    s.edit_row = row;
    s.edit_columns = columns;
}

fn confirm_table_edit() {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    let mut row_obj = Value::new_object();
    for (i, col) in s.edit_columns.iter().enumerate() {
        if i < s.edit_fields.len() {
            let mut buf = [0u8; 1024];
            let len = s.edit_fields[i].get_text(&mut buf) as usize;
            let text = core::str::from_utf8(&buf[..len]).unwrap_or("");
            row_obj.set(&col.key, text.into());
        }
    }

    let table_key = if let ModuleKind::External(desc) = &s.modules[idx].kind {
        let mut key = String::new();
        for f in &desc.fields {
            if let FieldType::Table { .. } = &f.field_type {
                key = f.key.clone();
                break;
            }
        }
        key
    } else {
        return;
    };

    let values = &mut s.modules[idx].values;
    if values.is_null() {
        *values = Value::new_object();
    }
    if values[table_key.as_str()].is_null() {
        values.set(table_key.as_str(), Value::new_array());
    }

    let edit_row = s.edit_row;
    if let Some(obj) = values.as_object_mut() {
        if let Some(arr_val) = obj.get_mut(table_key.as_str()) {
            if let Some(a) = arr_val.as_array_mut() {
                if edit_row == u32::MAX {
                    a.push(row_obj);
                } else if (edit_row as usize) < a.len() {
                    a[edit_row as usize] = row_obj;
                }
            }
        }
    }

    s.modules[idx].dirty = true;
    refresh_table_grid(idx, &table_key);
    hide_edit_panel();
}

pub fn delete_table_row(grid_id: u32) {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    let grid = ui::DataGrid::from_id(grid_id);
    let sel = grid.selected_row();
    if sel == u32::MAX {
        return;
    }

    let table_key = if let ModuleKind::External(desc) = &s.modules[idx].kind {
        let mut key = String::new();
        for f in &desc.fields {
            if let FieldType::Table { .. } = &f.field_type {
                key = f.key.clone();
                break;
            }
        }
        key
    } else {
        return;
    };

    if let Some(obj) = s.modules[idx].values.as_object_mut() {
        if let Some(arr_val) = obj.get_mut(table_key.as_str()) {
            if let Some(a) = arr_val.as_array_mut() {
                if (sel as usize) < a.len() {
                    a.remove(sel as usize);
                }
            }
        }
    }

    s.modules[idx].dirty = true;
    refresh_table_grid(idx, &table_key);
}

fn refresh_table_grid(idx: usize, table_key: &str) {
    let s = app();

    let columns = if let ModuleKind::External(desc) = &s.modules[idx].kind {
        let mut cols = Vec::new();
        for f in &desc.fields {
            if let FieldType::Table { columns } = &f.field_type {
                for c in columns {
                    cols.push(ColumnSpec {
                        key: c.key.clone(),
                        label: c.label.clone(),
                        placeholder: c.placeholder.clone(),
                        width: c.width,
                    });
                }
                break;
            }
        }
        cols
    } else {
        return;
    };

    if let ModuleKind::External(desc) = &s.modules[idx].kind {
        for (fi, f) in desc.fields.iter().enumerate() {
            if let FieldType::Table { .. } = &f.field_type {
                if fi < s.modules[idx].field_ctrl_ids.len() {
                    let gid = s.modules[idx].field_ctrl_ids[fi];
                    let grid = ui::DataGrid::from_id(gid);
                    form_builder::populate_grid(&grid, &s.modules[idx].values[table_key], &columns);
                }
                break;
            }
        }
    }
}

fn hide_edit_panel() {
    let s = app();
    s.edit_panel.set_visible(false);
    s.edit_grid_id = 0;
    s.edit_row = u32::MAX;
}

fn get_table_cell_value(module_idx: usize, col_key: &str, row: usize) -> String {
    let s = app();
    let table_key = if let ModuleKind::External(desc) = &s.modules[module_idx].kind {
        let mut key = String::new();
        for f in &desc.fields {
            if let FieldType::Table { .. } = &f.field_type {
                key = f.key.clone();
                break;
            }
        }
        key
    } else {
        return String::new();
    };

    let arr = &s.modules[module_idx].values[table_key.as_str()];
    if let Some(a) = arr.as_array() {
        if row < a.len() {
            return String::from(a[row][col_key].as_str().unwrap_or(""));
        }
    }
    String::new()
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn first_table_columns(fields: &[FieldDef]) -> Vec<ColumnSpec> {
    for f in fields {
        if let FieldType::Table { columns } = &f.field_type {
            return columns
                .iter()
                .map(|c| ColumnSpec {
                    key: c.key.clone(),
                    label: c.label.clone(),
                    placeholder: c.placeholder.clone(),
                    width: c.width,
                })
                .collect();
        }
    }
    Vec::new()
}
