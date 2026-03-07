# CoreVM VMManager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a cross-platform (Linux/Windows) VM management GUI using egui, with libcorevm as in-process VM engine.

**Architecture:** Single Cargo project with `#[cfg(target_os)]` for platform differences. egui/eframe for GPU-accelerated UI. VM runs in dedicated thread, framebuffer shared via `Arc<Mutex<>>`. Config files in anyOS-compatible key=value format.

**Tech Stack:** Rust, egui 0.31, eframe 0.31, libcorevm (host_test feature), rfd (file dialogs), uuid

---

### Task 1: Project Scaffold & Cargo Setup

**Files:**
- Create: `corevm/vmmanager/Cargo.toml`
- Create: `corevm/vmmanager/src/main.rs`
- Create: `corevm/tools/build_linux.sh`
- Create: `corevm/tools/build_win64.bat`

**Step 1: Create directory structure**

```bash
mkdir -p corevm/vmmanager/src
mkdir -p corevm/vmmanager/assets/icons
mkdir -p corevm/vmmanager/linux
mkdir -p corevm/vmmanager/win64
mkdir -p corevm/tools
```

**Step 2: Create Cargo.toml**

```toml
[package]
name = "corevm-vmmanager"
version = "0.1.0"
edition = "2021"

[dependencies]
eframe = "0.31"
egui = "0.31"
egui_extras = { version = "0.31", features = ["image"] }
libcorevm = { path = "../../libs/libcorevm", features = ["host_test"] }
uuid = { version = "1", features = ["v4"] }
rfd = "0.15"

[workspace]
```

**Step 3: Create minimal main.rs that opens an egui window**

```rust
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("CoreVM Manager"),
        ..Default::default()
    };
    eframe::run_native(
        "CoreVM Manager",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

#[derive(Default)]
struct App;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("CoreVM Manager");
        });
    }
}
```

**Step 4: Create build scripts**

`corevm/tools/build_linux.sh`:
```bash
#!/bin/bash
set -e
cd "$(dirname "$0")/../vmmanager"
cargo build --release
echo "Built: target/release/corevm-vmmanager"
```

`corevm/tools/build_win64.bat`:
```batch
@echo off
cd /d "%~dp0\..\vmmanager"
cargo build --release --target x86_64-pc-windows-msvc
echo Built: target\x86_64-pc-windows-msvc\release\corevm-vmmanager.exe
```

**Step 5: Verify it compiles and runs**

Run: `cd corevm/vmmanager && cargo build 2>&1`
Expected: Successful compilation (window opens if run)

**Step 6: Commit**

```bash
git add corevm/
git commit -m "feat(vmmanager): project scaffold with egui window"
```

---

### Task 2: Theme & Dark Mode Styling

**Files:**
- Create: `corevm/vmmanager/src/theme.rs`
- Modify: `corevm/vmmanager/src/main.rs`

**Step 1: Create theme.rs with VMware/VirtualBox-inspired dark theme**

```rust
use eframe::egui::{self, Color32, Rounding, Stroke, Style, Visuals};

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = Style::default();
    let mut visuals = Visuals::dark();

    // Background colors
    visuals.window_fill = Color32::from_rgb(30, 30, 30);
    visuals.panel_fill = Color32::from_rgb(37, 37, 38);
    visuals.faint_bg_color = Color32::from_rgb(45, 45, 48);
    visuals.extreme_bg_color = Color32::from_rgb(25, 25, 25);

    // Widget styling
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(45, 45, 48);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(204, 204, 204));
    visuals.widgets.noninteractive.rounding = Rounding::same(4.0);

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(55, 55, 58);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(204, 204, 204));

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(62, 62, 66);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 122, 204);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    // Selection
    visuals.selection.bg_fill = Color32::from_rgb(0, 122, 204);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    // Separator
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 60));

    style.visuals = visuals;

    // Spacing
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);

    ctx.set_style(style);
}

/// Accent color for active/running indicators
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0, 122, 204);
pub const SUCCESS_GREEN: Color32 = Color32::from_rgb(76, 175, 80);
pub const WARNING_ORANGE: Color32 = Color32::from_rgb(255, 152, 0);
pub const ERROR_RED: Color32 = Color32::from_rgb(244, 67, 54);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(30, 30, 30);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(45, 45, 48);
pub const STATUSBAR_BG: Color32 = Color32::from_rgb(0, 122, 204);
```

