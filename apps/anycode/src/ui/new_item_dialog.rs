use libanyui_client as ui;
use ui::Widget;

const DLG_W: u32 = 420;
const DLG_H: u32 = 330;

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

    let btn_storyboard = ui::Button::new(t("Storyboard"));
    btn_storyboard.set_position(24, 76);
    btn_storyboard.set_size(168, 38);
    btn_storyboard.set_color(tc.control_bg);
    content.add(&btn_storyboard);

    let btn_service = ui::Button::new(t("Connected Service"));
    btn_service.set_position(212, 76);
    btn_service.set_size(168, 38);
    btn_service.set_color(tc.control_bg);
    content.add(&btn_service);

    let btn_project = ui::Button::new(t("Rust UI App"));
    btn_project.set_position(24, 124);
    btn_project.set_size(168, 38);
    btn_project.set_color(tc.success);
    content.add(&btn_project);

    let hint = ui::Label::new(t(
        "UI Forms create .Designer metadata; Storyboards connect Forms and generate navigation handlers.",
    ));
    hint.set_position(24, 176);
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
        ui::Window::from_id(win_id).destroy();
    });

    btn_form.on_click(move |_| {
        crate::logic::commands::show_new_ui_form_dialog();
        ui::Window::from_id(win_id).destroy();
    });

    btn_storyboard.on_click(move |_| {
        crate::logic::commands::show_new_storyboard_dialog();
        ui::Window::from_id(win_id).destroy();
    });

    btn_service.on_click(move |_| {
        crate::logic::commands::manage_connected_services();
        ui::Window::from_id(win_id).destroy();
    });

    btn_project.on_click(move |_| {
        crate::logic::commands::show_new_project_dialog();
        ui::Window::from_id(win_id).destroy();
    });

    btn_cancel.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}
