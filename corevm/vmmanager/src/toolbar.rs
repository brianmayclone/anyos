use eframe::egui;

/// Actions the toolbar can trigger
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolbarAction {
    Start,
    Pause,
    Stop,
    Settings,
    Snapshot,
}

/// Render the toolbar. Returns an action if a button was clicked.
pub fn render_toolbar(
    ui: &mut egui::Ui,
    vm_selected: bool,
    vm_running: bool,
    vm_paused: bool,
) -> Option<ToolbarAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 0.0);

        // Start button
        let start_enabled = vm_selected && !vm_running;
        if ui
            .add_enabled(
                start_enabled,
                egui::Button::new("▶ Start").min_size(egui::vec2(80.0, 32.0)),
            )
            .clicked()
        {
            action = Some(ToolbarAction::Start);
        }

        // Pause button
        let pause_enabled = vm_selected && vm_running && !vm_paused;
        let pause_label = if vm_paused { "▶ Resume" } else { "⏸ Pause" };
        if vm_paused {
            if ui
                .add_enabled(
                    vm_selected,
                    egui::Button::new(pause_label).min_size(egui::vec2(80.0, 32.0)),
                )
                .clicked()
            {
                action = Some(ToolbarAction::Pause);
            }
        } else {
            if ui
                .add_enabled(
                    pause_enabled,
                    egui::Button::new(pause_label).min_size(egui::vec2(80.0, 32.0)),
                )
                .clicked()
            {
                action = Some(ToolbarAction::Pause);
            }
        }

        // Stop button
        let stop_enabled = vm_selected && (vm_running || vm_paused);
        if ui
            .add_enabled(
                stop_enabled,
                egui::Button::new("⏹ Stop").min_size(egui::vec2(80.0, 32.0)),
            )
            .clicked()
        {
            action = Some(ToolbarAction::Stop);
        }

        ui.separator();

        // Settings button
        let settings_enabled = vm_selected && !vm_running;
        if ui
            .add_enabled(
                settings_enabled,
                egui::Button::new("⚙ Settings").min_size(egui::vec2(90.0, 32.0)),
            )
            .clicked()
        {
            action = Some(ToolbarAction::Settings);
        }

        // Snapshot button
        if ui
            .add_enabled(
                vm_selected,
                egui::Button::new("📷 Snapshot").min_size(egui::vec2(90.0, 32.0)),
            )
            .clicked()
        {
            action = Some(ToolbarAction::Snapshot);
        }
    });

    action
}
