#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client as ui;
use libconf::{ConfClient, ConfError, ConfItem, ConfValue, NodeKind, RegistryScope};

anyos_std::entry!(main);

const WIN_W: u32 = 1120;
const WIN_H: u32 = 720;
const TOOLBAR_H: u32 = 40;
const STATUS_H: u32 = 28;
const DETAIL_H: u32 = 230;
const SIDEBAR_W: u32 = 260;
const REFRESH_MS: u32 = 1500;

struct TreeNode {
    id: u32,
    path: String,
}

struct App {
    client: Option<ConfClient>,
    items: Vec<ConfItem>,
    visible: Vec<usize>,
    tree_nodes: Vec<TreeNode>,
    current_dir: String,
    selected_path: String,
    search_text: String,
    grid: ui::DataGrid,
    tree: ui::TreeView,
    status: ui::Label,
    connection: ui::Label,
    heading: ui::Label,
    scope_picker: ui::DropDown,
    path_field: ui::TextField,
    type_picker: ui::DropDown,
    value_editor: ui::TextEditor,
    audit_editor: ui::TextEditor,
}

anyos_std::global_app_state!(App);

fn main() {
    if !ui::init() {
        anyos_std::println!("configexplorer: failed to load libanyui.so");
        return;
    }

    let tc = ui::theme::colors();
    let win = ui::Window::new("Config Explorer", -1, -1, WIN_W, WIN_H);

    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, TOOLBAR_H);
    toolbar.set_padding(8, 5, 8, 5);
    win.add(&toolbar);

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_icon(ui::ICON_REFRESH);
    btn_refresh.set_size(32, 28);

    toolbar.add_separator();
    let title = toolbar.add_label("Config Explorer");
    title.set_font_size(15);
    title.set_text_color(tc.text);
    title.set_size(170, 26);

    toolbar.add_separator();
    let scope_picker = ui::DropDown::new("System|User");
    scope_picker.set_size(120, 28);
    toolbar.add(&scope_picker);

    let search = ui::SearchField::new();
    search.set_placeholder("Search path, value, kind...");
    search.set_size(280, 28);
    toolbar.add(&search);

    let connection = toolbar.add_label("Connecting...");
    connection.set_font_size(12);
    connection.set_text_color(tc.warning);
    connection.set_size(180, 22);

    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, STATUS_H);
    status_bar.set_color(tc.toolbar_bg);
    status_bar.set_padding(10, 6, 10, 6);
    win.add(&status_bar);

    let status = ui::Label::new("Waiting for confd...");
    status.set_position(10, 6);
    status.set_size(WIN_W - 20, 16);
    status.set_text_color(tc.text_secondary);
    status.set_font_size(12);
    status_bar.add(&status);

    let detail_view = ui::View::new();
    detail_view.set_dock(ui::DOCK_BOTTOM);
    detail_view.set_size(WIN_W, DETAIL_H);
    detail_view.set_color(tc.editor_bg);
    detail_view.set_padding(12, 10, 12, 10);
    win.add(&detail_view);

    let heading = ui::Label::new("System Root");
    heading.set_position(12, 10);
    heading.set_size(420, 18);
    heading.set_text_color(tc.text_secondary);
    heading.set_font_size(11);
    detail_view.add(&heading);

    let path_label = ui::Label::new("Path");
    path_label.set_position(12, 34);
    path_label.set_size(60, 16);
    detail_view.add(&path_label);

    let path_field = ui::TextField::new();
    path_field.set_position(12, 54);
    path_field.set_size(430, 28);
    path_field.set_placeholder("services/httpd/config/log");
    detail_view.add(&path_field);

    let type_label = ui::Label::new("Type");
    type_label.set_position(454, 34);
    type_label.set_size(60, 16);
    detail_view.add(&type_label);

    let type_picker = ui::DropDown::new("Directory|String|Int|Bool");
    type_picker.set_position(454, 54);
    type_picker.set_size(120, 28);
    detail_view.add(&type_picker);

    let btn_save = ui::Button::new("Save");
    btn_save.set_position(588, 52);
    btn_save.set_size(96, 30);
    detail_view.add(&btn_save);

    let btn_delete = ui::Button::new("Delete");
    btn_delete.set_position(692, 52);
    btn_delete.set_size(96, 30);
    detail_view.add(&btn_delete);

    let value_label = ui::Label::new("Value");
    value_label.set_position(12, 92);
    value_label.set_size(60, 16);
    detail_view.add(&value_label);

    let value_editor = ui::TextEditor::new(540, 108);
    value_editor.set_position(12, 112);
    value_editor.set_editor_font(4, 12);
    detail_view.add(&value_editor);

    let audit_label = ui::Label::new("Audit");
    audit_label.set_position(566, 92);
    audit_label.set_size(60, 16);
    detail_view.add(&audit_label);

    let audit_editor = ui::TextEditor::new(530, 108);
    audit_editor.set_position(566, 112);
    audit_editor.set_editor_font(4, 12);
    audit_editor.set_read_only(true);
    detail_view.add(&audit_editor);

    let sep_bottom = ui::Divider::new();
    sep_bottom.set_dock(ui::DOCK_BOTTOM);
    sep_bottom.set_size(WIN_W, 1);
    win.add(&sep_bottom);

    let sidebar = ui::View::new();
    sidebar.set_dock(ui::DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, WIN_H - TOOLBAR_H - DETAIL_H - STATUS_H - 1);
    sidebar.set_color(tc.sidebar_bg);
    sidebar.set_padding(8, 8, 8, 8);
    win.add(&sidebar);

    let sidebar_title = ui::Label::new("Namespaces");
    sidebar_title.set_dock(ui::DOCK_TOP);
    sidebar_title.set_size(SIDEBAR_W - 20, 22);
    sidebar_title.set_text_color(tc.text_secondary);
    sidebar_title.set_font_size(11);
    sidebar.add(&sidebar_title);

    let tree = ui::TreeView::new(SIDEBAR_W - 16, WIN_H);
    tree.set_dock(ui::DOCK_FILL);
    tree.set_size(SIDEBAR_W - 16, WIN_H - TOOLBAR_H - DETAIL_H - STATUS_H - 40);
    tree.set_indent_width(14);
    tree.set_row_height(22);
    sidebar.add(&tree);

    let sep_side = ui::Divider::new();
    sep_side.set_dock(ui::DOCK_LEFT);
    sep_side.set_size(1, WIN_H);
    win.add(&sep_side);

    let grid = ui::DataGrid::new(WIN_W - SIDEBAR_W, WIN_H);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_row_height(22);
    grid.set_header_height(26);
    grid.set_font_size(12);
    let cols = alloc::vec![
        ui::ColumnDef::new("Entry").width(240),
        ui::ColumnDef::new("Kind").width(80),
        ui::ColumnDef::new("Value").width(360),
        ui::ColumnDef::new("Version").width(72).align(ui::ALIGN_RIGHT).numeric(),
        ui::ColumnDef::new("Updated").width(90).align(ui::ALIGN_RIGHT).numeric(),
    ];
    grid.set_columns(&cols);
    win.add(&grid);

    unsafe {
        APP = Some(App {
            client: None,
            items: Vec::new(),
            visible: Vec::new(),
            tree_nodes: Vec::new(),
            current_dir: String::new(),
            selected_path: String::new(),
            search_text: String::new(),
            grid,
            tree,
            status,
            connection,
            heading,
            scope_picker,
            path_field,
            type_picker,
            value_editor,
            audit_editor,
        });
    }

    app().tree.on_selection_changed(|e| {
        if let Some(path) = tree_path_for_node(e.index) {
            let a = app();
            a.current_dir = path;
            a.heading.set_text(&heading_for_scope(scope_from_ui(), &a.current_dir));
            refresh_visible(a);
        }
    });

    app().grid.on_selection_changed(|e| {
        show_detail(e.index);
    });

    search.on_text_changed(|e| {
        let mut buf = [0u8; 256];
        let n = ui::Control::from_id(e.id).get_text(&mut buf) as usize;
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
        let a = app();
        a.search_text = to_lower(text);
        refresh_visible(a);
    });

    app().scope_picker.on_selection_changed(|_| {
        let a = app();
        a.current_dir.clear();
        a.selected_path.clear();
        reload_snapshot();
    });

    btn_refresh.on_click(|_| reload_snapshot());
    btn_save.on_click(|_| save_current());
    btn_delete.on_click(|_| delete_current());

    ui::set_timer(REFRESH_MS, || reload_snapshot());
    reload_snapshot();
    ui::run();
}

