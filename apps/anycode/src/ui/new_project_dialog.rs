use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::app;

const DLG_W: u32 = 560;
const DLG_H: u32 = 340;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();
    let default_name = next_default_name();
    let default_path = default_project_path(&default_name);

    let win = ui::Window::new(t("New Project"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 62);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("New Project"));
    title.set_position(22, 14);
    title.set_size(400, 22);
    title.set_font_size(17);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t(
        "Creates an app project with MainForm, Storyboard and startup entry point.",
    ));
    subtitle.set_position(22, 39);
    subtitle.set_size(500, 18);
    subtitle.set_font_size(10);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(tc.editor_bg);
    win.add(&content);

    let lbl_name = ui::Label::new(t("Project name"));
    lbl_name.set_position(24, 28);
    lbl_name.set_size(130, 18);
    lbl_name.set_text_color(tc.text);
    content.add(&lbl_name);

    let name_field = ui::TextField::new();
    name_field.set_position(150, 22);
    name_field.set_size(360, 30);
    name_field.set_color(tc.control_bg);
    name_field.set_text_color(tc.text);
    name_field.set_text(&default_name);
    name_field.select_all();
    content.add(&name_field);

    let lbl_path = ui::Label::new(t("Location"));
    lbl_path.set_position(24, 76);
    lbl_path.set_size(130, 18);
    lbl_path.set_text_color(tc.text);
    content.add(&lbl_path);

    let path_field = ui::TextField::new();
    path_field.set_position(150, 70);
    path_field.set_size(360, 30);
    path_field.set_color(tc.control_bg);
    path_field.set_text_color(tc.text);
    path_field.set_text(&default_path);
    content.add(&path_field);

    let lbl_template = ui::Label::new(t("Template"));
    lbl_template.set_position(24, 118);
    lbl_template.set_size(130, 18);
    lbl_template.set_text_color(tc.text);
    content.add(&lbl_template);

    let template_combo = ui::DropDown::new(t("Rust UI App|Node.js UI App"));
    template_combo.set_position(150, 112);
    template_combo.set_size(360, 30);
    content.add(&template_combo);

    let template = ui::Label::new(t(
        "Designer and Storyboard use the same model for both targets.",
    ));
    template.set_position(150, 110);
    template.set_position(150, 150);
    template.set_size(360, 18);
    template.set_font_size(10);
    template.set_text_color(tc.text_secondary);
    content.add(&template);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 54);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_create = ui::Button::new(t("Create"));
    btn_create.set_size(88, 30);
    btn_create.set_position((DLG_W as i32) - 198, 12);
    btn_create.set_color(tc.success);
    footer.add(&btn_create);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_size(88, 30);
    btn_cancel.set_position((DLG_W as i32) - 100, 12);
    btn_cancel.set_color(tc.control_bg);
    footer.add(&btn_cancel);

    let name_id = name_field.id();
    let path_id = path_field.id();
    let template_id = template_combo.id();
    btn_create.on_click(move |_| {
        let created = if ui::Control::from_id(template_id).get_state() == 1 {
            crate::logic::commands::create_node_ui_project_named(
                read_string(name_id),
                read_string(path_id),
            )
        } else {
            crate::logic::commands::create_rust_ui_project_named(
                read_string(name_id),
                read_string(path_id),
            )
        };
        if created {
            ui::Window::from_id(win_id).destroy();
        }
    });

    btn_cancel.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}

fn next_default_name() -> String {
    let base = "AnyCodeApp";
    let mut idx = 1u32;
    loop {
        let name = if idx == 1 {
            String::from(base)
        } else {
            format!("{}{}", base, idx)
        };
        let path = default_project_path(&name);
        if !crate::util::path::exists(&path) {
            return name;
        }
        idx += 1;
    }
}

fn default_project_path(name: &str) -> String {
    let base = app()
        .current_project
        .as_ref()
        .map(|project| String::from(crate::util::path::parent(&project.root)))
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| String::from("/Users/Shared"));
    format!("{}/{}", base, name)
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}
