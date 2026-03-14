use eframe::egui;
use crate::app::FilePickTarget;
use crate::config::{VmConfig, BootOrder, BiosType, RamAlloc, NetMode, MacMode, GuestOs, GuestArch};
use crate::theme;

const LABEL_WIDTH: f32 = 110.0;
const FIELD_MIN_WIDTH: f32 = 250.0;
const BUTTON_SIZE: egui::Vec2 = egui::vec2(80.0, 28.0);
const SIDEBAR_WIDTH: f32 = 140.0;

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

fn host_info_bar(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(35, 40, 48))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(theme::ACCENT_BLUE, egui::RichText::new("\u{2139}").size(14.0)); // ℹ
                ui.add_space(4.0);
                ui.colored_label(
                    egui::Color32::from_rgb(160, 170, 185),
                    egui::RichText::new(text).size(12.0),
                );
            });
        });
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong().color(theme::ACCENT_BLUE));
    ui.separator();
    ui.add_space(2.0);
}

// ─── Settings categories ─────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Category {
    General,
    Processor,
    Memory,
    HardDisks,
    CdDvd,
    Network,
    Sound,
    Usb,
    Diagnostics,
    Expert,
}

impl Category {
    const ALL: &'static [Category] = &[
        Category::General,
        Category::Processor,
        Category::Memory,
        Category::HardDisks,
        Category::CdDvd,
        Category::Network,
        Category::Sound,
        Category::Usb,
        Category::Diagnostics,
        Category::Expert,
    ];

    fn label(&self) -> &'static str {
        match self {
            Category::General     => "General",
            Category::Processor   => "Processor",
            Category::Memory      => "Memory",
            Category::HardDisks   => "Hard Disks",
            Category::CdDvd       => "CD/DVD",
            Category::Network     => "Network",
            Category::Sound       => "Sound",
            Category::Usb         => "USB",
            Category::Diagnostics => "Diagnostics",
            Category::Expert      => "Expert",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Category::General     => "\u{2699}",  // ⚙
            Category::Processor   => "\u{2318}",  // ⌘
            Category::Memory      => "\u{25A6}",  // ▦
            Category::HardDisks   => "\u{1F4BE}", // 💾
            Category::CdDvd       => "\u{1F4BF}", // 💿
            Category::Network     => "\u{1F310}", // 🌐
            Category::Sound       => "\u{1F50A}", // 🔊
            Category::Usb         => "\u{1F50C}", // 🔌
            Category::Diagnostics => "\u{1F41B}", // 🐛
            Category::Expert      => "\u{1F527}", // 🔧
        }
    }
}

// ─── Settings Dialog ─────────────────────────────────────────────────────

pub struct SettingsDialog {
    config: VmConfig,
    category: Category,
    pub open: bool,
    pub saved: bool,
}

