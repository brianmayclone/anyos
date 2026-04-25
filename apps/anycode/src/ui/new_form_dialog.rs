use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

const DLG_W: u32 = 460;
const DLG_H: u32 = 210;

pub fn show(default_name: &str) {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();

    let win = ui::Window::new(t("New UI Form"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 58);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("New UI Form"));
    title.set_position(22, 14);
    title.set_size(360, 22);
    title.set_font_size(17);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t("Create a Rust UI form with designer metadata."));
    subtitle.set_position(22, 38);
    subtitle.set_size(390, 18);
    subtitle.set_font_size(10);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(tc.editor_bg);
    win.add(&content);

    let label = ui::Label::new(t("Form name"));
    label.set_position(24, 26);
    label.set_size(120, 18);
    label.set_text_color(tc.text);
    content.add(&label);

    let field = ui::TextField::new();
    field.set_position(126, 20);
    field.set_size(300, 30);
    field.set_color(tc.control_bg);
    field.set_text_color(tc.text);
    field.set_text(default_name);
    field.select_all();
    content.add(&field);

    let hint = ui::Label::new(t("Use a Rust type name, e.g. MainForm or SettingsDialog."));
    hint.set_position(126, 55);
    hint.set_size(300, 18);
    hint.set_font_size(10);
    hint.set_text_color(tc.text_secondary);
    content.add(&hint);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 54);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_create = ui::Button::new(t("Create"));
    btn_create.set_size(88, 30);
    btn_create.set_position((DLG_W as i32) - 196, 12);
    btn_create.set_color(tc.success);
    footer.add(&btn_create);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_size(88, 30);
    btn_cancel.set_position((DLG_W as i32) - 100, 12);
    btn_cancel.set_color(tc.control_bg);
    footer.add(&btn_cancel);

    let field_id = field.id();
    btn_create.on_click(move |_| {
        crate::logic::commands::create_ui_form_named(read_string(field_id));
        ui::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui::Control::from_id(win_id).set_visible(false);
    });
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 256];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}
