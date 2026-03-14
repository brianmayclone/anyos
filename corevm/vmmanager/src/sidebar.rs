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

/// Actions returned from the sidebar to the app
pub enum SidebarAction {
    /// A VM is being dragged to a folder (vm_uuid, target_folder_index or None for root)
    MoveVm { vm_uuid: String, target_folder: Option<usize> },
    /// Request to create a new VM
    CreateVm,
    /// Request to create a new folder
    CreateFolder,
    /// Request to rename a folder
    RenameFolder(usize),
    /// Request to delete a folder (VMs move to root)
    DeleteFolder(usize),
    /// Request to delete a VM
    DeleteVm(String),
}

pub struct SidebarLayout {
    pub folders: Vec<FolderEntry>,
    pub root_vms: Vec<String>,
}

impl Default for SidebarLayout {
    fn default() -> Self {
        Self {
            folders: vec![FolderEntry {
                name: "My Machines".to_string(),
                vm_uuids: Vec::new(),
                expanded: true,
            }],
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

        // Ensure at least one default folder
        if folders.is_empty() {
            folders.push(FolderEntry {
                name: "My Machines".to_string(),
                vm_uuids: Vec::new(),
                expanded: true,
            });
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

    /// Ensure a newly created VM is placed in the first folder
    pub fn add_vm(&mut self, uuid: String) {
        if !self.folders.is_empty() {
            self.folders[0].vm_uuids.push(uuid);
        } else {
            self.root_vms.push(uuid);
        }
    }

    /// Move a VM from wherever it is to a target folder (or root if None)
    pub fn move_vm(&mut self, uuid: &str, target_folder: Option<usize>) {
        // Remove from all folders and root
        for folder in &mut self.folders {
            folder.vm_uuids.retain(|u| u != uuid);
        }
        self.root_vms.retain(|u| u != uuid);

        // Add to target
        match target_folder {
            Some(idx) if idx < self.folders.len() => {
                self.folders[idx].vm_uuids.push(uuid.to_string());
            }
            _ => {
                self.root_vms.push(uuid.to_string());
            }
        }
    }

    /// Remove a VM from the layout entirely
    pub fn remove_vm(&mut self, uuid: &str) {
        for folder in &mut self.folders {
            folder.vm_uuids.retain(|u| u != uuid);
        }
        self.root_vms.retain(|u| u != uuid);
    }

    /// Ensure all known VM UUIDs are in the layout somewhere; orphans go to first folder
    pub fn ensure_all_vms(&mut self, all_uuids: &[String]) {
        let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
        for folder in &self.folders {
            for u in &folder.vm_uuids {
                known.insert(u.clone());
            }
        }
        for u in &self.root_vms {
            known.insert(u.clone());
        }
        let orphans: Vec<String> = all_uuids.iter()
            .filter(|u| !known.contains(u.as_str()))
            .cloned()
            .collect();
        for uuid in orphans {
            if !self.folders.is_empty() {
                self.folders[0].vm_uuids.push(uuid);
            } else {
                self.root_vms.push(uuid);
            }
        }
    }
}

fn state_color(state: VmState) -> Color32 {
    match state {
        VmState::Running => Color32::from_rgb(48, 209, 88),
        VmState::Paused => Color32::from_rgb(255, 159, 10),
        VmState::Stopped => Color32::from_rgb(99, 99, 102),
    }
}

/// Render a single VM entry. Returns a drag source ID if dragging.
fn render_vm_entry(
    ui: &mut egui::Ui,
    uuid: &str,
    vm_names: &HashMap<String, String>,
    vm_states: &HashMap<String, VmState>,
    vm_icons: &HashMap<String, &'static str>,
    selected: &mut Option<String>,
    drag_vm: &mut Option<String>,
    actions: &mut Vec<SidebarAction>,
    folder_names: &[String],
    current_folder: Option<usize>,
) {
    let name = vm_names
        .get(uuid)
        .map(|s| s.as_str())
        .unwrap_or("Unknown VM");
    let state = vm_states.get(uuid).copied().unwrap_or(VmState::Stopped);
    let color = state_color(state);

    let inner_resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // Draw a filled circle as status indicator
        let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        let center = dot_rect.center() + egui::vec2(0.0, 2.0);
        ui.painter().circle_filled(center, 4.0, color);
        let icon = vm_icons.get(uuid).copied().unwrap_or("\u{1F5A5}");
        ui.selectable_value(selected, Some(uuid.to_string()), format!("{} {}", icon, name))
    }).inner;

    if inner_resp.clicked() {
        *selected = Some(uuid.to_string());
    }

    // Drag source
    if inner_resp.dragged() {
        *drag_vm = Some(uuid.to_string());
    }

    // Context menu
    inner_resp.context_menu(|ui| {
        ui.label(egui::RichText::new(name).strong());
        ui.separator();

        // "Move to" submenu
        if folder_names.len() > 1 || current_folder.is_none() {
            ui.menu_button("Move to...", |ui| {
                for (i, fname) in folder_names.iter().enumerate() {
                    if Some(i) == current_folder {
                        continue; // skip current folder
                    }
                    if ui.button(fname).clicked() {
                        actions.push(SidebarAction::MoveVm {
                            vm_uuid: uuid.to_string(),
                            target_folder: Some(i),
                        });
                        ui.close_menu();
                    }
                }
            });
        }

        ui.separator();
        if ui.button("\u{1F5D1} Delete VM").clicked() {
            actions.push(SidebarAction::DeleteVm(uuid.to_string()));
            ui.close_menu();
        }
    });
}

/// Sidebar state for rename dialog
pub struct SidebarState {
    pub rename_folder_idx: Option<usize>,
    pub rename_buffer: String,
    pub show_new_folder: bool,
    pub new_folder_name: String,
    pub confirm_delete_vm: Option<String>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            rename_folder_idx: None,
            rename_buffer: String::new(),
            show_new_folder: false,
            new_folder_name: String::new(),
            confirm_delete_vm: None,
        }
    }
}

