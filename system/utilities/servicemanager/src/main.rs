#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::process;
use libami::{AmiClient, AmiValue};
use libanyui_client as ui;
use libconf::{ConfClient, ConfValue, NodeKind, RegistryScope};

anyos_std::entry!(main);

const WIN_W: u32 = 1160;
const WIN_H: u32 = 760;
const TOOLBAR_H: u32 = 40;
const STATUS_H: u32 = 28;
const DETAIL_H: u32 = 260;
const REFRESH_MS: u32 = 1200;

#[derive(Clone)]
struct ServiceEntry {
    name: String,
    exec: String,
    args: String,
    depends: String,
    wants: String,
    after: String,
    enabled: bool,
    startup_timeout_ms: u32,
    state: String,
    error: String,
}

struct RuntimeEntry {
    name: String,
    state: String,
    error: String,
}

struct App {
    conf: Option<ConfClient>,
    ami: Option<AmiClient>,
    services: Vec<ServiceEntry>,
    visible: Vec<usize>,
    search_text: String,
    selected_name: String,
    grid: ui::DataGrid,
    status: ui::Label,
    connection: ui::Label,
    heading: ui::Label,
    name_field: ui::TextField,
    exec_field: ui::TextField,
    args_field: ui::TextField,
    depends_field: ui::TextField,
    wants_field: ui::TextField,
    after_field: ui::TextField,
    timeout_field: ui::TextField,
    autostart_toggle: ui::Toggle,
    runtime_label: ui::Label,
    error_label: ui::Label,
}

anyos_std::global_app_state!(App);

