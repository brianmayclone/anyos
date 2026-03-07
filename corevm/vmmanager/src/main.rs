use eframe::egui;

mod app;
mod config;
mod dialogs;
mod display;
mod input;
mod platform;
mod sidebar;
mod statusbar;
mod theme;
mod settings;
mod toolbar;
mod vm;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("CoreVM Manager"),
        ..Default::default()
    };
    eframe::run_native(
        "CoreVM Manager",
        options,
        Box::new(|_cc| Ok(Box::new(app::CoreVmApp::new()))),
    )
}
