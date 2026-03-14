use eframe::egui;
use crate::theme;
use crate::dialogs::BUTTON_SIZE;

pub struct DiskInfo {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub used_by: Vec<String>,
}

pub struct DiskPoolDialog {
    pub open: bool,
    disks: Vec<DiskInfo>,
}

impl DiskPoolDialog {
    pub fn new(vm_configs: &[crate::config::VmConfig]) -> Self {
        let mut dlg = Self {
            open: true,
            disks: Vec::new(),
        };
        dlg.scan(vm_configs);
        dlg
    }

    fn scan(&mut self, vm_configs: &[crate::config::VmConfig]) {
        self.disks.clear();
        let pool_dir = crate::platform::disk_pool_dir();

        // Collect all disk paths referenced by VMs
        let mut vm_disk_usage: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for cfg in vm_configs {
            for disk in &cfg.disk_images {
                if !disk.is_empty() {
                    vm_disk_usage.entry(disk.clone())
                        .or_default()
                        .push(cfg.name.clone());
                }
            }
        }

        // Scan pool directory
        if let Ok(entries) = std::fs::read_dir(&pool_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if matches!(ext.as_str(), "img" | "raw" | "qcow2") {
                    let path_str = path.to_string_lossy().to_string();
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let used_by = vm_disk_usage.get(&path_str)
                        .cloned()
                        .unwrap_or_default();
                    self.disks.push(DiskInfo {
                        filename: path.file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path: path_str,
                        size_bytes: size,
                        used_by,
                    });
                }
            }
        }

        // Also add disks referenced by VMs but not in pool dir
        for (disk_path, vms) in &vm_disk_usage {
            if !self.disks.iter().any(|d| &d.path == disk_path) {
                let p = std::path::Path::new(disk_path);
                let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                self.disks.push(DiskInfo {
                    filename: p.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| disk_path.clone()),
                    path: disk_path.clone(),
                    size_bytes: size,
                    used_by: vms.clone(),
                });
            }
        }

        self.disks.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()));
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }
        let mut still_open = self.open;
        let mut button_close = false;

        let max_h = (ctx.screen_rect().height() - 40.0).max(200.0);

        egui::Window::new("Disk Pool")
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .min_width(550.0)
            .max_height(max_h)
            .default_size([600.0, max_h.min(400.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Pool directory:").color(egui::Color32::from_rgb(130, 130, 135)));
                    ui.label(egui::RichText::new(
                        crate::platform::disk_pool_dir().to_string_lossy().to_string()
                    ).monospace().color(egui::Color32::from_rgb(180, 180, 180)));
                });
                ui.separator();

                if self.disks.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No disk images found.")
                            .color(egui::Color32::from_rgb(130, 130, 135)));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Create a new disk or import one via File > Create Disk...")
                            .italics()
                            .color(egui::Color32::from_rgb(100, 100, 105)));
                    });
                    ui.add_space(20.0);
                } else {
                    let scroll_h = (ui.available_height() - 50.0).max(100.0);
                    egui::ScrollArea::vertical().max_height(scroll_h).show(ui, |ui| {
                        let label_color = egui::Color32::from_rgb(142, 142, 147);
                        let value_color = egui::Color32::from_rgb(210, 210, 215);

                        for (i, disk) in self.disks.iter().enumerate() {
                            if i > 0 {
                                ui.separator();
                            }
                            ui.horizontal(|ui| {
                                ui.colored_label(value_color, egui::RichText::new(&disk.filename).strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.colored_label(label_color, format_disk_size(disk.size_bytes));
                                });
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.colored_label(label_color, egui::RichText::new(&disk.path).small());
                            });
                            if !disk.used_by.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    ui.colored_label(
                                        theme::SUCCESS_GREEN,
                                        egui::RichText::new(format!("Used by: {}", disk.used_by.join(", "))).small(),
                                    );
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    ui.colored_label(
                                        egui::Color32::from_rgb(100, 100, 105),
                                        egui::RichText::new("Not attached to any VM").small().italics(),
                                    );
                                });
                            }
                        }
                    });
                }

                ui.separator();
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Close").min_size(BUTTON_SIZE)).clicked() {
                            button_close = true;
                        }
                        if ui.add(egui::Button::new("Open Directory").min_size(BUTTON_SIZE)).clicked() {
                            let dir = crate::platform::disk_pool_dir();
                            #[cfg(target_os = "linux")]
                            { let _ = std::process::Command::new("xdg-open").arg(&dir).spawn(); }
                            #[cfg(target_os = "windows")]
                            { let _ = std::process::Command::new("explorer").arg(&dir).spawn(); }
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

fn format_disk_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}
