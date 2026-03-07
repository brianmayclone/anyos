use eframe::egui;
use crate::config::VmConfig;

pub struct CreateVmDialog {
    name: String,
    ram_mb: u32,
    pub open: bool,
    pub created: Option<VmConfig>,
}

impl CreateVmDialog {
    pub fn new() -> Self {
        Self {
            name: "New VM".into(),
            ram_mb: 256,
            open: true,
            created: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }

        let mut still_open = self.open;

        egui::Window::new("Create New VM")
            .open(&mut still_open)
            .resizable(false)
            .default_size([350.0, 200.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.name);
                });

                ui.horizontal(|ui| {
                    ui.label("RAM (MB):");
                    egui::ComboBox::from_id_salt("ram_select")
                        .selected_text(format!("{}", self.ram_mb))
                        .show_ui(ui, |ui| {
                            for &mb in &[64, 128, 256, 512, 1024, 2048, 4096] {
                                ui.selectable_value(&mut self.ram_mb, mb, format!("{} MB", mb));
                            }
                        });
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        let mut config = VmConfig::default();
                        config.name = self.name.clone();
                        config.ram_mb = self.ram_mb;
                        self.created = Some(config);
                        self.open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });

        self.open = still_open;
        self.open
    }
}

pub struct CreateDiskDialog {
    path: String,
    size_mb: u64,
    pub open: bool,
    pub created: bool,
    pub error: Option<String>,
}

impl CreateDiskDialog {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            size_mb: 1024,
            open: true,
            created: false,
            error: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }

        let mut still_open = self.open;

        egui::Window::new("Create Disk Image")
            .open(&mut still_open)
            .resizable(false)
            .default_size([400.0, 200.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    ui.text_edit_singleline(&mut self.path);
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Save Disk Image")
                            .add_filter("Disk Image", &["img", "raw"])
                            .save_file()
                        {
                            self.path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Size:");
                    egui::ComboBox::from_id_salt("disk_size")
                        .selected_text(format!("{} MB", self.size_mb))
                        .show_ui(ui, |ui| {
                            for &mb in &[512u64, 1024, 2048, 4096, 8192, 16384] {
                                ui.selectable_value(&mut self.size_mb, mb, format!("{} MB", mb));
                            }
                        });
                });

                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(244, 67, 54), err);
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        if self.path.is_empty() {
                            self.error = Some("Please specify a path".into());
                        } else {
                            match std::fs::File::create(&self.path) {
                                Ok(file) => {
                                    if let Err(e) = file.set_len(self.size_mb * 1024 * 1024) {
                                        self.error = Some(format!("Failed to set size: {}", e));
                                    } else {
                                        self.created = true;
                                        self.open = false;
                                    }
                                }
                                Err(e) => {
                                    self.error = Some(format!("Failed to create file: {}", e));
                                }
                            }
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });

        self.open = still_open;
        self.open
    }
}

pub struct AboutDialog {
    pub open: bool,
}

impl AboutDialog {
    pub fn new() -> Self { Self { open: true } }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }
        let mut still_open = self.open;
        egui::Window::new("About CoreVM")
            .open(&mut still_open)
            .resizable(false)
            .default_size([300.0, 150.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("CoreVM Manager");
                    ui.label("Version 0.1.0");
                    ui.add_space(10.0);
                    ui.label("Cross-platform x86 Virtual Machine Manager");
                    ui.label("Powered by libcorevm");
                });
            });
        self.open = still_open;
        self.open
    }
}

pub struct SnapshotsDialog {
    pub open: bool,
}

impl SnapshotsDialog {
    pub fn new() -> Self { Self { open: true } }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }
        let mut still_open = self.open;
        egui::Window::new("Snapshots")
            .open(&mut still_open)
            .resizable(true)
            .default_size([400.0, 300.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label("No snapshots yet");
                    ui.add_space(20.0);
                    if ui.add_enabled(false, egui::Button::new("Take Snapshot")).clicked() {}
                    if ui.add_enabled(false, egui::Button::new("Restore")).clicked() {}
                    if ui.add_enabled(false, egui::Button::new("Delete")).clicked() {}
                    ui.add_space(10.0);
                    ui.label("Snapshot support coming soon");
                });
            });
        self.open = still_open;
        self.open
    }
}
