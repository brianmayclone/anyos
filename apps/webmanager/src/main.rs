#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::fs;
use anyos_std::process;
use anyos_std::println;
use anyos_std::{String, Vec, format, vec};
use libanyui_client as ui;

// ─── Constants ──────────────────────────────────────────────────────

const SITES_DIR: &str = "/System/etc/httpd/sites";
const IPC_PIPE_NAME: &str = "httpd";
const WIN_W: u32 = 960;
const WIN_H: u32 = 620;

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
    modified: bool,

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
    props_card: ui::Card,

    // TreeView node indices
    sites_root: u32,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("APP not initialized") }
}

// ─── Config I/O ─────────────────────────────────────────────────────

fn load_sites() -> Vec<SiteConfig> {
    let mut sites = Vec::new();
    let mut dir_buf = [0u8; 4096];
    let n = fs::readdir(SITES_DIR, &mut dir_buf);
    if n == u32::MAX {
        return sites;
    }

    let mut off = 0usize;
    for _ in 0..n as usize {
        if off + 64 > dir_buf.len() {
            break;
        }
        let entry_type = dir_buf[off];
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len];

        if entry_type == 0 {
            if let Ok(filename) = core::str::from_utf8(name_bytes) {
                let path = format!("{}/{}", SITES_DIR, filename);
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Some(site) = parse_site_file(&content, filename) {
                        sites.push(site);
                    }
                }
            }
        }
        off += 64;
    }
    sites
}

