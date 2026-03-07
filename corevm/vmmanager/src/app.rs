use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;

use crate::config::VmConfig;
use crate::platform;
use crate::sidebar::{self, SidebarLayout, VmState};
use crate::statusbar::{self, VmMetrics};
use crate::toolbar::{self, ToolbarAction};
use crate::theme;

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
    pub show_settings: bool,
    pub show_create_vm: bool,
    pub show_create_disk: bool,
    pub show_about: bool,
    pub show_snapshots: bool,
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
            show_settings: false,
            show_create_vm: false,
            show_create_disk: false,
            show_about: false,
            show_snapshots: false,
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
            ToolbarAction::Settings => self.show_settings = true,
            ToolbarAction::Snapshot => self.show_snapshots = true,
            // Start/Stop/Pause will be handled when vm.rs is implemented
            _ => {}
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
                self.handle_toolbar_action(action);
            }
            ui.separator();

            // Main content area
            if let Some(uuid) = &self.selected_vm.clone() {
                if let Some(vm) = self.find_vm(uuid) {
                    if vm.state == VmState::Running {
                        // Display framebuffer (placeholder until display.rs)
                        ui.centered_and_justified(|ui| {
                            ui.label("VM Display — framebuffer will render here");
                        });
                    } else {
                        // Summary panel (placeholder until Task 15)
                        render_summary(ui, vm);
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.heading("Select or create a VM to get started");
                });
            }
        });

        // Request repaint for live updates when VM is running
        if self.selected_vm.as_ref()
            .and_then(|u| self.find_vm(u))
            .map_or(false, |v| v.state == VmState::Running)
        {
            ctx.request_repaint();
        }
    }
}

/// Simple summary panel showing VM details
fn render_summary(ui: &mut egui::Ui, vm: &VmEntry) {
    ui.vertical(|ui| {
        ui.add_space(20.0);
        ui.heading(&vm.config.name);
        ui.add_space(10.0);

        egui::Grid::new("vm_summary")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Status:");
                ui.colored_label(egui::Color32::from_rgb(76, 175, 80), "Stopped");
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
    });
}
