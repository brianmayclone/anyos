#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::process;
use anyos_std::println;
use anyos_std::{String, Vec, format, vec, i18n};
use libconf::{ConfClient, ConfValue, NodeKind, RegistryScope};
use libanyui_client as ui;
use ui::ColumnDef;

// ─── Constants ──────────────────────────────────────────────────────

const SITES_ROOT: &str = "services/httpd/sites";
const IPC_PIPE_NAME: &str = "httpd";
const WIN_W: u32 = 960;
const WIN_H: u32 = 640;

// ─── Data Model ─────────────────────────────────────────────────────

struct RewriteRule {
    pattern: String,
    target: String,
}

struct SiteConfig {
    filename: String,
    name: String,
    port: u16,
    ssl: bool,
    ssl_port: u16,
    root: String,
    index: String,
    enabled: bool,
    rewrites: Vec<RewriteRule>,
}

impl SiteConfig {
    fn new_default(filename: &str) -> Self {
        SiteConfig {
            filename: String::from(filename),
            name: String::from("New Site"),
            port: 80,
            ssl: false,
            ssl_port: 443,
            root: String::from("/Users/Shared/www"),
            index: String::from("index.html,index.htm"),
            enabled: true,
            rewrites: Vec::new(),
        }
    }
}

// ─── Global Application State ───────────────────────────────────────

struct AppState {
    sites: Vec<SiteConfig>,
    selected_site: Option<usize>,

    // UI handles
    tree: ui::TreeView,
    name_field: ui::TextField,
    port_field: ui::TextField,
    ssl_check: ui::Checkbox,
    ssl_port_field: ui::TextField,
    root_field: ui::TextField,
    index_field: ui::TextField,
    enabled_check: ui::Checkbox,
    rewrite_grid: ui::DataGrid,
    status_label: ui::Label,
    right_panel: ui::View,

    // Toolbar buttons (for enable/disable)
    btn_delete: ui::IconButton,
    btn_apply: ui::IconButton,

    // TreeView root node index
    sites_root: u32,
}

anyos_std::global_app_state!(AppState);

// ─── Config I/O ─────────────────────────────────────────────────────

fn load_sites() -> Vec<SiteConfig> {
    let mut sites = Vec::new();
    let Ok(mut client) = ConfClient::connect("webmanager") else {
        return sites;
    };
    let Ok(items) = client.list(RegistryScope::System, SITES_ROOT) else {
        return sites;
    };

    for item in items {
        if item.kind != NodeKind::Directory {
            continue;
        }
        let Some(filename) = item.path.rsplit('/').next() else {
            continue;
        };
        if filename.starts_with('.') {
            continue;
        }
        if let Some(site) = load_site_from_registry(&mut client, filename) {
            sites.push(site);
        }
    }
    sites
}

fn conf_string(client: &mut ConfClient, path: &str) -> Option<String> {
    match client.get(RegistryScope::System, path).ok()?.value {
        Some(ConfValue::String(value)) => Some(value),
        Some(ConfValue::Int(value)) => Some(format!("{}", value)),
        Some(ConfValue::Bool(value)) => Some(if value { String::from("true") } else { String::from("false") }),
        None => None,
    }
}

fn conf_bool(client: &mut ConfClient, path: &str) -> Option<bool> {
    match client.get(RegistryScope::System, path).ok()?.value {
        Some(ConfValue::Bool(value)) => Some(value),
        _ => None,
    }
}