fn scope_from_ui() -> RegistryScope {
    if app().scope_picker.selected_index() == 0 {
        RegistryScope::System
    } else {
        RegistryScope::User
    }
}

fn scope_name(scope: RegistryScope) -> &'static str {
    match scope {
        RegistryScope::System => "SYSTEM",
        RegistryScope::User => "USER",
    }
}

fn error_summary(err: &ConfError) -> String {
    match err {
        ConfError::NotRunning => String::from("NotRunning"),
        ConfError::PipeCreateFailed => String::from("PipeCreateFailed"),
        ConfError::Disconnected => String::from("Disconnected"),
        ConfError::Timeout => String::from("Timeout"),
        ConfError::Protocol(msg) => format!("Protocol({})", msg),
        ConfError::Remote(msg) => format!("Remote({})", msg),
        ConfError::InvalidArgument(msg) => format!("InvalidArgument({})", msg),
    }
}

fn reload_snapshot() {
    let a = app();
    let tc = ui::theme::colors();
    let scope = scope_from_ui();

    anyos_std::println!(
        "configexplorer: reload scope={} dir='{}' client_present={}",
        scope_name(scope),
        a.current_dir.as_str(),
        a.client.is_some()
    );

    if a.client.is_none() {
        match ConfClient::connect("configexplorer") {
            Ok(client) => {
                anyos_std::println!(
                    "configexplorer: connect OK scope={}",
                    scope_name(scope)
                );
                a.client = Some(client);
                a.connection.set_text("Live");
                a.connection.set_text_color(tc.success);
            }
            Err(err) => {
                anyos_std::println!(
                    "configexplorer: connect FAILED scope={} err={}",
                    scope_name(scope),
                    error_summary(&err)
                );
                a.connection.set_text("Disconnected");
                a.connection.set_text_color(tc.destructive);
                a.status.set_text("confd is not reachable yet.");
                a.grid.set_row_count(0);
                a.tree.clear();
                a.tree_nodes.clear();
                return;
            }
        }
    }

    let result = {
        let client = a.client.as_mut().unwrap();
        client.list(scope, "")
    };

    match result {
        Ok(items) => {
            anyos_std::println!(
                "configexplorer: list OK scope={} items={}",
                scope_name(scope),
                items.len()
            );
            a.items = items;
            rebuild_tree(a);
            refresh_visible(a);
            restore_selection(a);
        }
        Err(err) => {
            anyos_std::println!(
                "configexplorer: list FAILED scope={} err={}",
                scope_name(scope),
                error_summary(&err)
            );
            a.grid.set_row_count(0);
            a.tree.clear();
            a.tree_nodes.clear();
            match err {
                ConfError::Remote(message) if message == "forbidden" => {
                    a.connection.set_text("Live");
                    a.connection.set_text_color(tc.success);
                    a.status.set_text("SYSTEM scope is forbidden for the current identity.");
                }
                ConfError::Timeout => {
                    a.connection.set_text("Slow");
                    a.connection.set_text_color(tc.warning);
                    a.status.set_text("Registry snapshot timed out. Retrying automatically.");
                }
                _ => {
                    a.client = None;
                    a.connection.set_text("Retrying...");
                    a.connection.set_text_color(tc.warning);
                    a.status.set_text("Lost connection to confd. Will retry automatically.");
                }
            }
        }
    }
}

