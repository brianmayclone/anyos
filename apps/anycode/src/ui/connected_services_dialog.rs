use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::app;
use crate::logic::connected_services::{self, ConnectedServiceKind};

const DLG_W: u32 = 780;
const DLG_H: u32 = 540;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();

    let win = ui::Window::new(t("Connected Services"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 66);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("Connected Services"));
    title.set_position(22, 14);
    title.set_size(420, 24);
    title.set_font_size(18);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t(
        "Generate Rust clients for OpenAPI, REST, WSDL/SOAP and gRPC service contracts.",
    ));
    subtitle.set_position(22, 40);
    subtitle.set_size(690, 18);
    subtitle.set_font_size(11);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let tab_bar = ui::TabBar::new("Installed|Add Service|Preview");
    tab_bar.set_dock(ui::DOCK_TOP);
    tab_bar.set_size(DLG_W, 32);
    tab_bar.set_color(tc.toolbar_bg);
    tab_bar.set_style(ui::STYLE_ACTIVE_BG, tc.editor_bg);
    tab_bar.set_style(ui::STYLE_ACTIVE_TEXT, tc.text);
    tab_bar.set_style(ui::STYLE_INACTIVE_BG, tc.toolbar_bg);
    tab_bar.set_style(ui::STYLE_INACTIVE_TEXT, tc.text_secondary);
    tab_bar.set_style(ui::STYLE_HOVER_BG, tc.sidebar_bg);
    tab_bar.set_style(ui::STYLE_ACCENT, tc.accent);
    tab_bar.set_style(ui::STYLE_RADIUS, 6);
    win.add(&tab_bar);

    let installed_page = ui::View::new();
    installed_page.set_dock(ui::DOCK_FILL);
    installed_page.set_color(tc.editor_bg);
    win.add(&installed_page);

    let add_page = ui::View::new();
    add_page.set_dock(ui::DOCK_FILL);
    add_page.set_color(tc.editor_bg);
    win.add(&add_page);

    let preview_page = ui::View::new();
    preview_page.set_dock(ui::DOCK_FILL);
    preview_page.set_color(tc.editor_bg);
    win.add(&preview_page);

    tab_bar.connect_panels(&[&installed_page, &add_page, &preview_page]);

    build_installed_page(&installed_page);
    build_add_page(&add_page, win_id);
    build_preview_page(&preview_page);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 52);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_close = ui::Button::new(t("Close"));
    btn_close.set_size(92, 30);
    btn_close.set_position((DLG_W as i32) - 114, 11);
    btn_close.set_color(tc.control_bg);
    footer.add(&btn_close);
    btn_close.on_click(move |_| {
        ui::Control::from_id(win_id).set_visible(false);
    });
}

fn build_installed_page(page: &ui::View) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Installed");
    title.set_position(22, 16);
    title.set_size(240, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let tree = ui::TreeView::new(728, 330);
    tree.set_position(22, 46);
    tree.set_indent_width(18);
    tree.set_row_height(22);
    page.add(&tree);

    let services = app()
        .current_project
        .as_ref()
        .map(connected_services::services_for_project)
        .unwrap_or_default();
    let discovered = app()
        .current_project
        .as_ref()
        .map(connected_services::discover_service_contracts)
        .unwrap_or_default();
    let root = tree.add_root(&format!("Connected Services ({})", services.len()));
    tree.set_node_style(root, 1);
    tree.set_expanded(root, true);
    if services.is_empty() {
        tree.add_child(root, "No connected services generated yet");
    }
    for service in &services {
        let node = tree.add_child(
            root,
            &format!("{} ({})", service.name, service.kind.display_name()),
        );
        tree.add_child(node, &format!("Module: {}", service.module_name));
        tree.add_child(node, &format!("Endpoint/spec: {}", service.endpoint));
        tree.add_child(node, &format!("Output: {}", service.output_dir));
        tree.set_expanded(node, false);
    }
    let discovered_root = tree.add_root(&format!("Discovered Contracts ({})", discovered.len()));
    tree.set_node_style(discovered_root, 1);
    for service in &discovered {
        tree.add_child(
            discovered_root,
            &format!(
                "{}: {} -> {}",
                service.kind.display_name(),
                service.endpoint,
                service.module_name
            ),
        );
    }
    tree.set_expanded(discovered_root, false);

    let hint = ui::Label::new("Generated clients live under src/generated/services/<module>.");
    hint.set_position(22, 386);
    hint.set_size(690, 18);
    hint.set_font_size(11);
    hint.set_text_color(tc.text_secondary);
    page.add(&hint);

    let btn_regenerate = ui::Button::new("Regenerate All");
    btn_regenerate.set_position(22, 414);
    btn_regenerate.set_size(132, 30);
    btn_regenerate.set_color(tc.accent);
    page.add(&btn_regenerate);

    let btn_remove = ui::Button::new("Remove First");
    btn_remove.set_position(166, 414);
    btn_remove.set_size(112, 30);
    btn_remove.set_color(0xff7f1d1d);
    page.add(&btn_remove);

    btn_regenerate.on_click(move |_| {
        let _ = crate::logic::commands::regenerate_connected_services();
    });
    btn_remove.on_click(move |_| {
        let _ = crate::logic::commands::remove_first_connected_service();
    });
}