fn load_site_from_registry(client: &mut ConfClient, filename: &str) -> Option<SiteConfig> {
    let mut site = SiteConfig::new_default(filename);
    let base = format!("{}/{}", SITES_ROOT, filename);

    if let Some(value) = conf_string(client, &format!("{}/name", base)) {
        site.name = value;
    }
    if let Some(value) = conf_string(client, &format!("{}/root", base)) {
        site.root = value;
    }
    if let Some(value) = conf_string(client, &format!("{}/index_csv", base)) {
        site.index = value;
    }
    if let Some(value) = conf_string(client, &format!("{}/port", base)) {
        site.port = parse_u16(&value).unwrap_or(80);
    }
    if let Some(value) = conf_bool(client, &format!("{}/enabled", base)) {
        site.enabled = value;
    }
    if let Some(value) = conf_bool(client, &format!("{}/ssl", base)) {
        site.ssl = value;
    }
    if let Some(value) = conf_string(client, &format!("{}/ssl_port", base)) {
        site.ssl_port = parse_u16(&value).unwrap_or(443);
    }
    if let Some(value) = conf_string(client, &format!("{}/rewrites_blob", base)) {
        for line in value.split('\n') {
            let line = line.trim();
            if line.is_empty() || !line.starts_with("rewrite=") {
                continue;
            }
            let val = line.trim_start_matches("rewrite=").trim();
            if let Some(space) = val.find(' ') {
                site.rewrites.push(RewriteRule {
                    pattern: String::from(val[..space].trim()),
                    target: String::from(val[space + 1..].trim()),
                });
            }
        }
    }

    if site.name.is_empty() {
        return None;
    }
    Some(site)
}

fn save_site(site: &SiteConfig) {
    let Ok(mut client) = ConfClient::connect("webmanager") else {
        return;
    };
    let base = format!("{}/{}", SITES_ROOT, site.filename);
    let _ = client.mkdir(RegistryScope::System, SITES_ROOT);
    let _ = client.mkdir(RegistryScope::System, &base);
    let _ = client.set(RegistryScope::System, &format!("{}/name", base), ConfValue::String(site.name.clone()));
    let _ = client.set(RegistryScope::System, &format!("{}/port", base), ConfValue::Int(site.port as i64));
    let _ = client.set(RegistryScope::System, &format!("{}/ssl", base), ConfValue::Bool(site.ssl));
    let _ = client.set(RegistryScope::System, &format!("{}/ssl_port", base), ConfValue::Int(site.ssl_port as i64));
    let _ = client.set(RegistryScope::System, &format!("{}/root", base), ConfValue::String(site.root.clone()));
    let _ = client.set(RegistryScope::System, &format!("{}/index_csv", base), ConfValue::String(site.index.clone()));
    let _ = client.set(RegistryScope::System, &format!("{}/enabled", base), ConfValue::Bool(site.enabled));

    let mut rewrites_blob = String::new();
    for rw in &site.rewrites {
        rewrites_blob.push_str(&format!("rewrite={} {}\n", rw.pattern, rw.target));
    }
    let _ = client.set(
        RegistryScope::System,
        &format!("{}/rewrites_blob", base),
        ConfValue::String(rewrites_blob),
    );
}

fn delete_site_file(filename: &str) {
    let Ok(mut client) = ConfClient::connect("webmanager") else {
        return;
    };
    let _ = client.del(RegistryScope::System, &format!("{}/{}", SITES_ROOT, filename));
}

// ─── Service Control ────────────────────────────────────────────────

fn is_httpd_running() -> bool {
    // Check by looking for a thread named "httpd" via sysinfo
    let mut buf = [0u8; 80 * 128];
    let count = anyos_std::sys::sysinfo(1, &mut buf) as usize;
    let name_target = b"httpd";
    for i in 0..count {
        let off = i * 80;
        if off + 80 > buf.len() {
            break;
        }
        let state = buf[off + 5];
        if state > 2 {
            continue; // dead
        }
        let name_start = off + 8;
        let mut len = 0;
        for j in 0..23 {
            if buf[name_start + j] == 0 { break; }
            len += 1;
        }
        if len == name_target.len() && &buf[name_start..name_start + len] == name_target {
            return true;
        }
    }
    false
}

fn start_httpd() {
    let tid = process::spawn("/System/bin/svc", "start httpd");
    if tid != u32::MAX {
        process::waitpid(tid);
    }
}

fn stop_httpd() {
    let tid = process::spawn("/System/bin/svc", "stop httpd");
    if tid != u32::MAX {
        process::waitpid(tid);
    }
}

fn reload_httpd() {
    let pipe = anyos_std::ipc::pipe_open(IPC_PIPE_NAME);
    if pipe != 0 {
        anyos_std::ipc::pipe_write(pipe, b"reload");
    }
}

// ─── UI Updates ─────────────────────────────────────────────────────

