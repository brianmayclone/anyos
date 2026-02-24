use libanyui_client as ui;
use ui::IconType;

const ICON_SZ: u32 = 24;

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
        let tc = ui::theme::colors();
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(900, 42);
        toolbar.set_color(tc.sidebar_bg);
        toolbar.set_padding(4, 4, 4, 4);

        let btn_new = toolbar.add_icon_button("");
        btn_new.set_size(34, 34);
        btn_new.set_system_icon("file-plus", IconType::Outline, tc.text, ICON_SZ);
        btn_new.set_tooltip("New File");

        let btn_open = toolbar.add_icon_button("");
        btn_open.set_size(34, 34);
        btn_open.set_system_icon("folder-open", IconType::Outline, tc.text, ICON_SZ);
        btn_open.set_tooltip("Open Folder");

        let btn_save = toolbar.add_icon_button("");
        btn_save.set_size(34, 34);
        btn_save.set_system_icon("device-floppy", IconType::Outline, tc.text, ICON_SZ);
        btn_save.set_tooltip("Save");

        let btn_save_all = toolbar.add_icon_button("");
        btn_save_all.set_size(34, 34);
        btn_save_all.set_system_icon("files", IconType::Outline, tc.text, ICON_SZ);
        btn_save_all.set_tooltip("Save All");

        toolbar.add_separator();

        let btn_build = toolbar.add_icon_button("");
        btn_build.set_size(34, 34);
        btn_build.set_system_icon("hammer", IconType::Outline, tc.check_mark, ICON_SZ);
        btn_build.set_color(tc.accent);
        btn_build.set_tooltip("Build");

        let btn_run = toolbar.add_icon_button("");
        btn_run.set_size(34, 34);
        btn_run.set_system_icon("player-play", IconType::Outline, tc.check_mark, ICON_SZ);
        btn_run.set_color(tc.success);
        btn_run.set_tooltip("Run");

        let btn_stop = toolbar.add_icon_button("");
        btn_stop.set_size(34, 34);
        btn_stop.set_system_icon("player-stop", IconType::Outline, tc.text, ICON_SZ);
        btn_stop.set_tooltip("Stop");

        toolbar.add_separator();

        let btn_settings = toolbar.add_icon_button("");
        btn_settings.set_size(34, 34);
        btn_settings.set_system_icon("settings", IconType::Outline, tc.text, ICON_SZ);
        btn_settings.set_tooltip("Settings");

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
