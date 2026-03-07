use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;

use crate::config::VmConfig;
use crate::dialogs::{AboutDialog, CreateDiskDialog, CreateVmDialog, SnapshotsDialog};
use crate::display::DisplayWidget;
use crate::input::{self, MouseCapture};
use crate::platform;
use crate::settings::SettingsDialog;
use crate::sidebar::{self, SidebarLayout, VmState};
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
}

impl CoreVmApp {
    pub fn new() -> Self {
        platform::ensure_dirs();

        let layout = SidebarLayout::load(&platform::layout_dir().join("layout.conf"));

        // Load all VM configs from config dir
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
        }
    }

    /// Get a map of uuid -> name for sidebar
    fn vm_names(&self) -> HashMap<String, String> {
        self.vms.iter().map(|v| (v.config.uuid.clone(), v.config.name.clone())).collect()
    }

    /// Get a map of uuid -> state for sidebar
    fn vm_states(&self) -> HashMap<String, VmState> {
        self.vms.iter().map(|v| (v.config.uuid.clone(), v.state)).collect()
    }

    /// Find VM entry by UUID
    pub fn find_vm(&self, uuid: &str) -> Option<&VmEntry> {
        self.vms.iter().find(|v| v.config.uuid == uuid)
    }

    pub fn find_vm_mut(&mut self, uuid: &str) -> Option<&mut VmEntry> {
        self.vms.iter_mut().find(|v| v.config.uuid == uuid)
    }

    /// Handle toolbar action
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

    /// Get metrics for selected VM
    fn selected_metrics(&self) -> Option<VmMetrics> {
        let uuid = self.selected_vm.as_ref()?;
        let vm = self.find_vm(uuid)?;
        if vm.state != VmState::Running {
            return None;
        }
        Some(VmMetrics {
            state_label: "Running",
            mips: vm.mips,
            ipc: vm.ipc,
            cpu_mode: match vm.cpu_mode {
                0 => "Real Mode",
                1 => "Protected Mode",
                2 => "Long Mode",
                _ => "Unknown",
            },
            jit_blocks: 0,
            jit_hit_rate: 0.0,
        })
    }
}

impl eframe::App for CoreVmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply_theme(ctx);

        // Collect deferred actions to avoid borrow conflicts
        let mut deferred_action: Option<ToolbarAction> = None;

        // Menu bar (top-most)
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

        // Status bar (bottom, must be before CentralPanel)
        let metrics = self.selected_metrics();
        statusbar::render_statusbar(ctx, metrics.as_ref(), self.selected_vm.is_some());

        // Sidebar (left)
        let names = self.vm_names();
        let states = self.vm_states();
        sidebar::render_sidebar(ctx, &mut self.layout, &names, &states, &mut self.selected_vm);

        // Toolbar + Central content
        egui::CentralPanel::default().show(ctx, |ui| {
            // Toolbar at top
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

            // Main content area
            if let Some(uuid) = &self.selected_vm.clone() {
                if let Some(vm) = self.find_vm(uuid) {
                    if vm.state == VmState::Running || vm.state == VmState::Paused {
                        // Display framebuffer
                        let fb = vm.framebuffer.clone();
                        let vm_handle = vm.vm_handle;
                        if let Ok(fb_data) = fb.lock() {
                            self.display.show(ui, ctx, &fb_data);
                        }
                        // Handle keyboard input
                        if let Some(handle) = vm_handle {
                            input::handle_keyboard_events(ctx, handle);
                        }
                    } else {
                        // Summary panel
                        render_summary(ui, vm, &mut deferred_action);
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.heading("Select or create a VM to get started");
                });
            }
        });

        // Process deferred action
        if let Some(action) = deferred_action {
            self.handle_toolbar_action(action);
        }

        // Show dialogs

        // Settings dialog
        if let Some(ref mut dialog) = self.settings_dialog {
            if !dialog.show(ctx) {
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

        // Create VM dialog
        if let Some(ref mut dialog) = self.create_vm_dialog {
            if !dialog.show(ctx) {
                if let Some(config) = dialog.created.take() {
                    let uuid = config.uuid.clone();
                    let _ = config.save(&platform::config_dir());
                    self.layout.root_vms.push(uuid.clone());
                    let _ = self.layout.save(&platform::layout_dir().join("layout.conf"));
                    self.vms.push(VmEntry::new(config));
                    self.selected_vm = Some(uuid);
                }
                self.create_vm_dialog = None;
            }
        }

        // Create Disk dialog
        if let Some(ref mut dialog) = self.create_disk_dialog {
            if !dialog.show(ctx) {
                self.create_disk_dialog = None;
            }
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

        // Error toast
        if let Some(msg) = self.error_message.clone() {
            let mut dismiss = false;
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::from_rgb(244, 67, 54), &msg);
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            if dismiss {
                self.error_message = None;
            }
        }

        // Request repaint for live updates when VM is running
        if self.selected_vm.as_ref()
            .and_then(|u| self.find_vm(u))
            .map_or(false, |v| v.state == VmState::Running)
        {
            ctx.request_repaint();
        }
    }
}

/// Summary panel showing VM details with state-colored status
fn render_summary(ui: &mut egui::Ui, vm: &VmEntry, deferred_action: &mut Option<ToolbarAction>) {
    ui.vertical(|ui| {
        ui.add_space(20.0);
        ui.heading(&vm.config.name);
        ui.add_space(10.0);

        let (state_label, state_color) = match vm.state {
            VmState::Running => ("Running", egui::Color32::from_rgb(76, 175, 80)),
            VmState::Paused => ("Paused", egui::Color32::from_rgb(255, 165, 0)),
            VmState::Stopped => ("Stopped", egui::Color32::from_rgb(128, 128, 128)),
        };

        egui::Grid::new("vm_summary")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Status:");
                ui.colored_label(state_color, state_label);
                ui.end_row();

                ui.label("RAM:");
                ui.label(format!("{} MB", vm.config.ram_mb));
                ui.end_row();

                ui.label("CPU Cores:");
                ui.label(format!("{}", vm.config.cpu_cores));
                ui.end_row();

                ui.label("BIOS:");
                ui.label(format!("{:?}", vm.config.bios_type));
                ui.end_row();

                ui.label("JIT:");
                ui.label(if vm.config.jit_enabled { "Enabled" } else { "Disabled" });
                ui.end_row();

                ui.label("Disk:");
                ui.label(if vm.config.disk_image.is_empty() { "None" } else { &vm.config.disk_image });
                ui.end_row();

                ui.label("ISO:");
                ui.label(if vm.config.iso_image.is_empty() { "None" } else { &vm.config.iso_image });
                ui.end_row();

                ui.label("Boot Order:");
                ui.label(format!("{:?}", vm.config.boot_order));
                ui.end_row();

                ui.label("Network:");
                ui.label(if vm.config.net_enabled {
                    format!("{:?}", vm.config.net_mode)
                } else {
                    "Disabled".into()
                });
                ui.end_row();
            });

        ui.add_space(20.0);

        if vm.state == VmState::Stopped {
            if ui.add(egui::Button::new("▶ Start VM").min_size(egui::vec2(120.0, 40.0))).clicked() {
                *deferred_action = Some(ToolbarAction::Start);
            }
        }
    });
}
