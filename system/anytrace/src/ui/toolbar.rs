//! Debug toolbar with attach, suspend, resume, step, and snapshot buttons.

use libanyui_client as ui;
use ui::IconType;

const ICON_SZ: u32 = 24;

/// Debug toolbar button handles.
pub struct DebugToolbar {
    pub toolbar: ui::Toolbar,
    pub btn_attach: ui::IconButton,
    pub btn_detach: ui::IconButton,
    pub btn_suspend: ui::IconButton,
    pub btn_resume: ui::IconButton,
    pub btn_step_into: ui::IconButton,
    pub btn_step_over: ui::IconButton,
    pub btn_step_out: ui::IconButton,
    pub btn_snapshot: ui::IconButton,
}

impl DebugToolbar {
    /// Create the toolbar and add it to the parent window.
    pub fn new(_parent: &impl ui::Widget) -> Self {
        let tc = ui::theme::colors();
        let toolbar = ui::Toolbar::new();
        toolbar.set_dock(ui::DOCK_TOP);
        toolbar.set_size(1400, 42);
        toolbar.set_color(tc.sidebar_bg);
        toolbar.set_padding(4, 4, 4, 4);

        let btn_attach = toolbar.add_icon_button("");
        btn_attach.set_size(34, 34);
        btn_attach.set_system_icon("plug", IconType::Outline, tc.success, ICON_SZ);
        btn_attach.set_tooltip("Attach to Process");

        let btn_detach = toolbar.add_icon_button("");
        btn_detach.set_size(34, 34);
        btn_detach.set_system_icon("plug-off", IconType::Outline, tc.destructive, ICON_SZ);
        btn_detach.set_tooltip("Detach");
        btn_detach.set_enabled(false);

        toolbar.add_separator();

        let btn_suspend = toolbar.add_icon_button("");
        btn_suspend.set_size(34, 34);
        btn_suspend.set_system_icon("player-pause", IconType::Outline, tc.warning, ICON_SZ);
        btn_suspend.set_tooltip("Suspend");
        btn_suspend.set_enabled(false);

        let btn_resume = toolbar.add_icon_button("");
        btn_resume.set_size(34, 34);
        btn_resume.set_system_icon("player-play", IconType::Outline, tc.success, ICON_SZ);
        btn_resume.set_tooltip("Resume");
        btn_resume.set_enabled(false);

        let btn_step_into = toolbar.add_icon_button("");
        btn_step_into.set_size(34, 34);
        btn_step_into.set_system_icon("arrow-down", IconType::Outline, tc.text, ICON_SZ);
        btn_step_into.set_tooltip("Step Into (F11)");
        btn_step_into.set_enabled(false);

        let btn_step_over = toolbar.add_icon_button("");
        btn_step_over.set_size(34, 34);
        btn_step_over.set_system_icon("arrow-right", IconType::Outline, tc.text, ICON_SZ);
        btn_step_over.set_tooltip("Step Over (F10)");
        btn_step_over.set_enabled(false);

        let btn_step_out = toolbar.add_icon_button("");
        btn_step_out.set_size(34, 34);
        btn_step_out.set_system_icon("arrow-up", IconType::Outline, tc.text, ICON_SZ);
        btn_step_out.set_tooltip("Step Out (Shift+F11)");
        btn_step_out.set_enabled(false);

        toolbar.add_separator();

        let btn_snapshot = toolbar.add_icon_button("");
        btn_snapshot.set_size(34, 34);
        btn_snapshot.set_system_icon("camera", IconType::Outline, tc.accent, ICON_SZ);
        btn_snapshot.set_tooltip("Take Snapshot");
        btn_snapshot.set_enabled(false);

        Self {
            toolbar,
            btn_attach,
            btn_detach,
            btn_suspend,
            btn_resume,
            btn_step_into,
            btn_step_over,
            btn_step_out,
            btn_snapshot,
        }
    }

    /// Update button enabled states based on debug session state.
    pub fn update_state(&self, attached: bool, suspended: bool) {
        self.btn_attach.set_enabled(!attached);
        self.btn_detach.set_enabled(attached);
        self.btn_suspend.set_enabled(attached && !suspended);
        self.btn_resume.set_enabled(attached && suspended);
        self.btn_step_into.set_enabled(attached && suspended);
        self.btn_step_over.set_enabled(attached && suspended);
        self.btn_step_out.set_enabled(attached && suspended);
        self.btn_snapshot.set_enabled(attached && suspended);
    }
}
