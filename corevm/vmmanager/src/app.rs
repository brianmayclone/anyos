use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;

use crate::config::VmConfig;
use crate::dialogs::{AboutDialog, CreateDiskDialog, CreateVmDialog, SnapshotsDialog};
use crate::display::DisplayWidget;
use crate::filebrowser::FileBrowserDialog;
use crate::input::{self, MouseCapture};
use crate::platform;
use crate::settings::SettingsDialog;
use crate::sidebar::{self, SidebarAction, SidebarLayout, SidebarState, VmState};
use crate::statusbar::{self, VmMetrics};
use crate::theme;
use crate::toolbar::{self, ToolbarAction};
use crate::vm;
use crate::vm::VmControl;

/// Shared framebuffer data between VM thread and UI
pub struct FrameBufferData {
    pub pixels: Vec<u8>,      // RGBA32
    pub width: u32,
    pub height: u32,
    pub text_mode: bool,
    pub text_buffer: Vec<u16>, // 80x25 = 2000 cells
    pub dirty: bool,
}

impl Default for FrameBufferData {
    fn default() -> Self {
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            text_mode: true,
            text_buffer: Vec::new(),
            dirty: false,
        }
    }
}

/// Runtime entry for a VM
pub struct VmEntry {
    pub config: VmConfig,
    pub state: VmState,
    pub vm_handle: Option<u64>,
    pub control: Option<Arc<VmControl>>,
    pub framebuffer: Arc<Mutex<FrameBufferData>>,
    pub vm_thread: Option<JoinHandle<()>>,
    pub instruction_count: u64,
    pub mips: f64,
    pub ipc: f64,
    pub cpu_mode: u32,  // 0=real, 1=protected, 2=long
}

impl VmEntry {
    pub fn new(config: VmConfig) -> Self {
        Self {
            config,
            state: VmState::Stopped,
            vm_handle: None,
            control: None,
            framebuffer: Arc::new(Mutex::new(FrameBufferData::default())),
            vm_thread: None,
            instruction_count: 0,
            mips: 0.0,
            ipc: 0.0,
            cpu_mode: 0,
        }
    }
}

/// Identifies which field a file dialog is picking for
#[derive(Clone, Debug)]
pub enum FilePickTarget {
    SettingsDisk,
    SettingsIso,
    CreateDiskPath,
}

pub struct CoreVmApp {
    pub vms: Vec<VmEntry>,
    pub layout: SidebarLayout,
    pub selected_vm: Option<String>,  // UUID
    pub display: DisplayWidget,
    pub mouse_capture: MouseCapture,
    pub settings_dialog: Option<SettingsDialog>,
    pub create_vm_dialog: Option<CreateVmDialog>,
    pub create_disk_dialog: Option<CreateDiskDialog>,
    pub about_dialog: Option<AboutDialog>,
    pub snapshots_dialog: Option<SnapshotsDialog>,
    pub error_message: Option<String>,
    pub file_browser: Option<FileBrowserDialog>,
    pub file_pick_target: Option<FilePickTarget>,
    pub sidebar_state: SidebarState,
    pub display_focused: bool,
    pub last_key_label: Option<String>,
    pub last_key_time: std::time::Instant,
}