**Step 2: Wire theme into main.rs**

Add `mod theme;` and call `theme::apply_theme(ctx)` at start of `update()`.

**Step 3: Verify it compiles**

Run: `cd corevm/vmmanager && cargo build 2>&1`

**Step 4: Commit**

```bash
git add corevm/vmmanager/src/theme.rs corevm/vmmanager/src/main.rs
git commit -m "feat(vmmanager): dark theme styling"
```

---

### Task 3: Platform Abstraction

**Files:**
- Create: `corevm/vmmanager/src/platform.rs`

**Step 1: Create platform.rs with config/BIOS path resolution**

```rust
use std::path::PathBuf;

/// Returns the directory where VM configs are stored.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/corevm/vms")
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\CoreVM".into());
        PathBuf::from(appdata).join("CoreVM\\vms")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("./vms")
    }
}

/// Returns the directory where layout.conf is stored.
pub fn layout_dir() -> PathBuf {
    config_dir().parent().unwrap_or(&config_dir()).to_path_buf()
}

/// Search paths for BIOS files.
pub fn bios_search_paths() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let mut paths = Vec::new();
    if let Some(d) = &exe_dir {
        paths.push(d.join("bios"));
        paths.push(d.to_path_buf());
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/share/corevm/bios"));
        paths.push(PathBuf::from("/usr/local/share/corevm/bios"));
    }

    paths
}

/// Find a BIOS file by name in search paths.
pub fn find_bios(name: &str) -> Option<PathBuf> {
    for dir in bios_search_paths() {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Ensure the config directory exists.
pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(config_dir());
}
```

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/platform.rs
git commit -m "feat(vmmanager): platform abstraction for paths"
```

---

### Task 4: VM Configuration (Load/Save)

**Files:**
- Create: `corevm/vmmanager/src/config.rs`

**Step 1: Create config.rs with VmConfig struct and key=value I/O**

The config format must be compatible with the anyOS VMManager. Reference `apps/vmmanager/src/main.rs` lines 452-646 for format.

```rust
use std::path::{Path, PathBuf};
use std::fs;

#[derive(Clone, Debug, PartialEq)]
pub enum BootOrder { DiskFirst, CdFirst, FloppyFirst }

#[derive(Clone, Debug, PartialEq)]
pub enum BiosType { CoreVm, SeaBios }

#[derive(Clone, Debug, PartialEq)]
pub enum RamAlloc { Preallocate, OnDemand }

#[derive(Clone, Debug, PartialEq)]
pub enum NetMode { Nat, Bridge }

#[derive(Clone, Debug, PartialEq)]
pub enum MacMode { Dynamic, Static }

#[derive(Clone, Debug)]
pub struct VmConfig {
    pub uuid: String,
    pub name: String,
    pub ram_mb: u32,
    pub cpu_cores: u32,
    pub disk_image: String,
    pub iso_image: String,
    pub boot_order: BootOrder,
    pub bios_type: BiosType,
    pub jit_enabled: bool,
    pub gpu_type: String,
    pub net_enabled: bool,
    pub net_mode: NetMode,
    pub net_host_nic: String,
    pub mac_mode: MacMode,
    pub mac_address: String,
    pub ram_alloc: RamAlloc,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string().replace("-", ""),
            name: "New VM".into(),
            ram_mb: 256,
            cpu_cores: 1,
            disk_image: String::new(),
            iso_image: String::new(),
            boot_order: BootOrder::CdFirst,
            bios_type: BiosType::SeaBios,
            jit_enabled: false,
            gpu_type: "svga".into(),
            net_enabled: false,
            net_mode: NetMode::Nat,
            net_host_nic: String::new(),
            mac_mode: MacMode::Dynamic,
            mac_address: String::new(),
            ram_alloc: RamAlloc::OnDemand,
        }
    }
}

