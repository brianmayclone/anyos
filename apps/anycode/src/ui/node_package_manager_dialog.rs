use libanyui_client as ui;
use ui::Widget;

use crate::app;
use crate::logic::node_packages::{self, NodeDependencyKind};

const DLG_W: u32 = 680;
const DLG_H: u32 = 520;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();
    let Some(project) = app().current_project.as_ref() else {
        return;
    };
    let packages = node_packages::packages_for_project(project);

    let win = ui::Window::new(t("Manage NPM Packages"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 58);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("Manage NPM Packages"));
    title.set_position(22, 13);
    title.set_size(360, 22);
    title.set_font_size(17);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t("Installed, Browse and Updates for package.json."));
    subtitle.set_position(22, 37);
    subtitle.set_size(560, 18);
    subtitle.set_font_size(10);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let tab_bar = ui::TabBar::new(t("Installed|Browse|Updates"));
    tab_bar.set_dock(ui::DOCK_TOP);
    tab_bar.set_size(DLG_W, 32);
    tab_bar.set_color(tc.toolbar_bg);
    win.add(&tab_bar);

    let installed = ui::View::new();
    installed.set_dock(ui::DOCK_FILL);
    installed.set_color(tc.editor_bg);
    win.add(&installed);
    build_installed_page(&installed, &packages);

    let browse = ui::View::new();
    browse.set_dock(ui::DOCK_FILL);
    browse.set_color(tc.editor_bg);
    win.add(&browse);
    build_browse_page(&browse);

    let updates = ui::View::new();
    updates.set_dock(ui::DOCK_FILL);
    updates.set_color(tc.editor_bg);
    win.add(&updates);
    build_updates_page(&updates, &packages);

    tab_bar.connect_panels(&[&installed, &browse, &updates]);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 52);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_install = ui::Button::new(t("Install/Restore"));
    btn_install.set_size(128, 30);
    btn_install.set_position((DLG_W as i32) - 252, 11);
    btn_install.set_color(tc.accent);
    footer.add(&btn_install);

    let btn_close = ui::Button::new(t("Close"));
    btn_close.set_size(88, 30);
    btn_close.set_position((DLG_W as i32) - 110, 11);
    btn_close.set_color(tc.control_bg);
    footer.add(&btn_close);

    btn_install.on_click(move |_| {
        crate::logic::commands::restore_node_packages();
    });
    btn_close.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}

fn build_installed_page(page: &ui::View, packages: &[node_packages::NodePackage]) {
    let tc = ui::theme::colors();
    let tree = ui::TreeView::new(620, 280);
    tree.set_position(20, 18);
    tree.set_size(620, 250);
    page.add(&tree);

    let root = tree.add_root("package.json");
    tree.set_node_style(root, 1);
    for kind in [
        NodeDependencyKind::Runtime,
        NodeDependencyKind::Dev,
        NodeDependencyKind::Optional,
    ] {
        let count = packages.iter().filter(|pkg| pkg.kind == kind).count();
        let section = tree.add_child(root, &alloc::format!("{} ({})", kind.display_name(), count));
        tree.set_node_text_color(section, tc.text_secondary);
        for package in packages.iter().filter(|pkg| pkg.kind == kind) {
            tree.add_child(
                section,
                &alloc::format!("{} {}", package.name, package.version),
            );
        }
        tree.set_expanded(section, true);
    }
    tree.set_expanded(root, true);

    let remove_name = text_row(page, "Remove", "", 286);

    let btn_remove = ui::Button::new("Remove");
    btn_remove.set_position(430, 286);
    btn_remove.set_size(120, 30);
    btn_remove.set_color(tc.destructive);
    page.add(&btn_remove);

    let remove_id = remove_name.id();
    btn_remove.on_click(move |_| {
        crate::logic::commands::remove_node_package(read_string(remove_id));
    });

    let hint = ui::Label::new(
        "Browse adds packages, Updates runs npm outdated/update, Install/Restore runs npm install.",
    );
    hint.set_position(22, 336);
    hint.set_size(600, 18);
    hint.set_font_size(10);
    hint.set_text_color(tc.text_secondary);
    page.add(&hint);
}