fn refresh_tree() {
    let s = app();
    s.tree.clear();
    s.sites_root = s.tree.add_root("Sites");
    s.tree.set_expanded(s.sites_root, true);
    s.tree.set_node_style(s.sites_root, ui::STYLE_BOLD);

    for (i, site) in s.sites.iter().enumerate() {
        let label = if site.enabled {
            format!("{} (:{}) ", site.name, site.port)
        } else {
            format!("{} (disabled)", site.name)
        };
        let node = s.tree.add_child(s.sites_root, &label);
        if !site.enabled {
            s.tree.set_node_text_color(node, ui::theme::colors().text_disabled);
        }
        if s.selected_site == Some(i) {
            s.tree.set_selected(node);
        }
    }

    if s.selected_site.is_none() && !s.sites.is_empty() {
        s.selected_site = Some(0);
        // Node 0 = root "Sites", node 1 = first site
        s.tree.set_selected(1);
    }

    // Update button states
    let has_selection = s.selected_site.is_some();
    s.btn_delete.set_enabled(has_selection);
    s.btn_apply.set_enabled(has_selection);
}

fn load_site_into_form() {
    let s = app();
    let idx = match s.selected_site {
        Some(i) if i < s.sites.len() => i,
        _ => {
            // No selection — hide the right panel content
            s.right_panel.set_visible(false);
            s.btn_delete.set_enabled(false);
            s.btn_apply.set_enabled(false);
            return;
        }
    };

    s.right_panel.set_visible(true);
    s.btn_delete.set_enabled(true);
    s.btn_apply.set_enabled(true);
    let site = &s.sites[idx];

    s.name_field.set_text(&site.name);
    s.port_field.set_text(&format!("{}", site.port));
    s.ssl_check.set_state(if site.ssl { 1 } else { 0 });
    s.ssl_port_field.set_text(&format!("{}", site.ssl_port));
    s.ssl_port_field.set_enabled(site.ssl);
    s.root_field.set_text(&site.root);
    s.index_field.set_text(&site.index);
    s.enabled_check.set_state(if site.enabled { 1 } else { 0 });

    // Update rewrite grid
    let mut rows: Vec<Vec<&str>> = Vec::new();
    for rw in &site.rewrites {
        rows.push(vec![&rw.pattern, &rw.target]);
    }
    s.rewrite_grid.set_data(&rows);
}

fn save_form_to_site() {
    let s = app();
    let idx = match s.selected_site {
        Some(i) if i < s.sites.len() => i,
        _ => return,
    };

    let mut buf = [0u8; 512];

    let len = s.name_field.get_text(&mut buf);
    if len > 0 {
        let name = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        s.sites[idx].name = String::from(name);
    }

    let len = s.port_field.get_text(&mut buf);
    let port_str = core::str::from_utf8(&buf[..len as usize]).unwrap_or("80");
    s.sites[idx].port = parse_u16(port_str).unwrap_or(80);

    s.sites[idx].ssl = s.ssl_check.get_state() == 1;

    let len = s.ssl_port_field.get_text(&mut buf);
    let ssl_port_str = core::str::from_utf8(&buf[..len as usize]).unwrap_or("443");
    s.sites[idx].ssl_port = parse_u16(ssl_port_str).unwrap_or(443);

    let len = s.root_field.get_text(&mut buf);
    if len > 0 {
        let root = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        s.sites[idx].root = String::from(root);
    }

    let len = s.index_field.get_text(&mut buf);
    if len > 0 {
        let index = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        s.sites[idx].index = String::from(index);
    }

    s.sites[idx].enabled = s.enabled_check.get_state() == 1;
}

fn update_status() {
    let s = app();
    let running = is_httpd_running();
    let status_str = if running { i18n::t("Running") } else { i18n::t("Stopped") };
    let site_count = s.sites.len();
    let enabled_count = s.sites.iter().filter(|s| s.enabled).count();

    let mut ports = Vec::new();
    for site in &s.sites {
        if site.enabled && !ports.contains(&site.port) {
            ports.push(site.port);
        }
    }

    let ports_str = if ports.is_empty() {
        String::from("none")
    } else {
        let mut s = String::new();
        for (i, p) in ports.iter().enumerate() {
            if i > 0 { s.push_str(", "); }
            s.push_str(&format!("{}", p));
        }
        s
    };

    let text = format!(
        "  httpd: {} | {} site(s), {} enabled | Ports: {}",
        status_str, site_count, enabled_count, ports_str
    );
    s.status_label.set_text(&text);
}

