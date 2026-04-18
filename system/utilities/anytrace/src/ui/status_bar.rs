//! Status bar with state, CPU, memory, and uptime info.

use libanyui_client as ui;
use ui::Widget;

/// Status bar panel.
pub struct StatusBar {
    pub view: ui::View,
    pub lbl_state: ui::Label,
    pub lbl_cpu: ui::Label,
    pub lbl_memory: ui::Label,
    pub lbl_uptime: ui::Label,
}

impl StatusBar {
    /// Create the status bar.
    pub fn new(_parent: &impl Widget) -> Self {
        let tc = ui::theme::colors();
        let view = ui::View::new();
        view.set_dock(ui::DOCK_BOTTOM);
        view.set_size(1400, 24);
        view.set_color(tc.sidebar_bg);

        let lbl_state = ui::Label::new("Detached");
        lbl_state.set_position(8, 3);
        lbl_state.set_size(250, 18);
        view.add(&lbl_state);

        let lbl_cpu = ui::Label::new("CPU: -");
        lbl_cpu.set_position(270, 3);
        lbl_cpu.set_size(120, 18);
        view.add(&lbl_cpu);

        let lbl_memory = ui::Label::new("Memory: -");
        lbl_memory.set_position(400, 3);
        lbl_memory.set_size(150, 18);
        view.add(&lbl_memory);

        let lbl_uptime = ui::Label::new("Uptime: -");
        lbl_uptime.set_position(560, 3);
        lbl_uptime.set_size(150, 18);
        view.add(&lbl_uptime);

        Self { view, lbl_state, lbl_cpu, lbl_memory, lbl_uptime }
    }

    /// Update the state label.
    pub fn set_state(&self, text: &str) {
        self.lbl_state.set_text(text);
    }

    /// Update the uptime display.
    pub fn update_uptime(&self) {
        let ms = anyos_std::sys::uptime_ms();
        let secs = ms / 1000;
        let mins = secs / 60;
        let hours = mins / 60;
        let text = alloc::format!(
            "Uptime: {:02}:{:02}:{:02}",
            hours, mins % 60, secs % 60
        );
        self.lbl_uptime.set_text(&text);
    }
}
