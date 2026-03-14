use eframe::egui;
use crate::theme;
use crate::dialogs::BUTTON_SIZE;

pub struct AboutDialog {
    pub open: bool,
}

impl AboutDialog {
    pub fn new() -> Self { Self { open: true } }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }
        let mut still_open = self.open;
        let mut button_close = false;

        egui::Window::new("About CoreVM")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(false)
            .min_width(300.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.heading("CoreVM Manager");
                    ui.label(egui::RichText::new("Version 0.1.0").color(egui::Color32::from_rgb(160, 160, 160)));
                    ui.add_space(8.0);
                    ui.label("Cross-platform x86 Virtual Machine Manager");
                    ui.label(egui::RichText::new("Powered by libcorevm").italics());
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("\u{00a9} 2026 CoreVM").color(egui::Color32::from_rgb(120, 120, 120)));
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("OK").fill(theme::ACCENT_BLUE).min_size(BUTTON_SIZE)).clicked() {
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