fn generate_unique_filename(sites: &[SiteConfig]) -> String {
    let mut num = 1u32;
    loop {
        let name = format!("site{}", num);
        let exists = sites.iter().any(|s| s.filename == name);
        if !exists {
            return name;
        }
        num += 1;
        if num > 999 {
            return format!("site{}", sites.len() + 1);
        }
    }
}

// ─── Main ───────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        println!("[Web Manager] Failed to init libanyui");
        return;
    }
    i18n::init();

    // ── Main window ──
    let win = ui::Window::new(i18n::t("Web Manager"), -1, -1, WIN_W, WIN_H);
    let tc = ui::theme::colors();

    // ═══════════════════════════════════════════════════════════════
    //  Toolbar (DOCK_TOP)
    // ═══════════════════════════════════════════════════════════════

    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    win.add(&toolbar);

    let btn_new = toolbar.add_icon_button(i18n::t("New Site"));
    btn_new.set_system_icon("circle-plus", ui::IconType::Outline, tc.text, 24);
    toolbar.add_separator();
    let btn_delete = toolbar.add_icon_button(i18n::t("Delete"));
    btn_delete.set_system_icon("trash", ui::IconType::Outline, tc.text, 24);
    btn_delete.set_enabled(false);
    toolbar.add_separator();
    let btn_start = toolbar.add_icon_button(i18n::t("Start"));
    btn_start.set_system_icon("player-play", ui::IconType::Outline, tc.success, 24);
    let btn_stop = toolbar.add_icon_button(i18n::t("Stop"));
    btn_stop.set_system_icon("player-stop", ui::IconType::Outline, tc.destructive, 24);
    toolbar.add_separator();
    let btn_apply = toolbar.add_icon_button(i18n::t("Apply"));
    btn_apply.set_system_icon("device-floppy", ui::IconType::Outline, tc.text, 24);
    btn_apply.set_enabled(false);
    let btn_reload = toolbar.add_icon_button(i18n::t("Reload"));
    btn_reload.set_system_icon("refresh", ui::IconType::Outline, tc.text, 24);

    // ═══════════════════════════════════════════════════════════════
    //  Status bar (DOCK_BOTTOM)
    // ═══════════════════════════════════════════════════════════════

    let status_label = ui::Label::new("  httpd: checking...");
    status_label.set_dock(ui::DOCK_BOTTOM);
    status_label.set_size(WIN_W, 24);
    status_label.set_color(ui::theme::darken(tc.window_bg, 5));
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(11);
    win.add(&status_label);

    // ═══════════════════════════════════════════════════════════════
    //  Main split: sidebar (left) | properties (right)
    // ═══════════════════════════════════════════════════════════════

    let main_split = ui::SplitView::new();
    main_split.set_dock(ui::DOCK_FILL);
    main_split.set_split_ratio(22);
    main_split.set_min_split(15);
    main_split.set_max_split(40);
    win.add(&main_split);

    // ── Left: TreeView sidebar ──
    let sidebar = ui::View::new();
    sidebar.set_color(tc.sidebar_bg);
    main_split.add(&sidebar);

    let tree = ui::TreeView::new(200, 500);
    tree.set_dock(ui::DOCK_FILL);
    tree.set_indent_width(16);
    tree.set_row_height(24);
    sidebar.add(&tree);

    // Context menu for tree (right-click actions)
    let tree_menu_str = format!("{}|{}|-|{}|{}", i18n::t("New Site"), i18n::t("Delete Site"), i18n::t("Enable"), i18n::t("Disable"));
    let tree_menu = ui::ContextMenu::new(&tree_menu_str);
    win.add(&tree_menu);
    tree.set_context_menu(&tree_menu);

    // ── Right: Properties panel ──
    let right_panel = ui::View::new();
    right_panel.set_color(tc.window_bg);
    main_split.add(&right_panel);

    // ── Site Configuration Card ──
    let props_card = ui::Card::new();
    props_card.set_dock(ui::DOCK_TOP);
    props_card.set_size(0, 340);
    props_card.set_padding(16, 12, 16, 12);
    right_panel.add(&props_card);

    // Title
    let title_label = ui::Label::new(i18n::t("Site Configuration"));
    title_label.set_position(16, 8);
    title_label.set_size(300, 22);
    title_label.set_font_size(14);
    title_label.set_text_color(tc.accent_hover);
    props_card.add(&title_label);

    // Form layout constants
    let form_x: i32 = 16;
    let label_w: u32 = 110;
    let field_x: i32 = form_x + label_w as i32 + 8;
    let field_w: u32 = 340;
    let row_h: i32 = 36;
    let mut y: i32 = 38;

    // Name
    let lbl = ui::Label::new(i18n::t("Name:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let name_field = ui::TextField::new();
    name_field.set_position(field_x, y);
    name_field.set_size(field_w, 26);
    name_field.set_placeholder(i18n::t("Site name"));
    props_card.add(&name_field);
    y += row_h;

    // Port
    let lbl = ui::Label::new(i18n::t("Port:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let port_field = ui::TextField::new();
    port_field.set_position(field_x, y);
    port_field.set_size(80, 26);
    port_field.set_placeholder("80");
    props_card.add(&port_field);
    y += row_h;

    // SSL + SSL Port
    let lbl = ui::Label::new(i18n::t("SSL:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let ssl_check = ui::Checkbox::new(i18n::t("Enable SSL"));
    ssl_check.set_position(field_x, y + 2);
    ssl_check.set_size(100, 22);
    props_card.add(&ssl_check);

    let ssl_port_lbl = ui::Label::new("SSL Port:");
    ssl_port_lbl.set_position(field_x + 120, y + 4);
    ssl_port_lbl.set_size(70, 20);
    ssl_port_lbl.set_text_color(tc.text);
    props_card.add(&ssl_port_lbl);
    let ssl_port_field = ui::TextField::new();
    ssl_port_field.set_position(field_x + 195, y);
    ssl_port_field.set_size(80, 26);
    ssl_port_field.set_placeholder("443");
    ssl_port_field.set_enabled(false);
    props_card.add(&ssl_port_field);
    y += row_h;

    // Document Root
    let lbl = ui::Label::new(i18n::t("Document Root:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let root_field = ui::TextField::new();
    root_field.set_position(field_x, y);
    root_field.set_size(field_w - 80, 26);
    root_field.set_placeholder("/Users/Shared/www");
    props_card.add(&root_field);

    let btn_browse = ui::Button::new(i18n::t("Browse"));
    btn_browse.set_position(field_x + (field_w as i32 - 70), y);
    btn_browse.set_size(70, 26);
    props_card.add(&btn_browse);
    y += row_h;

    // Index Files
    let lbl = ui::Label::new(i18n::t("Index Files:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let index_field = ui::TextField::new();
    index_field.set_position(field_x, y);
    index_field.set_size(field_w, 26);
    index_field.set_placeholder("index.html,index.htm");
    props_card.add(&index_field);
    y += row_h;

    // Enabled
    let lbl = ui::Label::new(i18n::t("Status:"));
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    lbl.set_text_color(tc.text);
    props_card.add(&lbl);
    let enabled_check = ui::Checkbox::new(i18n::t("Enabled"));
    enabled_check.set_position(field_x, y + 2);
    enabled_check.set_size(100, 22);
    props_card.add(&enabled_check);

    // ── Rewrite Rules section ──
    let rewrite_header = ui::View::new();
    rewrite_header.set_dock(ui::DOCK_TOP);
    rewrite_header.set_size(0, 36);
    rewrite_header.set_color(tc.sidebar_bg);
    right_panel.add(&rewrite_header);

    let rewrite_title = ui::Label::new(i18n::t("URL Rewrite Rules"));
    rewrite_title.set_position(16, 8);
    rewrite_title.set_size(200, 20);
    rewrite_title.set_font_size(13);
    rewrite_title.set_text_color(tc.accent_hover);
    rewrite_header.add(&rewrite_title);

    let btn_add_rule = ui::Button::new("+ Add");
    btn_add_rule.set_position(340, 5);
    btn_add_rule.set_size(70, 26);
    rewrite_header.add(&btn_add_rule);

    let btn_del_rule = ui::Button::new("- Remove");
    btn_del_rule.set_position(418, 5);
    btn_del_rule.set_size(80, 26);
    rewrite_header.add(&btn_del_rule);

    // Rewrite DataGrid
    let rewrite_grid = ui::DataGrid::new(700, 200);
    rewrite_grid.set_dock(ui::DOCK_FILL);
    rewrite_grid.set_columns(&[
        ColumnDef::new(i18n::t("Pattern")).width(300),
        ColumnDef::new(i18n::t("Target")).width(300),
    ]);
    rewrite_grid.set_row_height(22);
    rewrite_grid.set_selection_mode(ui::SELECTION_SINGLE);
    right_panel.add(&rewrite_grid);

    // ═══════════════════════════════════════════════════════════════
    //  Initialize AppState
    // ═══════════════════════════════════════════════════════════════

    let sites = load_sites();
    let selected = if sites.is_empty() { None } else { Some(0) };

    unsafe {
        APP = Some(AppState {
            sites,
            selected_site: selected,
            tree,
            name_field,
            port_field,
            ssl_check,
            ssl_port_field,
            root_field,
            index_field,
            enabled_check,
            rewrite_grid,
            status_label,
            right_panel,
            btn_delete: btn_delete,
            btn_apply: btn_apply,
            sites_root: 0,
        });
    }

    refresh_tree();
    load_site_into_form();
    update_status();

    // ═══════════════════════════════════════════════════════════════
    //  Event Handlers
    // ═══════════════════════════════════════════════════════════════

    // TreeView selection changed
    // The event index is the flat row index: 0 = root "Sites", 1..N = site nodes
    app().tree.on_selection_changed(|e| {
        let s = app();
        let row = e.index;
        if row == 0 || row == u32::MAX {
            // Root node or nothing — don't change selection
            return;
        }
        let site_index = (row - 1) as usize;
        if site_index < s.sites.len() {
            // Save current form before switching
            if s.selected_site.is_some() {
                save_form_to_site();
            }
            s.selected_site = Some(site_index);
            load_site_into_form();
        }
    });

    // TreeView context menu handler
    // Items: 0=New Site, 1=Delete Site, 2=Enable, 3=Disable
    tree_menu.on_item_click(|e| {
        match e.index {
            0 => {
                // New Site
                let s = app();
                let filename = generate_unique_filename(&s.sites);
                let mut site = SiteConfig::new_default(&filename);
                site.name = format!("New Site {}", s.sites.len() + 1);
                save_site(&site);
                s.sites.push(site);
                s.selected_site = Some(s.sites.len() - 1);
                refresh_tree();
                load_site_into_form();
                s.name_field.focus();
            }
            1 => {
                // Delete Site
                let s = app();
                if let Some(idx) = s.selected_site {
                    if idx < s.sites.len() {
                        let filename = s.sites[idx].filename.clone();
                        delete_site_file(&filename);
                        s.sites.remove(idx);
                        if s.sites.is_empty() {
                            s.selected_site = None;
                        } else if idx >= s.sites.len() {
                            s.selected_site = Some(s.sites.len() - 1);
                        } else {
                            s.selected_site = Some(idx);
                        }
                        refresh_tree();
                        load_site_into_form();
                        update_status();
                    }
                }
            }
            3 => {
                // Enable
                let s = app();
                if let Some(idx) = s.selected_site {
                    if idx < s.sites.len() {
                        s.sites[idx].enabled = true;
                        save_site(&s.sites[idx]);
                        s.enabled_check.set_state(1);
                        refresh_tree();
                        update_status();
                    }
                }
            }
            4 => {
                // Disable
                let s = app();
                if let Some(idx) = s.selected_site {
                    if idx < s.sites.len() {
                        s.sites[idx].enabled = false;
                        save_site(&s.sites[idx]);
                        s.enabled_check.set_state(0);
                        refresh_tree();
                        update_status();
                    }
                }
            }
            _ => {}
        }
    });

    // SSL checkbox toggles SSL port field
    ssl_check.on_checked_changed(|e| {
        let s = app();
        s.ssl_port_field.set_enabled(e.checked);
    });

    // Browse button for document root
    btn_browse.on_click(|_| {
        if let Some(path) = ui::FileDialog::open_folder() {
            app().root_field.set_text(&path);
        }
    });

    // ── New Site ──
    btn_new.on_click(|_| {
        let s = app();
        let filename = generate_unique_filename(&s.sites);
        let mut site = SiteConfig::new_default(&filename);
        site.name = format!("New Site {}", s.sites.len() + 1);
        save_site(&site);
        s.sites.push(site);
        s.selected_site = Some(s.sites.len() - 1);
        refresh_tree();
        load_site_into_form();
        // Focus the name field for immediate editing
        s.name_field.focus();
    });

    // ── Delete Site ──
    btn_delete.on_click(|_| {
        let s = app();
        if let Some(idx) = s.selected_site {
            if idx < s.sites.len() {
                let filename = s.sites[idx].filename.clone();
                delete_site_file(&filename);
                s.sites.remove(idx);
                if s.sites.is_empty() {
                    s.selected_site = None;
                } else if idx >= s.sites.len() {
                    s.selected_site = Some(s.sites.len() - 1);
                } else {
                    s.selected_site = Some(idx);
                }
                refresh_tree();
                load_site_into_form();
                update_status();
            }
        }
    });

    // ── Apply (save current site) ──
    btn_apply.on_click(|_| {
        let s = app();
        save_form_to_site();
        if let Some(idx) = s.selected_site {
            if idx < s.sites.len() {
                save_site(&s.sites[idx]);
                refresh_tree();
                update_status();
            }
        }
    });

    // ── Start httpd ──
    btn_start.on_click(|_| {
        app().status_label.set_text("  httpd: starting...");
        start_httpd();
        process::sleep(500);
        update_status();
    });

    // ── Stop httpd ──
    btn_stop.on_click(|_| {
        app().status_label.set_text("  httpd: stopping...");
        stop_httpd();
        process::sleep(500);
        update_status();
    });

    // ── Reload httpd config ──
    btn_reload.on_click(|_| {
        // Save all sites first
        let s = app();
        save_form_to_site();
        for site in &s.sites {
            save_site(site);
        }
        s.status_label.set_text("  httpd: reloading...");
        reload_httpd();
        process::sleep(300);
        update_status();
    });

    // ── Add rewrite rule ──
    btn_add_rule.on_click(|_| {
        let s = app();
        if let Some(idx) = s.selected_site {
            if idx < s.sites.len() {
                s.sites[idx].rewrites.push(RewriteRule {
                    pattern: String::from("/path"),
                    target: String::from("/new-path"),
                });
                let mut rows: Vec<Vec<&str>> = Vec::new();
                for rw in &s.sites[idx].rewrites {
                    rows.push(vec![&rw.pattern, &rw.target]);
                }
                s.rewrite_grid.set_data(&rows);
            }
        }
    });

    // ── Remove rewrite rule ──
    btn_del_rule.on_click(|_| {
        let s = app();
        let sel_row = s.rewrite_grid.selected_row();
        if sel_row != u32::MAX {
            if let Some(idx) = s.selected_site {
                if idx < s.sites.len() && (sel_row as usize) < s.sites[idx].rewrites.len() {
                    s.sites[idx].rewrites.remove(sel_row as usize);
                    let mut rows: Vec<Vec<&str>> = Vec::new();
                    for rw in &s.sites[idx].rewrites {
                        rows.push(vec![&rw.pattern, &rw.target]);
                    }
                    s.rewrite_grid.set_data(&rows);
                }
            }
        }
    });

    // ── Periodic status update (every 5s) ──
    ui::set_timer(5000, || {
        update_status();
    });

    // ── Run event loop ──
    ui::run();
}

// ─── Utilities ──────────────────────────────────────────────────────

fn parse_u16(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val * 10 + (b - b'0') as u32;
        if val > 65535 {
            return None;
        }
    }
    Some(val as u16)
}