impl SettingsDialog {
    pub fn new(config: &VmConfig) -> Self {
        Self {
            config: config.clone(),
            category: Category::General,
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

    pub fn add_disk_image(&mut self, path: String) {
        self.config.disk_images.push(path);
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

        let max_h = (ctx.screen_rect().height() - 40.0).max(300.0);

        egui::Window::new(format!("{} - Settings", self.config.name))
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .min_width(650.0)
            .min_height(350.0)
            .max_height(max_h)
            .default_size([700.0, max_h.min(500.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let avail_h = ui.available_height() - 46.0; // room for button bar

                ui.horizontal(|ui| {
                    // ── Left sidebar (category list) ──
                    ui.allocate_ui_with_layout(
                        egui::vec2(SIDEBAR_WIDTH, avail_h.max(200.0)),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            self.render_sidebar(ui);
                        },
                    );

                    ui.separator();

                    // ── Right content pane ──
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .max_height(avail_h.max(200.0))
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui.set_min_width(400.0);
                                match self.category {
                                    Category::General     => self.page_general(ui),
                                    Category::Processor   => self.page_processor(ui),
                                    Category::Memory      => self.page_memory(ui),
                                    Category::HardDisks   => Self::page_hard_disks(&mut self.config, ui, &mut browse_target),
                                    Category::CdDvd       => Self::page_cd_dvd(&mut self.config, ui, &mut browse_target),
                                    Category::Network     => self.page_network(ui),
                                    Category::Sound       => self.page_sound(ui),
                                    Category::Usb         => Self::page_placeholder(ui, "USB", "USB controller emulation is not yet implemented."),
                                    Category::Diagnostics => self.page_diagnostics(ui),
                                    Category::Expert      => self.page_expert(ui),
                                }
                                ui.add_space(8.0);
                            });
                    });
                });

                ui.separator();

                // ── Button bar ──
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

    // ── Sidebar ──────────────────────────────────────────────────────────

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        let sidebar_bg = egui::Color32::from_rgb(30, 30, 32);
        let selected_bg = egui::Color32::from_rgb(50, 50, 55);
        let hover_bg = egui::Color32::from_rgb(42, 42, 46);
        let text_normal = egui::Color32::from_rgb(180, 180, 185);
        let text_selected = egui::Color32::from_rgb(240, 240, 245);
        let text_hover = egui::Color32::from_rgb(220, 220, 225);
        let icon_color = theme::ACCENT_BLUE;

        egui::Frame::new()
            .fill(sidebar_bg)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.set_min_width(SIDEBAR_WIDTH - 8.0);

                // Pre-check hover state by reserving rects first
                for &cat in Category::ALL {
                    let is_selected = self.category == cat;

                    // Allocate rect to check hover before drawing
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(SIDEBAR_WIDTH - 12.0, 26.0),
                        egui::Sense::click(),
                    );

                    let is_hovered = resp.hovered();

                    // Determine colors based on state
                    let bg = if is_selected {
                        selected_bg
                    } else if is_hovered {
                        hover_bg
                    } else {
                        sidebar_bg
                    };

                    let text_color = if is_selected {
                        text_selected
                    } else if is_hovered {
                        text_hover
                    } else {
                        text_normal
                    };

                    // Draw background
                    ui.painter().rect_filled(rect, 4.0, bg);

                    // Draw accent bar on selected
                    if is_selected {
                        let bar = egui::Rect::from_min_size(
                            rect.left_top() + egui::vec2(0.0, 2.0),
                            egui::vec2(3.0, rect.height() - 4.0),
                        );
                        ui.painter().rect_filled(bar, 1.0, icon_color);
                    }

                    // Draw text
                    let text = format!(" {}  {}", cat.icon(), cat.label());
                    let galley = ui.painter().layout_no_wrap(
                        text,
                        egui::FontId::proportional(13.0),
                        text_color,
                    );
                    let text_pos = egui::pos2(
                        rect.left() + 6.0,
                        rect.center().y - galley.size().y * 0.5,
                    );
                    ui.painter().galley(text_pos, galley, text_color);

                    if resp.clicked() {
                        self.category = cat;
                    }
                }
            });
    }

    // ── Pages ────────────────────────────────────────────────────────────

    fn page_general(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "General");

        labeled_row(ui, "Name:", |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.config.name).desired_width(FIELD_MIN_WIDTH));
        });

        section_heading(ui, "Guest Operating System");

        labeled_row(ui, "OS:", |ui| {
            egui::ComboBox::from_id_salt("guest_os")
                .width(FIELD_MIN_WIDTH)
                .selected_text(self.config.guest_os.label())
                .show_ui(ui, |ui| {
                    let mut current_cat = "";
                    for os in GuestOs::ALL {
                        let cat = os.category();
                        if cat != current_cat {
                            if !current_cat.is_empty() {
                                ui.separator();
                            }
                            ui.colored_label(
                                egui::Color32::from_rgb(120, 120, 130),
                                egui::RichText::new(cat).strong().small(),
                            );
                            current_cat = cat;
                        }
                        ui.selectable_value(&mut self.config.guest_os, os.clone(), os.label());
                    }
                });
        });

        labeled_row(ui, "Architecture:", |ui| {
            ui.radio_value(&mut self.config.guest_arch, GuestArch::X64, "64-bit (x86_64)");
            ui.radio_value(&mut self.config.guest_arch, GuestArch::X86, "32-bit (x86)");
        });

        section_heading(ui, "Boot");

        labeled_row(ui, "Boot Order:", |ui| {
            ui.radio_value(&mut self.config.boot_order, BootOrder::DiskFirst, "Disk");
            ui.radio_value(&mut self.config.boot_order, BootOrder::CdFirst, "CD");
            ui.radio_value(&mut self.config.boot_order, BootOrder::FloppyFirst, "Floppy");
        });
    }

    fn page_processor(&mut self, ui: &mut egui::Ui) {
        let host = crate::platform::host_info();

        section_heading(ui, "Processor");

        // Host info bar
        host_info_bar(ui, &format!(
            "Host: {} cores available",
            host.cpu_cores,
        ));

        ui.add_space(6.0);

        labeled_row(ui, "CPU Cores:", |ui| {
            let mut cores = self.config.cpu_cores as f32;
            let max = (host.cpu_cores as f32).max(2.0);
            ui.add(egui::Slider::new(&mut cores, 1.0..=max).step_by(1.0).clamp_to_range(true));
            self.config.cpu_cores = (cores as u32).max(1);
        });

        // Recommendation
        ui.add_space(4.0);
        let recommended = (host.cpu_cores / 2).max(1);
        let warn = self.config.cpu_cores > host.cpu_cores;
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + 8.0);
            if warn {
                ui.colored_label(
                    theme::WARNING_ORANGE,
                    format!("Exceeds host cores ({})! Performance may suffer.", host.cpu_cores),
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(110, 110, 115),
                    format!("Recommended: {} cores (half of host)", recommended),
                );
            }
        });
    }

    fn page_memory(&mut self, ui: &mut egui::Ui) {
        let host = crate::platform::host_info();
        let host_ram = host.ram_total_mb;
        let recommended_max = (host_ram * 3 / 4) as u32; // 75% of host
        let slider_max = (host_ram as f32).max(1024.0);

        section_heading(ui, "Memory");

        // Host info bar
        host_info_bar(ui, &format!(
            "Host: {:.1} GB total  |  Recommended max: {:.1} GB (75%)",
            host_ram as f64 / 1024.0,
            recommended_max as f64 / 1024.0,
        ));

        ui.add_space(6.0);

        labeled_row(ui, "RAM:", |ui| {
            let mut ram = self.config.ram_mb as f32;
            ui.add(egui::Slider::new(&mut ram, 16.0..=slider_max).step_by(16.0).suffix(" MB"));
            self.config.ram_mb = ram as u32;
        });

        // Human-readable + warning
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + 8.0);
            let gb = self.config.ram_mb as f64 / 1024.0;
            let pct = (self.config.ram_mb as f64 / host_ram as f64 * 100.0) as u32;

            if gb >= 1.0 {
                ui.colored_label(
                    egui::Color32::from_rgb(180, 180, 185),
                    format!("{:.1} GB", gb),
                );
                ui.colored_label(
                    egui::Color32::from_rgb(100, 100, 105),
                    format!("({}% of host)", pct),
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(180, 180, 185),
                    format!("{} MB", self.config.ram_mb),
                );
            }
        });

        if self.config.ram_mb > recommended_max {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + 8.0);
                ui.colored_label(
                    theme::WARNING_ORANGE,
                    format!("Exceeds recommended maximum ({:.1} GB). Host may become unstable.",
                        recommended_max as f64 / 1024.0),
                );
            });
        }

        // Quick presets
        ui.add_space(8.0);
        labeled_row(ui, "Presets:", |ui| {
            for &mb in &[256u32, 512, 1024, 2048, 4096, 8192, 16384] {
                if mb as u64 > host_ram { break; }
                let label = if mb >= 1024 {
                    format!("{} GB", mb / 1024)
                } else {
                    format!("{} MB", mb)
                };
                if ui.selectable_label(self.config.ram_mb == mb, &label).clicked() {
                    self.config.ram_mb = mb;
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        labeled_row(ui, "Allocation:", |ui| {
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::OnDemand, "On Demand");
            ui.radio_value(&mut self.config.ram_alloc, RamAlloc::Preallocate, "Preallocate");
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + 8.0);
            ui.colored_label(
                egui::Color32::from_rgb(100, 100, 105),
                if self.config.ram_alloc == RamAlloc::OnDemand {
                    "Memory is allocated as the guest uses it."
                } else {
                    "All memory is reserved immediately at VM start."
                },
            );
        });
    }

    fn page_hard_disks(config: &mut VmConfig, ui: &mut egui::Ui, browse_target: &mut Option<FilePickTarget>) {
        section_heading(ui, "Hard Disks");

        let max_disks = 5;
        let mut remove_idx: Option<usize> = None;

        if config.disk_images.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(
                egui::Color32::from_rgb(120, 120, 125),
                "No hard disks attached to this virtual machine.",
            );
            ui.add_space(8.0);
        } else {
            let card_bg = egui::Color32::from_rgb(38, 38, 40);
            let card_border = egui::Color32::from_rgb(55, 55, 58);

            for (i, disk) in config.disk_images.iter().enumerate() {
                let filename = std::path::Path::new(disk)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| disk.clone());

                let size_str = std::path::Path::new(disk)
                    .metadata()
                    .map(|m| format_disk_size(m.len()))
                    .unwrap_or_else(|_| "?".into());

                egui::Frame::new()
                    .fill(card_bg)
                    .stroke(egui::Stroke::new(0.5, card_border))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Disk icon + name
                            ui.colored_label(theme::ACCENT_BLUE, egui::RichText::new("\u{1F4BE}").size(18.0));
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&filename)
                                    .color(egui::Color32::from_rgb(210, 210, 215))
                                    .strong());
                                ui.horizontal(|ui| {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(120, 120, 125),
                                        egui::RichText::new(format!("AHCI Port {} | {}", if i == 0 { 0 } else { i + 1 }, size_str)).small(),
                                    );
                                });
                                ui.colored_label(
                                    egui::Color32::from_rgb(100, 100, 105),
                                    egui::RichText::new(disk).small(),
                                );
                            });

                            // Remove button (right-aligned)
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Remove").clicked() {
                                    remove_idx = Some(i);
                                }
                            });
                        });
                    });

                ui.add_space(4.0);
            }
        }

        if let Some(idx) = remove_idx {
            config.disk_images.remove(idx);
        }

        ui.add_space(4.0);
        let can_add = config.disk_images.len() < max_disks;
        ui.horizontal(|ui| {
            if ui.add_enabled(can_add, egui::Button::new("Add Disk...")).clicked() {
                *browse_target = Some(FilePickTarget::AddDisk);
            }
            if !can_add {
                ui.colored_label(
                    egui::Color32::from_rgb(120, 120, 125),
                    format!("Maximum of {} disks reached.", max_disks),
                );
            }
        });
    }

    fn page_cd_dvd(config: &mut VmConfig, ui: &mut egui::Ui, browse_target: &mut Option<FilePickTarget>) {
        section_heading(ui, "CD/DVD Drive");

        labeled_row(ui, "ISO Image:", |ui| {
            ui.add(egui::TextEdit::singleline(&mut config.iso_image).desired_width(FIELD_MIN_WIDTH));
            if ui.button("Browse...").clicked() {
                *browse_target = Some(FilePickTarget::SettingsIso);
            }
        });

        if !config.iso_image.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + 8.0);
                if ui.small_button("Eject").clicked() {
                    config.iso_image.clear();
                }
            });
        }
    }

    fn page_network(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Network Adapter");

        ui.checkbox(&mut self.config.net_enabled, "Enable Network Adapter");

        if self.config.net_enabled {
            ui.add_space(4.0);

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

            ui.add_space(4.0);

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

    fn page_sound(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Audio");

        ui.checkbox(&mut self.config.audio_enabled, "Enable Audio Controller");

        if self.config.audio_enabled {
            ui.add_space(4.0);
            labeled_row(ui, "Controller:", |ui| {
                ui.label(egui::RichText::new("Intel AC'97 (ICH)").color(egui::Color32::from_rgb(160, 160, 160)));
            });
        }

        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(120, 120, 125),
            "Emulates an Intel 82801AA AC'97 audio controller.\nDisable for server guests or if audio causes issues.",
        );
    }

    fn page_diagnostics(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Diagnostics");

        ui.checkbox(&mut self.config.diagnostics, "Enable Diagnostics Window");

        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(120, 120, 125),
            "When enabled, a diagnostics window will open alongside the VM\nshowing I/O ports, MMIO, interrupts, and CPU state.",
        );
    }

    fn page_expert(&mut self, ui: &mut egui::Ui) {
        // Warning banner
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(60, 40, 20))
            .stroke(egui::Stroke::new(1.0, theme::WARNING_ORANGE))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(theme::WARNING_ORANGE, egui::RichText::new("\u{26A0}").size(18.0));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.colored_label(
                            theme::WARNING_ORANGE,
                            egui::RichText::new("Expert Settings").strong(),
                        );
                        ui.colored_label(
                            egui::Color32::from_rgb(210, 180, 130),
                            "Changing these settings can cause VM instability.\n\
                             For production use, keep the recommended defaults (SeaBIOS).",
                        );
                    });
                });
            });

        ui.add_space(8.0);
        section_heading(ui, "Firmware");

        labeled_row(ui, "BIOS:", |ui| {
            ui.radio_value(&mut self.config.bios_type, BiosType::SeaBios, "SeaBIOS");
            ui.radio_value(&mut self.config.bios_type, BiosType::CoreVm, "CoreVM");
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + 8.0);
            ui.colored_label(
                egui::Color32::from_rgb(100, 100, 105),
                match self.config.bios_type {
                    BiosType::SeaBios => "Industry-standard BIOS. Recommended for most guests.",
                    BiosType::CoreVm  => "Experimental CoreVM BIOS. For development use only.",
                },
            );
        });
    }

    fn page_placeholder(ui: &mut egui::Ui, title: &str, message: &str) {
        section_heading(ui, title);
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(130, 130, 135),
                message,
            );
        });
    }
}

fn format_disk_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{} B", bytes)
    }
}