fn build_browse_page(page: &ui::View) {
    let tc = ui::theme::colors();
    let name = text_row(page, "Package", "", 22);
    let version = text_row(page, "Version", "latest", 68);

    let kind = ui::DropDown::new("Dependencies|Dev Dependencies|Optional Dependencies");
    kind.set_position(130, 114);
    kind.set_size(280, 28);
    page.add(&kind);

    let btn_add = ui::Button::new("Add / Update");
    btn_add.set_position(430, 114);
    btn_add.set_size(120, 30);
    btn_add.set_color(tc.accent);
    page.add(&btn_add);

    let name_id = name.id();
    let version_id = version.id();
    let kind_id = kind.id();
    btn_add.on_click(move |_| {
        let kind = ui::Control::from_id(kind_id).get_state();
        crate::logic::commands::add_node_package(
            read_string(name_id),
            read_string(version_id),
            kind,
        );
    });

    let note =
        ui::Label::new("Quick packages write package.json safely; run Install/Restore afterwards.");
    note.set_position(130, 158);
    note.set_size(470, 18);
    note.set_font_size(10);
    note.set_text_color(tc.text_secondary);
    page.add(&note);

    let quick_title = ui::Label::new("Common packages");
    quick_title.set_position(24, 196);
    quick_title.set_size(220, 18);
    quick_title.set_text_color(tc.text);
    page.add(&quick_title);

    for (idx, package) in node_packages::SUGGESTED_PACKAGES.iter().enumerate() {
        let x = if idx % 2 == 0 { 24 } else { 334 };
        let y = 226 + ((idx / 2) as i32) * 48;
        quick_package_row(page, package, x, y);
    }
}

fn build_updates_page(page: &ui::View, packages: &[node_packages::NodePackage]) {
    let tc = ui::theme::colors();
    let text = ui::Label::new(&node_packages::update_check_message(packages.len()));
    text.set_position(22, 24);
    text.set_size(600, 24);
    text.set_text_color(tc.text);
    page.add(&text);

    let hint =
        ui::Label::new("Use npm outdated/update once the project has restored node_modules.");
    hint.set_position(22, 54);
    hint.set_size(600, 18);
    hint.set_font_size(10);
    hint.set_text_color(tc.text_secondary);
    page.add(&hint);

    let btn_check = ui::Button::new("Check Updates");
    btn_check.set_position(22, 92);
    btn_check.set_size(130, 30);
    btn_check.set_color(tc.control_bg);
    page.add(&btn_check);
    btn_check.on_click(move |_| {
        crate::logic::commands::check_node_package_updates();
    });

    let btn_update = ui::Button::new("Update Packages");
    btn_update.set_position(166, 92);
    btn_update.set_size(140, 30);
    btn_update.set_color(tc.accent);
    page.add(&btn_update);
    btn_update.on_click(move |_| {
        crate::logic::commands::update_node_packages();
    });
}

fn text_row(page: &ui::View, label: &str, value: &str, y: i32) -> ui::TextField {
    let tc = ui::theme::colors();
    let lbl = ui::Label::new(label);
    lbl.set_position(24, y + 5);
    lbl.set_size(100, 18);
    lbl.set_text_color(tc.text);
    page.add(&lbl);

    let field = ui::TextField::new();
    field.set_position(130, y);
    field.set_size(280, 30);
    field.set_color(tc.control_bg);
    field.set_text_color(tc.text);
    field.set_text(value);
    page.add(&field);
    field
}

fn quick_package_row(
    page: &ui::View,
    package: &node_packages::SuggestedNodePackage,
    x: i32,
    y: i32,
) {
    let tc = ui::theme::colors();
    let btn = ui::Button::new(package.name);
    btn.set_position(x, y);
    btn.set_size(138, 30);
    btn.set_color(match package.kind {
        NodeDependencyKind::Runtime => tc.accent,
        NodeDependencyKind::Dev => tc.control_bg,
        NodeDependencyKind::Optional => tc.control_bg,
    });
    page.add(&btn);

    let description = ui::Label::new(package.description);
    description.set_position(x + 148, y + 6);
    description.set_size(150, 18);
    description.set_font_size(10);
    description.set_text_color(tc.text_secondary);
    page.add(&description);

    let name = package.name;
    let version = package.version;
    let kind = match package.kind {
        NodeDependencyKind::Runtime => 0,
        NodeDependencyKind::Dev => 1,
        NodeDependencyKind::Optional => 2,
    };
    btn.on_click(move |_| {
        crate::logic::commands::add_node_package(name.into(), version.into(), kind);
    });
}

fn read_string(id: u32) -> alloc::string::String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}