fn rebuild_tree(a: &mut App) {
    let selected = a.current_dir.clone();
    a.tree.clear();
    a.tree_nodes.clear();

    let root_label = if scope_from_ui() == RegistryScope::System {
        "System Registry"
    } else {
        "User Registry"
    };
    let root = a.tree.add_root(root_label);
    a.tree.set_node_style(root, ui::STYLE_BOLD);
    a.tree_nodes.push(TreeNode { id: root, path: String::new() });

    let dirs = collect_dirs(&a.items);
    for dir in &dirs {
        ensure_tree_path(a, root, dir);
    }

    let selected_node = find_tree_node_for_path(&a.tree_nodes, &selected).unwrap_or(root);
    a.tree.set_selected(selected_node);
}

fn ensure_tree_path(a: &mut App, root: u32, path: &str) {
    let mut current = String::new();
    let mut parent = root;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);

        if let Some(node_id) = find_tree_node_for_path(&a.tree_nodes, &current) {
            parent = node_id;
            continue;
        }

        let node = a.tree.add_child(parent, segment);
        a.tree_nodes.push(TreeNode { id: node, path: current.clone() });
        parent = node;
    }
}

fn refresh_visible(a: &mut App) {
    a.visible.clear();
    for (idx, item) in a.items.iter().enumerate() {
        if !is_direct_child_of(&item.path, &a.current_dir) {
            continue;
        }
        if !matches_search(item, &a.search_text) {
            continue;
        }
        a.visible.push(idx);
    }
    populate_grid(a);
}

