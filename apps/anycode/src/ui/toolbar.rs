use libanyui_client as ui;

/// Toolbar button handles for event wiring.
pub struct AppToolbar {
    pub toolbar: ui::Toolbar,
    pub btn_new: ui::Button,
    pub btn_open: ui::Button,
    pub btn_save: ui::Button,
    pub btn_save_all: ui::Button,
    pub btn_build: ui::Button,
    pub btn_run: ui::Button,
    pub btn_stop: ui::Button,
    pub btn_settings: ui::Button,
}

impl AppToolbar {
    /// Create the toolbar with all buttons and add it to the parent.
    pub fn new(_parent: &impl ui::Widget) -> Self {
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(900, 36);
        toolbar.set_color(0xFF252526);

        let btn_new = toolbar.add_button("New");
        btn_new.set_size(60, 28);

        let btn_open = toolbar.add_button("Open Folder");
        btn_open.set_size(90, 28);

        let btn_save = toolbar.add_button("Save");
        btn_save.set_size(60, 28);

        let btn_save_all = toolbar.add_button("Save All");
        btn_save_all.set_size(70, 28);

        toolbar.add_separator();

        let btn_build = toolbar.add_button("Build");
        btn_build.set_size(60, 28);
        btn_build.set_color(0xFF0E639C);

        let btn_run = toolbar.add_button("Run");
        btn_run.set_size(60, 28);
        btn_run.set_color(0xFF388A34);

        let btn_stop = toolbar.add_button("Stop");
        btn_stop.set_size(60, 28);

        toolbar.add_separator();

        let btn_settings = toolbar.add_button("Settings");
        btn_settings.set_size(70, 28);

        // parent.add() — toolbar needs to be added by the caller
        // since parent might be Window (which is a Container)

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
