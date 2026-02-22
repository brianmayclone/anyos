use libanyui_client as ui;

/// Toolbar button handles for event wiring.
pub struct AppToolbar {
    pub toolbar: ui::Toolbar,
    pub btn_new: ui::IconButton,
    pub btn_open: ui::IconButton,
    pub btn_save: ui::IconButton,
    pub btn_save_all: ui::IconButton,
    pub btn_build: ui::IconButton,
    pub btn_run: ui::IconButton,
    pub btn_stop: ui::IconButton,
    pub btn_settings: ui::IconButton,
}

impl AppToolbar {
    /// Create the toolbar with all icon buttons and add it to the parent.
    pub fn new(_parent: &impl ui::Widget) -> Self {
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(900, 36);
        toolbar.set_color(0xFF252526);
        toolbar.set_padding(4, 4, 4, 4);

        let btn_new = toolbar.add_icon_button("");
        btn_new.set_size(28, 28);
        btn_new.set_icon(ui::ICON_NEW_FILE);

        let btn_open = toolbar.add_icon_button("");
        btn_open.set_size(28, 28);
        btn_open.set_icon(ui::ICON_FOLDER_OPEN);

        let btn_save = toolbar.add_icon_button("");
        btn_save.set_size(28, 28);
        btn_save.set_icon(ui::ICON_SAVE);

        let btn_save_all = toolbar.add_icon_button("");
        btn_save_all.set_size(28, 28);
        btn_save_all.set_icon(ui::ICON_SAVE_ALL);

        toolbar.add_separator();

        let btn_build = toolbar.add_icon_button("");
        btn_build.set_size(28, 28);
        btn_build.set_icon(ui::ICON_BUILD);
        btn_build.set_color(0xFF0E639C);

        let btn_run = toolbar.add_icon_button("");
        btn_run.set_size(28, 28);
        btn_run.set_icon(ui::ICON_PLAY);
        btn_run.set_color(0xFF388A34);

        let btn_stop = toolbar.add_icon_button("");
        btn_stop.set_size(28, 28);
        btn_stop.set_icon(ui::ICON_STOP);

        toolbar.add_separator();

        let btn_settings = toolbar.add_icon_button("");
        btn_settings.set_size(28, 28);
        btn_settings.set_icon(ui::ICON_SETTINGS);

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
