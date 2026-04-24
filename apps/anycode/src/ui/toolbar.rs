use libanyui_client as ui;
use ui::IconType;

use crate::logic::config::Config;

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
    pub fn new(_parent: &impl ui::Widget, config: &Config) -> Self {
        let tc = ui::theme::colors();
        let toolbar_h = (config.font_size + 24).max(38);
        let icon_sz = (config.font_size + 9).min(24);
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(1024, toolbar_h);
        toolbar.set_color(tc.toolbar_bg);
        toolbar.set_padding(8, 4, 8, 4);

        let t = anyos_std::i18n::t;

        let btn_new = make_plain_btn(
            &toolbar,
            "file-plus",
            tc.text,
            t("New File"),
            toolbar_h,
            icon_sz,
        );
        let btn_open = make_plain_btn(
            &toolbar,
            "folder-open",
            tc.text,
            t("Open Folder"),
            toolbar_h,
            icon_sz,
        );
        let btn_save = make_plain_btn(
            &toolbar,
            "device-floppy",
            tc.text,
            t("Save"),
            toolbar_h,
            icon_sz,
        );
        let btn_save_all = make_plain_btn(
            &toolbar,
            "files",
            tc.text,
            t("Save All"),
            toolbar_h,
            icon_sz,
        );

        toolbar.add_separator();

        let btn_build = make_plain_btn(
            &toolbar,
            "hammer",
            tc.accent,
            t("Build"),
            toolbar_h,
            icon_sz,
        );
        let btn_run = make_plain_btn(
            &toolbar,
            "player-play",
            tc.success,
            t("Run"),
            toolbar_h,
            icon_sz,
        );
        let btn_stop = make_plain_btn(
            &toolbar,
            "player-stop",
            tc.text_secondary,
            t("Stop"),
            toolbar_h,
            icon_sz,
        );

        toolbar.add_separator();

        let btn_settings = make_plain_btn(
            &toolbar,
            "settings",
            tc.text,
            t("Settings"),
            toolbar_h,
            icon_sz,
        );

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

fn make_plain_btn(
    toolbar: &ui::Toolbar,
    icon: &str,
    color: u32,
    tooltip: &str,
    toolbar_h: u32,
    icon_sz: u32,
) -> ui::PlainButton {
    let btn = ui::PlainButton::new("");
    let btn_size = toolbar_h.saturating_sub(8).max(30);
    btn.set_size(btn_size, btn_size);
    btn.set_system_icon(icon, IconType::Outline, color, icon_sz);
    btn.set_tooltip(tooltip);
    toolbar.add(&btn);
    btn
}
