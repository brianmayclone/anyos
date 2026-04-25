use libanyui_client as ui;
use ui::IconType;

use crate::logic::config::Config;

/// Main IDE toolbar with product context and borderless action buttons.
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
        toolbar.set_padding(10, 5, 10, 5);

        let brand = ui::View::new();
        brand.set_size(188, toolbar_h.saturating_sub(10).max(30));
        brand.set_color(tc.toolbar_bg);

        let brand_accent = ui::View::new();
        brand_accent.set_dock(ui::DOCK_LEFT);
        brand_accent.set_size(3, toolbar_h.saturating_sub(10).max(30));
        brand_accent.set_color(tc.accent);
        brand.add(&brand_accent);

        let brand_title = ui::Label::new("anyCode");
        brand_title.set_position(12, 2);
        brand_title.set_size(82, 15);
        brand_title.set_font_size(13);
        brand_title.set_text_color(tc.text);
        brand.add(&brand_title);

        let brand_subtitle = ui::Label::new("Rust Studio");
        brand_subtitle.set_position(12, 18);
        brand_subtitle.set_size(112, 12);
        brand_subtitle.set_font_size(10);
        brand_subtitle.set_text_color(tc.text_secondary);
        brand.add(&brand_subtitle);

        let brand_pill = ui::Label::new("ccargo");
        brand_pill.set_position(124, 8);
        brand_pill.set_size(58, 16);
        brand_pill.set_font_size(10);
        brand_pill.set_text_color(tc.accent);
        brand_pill.set_text_align(ui::TEXT_ALIGN_CENTER);
        brand.add(&brand_pill);

        toolbar.add(&brand);
        toolbar.add_separator();

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

        let spacer = ui::View::new();
        spacer.set_size(18, toolbar_h.saturating_sub(8).max(30));
        spacer.set_color(tc.toolbar_bg);
        toolbar.add(&spacer);

        let status = ui::Label::new("Ready");
        status.set_size(68, toolbar_h.saturating_sub(10).max(28));
        status.set_font_size(11);
        status.set_text_color(tc.text_secondary);
        status.set_text_align(ui::TEXT_ALIGN_CENTER);
        toolbar.add(&status);

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
