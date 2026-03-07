use eframe::egui;
use crate::config::{VmConfig, BootOrder, BiosType, RamAlloc, NetMode, MacMode};

#[derive(PartialEq)]
enum SettingsTab { General, Devices, Boot }

pub struct SettingsDialog {
    config: VmConfig,
    tab: SettingsTab,
    pub open: bool,
    pub saved: bool,
}

impl SettingsDialog {
    pub fn new(config: &VmConfig) -> Self {
        Self {
            config: config.clone(),
            tab: SettingsTab::General,
            open: true,
            saved: false,
        }
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }

    /// Show the settings window. Returns true while window is open.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }

        let mut still_open = self.open;

        egui::Window::new("VM Settings")
            .open(&mut still_open)
            .resizable(true)
            .default_size([500.0, 450.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, SettingsTab::General, "General");
                    ui.selectable_value(&mut self.tab, SettingsTab::Devices, "Devices");
                    ui.selectable_value(&mut self.tab, SettingsTab::Boot, "Boot");
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    match self.tab {
                        SettingsTab::General => self.general_tab(ui),
                        SettingsTab::Devices => self.devices_tab(ui),
                        SettingsTab::Boot => self.boot_tab(ui),
                    }
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.saved = true;
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

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.config.name);
        });

        ui.horizontal(|ui| {
            ui.label("RAM (MB):");
            let mut ram = self.config.ram_mb as f32;
            ui.add(egui::Slider::new(&mut ram, 16.0..=8192.0).step_by(16.0));
            self.config.ram_mb = ram as u32;
        });

        ui.horizontal(|ui| {
            ui.label("CPU Cores:");
            let cores = [1u32, 2, 4, 8, 16];
            for &c in &cores {
                if ui.selectable_label(self.config.cpu_cores == c, format!("{}", c)).clicked() {
                    self.config.cpu_cores = c;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("RAM Allocation:");
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::OnDemand, "On Demand");
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::Preallocate, "Preallocate");
        });

        ui.horizontal(|ui| {
            ui.label("BIOS:");
            ui.radio_value(&mut self.config.bios_type, BiosType::SeaBios, "SeaBIOS");
            ui.radio_value(&mut self.config.bios_type, BiosType::CoreVm, "CoreVM");
        });

        ui.checkbox(&mut self.config.jit_enabled, "Enable JIT");
    }

    fn devices_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("GPU:");
            ui.label("SVGA Framebuffer");
        });

        ui.separator();

        ui.checkbox(&mut self.config.net_enabled, "Enable Network Adapter");

        if self.config.net_enabled {
            ui.indent("net_settings", |ui| {
                ui.label("Intel E1000 Network Adapter");
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    ui.radio_value(&mut self.config.net_mode, NetMode::Nat, "NAT");
                    ui.radio_value(&mut self.config.net_mode, NetMode::Bridge, "Bridge");
                });

                if self.config.net_mode == NetMode::Bridge {
                    ui.horizontal(|ui| {
                        ui.label("Host NIC:");
                        ui.text_edit_singleline(&mut self.config.net_host_nic);
                    });
                }

                ui.horizontal(|ui| {
                    ui.label("MAC:");
                    ui.radio_value(&mut self.config.mac_mode, MacMode::Dynamic, "Dynamic");
                    ui.radio_value(&mut self.config.mac_mode, MacMode::Static, "Static");
                });

                if self.config.mac_mode == MacMode::Static {
                    ui.horizontal(|ui| {
                        ui.label("MAC Address:");
                        ui.text_edit_singleline(&mut self.config.mac_address);
                    });
                }
            });
        }
    }

    fn boot_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Boot Order:");
            ui.radio_value(&mut self.config.boot_order, BootOrder::DiskFirst, "Disk First");
            ui.radio_value(&mut self.config.boot_order, BootOrder::CdFirst, "CD First");
            ui.radio_value(&mut self.config.boot_order, BootOrder::FloppyFirst, "Floppy First");
        });

        ui.horizontal(|ui| {
            ui.label("Disk Image:");
            ui.text_edit_singleline(&mut self.config.disk_image);
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select Disk Image")
                    .pick_file()
                {
                    self.config.disk_image = path.to_string_lossy().to_string();
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("ISO Image:");
            ui.text_edit_singleline(&mut self.config.iso_image);
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select ISO Image")
                    .add_filter("ISO", &["iso"])
                    .pick_file()
                {
                    self.config.iso_image = path.to_string_lossy().to_string();
                }
            }
        });
    }
}