impl VmConfig {
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(format!("{}.conf", self.uuid));
        let boot = match self.boot_order {
            BootOrder::DiskFirst => "disk",
            BootOrder::CdFirst => "cd",
            BootOrder::FloppyFirst => "floppy",
        };
        let bios = match self.bios_type {
            BiosType::CoreVm => "corevm",
            BiosType::SeaBios => "seabios",
        };
        let alloc = match self.ram_alloc {
            RamAlloc::Preallocate => "preallocate",
            RamAlloc::OnDemand => "ondemand",
        };
        let net_mode = match self.net_mode {
            NetMode::Nat => "nat",
            NetMode::Bridge => "bridge",
        };
        let mac_mode = match self.mac_mode {
            MacMode::Dynamic => "dynamic",
            MacMode::Static => "static",
        };
        let content = format!(
            "name={}\nram={}\ncpu_cores={}\ndisk={}\niso={}\nboot={}\nbios={}\njit={}\n\
             ram_alloc={}\ngpu={}\nnet_enabled={}\nnet_mode={}\nnet_host_nic={}\n\
             mac_mode={}\nmac_address={}\n",
            self.name, self.ram_mb, self.cpu_cores, self.disk_image, self.iso_image,
            boot, bios, if self.jit_enabled { "1" } else { "0" },
            alloc, self.gpu_type,
            if self.net_enabled { "1" } else { "0" },
            net_mode, self.net_host_nic, mac_mode, self.mac_address,
        );
        fs::write(&path, content)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = fs::read_to_string(path)?;
        let uuid = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut cfg = VmConfig { uuid, ..Default::default() };

        for line in content.lines() {
            let Some((key, val)) = line.split_once('=') else { continue };
            match key.trim() {
                "name" => cfg.name = val.to_string(),
                "ram" => cfg.ram_mb = val.parse().unwrap_or(256),
                "cpu_cores" => cfg.cpu_cores = val.parse().unwrap_or(1),
                "disk" => cfg.disk_image = val.to_string(),
                "iso" => cfg.iso_image = val.to_string(),
                "boot" => cfg.boot_order = match val {
                    "disk" => BootOrder::DiskFirst,
                    "floppy" => BootOrder::FloppyFirst,
                    _ => BootOrder::CdFirst,
                },
                "bios" => cfg.bios_type = match val {
                    "corevm" => BiosType::CoreVm,
                    _ => BiosType::SeaBios,
                },
                "jit" => cfg.jit_enabled = val == "1",
                "ram_alloc" => cfg.ram_alloc = match val {
                    "preallocate" => RamAlloc::Preallocate,
                    _ => RamAlloc::OnDemand,
                },
                "gpu" => cfg.gpu_type = val.to_string(),
                "net_enabled" => cfg.net_enabled = val == "1",
                "net_mode" => cfg.net_mode = match val {
                    "bridge" => NetMode::Bridge,
                    _ => NetMode::Nat,
                },
                "net_host_nic" => cfg.net_host_nic = val.to_string(),
                "mac_mode" => cfg.mac_mode = match val {
                    "static" => MacMode::Static,
                    _ => MacMode::Dynamic,
                },
                "mac_address" => cfg.mac_address = val.to_string(),
                _ => {}
            }
        }
        Ok(cfg)
    }

    pub fn config_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.conf", self.uuid))
    }
}
```

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/config.rs
git commit -m "feat(vmmanager): VM config load/save (anyOS-compatible format)"
```

---

### Task 5: Sidebar Layout (VM List with Folders)

**Files:**
- Create: `corevm/vmmanager/src/sidebar.rs`

**Step 1: Create sidebar.rs with folder/VM tree, layout persistence**

Layout format (compatible with anyOS):
```
folder:Name
vm:<uuid>
end
vm:<uuid>
```

The sidebar should render:
- Folder headers (collapsible)
- VM entries with status icon (green=running, gray=stopped, orange=paused)
- Right-click context menu (New VM, New Folder, Delete, Clone)
- Selected VM highlighted with accent color

