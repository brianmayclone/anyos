use eframe::egui;
use crate::theme;
use crate::dialogs::{labeled_row, button_row, FIELD_MIN_WIDTH};

#[derive(PartialEq, Clone, Copy)]
pub enum AddDiskMode {
    LoadExisting,
    ImportVmdk,
    CreateNew,
}

pub struct AddDiskDialog {
    mode: AddDiskMode,
    path: String,
    size_mb: u64,
    custom_size: String,
    custom_unit: SizeUnit,
    use_custom_size: bool,
    pub open: bool,
    pub result_path: Option<String>,
    pub error: Option<String>,
    importing: bool,
}

#[derive(PartialEq, Clone, Copy)]
enum SizeUnit { MB, GB, TB }

impl AddDiskDialog {
    pub fn new() -> Self {
        Self {
            mode: AddDiskMode::LoadExisting,
            path: String::new(),
            size_mb: 8192,
            custom_size: String::new(),
            custom_unit: SizeUnit::GB,
            use_custom_size: false,
            open: true,
            result_path: None,
            error: None,
            importing: false,
        }
    }

    pub fn set_path(&mut self, path: String) {
        self.path = path;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Show the dialog. Returns Some(AddDiskMode) if Browse was clicked.
    pub fn show_with_browse(&mut self, ctx: &egui::Context) -> Option<AddDiskMode> {
        if !self.open { return None; }

        let mut still_open = self.open;
        let mut button_close = false;
        let mut browse: Option<AddDiskMode> = None;

        egui::Window::new("Add Disk")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(false)
            .min_width(500.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    ui.radio_value(&mut self.mode, AddDiskMode::LoadExisting, "Load Existing");
                    ui.radio_value(&mut self.mode, AddDiskMode::ImportVmdk, "Import VMDK");
                    ui.radio_value(&mut self.mode, AddDiskMode::CreateNew, "Create New");
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                match self.mode {
                    AddDiskMode::LoadExisting => {
                        labeled_row(ui, "Image:", |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(FIELD_MIN_WIDTH));
                            if ui.button("Browse...").clicked() {
                                browse = Some(AddDiskMode::LoadExisting);
                            }
                        });
                        ui.add_space(2.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(130, 130, 135),
                            "Select an existing .img or .raw disk image file.",
                        );
                    }
                    AddDiskMode::ImportVmdk => {
                        labeled_row(ui, "VMDK File:", |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(FIELD_MIN_WIDTH));
                            if ui.button("Browse...").clicked() {
                                browse = Some(AddDiskMode::ImportVmdk);
                            }
                        });
                        ui.add_space(2.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(130, 130, 135),
                            "The VMDK will be converted to a raw .img in the disk pool.",
                        );
                        if self.importing {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Importing...");
                            });
                        }
                    }
                    AddDiskMode::CreateNew => {
                        labeled_row(ui, "Path:", |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(FIELD_MIN_WIDTH));
                            if ui.button("Browse...").clicked() {
                                browse = Some(AddDiskMode::CreateNew);
                            }
                        });

                        labeled_row(ui, "Size:", |ui| {
                            if !self.use_custom_size {
                                egui::ComboBox::from_id_salt("add_disk_size")
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
                    }
                }

                if let Some(err) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(theme::ERROR_RED, err);
                }

                ui.add_space(4.0);
                ui.separator();

                let ok_label = match self.mode {
                    AddDiskMode::LoadExisting => "Add",
                    AddDiskMode::ImportVmdk => "Import",
                    AddDiskMode::CreateNew => "Create",
                };
                let (ok, cancel) = button_row(ui, ok_label);
                if ok && !self.importing {
                    self.execute();
                    if self.result_path.is_some() {
                        button_close = true;
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

    fn execute(&mut self) {
        self.error = None;
        if self.path.is_empty() {
            self.error = Some("Please specify a file path.".into());
            return;
        }

        match self.mode {
            AddDiskMode::LoadExisting => {
                let p = std::path::Path::new(&self.path);
                if !p.exists() {
                    self.error = Some("File does not exist.".into());
                } else {
                    self.result_path = Some(self.path.clone());
                }
            }
            AddDiskMode::ImportVmdk => {
                let src = std::path::Path::new(&self.path);
                if !src.exists() {
                    self.error = Some("VMDK file does not exist.".into());
                    return;
                }
                let stem = src.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "imported".into());
                let pool = crate::platform::disk_pool_dir();
                let dest = pool.join(format!("{}.img", stem));

                match std::fs::copy(src, &dest) {
                    Ok(bytes) => {
                        eprintln!("[vmmanager] Imported VMDK: {} -> {} ({} bytes)", self.path, dest.display(), bytes);
                        self.result_path = Some(dest.to_string_lossy().to_string());
                    }
                    Err(e) => {
                        self.error = Some(format!("Import failed: {}", e));
                    }
                }
            }
            AddDiskMode::CreateNew => {
                let actual_mb = if self.use_custom_size {
                    match self.custom_size.trim().parse::<u64>() {
                        Ok(val) if val > 0 => match self.custom_unit {
                            SizeUnit::MB => val,
                            SizeUnit::GB => val * 1024,
                            SizeUnit::TB => val * 1024 * 1024,
                        },
                        _ => {
                            self.error = Some("Please enter a valid number.".into());
                            return;
                        }
                    }
                } else {
                    self.size_mb
                };
                match std::fs::File::create(&self.path) {
                    Ok(file) => {
                        if let Err(e) = file.set_len(actual_mb * 1024 * 1024) {
                            self.error = Some(format!("Failed to set size: {}", e));
                        } else {
                            self.result_path = Some(self.path.clone());
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

fn format_size(mb: u64) -> String {
    if mb >= 1024 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{} MB", mb)
    }
}
