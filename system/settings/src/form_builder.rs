//! Build anyui controls dynamically from module field descriptors.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libanyui_client as ui;
use ui::Widget;

use crate::types::*;

/// Build the form UI for an external module inside a parent View.
/// Returns (panel_id, Vec<field_control_ids>).
pub fn build_form(
    parent: &ui::ScrollView,
    descriptor: &ModuleDescriptor,
    values: &Value,
) -> (u32, Vec<u32>) {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Description at top
    if !descriptor.description.is_empty() {
        let desc = ui::Label::new(&descriptor.description);
        desc.set_dock(ui::DOCK_TOP);
        desc.set_size(560, 28);
        desc.set_font_size(12);
        desc.set_text_color(0xFF808080);
        desc.set_margin(16, 8, 16, 4);
        panel.add(&desc);
    }

    let mut ctrl_ids = Vec::new();

    for field in &descriptor.fields {
        let ctrl_id = match &field.field_type {
            FieldType::Text => build_text_row(&panel, field, values),
            FieldType::Number { .. } => build_text_row(&panel, field, values),
            FieldType::Bool => build_bool_row(&panel, field, values),
            FieldType::Choice { options } => build_choice_row(&panel, field, values, options),
            FieldType::Path { browse_folder } => build_path_row(&panel, field, values, *browse_folder),
            FieldType::Table { columns } => build_table_row(&panel, field, values, columns),
        };
        ctrl_ids.push(ctrl_id);
    }

    parent.add(&panel);
    (panel.id(), ctrl_ids)
}

/// Read current values from the form controls back into a Value.
pub fn collect_values(fields: &[FieldDef], ctrl_ids: &[u32], table_values: &Value) -> Value {
    let mut obj = Value::new_object();
    for (i, field) in fields.iter().enumerate() {
        if i >= ctrl_ids.len() {
            break;
        }
        let ctrl_id = ctrl_ids[i];
        match &field.field_type {
            FieldType::Text | FieldType::Number { .. } | FieldType::Path { .. } => {
                let ctrl = ui::Control::from_id(ctrl_id);
                let mut buf = [0u8; 1024];
                let len = ctrl.get_text(&mut buf) as usize;
                let text = core::str::from_utf8(&buf[..len]).unwrap_or("");
                obj.set(&field.key, text.into());
            }
            FieldType::Bool => {
                let ctrl = ui::Control::from_id(ctrl_id);
                let checked = ctrl.get_state() != 0;
                obj.set(&field.key, checked.into());
            }
            FieldType::Choice { options } => {
                let ctrl = ui::Control::from_id(ctrl_id);
                let idx = ctrl.get_state() as usize;
                if idx < options.len() {
                    obj.set(&field.key, options[idx].as_str().into());
                }
            }
            FieldType::Table { .. } => {
                // Table values are maintained in ModuleEntry.values directly
                let arr = &table_values[field.key.as_str()];
                if !arr.is_null() {
                    obj.set(&field.key, arr.clone());
                } else {
                    obj.set(&field.key, Value::new_array());
                }
            }
        }
    }
    obj
}

// ── Row builders ────────────────────────────────────────────────────────────

fn build_text_row(parent: &ui::View, field: &FieldDef, values: &Value) -> u32 {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(560, 36);
    row.set_margin(16, 4, 16, 4);

    let label = ui::Label::new(&field.label);
    label.set_position(0, 8);
    label.set_size(180, 20);
    label.set_text_color(0xFFCCCCCC);
    label.set_font_size(13);
    row.add(&label);

    let tf = ui::TextField::new();
    tf.set_position(190, 4);
    tf.set_size(350, 28);

    // Current value
    let val = match &values[field.key.as_str()] {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            use core::fmt::Write;
            let mut s = String::new();
            let _ = write!(s, "{}", n);
            s
        }
        _ => String::from(&field.default_str),
    };
    if !val.is_empty() {
        tf.set_text(&val);
    }

    // Placeholder for number fields
    if let FieldType::Number { min, max } = &field.field_type {
        if *min != i64::MIN || *max != i64::MAX {
            use core::fmt::Write;
            let mut ph = String::new();
            let _ = write!(ph, "{}..{}", min, max);
            tf.set_placeholder(&ph);
        }
    }

    row.add(&tf);
    parent.add(&row);
    tf.id()
}

fn build_bool_row(parent: &ui::View, field: &FieldDef, values: &Value) -> u32 {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(560, 36);
    row.set_margin(16, 4, 16, 4);

    let label = ui::Label::new(&field.label);
    label.set_position(0, 8);
    label.set_size(180, 20);
    label.set_text_color(0xFFCCCCCC);
    label.set_font_size(13);
    row.add(&label);

    let current = values[field.key.as_str()]
        .as_bool()
        .or_else(|| values[field.key.as_str()].as_str().map(|s| s == "true"))
        .unwrap_or(field.default_str == "true");
    let toggle = ui::Toggle::new(current);
    toggle.set_position(190, 4);
    row.add(&toggle);

    parent.add(&row);
    toggle.id()
}

