use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;

use crate::config::VmConfig;
use crate::dialogs::{AboutDialog, AddDiskDialog, AddDiskMode, CreateDiskDialog, CreateVmDialog, DiskPoolDialog, SnapshotsDialog};
use crate::display::DisplayWidget;
use crate::filebrowser::FileBrowserDialog;
use crate::input;
use crate::platform;
use crate::diagnostics::{DiagLog, DiagnosticsWindow};
use crate::settings::SettingsDialog;
use crate::sidebar::{self, SidebarAction, SidebarLayout, SidebarState, VmState};
use crate::statusbar::{self, VmMetrics};
use crate::theme;
use crate::toolbar::{self, ToolbarAction};
use crate::vm;
use crate::vm::VmControl;
use libcorevm::ffi::{corevm_ps2_key_press, corevm_ps2_key_release};

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
    pub cpu_mode: u32,  // 0=real, 1=protected, 2=long
    pub diag_log: DiagLog,
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
            cpu_mode: 0,
            diag_log: DiagLog::new(),
        }
    }
}

/// Identifies which field a file dialog is picking for
#[derive(Clone, Debug)]
pub enum FilePickTarget {
    SettingsIso,
    CreateDiskPath,
    AddDisk,
    AddDiskBrowseExisting,
    AddDiskBrowseVmdk,
    AddDiskBrowseCreate,
    ExportVmLog,
    ExportBiosLog,
}

pub struct CoreVmApp {
    pub vms: Vec<VmEntry>,
    pub layout: SidebarLayout,
    pub selected_vm: Option<String>,  // UUID
    pub display: DisplayWidget,
    pub settings_dialog: Option<SettingsDialog>,
    pub create_vm_dialog: Option<CreateVmDialog>,
    pub create_disk_dialog: Option<CreateDiskDialog>,
    pub add_disk_dialog: Option<AddDiskDialog>,
    pub disk_pool_dialog: Option<DiskPoolDialog>,
    pub about_dialog: Option<AboutDialog>,
    pub snapshots_dialog: Option<SnapshotsDialog>,
    pub diagnostics_window: Option<DiagnosticsWindow>,
    pub error_message: Option<String>,
    pub info_message: Option<String>,
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
            settings_dialog: None,
            create_vm_dialog: None,
            create_disk_dialog: None,
            add_disk_dialog: None,
            disk_pool_dialog: None,
            about_dialog: None,
            snapshots_dialog: None,
            diagnostics_window: None,
            error_message: None,
            info_message: None,
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

