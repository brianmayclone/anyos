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
mod builtin_general;
mod builtin_display;
mod builtin_wallpaper;
mod builtin_network;
mod builtin_apps;
mod builtin_about;

use types::*;

anyos_std::entry!(main);

// ── Global state ────────────────────────────────────────────────────────────

struct AppState {
    modules: Vec<ModuleEntry>,
    sidebar_ids: Vec<u32>,
    active_idx: usize,
    content_scroll: ui::ScrollView,
    status_label: ui::Label,
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

    let win = ui::Window::new("Settings", -1, -1, 780, 500);

    // ── Toolbar ─────────────────────────────────────────────────────────
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(780, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_apply = toolbar.add_icon_button("Apply");
    btn_apply.set_size(60, 28);
    btn_apply.on_click(|_| {
        apply_settings();
    });

    let btn_reset = toolbar.add_icon_button("Reset");
    btn_reset.set_size(60, 28);
    btn_reset.on_click(|_| {
        reset_settings();
    });

    let btn_reload = toolbar.add_icon_button("Reload");
    btn_reload.set_size(70, 28);
    btn_reload.on_click(|_| {
        reload_module();
    });

    win.add(&toolbar);

    // ── Status bar (DOCK_BOTTOM, add before DOCK_FILL) ──────────────────
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(780, 24);
    status_bar.set_color(0xFF252525);

    let status_label = ui::Label::new("Ready");
    status_label.set_position(8, 4);
    status_label.set_size(500, 16);
    status_label.set_font_size(11);
    status_label.set_text_color(0xFF808080);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Edit panel (DOCK_BOTTOM, for table row editing) ─────────────────
    let edit_panel = ui::View::new();
    edit_panel.set_dock(ui::DOCK_BOTTOM);
    edit_panel.set_size(780, 44);
    edit_panel.set_color(0xFF2D2D30);
    edit_panel.set_visible(false);
    win.add(&edit_panel);

    // ── SplitView (DOCK_FILL, add last) ─────────────────────────────────
    let split = ui::SplitView::new();
    split.set_dock(ui::DOCK_FILL);
    split.set_orientation(ui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(25);
    split.set_min_split(15);
    split.set_max_split(40);
    win.add(&split);

    // ── Left: Sidebar ───────────────────────────────────────────────────
    let sidebar_panel = ui::View::new();
    sidebar_panel.set_dock(ui::DOCK_FILL);
    sidebar_panel.set_color(0xFF1E1E1E);

    let sidebar_header = ui::Label::new("SETTINGS");
    sidebar_header.set_dock(ui::DOCK_TOP);
    sidebar_header.set_size(180, 24);
    sidebar_header.set_font_size(11);
    sidebar_header.set_text_color(0xFF777777);
    sidebar_header.set_margin(12, 8, 0, 4);
    sidebar_panel.add(&sidebar_header);

    let mut sidebar_ids: Vec<u32> = Vec::new();
    let mut last_category = String::new();

    for (i, module) in modules.iter().enumerate() {
        // Category header if changed
        if module.category != last_category {
            if !last_category.is_empty() {
                // Spacer
                let spacer = ui::View::new();
                spacer.set_dock(ui::DOCK_TOP);
                spacer.set_size(180, 8);
                sidebar_panel.add(&spacer);
            }
            if module.category != "System" {
                let cat_label = ui::Label::new(&module.category);
                cat_label.set_dock(ui::DOCK_TOP);
                cat_label.set_size(180, 20);
                cat_label.set_font_size(10);
                cat_label.set_text_color(0xFF666666);
                cat_label.set_margin(12, 2, 0, 2);
                sidebar_panel.add(&cat_label);
            }
            last_category = module.category.clone();
        }

        let item = ui::Label::new(&module.name);
        item.set_dock(ui::DOCK_TOP);
        item.set_size(180, 30);
        item.set_font_size(13);
        item.set_text_color(if i == 0 { 0xFFFFFFFF } else { 0xFFCCCCCC });
        item.set_padding(28, 6, 8, 6);
        item.set_margin(4, 1, 4, 1);
        if i == 0 {
            item.set_color(0xFF094771);
        }
        item.on_click_raw(sidebar_click_handler, i as u64);
        sidebar_ids.push(item.id());
        sidebar_panel.add(&item);
    }

    split.add(&sidebar_panel);

    // ── Right: Content ScrollView ───────────────────────────────────────
    let content_scroll = ui::ScrollView::new();
    content_scroll.set_dock(ui::DOCK_FILL);
    content_scroll.set_color(0xFF1E1E1E);

    split.add(&content_scroll);

    // ── Initialize state ────────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            modules,
            sidebar_ids,
            active_idx: 0,
            content_scroll,
            status_label,
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

    // Build the first module panel
    build_module_panel(0);

    // ── Keyboard shortcuts ──────────────────────────────────────────────
    win.on_key_down(|e| {
        if e.ctrl() && e.char_code == b's' as u32 {
            apply_settings();
        } else if e.keycode == ui::KEY_ESCAPE {
            hide_edit_panel();
        } else if e.keycode == ui::KEY_F5 {
            reload_module();
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

    // Hide edit panel
    hide_edit_panel();
    s.status_label.set_text("Ready");
}

fn build_module_panel(idx: usize) {
    let s = app();
    if idx >= s.modules.len() {
        return;
    }

    if s.modules[idx].panel_id != 0 {
        // Already built, just show
        let panel = ui::Control::from_id(s.modules[idx].panel_id);
        panel.set_visible(true);
        return;
    }

    // Build the panel
    let scroll = &s.content_scroll;
    let panel_id = match &s.modules[idx].kind {
        ModuleKind::Builtin(id) => match id {
            BuiltinId::General => builtin_general::build(scroll),
            BuiltinId::Display => builtin_display::build(scroll),
            BuiltinId::Wallpaper => builtin_wallpaper::build(scroll),
            BuiltinId::Network => builtin_network::build(scroll),
            BuiltinId::Apps => builtin_apps::build(scroll),
            BuiltinId::About => builtin_about::build(scroll),
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

// ── Apply / Reset / Reload ──────────────────────────────────────────────────

fn apply_settings() {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    // Extract what we need from the descriptor before mutating
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
            s.status_label.set_text("Built-in modules apply immediately.");
            return;
        }
    };

    if is_external {
        if config_io::save_config(&path, format, &new_values, &columns) {
            s.modules[idx].values = new_values;
            s.modules[idx].dirty = false;
            s.status_label.set_text("Settings saved.");

            if !ipc_name.is_empty() {
                let chan = anyos_std::ipc::evt_chan_create(&ipc_name);
                anyos_std::ipc::evt_chan_emit(chan, &[1, 0, 0, 0, 0]);
            }
        } else {
            s.status_label.set_text("Error saving settings.");
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
        s.status_label.set_text("Settings reset.");
    }
}

fn reload_module() {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    // For built-in modules: tear down and rebuild
    if s.modules[idx].panel_id != 0 {
        let old = ui::Control::from_id(s.modules[idx].panel_id);
        old.set_visible(false);
        s.modules[idx].panel_id = 0;
    }

    // For external: also reload values from file
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
    }

    build_module_panel(idx);
    s.status_label.set_text("Module reloaded.");
}

// ── Table editing (for external modules with table fields) ──────────────────

pub fn open_table_edit(grid_id: u32, row: u32) {
    let s = app();
    let idx = s.active_idx;
    if idx >= s.modules.len() {
        return;
    }

    // Find the table field and its columns
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

    // Clear and rebuild edit panel fields
    s.edit_panel.set_visible(true);
    // Resize for columns
    let panel_h = 44i32;
    s.edit_panel.set_size(780, panel_h as u32);

    // Remove old fields
    s.edit_fields.clear();

    let mut x = 8i32;
    for col in &columns {
        let tf = ui::TextField::new();
        tf.set_position(x, 8);
        let w = (col.width as i32).max(60);
        tf.set_size(w as u32, 28);
        tf.set_placeholder(&col.placeholder);

        // Populate from existing row data
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

    // OK button
    let btn_ok = ui::IconButton::new("OK");
    btn_ok.set_position(x, 8);
    btn_ok.set_size(40, 28);
    btn_ok.on_click(|_| {
        confirm_table_edit();
    });
    s.edit_panel.add(&btn_ok);

    // Cancel button
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

    // Build a new row object from edit fields
    let mut row_obj = Value::new_object();
    for (i, col) in s.edit_columns.iter().enumerate() {
        if i < s.edit_fields.len() {
            let mut buf = [0u8; 1024];
            let len = s.edit_fields[i].get_text(&mut buf) as usize;
            let text = core::str::from_utf8(&buf[..len]).unwrap_or("");
            row_obj.set(&col.key, text.into());
        }
    }

    // Find the table field key
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

    // Update values
    let values = &mut s.modules[idx].values;
    if values.is_null() {
        *values = Value::new_object();
    }

    // Ensure the table array exists
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

    // Refresh grid
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

    // Find table field key
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

    // Remove from values
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

    // Get columns
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

    // Find grid ID from ctrl_ids (the table field)
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