impl CoreVmApp {
    pub fn new() -> Self {
        platform::ensure_dirs();

        let mut layout = SidebarLayout::load(&platform::layout_dir().join("layout.conf"));

        let mut vms = Vec::new();
        if let Ok(entries) = std::fs::read_dir(platform::config_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "conf") {
                    if let Ok(config) = VmConfig::load(&path) {
                        vms.push(VmEntry::new(config));
                    }
                }
            }
        }

        // Ensure all loaded VMs appear in the layout
        let all_uuids: Vec<String> = vms.iter().map(|v| v.config.uuid.clone()).collect();
        layout.ensure_all_vms(&all_uuids);

        Self {
            vms,
            layout,
            selected_vm: None,
            display: DisplayWidget::new(),
            mouse_capture: MouseCapture::default(),
            settings_dialog: None,
            create_vm_dialog: None,
            create_disk_dialog: None,
            about_dialog: None,
            snapshots_dialog: None,
            error_message: None,
            file_browser: None,
            file_pick_target: None,
            sidebar_state: SidebarState::default(),
            display_focused: false,
            last_key_label: None,
            last_key_time: std::time::Instant::now(),
        }
    }

    fn vm_names(&self) -> HashMap<String, String> {
        self.vms.iter().map(|v| (v.config.uuid.clone(), v.config.name.clone())).collect()
    }

    fn vm_states(&self) -> HashMap<String, VmState> {
        self.vms.iter().map(|v| (v.config.uuid.clone(), v.state)).collect()
    }

    pub fn find_vm(&self, uuid: &str) -> Option<&VmEntry> {
        self.vms.iter().find(|v| v.config.uuid == uuid)
    }

    pub fn find_vm_mut(&mut self, uuid: &str) -> Option<&mut VmEntry> {
        self.vms.iter_mut().find(|v| v.config.uuid == uuid)
    }

    fn handle_toolbar_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::Start => {
                if let Some(uuid) = self.selected_vm.clone() {
                    if let Some(entry) = self.find_vm_mut(&uuid) {
                        if let Err(e) = vm::start_vm(entry) {
                            self.error_message = Some(format!("Failed to start VM: {}", e));
                        }
                    }
                }
            }
            ToolbarAction::Stop => {
                if let Some(uuid) = self.selected_vm.clone() {
                    if let Some(entry) = self.find_vm_mut(&uuid) {
                        vm::stop_vm(entry);
                    }
                }
            }
            ToolbarAction::Pause => {
                if let Some(uuid) = self.selected_vm.clone() {
                    if let Some(entry) = self.find_vm_mut(&uuid) {
                        if entry.state == VmState::Running {
                            vm::pause_vm(entry);
                        } else if entry.state == VmState::Paused {
                            vm::resume_vm(entry);
                        }
                    }
                }
            }
            ToolbarAction::Settings => {
                if let Some(uuid) = self.selected_vm.clone() {
                    if let Some(entry) = self.find_vm(&uuid) {
                        self.settings_dialog = Some(SettingsDialog::new(&entry.config));
                    }
                }
            }
            ToolbarAction::Snapshot => {
                self.snapshots_dialog = Some(SnapshotsDialog::new());
            }
        }
    }

    fn selected_metrics(&self) -> Option<VmMetrics> {
        let uuid = self.selected_vm.as_ref()?;
        let vm = self.find_vm(uuid)?;
        if vm.state != VmState::Running { return None; }

        let (mips, total_insn) = if let Some(ref ctl) = vm.control {
            let mips_bits = ctl.mips.load(std::sync::atomic::Ordering::Relaxed);
            let mips = f64::from_bits(mips_bits);
            let total = ctl.total_instructions.load(std::sync::atomic::Ordering::Relaxed);
            (mips, total)
        } else {
            (0.0, 0)
        };

        Some(VmMetrics {
            state_label: "Running",
            mips,
            ipc: 0.0,
            cpu_mode: "N/A",
            jit_blocks: 0,
            jit_hit_rate: 0.0,
        })
    }
}