    fn vm_icons(&self) -> HashMap<String, &'static str> {
        self.vms.iter().map(|v| {
            let icon = if v.config.guest_os.is_windows() {
                "\u{1FA9F}" // 🪟
            } else if v.config.guest_os.is_linux() {
                "\u{1F427}" // 🐧
            } else {
                "\u{1F5A5}" // 🖥
            };
            (v.config.uuid.clone(), icon)
        }).collect()
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
                    let mut started_ok = false;
                    let mut usb_tablet = false;
                    let mut diag_name = String::new();
                    if let Some(entry) = self.find_vm_mut(&uuid) {
                        usb_tablet = entry.config.usb_tablet;
                        if let Err(e) = vm::start_vm(entry) {
                            self.error_message = Some(format!("Failed to start VM: {}", e));
                        } else {
                            started_ok = true;
                            if entry.config.diagnostics {
                                diag_name = entry.config.name.clone();
                            }
                        }
                    }
                    if started_ok {
                        self.display.usb_tablet_mode = usb_tablet;
                        if !diag_name.is_empty() {
                            self.diagnostics_window = Some(DiagnosticsWindow::new(&diag_name));
                        }
                    }
                }
            }
            ToolbarAction::Stop => {
                if let Some(uuid) = self.selected_vm.clone() {
                    if let Some(entry) = self.find_vm_mut(&uuid) {
                        vm::stop_vm(entry);
                    }
                    // Mark for mouse release (actual release in update() where ctx is available)
                    if self.display.mouse_captured {
                        self.display.mouse_captured = false;
                        self.display.needs_cursor_restore = true;
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
            ToolbarAction::Screenshot => {
                let result = self.selected_vm.as_ref()
                    .and_then(|uuid| self.find_vm(uuid))
                    .map(|entry| entry.framebuffer.clone());
                if let Some(fb_arc) = result {
                    if let Ok(fb) = fb_arc.lock() {
                        if fb.width > 0 && fb.height > 0 && !fb.pixels.is_empty() {
                            match copy_framebuffer_to_clipboard(&fb) {
                                Ok(()) => {
                                    self.info_message = Some(format!(
                                        "Screenshot copied to clipboard ({}x{})",
                                        fb.width, fb.height
                                    ));
                                }
                                Err(e) => {
                                    self.error_message = Some(format!("Screenshot failed: {}", e));
                                }
                            }
                        } else {
                            self.error_message = Some("No framebuffer data available.".into());
                        }
                    }
                }
            }
            ToolbarAction::ClipboardToGuest => {
                // Read host clipboard text and inject as PS/2 keystrokes
                if let Some(uuid) = &self.selected_vm.clone() {
                    if let Some(vm) = self.find_vm(uuid) {
                        if let Some(handle) = vm.vm_handle {
                            match arboard::Clipboard::new() {
                                Ok(mut clipboard) => {
                                    match clipboard.get_text() {
                                        Ok(text) => {
                                            if text.is_empty() {
                                                self.error_message = Some("Host clipboard is empty.".into());
                                            } else {
                                                let typed = input::type_string_to_vm(handle, &text);
                                                self.error_message = Some(format!(
                                                    "Pasted {} characters to guest.", typed
                                                ));
                                            }
                                        }
                                        Err(e) => {
                                            self.error_message = Some(format!(
                                                "Failed to read clipboard: {}", e
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    self.error_message = Some(format!(
                                        "Failed to open clipboard: {}", e
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            ToolbarAction::ClipboardFromGuest => {
                // Copy VGA text buffer content to host clipboard
                let result = self.selected_vm.as_ref()
                    .and_then(|uuid| self.find_vm(uuid))
                    .map(|entry| entry.framebuffer.clone());
                if let Some(fb_arc) = result {
                    if let Ok(fb) = fb_arc.lock() {
                        if fb.text_mode && !fb.text_buffer.is_empty() {
                            // VGA text mode: 80 columns, rows = buffer_len / 80
                            let cols = 80u32;
                            let rows = (fb.text_buffer.len() as u32) / cols;
                            let text = extract_vga_text(&fb.text_buffer, cols, rows);
                            if text.trim().is_empty() {
                                self.error_message = Some("Guest text buffer is empty.".into());
                            } else {
                                match arboard::Clipboard::new() {
                                    Ok(mut clipboard) => {
                                        match clipboard.set_text(&text) {
                                            Ok(()) => {
                                                let lines = text.lines().count();
                                                self.error_message = Some(format!(
                                                    "Copied {} lines of guest text to clipboard.", lines
                                                ));
                                            }
                                            Err(e) => {
                                                self.error_message = Some(format!(
                                                    "Failed to set clipboard: {}", e
                                                ));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        self.error_message = Some(format!(
                                            "Failed to open clipboard: {}", e
                                        ));
                                    }
                                }
                            }
                        } else {
                            self.error_message = Some(
                                "Guest is not in text mode — use Screenshot instead.".into()
                            );
                        }
                    }
                }
            }
        }
    }

    fn selected_metrics(&self) -> Option<VmMetrics> {
        let uuid = self.selected_vm.as_ref()?;
        let vm = self.find_vm(uuid)?;
        if vm.state != VmState::Running { return None; }

        Some(VmMetrics {
            state_label: "Running",
        })
    }
}

impl eframe::App for CoreVmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply_theme(ctx);

        // Restore cursor if mouse capture was cleared externally (e.g., VM stopped via toolbar).
        // The display's release_mouse() can't be called from toolbar handlers because they
        // don't have ctx, so we check and restore here.
        if self.display.needs_cursor_restore {
            self.display.needs_cursor_restore = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }

        // Check for mouse release shortcuts BEFORE keyboard handling consumes events.
        // handle_keyboard_events removes all Key events, so Ctrl+Alt+G would never
        // reach display.show() otherwise.
        if self.display.mouse_captured {
            // Check via events (before handle_keyboard_events consumes them)
            let release = ctx.input(|i| {
                i.events.iter().any(|e| matches!(e,
                    egui::Event::Key { key: egui::Key::G, pressed: true, modifiers, .. }
                        if modifiers.ctrl && modifiers.alt
                ) || matches!(e,
                    egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. }
                        if modifiers.ctrl && modifiers.alt
                ))
            });
            // Also check via modifier state (works even if events are broken)
            let mod_release = ctx.input(|i| {
                i.modifiers.ctrl && i.modifiers.alt
                    && (i.key_pressed(egui::Key::G) || i.key_pressed(egui::Key::Escape))
            });
            if release || mod_release {
                self.display.release_mouse(ctx);
            }
        }

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

        // Determine VM state for toolbar buttons
        let (vm_selected, vm_running, vm_paused, vm_diag_enabled) = if let Some(uuid) = &self.selected_vm {
            if let Some(vm) = self.find_vm(uuid) {
                (true, vm.state == VmState::Running, vm.state == VmState::Paused, vm.config.diagnostics)
            } else {
                (false, false, false, false)
            }
        } else {
            (false, false, false, false)
        };

        // Combined menu + toolbar bar (VMware-style)
        egui::TopBottomPanel::top("menu_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(36, 36, 38))
                    .inner_margin(egui::Margin::symmetric(6, 2)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Menu buttons (left side)
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
                            if ui.button("Disk Pool...").clicked() {
                                let configs: Vec<_> = self.vms.iter().map(|v| v.config.clone()).collect();
                                self.disk_pool_dialog = Some(DiskPoolDialog::new(&configs));
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
                            let can_send = vm_selected && vm_running;
                            if ui.add_enabled(can_send, egui::Button::new("Send Ctrl+Alt+Del")).clicked() {
                                if let Some(uuid) = &self.selected_vm {
                                    if let Some(entry) = self.vms.iter().find(|v| &v.config.uuid == uuid) {
                                        if let Some(handle) = entry.vm_handle {
                                            // Ctrl press, Alt press, E0+Del press, E0+Del release, Alt release, Ctrl release
                                            corevm_ps2_key_press(handle, 0x1D); // Ctrl
                                            corevm_ps2_key_press(handle, 0x38); // Alt
                                            corevm_ps2_key_press(handle, 0xE0); // Extended prefix
                                            corevm_ps2_key_press(handle, 0x53); // Del make
                                            corevm_ps2_key_press(handle, 0xE0); // Extended prefix
                                            corevm_ps2_key_release(handle, 0x53); // Del break (0xD3)
                                            corevm_ps2_key_release(handle, 0x38); // Alt release (0xB8)
                                            corevm_ps2_key_release(handle, 0x1D); // Ctrl release (0x9D)
                                        }
                                    }
                                }
                                ui.close_menu();
                            }
                        });

                        if vm_selected && vm_diag_enabled {
                            ui.menu_button("Diagnostics", |ui| {
                                let diag_open = self.diagnostics_window.is_some();
                                let label = if diag_open { "Hide Diagnostics Window" } else { "Show Diagnostics Window" };
                                if ui.add_enabled(vm_running, egui::Button::new(label)).clicked() {
                                    if diag_open {
                                        self.diagnostics_window = None;
                                    } else if let Some(uuid) = &self.selected_vm {
                                        if let Some(entry) = self.vms.iter().find(|v| &v.config.uuid == uuid) {
                                            self.diagnostics_window = Some(DiagnosticsWindow::new(&entry.config.name));
                                        }
                                    }
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.add_enabled(vm_running, egui::Button::new("Export VM Log...")).clicked() {
                                    let ts = chrono_timestamp();
                                    self.file_browser = Some(FileBrowserDialog::new_save_with_name(
                                        "Export VM Log", &["*.txt", "*.log"],
                                        &format!("vm_log_{}.txt", ts),
                                    ));
                                    self.file_pick_target = Some(FilePickTarget::ExportVmLog);
                                    ui.close_menu();
                                }
                                if ui.add_enabled(vm_running, egui::Button::new("Export BIOS Log...")).clicked() {
                                    let ts = chrono_timestamp();
                                    self.file_browser = Some(FileBrowserDialog::new_save_with_name(
                                        "Export BIOS Log", &["*.txt", "*.log"],
                                        &format!("bios_log_{}.txt", ts),
                                    ));
                                    self.file_pick_target = Some(FilePickTarget::ExportBiosLog);
                                    ui.close_menu();
                                }
                            });
                        }

                        ui.menu_button("Help", |ui| {
                            if ui.button("About CoreVM...").clicked() {
                                self.about_dialog = Some(AboutDialog::new());
                                ui.close_menu();
                            }
                        });

                        // Divider between menus and toolbar
                        ui.add_space(8.0);
                        ui.colored_label(theme::TEXT_TERTIARY, "|");
                        ui.add_space(8.0);

                        // Toolbar buttons
                        if let Some(action) = toolbar::render_toolbar(ui, vm_selected, vm_running, vm_paused) {
                            deferred_action = Some(action);
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
        let icons = self.vm_icons();
        let sidebar_actions = sidebar::render_sidebar(
            ctx, &mut self.layout, &names, &states, &icons,
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
            if let Some(uuid) = &self.selected_vm.clone() {
                // Extract state and data from vm without holding borrow on self
                let vm_info = self.find_vm(uuid).map(|vm| {
                    (vm.state, vm.framebuffer.clone(), vm.vm_handle)
                });

                if let Some((state, fb, vm_handle)) = vm_info {
                    if state == VmState::Running || state == VmState::Paused {
                        let (display_focused, _display_rect) = if let Ok(fb_data) = fb.lock() {
                            self.display.show(ui, ctx, &fb_data, vm_handle)
                        } else {
                            (false, None)
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
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        ui.label(
                            egui::RichText::new("No Machine Selected")
                                .size(20.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Select a virtual machine from the sidebar or create a new one")
                                .size(13.0)
                                .color(theme::TEXT_TERTIARY),
                        );
                    });
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
                Some(FilePickTarget::AddDiskBrowseExisting)
                | Some(FilePickTarget::AddDiskBrowseVmdk)
                | Some(FilePickTarget::AddDiskBrowseCreate) => {
                    if let Some(ref mut dlg) = self.add_disk_dialog {
                        dlg.set_path(path);
                    }
                }
                Some(FilePickTarget::ExportVmLog) => {
                    if let Some(uuid) = &self.selected_vm {
                        if let Some(entry) = self.vms.iter().find(|v| &v.config.uuid == uuid) {
                            let text = entry.diag_log.export_vm_log();
                            let _ = std::fs::write(&path, text);
                        }
                    }
                }
                Some(FilePickTarget::ExportBiosLog) => {
                    if let Some(uuid) = &self.selected_vm {
                        if let Some(entry) = self.vms.iter().find(|v| &v.config.uuid == uuid) {
                            let text = entry.diag_log.export_bios_log();
                            let _ = std::fs::write(&path, text);
                        }
                    }
                }
                Some(FilePickTarget::AddDisk) | None => {}
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
                FilePickTarget::SettingsIso => {
                    self.file_browser = Some(FileBrowserDialog::new_open("Select ISO Image", &["iso"]));
                }
                FilePickTarget::AddDisk => {
                    // Open AddDiskDialog instead of file browser
                    self.add_disk_dialog = Some(AddDiskDialog::new());
                    self.file_pick_target = None;
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

        // Add Disk dialog
        let mut add_disk_browse: Option<AddDiskMode> = None;
        if let Some(ref mut dialog) = self.add_disk_dialog {
            if let Some(mode) = dialog.show_with_browse(ctx) {
                add_disk_browse = Some(mode);
            }
            if !dialog.is_open() {
                if let Some(path) = dialog.result_path.take() {
                    // Add the disk to the settings dialog if open
                    if let Some(ref mut settings) = self.settings_dialog {
                        settings.add_disk_image(path);
                    }
                }
                self.add_disk_dialog = None;
            }
        }
        if let Some(mode) = add_disk_browse {
            match mode {
                AddDiskMode::LoadExisting => {
                    self.file_pick_target = Some(FilePickTarget::AddDiskBrowseExisting);
                    self.file_browser = Some(FileBrowserDialog::new_open("Select Disk Image", &["img", "raw", "qcow2"]));
                }
                AddDiskMode::ImportVmdk => {
                    self.file_pick_target = Some(FilePickTarget::AddDiskBrowseVmdk);
                    self.file_browser = Some(FileBrowserDialog::new_open("Select VMDK File", &["vmdk"]));
                }
                AddDiskMode::CreateNew => {
                    self.file_pick_target = Some(FilePickTarget::AddDiskBrowseCreate);
                    self.file_browser = Some(FileBrowserDialog::new_save("Save Disk Image", &["img", "raw"]));
                }
            }
        }

        // Disk Pool dialog
        if let Some(ref mut dialog) = self.disk_pool_dialog {
            if !dialog.show(ctx) {
                self.disk_pool_dialog = None;
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

        // Diagnostics window — shown in a separate OS-level window (viewport)
        if let Some(ref mut diag_win) = self.diagnostics_window {
            if diag_win.open {
                // Collect the diag log reference before the closure
                let diag_log = self.selected_vm.as_ref().and_then(|uuid| {
                    self.vms.iter().find(|v| &v.config.uuid == uuid)
                }).map(|entry| entry.diag_log.clone());

                if let Some(log) = diag_log {
                    let title = format!("Diagnostics - {}", diag_win.vm_name);
                    let vp_id = egui::ViewportId::from_hash_of("diagnostics_viewport");
                    ctx.show_viewport_immediate(vp_id, egui::ViewportBuilder::default()
                        .with_title(&title)
                        .with_inner_size([700.0, 500.0])
                        .with_min_inner_size([400.0, 200.0]),
                        |ctx, _class| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                diag_win.show_contents(ui, &log);
                            });
                            if ctx.input(|i| i.viewport().close_requested()) {
                                diag_win.open = false;
                            }
                        },
                    );
                }
            }
            if !diag_win.open {
                self.diagnostics_window = None;
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

        // Info dialog (success messages — green accent)
        if let Some(msg) = self.info_message.clone() {
            let mut dismiss = false;
            egui::Window::new("Info")
                .collapsible(false)
                .resizable(false)
                .min_width(350.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.screen_rect().center())
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("\u{2714}").size(20.0).color(theme::SUCCESS_GREEN));
                        ui.add_space(4.0);
                        ui.label(&msg);
                    });
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("OK").fill(theme::SUCCESS_GREEN).min_size(egui::vec2(80.0, 28.0))).clicked() {
                                dismiss = true;
                            }
                        });
                    });
                });
            if dismiss {
                self.info_message = None;
            }
        }

        // Check if any running VM thread has exited (or requested reboot)
        let mut reboot_uuid: Option<String> = None;
        let mut vm_exited = false;
        for vm in &mut self.vms {
            if vm.state == VmState::Running {
                if let Some(ref ctl) = vm.control {
                    if ctl.exited.load(std::sync::atomic::Ordering::Relaxed) {
                        let wants_reboot = ctl.reboot_requested.load(std::sync::atomic::Ordering::Relaxed);
                        if wants_reboot {
                            // Guest requested reboot — stop and restart
                            vm.state = VmState::Stopped;
                            vm::stop_vm(vm);
                            reboot_uuid = Some(vm.config.uuid.clone());
                        } else {
                            let reason = ctl.exit_reason.lock()
                                .map(|r| r.clone())
                                .unwrap_or_default();
                            vm.state = VmState::Stopped;
                            vm_exited = true;
                            self.error_message = Some(format!(
                                "VM '{}' stopped ({})",
                                vm.config.name, reason
                            ));
                        }
                    }
                }
            }
        }
        // Handle reboot: restart the VM
        if let Some(uuid) = reboot_uuid {
            if let Some(entry) = self.find_vm_mut(&uuid) {
                if let Err(e) = vm::start_vm(entry) {
                    self.error_message = Some(format!("Reboot failed: {}", e));
                }
            }
        }

        // Release mouse capture when VM exits unexpectedly
        if vm_exited && self.display.mouse_captured {
            self.display.mouse_captured = false;
            self.display.needs_cursor_restore = true;
        }

        // Repaint when VM running
        if self.vms.iter().any(|v| v.state == VmState::Running) {
            ctx.request_repaint();
        }
    }
}

fn render_summary(ui: &mut egui::Ui, vm: &VmEntry, deferred_action: &mut Option<ToolbarAction>) {
    let available = ui.available_size();

    let screen_aspect = 16.0 / 10.0;
    let max_screen_h = (available.y * 0.45).clamp(150.0, 350.0);
    let max_screen_w = (available.x - 80.0).max(250.0);
    let (screen_w, screen_h) = if max_screen_w / max_screen_h > screen_aspect {
        (max_screen_h * screen_aspect, max_screen_h)
    } else {
        (max_screen_w, max_screen_w / screen_aspect)
    };

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        // Dark screen rectangle with subtle shadow
        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(screen_w, screen_h),
            egui::Sense::hover(),
        );

        let painter = ui.painter_at(rect);

        // Shadow behind screen
        let shadow_rect = rect.expand(3.0).translate(egui::vec2(0.0, 2.0));
        painter.rect_filled(shadow_rect, 12.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 40));

        // Screen background
        painter.rect_filled(rect, 10.0, egui::Color32::from_rgb(18, 18, 20));

        // Subtle border
        painter.rect_stroke(rect, 10.0, egui::Stroke::new(0.5, egui::Color32::from_rgb(50, 50, 54)), egui::StrokeKind::Outside);

        // Power icon (large circle)
        let icon_center = rect.center() - egui::vec2(0.0, 20.0);
        painter.circle_stroke(icon_center, 28.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(72, 72, 76)));
        painter.line_segment(
            [icon_center - egui::vec2(0.0, 14.0), icon_center - egui::vec2(0.0, 30.0)],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(72, 72, 76)),
        );

        // VM name
        painter.text(
            rect.center() + egui::vec2(0.0, 25.0),
            egui::Align2::CENTER_CENTER,
            &vm.config.name,
            egui::FontId::proportional(18.0),
            egui::Color32::from_rgb(142, 142, 147),
        );

        // State label
        let (state_label, state_color) = match vm.state {
            VmState::Running => ("Running", theme::SUCCESS_GREEN),
            VmState::Paused => ("Paused", theme::WARNING_ORANGE),
            VmState::Stopped => ("Powered Off", egui::Color32::from_rgb(99, 99, 102)),
        };
        painter.text(
            rect.center() + egui::vec2(0.0, 48.0),
            egui::Align2::CENTER_CENTER,
            state_label,
            egui::FontId::proportional(13.0),
            state_color,
        );

        ui.add_space(16.0);

        // --- Info cards below screen ---
        let card_bg = egui::Color32::from_rgb(44, 44, 46);
        let card_radius = 10.0;
        let label_color = egui::Color32::from_rgb(142, 142, 147);
        let value_color = egui::Color32::from_rgb(220, 220, 225);

        // Collect info items
        let ram_str = if vm.config.ram_mb >= 1024 {
            format!("{:.1} GB", vm.config.ram_mb as f64 / 1024.0)
        } else {
            format!("{} MB", vm.config.ram_mb)
        };
        let mut items: Vec<(&str, String)> = vec![
            ("OS", format!("{} ({})", vm.config.guest_os.label(),
                match vm.config.guest_arch {
                    crate::config::GuestArch::X64 => "64-bit",
                    crate::config::GuestArch::X86 => "32-bit",
                })),
            ("Memory", ram_str),
            ("CPUs", format!("{} {}", vm.config.cpu_cores, if vm.config.cpu_cores == 1 { "core" } else { "cores" })),
        ];
        for (i, disk) in vm.config.disk_images.iter().enumerate() {
            if !disk.is_empty() {
                let disk_name = std::path::Path::new(disk)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| disk.clone());
                items.push(("Disk", disk_name));
            }
        }
        if !vm.config.iso_image.is_empty() {
            let iso_name = std::path::Path::new(&vm.config.iso_image)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| vm.config.iso_image.clone());
            items.push(("CD/DVD", iso_name));
        }

        // Render as a grouped card
        let card_width = screen_w.min(500.0);
        egui::Frame::new()
            .fill(card_bg)
            .corner_radius(egui::CornerRadius::same(card_radius as u8))
            .inner_margin(egui::Margin::symmetric(16, 10))
            .show(ui, |ui| {
                ui.set_width(card_width);
                for (i, (label, value)) in items.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        ui.colored_label(label_color, egui::RichText::new(*label).size(13.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.colored_label(value_color, egui::RichText::new(value.as_str()).size(13.0));
                        });
                    });
                }
            });

        ui.add_space(16.0);

        // Power On button
        if vm.state == VmState::Stopped {
            if ui.add(
                egui::Button::new(
                    egui::RichText::new("Power On")
                        .size(15.0)
                        .color(egui::Color32::WHITE),
                )
                .fill(theme::ACCENT_BLUE)
                .corner_radius(egui::CornerRadius::same(10))
                .min_size(egui::vec2(180.0, 42.0)),
            )
            .clicked()
            {
                *deferred_action = Some(ToolbarAction::Start);
            }
        }
    });
}