Implementation: TreeView using `egui::CollapsingHeader` for folders, selectable labels for VMs.

The struct `SidebarLayout` holds folder entries and root VMs. Persisted in `layout.conf`.

Full code: ~200 lines covering `SidebarLayout`, `FolderEntry`, `load_layout`, `save_layout`, `render_sidebar` function.

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/sidebar.rs
git commit -m "feat(vmmanager): sidebar with folder tree and VM list"
```

---

### Task 6: App State & Main UI Routing

**Files:**
- Create: `corevm/vmmanager/src/app.rs`
- Modify: `corevm/vmmanager/src/main.rs`

**Step 1: Create app.rs with AppState**

AppState holds:
- `vms: Vec<VmEntry>` — all known VMs with config + runtime state
- `layout: SidebarLayout` — sidebar folder structure
- `selected_vm: Option<String>` — UUID of selected VM
- `show_settings: bool`, `show_create_vm: bool`, `show_create_disk: bool` — dialog states

VmEntry holds:
- `config: VmConfig`
- `state: VmState` (Stopped/Running/Paused)
- `vm_handle: Option<u64>` — libcorevm handle
- `framebuffer: Arc<Mutex<FrameBufferData>>` — shared with VM thread
- `vm_thread: Option<JoinHandle<()>>`
- `instruction_count: u64`
- `mips: f64`

FrameBufferData:
- `pixels: Vec<u8>` (RGBA32)
- `width: u32`, `height: u32`
- `text_mode: bool`
- `text_buffer: Vec<u16>` (80x25 cells for text mode)
- `dirty: bool`

**Step 2: Implement top-level UI layout in `update()`**

```
- Menu bar (File > New VM, Open Config Dir, Quit | VM > Start, Stop, Pause, Settings | Help > About)
- Toolbar panel (top)
- Sidebar panel (left, fixed 220px)
- Central panel: if VM running -> display widget, else -> summary panel
- Status bar (bottom)
```

**Step 3: Wire into main.rs**

Replace inline App with `mod app; use app::CoreVmApp;`, load VMs from config dir on startup.

**Step 4: Commit**

```bash
git add corevm/vmmanager/src/app.rs corevm/vmmanager/src/main.rs
git commit -m "feat(vmmanager): app state and main UI layout"
```

---

### Task 7: Toolbar

**Files:**
- Create: `corevm/vmmanager/src/toolbar.rs`

**Step 1: Create toolbar with Start/Stop/Pause/Settings/Snapshot buttons**

Buttons are enabled/disabled based on selected VM state:
- Start: enabled when VM selected and stopped
- Pause: enabled when running
- Stop: enabled when running or paused
- Settings: enabled when stopped
- Snapshot: enabled always (placeholder)

Render as horizontal button bar with icons (Unicode symbols initially: ▶ ⏸ ⏹ ⚙ 📷).
Background: `TOOLBAR_BG` color. Buttons use accent color when active.

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/toolbar.rs
git commit -m "feat(vmmanager): toolbar with VM control buttons"
```

---

### Task 8: Status Bar

**Files:**
- Create: `corevm/vmmanager/src/statusbar.rs`

**Step 1: Create status bar showing VM runtime info**

Bottom panel with `STATUSBAR_BG` background (blue accent). Shows:
- VM state (Running/Stopped/Paused)
- MIPS (millions of instructions per second)
- IPC (instructions per corevm_run call)
- CPU mode (Real/Protected/Long)
- JIT stats (blocks compiled, hit rate)

