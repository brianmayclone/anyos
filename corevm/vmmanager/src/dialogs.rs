use eframe::egui;
use crate::config::VmConfig;
use crate::theme;

const LABEL_WIDTH: f32 = 100.0;
const FIELD_MIN_WIDTH: f32 = 220.0;
const BUTTON_SIZE: egui::Vec2 = egui::vec2(80.0, 28.0);

fn labeled_row(ui: &mut egui::Ui, label: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_WIDTH, 20.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| { ui.label(label); },
        );
        add_contents(ui);
    });
}

fn button_row(ui: &mut egui::Ui, ok_label: &str) -> (bool, bool) {
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

// ─── Create VM Dialog ─────────────────────────────────────────────────────

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
        let mut button_close = false;

        egui::Window::new("Create New Virtual Machine")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(false)
            .min_width(400.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                labeled_row(ui, "Name:", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(FIELD_MIN_WIDTH));
                });

                labeled_row(ui, "RAM:", |ui| {
                    egui::ComboBox::from_id_salt("create_vm_ram")
                        .width(FIELD_MIN_WIDTH)
                        .selected_text(format!("{} MB", self.ram_mb))
                        .show_ui(ui, |ui| {
                            for &mb in &[64, 128, 256, 512, 1024, 2048, 4096] {
                                ui.selectable_value(&mut self.ram_mb, mb, format!("{} MB", mb));
                            }
                        });
                });

                ui.separator();

                let (ok, cancel) = button_row(ui, "Create");
                if ok {
                    let mut config = VmConfig::default();
                    config.name = self.name.clone();
                    config.ram_mb = self.ram_mb;
                    self.created = Some(config);
                    button_close = true;
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
        self.open
    }
}

// ─── Create Disk Dialog ───────────────────────────────────────────────────

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

    pub fn set_path(&mut self, path: String) {
        self.path = path;
    }

    pub fn is_open(&self) -> bool {
        self.open
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
                    egui::ComboBox::from_id_salt("create_disk_size")
                        .width(FIELD_MIN_WIDTH)
                        .selected_text(if self.size_mb >= 1024 {
                            format!("{} GB", self.size_mb / 1024)
                        } else {
                            format!("{} MB", self.size_mb)
                        })
                        .show_ui(ui, |ui| {
                            for &mb in &[256u64, 512, 1024, 2048, 4096, 8192, 16384, 32768] {
                                let label = if mb >= 1024 {
                                    format!("{} GB", mb / 1024)
                                } else {
                                    format!("{} MB", mb)
                                };
                                ui.selectable_value(&mut self.size_mb, mb, label);
                            }
                        });
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
                        match std::fs::File::create(&self.path) {
                            Ok(file) => {
                                if let Err(e) = file.set_len(self.size_mb * 1024 * 1024) {
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

// ─── About Dialog ─────────────────────────────────────────────────────────

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
                    ui.label(egui::RichText::new("© 2026 CoreVM").color(egui::Color32::from_rgb(120, 120, 120)));
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

// ─── Snapshots Dialog ─────────────────────────────────────────────────────

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