impl eframe::App for CoreVmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply_theme(ctx);

        // Intercept keyboard events BEFORE egui widgets consume Enter/Tab/etc.
        // Use display_focused from the previous frame (updated at end of this frame).
        if self.display_focused {
            if let Some(uuid) = &self.selected_vm {
                if let Some(vm) = self.vms.iter().find(|v| &v.config.uuid == uuid) {
                    if let Some(handle) = vm.vm_handle {
                        if let Some(label) = input::handle_keyboard_events(ctx, handle, true) {
                            self.last_key_label = Some(label);
                            self.last_key_time = std::time::Instant::now();
                        }
                    }
                }
            }
        }

        // Expire last key display after 5 seconds
        if self.last_key_label.is_some() && self.last_key_time.elapsed().as_secs() >= 5 {
            self.last_key_label = None;
        }

        let mut deferred_action: Option<ToolbarAction> = None;

        // Menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New VM...").clicked() {
                        self.create_vm_dialog = Some(CreateVmDialog::new());
                        ui.close_menu();
                    }
                    if ui.button("Create Disk...").clicked() {
                        self.create_disk_dialog = Some(CreateDiskDialog::new());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open Config Directory").clicked() {
                        let dir = platform::config_dir();
                        #[cfg(target_os = "linux")]
                        { let _ = std::process::Command::new("xdg-open").arg(&dir).spawn(); }
                        #[cfg(target_os = "windows")]
                        { let _ = std::process::Command::new("explorer").arg(&dir).spawn(); }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("VM", |ui| {
                    let has_sel = self.selected_vm.is_some();
                    let is_running = self.selected_vm.as_ref()
                        .and_then(|u| self.find_vm(u))
                        .map_or(false, |v| v.state == VmState::Running);
                    let is_paused = self.selected_vm.as_ref()
                        .and_then(|u| self.find_vm(u))
                        .map_or(false, |v| v.state == VmState::Paused);
                    let is_stopped = self.selected_vm.as_ref()
                        .and_then(|u| self.find_vm(u))
                        .map_or(true, |v| v.state == VmState::Stopped);

                    if ui.add_enabled(has_sel && is_stopped, egui::Button::new("Start")).clicked() {
                        deferred_action = Some(ToolbarAction::Start);
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel && (is_running || is_paused), egui::Button::new("Pause / Resume")).clicked() {
                        deferred_action = Some(ToolbarAction::Pause);
                        ui.close_menu();
                    }
                    if ui.add_enabled(has_sel && !is_stopped, egui::Button::new("Stop")).clicked() {
                        deferred_action = Some(ToolbarAction::Stop);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(has_sel && is_stopped, egui::Button::new("Settings...")).clicked() {
                        deferred_action = Some(ToolbarAction::Settings);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About CoreVM...").clicked() {
                        self.about_dialog = Some(AboutDialog::new());
                        ui.close_menu();
                    }
                });
            });
        });

        // Status bar
        let metrics = self.selected_metrics();
        statusbar::render_statusbar(ctx, metrics.as_ref(), self.selected_vm.is_some(), self.last_key_label.as_deref());

        // Sidebar
        let names = self.vm_names();
        let states = self.vm_states();
        let sidebar_actions = sidebar::render_sidebar(
            ctx, &mut self.layout, &names, &states,
            &mut self.selected_vm, &mut self.sidebar_state,
        );

        // Handle sidebar actions
        for action in sidebar_actions {
            match action {
                SidebarAction::MoveVm { vm_uuid, target_folder } => {
                    self.layout.move_vm(&vm_uuid, target_folder);
                    let _ = self.layout.save(&platform::layout_dir().join("layout.conf"));
                }
                SidebarAction::CreateVm => {
                    self.create_vm_dialog = Some(CreateVmDialog::new());
                }
                SidebarAction::CreateFolder => {
                    // Handled inline in sidebar
                }
                SidebarAction::RenameFolder(_) => {
                    // Handled inline in sidebar
                }
                SidebarAction::DeleteFolder(idx) => {
                    if idx < self.layout.folders.len() {
                        let orphans: Vec<String> = self.layout.folders[idx].vm_uuids.drain(..).collect();
                        self.layout.folders.remove(idx);
                        // Move orphaned VMs to first folder
                        if !self.layout.folders.is_empty() {
                            self.layout.folders[0].vm_uuids.extend(orphans);
                        } else {
                            self.layout.root_vms.extend(orphans);
                        }
                        let _ = self.layout.save(&platform::layout_dir().join("layout.conf"));
                    }
                }
                SidebarAction::DeleteVm(uuid) => {
                    // Only allow deleting stopped VMs
                    let is_stopped = self.find_vm(&uuid)
                        .map_or(true, |v| v.state == VmState::Stopped);
                    if is_stopped {
                        self.layout.remove_vm(&uuid);
                        // Remove config file
                        let config_path = platform::config_dir().join(format!("{}.conf", uuid));
                        let _ = std::fs::remove_file(&config_path);
                        self.vms.retain(|v| v.config.uuid != uuid);
                        if self.selected_vm.as_deref() == Some(&uuid) {
                            self.selected_vm = None;
                        }
                        let _ = self.layout.save(&platform::layout_dir().join("layout.conf"));
                    } else {
                        self.error_message = Some("Cannot delete a running VM. Stop it first.".into());
                    }
                }
            }
        }

        // Central panel
        egui::CentralPanel::default().show(ctx, |ui| {
            let (vm_selected, vm_running, vm_paused) = if let Some(uuid) = &self.selected_vm {
                if let Some(vm) = self.find_vm(uuid) {
                    (true, vm.state == VmState::Running, vm.state == VmState::Paused)
                } else {
                    (false, false, false)
                }
            } else {
                (false, false, false)
            };

            if let Some(action) = toolbar::render_toolbar(ui, vm_selected, vm_running, vm_paused) {
                deferred_action = Some(action);
            }
            ui.separator();

            if let Some(uuid) = &self.selected_vm.clone() {
                // Extract state and data from vm without holding borrow on self
                let vm_info = self.find_vm(uuid).map(|vm| {
                    (vm.state, vm.framebuffer.clone())
                });
                if let Some((state, fb)) = vm_info {
                    if state == VmState::Running || state == VmState::Paused {
                        let display_focused = if let Ok(fb_data) = fb.lock() {
                            self.display.show(ui, ctx, &fb_data)
                        } else {
                            false
                        };
                        self.display_focused = display_focused;
                    } else {
                        self.display_focused = false;
                        if let Some(vm) = self.find_vm(uuid) {
                            render_summary(ui, vm, &mut deferred_action);
                        }
                    }
                }
            } else {
                self.display_focused = false;
                ui.centered_and_justified(|ui| {
                    ui.heading("Select or create a VM to get started");
                });
            }
        });

        // Process deferred action
        if let Some(action) = deferred_action {
            self.handle_toolbar_action(action);
        }

        // ── File browser dialog ──
        let mut file_picked: Option<String> = None;
        if let Some(ref mut browser) = self.file_browser {
            if !browser.show(ctx) {
                file_picked = browser.picked.take();
            }
        }
        if let Some(path) = file_picked {
            match &self.file_pick_target {
                Some(FilePickTarget::SettingsDisk) => {
                    if let Some(ref mut dlg) = self.settings_dialog {
                        dlg.set_disk_image(path);
                    }
                }
                Some(FilePickTarget::SettingsIso) => {
                    if let Some(ref mut dlg) = self.settings_dialog {
                        dlg.set_iso_image(path);
                    }
                }
                Some(FilePickTarget::CreateDiskPath) => {
                    if let Some(ref mut dlg) = self.create_disk_dialog {
                        dlg.set_path(path);
                    }
                }
                None => {}
            }
            self.file_pick_target = None;
            self.file_browser = None;
        }
        // Clean up closed browser
        if self.file_browser.as_ref().map_or(false, |b| !b.open) {
            self.file_browser = None;
            self.file_pick_target = None;
        }

        // ── Dialogs ──

        // Settings dialog
        let mut browse_target: Option<FilePickTarget> = None;
        if let Some(ref mut dialog) = self.settings_dialog {
            if let Some(target) = dialog.show_with_browse(ctx) {
                browse_target = Some(target);
            }
            if !dialog.is_open() {
                if dialog.saved {
                    let config = dialog.config().clone();
                    if let Some(uuid) = &self.selected_vm.clone() {
                        if let Some(entry) = self.find_vm_mut(uuid) {
                            entry.config = config.clone();
                            let _ = config.save(&platform::config_dir());
                        }
                    }
                }
                self.settings_dialog = None;
            }
        }
        if let Some(target) = browse_target {
            self.file_pick_target = Some(target.clone());
            match &target {
                FilePickTarget::SettingsDisk => {
                    self.file_browser = Some(FileBrowserDialog::new_open("Select Disk Image", &["img", "raw", "qcow2"]));
                }
                FilePickTarget::SettingsIso => {
                    self.file_browser = Some(FileBrowserDialog::new_open("Select ISO Image", &["iso"]));
                }
                _ => {}
            }
        }

        // Create VM dialog
        if let Some(ref mut dialog) = self.create_vm_dialog {
            if !dialog.show(ctx) {
                if let Some(config) = dialog.created.take() {
                    let uuid = config.uuid.clone();
                    let _ = config.save(&platform::config_dir());
                    self.layout.add_vm(uuid.clone());
                    let _ = self.layout.save(&platform::layout_dir().join("layout.conf"));
                    self.vms.push(VmEntry::new(config));
                    self.selected_vm = Some(uuid);
                }
                self.create_vm_dialog = None;
            }
        }

        // Create Disk dialog
        let mut disk_browse = false;
        if let Some(ref mut dialog) = self.create_disk_dialog {
            if dialog.show_with_browse(ctx) {
                disk_browse = true;
            }
            if !dialog.is_open() {
                self.create_disk_dialog = None;
            }
        }
        if disk_browse {
            self.file_pick_target = Some(FilePickTarget::CreateDiskPath);
            self.file_browser = Some(FileBrowserDialog::new_save("Save Disk Image", &["img", "raw"]));
        }

        // About dialog
        if let Some(ref mut dialog) = self.about_dialog {
            if !dialog.show(ctx) {
                self.about_dialog = None;
            }
        }

        // Snapshots dialog
        if let Some(ref mut dialog) = self.snapshots_dialog {
            if !dialog.show(ctx) {
                self.snapshots_dialog = None;
            }
        }

        // Error dialog
        if let Some(msg) = self.error_message.clone() {
            let mut dismiss = false;
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .min_width(350.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.screen_rect().center())
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⚠").size(20.0).color(theme::ERROR_RED));
                        ui.add_space(4.0);
                        ui.label(&msg);
                    });
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("OK").fill(theme::ACCENT_BLUE).min_size(egui::vec2(80.0, 28.0))).clicked() {
                                dismiss = true;
                            }
                        });
                    });
                });
            if dismiss {
                self.error_message = None;
            }
        }

        // Check if any running VM thread has exited
        for vm in &mut self.vms {
            if vm.state == VmState::Running {
                if let Some(ref ctl) = vm.control {
                    if ctl.exited.load(std::sync::atomic::Ordering::Relaxed) {
                        let reason = ctl.exit_reason.load(std::sync::atomic::Ordering::Relaxed);
                        vm.state = VmState::Stopped;
                        self.error_message = Some(format!(
                            "VM '{}' stopped unexpectedly (exit reason: {})",
                            vm.config.name, reason
                        ));
                    }
                }
            }
        }

        // Repaint when VM running
        if self.vms.iter().any(|v| v.state == VmState::Running) {
            ctx.request_repaint();
        }
    }
}