When no VM running: "Ready" or "No VM selected".

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/statusbar.rs
git commit -m "feat(vmmanager): status bar with runtime metrics"
```

---

### Task 9: VM Lifecycle (create/start/stop) with libcorevm

**Files:**
- Create: `corevm/vmmanager/src/vm.rs`

**Step 1: Create vm.rs with VM lifecycle management**

This is the core integration with libcorevm. Reference `test_vmd.rs` and `test_vmd_x11.rs` for the API usage pattern.

Key functions:

`start_vm(entry: &mut VmEntry)`:
1. `corevm_create_ex(ram_mb, cores)` → handle
2. `corevm_setup_standard_devices(handle)`
3. `corevm_setup_pci_bus(handle)`
4. `corevm_setup_ide(handle)`
5. Load BIOS based on config (SeaBIOS or CoreVM BIOS)
6. If ISO: `corevm_ide_attach_slave(handle, iso_ptr, iso_len)`
7. If disk: `corevm_ide_attach_disk(handle, disk_ptr, disk_len)` or `_fd` variant
8. If JIT: `corevm_jit_enable(handle, 1)`
9. Spawn thread that loops `corevm_run(handle, 500_000)` + IDE IRQ polling
10. Thread updates `FrameBufferData` via `corevm_vga_get_framebuffer` / `corevm_vga_get_text_buffer`

`stop_vm(entry: &mut VmEntry)`:
1. Set stop flag (`AtomicBool`)
2. Join thread
3. `corevm_destroy(handle)`

`pause_vm` / `resume_vm`:
1. Toggle pause flag checked by VM thread

The VM thread:
- Runs in a loop calling `corevm_run` with batch size ~500K instructions
- After each batch: poll IDE IRQ, update framebuffer, update metrics
- Checks stop/pause flags
- On ExitReason::Halt or exception: update state, stop loop

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/vm.rs
git commit -m "feat(vmmanager): VM lifecycle with libcorevm integration"
```

---

### Task 10: Framebuffer Display

**Files:**
- Create: `corevm/vmmanager/src/display.rs`

**Step 1: Create display.rs with framebuffer → egui texture rendering**

Two rendering paths:

**Text mode** (when `text_mode == true`):
- Read 80x25 u16 cells from `text_buffer`
- Render each character into an 8x16 pixel RGBA buffer (640x400 pixels)
- Use standard VGA font (embedded 8x16 bitmap font, ~4KB table)
- Apply VGA 16-color palette for foreground/background from attribute byte
- Upload as egui `TextureHandle`

**Graphics mode** (framebuffer):
- Convert from source BPP to RGBA32:
  - 4bpp: index into 16-color palette
  - 8bpp: index into 256-color palette (or grayscale)
  - 16bpp: RGB565 → RGBA
  - 24bpp: BGR → RGBA
  - 32bpp: BGRA → RGBA
- Upload as egui `TextureHandle`
- Scale to fit central panel maintaining aspect ratio

**Dirty flag**: Only re-upload texture when `dirty == true`, then clear flag.

The display widget should:
- Fill the central panel
- Show black background with centered, aspect-ratio-preserved VM display
- Request focus when clicked (for keyboard capture)

Reference `test_vmd_x11.rs` `blit_fb_to_bgra()` for BPP conversion logic.

**Step 2: Embed VGA 8x16 font**

Include a standard VGA font as `const VGA_FONT: &[u8; 4096] = include_bytes!("../assets/vga_font.bin");` or hardcode the standard CP437 8x16 font table.

**Step 3: Commit**

```bash
git add corevm/vmmanager/src/display.rs
git commit -m "feat(vmmanager): framebuffer display with text and graphics mode"
```

---

### Task 11: Keyboard Input

**Files:**
- Create: `corevm/vmmanager/src/input.rs`

**Step 1: Create input.rs with egui key → PS/2 scancode mapping**

Map `egui::Key` variants to PS/2 scancode set 2 values. Also handle raw character input for printable ASCII.

Reference `test_vmd_x11.rs` scancode tables for the mapping.

Key mapping table (scancode set 2):
- A-Z: 0x1C, 0x32, 0x21, 0x23, 0x24, 0x2B, 0x34, 0x33, 0x43, 0x3B, 0x42, 0x4B, 0x3A, 0x31, 0x44, 0x4D, 0x15, 0x2D, 0x1B, 0x2C, 0x3C, 0x2A, 0x1D, 0x22, 0x35, 0x1A
- 0-9: 0x45, 0x16, 0x1E, 0x26, 0x25, 0x2E, 0x36, 0x3D, 0x3E, 0x46
- Enter: 0x5A, Escape: 0x76, Backspace: 0x66, Tab: 0x0D, Space: 0x29
- Arrow keys (E0 prefix): Left 0x6B, Right 0x74, Up 0x75, Down 0x72

