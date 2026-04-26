use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::app;
use crate::logic::crates::{self, DependencyKind};

const DLG_W: u32 = 760;
const DLG_H: u32 = 520;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();

    let win = ui::Window::new(t("Manage Crates"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 64);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("Manage Crates"));
    title.set_position(22, 14);
    title.set_size(420, 24);
    title.set_font_size(18);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t(
        "Cargo dependency management for the active Rust project.",
    ));
    subtitle.set_position(22, 40);
    subtitle.set_size(560, 18);
    subtitle.set_font_size(11);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let tab_bar = ui::TabBar::new("Installed|Browse|Updates");
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

    let browse_page = ui::View::new();
    browse_page.set_dock(ui::DOCK_FILL);
    browse_page.set_color(tc.editor_bg);
    win.add(&browse_page);

    let updates_page = ui::View::new();
    updates_page.set_dock(ui::DOCK_FILL);
    updates_page.set_color(tc.editor_bg);
    win.add(&updates_page);

    tab_bar.connect_panels(&[&installed_page, &browse_page, &updates_page]);

    let deps = app()
        .current_project
        .as_ref()
        .map(crates::dependencies_for_project)
        .unwrap_or_default();

    build_installed_page(&installed_page, &deps);
    build_browse_page(&browse_page, win_id);
    build_updates_page(&updates_page, &deps);

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
        ui::Window::from_id(win_id).destroy();
    });
}

fn build_installed_page(page: &ui::View, deps: &[crates::CrateDependency]) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Installed");
    title.set_position(22, 16);
    title.set_size(240, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let tree = ui::TreeView::new(712, 330);
    tree.set_position(22, 46);
    tree.set_indent_width(18);
    tree.set_row_height(22);
    page.add(&tree);
    populate_dependencies_tree(&tree, deps);

    let hint = ui::Label::new("Installed crates are read from Cargo.toml dependency sections.");
    hint.set_position(22, 386);
    hint.set_size(620, 18);
    hint.set_font_size(11);
    hint.set_text_color(tc.text_secondary);
    page.add(&hint);
}

fn build_browse_page(page: &ui::View, win_id: u32) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Browse");
    title.set_position(22, 16);
    title.set_size(240, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let search = ui::SearchField::new();
    search.set_position(22, 48);
    search.set_size(420, 30);
    search.set_placeholder("Search crates.io...");
    page.add(&search);

    let btn_search = ui::Button::new("Search");
    btn_search.set_position(454, 48);
    btn_search.set_size(92, 30);
    btn_search.set_color(tc.accent);
    page.add(&btn_search);

    let version = ui::TextField::new();
    version.set_position(22, 96);
    version.set_size(160, 28);
    version.set_color(tc.control_bg);
    version.set_text_color(tc.text);
    version.set_text("1.0");
    page.add(&version);

    let kind = ui::DropDown::new("Dependencies|Dev Dependencies|Build Dependencies");
    kind.set_position(194, 96);
    kind.set_size(210, 28);
    page.add(&kind);

    let btn_add = ui::Button::new("Add to Project");
    btn_add.set_position(416, 96);
    btn_add.set_size(130, 28);
    btn_add.set_color(tc.success);
    page.add(&btn_add);

    let results = ui::TreeView::new(712, 220);
    results.set_position(22, 142);
    results.set_row_height(22);
    page.add(&results);
    let root = results.add_root("crates.io");
    results.add_child(root, "Search backend placeholder");
    results.set_expanded(root, true);

    let status = ui::Label::new("Use Browse to add a crate by name and version.");
    status.set_position(22, 374);
    status.set_size(690, 20);
    status.set_font_size(11);
    status.set_text_color(tc.text_secondary);
    page.add(&status);

    let search_id = search.id();
    let version_id = version.id();
    let kind_id = kind.id();
    let status_id = status.id();

    btn_search.on_click(move |_| {
        let msg = crates::search_message(&read_string(search_id));
        ui::Control::from_id(status_id).set_text(&msg);
    });

    btn_add.on_click(move |_| {
        crate::logic::commands::add_crate_dependency(
            read_string(search_id),
            read_string(version_id),
            ui::Control::from_id(kind_id).get_state(),
        );
        ui::Window::from_id(win_id).destroy();
    });
}

fn build_updates_page(page: &ui::View, deps: &[crates::CrateDependency]) {
    let tc = ui::theme::colors();
    let title = ui::Label::new("Updates");
    title.set_position(22, 16);
    title.set_size(240, 22);
    title.set_font_size(14);
    title.set_text_color(tc.text);
    page.add(&title);

    let tree = ui::TreeView::new(712, 278);
    tree.set_position(22, 46);
    tree.set_indent_width(18);
    tree.set_row_height(22);
    page.add(&tree);
    let root = tree.add_root("Available Updates");
    tree.add_child(
        root,
        "Online crates.io version lookup is prepared but not connected yet",
    );
    for dep in deps {
        tree.add_child(
            root,
            &format!("{} {} ({})", dep.name, dep.version, dep.package_name),
        );
    }
    tree.set_expanded(root, true);

    let update_name = ui::TextField::new();
    update_name.set_position(22, 342);
    update_name.set_size(180, 28);
    update_name.set_color(tc.control_bg);
    update_name.set_text_color(tc.text);
    update_name.set_placeholder("crate name");
    page.add(&update_name);

    let update_version = ui::TextField::new();
    update_version.set_position(214, 342);
    update_version.set_size(120, 28);
    update_version.set_color(tc.control_bg);
    update_version.set_text_color(tc.text);
    update_version.set_placeholder("version");
    page.add(&update_version);

    let kind = ui::DropDown::new("Dependencies|Dev Dependencies|Build Dependencies");
    kind.set_position(346, 342);
    kind.set_size(210, 28);
    page.add(&kind);

    let btn_update = ui::Button::new("Update");
    btn_update.set_position(568, 342);
    btn_update.set_size(92, 28);
    btn_update.set_color(tc.success);
    page.add(&btn_update);

    let status = ui::Label::new(&crates::update_check_message(deps.len()));
    status.set_position(22, 382);
    status.set_size(690, 20);
    status.set_font_size(11);
    status.set_text_color(tc.text_secondary);
    page.add(&status);

    let name_id = update_name.id();
    let version_id = update_version.id();
    let kind_id = kind.id();
    btn_update.on_click(move |_| {
        crate::logic::commands::update_crate_dependency(
            read_string(name_id),
            read_string(version_id),
            ui::Control::from_id(kind_id).get_state(),
        );
    });
}

fn populate_dependencies_tree(tree: &ui::TreeView, deps: &[crates::CrateDependency]) {
    let root = tree.add_root("Cargo Dependencies");
    if deps.is_empty() {
        tree.add_child(root, "No dependencies found");
    }
    for kind in [
        DependencyKind::Normal,
        DependencyKind::Dev,
        DependencyKind::Build,
    ] {
        let section = tree.add_child(root, kind.display_name());
        for dep in deps.iter().filter(|dep| dep.kind == kind) {
            tree.add_child(
                section,
                &format!(
                    "{} {} - {} ({})",
                    dep.name, dep.version, dep.package_name, dep.manifest_path
                ),
            );
        }
        tree.set_expanded(section, true);
    }
    tree.set_expanded(root, true);
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}
