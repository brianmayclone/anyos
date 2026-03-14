//! Shared dialog helpers and re-exports.

use eframe::egui;
use crate::theme;

pub const LABEL_WIDTH: f32 = 100.0;
pub const FIELD_MIN_WIDTH: f32 = 220.0;
pub const BUTTON_SIZE: egui::Vec2 = egui::vec2(80.0, 28.0);

pub fn labeled_row(ui: &mut egui::Ui, label: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_WIDTH, 20.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| { ui.label(label); },
        );
        add_contents(ui);
    });
}

pub fn button_row(ui: &mut egui::Ui, ok_label: &str) -> (bool, bool) {
    let mut ok = false;
    let mut cancel = false;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("Cancel").min_size(BUTTON_SIZE)).clicked() {
                cancel = true;
            }
            if ui.add(egui::Button::new(ok_label).fill(theme::ACCENT_BLUE).min_size(BUTTON_SIZE)).clicked() {
                ok = true;
            }
        });
    });
    (ok, cancel)
}

// Re-exports from separate dialog files
pub use crate::dialog_create_vm::CreateVmDialog;
pub use crate::dialog_create_disk::CreateDiskDialog;
pub use crate::dialog_add_disk::{AddDiskDialog, AddDiskMode};
pub use crate::dialog_disk_pool::DiskPoolDialog;
pub use crate::dialog_about::AboutDialog;
pub use crate::dialog_snapshots::SnapshotsDialog;
