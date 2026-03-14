use eframe::egui;
use crate::dialogs::BUTTON_SIZE;

pub struct SnapshotsDialog {
    pub open: bool,
}

impl SnapshotsDialog {
    pub fn new() -> Self { Self { open: true } }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }
        let mut still_open = self.open;
        let mut button_close = false;

        egui::Window::new("Snapshots")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .min_width(350.0)
            .default_size([400.0, 250.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No snapshots yet").color(egui::Color32::from_rgb(160, 160, 160)));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Snapshot support coming soon.").italics().color(egui::Color32::from_rgb(120, 120, 120)));
                });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Close").min_size(BUTTON_SIZE)).clicked() {
                            button_close = true;
                        }
                    });
                });
            });

        if button_close {
            self.open = false;
        } else {
            self.open = still_open;
        }
        self.open
    }
}