fn populate_grid(a: &App) {
    let tc = ui::theme::colors();
    let mut text_colors = alloc::vec![0u32; a.visible.len() * 5];
    let mut bg_colors = alloc::vec![0u32; a.visible.len() * 5];
    a.grid.set_row_count(a.visible.len() as u32);

    for (row, idx) in a.visible.iter().enumerate() {
        let item = &a.items[*idx];
        let name = last_segment(&item.path);
        a.grid.set_cell(row as u32, 0, &name);
        a.grid.set_cell(row as u32, 1, kind_name(item));
        a.grid.set_cell(row as u32, 2, &value_text(item));
        a.grid.set_cell(row as u32, 3, &format!("{}", item.version));
        a.grid.set_cell(row as u32, 4, &format!("{} ms", item.updated_at));

        let base = if row % 2 == 0 { tc.window_bg } else { tc.alt_row_bg };
        for col in 0..5 {
            bg_colors[row * 5 + col] = base;
        }
        text_colors[row * 5 + 1] = match item.kind {
            NodeKind::Directory => tc.accent,
            NodeKind::Value => tc.text_secondary,
        };
    }

    a.grid.set_cell_colors(&text_colors);
    a.grid.set_cell_bg_colors(&bg_colors);
    a.status.set_text(&format!(
        "{} entries in {}{}",
        a.visible.len(),
        if a.current_dir.is_empty() { "/" } else { a.current_dir.as_str() },
        if a.search_text.is_empty() { "" } else { " (filtered)" }
    ));
}

fn restore_selection(a: &mut App) {
    if a.visible.is_empty() {
        a.path_field.set_text("");
        a.value_editor.set_text("");
        a.audit_editor.set_text("");
        return;
    }

    for (row, idx) in a.visible.iter().enumerate() {
        if a.items[*idx].path == a.selected_path {
            a.grid.set_selected_row(row as u32);
            show_detail(row as u32);
            return;
        }
    }

    a.grid.set_selected_row(0);
    show_detail(0);
}

fn show_detail(row: u32) {
    let a = app();
    if (row as usize) >= a.visible.len() {
        return;
    }

    let item = &a.items[a.visible[row as usize]];
    a.selected_path = item.path.clone();
    a.path_field.set_text(&item.path);
    a.value_editor.set_text(&value_text(item));
    a.type_picker.set_selected_index(match item.kind {
        NodeKind::Directory => 0,
        NodeKind::Value => match item.value {
            Some(ConfValue::String(_)) => 1,
            Some(ConfValue::Int(_)) => 2,
            Some(ConfValue::Bool(_)) => 3,
            None => 1,
        },
    });
    a.heading.set_text(&heading_for_scope(scope_from_ui(), &parent_dir(&item.path)));
    load_audit(&item.path);
}

fn load_audit(path: &str) {
    let a = app();
    let text = if let Some(client) = a.client.as_mut() {
        match client.audit(scope_from_ui(), path, 8) {
            Ok(entries) => {
                if entries.is_empty() {
                    String::from("No audit records yet.")
                } else {
                    let mut out = String::new();
                    for entry in &entries {
                        out.push_str(&format!(
                            "#{} {} {} by {} [{}]\n",
                            entry.seq, entry.action, entry.status, entry.actor_name, entry.path
                        ));
                    }
                    out
                }
            }
            Err(_) => String::from("Audit unavailable."),
        }
    } else {
        String::from("Audit unavailable.")
    };
    a.audit_editor.set_text(&text);
}

fn save_current() {
    let path = read_text_field(&app().path_field);
    if path.is_empty() {
        app().status.set_text("Path is required.");
        return;
    }

    let scope = scope_from_ui();
    let result = {
        let a = app();
        let Some(client) = a.client.as_mut() else {
            a.status.set_text("confd is not connected.");
            return;
        };
        match a.type_picker.selected_index() {
            0 => client.mkdir(scope, &path),
            1 => client.set(scope, &path, ConfValue::String(read_text_editor(&a.value_editor))),
            2 => match parse_i64(&read_text_editor(&a.value_editor)) {
                Some(value) => client.set(scope, &path, ConfValue::Int(value)),
                None => {
                    a.status.set_text("Invalid integer value.");
                    return;
                }
            },
            3 => match parse_bool(&read_text_editor(&a.value_editor)) {
                Some(value) => client.set(scope, &path, ConfValue::Bool(value)),
                None => {
                    a.status.set_text("Boolean must be true/false/1/0/yes/no.");
                    return;
                }
            },
            _ => {
                a.status.set_text("Unsupported type.");
                return;
            }
        }
    };

    match result {
        Ok(_) => {
            let a = app();
            a.selected_path = path;
            a.status.set_text("Saved.");
            reload_snapshot();
        }
        Err(_) => app().status.set_text("Save failed."),
    }
}