fn main() {
    if !ui::init() {
        anyos_std::println!("servicemanager: failed to load libanyui.so");
        return;
    }

    let tc = ui::theme::colors();
    let win = ui::Window::new("Service Manager", -1, -1, WIN_W, WIN_H);

    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, TOOLBAR_H);
    toolbar.set_padding(8, 5, 8, 5);
    win.add(&toolbar);

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_icon(ui::ICON_REFRESH);
    btn_refresh.set_size(32, 28);

    toolbar.add_separator();
    let title = toolbar.add_label("Service Manager");
    title.set_font_size(15);
    title.set_text_color(tc.text);
    title.set_size(180, 26);

    toolbar.add_separator();
    let search = ui::SearchField::new();
    search.set_placeholder("Search services, state, executable...");
    search.set_size(300, 28);
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

    let status = ui::Label::new("Waiting for confd and amid...");
    status.set_position(10, 6);
    status.set_size(WIN_W - 20, 16);
    status.set_text_color(tc.text_secondary);
    status.set_font_size(12);
    status_bar.add(&status);

    let detail = ui::View::new();
    detail.set_dock(ui::DOCK_BOTTOM);
    detail.set_size(WIN_W, DETAIL_H);
    detail.set_color(tc.editor_bg);
    detail.set_padding(12, 10, 12, 10);
    win.add(&detail);

    let heading = ui::Label::new("Service Details");
    heading.set_position(12, 10);
    heading.set_size(360, 18);
    heading.set_text_color(tc.text_secondary);
    heading.set_font_size(11);
    detail.add(&heading);

    let name_label = ui::Label::new("Name");
    name_label.set_position(12, 36);
    name_label.set_size(80, 16);
    detail.add(&name_label);

    let name_field = ui::TextField::new();
    name_field.set_position(12, 56);
    name_field.set_size(220, 28);
    name_field.set_placeholder("httpd");
    detail.add(&name_field);

    let timeout_label = ui::Label::new("Startup Timeout");
    timeout_label.set_position(244, 36);
    timeout_label.set_size(120, 16);
    detail.add(&timeout_label);

    let timeout_field = ui::TextField::new();
    timeout_field.set_position(244, 56);
    timeout_field.set_size(120, 28);
    timeout_field.set_placeholder("5000");
    detail.add(&timeout_field);

    let autostart_label = ui::Label::new("Auto-start");
    autostart_label.set_position(378, 36);
    autostart_label.set_size(90, 16);
    detail.add(&autostart_label);

    let autostart_toggle = ui::Toggle::new(true);
    autostart_toggle.set_position(378, 58);
    detail.add(&autostart_toggle);

    let exec_label = ui::Label::new("Executable");
    exec_label.set_position(12, 92);
    exec_label.set_size(90, 16);
    detail.add(&exec_label);

    let exec_field = ui::TextField::new();
    exec_field.set_position(12, 112);
    exec_field.set_size(420, 28);
    exec_field.set_placeholder("/System/bin/httpd");
    detail.add(&exec_field);

    let args_label = ui::Label::new("Arguments");
    args_label.set_position(444, 92);
    args_label.set_size(90, 16);
    detail.add(&args_label);

    let args_field = ui::TextField::new();
    args_field.set_position(444, 112);
    args_field.set_size(300, 28);
    args_field.set_placeholder("--foreground");
    detail.add(&args_field);

    let depends_label = ui::Label::new("Depends");
    depends_label.set_position(12, 148);
    depends_label.set_size(80, 16);
    detail.add(&depends_label);

    let depends_field = ui::TextField::new();
    depends_field.set_position(12, 168);
    depends_field.set_size(230, 28);
    depends_field.set_placeholder("networkd,logd");
    detail.add(&depends_field);

    let wants_label = ui::Label::new("Wants");
    wants_label.set_position(254, 148);
    wants_label.set_size(80, 16);
    detail.add(&wants_label);

    let wants_field = ui::TextField::new();
    wants_field.set_position(254, 168);
    wants_field.set_size(230, 28);
    wants_field.set_placeholder("logd");
    detail.add(&wants_field);

    let after_label = ui::Label::new("After");
    after_label.set_position(496, 148);
    after_label.set_size(80, 16);
    detail.add(&after_label);

    let after_field = ui::TextField::new();
    after_field.set_position(496, 168);
    after_field.set_size(248, 28);
    after_field.set_placeholder("networkd");
    detail.add(&after_field);

    let runtime_label = ui::Label::new("Runtime: unknown");
    runtime_label.set_position(12, 206);
    runtime_label.set_size(360, 18);
    runtime_label.set_text_color(tc.text);
    detail.add(&runtime_label);

    let error_label = ui::Label::new("Last error: -");
    error_label.set_position(12, 228);
    error_label.set_size(560, 18);
    error_label.set_text_color(tc.text_secondary);
    error_label.set_font_size(11);
    detail.add(&error_label);

    let btn_apply = ui::Button::new("Apply");
    btn_apply.set_position(770, 30);
    btn_apply.set_size(110, 30);
    detail.add(&btn_apply);

    let btn_install = ui::Button::new("Install");
    btn_install.set_position(892, 30);
    btn_install.set_size(110, 30);
    detail.add(&btn_install);

    let btn_remove = ui::Button::new("Remove");
    btn_remove.set_position(1014, 30);
    btn_remove.set_size(110, 30);
    detail.add(&btn_remove);

    let btn_enable = ui::Button::new("Enable");
    btn_enable.set_position(770, 72);
    btn_enable.set_size(110, 30);
    detail.add(&btn_enable);

    let btn_disable = ui::Button::new("Disable");
    btn_disable.set_position(892, 72);
    btn_disable.set_size(110, 30);
    detail.add(&btn_disable);

    let btn_restart = ui::Button::new("Restart");
    btn_restart.set_position(1014, 72);
    btn_restart.set_size(110, 30);
    detail.add(&btn_restart);

    let btn_start = ui::Button::new("Start");
    btn_start.set_position(770, 114);
    btn_start.set_size(110, 30);
    detail.add(&btn_start);

    let btn_stop = ui::Button::new("Stop");
    btn_stop.set_position(892, 114);
    btn_stop.set_size(110, 30);
    detail.add(&btn_stop);

    let btn_new = ui::Button::new("New");
    btn_new.set_position(1014, 114);
    btn_new.set_size(110, 30);
    detail.add(&btn_new);

    let sep_bottom = ui::Divider::new();
    sep_bottom.set_dock(ui::DOCK_BOTTOM);
    sep_bottom.set_size(WIN_W, 1);
    win.add(&sep_bottom);

    let grid = ui::DataGrid::new(WIN_W, WIN_H);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_row_height(24);
    grid.set_header_height(26);
    grid.set_font_size(12);
    grid.set_columns(&[
        ui::ColumnDef::new("Service").width(180),
        ui::ColumnDef::new("State").width(120),
        ui::ColumnDef::new("Startup").width(90),
        ui::ColumnDef::new("Executable").width(420),
        ui::ColumnDef::new("Args").width(280),
    ]);
    win.add(&grid);

    unsafe {
        APP = Some(App {
            conf: None,
            ami: None,
            services: Vec::new(),
            visible: Vec::new(),
            search_text: String::new(),
            selected_name: String::new(),
            grid,
            status,
            connection,
            heading,
            name_field,
            exec_field,
            args_field,
            depends_field,
            wants_field,
            after_field,
            timeout_field,
            autostart_toggle,
            runtime_label,
            error_label,
        });
    }

    app().grid.on_selection_changed(|e| {
        if let Some(name) = selected_name_for_row(e.index) {
            let a = app();
            a.selected_name = name;
            show_selected_service();
        }
    });

    search.on_text_changed(|e| {
        let a = app();
        a.search_text = to_lower(&e.text());
        refresh_visible();
    });

    btn_refresh.on_click(|_| reload_snapshot());
    btn_new.on_click(|_| clear_editor_for_new_service());

    btn_apply.on_click(|_| {
        if save_current_service(false) {
            set_status("Service configuration saved.");
            reload_snapshot();
        }
    });

    btn_install.on_click(|_| {
        if save_current_service(true) {
            let name = read_field(&app().name_field);
            if !name.is_empty() {
                run_svc_command("install", &name);
            }
            set_status("Service installed or enabled.");
            reload_snapshot();
        }
    });

    btn_remove.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("remove", &name);
            set_status("Service removed.");
            reload_snapshot();
        }
    });

    btn_enable.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("enable", &name);
            set_status("Auto-start enabled.");
            reload_snapshot();
        }
    });

    btn_disable.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("disable", &name);
            set_status("Auto-start disabled.");
            reload_snapshot();
        }
    });

    btn_start.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("start", &name);
            set_status("Start requested.");
            reload_snapshot();
        }
    });

    btn_stop.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("stop", &name);
            set_status("Stop requested.");
            reload_snapshot();
        }
    });

    btn_restart.on_click(|_| {
        let name = current_service_name();
        if !name.is_empty() {
            run_svc_command("restart", &name);
            set_status("Restart requested.");
            reload_snapshot();
        }
    });

    ui::set_timer(REFRESH_MS, || reload_snapshot());
    reload_snapshot();
    ui::run();
}