fn render_summary(ui: &mut egui::Ui, vm: &VmEntry, deferred_action: &mut Option<ToolbarAction>) {
    let available = ui.available_size();

    // --- Dark "screen" placeholder (VMware-style) ---
    let screen_aspect = 4.0 / 3.0;
    let max_screen_h = (available.y - 120.0).max(200.0);
    let max_screen_w = (available.x - 40.0).max(300.0);
    let (screen_w, screen_h) = if max_screen_w / max_screen_h > screen_aspect {
        (max_screen_h * screen_aspect, max_screen_h)
    } else {
        (max_screen_w, max_screen_w / screen_aspect)
    };

    ui.vertical_centered(|ui| {
        ui.add_space(10.0);

        // Dark screen rectangle
        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(screen_w, screen_h),
            egui::Sense::hover(),
        );

        let painter = ui.painter_at(rect);

        // Screen background — dark gradient
        painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 22));

        // Subtle border
        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 65)), egui::StrokeKind::Outside);

        // VM name centered in screen
        painter.text(
            rect.center() - egui::vec2(0.0, 30.0),
            egui::Align2::CENTER_CENTER,
            &vm.config.name,
            egui::FontId::proportional(24.0),
            egui::Color32::from_rgb(120, 120, 130),
        );

        // State label
        let (state_label, state_color) = match vm.state {
            VmState::Running => ("Running", egui::Color32::from_rgb(76, 175, 80)),
            VmState::Paused => ("Paused", egui::Color32::from_rgb(255, 165, 0)),
            VmState::Stopped => ("Powered Off", egui::Color32::from_rgb(100, 100, 110)),
        };
        painter.text(
            rect.center() + egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            state_label,
            egui::FontId::proportional(14.0),
            state_color,
        );

        // Small power icon hint
        if vm.state == VmState::Stopped {
            painter.text(
                rect.center() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Click ▶ Start to power on this virtual machine",
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(80, 80, 90),
            );
        }

        ui.add_space(8.0);

        // --- Info bar below screen ---
        ui.horizontal(|ui| {
            let info_style = egui::Color32::from_rgb(170, 170, 180);
            let dim_style = egui::Color32::from_rgb(100, 100, 110);

            ui.colored_label(dim_style, "RAM:");
            ui.colored_label(info_style, format!("{} MB", vm.config.ram_mb));
            ui.add_space(12.0);

            ui.colored_label(dim_style, "CPUs:");
            ui.colored_label(info_style, format!("{}", vm.config.cpu_cores));
            ui.add_space(12.0);

            ui.colored_label(dim_style, "BIOS:");
            ui.colored_label(info_style, format!("{:?}", vm.config.bios_type));
            ui.add_space(12.0);

            if vm.config.jit_enabled {
                ui.colored_label(info_style, "JIT");
                ui.add_space(12.0);
            }

            if !vm.config.disk_image.is_empty() {
                ui.colored_label(dim_style, "Disk:");
                let disk_name = std::path::Path::new(&vm.config.disk_image)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| vm.config.disk_image.clone());
                ui.colored_label(info_style, disk_name);
                ui.add_space(12.0);
            }

            if !vm.config.iso_image.is_empty() {
                ui.colored_label(dim_style, "ISO:");
                let iso_name = std::path::Path::new(&vm.config.iso_image)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| vm.config.iso_image.clone());
                ui.colored_label(info_style, iso_name);
            }
        });

        ui.add_space(8.0);

        // Start button
        if vm.state == VmState::Stopped {
            if ui.add(
                egui::Button::new(
                    egui::RichText::new("▶  Power On").size(16.0).color(egui::Color32::WHITE),
                )
                .fill(theme::ACCENT_BLUE)
                .min_size(egui::vec2(160.0, 40.0)),
            )
            .clicked()
            {
                *deferred_action = Some(ToolbarAction::Start);
            }
        }
    });
}
