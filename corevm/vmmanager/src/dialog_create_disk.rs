use eframe::egui;
use crate::theme;
use crate::dialogs::{labeled_row, button_row, FIELD_MIN_WIDTH};

#[derive(PartialEq, Clone, Copy)]
enum SizeUnit { MB, GB, TB }

pub struct CreateDiskDialog {
    path: String,
    size_mb: u64,
    custom_size: String,
    custom_unit: SizeUnit,
    use_custom_size: bool,
    pub open: bool,
    pub created: bool,
    pub error: Option<String>,
}

impl CreateDiskDialog {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            size_mb: 1024,
            custom_size: String::new(),
            custom_unit: SizeUnit::GB,
            use_custom_size: false,
            open: true,
            created: false,
            error: None,
        }
    }

    pub fn set_path(&mut self, path: String) {
        self.path = path;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn resolve_size_mb(&self) -> Result<u64, String> {
        if self.use_custom_size {
            match self.custom_size.trim().parse::<u64>() {
                Ok(val) if val > 0 => Ok(match self.custom_unit {
                    SizeUnit::MB => val,
                    SizeUnit::GB => val * 1024,
                    SizeUnit::TB => val * 1024 * 1024,
                }),
                _ => Err("Please enter a valid number.".into()),
            }
        } else {
            Ok(self.size_mb)
        }
    }

    /// Returns true if Browse was clicked
    pub fn show_with_browse(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }

        let mut still_open = self.open;
        let mut button_close = false;
        let mut browse = false;

        egui::Window::new("Create Disk Image")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(false)
            .min_width(450.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                labeled_row(ui, "Path:", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(FIELD_MIN_WIDTH));
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });

                labeled_row(ui, "Size:", |ui| {
                    if !self.use_custom_size {
                        egui::ComboBox::from_id_salt("create_disk_size")
                            .width(150.0)
                            .selected_text(format_size(self.size_mb))
                            .show_ui(ui, |ui| {
                                for &mb in &[256u64, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072] {
                                    ui.selectable_value(&mut self.size_mb, mb, format_size(mb));
                                }
                            });
                    } else {
                        ui.add(egui::TextEdit::singleline(&mut self.custom_size)
                            .desired_width(80.0)
                            .hint_text("e.g. 50"));
                        ui.radio_value(&mut self.custom_unit, SizeUnit::MB, "MB");
                        ui.radio_value(&mut self.custom_unit, SizeUnit::GB, "GB");
                        ui.radio_value(&mut self.custom_unit, SizeUnit::TB, "TB");
                    }
                    if ui.small_button(if self.use_custom_size { "Preset" } else { "Custom" }).clicked() {
                        self.use_custom_size = !self.use_custom_size;
                    }
                });

                if let Some(err) = &self.error {
                    ui.colored_label(theme::ERROR_RED, err);
                }

                ui.separator();

                let (ok, cancel) = button_row(ui, "Create");
                if ok {
                    if self.path.is_empty() {
                        self.error = Some("Please specify a file path.".into());
                    } else {
                        match self.resolve_size_mb() {
                            Err(e) => { self.error = Some(e); }
                            Ok(actual_mb) => {
                                match std::fs::File::create(&self.path) {
                                    Ok(file) => {
                                        if let Err(e) = file.set_len(actual_mb * 1024 * 1024) {
                                            self.error = Some(format!("Failed to set size: {}", e));
                                        } else {
                                            self.created = true;
                                            button_close = true;
                                        }
                                    }
                                    Err(e) => {
                                        self.error = Some(format!("Failed to create file: {}", e));
                                    }
                                }
                            }
                        }
                    }
                }
                if cancel {
                    button_close = true;
                }
            });

        if button_close {
            self.open = false;
        } else {
            self.open = still_open;
        }
        browse
    }
}

fn format_size(mb: u64) -> String {
    if mb >= 1024 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{} MB", mb)
    }
}