fn reload_snapshot() {
    let a = app();
    let tc = ui::theme::colors();

    if a.conf.is_none() {
        match ConfClient::connect("servicemanager") {
            Ok(client) => a.conf = Some(client),
            Err(err) => {
                a.connection.set_text("confd offline");
                a.connection.set_text_color(tc.destructive);
                a.status.set_text(&format!("confd unavailable: {:?}", err));
                a.grid.set_row_count(0);
                return;
            }
        }
    }

    if a.ami.is_none() {
        if let Ok(client) = AmiClient::connect("servicemanager") {
            a.ami = Some(client);
        }
    }

    match load_services() {
        Ok(services) => {
            a.services = services;
            a.connection.set_text(if a.ami.is_some() {
                "Live"
            } else {
                "confd only"
            });
            a.connection.set_text_color(if a.ami.is_some() {
                tc.success
            } else {
                tc.warning
            });
            refresh_visible();
            if a.selected_name.is_empty() && !a.visible.is_empty() {
                let idx = a.visible[0];
                a.selected_name = a.services[idx].name.clone();
            }
            restore_selection();
            show_selected_service();
        }
        Err(message) => {
            a.conf = None;
            a.connection.set_text("Retrying...");
            a.connection.set_text_color(tc.warning);
            a.status.set_text(&message);
        }
    }
}

fn load_services() -> Result<Vec<ServiceEntry>, String> {
    let a = app();
    let Some(client) = a.conf.as_mut() else {
        return Err(String::from("confd client missing"));
    };

    let items = client
        .list(RegistryScope::System, "services")
        .map_err(|err| format!("Failed to load services: {:?}", err))?;
    let runtime = load_runtime_entries();
    let mut services = Vec::new();
    let mut names = Vec::new();

    for item in &items {
        if item.kind != NodeKind::Directory {
            continue;
        }
        let Some(name) = item.path.strip_prefix("services/") else {
            continue;
        };
        if name.is_empty() || name.contains('/') {
            continue;
        }
        if names
            .iter()
            .any(|existing: &String| existing.as_str() == name)
        {
            continue;
        }
        names.push(String::from(name));
        if let Some(service) = read_service_entry(name, &items, &runtime) {
            services.push(service);
        }
    }

    services.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    Ok(services)
}

fn read_service_entry(
    name: &str,
    items: &[libconf::ConfItem],
    runtime: &[RuntimeEntry],
) -> Option<ServiceEntry> {
    let removed = conf_bool(items, name, "removed").unwrap_or(false);
    if removed {
        return None;
    }

    let exec = conf_string(items, name, "exec")?;
    let args = conf_string(items, name, "args").unwrap_or_default();
    let depends = conf_string(items, name, "depends").unwrap_or_default();
    let wants = conf_string(items, name, "wants").unwrap_or_default();
    let after = conf_string(items, name, "after").unwrap_or_default();
    let enabled = conf_bool(items, name, "enabled").unwrap_or(true);
    let startup_timeout_ms = conf_u32(items, name, "startup_timeout_ms").unwrap_or(0);
    let (state, error) = read_runtime_state(runtime, name);

    Some(ServiceEntry {
        name: String::from(name),
        exec,
        args,
        depends,
        wants,
        after,
        enabled,
        startup_timeout_ms,
        state,
        error,
    })
}