/// Extract text from a VGA text-mode buffer (array of u16: low byte = char, high byte = attr).
/// Returns the text with trailing spaces trimmed per line.
fn extract_vga_text(text_buffer: &[u16], cols: u32, rows: u32) -> String {
    let cols = cols.max(1) as usize;
    let rows = rows.max(1) as usize;
    let mut result = String::with_capacity(cols * rows);

    for row in 0..rows {
        let start = row * cols;
        let end = (start + cols).min(text_buffer.len());
        if start >= text_buffer.len() {
            break;
        }

        let mut line = String::with_capacity(cols);
        for i in start..end {
            let ch = (text_buffer[i] & 0xFF) as u8;
            // Map VGA characters: printable ASCII or space
            if ch >= 0x20 && ch < 0x7F {
                line.push(ch as char);
            } else {
                line.push(' ');
            }
        }
        // Trim trailing spaces
        let trimmed = line.trim_end();
        result.push_str(trimmed);
        result.push('\n');
    }

    // Remove trailing empty lines
    while result.ends_with("\n\n") {
        result.pop();
    }

    result
}

/// Copy the current framebuffer contents to the OS clipboard as an image.
fn copy_framebuffer_to_clipboard(fb: &FrameBufferData) -> Result<(), String> {
    let w = fb.width as usize;
    let h = fb.height as usize;

    if fb.pixels.len() < w * h * 4 {
        return Err("Framebuffer data incomplete".into());
    }

    // arboard expects RGBA pixels
    let img = arboard::ImageData {
        width: w,
        height: h,
        bytes: std::borrow::Cow::Borrowed(&fb.pixels[..w * h * 4]),
    };

    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| format!("Failed to open clipboard: {}", e))?;
    clipboard.set_image(img)
        .map_err(|e| format!("Failed to set clipboard image: {}", e))?;

    Ok(())
}


/// Generate a compact timestamp string for filenames (YYYYMMDD_HHMMSS).
fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Simple date calc (no leap-second accuracy needed for filenames)
    let mut y = 1970u64;
    let mut rem = days;
    loop {
        let ydays = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if rem < ydays { break; }
        rem -= ydays;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0u64;
    for &md in &mdays {
        if rem < md { break; }
        rem -= md;
        mo += 1;
    }
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, mo + 1, rem + 1, h, m, s)
}
