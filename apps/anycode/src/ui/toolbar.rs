use libanyui_client as ui;
use ui::IconType;

const ICON_SZ: u32 = 24;

/// Toolbar with PlainButton (borderless flat icon buttons).
pub struct AppToolbar {
    pub toolbar: ui::Toolbar,
    pub btn_new: ui::PlainButton,
    pub btn_open: ui::PlainButton,
    pub btn_save: ui::PlainButton,
    pub btn_save_all: ui::PlainButton,
    pub btn_build: ui::PlainButton,
    pub btn_run: ui::PlainButton,
    pub btn_stop: ui::PlainButton,
    pub btn_settings: ui::PlainButton,
}

impl AppToolbar {
    pub fn new(_parent: &impl ui::Widget) -> Self {
        let tc = ui::theme::colors();
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(1024, 42);
        toolbar.set_color(tc.sidebar_bg);
        toolbar.set_padding(4, 4, 4, 4);

        let t = anyos_std::i18n::t;

        let btn_new = make_plain_btn(&toolbar, "file-plus", tc.text, t("New File"));
        let btn_open = make_plain_btn(&toolbar, "folder-open", tc.text, t("Open Folder"));
        let btn_save = make_plain_btn(&toolbar, "device-floppy", tc.text, t("Save"));
        let btn_save_all = make_plain_btn(&toolbar, "files", tc.text, t("Save All"));

        toolbar.add_separator();

        let btn_build = make_plain_btn(&toolbar, "hammer", tc.accent, t("Build"));
        let btn_run = make_plain_btn(&toolbar, "player-play", tc.success, t("Run"));
        let btn_stop = make_plain_btn(&toolbar, "player-stop", tc.text, t("Stop"));

        toolbar.add_separator();

        let btn_settings = make_plain_btn(&toolbar, "settings", tc.text, t("Settings"));

        Self {
            toolbar,
            btn_new,
            btn_open,
            btn_save,
            btn_save_all,
            btn_build,
            btn_run,
            btn_stop,
            btn_settings,
        }
    }
}

fn make_plain_btn(toolbar: &ui::Toolbar, icon: &str, color: u32, tooltip: &str) -> ui::PlainButton {
    let btn = ui::PlainButton::new("");
    btn.set_size(34, 34);
    btn.set_system_icon(icon, IconType::Outline, color, ICON_SZ);
    btn.set_tooltip(tooltip);
    toolbar.add(&btn);
    btn
}