fn build_add_page(page: &ui::View, win_id: u32) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Add Connected Service");
    title.set_position(22, 16);
    title.set_size(280, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let kind = ui::DropDown::new("OpenAPI / REST|WSDL / SOAP|gRPC / Protobuf|REST Endpoint");
    kind.set_position(22, 52);
    kind.set_size(240, 30);
    page.add(&kind);

    let name = ui::TextField::new();
    name.set_position(282, 52);
    name.set_size(200, 30);
    name.set_placeholder("Service name");
    name.set_color(tc.control_bg);
    name.set_text_color(tc.text);
    page.add(&name);

    let module = ui::TextField::new();
    module.set_position(502, 52);
    module.set_size(220, 30);
    module.set_placeholder("rust_module_name");
    module.set_color(tc.control_bg);
    module.set_text_color(tc.text);
    page.add(&module);

    let endpoint = ui::TextField::new();
    endpoint.set_position(22, 104);
    endpoint.set_size(700, 30);
    endpoint
        .set_placeholder("https://service/openapi.json, service.wsdl, api.proto or endpoint URL");
    endpoint.set_color(tc.control_bg);
    endpoint.set_text_color(tc.text);
    page.add(&endpoint);

    let btn_generate = ui::Button::new("Generate Rust Client");
    btn_generate.set_position(22, 154);
    btn_generate.set_size(180, 32);
    btn_generate.set_color(tc.accent);
    page.add(&btn_generate);

    let status =
        ui::Label::new("Creates a transport-neutral Rust client stub and service manifest.");
    status.set_position(218, 160);
    status.set_size(504, 22);
    status.set_font_size(11);
    status.set_text_color(tc.text_secondary);
    page.add(&status);

    let details = ui::TreeView::new(700, 190);
    details.set_position(22, 210);
    details.set_indent_width(18);
    details.set_row_height(22);
    page.add(&details);
    let root = details.add_root("Generator plan");
    details.add_child(root, "OpenAPI / REST: DTO and request client skeleton");
    details.add_child(root, "WSDL / SOAP: SOAP envelope client skeleton");
    details.add_child(root, "gRPC / Protobuf: protobuf client skeleton");
    details.add_child(
        root,
        "Regeneration keeps service metadata in .anycode-connected-services",
    );
    details.set_expanded(root, true);

    let kind_id = kind.id();
    let name_id = name.id();
    let module_id = module.id();
    let endpoint_id = endpoint.id();
    let status_id = status.id();
    btn_generate.on_click(move |_| {
        let result = crate::logic::commands::add_connected_service(
            read_string(name_id),
            read_string(endpoint_id),
            read_string(module_id),
            ui::Control::from_id(kind_id).get_state(),
        );
        match result {
            Ok(msg) => {
                ui::Control::from_id(status_id).set_text(&msg);
                ui::Control::from_id(win_id).set_visible(false);
            }
            Err(msg) => {
                ui::Control::from_id(status_id).set_text(&msg);
            }
        }
    });
}

fn build_preview_page(page: &ui::View) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Generated Shape");
    title.set_position(22, 16);
    title.set_size(240, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let preview = ui::TextEditor::new(700, 320);
    preview.set_position(22, 48);
    preview.set_text(
        "src/generated/services/<module>/\n  mod.rs\n  client.rs\n  models.rs\n  README.md\n\nclient.rs contains <Service>Client::default() plus a typed error enum.\nThe transport layer is intentionally explicit so generated code stays auditable.",
    );
    page.add(&preview);
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}

#[allow(dead_code)]
fn _kind_from_index(index: u32) -> ConnectedServiceKind {
    ConnectedServiceKind::from_index(index)
}
