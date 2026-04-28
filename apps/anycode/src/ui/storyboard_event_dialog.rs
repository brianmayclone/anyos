use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client as ui;
use ui::Widget;

const DLG_W: u32 = 420;
const DLG_H: u32 = 214;

pub fn show(
    title_text: &str,
    event_items: &str,
    mut on_accept: impl FnMut(String) -> bool + 'static,
) {
    let t = anyos_std::i18n::t;
    let tc = ui::theme::colors();
    let items = if event_items.trim().is_empty() {
        String::from("OnClick")
    } else {
        String::from(event_items)
    };
    let events = split_items(&items);

    let win = ui::Window::new(t("Storyboard Trigger"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 58);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let title = ui::Label::new(title_text);
    title.set_position(22, 13);
    title.set_size(360, 22);
    title.set_font_size(16);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t("Choose which control event starts this transition."));
    subtitle.set_position(22, 38);
    subtitle.set_size(360, 18);
    subtitle.set_font_size(10);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(tc.editor_bg);
    win.add(&content);

    let lbl = ui::Label::new(t("Trigger event"));
    lbl.set_position(24, 28);
    lbl.set_size(130, 18);
    lbl.set_font_size(11);
    lbl.set_text_color(tc.text_secondary);
    content.add(&lbl);

    let dropdown = ui::DropDown::new(&items);
    dropdown.set_position(150, 22);
    dropdown.set_size(230, 28);
    dropdown.set_selected_index(0);
    content.add(&dropdown);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(DLG_W, 54);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_ok = ui::Button::new(t("OK"));
    btn_ok.set_position((DLG_W as i32) - 204, 12);
    btn_ok.set_size(86, 30);
    btn_ok.set_color(tc.accent);
    footer.add(&btn_ok);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_position((DLG_W as i32) - 110, 12);
    btn_cancel.set_size(88, 30);
    btn_cancel.set_color(tc.control_bg);
    footer.add(&btn_cancel);

    btn_ok.on_click(move |_| {
        let idx = dropdown.selected_index() as usize;
        let event = events
            .get(idx)
            .cloned()
            .unwrap_or_else(|| String::from("OnClick"));
        if on_accept(event) {
            ui::Window::from_id(win_id).destroy();
        }
    });

    btn_cancel.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}

fn split_items(items: &str) -> Vec<String> {
    let mut out = Vec::new();
    for item in items.split('|') {
        let item = item.trim();
        if !item.is_empty() {
            out.push(String::from(item));
        }
    }
    if out.is_empty() {
        out.push(String::from("OnClick"));
    }
    out
}