Functions:
- `handle_key_event(key: egui::Key, pressed: bool, vm_handle: u64)` — translates and sends via `corevm_ps2_key_press/release`
- `scancode_for_key(key: egui::Key) -> Option<u8>` — lookup table

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/input.rs
git commit -m "feat(vmmanager): keyboard input with PS/2 scancode mapping"
```

---

### Task 12: Mouse Input

**Files:**
- Modify: `corevm/vmmanager/src/input.rs`

**Step 1: Add mouse handling to input.rs**

When display widget has focus:
- Track mouse position delta between frames
- Convert to relative PS/2 mouse movement
- Send via `corevm_ps2_mouse_move(handle, dx, dy, buttons)`
- Mouse capture: click on display to capture, Ctrl+Alt to release
- While captured: hide cursor, accumulate deltas

Button mapping:
- Left click: button bit 0
- Right click: button bit 1
- Middle click: button bit 2

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/input.rs
git commit -m "feat(vmmanager): mouse input with capture mode"
```

---

### Task 13: Settings Dialog

**Files:**
- Create: `corevm/vmmanager/src/settings.rs`

**Step 1: Create settings dialog with tabbed interface**

Modal window with three tabs:

**General Tab:**
- Name: text field
- RAM: slider (16 MB to 8192 MB, step 16)
- CPU Cores: segmented control (1, 2, 4, 8, 16)
- RAM Allocation: radio (Preallocate / On Demand)
- BIOS: radio (CoreVM / SeaBIOS)
- JIT: checkbox

**Devices Tab:**
- GPU: dropdown (SVGA Framebuffer) — currently only one option
- Network: checkbox enable
- Network Mode: radio (NAT / Bridge) — shown when enabled
- Bridge NIC: text field — shown when Bridge selected
- MAC Mode: radio (Dynamic / Static)
- MAC Address: text field — shown when Static selected

**Boot Tab:**
- Boot Order: radio (Disk First / CD First / Floppy First)
- Disk Image: text field + Browse button (rfd file dialog)
- ISO Image: text field + Browse button
- Create Disk button → opens create disk dialog

Footer: Save / Cancel buttons. Save writes config and closes dialog.

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/settings.rs
git commit -m "feat(vmmanager): settings dialog with tabs"
```

---

### Task 14: Dialogs (Create VM, Create Disk, About)

**Files:**
- Create: `corevm/vmmanager/src/dialogs.rs`

**Step 1: Create dialogs**

**Create VM Dialog:**
- Name field (default: "New VM")
- RAM dropdown (64, 128, 256, 512, 1024, 2048, 4096)
- OK creates VmConfig with defaults, saves, adds to layout

**Create Disk Dialog:**
- Path field + Browse button
- Size field (MB) with presets (512, 1024, 2048, 4096, 8192, 16384)
- Create button: creates raw file filled with zeros (`File::set_len(size * 1024 * 1024)`)

**About Dialog:**
- "CoreVM Manager v0.1.0"
- "Cross-platform x86 Virtual Machine Manager"
- "Powered by libcorevm"

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/dialogs.rs
git commit -m "feat(vmmanager): create VM, create disk, and about dialogs"
```

---

### Task 15: Summary Panel (VM Details when not running)

**Files:**
- Modify: `corevm/vmmanager/src/app.rs`

**Step 1: Add summary panel to central area**

When a VM is selected but not running, show a VirtualBox-style summary:
- VM name (large heading)
- Grid of key-value pairs:
  - Status: Stopped / Running / Paused (colored)
  - RAM: 512 MB
  - CPU Cores: 4
  - BIOS: SeaBIOS
  - JIT: Enabled / Disabled
  - Disk: /path/to/disk.img (or "None")
  - ISO: /path/to/iso.iso (or "None")
  - Boot Order: CD First
  - Network: NAT / Bridge / Disabled