fn build_choice_row(parent: &ui::View, field: &FieldDef, values: &Value, options: &[String]) -> u32 {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(560, 36);
    row.set_margin(16, 4, 16, 4);

    let label = ui::Label::new(&field.label);
    label.set_position(0, 8);
    label.set_size(180, 20);
    label.set_text_color(0xFFCCCCCC);
    label.set_font_size(13);
    row.add(&label);

    // Build pipe-separated label string
    let mut labels = String::new();
    for (i, opt) in options.iter().enumerate() {
        if i > 0 {
            labels.push('|');
        }
        labels.push_str(opt);
    }
    let seg = ui::SegmentedControl::new(&labels);
    seg.set_position(190, 4);
    seg.set_size(350, 28);

    // Set current selection
    let current = values[field.key.as_str()].as_str().unwrap_or(&field.default_str);
    for (i, opt) in options.iter().enumerate() {
        if opt.as_str() == current {
            seg.set_state(i as u32);
            break;
        }
    }

    row.add(&seg);
    parent.add(&row);
    seg.id()
}

fn build_path_row(parent: &ui::View, field: &FieldDef, values: &Value, browse_folder: bool) -> u32 {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(560, 36);
    row.set_margin(16, 4, 16, 4);

    let label = ui::Label::new(&field.label);
    label.set_position(0, 8);
    label.set_size(180, 20);
    label.set_text_color(0xFFCCCCCC);
    label.set_font_size(13);
    row.add(&label);

    let tf = ui::TextField::new();
    tf.set_position(190, 4);
    tf.set_size(310, 28);
    let val = values[field.key.as_str()].as_str().unwrap_or(&field.default_str);
    if !val.is_empty() {
        tf.set_text(val);
    }
    row.add(&tf);

    let browse_btn = ui::IconButton::new("...");
    browse_btn.set_position(506, 4);
    browse_btn.set_size(34, 28);
    let tf_id = tf.id();
    let is_folder = browse_folder;
    browse_btn.on_click(move |_| {
        let path = if is_folder {
            ui::FileDialog::open_folder()
        } else {
            ui::FileDialog::open_file()
        };
        if let Some(p) = path {
            ui::Control::from_id(tf_id).set_text(&p);
        }
    });
    row.add(&browse_btn);

    parent.add(&row);
    tf.id()
}

fn build_table_row(parent: &ui::View, field: &FieldDef, values: &Value, columns: &[ColumnSpec]) -> u32 {
    let group = ui::GroupBox::new(&field.label);
    group.set_dock(ui::DOCK_TOP);
    group.set_size(560, 280);
    group.set_margin(16, 8, 16, 8);

    // Button bar
    let btn_bar = ui::View::new();
    btn_bar.set_dock(ui::DOCK_TOP);
    btn_bar.set_size(540, 32);

    let btn_add = ui::IconButton::new("Add");
    btn_add.set_position(0, 2);
    btn_add.set_size(50, 28);
    btn_bar.add(&btn_add);

    let btn_edit = ui::IconButton::new("Edit");
    btn_edit.set_position(54, 2);
    btn_edit.set_size(50, 28);
    btn_bar.add(&btn_edit);

    let btn_delete = ui::IconButton::new("Delete");
    btn_delete.set_position(108, 2);
    btn_delete.set_size(60, 28);
    btn_bar.add(&btn_delete);

    group.add(&btn_bar);

    // DataGrid
    let col_defs: Vec<ui::ColumnDef> = columns
        .iter()
        .map(|c| ui::ColumnDef::new(&c.label).width(c.width))
        .collect();

    let grid = ui::DataGrid::new(540, 220);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_columns(&col_defs);
    grid.set_row_height(22);
    grid.set_header_height(24);

    // Populate grid from values
    populate_grid(&grid, &values[field.key.as_str()], columns);

    group.add(&grid);
    parent.add(&group);

    // Wire up button callbacks
    let grid_id = grid.id();
    btn_add.on_click(move |_| {
        crate::open_table_edit(grid_id, u32::MAX);
    });
    btn_edit.on_click(move |_| {
        let sel = ui::DataGrid::from_id(grid_id).selected_row();
        if sel != u32::MAX {
            crate::open_table_edit(grid_id, sel);
        }
    });
    btn_delete.on_click(move |_| {
        crate::delete_table_row(grid_id);
    });
    grid.on_submit(move |_| {
        let sel = ui::DataGrid::from_id(grid_id).selected_row();
        if sel != u32::MAX {
            crate::open_table_edit(grid_id, sel);
        }
    });

    grid.id()
}

/// Populate a DataGrid from a JSON array of objects.
pub fn populate_grid(grid: &ui::DataGrid, values: &Value, columns: &[ColumnSpec]) {
    if let Some(arr) = values.as_array() {
        grid.set_row_count(arr.len() as u32);
        if arr.is_empty() {
            return;
        }
        // Build data_raw: rows separated by 0x1E, columns by 0x1F
        let mut data = Vec::new();
        for (ri, row) in arr.iter().enumerate() {
            if ri > 0 {
                data.push(0x1E);
            }
            for (ci, col) in columns.iter().enumerate() {
                if ci > 0 {
                    data.push(0x1F);
                }
                let val = row[col.key.as_str()].as_str().unwrap_or("");
                data.extend_from_slice(val.as_bytes());
            }
        }
        grid.set_data_raw(&data);
    } else {
        grid.set_row_count(0);
    }
}