pub fn render_sidebar(
    ctx: &egui::Context,
    layout: &mut SidebarLayout,
    vm_names: &HashMap<String, String>,
    vm_states: &HashMap<String, VmState>,
    vm_icons: &HashMap<String, &'static str>,
    selected: &mut Option<String>,
    sidebar_state: &mut SidebarState,
) -> Vec<SidebarAction> {
    let mut actions = Vec::new();
    let mut drag_vm: Option<String> = None;
    let folder_names: Vec<String> = layout.folders.iter().map(|f| f.name.clone()).collect();

    egui::SidePanel::left("sidebar")
        .exact_width(230.0)
        .frame(
            egui::Frame::new()
                .fill(Color32::from_rgb(28, 28, 30))
                .inner_margin(egui::Margin::symmetric(10, 12)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;

            // Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Machines")
                        .size(13.0)
                        .strong()
                        .color(Color32::from_rgb(142, 142, 147)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(
                        egui::Button::new(egui::RichText::new("\u{1F4C1}").size(13.0))
                            .fill(Color32::from_rgb(54, 54, 56))
                            .corner_radius(egui::CornerRadius::same(6))
                            .min_size(egui::vec2(24.0, 24.0)),
                    ).on_hover_text("New folder").clicked() {
                        sidebar_state.show_new_folder = true;
                        sidebar_state.new_folder_name = "New Folder".to_string();
                    }
                    if ui.add(
                        egui::Button::new(egui::RichText::new("\u{1F5A5}+").size(13.0).color(Color32::WHITE))
                            .fill(Color32::from_rgb(10, 132, 255))
                            .corner_radius(egui::CornerRadius::same(6))
                            .min_size(egui::vec2(24.0, 24.0)),
                    ).on_hover_text("New VM").clicked() {
                        actions.push(SidebarAction::CreateVm);
                    }
                });
            });
            ui.add_space(6.0);

            // New folder inline editor
            if sidebar_state.show_new_folder {
                ui.horizontal(|ui| {
                    ui.label("\u{1F4C1}");
                    let te = egui::TextEdit::singleline(&mut sidebar_state.new_folder_name)
                        .desired_width(140.0);
                    let resp = ui.add(te);
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.small_button("\u{2713}").clicked() {
                        let name = sidebar_state.new_folder_name.trim().to_string();
                        if !name.is_empty() {
                            layout.folders.push(FolderEntry {
                                name,
                                vm_uuids: Vec::new(),
                                expanded: true,
                            });
                        }
                        sidebar_state.show_new_folder = false;
                    }
                    if ui.small_button("\u{2717}").clicked() {
                        sidebar_state.show_new_folder = false;
                    }
                });
                ui.add_space(2.0);
            }

            // Rename folder inline editor
            if let Some(rename_idx) = sidebar_state.rename_folder_idx {
                if rename_idx < layout.folders.len() {
                    ui.horizontal(|ui| {
                        ui.label("\u{1F4C1}");
                        let te = egui::TextEdit::singleline(&mut sidebar_state.rename_buffer)
                            .desired_width(140.0);
                        let resp = ui.add(te);
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.small_button("\u{2713}").clicked() {
                            let name = sidebar_state.rename_buffer.trim().to_string();
                            if !name.is_empty() {
                                layout.folders[rename_idx].name = name;
                            }
                            sidebar_state.rename_folder_idx = None;
                        }
                        if ui.small_button("\u{2717}").clicked() {
                            sidebar_state.rename_folder_idx = None;
                        }
                    });
                }
            }

            // Render folders
            let num_folders = layout.folders.len();
            for fi in 0..num_folders {
                let folder_name = layout.folders[fi].name.clone();
                let folder_expanded = layout.folders[fi].expanded;

                let header_text = egui::RichText::new(format!("\u{1F4C1} {}", folder_name))
                    .size(13.0)
                    .color(Color32::from_rgb(210, 210, 215));

                let header = egui::CollapsingHeader::new(header_text)
                    .id_salt(format!("folder_{}", fi))
                    .default_open(folder_expanded)
                    .show(ui, |ui| {
                        let vm_uuids: Vec<String> = layout.folders[fi].vm_uuids.clone();
                        for uuid in &vm_uuids {
                            render_vm_entry(
                                ui, uuid, vm_names, vm_states, vm_icons, selected,
                                &mut drag_vm, &mut actions, &folder_names, Some(fi),
                            );
                        }
                        if vm_uuids.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(72, 72, 74),
                                egui::RichText::new("  No machines").size(12.0).italics(),
                            );
                        }
                    });

                layout.folders[fi].expanded = header.fully_open();

                // Folder context menu on the header
                header.header_response.context_menu(|ui| {
                    ui.label(egui::RichText::new(&folder_name).strong());
                    ui.separator();
                    if ui.button("\u{270F} Rename").clicked() {
                        sidebar_state.rename_folder_idx = Some(fi);
                        sidebar_state.rename_buffer = folder_name.clone();
                        ui.close_menu();
                    }
                    // Don't allow deleting the last folder
                    if num_folders > 1 {
                        if ui.button("\u{1F5D1} Delete Folder").clicked() {
                            actions.push(SidebarAction::DeleteFolder(fi));
                            ui.close_menu();
                        }
                    }
                });

                // Drop target: if dragging a VM over this folder header
                if drag_vm.is_some() {
                    let drop_resp = header.header_response.clone();
                    if drop_resp.hovered() && ui.input(|i| i.pointer.any_released()) {
                        if let Some(vm_uuid) = drag_vm.take() {
                            actions.push(SidebarAction::MoveVm {
                                vm_uuid,
                                target_folder: Some(fi),
                            });
                        }
                    }
                }
            }

            // Root VMs (outside folders)
            if !layout.root_vms.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);
                ui.colored_label(Color32::from_rgb(99, 99, 102), egui::RichText::new("Unsorted").size(12.0));
                let root_uuids = layout.root_vms.clone();
                for uuid in &root_uuids {
                    render_vm_entry(
                        ui, uuid, vm_names, vm_states, vm_icons, selected,
                        &mut drag_vm, &mut actions, &folder_names, None,
                    );
                }
            }
        });

    actions
}