fn conf_path(name: &str, field: &str) -> String {
    format!("services/{}/config/{}", name, field)
}

fn conf_item_value<'a>(
    items: &'a [libconf::ConfItem],
    name: &str,
    field: &str,
) -> Option<&'a ConfValue> {
    let path = conf_path(name, field);
    items.iter().find(|item| item.path == path)?.value.as_ref()
}

fn conf_string(items: &[libconf::ConfItem], name: &str, field: &str) -> Option<String> {
    match conf_item_value(items, name, field)? {
        ConfValue::String(value) => Some(value.clone()),
        ConfValue::ExternalRef(value) => Some(value.clone()),
        _ => None,
    }
}

fn conf_bool(items: &[libconf::ConfItem], name: &str, field: &str) -> Option<bool> {
    match conf_item_value(items, name, field)? {
        ConfValue::Bool(value) => Some(*value),
        ConfValue::Int(value) => Some(*value != 0),
        _ => None,
    }
}

fn conf_u32(items: &[libconf::ConfItem], name: &str, field: &str) -> Option<u32> {
    match conf_item_value(items, name, field)? {
        ConfValue::Int(value) if *value >= 0 => Some(*value as u32),
        _ => None,
    }
}

fn load_runtime_entries() -> Vec<RuntimeEntry> {
    let a = app();
    let Some(ami) = a.ami.as_mut() else {
        return Vec::new();
    };
    let Ok(items) = ami.list("svc.") else {
        return Vec::new();
    };

    let mut runtime = Vec::new();
    for item in items {
        let Some(rest) = item.key.strip_prefix("svc.") else {
            continue;
        };
        let Some(dot) = rest.find('.') else {
            continue;
        };
        let name = &rest[..dot];
        let field = &rest[dot + 1..];
        if name.is_empty() {
            continue;
        }

        let idx = if let Some(idx) = runtime
            .iter()
            .position(|entry: &RuntimeEntry| entry.name == name)
        {
            idx
        } else {
            runtime.push(RuntimeEntry {
                name: String::from(name),
                state: String::from("stopped"),
                error: String::new(),
            });
            runtime.len() - 1
        };

        match field {
            "state" => {
                if let AmiValue::String(value) = item.value {
                    runtime[idx].state = value;
                }
            }
            "error" => {
                if let AmiValue::String(value) = item.value {
                    runtime[idx].error = value;
                }
            }
            _ => {}
        }
    }

    runtime
}

fn read_runtime_state(runtime: &[RuntimeEntry], name: &str) -> (String, String) {
    if let Some(entry) = runtime.iter().find(|entry| entry.name == name) {
        return (entry.state.clone(), entry.error.clone());
    }
    (String::from("stopped"), String::new())
}

fn refresh_visible() {
    let a = app();
    a.visible.clear();

    for (idx, service) in a.services.iter().enumerate() {
        if matches_search(service) {
            a.visible.push(idx);
        }
    }

    a.grid.set_row_count(a.visible.len() as u32);
    for (row, service_idx) in a.visible.iter().enumerate() {
        let service = &a.services[*service_idx];
        a.grid.set_cell(row as u32, 0, &service.name);
        a.grid.set_cell(row as u32, 1, &service.state);
        a.grid.set_cell(
            row as u32,
            2,
            if service.enabled {
                "Enabled"
            } else {
                "Disabled"
            },
        );
        a.grid.set_cell(row as u32, 3, &service.exec);
        a.grid.set_cell(row as u32, 4, &service.args);
    }
}

fn matches_search(service: &ServiceEntry) -> bool {
    let query = app().search_text.as_str();
    if query.is_empty() {
        return true;
    }

    contains_lower(&service.name, query)
        || contains_lower(&service.exec, query)
        || contains_lower(&service.args, query)
        || contains_lower(&service.state, query)
}

fn restore_selection() {
    let a = app();
    for (row, idx) in a.visible.iter().enumerate() {
        if a.services[*idx].name == a.selected_name {
            a.grid.set_selected_row(row as u32);
            return;
        }
    }
}

fn selected_name_for_row(row: u32) -> Option<String> {
    let a = app();
    let idx = *a.visible.get(row as usize)?;
    Some(a.services[idx].name.clone())
}

