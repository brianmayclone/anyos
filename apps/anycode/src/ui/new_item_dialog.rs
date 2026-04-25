use libanyui_client as ui;
use ui::Widget;

const DLG_W: u32 = 420;
const DLG_H: u32 = 210;

pub fn show() {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();

    let win = ui::Window::new(t("New"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 58);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(t("New Item"));
    title.set_position(22, 14);
    title.set_size(320, 22);
    title.set_font_size(17);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t("Choose what you want to create."));
    subtitle.set_position(22, 38);
    subtitle.set_size(340, 18);
    subtitle.set_font_size(10);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(tc.editor_bg);
    win.add(&content);

    let btn_file = ui::Button::new(t("Text File"));
    btn_file.set_position(24, 24);
    btn_file.set_size(168, 42);
    btn_file.set_color(tc.control_bg);
    content.add(&btn_file);

    let btn_form = ui::Button::new(t("UI Form"));
    btn_form.set_position(212, 24);
    btn_form.set_size(168, 42);
    btn_form.set_color(tc.accent);
    content.add(&btn_form);

    let hint = ui::Label::new(t(
        "UI Forms create .Designer metadata plus Rust codebehind files.",
    ));
    hint.set_position(24, 82);
    hint.set_size(360, 18);
    hint.set_font_size(10);
    hint.set_text_color(tc.text_secondary);
    content.add(&hint);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 54);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_size(88, 30);
    btn_cancel.set_position((DLG_W as i32) - 110, 12);
    btn_cancel.set_color(tc.control_bg);
    footer.add(&btn_cancel);

    btn_file.on_click(move |_| {
        crate::logic::commands::new_text_file();
        ui::Control::from_id(win_id).set_visible(false);
    });

    btn_form.on_click(move |_| {
        crate::logic::commands::show_new_ui_form_dialog();
        ui::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui::Control::from_id(win_id).set_visible(false);
    });
}