fn delete_current() {
    let path = read_text_field(&app().path_field);
    if path.is_empty() {
        app().status.set_text("Select a key or directory first.");
        return;
    }

    let result = {
        let a = app();
        let Some(client) = a.client.as_mut() else {
            a.status.set_text("confd is not connected.");
            return;
        };
        client.del(scope_from_ui(), &path)
    };

    match result {
        Ok(_) => {
            let a = app();
            a.selected_path.clear();
            a.status.set_text("Deleted.");
            reload_snapshot();
        }
        Err(_) => app().status.set_text("Delete failed."),
    }
}

fn collect_dirs(items: &[ConfItem]) -> Vec<String> {
    let mut dirs = Vec::new();
    for item in items {
        let mut path = match item.kind {
            NodeKind::Directory => item.path.clone(),
            NodeKind::Value => parent_dir(&item.path),
        };
        while !path.is_empty() {
            insert_sorted_unique(&mut dirs, &path);
            path = parent_dir(&path);
        }
    }
    dirs
}

fn tree_path_for_node(node_id: u32) -> Option<String> {
    for node in &app().tree_nodes {
        if node.id == node_id {
            return Some(node.path.clone());
        }
    }
    None
}

fn find_tree_node_for_path(nodes: &[TreeNode], path: &str) -> Option<u32> {
    for node in nodes {
        if node.path == path {
            return Some(node.id);
        }
    }
    None
}

fn is_direct_child_of(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return !path.is_empty() && !path.contains('/');
    }
    if path == dir || !path.starts_with(dir) {
        return false;
    }
    let Some(rest) = path.strip_prefix(dir) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    !rest.is_empty() && !rest.contains('/')
}

fn matches_search(item: &ConfItem, search: &str) -> bool {
    if search.is_empty() {
        return true;
    }
    let path = to_lower(&item.path);
    let kind = to_lower(kind_name(item));
    let value = to_lower(&value_text(item));
    path.contains(search) || kind.contains(search) || value.contains(search)
}

fn heading_for_scope(scope: RegistryScope, dir: &str) -> String {
    let scope_name = if scope == RegistryScope::System { "System" } else { "User" };
    if dir.is_empty() {
        format!("{} Root", scope_name)
    } else {
        format!("{}: {}", scope_name, dir)
    }
}

fn kind_name(item: &ConfItem) -> &'static str {
    match item.kind {
        NodeKind::Directory => "dir",
        NodeKind::Value => match item.value {
            Some(ConfValue::String(_)) => "string",
            Some(ConfValue::Int(_)) => "int",
            Some(ConfValue::Bool(_)) => "bool",
            None => "value",
        },
    }
}

fn value_text(item: &ConfItem) -> String {
    match &item.value {
        Some(ConfValue::String(value)) => value.clone(),
        Some(ConfValue::Int(value)) => format!("{}", *value),
        Some(ConfValue::Bool(value)) => {
            if *value { String::from("true") } else { String::from("false") }
        }
        None => String::new(),
    }
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => String::from(&path[..pos]),
        None => String::new(),
    }
}

fn last_segment(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => String::from(&path[pos + 1..]),
        None => String::from(path),
    }
}

fn read_text_field(field: &ui::TextField) -> String {
    let mut buf = [0u8; 512];
    let n = field.get_text(&mut buf) as usize;
    String::from(core::str::from_utf8(&buf[..n]).unwrap_or("").trim())
}

fn read_text_editor(editor: &ui::TextEditor) -> String {
    let mut buf = alloc::vec![0u8; 8192];
    let n = editor.get_text(&mut buf) as usize;
    String::from(core::str::from_utf8(&buf[..n]).unwrap_or(""))
}

fn parse_i64(text: &str) -> Option<i64> {
    let bytes = text.trim().as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut idx = 0usize;
    let mut negative = false;
    if bytes[0] == b'-' {
        negative = true;
        idx = 1;
    }
    let mut value = 0i64;
    while idx < bytes.len() {
        let b = bytes[idx];
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as i64)?;
        idx += 1;
    }
    if negative { Some(-value) } else { Some(value) }
}

fn parse_bool(text: &str) -> Option<bool> {
    match to_lower(text.trim()).as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn insert_sorted_unique(items: &mut Vec<String>, value: &str) {
    if items.iter().any(|item| item == value) {
        return;
    }
    items.push(String::from(value));
    items.sort();
}

fn to_lower(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        for lower in ch.to_lowercase() {
            out.push(lower);
        }
    }
    out
}
