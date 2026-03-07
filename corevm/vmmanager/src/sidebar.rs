use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VmState {
    Stopped,
    Running,
    Paused,
}

pub struct FolderEntry {
    pub name: String,
    pub vm_uuids: Vec<String>,
    pub expanded: bool,
}

pub struct SidebarLayout {
    pub folders: Vec<FolderEntry>,
    pub root_vms: Vec<String>,
}

impl Default for SidebarLayout {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            root_vms: Vec::new(),
        }
    }
}

impl SidebarLayout {
    pub fn load(path: &Path) -> Self {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        let mut folders = Vec::new();
        let mut root_vms = Vec::new();
        let mut current_folder: Option<FolderEntry> = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix("folder:") {
                if let Some(f) = current_folder.take() {
                    folders.push(f);
                }
                current_folder = Some(FolderEntry {
                    name: name.to_string(),
                    vm_uuids: Vec::new(),
                    expanded: true,
                });
            } else if let Some(uuid) = line.strip_prefix("vm:") {
                if let Some(ref mut f) = current_folder {
                    f.vm_uuids.push(uuid.to_string());
                } else {
                    root_vms.push(uuid.to_string());
                }
            } else if line == "end" {
                if let Some(f) = current_folder.take() {
                    folders.push(f);
                }
            }
        }
        if let Some(f) = current_folder.take() {
            folders.push(f);
        }

        Self { folders, root_vms }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::fmt::Write as _;
        let mut out = String::new();
        for folder in &self.folders {
            let _ = writeln!(out, "folder:{}", folder.name);
            for uuid in &folder.vm_uuids {
                let _ = writeln!(out, "vm:{}", uuid);
            }
            let _ = writeln!(out, "end");
        }
        for uuid in &self.root_vms {
            let _ = writeln!(out, "vm:{}", uuid);
        }
        std::fs::write(path, out)
    }
}

fn state_color(state: VmState) -> Color32 {
    match state {
        VmState::Running => Color32::from_rgb(0, 200, 0),
        VmState::Paused => Color32::from_rgb(255, 165, 0),
        VmState::Stopped => Color32::from_rgb(128, 128, 128),
    }
}

fn render_vm_entry(
    ui: &mut egui::Ui,
    uuid: &str,
    vm_names: &HashMap<String, String>,
    vm_states: &HashMap<String, VmState>,
    selected: &mut Option<String>,
) {
    let name = vm_names
        .get(uuid)
        .map(|s| s.as_str())
        .unwrap_or("Unknown VM");
    let state = vm_states.get(uuid).copied().unwrap_or(VmState::Stopped);
    let color = state_color(state);
    let is_selected = selected.as_deref() == Some(uuid);

    let response = ui.horizontal(|ui| {
        ui.colored_label(color, "\u{25CF}");
        ui.selectable_value(selected, Some(uuid.to_string()), name)
    });

    if response.inner.clicked() {
        *selected = Some(uuid.to_string());
    }
}

pub fn render_sidebar(
    ctx: &egui::Context,
    layout: &mut SidebarLayout,
    vm_names: &HashMap<String, String>,
    vm_states: &HashMap<String, VmState>,
    selected: &mut Option<String>,
) {
    let sidebar_bg = Color32::from_rgb(30, 30, 30);

    egui::SidePanel::left("sidebar")
        .exact_width(220.0)
        .frame(
            egui::Frame::new()
                .fill(sidebar_bg)
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;

            ui.label(
                egui::RichText::new("Virtual Machines")
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            for folder in &mut layout.folders {
                let header =
                    egui::CollapsingHeader::new(egui::RichText::new(format!("\u{1F4C1} {}", folder.name)).color(Color32::from_rgb(200, 200, 200)))
                        .default_open(folder.expanded)
                        .show(ui, |ui| {
                            for uuid in &folder.vm_uuids {
                                render_vm_entry(ui, uuid, vm_names, vm_states, selected);
                            }
                        });
                folder.expanded = header.fully_open();
            }

            for uuid in &layout.root_vms.clone() {
                render_vm_entry(ui, uuid, vm_names, vm_states, selected);
            }
        });
}