fn parse_site_file(content: &str, filename: &str) -> Option<SiteConfig> {
    let mut site = SiteConfig::new_default(filename);
    site.filename = String::from(filename);

    for line in content.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(val) = line.strip_prefix("name=") {
            site.name = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("port=") {
            site.port = parse_u16(val.trim()).unwrap_or(80);
        } else if let Some(val) = line.strip_prefix("ssl=") {
            site.ssl = val.trim() == "true";
        } else if let Some(val) = line.strip_prefix("ssl_port=") {
            site.ssl_port = parse_u16(val.trim()).unwrap_or(443);
        } else if let Some(val) = line.strip_prefix("root=") {
            site.root = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("index=") {
            site.index = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("enabled=") {
            site.enabled = val.trim() == "true";
        } else if let Some(val) = line.strip_prefix("rewrite=") {
            let val = val.trim();
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
    let mut content = String::new();
    content.push_str(&format!("name={}\n", site.name));
    content.push_str(&format!("port={}\n", site.port));
    content.push_str(&format!("ssl={}\n", if site.ssl { "true" } else { "false" }));
    content.push_str(&format!("ssl_port={}\n", site.ssl_port));
    content.push_str(&format!("root={}\n", site.root));
    content.push_str(&format!("index={}\n", site.index));
    content.push_str(&format!("enabled={}\n", if site.enabled { "true" } else { "false" }));
    for rw in &site.rewrites {
        content.push_str(&format!("rewrite={} {}\n", rw.pattern, rw.target));
    }

    let path = format!("{}/{}", SITES_DIR, site.filename);
    let _ = fs::write_bytes(&path, content.as_bytes());
}

fn delete_site_file(filename: &str) {
    let path = format!("{}/{}", SITES_DIR, filename);
    fs::unlink(&path);
}

// ─── Service Control ────────────────────────────────────────────────

fn is_httpd_running() -> bool {
    let tid = process::spawn("/System/bin/svc", "status httpd");
    if tid == u32::MAX {
        return false;
    }
    let exit_code = process::waitpid(tid);
    exit_code == 0
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
    s.tree.set_node_style(s.sites_root, 1); // STYLE_BOLD

    for (i, site) in s.sites.iter().enumerate() {
        let label = if site.enabled {
            format!("{} (:{}) ", site.name, site.port)
        } else {
            format!("{} (disabled)", site.name)
        };
        let node = s.tree.add_child(s.sites_root, &label);
        if !site.enabled {
            s.tree.set_node_text_color(node, 0xFF888888);
        }
        // Select first or previously selected
        if s.selected_site == Some(i) {
            s.tree.set_selected(node);
        }
    }

    if s.selected_site.is_none() && !s.sites.is_empty() {
        s.selected_site = Some(0);
        s.tree.set_selected(1); // node index 1 = first child after root
    }
}

fn load_site_into_form() {
    let s = app();
    let idx = match s.selected_site {
        Some(i) if i < s.sites.len() => i,
        _ => {
            s.props_card.set_visible(false);
            return;
        }
    };

    s.props_card.set_visible(true);
    let site = &s.sites[idx];

    s.name_field.set_text(&site.name);
    s.port_field.set_text(&format!("{}", site.port));
    s.ssl_check.set_state(if site.ssl { 1 } else { 0 });
    s.ssl_port_field.set_text(&format!("{}", site.ssl_port));
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

    let mut buf = [0u8; 256];

    let len = s.name_field.get_text(&mut buf);
    let name = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    s.sites[idx].name = String::from(name);

    let len = s.port_field.get_text(&mut buf);
    let port_str = core::str::from_utf8(&buf[..len as usize]).unwrap_or("80");
    s.sites[idx].port = parse_u16(port_str).unwrap_or(80);

    s.sites[idx].ssl = s.ssl_check.get_state() == 1;

    let len = s.ssl_port_field.get_text(&mut buf);
    let ssl_port_str = core::str::from_utf8(&buf[..len as usize]).unwrap_or("443");
    s.sites[idx].ssl_port = parse_u16(ssl_port_str).unwrap_or(443);

    let len = s.root_field.get_text(&mut buf);
    let root = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    s.sites[idx].root = String::from(root);

    let len = s.index_field.get_text(&mut buf);
    let index = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    s.sites[idx].index = String::from(index);

    s.sites[idx].enabled = s.enabled_check.get_state() == 1;
}

fn update_status() {
    let s = app();
    let running = is_httpd_running();
    let status_str = if running { "Running" } else { "Stopped" };
    let site_count = s.sites.len();
    let enabled_count = s.sites.iter().filter(|s| s.enabled).count();
    let ports: Vec<u16> = {
        let mut v = Vec::new();
        for site in &s.sites {
            if site.enabled && !v.contains(&site.port) {
                v.push(site.port);
            }
        }
        v
    };

    let text = format!(
        "  httpd: {} | {} site(s), {} enabled | Ports: {:?}",
        status_str, site_count, enabled_count, ports
    );
    s.status_label.set_text(&text);
}

// ─── Main ───────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        println!("[Web Manager] Failed to init libanyui");
        return;
    }

    // Ensure config directory exists
    fs::mkdir(SITES_DIR);

    // Create main window
    let win = ui::Window::new("Web Manager", -1, -1, WIN_W, WIN_H);

    // ── Toolbar ──
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    win.add(&toolbar);

    let btn_new = toolbar.add_button("+ New Site");
    toolbar.add_separator();
    let btn_delete = toolbar.add_button("Delete");
    toolbar.add_separator();
    let btn_start = toolbar.add_button("Start");
    let btn_stop = toolbar.add_button("Stop");
    toolbar.add_separator();
    let btn_apply = toolbar.add_button("Apply");
    let btn_reload = toolbar.add_button("Reload");

    // ── Status bar ──
    let status_label = ui::Label::new("  httpd: checking...");
    status_label.set_dock(ui::DOCK_BOTTOM);
    status_label.set_size(WIN_W, 24);
    status_label.set_color(0xFF1E1E1E);
    status_label.set_text_color(0xFFAAAAAA);
    status_label.set_font_size(11);
    win.add(&status_label);

    // ── Main split: sidebar | properties ──
    let main_split = ui::SplitView::new();
    main_split.set_dock(ui::DOCK_FILL);
    main_split.set_split_ratio(22);
    main_split.set_min_split(15);
    main_split.set_max_split(35);
    win.add(&main_split);

    // ── Left: TreeView sidebar ──
    let sidebar = ui::View::new();
    sidebar.set_color(0xFF252526);
    main_split.add(&sidebar);

    let tree = ui::TreeView::new(220, 500);
    tree.set_dock(ui::DOCK_FILL);
    tree.set_indent_width(16);
    tree.set_row_height(24);
    sidebar.add(&tree);

    // ── Right: Properties panel ──
    let right_panel = ui::View::new();
    right_panel.set_color(0xFF1E1E1E);
    main_split.add(&right_panel);

    // Properties card
    let props_card = ui::Card::new();
    props_card.set_dock(ui::DOCK_TOP);
    props_card.set_size(0, 320);
    props_card.set_padding(16, 12, 16, 12);
    right_panel.add(&props_card);

    // Title label
    let title_label = ui::Label::new("Site Configuration");
    title_label.set_position(16, 8);
    title_label.set_size(300, 20);
    title_label.set_font_size(14);
    title_label.set_text_color(0xFFFFFFFF);
    props_card.add(&title_label);

    // Form fields with labels - use manual positioning
    let form_x: i32 = 16;
    let label_w: u32 = 110;
    let field_x: i32 = form_x + label_w as i32 + 8;
    let field_w: u32 = 320;
    let row_h: i32 = 32;
    let mut y: i32 = 36;

    // Name
    let lbl = ui::Label::new("Name:");
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    props_card.add(&lbl);
    let name_field = ui::TextField::new();
    name_field.set_position(field_x, y);
    name_field.set_size(field_w, 24);
    name_field.set_placeholder("Site name");
    props_card.add(&name_field);
    y += row_h;

    // Port
    let lbl = ui::Label::new("Port:");
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    props_card.add(&lbl);
    let port_field = ui::TextField::new();
    port_field.set_position(field_x, y);
    port_field.set_size(80, 24);
    port_field.set_placeholder("80");
    props_card.add(&port_field);
    y += row_h;

    // SSL + SSL Port
    let ssl_check = ui::Checkbox::new("SSL");
    ssl_check.set_position(field_x, y);
    ssl_check.set_size(60, 24);
    props_card.add(&ssl_check);

    let lbl = ui::Label::new("SSL Port:");
    lbl.set_position(field_x + 80, y + 4);
    lbl.set_size(70, 20);
    props_card.add(&lbl);
    let ssl_port_field = ui::TextField::new();
    ssl_port_field.set_position(field_x + 155, y);
    ssl_port_field.set_size(80, 24);
    ssl_port_field.set_placeholder("443");
    props_card.add(&ssl_port_field);
    y += row_h;

    // Document Root
    let lbl = ui::Label::new("Document Root:");
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    props_card.add(&lbl);
    let root_field = ui::TextField::new();
    root_field.set_position(field_x, y);
    root_field.set_size(field_w, 24);
    root_field.set_placeholder("/Users/Shared/www");
    props_card.add(&root_field);
    y += row_h;

    // Index Files
    let lbl = ui::Label::new("Index Files:");
    lbl.set_position(form_x, y + 4);
    lbl.set_size(label_w, 20);
    props_card.add(&lbl);
    let index_field = ui::TextField::new();
    index_field.set_position(field_x, y);
    index_field.set_size(field_w, 24);
    index_field.set_placeholder("index.html,index.htm");
    props_card.add(&index_field);
    y += row_h;

    // Enabled
    let enabled_check = ui::Checkbox::new("Enabled");
    enabled_check.set_position(field_x, y);
    enabled_check.set_size(100, 24);
    props_card.add(&enabled_check);

    // ── Rewrite Rules section ──
    let rewrite_label = ui::Label::new("URL Rewrite Rules");
    rewrite_label.set_dock(ui::DOCK_TOP);
    rewrite_label.set_size(0, 28);
    rewrite_label.set_padding(16, 6, 0, 0);
    rewrite_label.set_font_size(13);
    rewrite_label.set_text_color(0xFFCCCCCC);
    right_panel.add(&rewrite_label);

    // Rewrite button bar
    let rw_btn_bar = ui::View::new();
    rw_btn_bar.set_dock(ui::DOCK_TOP);
    rw_btn_bar.set_size(0, 32);
    right_panel.add(&rw_btn_bar);

    let btn_add_rule = ui::Button::new("+ Add Rule");
    btn_add_rule.set_position(16, 2);
    btn_add_rule.set_size(100, 26);
    rw_btn_bar.add(&btn_add_rule);

    let btn_del_rule = ui::Button::new("- Remove");
    btn_del_rule.set_position(124, 2);
    btn_del_rule.set_size(90, 26);
    rw_btn_bar.add(&btn_del_rule);

    // Rewrite DataGrid
    let rewrite_grid = ui::DataGrid::new(700, 200);
    rewrite_grid.set_dock(ui::DOCK_FILL);
    rewrite_grid.set_columns(&[
        ui::ColumnDef::new("Pattern").width(300),
        ui::ColumnDef::new("Target").width(300),
    ]);
    rewrite_grid.set_row_height(22);
    rewrite_grid.set_selection_mode(ui::SELECTION_SINGLE);
    right_panel.add(&rewrite_grid);

    // ── Initialize AppState ──
    let sites = load_sites();
    let selected = if sites.is_empty() { None } else { Some(0) };

    unsafe {
        APP = Some(AppState {
            sites,
            selected_site: selected,
            modified: false,
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
            props_card,
            sites_root: 0,
        });
    }

    refresh_tree();
    load_site_into_form();
    update_status();

    // ── Event Handlers ──

    // TreeView selection
    app().tree.on_selection_changed(|e| {
        let s = app();
        // Node 0 = root, nodes 1..N = sites
        if e.index > 0 && (e.index - 1) < s.sites.len() as u32 {
            // Save current form first
            if s.selected_site.is_some() {
                save_form_to_site();
            }
            s.selected_site = Some((e.index - 1) as usize);
            load_site_into_form();
        }
    });

    // New Site
    btn_new.on_click(|_| {
        let s = app();
        // Generate unique filename
        let mut num = s.sites.len() + 1;
        let filename = loop {
            let name = format!("site{}", num);
            let path = format!("{}/{}", SITES_DIR, name);
            let mut stat = [0u32; 7];
            if fs::stat(&path, &mut stat) != 0 {
                break name;
            }
            num += 1;
        };

        let mut site = SiteConfig::new_default(&filename);
        site.name = format!("New Site {}", s.sites.len() + 1);
        save_site(&site);
        s.sites.push(site);
        s.selected_site = Some(s.sites.len() - 1);
        refresh_tree();
        load_site_into_form();
    });

    // Delete Site
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
                }
                refresh_tree();
                load_site_into_form();
            }
        }
    });

    // Apply (save current site)
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

    // Start httpd
    btn_start.on_click(|_| {
        start_httpd();
        process::sleep(500);
        update_status();
    });

    // Stop httpd
    btn_stop.on_click(|_| {
        stop_httpd();
        process::sleep(500);
        update_status();
    });

    // Reload httpd
    btn_reload.on_click(|_| {
        // Save all sites first
        let s = app();
        save_form_to_site();
        for site in &s.sites {
            save_site(site);
        }
        reload_httpd();
        process::sleep(300);
        update_status();
    });

    // Add rewrite rule
    btn_add_rule.on_click(|_| {
        let s = app();
        if let Some(idx) = s.selected_site {
            if idx < s.sites.len() {
                s.sites[idx].rewrites.push(RewriteRule {
                    pattern: String::from("/example"),
                    target: String::from("/new-path"),
                });
                // Refresh grid
                let mut rows: Vec<Vec<&str>> = Vec::new();
                for rw in &s.sites[idx].rewrites {
                    rows.push(vec![&rw.pattern, &rw.target]);
                }
                s.rewrite_grid.set_data(&rows);
            }
        }
    });

    // Remove rewrite rule
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

    // Periodic status update
    ui::set_timer(5000, || {
        update_status();
    });

    // ── Run event loop ──
    ui::run();
}

// ─── Utilities ──────────────────────────────────────────────────────

fn parse_u16(s: &str) -> Option<u16> {
    let mut val: u32 = 0;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
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
