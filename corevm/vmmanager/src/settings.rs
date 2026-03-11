use eframe::egui;
use crate::app::FilePickTarget;
use crate::config::{VmConfig, BootOrder, BiosType, RamAlloc, NetMode, MacMode};
use crate::theme;

const LABEL_WIDTH: f32 = 110.0;
const FIELD_MIN_WIDTH: f32 = 250.0;
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

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong().color(theme::ACCENT_BLUE));
    ui.separator();
    ui.add_space(2.0);
}

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

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_disk_image(&mut self, path: String) {
        self.config.disk_image = path;
    }

    pub fn set_iso_image(&mut self, path: String) {
        self.config.iso_image = path;
    }

    /// Show the settings window. Returns Some(FilePickTarget) if Browse was clicked.
    pub fn show_with_browse(&mut self, ctx: &egui::Context) -> Option<FilePickTarget> {
        if !self.open { return None; }

        let mut still_open = self.open;
        let mut button_close = false;
        let mut browse_target: Option<FilePickTarget> = None;

        let max_h = (ctx.screen_rect().height() - 40.0).max(200.0);

        egui::Window::new("VM Settings")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .min_width(520.0)
            .max_height(max_h)
            .default_size([540.0, max_h.min(420.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                // Tab bar
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, SettingsTab::General, egui::RichText::new("  General  "));
                    ui.selectable_value(&mut self.tab, SettingsTab::Devices, egui::RichText::new("  Devices  "));
                    ui.selectable_value(&mut self.tab, SettingsTab::Boot, egui::RichText::new("  Boot  "));
                });
                ui.separator();

                // Content — limit scroll height so buttons stay visible
                let scroll_h = (ui.available_height() - 50.0).max(100.0);
                egui::ScrollArea::vertical().max_height(scroll_h).auto_shrink([false; 2]).show(ui, |ui| {
                    match self.tab {
                        SettingsTab::General => self.general_tab(ui),
                        SettingsTab::Devices => self.devices_tab(ui),
                        SettingsTab::Boot => Self::boot_tab_static(&mut self.config, ui, &mut browse_target),
                    }
                    ui.add_space(4.0);
                });

                ui.separator();

                // Buttons
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Cancel").min_size(BUTTON_SIZE)).clicked() {
                            button_close = true;
                        }
                        if ui.add(egui::Button::new("Save").fill(theme::ACCENT_BLUE).min_size(BUTTON_SIZE)).clicked() {
                            self.saved = true;
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
        browse_target
    }

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Machine");

        labeled_row(ui, "Name:", |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.config.name).desired_width(FIELD_MIN_WIDTH));
        });

        labeled_row(ui, "RAM:", |ui| {
            let mut ram = self.config.ram_mb as f32;
            ui.add(egui::Slider::new(&mut ram, 16.0..=8192.0).step_by(16.0).suffix(" MB"));
            self.config.ram_mb = ram as u32;
        });

        labeled_row(ui, "CPU Cores:", |ui| {
            for &c in &[1u32, 2, 4, 8, 16] {
                if ui.selectable_label(self.config.cpu_cores == c, format!("{}", c)).clicked() {
                    self.config.cpu_cores = c;
                }
            }
        });

        labeled_row(ui, "RAM Alloc:", |ui| {
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::OnDemand, "On Demand");
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::Preallocate, "Preallocate");
        });

        section_heading(ui, "Firmware");

        labeled_row(ui, "BIOS:", |ui| {
            ui.radio_value(&mut self.config.bios_type, BiosType::SeaBios, "SeaBIOS");
            ui.radio_value(&mut self.config.bios_type, BiosType::CoreVm, "CoreVM");
        });

        section_heading(ui, "Debugging");

        labeled_row(ui, "", |ui| {
            ui.checkbox(&mut self.config.diagnostics, "Enable Diagnostics Window");
        });
    }

    fn devices_tab(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Display");

        labeled_row(ui, "GPU:", |ui| {
            ui.label("SVGA Framebuffer");
        });

        section_heading(ui, "Network");

        labeled_row(ui, "", |ui| {
            ui.checkbox(&mut self.config.net_enabled, "Enable Network Adapter");
        });

        if self.config.net_enabled {
            labeled_row(ui, "Adapter:", |ui| {
                ui.label(egui::RichText::new("Intel E1000").color(egui::Color32::from_rgb(160, 160, 160)));
            });

            labeled_row(ui, "Mode:", |ui| {
                ui.radio_value(&mut self.config.net_mode, NetMode::Nat, "NAT");
                ui.radio_value(&mut self.config.net_mode, NetMode::Bridge, "Bridge");
            });

            if self.config.net_mode == NetMode::Bridge {
                labeled_row(ui, "Host NIC:", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.config.net_host_nic).desired_width(FIELD_MIN_WIDTH));
                });
            }

            labeled_row(ui, "MAC:", |ui| {
                ui.radio_value(&mut self.config.mac_mode, MacMode::Dynamic, "Dynamic");
                ui.radio_value(&mut self.config.mac_mode, MacMode::Static, "Static");
            });

            if self.config.mac_mode == MacMode::Static {
                labeled_row(ui, "MAC Address:", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.config.mac_address)
                        .desired_width(FIELD_MIN_WIDTH)
                        .hint_text("00:11:22:33:44:55"));
                });
            }
        }
    }

    fn boot_tab_static(config: &mut VmConfig, ui: &mut egui::Ui, browse_target: &mut Option<FilePickTarget>) {
        section_heading(ui, "Boot Order");

        labeled_row(ui, "Priority:", |ui| {
            ui.radio_value(&mut config.boot_order, BootOrder::DiskFirst, "Disk First");
            ui.radio_value(&mut config.boot_order, BootOrder::CdFirst, "CD First");
            ui.radio_value(&mut config.boot_order, BootOrder::FloppyFirst, "Floppy First");
        });

        section_heading(ui, "Storage");

        labeled_row(ui, "Disk Image:", |ui| {
            ui.add(egui::TextEdit::singleline(&mut config.disk_image).desired_width(FIELD_MIN_WIDTH));
            if ui.button("Browse...").clicked() {
                *browse_target = Some(FilePickTarget::SettingsDisk);
            }
        });

        labeled_row(ui, "ISO Image:", |ui| {
            ui.add(egui::TextEdit::singleline(&mut config.iso_image).desired_width(FIELD_MIN_WIDTH));
            if ui.button("Browse...").clicked() {
                *browse_target = Some(FilePickTarget::SettingsIso);
            }
        });
    }
}