fn show_selected_service() {
    let a = app();
    if let Some(service) = a
        .services
        .iter()
        .find(|service| service.name == a.selected_name)
    {
        a.heading
            .set_text(&format!("Service Details — {}", service.name));
        a.name_field.set_text(&service.name);
        a.exec_field.set_text(&service.exec);
        a.args_field.set_text(&service.args);
        a.depends_field.set_text(&service.depends);
        a.wants_field.set_text(&service.wants);
        a.after_field.set_text(&service.after);
        a.timeout_field
            .set_text(&format!("{}", service.startup_timeout_ms));
        a.autostart_toggle
            .set_state(if service.enabled { 1 } else { 0 });
        a.runtime_label.set_text(&format!(
            "Runtime: {} | Startup: {}",
            service.state,
            if service.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ));
        let error_text = if service.error.is_empty() {
            String::from("Last error: -")
        } else {
            format!("Last error: {}", service.error)
        };
        a.error_label.set_text(&error_text);
    } else {
        clear_editor_for_new_service();
    }
}

fn clear_editor_for_new_service() {
    let a = app();
    a.selected_name.clear();
    a.heading.set_text("Service Details — New Service");
    a.name_field.set_text("");
    a.exec_field.set_text("");
    a.args_field.set_text("");
    a.depends_field.set_text("");
    a.wants_field.set_text("");
    a.after_field.set_text("");
    a.timeout_field.set_text("5000");
    a.autostart_toggle.set_state(1);
    a.runtime_label.set_text("Runtime: not installed");
    a.error_label.set_text("Last error: -");
}

fn save_current_service(force_enable: bool) -> bool {
    let name = read_field(&app().name_field);
    if name.is_empty() {
        set_status("Service name is required.");
        return false;
    }

    let exec = read_field(&app().exec_field);
    if exec.is_empty() {
        set_status("Executable path is required.");
        return false;
    }

    let args = read_field(&app().args_field);
    let depends = read_field(&app().depends_field);
    let wants = read_field(&app().wants_field);
    let after = read_field(&app().after_field);
    let timeout_text = read_field(&app().timeout_field);
    let timeout = parse_u32_or(&timeout_text, 5000);
    let enabled = force_enable || app().autostart_toggle.get_state() != 0;

    let a = app();
    let Some(client) = a.conf.as_mut() else {
        set_status("confd is not connected.");
        return false;
    };

    for path in [
        format!("services/{}", name),
        format!("services/{}/config", name),
    ] {
        if client.mkdir(RegistryScope::System, &path).is_err()
            && client
                .get(RegistryScope::System, &path)
                .map(|item| item.kind == NodeKind::Directory)
                .unwrap_or(false)
                == false
        {
            set_status("Failed to create service directory.");
            return false;
        }
    }

    let mut ok = true;
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "exec"),
            ConfValue::String(exec),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "args"),
            ConfValue::String(args),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "depends"),
            ConfValue::String(depends),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "wants"),
            ConfValue::String(wants),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "after"),
            ConfValue::String(after),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "startup_timeout_ms"),
            ConfValue::Int(timeout as i64),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "enabled"),
            ConfValue::Bool(enabled),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &conf_path(&name, "removed"),
            ConfValue::Bool(false),
        )
        .is_ok();

    if ok {
        app().selected_name = name;
    } else {
        set_status("Failed to save service configuration.");
    }
    ok
}

fn run_svc_command(command: &str, name: &str) {
    let args = format!("svc {} {}", command, name);
    let tid = process::spawn("/System/bin/svc", &args);
    if tid != 0 && tid != u32::MAX {
        let _ = process::detach(tid);
    } else {
        set_status("Failed to launch /System/bin/svc.");
    }
}

fn current_service_name() -> String {
    let selected = read_field(&app().name_field);
    if selected.is_empty() {
        app().selected_name.clone()
    } else {
        selected
    }
}

fn read_field(field: &ui::TextField) -> String {
    let mut buf = [0u8; 256];
    let len = field.get_text(&mut buf) as usize;
    core::str::from_utf8(&buf[..len.min(buf.len())])
        .unwrap_or("")
        .trim()
        .into()
}

fn parse_u32_or(text: &str, default: u32) -> u32 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return default;
    }

    let mut value = 0u32;
    for b in bytes {
        if !b.is_ascii_digit() {
            return default;
        }
        value = value.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    value
}

fn contains_lower(haystack: &str, needle: &str) -> bool {
    to_lower(haystack).contains(needle)
}

fn to_lower(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.as_bytes() {
        out.push(b.to_ascii_lowercase() as char);
    }
    out
}

fn set_status(text: &str) {
    app().status.set_text(text);
}