- "Start" button (large, centered, accent color)
- "Settings" button

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/app.rs
git commit -m "feat(vmmanager): VM summary panel with details view"
```

---

### Task 16: Menu Bar

**Files:**
- Modify: `corevm/vmmanager/src/app.rs`

**Step 1: Add menu bar**

```
File:
  New VM...          (opens create VM dialog)
  Open Config Dir    (opens file manager at config_dir)
  Quit               (exits app)

VM:
  Start              (start selected VM)
  Pause              (pause selected VM)
  Stop               (stop selected VM)
  Settings...        (open settings for selected VM)

View:
  Fullscreen         (toggle fullscreen — egui viewport)

Help:
  About CoreVM...    (about dialog)
```

Menu items enabled/disabled based on selection and VM state.

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/app.rs
git commit -m "feat(vmmanager): menu bar with File/VM/View/Help"
```

---

### Task 17: Snapshots UI (Placeholder)

**Files:**
- Modify: `corevm/vmmanager/src/dialogs.rs`

**Step 1: Add snapshots dialog**

Simple list view with:
- "Take Snapshot" button (shows "not yet implemented" toast)
- "Restore" button (disabled)
- "Delete" button (disabled)
- Empty list placeholder: "No snapshots yet"

This is a UI placeholder for future snapshot backend support.

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/dialogs.rs
git commit -m "feat(vmmanager): snapshots UI placeholder"
```

---

### Task 18: Network Config UI

**Files:**
- Modify: `corevm/vmmanager/src/settings.rs`

**Step 1: Enhance network section in Devices tab**

Already partially covered in Task 13. Add:
- Visual network adapter card with "Intel E1000" label
- Connection status indicator
- MAC address auto-generation button (generates random 02:xx:xx:xx:xx:xx)

**Step 2: Commit**

```bash
git add corevm/vmmanager/src/settings.rs
git commit -m "feat(vmmanager): enhanced network configuration UI"
```

---

### Task 19: Wire Everything Together & Polish

**Files:**
- Modify: `corevm/vmmanager/src/main.rs`
- Modify: `corevm/vmmanager/src/app.rs`

**Step 1: Ensure all modules are wired**

All `mod` declarations in main.rs:
```rust
mod app;
mod config;
mod dialogs;
mod display;
mod input;
mod platform;
mod settings;
mod sidebar;
mod statusbar;
mod theme;
mod toolbar;
mod vm;
```

**Step 2: Integration test — full flow**

Manual test:
1. Launch app
2. Create new VM
3. Set RAM, CPU cores
4. Attach an ISO (e.g., a small Linux live CD)
5. Start VM → should show BIOS boot in text mode
6. Keyboard input works
7. Stop VM
8. Close and reopen → VM persists in sidebar

**Step 3: Final commit**

```bash
git add corevm/
git commit -m "feat(vmmanager): wire all modules and polish UI"
```

---

### Task 20: Linux Desktop Entry & Windows Resource File

**Files:**
- Create: `corevm/vmmanager/linux/corevm-manager.desktop`
- Create: `corevm/vmmanager/win64/app.rc`

**Step 1: Linux .desktop file**

```ini
[Desktop Entry]
Name=CoreVM Manager
Comment=Cross-platform x86 Virtual Machine Manager
Exec=corevm-vmmanager
Icon=corevm-manager
Type=Application
Categories=System;Emulator;
```

**Step 2: Windows resource file**

```rc
1 ICON "corevm.ico"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904E4"
    BEGIN
      VALUE "ProductName", "CoreVM Manager"
      VALUE "FileDescription", "Cross-platform x86 Virtual Machine Manager"
      VALUE "FileVersion", "0.1.0"
      VALUE "ProductVersion", "0.1.0"
    END
  END
END
```

**Step 3: Commit**

```bash
git add corevm/vmmanager/linux/ corevm/vmmanager/win64/
git commit -m "feat(vmmanager): Linux desktop entry and Windows resources"
```
