//! VM Manager — VMware Workstation-style virtual machine manager for anyOS.
//!
//! Provides a graphical interface for creating, configuring, and running x86
//! virtual machines powered by libcorevm. Features include:
//! - VM list sidebar with status indicators
//! - Live VGA framebuffer display on a Canvas control
//! - Real-time CPU/memory monitoring
//! - Settings dialog for editing VM configurations
//! - Keyboard and mouse forwarding to the guest OS
//!
//! VM configurations are persisted in `/System/shared/vmmanager/vms.conf`.

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::ipc;
use libanyui_client as anyui;
use libanyui_client::Widget;

anyos_std::entry!(main);

// ── Constants ──────────────────────────────────────────────────────────

/// Main window dimensions.
const WIN_W: u32 = 900;
const WIN_H: u32 = 640;

/// Sidebar width in pixels.
const SIDEBAR_W: u32 = 200;

/// Height of the compact info bar below the VGA canvas.
const INFO_BAR_H: u32 = 28;

/// SHM header size in bytes (must match vmd).
const SHM_HEADER: usize = 64;
/// VMD SHM size (must match `bin/vmd/src/main.rs`).
const SHM_SIZE: usize = 4 * 1024 * 1024;

/// Maximum number of VMs that can be configured.
const MAX_VMS: usize = 16;

/// Directory containing per-VM config files (`<uuid>.conf`).
const VMS_DIR: &str = "/System/shared/vmmanager/vms";

/// Path to the sidebar layout file (folders + VM ordering).
const LAYOUT_PATH: &str = "/System/shared/vmmanager/layout.conf";

/// Standard VGA 16-color palette (ARGB format).
const VGA_COLORS: [u32; 16] = [
    0xFF000000, // 0: black
    0xFF0000AA, // 1: blue
    0xFF00AA00, // 2: green
    0xFF00AAAA, // 3: cyan
    0xFFAA0000, // 4: red
    0xFFAA00AA, // 5: magenta
    0xFFAA5500, // 6: brown
    0xFFAAAAAA, // 7: light gray
    0xFF555555, // 8: dark gray
    0xFF5555FF, // 9: light blue
    0xFF55FF55, // 10: light green
    0xFF55FFFF, // 11: light cyan
    0xFFFF5555, // 12: light red
    0xFFFF55FF, // 13: light magenta
    0xFFFFFF55, // 14: yellow
    0xFFFFFFFF, // 15: white
];

// ── Data model ─────────────────────────────────────────────────────────

/// Boot device priority ordering.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BootOrder {
    DiskFirst,
    CdFirst,
    FloppyFirst,
}

/// BIOS firmware type for the VM.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BiosType {
    /// Custom CoreVM BIOS (64 KB, loaded at 0xF0000).
    CoreVm,
    /// SeaBIOS open-source PC BIOS (256 KB, loaded at 0xC0000).
    SeaBios,
}

/// GPU type for the VM.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GpuType {
    /// SVGA framebuffer (VGA/Bochs VBE).
    SvgaFb,
}

/// Network adapter mode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NetMode {
    /// Network Address Translation (guest behind NAT).
    Nat,
    /// Bridged — guest appears on the host network directly.
    Bridge,
}

/// MAC address assignment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MacMode {
    /// Automatically generated MAC address.
    Dynamic,
    /// User-specified static MAC address.
    Static,
}

/// RAM allocation strategy.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RamAlloc {
    /// Allocate all guest RAM upfront at VM creation.
    Preallocate,
    /// Allocate guest RAM pages on first access (demand paging).
    OnDemand,
}

/// Persistent configuration for a single VM.
#[derive(Clone)]
struct VmConfig {
    /// Unique identifier (32-char hex) for IPC lookups by vmd.
    uuid: String,
    /// Human-readable name displayed in the sidebar.
    name: String,
    /// Guest RAM size in megabytes.
    ram_mb: u32,
    /// Number of virtual CPU cores to expose to the guest.
    cpu_cores: u8,
    /// RAM allocation strategy.
    ram_alloc: RamAlloc,
    /// Path to a raw disk image file on the host filesystem.
    disk_image: String,
    /// Path to an ISO image file for CD-ROM emulation.
    iso_image: String,
    /// Boot device ordering.
    boot_order: BootOrder,
    /// BIOS firmware type.
    bios_type: BiosType,
    /// GPU type.
    gpu_type: GpuType,
    /// Whether the network adapter is enabled.
    net_enabled: bool,
    /// Network adapter mode (NAT / Bridge).
    net_mode: NetMode,
    /// Host NIC name for bridged mode.
    net_host_nic: String,
    /// MAC address mode (dynamic / static).
    mac_mode: MacMode,
    /// Static MAC address (only used when mac_mode is Static).
    mac_address: String,
}

impl VmConfig {
    /// Create a new VM configuration with defaults and a generated UUID.
    fn new(name: &str) -> Self {
        VmConfig {
            uuid: generate_uuid(),
            name: String::from(name),
            ram_mb: 64,
            cpu_cores: 1,
            ram_alloc: RamAlloc::OnDemand,
            disk_image: String::new(),
            iso_image: String::new(),
            boot_order: BootOrder::DiskFirst,
            bios_type: BiosType::CoreVm,
            gpu_type: GpuType::SvgaFb,
            net_enabled: false,
            net_mode: NetMode::Nat,
            net_host_nic: String::new(),
            mac_mode: MacMode::Dynamic,
            mac_address: String::new(),
        }
    }
}

/// Generate a 32-character lowercase hex UUID from 16 random bytes.
fn generate_uuid() -> String {
    let mut bytes = [0u8; 16];
    anyos_std::sys::random(&mut bytes);
    let hex = b"0123456789abcdef";
    let mut s = String::with_capacity(32);
    for &b in &bytes {
        s.push(hex[(b >> 4) as usize] as char);
        s.push(hex[(b & 0x0F) as usize] as char);
    }
    s
}

/// Runtime state of a single VM.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VmState {
    Stopped,
    Running,
    Paused,
}

/// A configured VM entry combining persistent config with runtime state.
struct VmEntry {
    /// Persistent configuration.
    config: VmConfig,
    /// Current runtime state.
    state: VmState,
    /// TID of the vmd daemon process (0 if not spawned).
    vmd_tid: u32,
    /// Command pipe ID (vmmanager -> vmd).
    cmd_pipe: u32,
    /// Status pipe ID (vmd -> vmmanager).
    status_pipe: u32,
    /// Shared memory ID for VGA framebuffer.
    shm_id: u32,
    /// Mapped SHM base pointer (null if not mapped).
    shm_ptr: *const u8,
}

/// Compact single-line info bar showing VM state, RAM, and mode.
struct VmInfoLabels {
    label: anyui::Label,
}

/// Controls used in the settings dialog window.
struct SettingsDialog {
    win: anyui::Window,
    // ── General tab ──
    name_field: anyui::TextField,
    ram_slider: anyui::Slider,
    ram_value_label: anyui::Label,
    cores_seg: anyui::SegmentedControl,
    ram_alloc_seg: anyui::SegmentedControl,
    bios_seg: anyui::SegmentedControl,
    // ── Devices tab ──
    gpu_seg: anyui::SegmentedControl,
    net_toggle: anyui::Toggle,
    net_mode_seg: anyui::SegmentedControl,
    net_host_field: anyui::TextField,
    mac_mode_seg: anyui::SegmentedControl,
    mac_field: anyui::TextField,
    // ── Boot tab ──
    boot_seg: anyui::SegmentedControl,
    disk_field: anyui::TextField,
    iso_field: anyui::TextField,
}

/// Controls used in the "Create Disk Image" dialog.
struct CreateDiskDialog {
    win: anyui::Window,
    /// Text field displaying/editing the destination path.
    path_field: anyui::TextField,
    /// Text field for the disk size in MB.
    size_field: anyui::TextField,
    /// ID of the `disk_field` in the settings dialog to update after creation.
    disk_field_id: u32,
}

/// A named folder in the sidebar tree containing VM UUIDs.
struct FolderEntry {
    /// Display name of the folder.
    name: String,
    /// UUIDs of VMs in this folder, in display order.
    vm_uuids: Vec<String>,
}

/// Sidebar layout: folders and root-level VMs in display order.
struct SidebarLayout {
    /// Named folders with their VM UUIDs.
    folders: Vec<FolderEntry>,
    /// VM UUIDs at root level (not in any folder).
    root_vms: Vec<String>,
}

impl SidebarLayout {
    /// Create an empty layout.
    fn new() -> Self {
        SidebarLayout {
            folders: Vec::new(),
            root_vms: Vec::new(),
        }
    }
}

/// Identifies what a TreeView node represents.
#[derive(Clone)]
enum NodeKind {
    /// The "My Machines" root node.
    Root,
    /// A folder (index into `SidebarLayout::folders`).
    Folder(usize),
    /// A VM (index into `AppState::vms`).
    Vm(usize),
}

// ── Application state ──────────────────────────────────────────────────

/// Global application state holding all UI controls and VM data.
struct AppState {
    // Main window controls
    win: anyui::Window,
    sidebar: anyui::View,
    canvas: anyui::Canvas,
    toolbar: anyui::Toolbar,
    status_label: anyui::Label,
    info: VmInfoLabels,
    content_view: anyui::View,

    // Sidebar tree view for VM list.
    sidebar_tree: anyui::TreeView,
    /// Index of the "My Machines" root node in the tree.
    tree_root: u32,
    /// Maps TreeView node index → NodeKind for selection handling.
    node_map: Vec<NodeKind>,

    // VM data
    vms: Vec<VmEntry>,
    selected_vm: usize,

    /// Sidebar folder/VM layout.
    layout: SidebarLayout,

    // Settings dialog (created on demand)
    settings: Option<SettingsDialog>,

    // Create Disk dialog (created on demand)
    create_disk_dlg: Option<CreateDiskDialog>,

    // Toolbar action button IDs (for enable/disable updates).
    btn_start_id: u32,
    btn_stop_id: u32,
    btn_settings_id: u32,
    btn_delete_id: u32,
}

static mut APP: Option<AppState> = None;

/// Get a mutable reference to the global application state.
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

/// Return true when the current sidebar selection is an actual VM node.
fn sidebar_selection_is_vm() -> bool {
    let a = app();
    let sel = a.sidebar_tree.selected() as usize;
    matches!(a.node_map.get(sel), Some(NodeKind::Vm(_)))
}

// ── Number formatting (delegates to anyos_std::fmt) ────────────────────

use anyos_std::fmt as stdfmt;

fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    stdfmt::fmt_u32(buf, val)
}

fn fmt_u64<'a>(buf: &'a mut [u8; 20], val: u64) -> &'a str {
    stdfmt::fmt_u64(buf, val)
}

/// Build a label + value string (e.g. "RAM: 64 MB") into `buf`.
fn fmt_label_val<'a>(buf: &'a mut [u8], label: &str, val: u32, suffix: &str) -> &'a str {
    let mut p = 0;
    for &b in label.as_bytes() {
        if p < buf.len() - 1 {
            buf[p] = b;
            p += 1;
        }
    }
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, val);
    for &b in s.as_bytes() {
        if p < buf.len() - 1 {
            buf[p] = b;
            p += 1;
        }
    }
    for &b in suffix.as_bytes() {
        if p < buf.len() - 1 {
            buf[p] = b;
            p += 1;
        }
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

/// Build a label + u64 value string into `buf`.
fn fmt_label_u64<'a>(buf: &'a mut [u8], label: &str, val: u64) -> &'a str {
    let mut p = 0;
    for &b in label.as_bytes() {
        if p < buf.len() - 1 {
            buf[p] = b;
            p += 1;
        }
    }
    let mut tmp = [0u8; 20];
    let s = fmt_u64(&mut tmp, val);
    for &b in s.as_bytes() {
        if p < buf.len() - 1 {
            buf[p] = b;
            p += 1;
        }
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

// ── Configuration persistence ──────────────────────────────────────────
//
// Each VM is stored in its own file: `<VMS_DIR>/<uuid>.conf`.
// The sidebar layout (folders + ordering) is stored in `LAYOUT_PATH`.

/// Ensure the VM storage directories exist.
fn ensure_dirs() {
    fs::mkdir("/System/shared");
    fs::mkdir("/System/shared/vmmanager");
    fs::mkdir(VMS_DIR);
}

/// Save a single VM configuration to `<VMS_DIR>/<uuid>.conf`.
fn save_vm_config(config: &VmConfig) {
    ensure_dirs();
    let mut path_buf = [0u8; 128];
    let path = build_vm_path(&mut path_buf, &config.uuid);

    let mut data: Vec<u8> = Vec::with_capacity(512);
    data.extend_from_slice(b"name=");
    data.extend_from_slice(config.name.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"ram=");
    let mut buf = [0u8; 12];
    let s = fmt_u32(&mut buf, config.ram_mb);
    data.extend_from_slice(s.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"cpu_cores=");
    let s = fmt_u32(&mut buf, config.cpu_cores as u32);
    data.extend_from_slice(s.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"disk=");
    data.extend_from_slice(config.disk_image.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"iso=");
    data.extend_from_slice(config.iso_image.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"boot=");
    let boot_str = match config.boot_order {
        BootOrder::DiskFirst => "disk",
        BootOrder::CdFirst => "cd",
        BootOrder::FloppyFirst => "floppy",
    };
    data.extend_from_slice(boot_str.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"bios=");
    data.extend_from_slice(match config.bios_type {
        BiosType::CoreVm => b"corevm" as &[u8],
        BiosType::SeaBios => b"seabios",
    });
    data.push(b'\n');
    data.extend_from_slice(b"ram_alloc=");
    data.extend_from_slice(match config.ram_alloc {
        RamAlloc::Preallocate => b"prealloc" as &[u8],
        RamAlloc::OnDemand => b"ondemand",
    });
    data.push(b'\n');
    data.extend_from_slice(b"gpu=");
    data.extend_from_slice(match config.gpu_type {
        GpuType::SvgaFb => b"svga" as &[u8],
    });
    data.push(b'\n');
    data.extend_from_slice(b"net_enabled=");
    data.extend_from_slice(if config.net_enabled { b"1" } else { b"0" });
    data.push(b'\n');
    data.extend_from_slice(b"net_mode=");
    data.extend_from_slice(match config.net_mode {
        NetMode::Nat => b"nat" as &[u8],
        NetMode::Bridge => b"bridge",
    });
    data.push(b'\n');
    data.extend_from_slice(b"net_host_nic=");
    data.extend_from_slice(config.net_host_nic.as_bytes());
    data.push(b'\n');
    data.extend_from_slice(b"mac_mode=");
    data.extend_from_slice(match config.mac_mode {
        MacMode::Dynamic => b"dynamic" as &[u8],
        MacMode::Static => b"static",
    });
    data.push(b'\n');
    data.extend_from_slice(b"mac_address=");
    data.extend_from_slice(config.mac_address.as_bytes());
    data.push(b'\n');

    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &data);
        fs::close(fd);
    }
}

/// Delete a VM configuration file.
fn delete_vm_config(uuid: &str) {
    let mut path_buf = [0u8; 128];
    let path = build_vm_path(&mut path_buf, uuid);
    fs::unlink(path);
}

/// Build the file path `<VMS_DIR>/<uuid>.conf` into `buf`.
fn build_vm_path<'a>(buf: &'a mut [u8; 128], uuid: &str) -> &'a str {
    let dir = VMS_DIR.as_bytes();
    let ext = b".conf";
    let uuid_b = uuid.as_bytes();
    let total = dir.len() + 1 + uuid_b.len() + ext.len();
    let mut p = 0;
    for &b in dir {
        buf[p] = b;
        p += 1;
    }
    buf[p] = b'/';
    p += 1;
    for &b in uuid_b {
        if p < 127 {
            buf[p] = b;
            p += 1;
        }
    }
    for &b in ext {
        if p < 127 {
            buf[p] = b;
            p += 1;
        }
    }
    let _ = total; // suppress unused warning
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

/// Load a single VM config by UUID from its `.conf` file.
fn load_vm_config(uuid: &str) -> Option<VmConfig> {
    let mut path_buf = [0u8; 128];
    let path = build_vm_path(&mut path_buf, uuid);

    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut buf = [0u8; 1024];
    let n = fs::read(fd, &mut buf);
    fs::close(fd);
    if n == 0 || n == u32::MAX {
        return None;
    }

    let text = &buf[..n as usize];
    let mut config = VmConfig::new("");
    config.uuid = String::from(uuid);

    for line in ByteLines::new(text) {
        if let Some(val) = strip_prefix(line, b"name=") {
            config.name = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"ram=") {
            config.ram_mb = parse_u32(val).unwrap_or(64);
        } else if let Some(val) = strip_prefix(line, b"cpu_cores=") {
            let cores = parse_u32(val).unwrap_or(1).clamp(1, 64);
            config.cpu_cores = cores as u8;
        } else if let Some(val) = strip_prefix(line, b"disk=") {
            config.disk_image = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"iso=") {
            config.iso_image = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"boot=") {
            config.boot_order = match val {
                b"cd" => BootOrder::CdFirst,
                b"floppy" => BootOrder::FloppyFirst,
                _ => BootOrder::DiskFirst,
            };
        } else if let Some(val) = strip_prefix(line, b"bios=") {
            config.bios_type = match val {
                b"seabios" => BiosType::SeaBios,
                _ => BiosType::CoreVm,
            };
        } else if let Some(val) = strip_prefix(line, b"ram_alloc=") {
            config.ram_alloc = match val {
                b"prealloc" => RamAlloc::Preallocate,
                _ => RamAlloc::OnDemand,
            };
        } else if let Some(val) = strip_prefix(line, b"gpu=") {
            config.gpu_type = match val {
                _ => GpuType::SvgaFb,
            };
        } else if let Some(val) = strip_prefix(line, b"net_enabled=") {
            config.net_enabled = val == b"1";
        } else if let Some(val) = strip_prefix(line, b"net_mode=") {
            config.net_mode = match val {
                b"bridge" => NetMode::Bridge,
                _ => NetMode::Nat,
            };
        } else if let Some(val) = strip_prefix(line, b"net_host_nic=") {
            config.net_host_nic = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"mac_mode=") {
            config.mac_mode = match val {
                b"static" => MacMode::Static,
                _ => MacMode::Dynamic,
            };
        } else if let Some(val) = strip_prefix(line, b"mac_address=") {
            config.mac_address = bytes_to_string(val);
        }
    }

    if config.name.is_empty() {
        return None;
    }
    Some(config)
}

/// Save the sidebar layout (folders + VM ordering) to `LAYOUT_PATH`.
///
/// Format:
/// ```text
/// folder:<name>
/// vm:<uuid>
/// end
/// vm:<uuid>          (root-level VM)
/// ```
fn save_layout(layout: &SidebarLayout) {
    ensure_dirs();
    let mut data: Vec<u8> = Vec::with_capacity(512);

    for folder in &layout.folders {
        data.extend_from_slice(b"folder:");
        data.extend_from_slice(folder.name.as_bytes());
        data.push(b'\n');
        for uuid in &folder.vm_uuids {
            data.extend_from_slice(b"vm:");
            data.extend_from_slice(uuid.as_bytes());
            data.push(b'\n');
        }
        data.extend_from_slice(b"end\n");
    }
    for uuid in &layout.root_vms {
        data.extend_from_slice(b"vm:");
        data.extend_from_slice(uuid.as_bytes());
        data.push(b'\n');
    }

    let fd = fs::open(LAYOUT_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &data);
        fs::close(fd);
    }
}

/// Load the sidebar layout from `LAYOUT_PATH`.
fn load_layout() -> SidebarLayout {
    let mut layout = SidebarLayout::new();
    let fd = fs::open(LAYOUT_PATH, 0);
    if fd == u32::MAX {
        return layout;
    }

    let mut buf = [0u8; 4096];
    let n = fs::read(fd, &mut buf);
    fs::close(fd);
    if n == 0 || n == u32::MAX {
        return layout;
    }

    let text = &buf[..n as usize];
    let mut in_folder = false;

    for line in ByteLines::new(text) {
        if let Some(val) = strip_prefix(line, b"folder:") {
            layout.folders.push(FolderEntry {
                name: bytes_to_string(val),
                vm_uuids: Vec::new(),
            });
            in_folder = true;
        } else if line == b"end" {
            in_folder = false;
        } else if let Some(val) = strip_prefix(line, b"vm:") {
            let uuid = bytes_to_string(val);
            if in_folder {
                if let Some(folder) = layout.folders.last_mut() {
                    folder.vm_uuids.push(uuid);
                }
            } else {
                layout.root_vms.push(uuid);
            }
        }
    }

    layout
}

/// Load all VMs and the sidebar layout from disk.
///
/// VMs referenced in `layout.conf` are loaded in layout order.
/// Any `.conf` files in the VMs directory not referenced in the layout
/// are appended as root-level VMs (handles orphans from crashes).
fn load_all_vms() -> (Vec<VmEntry>, SidebarLayout) {
    let mut layout = load_layout();
    let mut vms: Vec<VmEntry> = Vec::new();
    let mut loaded_uuids: Vec<String> = Vec::new();

    // Helper: load a VM by UUID and append to the list.
    let mut load_uuid = |uuid: &str, vms: &mut Vec<VmEntry>, loaded: &mut Vec<String>| {
        if loaded.iter().any(|u| u == uuid) {
            return; // Already loaded (duplicate in layout).
        }
        if let Some(config) = load_vm_config(uuid) {
            vms.push(VmEntry {
                config,
                state: VmState::Stopped,
                vmd_tid: 0,
                cmd_pipe: 0,
                status_pipe: 0,
                shm_id: 0,
                shm_ptr: core::ptr::null(),
            });
            loaded.push(String::from(uuid));
        }
    };

    // Load VMs in layout order: folders first, then root-level.
    for folder in &layout.folders {
        for uuid in &folder.vm_uuids {
            load_uuid(uuid, &mut vms, &mut loaded_uuids);
        }
    }
    for uuid in &layout.root_vms {
        load_uuid(uuid, &mut vms, &mut loaded_uuids);
    }

    // Scan the VMs directory for orphaned config files.
    let mut dir_buf = [0u8; 4096];
    let count = fs::readdir(VMS_DIR, &mut dir_buf);
    if count != u32::MAX {
        for i in 0..count as usize {
            let entry = &dir_buf[i * 64..(i + 1) * 64];
            let typ = entry[0];
            let name_len = entry[1] as usize;
            if typ != 0 || name_len < 6 {
                continue; // Not a regular file or too short for "x.conf".
            }
            let name = &entry[8..8 + name_len];
            // Must end with ".conf".
            if name.len() < 6 || &name[name.len() - 5..] != b".conf" {
                continue;
            }
            let uuid_bytes = &name[..name.len() - 5];
            if let Ok(uuid_str) = core::str::from_utf8(uuid_bytes) {
                if !loaded_uuids.iter().any(|u| u == uuid_str) {
                    // Orphaned config — load and add to root_vms in layout.
                    if let Some(config) = load_vm_config(uuid_str) {
                        layout.root_vms.push(String::from(uuid_str));
                        vms.push(VmEntry {
                            config,
                            state: VmState::Stopped,
                            vmd_tid: 0,
                            cmd_pipe: 0,
                            status_pipe: 0,
                            shm_id: 0,
                            shm_ptr: core::ptr::null(),
                        });
                    }
                }
            }
        }
    }

    (vms, layout)
}

/// Convenience: save a single VM config and the current layout.
fn save_vm_and_layout(config: &VmConfig, layout: &SidebarLayout) {
    save_vm_config(config);
    save_layout(layout);
}

/// Remove a VM UUID from the layout (folders and root-level list).
fn remove_uuid_from_layout(layout: &mut SidebarLayout, uuid: &str) {
    for folder in &mut layout.folders {
        folder.vm_uuids.retain(|u| u != uuid);
    }
    layout.root_vms.retain(|u| u != uuid);
}

/// Simple line iterator over a byte slice, splitting on `\n`.
struct ByteLines<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteLines<'a> {
    fn new(data: &'a [u8]) -> Self {
        ByteLines { data, pos: 0 }
    }
}

impl<'a> Iterator for ByteLines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        let end = self.pos;
        if self.pos < self.data.len() {
            self.pos += 1; // skip '\n'
        }
        // Trim trailing '\r' for Windows-style line endings.
        let end = if end > start && self.data[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        Some(&self.data[start..end])
    }
}

/// Check if `line` starts with `prefix` and return the remainder.
fn strip_prefix<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if line.len() >= prefix.len() && &line[..prefix.len()] == prefix {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

/// Parse a decimal `u32` from ASCII bytes.
fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for &b in s {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32);
    }
    Some(val)
}

/// Convert a byte slice to an owned `String` (assumes valid ASCII/UTF-8).
fn bytes_to_string(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &byte in b {
        s.push(byte as char);
    }
    s
}

// ── Sidebar rendering ──────────────────────────────────────────────────

/// Rebuild the sidebar tree to reflect the current layout and VM list.
///
/// Builds folder nodes from `layout.folders`, then adds root-level VMs.
/// Populates `node_map` so the selection handler can map node indices
/// back to folders or VMs.
/// Enable or disable toolbar action buttons based on the current selection state.
///
/// Start is enabled only when a stopped VM is selected.  Stop is enabled only
/// when the selected VM is running.  Settings and Delete require any VM to be
/// selected.
fn update_toolbar_buttons() {
    let a = app();
    let has_vm = a.selected_vm < a.vms.len();
    let is_running = has_vm && a.vms[a.selected_vm].state == VmState::Running;

    anyui::Control::from_id(a.btn_start_id).set_enabled(has_vm && !is_running);
    anyui::Control::from_id(a.btn_stop_id).set_enabled(has_vm && is_running);
    anyui::Control::from_id(a.btn_settings_id).set_enabled(has_vm);
    anyui::Control::from_id(a.btn_delete_id).set_enabled(has_vm);
}

fn rebuild_sidebar() {
    let a = app();

    a.sidebar_tree.clear();
    a.node_map.clear();

    let root = a.sidebar_tree.add_root("My Machines");
    a.tree_root = root;
    a.sidebar_tree.set_node_style(root, anyui::STYLE_BOLD);
    a.sidebar_tree.set_expanded(root, true);
    a.node_map.push(NodeKind::Root);

    // Collect layout info into local vecs to avoid borrow conflicts.
    // Each folder: (folder_index, name, [(vm_index)]).
    let mut folder_ops: Vec<(usize, String, Vec<usize>)> = Vec::new();
    for (fi, folder) in a.layout.folders.iter().enumerate() {
        let mut vm_indices = Vec::new();
        for uuid in &folder.vm_uuids {
            if let Some(vi) = a.vms.iter().position(|e| e.config.uuid == *uuid) {
                vm_indices.push(vi);
            }
        }
        folder_ops.push((fi, folder.name.clone(), vm_indices));
    }
    let mut root_vm_indices: Vec<usize> = Vec::new();
    for uuid in &a.layout.root_vms {
        if let Some(vi) = a.vms.iter().position(|e| e.config.uuid == *uuid) {
            root_vm_indices.push(vi);
        }
    }

    // Add folders and their VMs.
    for (fi, fname, vm_indices) in &folder_ops {
        let folder_node = a.sidebar_tree.add_child(root, fname);
        a.sidebar_tree
            .set_node_style(folder_node, anyui::STYLE_BOLD);
        a.sidebar_tree.set_expanded(folder_node, true);
        a.node_map.push(NodeKind::Folder(*fi));

        for &vi in vm_indices {
            add_vm_node(a, folder_node, vi);
        }
    }

    // Add root-level VMs (not in any folder).
    for &vi in &root_vm_indices {
        add_vm_node(a, root, vi);
    }

    update_toolbar_buttons();
}

/// Add a VM node to the tree under `parent` and record it in `node_map`.
fn add_vm_node(a: &mut AppState, parent: u32, vm_idx: usize) -> u32 {
    let entry = &a.vms[vm_idx];
    let node = a.sidebar_tree.add_child(parent, &entry.config.name);

    let color = match entry.state {
        VmState::Running => 0xFF00DD66u32,
        VmState::Paused => 0xFFFFCC00u32,
        VmState::Stopped => 0xFFAABBCCu32,
    };
    a.sidebar_tree.set_node_text_color(node, color);

    if vm_idx == a.selected_vm {
        a.sidebar_tree.set_selected(node);
    }

    a.node_map.push(NodeKind::Vm(vm_idx));
    node
}

// ── Status bar ─────────────────────────────────────────────────────────

/// Update the status bar label with current summary.
fn update_status_bar() {
    let a = app();
    let running_count = a.vms.iter().filter(|e| e.state == VmState::Running).count();

    let mut buf = [0u8; 64];
    let mut p = 0;
    let prefix = b"Ready | ";
    for &b in prefix {
        if p < 63 {
            buf[p] = b;
            p += 1;
        }
    }
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, running_count as u32);
    for &b in s.as_bytes() {
        if p < 63 {
            buf[p] = b;
            p += 1;
        }
    }
    let suffix = b" VM running";
    for &b in suffix {
        if p < 63 {
            buf[p] = b;
            p += 1;
        }
    }
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..p]) };
    a.status_label.set_text(text);
}

// ── VM info panel ──────────────────────────────────────────────────────

/// Update the VM info labels for the currently selected VM.
fn copy_str(buf: &mut [u8], s: &str) -> usize {
    let b = s.as_bytes();
    let n = b.len().min(buf.len());
    buf[..n].copy_from_slice(&b[..n]);
    n
}

fn write_u32(buf: &mut [u8], val: u32) -> usize {
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, val);
    copy_str(buf, s)
}

fn write_u64(buf: &mut [u8], val: u64) -> usize {
    let mut tmp = [0u8; 20];
    let s = fmt_u64(&mut tmp, val);
    copy_str(buf, s)
}

fn update_info_labels() {
    let a = app();

    if a.selected_vm >= a.vms.len() {
        a.info.label.set_text(" No VM selected");
        a.info.label.set_text_color(0xFF666680);
        return;
    }

    let entry = &a.vms[a.selected_vm];

    let state_str = match entry.state {
        VmState::Running => "Running",
        VmState::Paused => "Paused",
        VmState::Stopped => "Stopped",
    };
    let color = match entry.state {
        VmState::Running => 0xFF88CCAA,
        VmState::Paused => 0xFFCCBB88,
        VmState::Stopped => 0xFF666680,
    };

    // Build compact info string: " State · RAM: 64 MB · Mode: x86"
    let mut buf = [0u8; 160];
    let mut pos = 0;
    // State
    pos += copy_str(&mut buf[pos..], " ");
    pos += copy_str(&mut buf[pos..], state_str);
    // RAM
    pos += copy_str(&mut buf[pos..], "  |RAM: ");
    pos += write_u32(&mut buf[pos..], entry.config.ram_mb);
    pos += copy_str(&mut buf[pos..], " MB");
    // Mode
    if entry.state == VmState::Running {
        pos += copy_str(&mut buf[pos..], "  |x86 (vmd)");
    }
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        a.info.label.set_text(s);
    }
    a.info.label.set_text_color(color);
}

// ── VM lifecycle operations ────────────────────────────────────────────

/// Start the selected VM.
///
/// Spawns a vmd daemon process, creates IPC pipes, sends configuration
/// commands, and begins execution. The VGA framebuffer is shared via SHM.
fn start_selected_vm() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        return;
    }
    if a.vms[a.selected_vm].state != VmState::Stopped {
        return;
    }

    // Persist config before IPC so vmd reads the latest state.
    save_vm_config(&a.vms[a.selected_vm].config);

    let entry = &mut a.vms[a.selected_vm];

    anyos_std::println!("vmmanager: starting VM '{}'", entry.config.name);

    // Create IPC pipes before spawning vmd.
    let cmd_pipe = ipc::pipe_create("vmd_cmd");
    let status_pipe = ipc::pipe_create("vmd_status");

    if cmd_pipe == 0 || status_pipe == 0 {
        anyos_std::println!("vmmanager: failed to create IPC pipes");
        a.status_label.set_text("Error: failed to create IPC pipes");
        a.status_label.set_text_color(0xFFFF4040);
        return;
    }

    // Spawn the vmd daemon.
    let vmd_tid = anyos_std::process::spawn("/System/bin/vmd", "");
    if vmd_tid == u32::MAX {
        anyos_std::println!("vmmanager: failed to spawn vmd");
        a.status_label.set_text("Error: failed to spawn vmd");
        a.status_label.set_text_color(0xFFFF4040);
        ipc::pipe_close(cmd_pipe);
        ipc::pipe_close(status_pipe);
        return;
    }

    entry.cmd_pipe = cmd_pipe;
    entry.status_pipe = status_pipe;
    entry.vmd_tid = vmd_tid;

    // Wait for vmd to signal "ready" before sending commands (up to 5 seconds).
    let mut resp_buf = [0u8; 256];
    let mut vmd_ready = false;
    for _ in 0..100 {
        anyos_std::process::sleep(50);
        let n = ipc::pipe_read(status_pipe, &mut resp_buf);
        if n == 0 || n == u32::MAX {
            continue;
        }
        let resp = core::str::from_utf8(&resp_buf[..n as usize]).unwrap_or("");
        anyos_std::println!("vmmanager: pipe_read got {} bytes: '{}'", n, resp);
        if resp.split('\n').any(|l| l.trim() == "ready") {
            vmd_ready = true;
            anyos_std::println!("vmmanager: vmd is ready");
            break;
        }
    }
    if !vmd_ready {
        anyos_std::println!("vmmanager: WARNING: vmd did not become ready (100 polls exhausted)");
        a.status_label.set_text("Error: vmd timeout");
        a.status_label.set_text_color(0xFFFF4040);
        return;
    }

    // Send VM creation command — vmd reads full config by UUID.
    let create_cmd = format!("create {}", entry.config.uuid);
    anyos_std::println!("vmmanager: sending create command: '{}'", create_cmd);
    ipc::pipe_write(cmd_pipe, create_cmd.as_bytes());

    // Poll for "created" response with SHM ID (up to 5 seconds).
    let mut got_created = false;
    for _ in 0..100 {
        anyos_std::process::sleep(50);
        let n = ipc::pipe_read(status_pipe, &mut resp_buf);
        if n == 0 || n == u32::MAX {
            continue;
        }
        let resp = core::str::from_utf8(&resp_buf[..n as usize]).unwrap_or("");
        for line in resp.split('\n') {
            let line = line.trim();
            if line.starts_with("created") {
                parse_created_response(entry, line);
                got_created = true;
            }
        }
        if got_created {
            break;
        }
    }
    if !got_created {
        anyos_std::println!("vmmanager: WARNING: did not receive 'created' response from vmd");
    }

    // Start execution (disk and ISO are loaded by vmd from config).
    ipc::pipe_write(cmd_pipe, b"start");
    entry.state = VmState::Running;

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();
    update_toolbar_buttons();
}

/// Parse a "created <vm_id> <shm_id>" response and map SHM.
fn parse_created_response(entry: &mut VmEntry, resp: &str) {
    // Expected: "created 0 <shm_id>"
    if resp.starts_with("created") {
        let parts: Vec<&str> = resp.split(' ').collect();
        if parts.len() >= 3 {
            let shm_id = parse_u32_simple(parts[2]);
            if shm_id != 0 {
                entry.shm_id = shm_id;
                let addr = ipc::shm_map(shm_id);
                if addr != 0 {
                    entry.shm_ptr = addr as *const u8;
                    anyos_std::println!("vmmanager: SHM mapped (id={}, addr=0x{:X})", shm_id, addr);
                }
            }
        }
    }
}

/// Simple decimal parser for no_std.
fn parse_u32_simple(s: &str) -> u32 {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        }
    }
    val
}

/// Stop the selected VM.
fn stop_selected_vm() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        return;
    }

    let entry = &mut a.vms[a.selected_vm];
    if entry.state == VmState::Stopped {
        return;
    }

    // Send stop and quit commands to vmd.
    if entry.cmd_pipe != 0 {
        ipc::pipe_write(entry.cmd_pipe, b"stop");
        anyos_std::process::sleep(10);
        ipc::pipe_write(entry.cmd_pipe, b"quit");
    }

    // Cleanup IPC resources.
    cleanup_vm_ipc(entry);

    entry.state = VmState::Stopped;

    // Clear the canvas to show the VM is off.
    a.canvas.clear(0xFF1E1E1E);

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();
    update_toolbar_buttons();
}

/// Clean up IPC resources for a VM entry.
fn cleanup_vm_ipc(entry: &mut VmEntry) {
    if entry.shm_id != 0 {
        ipc::shm_unmap(entry.shm_id);
        entry.shm_id = 0;
    }
    entry.shm_ptr = core::ptr::null();
    if entry.cmd_pipe != 0 {
        ipc::pipe_close(entry.cmd_pipe);
        entry.cmd_pipe = 0;
    }
    if entry.status_pipe != 0 {
        ipc::pipe_close(entry.status_pipe);
        entry.status_pipe = 0;
    }
    entry.vmd_tid = 0;
}

/// Delete the selected VM (must be stopped first).
fn delete_selected_vm() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        return;
    }
    if a.vms[a.selected_vm].state != VmState::Stopped {
        return;
    }

    // Delete the config file and remove from layout.
    let uuid = a.vms[a.selected_vm].config.uuid.clone();
    delete_vm_config(&uuid);
    remove_uuid_from_layout(&mut a.layout, &uuid);
    save_layout(&a.layout);

    a.vms.remove(a.selected_vm);
    if a.selected_vm > 0 && a.selected_vm >= a.vms.len() {
        a.selected_vm = a.vms.len().saturating_sub(1);
    }

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();

    // Clear canvas since the VM is removed.
    a.canvas.clear(0xFF1E1E1E);
}

// ── VGA framebuffer rendering ──────────────────────────────────────────

/// Render the selected VM's VGA text mode buffer to the canvas.
///
/// Each character cell is drawn as an 8x16 pixel block using a minimal
/// built-in 8x8 bitmap font centered within the cell.
fn render_text_mode(canvas: &anyui::Canvas, text_buf: &[u16]) {
    let cols: usize = 80;
    let rows: usize = 25;
    let native_w: u32 = 640; // 80 * 8
    let native_h: u32 = 400; // 25 * 16

    let cw = canvas.get_stride();
    let ch = canvas.get_height();
    if cw == 0 || ch == 0 {
        return;
    }

    // Aspect-ratio-preserving scale.
    let scale_x = cw as f32 / native_w as f32;
    let scale_y = ch as f32 / native_h as f32;
    let scale = if scale_x < scale_y { scale_x } else { scale_y };
    let rw = (native_w as f32 * scale) as u32;
    let rh = (native_h as f32 * scale) as u32;
    let ox = ((cw - rw) / 2) as i32;
    let oy = ((ch - rh) / 2) as i32;

    let char_w = (8.0 * scale) as i32;
    let char_h = (16.0 * scale) as i32;
    if char_w < 1 || char_h < 1 {
        return;
    }

    for row in 0..rows {
        for col in 0..cols {
            let idx = row * cols + col;
            if idx >= text_buf.len() {
                break;
            }
            let entry = text_buf[idx];
            let c = (entry & 0xFF) as u8;
            let attr = (entry >> 8) as u8;
            let fg_idx = (attr & 0x0F) as usize;
            let bg_idx = ((attr >> 4) & 0x07) as usize;
            let fg = VGA_COLORS[fg_idx];
            let bg = VGA_COLORS[bg_idx];

            let x = ox + (col as i32) * char_w;
            let y = oy + (row as i32) * char_h;

            canvas.fill_rect(x, y, char_w as u32, char_h as u32, bg);

            if c > 0x20 && c < 0x7F {
                render_char_scaled(canvas, x, y, c, fg, scale);
            }
        }
    }
}

/// Render a single ASCII character as pixel blocks on the canvas.
///
/// Uses a minimal built-in 8x8 bitmap font. The character is rendered
/// offset 4 pixels down to center within the 8x16 cell.
fn render_char_pixels(canvas: &anyui::Canvas, x: i32, y: i32, ch: u8, color: u32) {
    let glyph = get_glyph(ch);
    for row in 0..8 {
        let bits = glyph[row];
        for col in 0..8 {
            if bits & (0x80 >> col) != 0 {
                canvas.set_pixel(x + col, y + row as i32 + 4, color);
            }
        }
    }
}

/// Render a character glyph scaled by a factor. Each glyph pixel becomes a
/// filled rectangle of size (scale × scale).
fn render_char_scaled(canvas: &anyui::Canvas, x: i32, y: i32, ch: u8, color: u32, scale: f32) {
    let glyph = get_glyph(ch);
    let pw = (scale) as i32; // pixel width
    let ph = (scale) as i32; // pixel height
    if pw < 1 || ph < 1 {
        return;
    }
    let glyph_y_off = (4.0 * scale) as i32; // center 8x8 in 8x16 cell
    for row in 0..8u32 {
        let bits = glyph[row as usize];
        if bits == 0 {
            continue;
        }
        let ry = y + glyph_y_off + (row as f32 * scale) as i32;
        for col in 0..8u32 {
            if bits & (0x80 >> col) != 0 {
                let rx = x + (col as f32 * scale) as i32;
                canvas.fill_rect(rx, ry, pw as u32, ph as u32, color);
            }
        }
    }
}

/// Get a minimal 8x8 glyph bitmap for a printable ASCII character.
///
/// Returns an 8-element array where each element is a bitmask for one
/// row of 8 pixels (MSB = leftmost pixel).
fn get_glyph(ch: u8) -> [u8; 8] {
    match ch {
        b'A' => [0x18, 0x3C, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        b'B' => [0x7C, 0x66, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00],
        b'C' => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00],
        b'D' => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00],
        b'E' => [0x7E, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x7E, 0x00],
        b'F' => [0x7E, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x60, 0x00],
        b'G' => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00],
        b'H' => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        b'I' => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'J' => [0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00],
        b'K' => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00],
        b'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00],
        b'M' => [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0x00],
        b'N' => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00],
        b'O' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'P' => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00],
        b'Q' => [0x3C, 0x66, 0x66, 0x66, 0x6A, 0x6C, 0x36, 0x00],
        b'R' => [0x7C, 0x66, 0x66, 0x7C, 0x6C, 0x66, 0x66, 0x00],
        b'S' => [0x3C, 0x66, 0x70, 0x3C, 0x0E, 0x66, 0x3C, 0x00],
        b'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'W' => [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00],
        b'X' => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00],
        b'Y' => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00],
        b'Z' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        b'a' => [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00],
        b'b' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00],
        b'c' => [0x00, 0x00, 0x3C, 0x60, 0x60, 0x60, 0x3C, 0x00],
        b'd' => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'e' => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00],
        b'f' => [0x1C, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x00],
        b'g' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        b'h' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'i' => [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'j' => [0x0C, 0x00, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38],
        b'k' => [0x60, 0x60, 0x6C, 0x78, 0x78, 0x6C, 0x66, 0x00],
        b'l' => [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'm' => [0x00, 0x00, 0x76, 0x7F, 0x6B, 0x6B, 0x63, 0x00],
        b'n' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'o' => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'p' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60],
        b'q' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06],
        b'r' => [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0x00],
        b's' => [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x00],
        b't' => [0x30, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x1C, 0x00],
        b'u' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'v' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'w' => [0x00, 0x00, 0x63, 0x6B, 0x7F, 0x7F, 0x36, 0x00],
        b'x' => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x00],
        b'y' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        b'z' => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'0' => [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00],
        b'1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        b'2' => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'3' => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00],
        b'4' => [0x0C, 0x1C, 0x3C, 0x6C, 0x7E, 0x0C, 0x0C, 0x00],
        b'5' => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00],
        b'6' => [0x3C, 0x66, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00],
        b'7' => [0x7E, 0x06, 0x0C, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'8' => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00],
        b'9' => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00],
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'!' => [0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x18, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        b',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30],
        b':' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00],
        b';' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x30],
        b'-' => [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        b'+' => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
        b'=' => [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00],
        b'/' => [0x02, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x40, 0x00],
        b'\\' => [0x40, 0x60, 0x30, 0x18, 0x0C, 0x06, 0x02, 0x00],
        b'(' => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00],
        b')' => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00],
        b'[' => [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00],
        b']' => [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00],
        b'{' => [0x0E, 0x18, 0x18, 0x70, 0x18, 0x18, 0x0E, 0x00],
        b'}' => [0x70, 0x18, 0x18, 0x0E, 0x18, 0x18, 0x70, 0x00],
        b'<' => [0x06, 0x0C, 0x18, 0x30, 0x18, 0x0C, 0x06, 0x00],
        b'>' => [0x60, 0x30, 0x18, 0x0C, 0x18, 0x30, 0x60, 0x00],
        b'?' => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x00, 0x18, 0x00],
        b'@' => [0x3C, 0x66, 0x6E, 0x6E, 0x60, 0x62, 0x3C, 0x00],
        b'#' => [0x36, 0x36, 0x7F, 0x36, 0x7F, 0x36, 0x36, 0x00],
        b'$' => [0x18, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x18, 0x00],
        b'%' => [0x62, 0x66, 0x0C, 0x18, 0x30, 0x66, 0x46, 0x00],
        b'&' => [0x38, 0x6C, 0x38, 0x76, 0xDC, 0xCC, 0x76, 0x00],
        b'*' => [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00],
        b'_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00],
        b'~' => [0x00, 0x00, 0x76, 0xDC, 0x00, 0x00, 0x00, 0x00],
        b'^' => [0x18, 0x3C, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'|' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'\'' => [0x18, 0x18, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'"' => [0x66, 0x66, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'`' => [0x30, 0x18, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00],
        // Fallback: filled block for unknown characters.
        _ => [0x7E, 0x7E, 0x7E, 0x7E, 0x7E, 0x7E, 0x7E, 0x00],
    }
}

/// Render the selected VM's VGA graphics framebuffer to the canvas.
///
/// Converts the guest framebuffer pixel data to ARGB and writes directly
/// to the canvas pixel buffer for maximum performance.
fn render_graphics_mode(canvas: &anyui::Canvas, fb: &[u8], width: u32, height: u32, bpp: u8) {
    let buf_ptr = canvas.get_buffer();
    if buf_ptr.is_null() {
        return;
    }
    let stride = canvas.get_stride();
    let cw = canvas.get_stride();
    let ch = canvas.get_height();
    if cw == 0 || ch == 0 {
        return;
    }

    // Aspect-ratio-preserving scale with centering.
    let scale_x = cw as f32 / width as f32;
    let scale_y = ch as f32 / height as f32;
    let scale = if scale_x < scale_y { scale_x } else { scale_y };
    let rw = (width as f32 * scale) as u32;
    let rh = (height as f32 * scale) as u32;
    let ox = (cw - rw) / 2;
    let oy = (ch - rh) / 2;

    let render_w = rw;
    let render_h = rh;

    // Nearest-neighbor scaling: map destination pixel → source pixel.
    let get_color = |sx: u32, sy: u32| -> u32 {
        match bpp {
            32 => {
                let off = ((sy * width + sx) * 4) as usize;
                if off + 3 < fb.len() {
                    let b = fb[off] as u32;
                    let g = fb[off + 1] as u32;
                    let r = fb[off + 2] as u32;
                    0xFF000000 | (r << 16) | (g << 8) | b
                } else {
                    0xFF000000
                }
            }
            24 => {
                let off = ((sy * width + sx) * 3) as usize;
                if off + 2 < fb.len() {
                    let b = fb[off] as u32;
                    let g = fb[off + 1] as u32;
                    let r = fb[off + 2] as u32;
                    0xFF000000 | (r << 16) | (g << 8) | b
                } else {
                    0xFF000000
                }
            }
            8 => {
                let off = (sy * width + sx) as usize;
                if off < fb.len() {
                    let idx = fb[off] as usize;
                    if idx < 16 {
                        VGA_COLORS[idx]
                    } else {
                        let g = (idx as u32) & 0xFF;
                        0xFF000000 | (g << 16) | (g << 8) | g
                    }
                } else {
                    0xFF000000
                }
            }
            4 => {
                // CoreVM exports 4bpp legacy mode as 1 byte per pixel index.
                // Use low nibble as VGA palette entry.
                let off = (sy * width + sx) as usize;
                if off < fb.len() {
                    let idx = (fb[off] & 0x0F) as usize;
                    VGA_COLORS[idx]
                } else {
                    0xFF000000
                }
            }
            _ => 0xFF2D2D2D,
        }
    };

    for dy in 0..render_h {
        let sy = ((dy as f32 / scale) as u32).min(height - 1);
        let dst_y = oy + dy;
        if dst_y >= ch {
            break;
        }
        for dx in 0..render_w {
            let sx = ((dx as f32 / scale) as u32).min(width - 1);
            let dst_x = ox + dx;
            if dst_x >= cw {
                break;
            }
            let color = get_color(sx, sy);
            unsafe {
                let dst = buf_ptr.add((dst_y * stride + dst_x) as usize);
                *dst = color;
            }
        }
    }
}

// ── Keyboard scancode mapping ──────────────────────────────────────────

/// Map an anyui keycode to a PS/2 scancode set 1 make code.
///
/// The compositor sends raw PS/2 scancodes as keycodes for regular keys,
/// and KEY_* constants (>= 0x100) for special keys.  For regular keys
/// the scancode can be forwarded directly; for special keys we map back
/// to the original PS/2 scancode.  Modifier scancodes (Shift, Ctrl, Alt)
/// are forwarded as-is so that the guest sees proper key-down / key-up.
///
/// Returns 0 for unmapped keys.
fn keycode_to_scancode(keycode: u32) -> u8 {
    // Special keys (KEY_* constants from compositor, >= 0x100)
    match keycode {
        anyui::KEY_ENTER => return 0x1C,
        anyui::KEY_BACKSPACE => return 0x0E,
        anyui::KEY_TAB => return 0x0F,
        anyui::KEY_ESCAPE => return 0x01,
        anyui::KEY_SPACE => return 0x39,
        anyui::KEY_UP => return 0x48,
        anyui::KEY_DOWN => return 0x50,
        anyui::KEY_LEFT => return 0x4B,
        anyui::KEY_RIGHT => return 0x4D,
        anyui::KEY_DELETE => return 0x53,
        anyui::KEY_HOME => return 0x47,
        anyui::KEY_END => return 0x4F,
        anyui::KEY_PAGE_UP => return 0x49,
        anyui::KEY_PAGE_DOWN => return 0x51,
        anyui::KEY_F1 => return 0x3B,
        anyui::KEY_F2 => return 0x3C,
        anyui::KEY_F3 => return 0x3D,
        anyui::KEY_F4 => return 0x3E,
        anyui::KEY_F5 => return 0x3F,
        anyui::KEY_F6 => return 0x40,
        anyui::KEY_F7 => return 0x41,
        anyui::KEY_F8 => return 0x42,
        anyui::KEY_F9 => return 0x43,
        anyui::KEY_F10 => return 0x44,
        anyui::KEY_F11 => return 0x57,
        anyui::KEY_F12 => return 0x58,
        _ => {}
    }

    // Regular keys: the compositor passes raw PS/2 scancodes (< 0x80)
    // through encode_scancode() unchanged.  Forward them directly.
    // This includes letter keys, digit keys, punctuation, and modifier
    // scancodes (Shift=0x2A/0x36, Ctrl=0x1D, Alt=0x38, CapsLock=0x3A).
    if keycode < 0x80 {
        return keycode as u8;
    }

    0
}

/// Send a key-down or key-up event to the selected VM (if running).
/// Returns `true` if the event was forwarded, `false` otherwise.
fn send_key_to_vm(keycode: u32, is_down: bool) -> bool {
    let a = app();
    if a.selected_vm < a.vms.len() {
        let entry = &a.vms[a.selected_vm];
        if entry.state == VmState::Running && entry.cmd_pipe != 0 {
            let scancode = keycode_to_scancode(keycode);
            if scancode != 0 {
                let dir = if is_down { "keydown" } else { "keyup" };
                let cmd = format!("{} {}", dir, scancode);
                ipc::pipe_write(entry.cmd_pipe, cmd.as_bytes());
                return true;
            }
        }
    }
    false
}

// ── Create Disk Image dialog ────────────────────────────────────────────

/// Open the "Create Disk Image" dialog.
///
/// `disk_field_id` is the ID of the `disk_field` TextField in the settings
/// dialog; after a successful disk creation the field is updated with the new
/// image path.
fn open_create_disk_dialog(disk_field_id: u32) {
    let a = app();

    // Close any existing create-disk dialog first.
    if a.create_disk_dlg.is_some() {
        close_create_disk_dialog();
    }

    let win = anyui::Window::new("Create Disk Image", -1, -1, 420, 210);
    if let Some(ref settings) = a.settings {
        win.set_modal(&settings.win);
    } else {
        win.set_modal(&a.win);
    }

    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    content.set_color(0xFF1E1E1E);

    // Path row.
    let path_lbl = anyui::Label::new("Path:");
    path_lbl.set_position(16, 18);
    path_lbl.set_size(70, 24);
    path_lbl.set_text_color(0xFFE6E6E6);
    content.add(&path_lbl);

    let path_field = anyui::TextField::new();
    path_field.set_position(90, 14);
    path_field.set_size(226, 28);
    path_field.set_placeholder("/Users/Shared/vmmanager/disk.img");
    content.add(&path_field);

    let path_browse = anyui::Button::new("Browse…");
    path_browse.set_position(320, 14);
    path_browse.set_size(80, 28);
    content.add(&path_browse);

    // Size row.
    let size_lbl = anyui::Label::new("Size:");
    size_lbl.set_position(16, 60);
    size_lbl.set_size(70, 24);
    size_lbl.set_text_color(0xFFE6E6E6);
    content.add(&size_lbl);

    let size_field = anyui::TextField::new();
    size_field.set_position(90, 56);
    size_field.set_size(100, 28);
    size_field.set_text("512");
    content.add(&size_field);

    let unit_lbl = anyui::Label::new("MB (sparse, not pre-allocated)");
    unit_lbl.set_position(196, 60);
    unit_lbl.set_size(210, 24);
    unit_lbl.set_text_color(0xFF888888);
    unit_lbl.set_font_size(11);
    content.add(&unit_lbl);

    // Buttons.
    let create_btn = anyui::Button::new("Create");
    create_btn.set_position(218, 162);
    create_btn.set_size(80, 30);
    content.add(&create_btn);

    let cancel_btn = anyui::Button::new("Cancel");
    cancel_btn.set_position(308, 162);
    cancel_btn.set_size(80, 30);
    content.add(&cancel_btn);

    win.add(&content);

    // Wire up Browse: open save-file dialog and fill path field.
    let path_field_id = path_field.id();
    path_browse.on_click(move |_| {
        if let Some(path) = anyui::FileDialog::save_file("disk.img") {
            anyui::Control::from_id(path_field_id).set_text(&path);
        }
    });

    // Wire up Create button.
    create_btn.on_click(|_| {
        do_create_disk();
    });

    // Wire up Cancel and window close.
    cancel_btn.on_click(|_| {
        close_create_disk_dialog();
    });
    win.on_close(|_| {
        close_create_disk_dialog();
    });

    a.create_disk_dlg = Some(CreateDiskDialog {
        win,
        path_field,
        size_field,
        disk_field_id,
    });
}

/// Create the sparse disk image from the values entered in the create-disk
/// dialog and update the settings dialog's disk field.
///
/// The image is created as a sparse (not pre-allocated) flat file: a valid
/// 512-byte MBR sector is written at the start, then the file is extended to
/// the requested size by seeking to the last byte and writing a single zero.
/// On filesystems that support sparse files the intermediate region will not
/// consume physical disk space.
fn do_create_disk() {
    let a = app();
    let dlg = match a.create_disk_dlg.as_ref() {
        Some(d) => d,
        None => return,
    };

    // Read destination path.
    let mut path_buf = [0u8; 257];
    let path_len = dlg.path_field.get_text(&mut path_buf);
    if path_len == 0 {
        return;
    }
    let path = bytes_to_string(&path_buf[..path_len as usize]);

    // Read size (MB), clamp to 1–2047 MB so the i32 seek doesn't overflow.
    let mut size_buf = [0u8; 16];
    let size_len = dlg.size_field.get_text(&mut size_buf);
    let size_mb = parse_u32(&size_buf[..size_len as usize])
        .unwrap_or(512)
        .max(1)
        .min(2047);

    let disk_field_id = dlg.disk_field_id;

    // Create the file (truncate if it already exists).
    let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return;
    }

    // Write a minimal MBR header (zeros + 0x55AA boot signature).
    let mut header = [0u8; 512];
    header[510] = 0x55;
    header[511] = 0xAA;
    fs::write(fd, &header);

    // Extend to full size by seeking to the last byte and writing a zero.
    // SEEK_CUR skips past the 512 bytes already written.
    let seek_bytes = (size_mb as i32) * 1024 * 1024 - 512 - 1;
    if seek_bytes > 0 {
        fs::lseek(fd, seek_bytes, fs::SEEK_CUR);
    }
    fs::write(fd, &[0u8]);

    fs::close(fd);

    // Update the disk_field in the settings dialog.
    anyui::Control::from_id(disk_field_id).set_text(&path);

    close_create_disk_dialog();
}

/// Close and destroy the create-disk dialog.
fn close_create_disk_dialog() {
    let a = app();
    if let Some(dlg) = a.create_disk_dlg.take() {
        dlg.win.destroy();
    }
}

// ── Settings dialog ────────────────────────────────────────────────────

/// Helper: create a label at fixed position inside a parent view.
fn settings_label(parent: &anyui::View, text: &str, x: i32, y: i32) {
    let lbl = anyui::Label::new(text);
    lbl.set_position(x, y);
    lbl.set_size(100, 24);
    lbl.set_text_color(0xFFE6E6E6);
    parent.add(&lbl);
}

fn cpu_cores_to_seg_state(cores: u8) -> u32 {
    match cores {
        2 => 1,
        4 => 2,
        8 => 3,
        _ => 0,
    }
}

fn seg_state_to_cpu_cores(state: u32) -> u8 {
    match state {
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 1,
    }
}

/// Open the settings dialog for the currently selected VM.
///
/// Creates a tabbed dialog with three tabs:
/// - **General**: Name, RAM, RAM allocation, BIOS, CPU cores
/// - **Devices**: GPU, Network adapter (mode, host NIC, MAC)
/// - **Boot**: Boot order, Disk image, ISO image
fn open_settings_dialog() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        return;
    }

    // Close any existing settings dialog.
    if a.settings.is_some() {
        close_settings_dialog();
    }

    let config = a.vms[a.selected_vm].config.clone();

    let win = anyui::Window::new("VM Settings", -1, -1, 500, 440);
    win.set_modal(&a.win);

    // Outer content container.
    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    content.set_color(0xFF1E1E1E);

    // ── Tab selector ────────────────────────────────────────────────
    let tab_seg = anyui::SegmentedControl::new("General|Devices|Boot");
    tab_seg.set_position(16, 12);
    tab_seg.set_size(468, 28);
    content.add(&tab_seg);

    // ════════════════════════════════════════════════════════════════
    //  Tab 0: General
    // ════════════════════════════════════════════════════════════════
    let general_panel = anyui::View::new();
    general_panel.set_position(0, 48);
    general_panel.set_size(500, 340);
    content.add(&general_panel);

    // Name
    settings_label(&general_panel, "Name:", 16, 8);
    let name_field = anyui::TextField::new();
    name_field.set_position(120, 4);
    name_field.set_size(360, 28);
    name_field.set_text(&config.name);
    general_panel.add(&name_field);

    // RAM slider
    settings_label(&general_panel, "RAM:", 16, 48);
    let slider_val = ((config.ram_mb.saturating_sub(16)) * 100 / 496).min(100);
    let ram_slider = anyui::Slider::new(slider_val);
    ram_slider.set_position(120, 48);
    ram_slider.set_size(280, 24);
    general_panel.add(&ram_slider);

    let mut rbuf = [0u8; 16];
    let ram_text = fmt_label_val(&mut rbuf, "", config.ram_mb, " MB");
    let ram_value_label = anyui::Label::new(ram_text);
    ram_value_label.set_position(410, 48);
    ram_value_label.set_size(70, 24);
    ram_value_label.set_text_color(0xFFE6E6E6);
    general_panel.add(&ram_value_label);

    // RAM allocation
    settings_label(&general_panel, "RAM Alloc:", 16, 88);
    let ram_alloc_seg = anyui::SegmentedControl::new("On-Demand|Pre-allocate");
    ram_alloc_seg.set_position(120, 84);
    ram_alloc_seg.set_size(240, 28);
    ram_alloc_seg.set_state(match config.ram_alloc {
        RamAlloc::OnDemand => 0,
        RamAlloc::Preallocate => 1,
    });
    general_panel.add(&ram_alloc_seg);

    // BIOS type
    settings_label(&general_panel, "BIOS:", 16, 128);
    let bios_seg = anyui::SegmentedControl::new("CoreVM|SeaBIOS");
    bios_seg.set_position(120, 124);
    bios_seg.set_size(240, 28);
    bios_seg.set_state(match config.bios_type {
        BiosType::CoreVm => 0,
        BiosType::SeaBios => 1,
    });
    general_panel.add(&bios_seg);

    // CPU cores
    settings_label(&general_panel, "CPU Cores:", 16, 168);
    let cores_seg = anyui::SegmentedControl::new("1|2|4|8");
    cores_seg.set_position(120, 164);
    cores_seg.set_size(240, 28);
    cores_seg.set_state(cpu_cores_to_seg_state(config.cpu_cores));
    general_panel.add(&cores_seg);

    // ════════════════════════════════════════════════════════════════
    //  Tab 1: Devices
    // ════════════════════════════════════════════════════════════════
    let devices_panel = anyui::View::new();
    devices_panel.set_position(0, 48);
    devices_panel.set_size(500, 340);
    content.add(&devices_panel);

    // GPU
    settings_label(&devices_panel, "GPU:", 16, 8);
    let gpu_seg = anyui::SegmentedControl::new("SVGA Framebuffer");
    gpu_seg.set_position(120, 4);
    gpu_seg.set_size(240, 28);
    gpu_seg.set_state(0);
    devices_panel.add(&gpu_seg);

    // ── Network section ─────────────────────────────────────────────
    let net_header = anyui::Label::new("Network Adapter");
    net_header.set_position(16, 48);
    net_header.set_size(200, 20);
    net_header.set_text_color(0xFF8888CC);
    net_header.set_font_size(12);
    devices_panel.add(&net_header);

    // Network enabled
    settings_label(&devices_panel, "Enabled:", 16, 76);
    let net_toggle = anyui::Toggle::new(config.net_enabled);
    net_toggle.set_position(120, 76);
    net_toggle.set_size(48, 24);
    devices_panel.add(&net_toggle);

    // Network mode
    settings_label(&devices_panel, "Mode:", 16, 116);
    let net_mode_seg = anyui::SegmentedControl::new("NAT|Bridge");
    net_mode_seg.set_position(120, 112);
    net_mode_seg.set_size(200, 28);
    net_mode_seg.set_state(match config.net_mode {
        NetMode::Nat => 0,
        NetMode::Bridge => 1,
    });
    devices_panel.add(&net_mode_seg);

    // Host NIC (for bridged mode)
    settings_label(&devices_panel, "Host NIC:", 16, 156);
    let net_host_field = anyui::TextField::new();
    net_host_field.set_position(120, 152);
    net_host_field.set_size(240, 28);
    net_host_field.set_text(&config.net_host_nic);
    net_host_field.set_placeholder("e.g. en0");
    devices_panel.add(&net_host_field);

    // MAC address mode
    settings_label(&devices_panel, "MAC:", 16, 196);
    let mac_mode_seg = anyui::SegmentedControl::new("Dynamic|Static");
    mac_mode_seg.set_position(120, 192);
    mac_mode_seg.set_size(200, 28);
    mac_mode_seg.set_state(match config.mac_mode {
        MacMode::Dynamic => 0,
        MacMode::Static => 1,
    });
    devices_panel.add(&mac_mode_seg);

    // MAC address (static)
    settings_label(&devices_panel, "MAC Addr:", 16, 236);
    let mac_field = anyui::TextField::new();
    mac_field.set_position(120, 232);
    mac_field.set_size(240, 28);
    mac_field.set_text(&config.mac_address);
    mac_field.set_placeholder("52:54:00:12:34:56");
    devices_panel.add(&mac_field);

    // ════════════════════════════════════════════════════════════════
    //  Tab 2: Boot
    // ════════════════════════════════════════════════════════════════
    let boot_panel = anyui::View::new();
    boot_panel.set_position(0, 48);
    boot_panel.set_size(500, 340);
    content.add(&boot_panel);

    // Boot order
    settings_label(&boot_panel, "Boot Order:", 16, 8);
    let boot_seg = anyui::SegmentedControl::new("Disk|CD|Floppy");
    boot_seg.set_position(120, 4);
    boot_seg.set_size(360, 28);
    boot_seg.set_state(match config.boot_order {
        BootOrder::DiskFirst => 0u32,
        BootOrder::CdFirst => 1,
        BootOrder::FloppyFirst => 2,
    });
    boot_panel.add(&boot_seg);

    // Disk image
    settings_label(&boot_panel, "Disk Image:", 16, 52);
    let disk_field = anyui::TextField::new();
    disk_field.set_position(120, 48);
    disk_field.set_size(210, 28);
    disk_field.set_text(&config.disk_image);
    disk_field.set_placeholder("/path/to/disk.img");
    boot_panel.add(&disk_field);

    let disk_browse = anyui::Button::new("Browse...");
    disk_browse.set_position(334, 48);
    disk_browse.set_size(66, 28);
    boot_panel.add(&disk_browse);

    let disk_create = anyui::Button::new("New...");
    disk_create.set_position(404, 48);
    disk_create.set_size(68, 28);
    boot_panel.add(&disk_create);

    // ISO image
    settings_label(&boot_panel, "ISO Image:", 16, 96);
    let iso_field = anyui::TextField::new();
    iso_field.set_position(120, 92);
    iso_field.set_size(210, 28);
    iso_field.set_text(&config.iso_image);
    iso_field.set_placeholder("/path/to/boot.iso");
    boot_panel.add(&iso_field);

    let iso_browse = anyui::Button::new("Browse...");
    iso_browse.set_position(334, 92);
    iso_browse.set_size(66, 28);
    boot_panel.add(&iso_browse);

    // ── Connect tabs to panels (auto show/hide) ─────────────────────
    tab_seg.connect_panels(&[&general_panel, &devices_panel, &boot_panel]);

    // ── Save / Cancel buttons ───────────────────────────────────────
    let save_btn = anyui::Button::new("Save");
    save_btn.set_position(300, 398);
    save_btn.set_size(80, 30);
    content.add(&save_btn);

    let cancel_btn = anyui::Button::new("Cancel");
    cancel_btn.set_position(390, 398);
    cancel_btn.set_size(80, 30);
    content.add(&cancel_btn);

    win.add(&content);

    // ── Wire up callbacks ───────────────────────────────────────────

    // RAM slider value display.
    let ram_val_id = ram_value_label.id();
    ram_slider.on_value_changed(move |e| {
        let ram_mb = 16 + (e.value as u32) * 496 / 100;
        let ram_mb = ((ram_mb + 8) / 16) * 16;
        let mut buf = [0u8; 16];
        let s = fmt_label_val(&mut buf, "", ram_mb, " MB");
        anyui::Control::from_id(ram_val_id).set_text(s);
    });

    // Browse for disk image.
    let disk_field_id = disk_field.id();
    disk_browse.on_click(move |_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            anyui::Control::from_id(disk_field_id).set_text(&path);
        }
    });

    // Create new disk image.
    disk_create.on_click(move |_| {
        open_create_disk_dialog(disk_field_id);
    });

    // Browse for ISO.
    let iso_field_id = iso_field.id();
    iso_browse.on_click(move |_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            anyui::Control::from_id(iso_field_id).set_text(&path);
        }
    });

    // Save button.
    save_btn.on_click(|_| {
        save_settings();
    });

    // Cancel button.
    cancel_btn.on_click(|_| {
        close_settings_dialog();
    });

    // Close window.
    win.on_close(|_| {
        close_settings_dialog();
    });

    a.settings = Some(SettingsDialog {
        win,
        // General
        name_field,
        ram_slider,
        ram_value_label,
        cores_seg,
        ram_alloc_seg,
        bios_seg,
        // Devices
        gpu_seg,
        net_toggle,
        net_mode_seg,
        net_host_field,
        mac_mode_seg,
        mac_field,
        // Boot
        boot_seg,
        disk_field,
        iso_field,
    });
}

/// Save the settings dialog values to the selected VM config.
fn save_settings() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        close_settings_dialog();
        return;
    }

    if let Some(ref dlg) = a.settings {
        // ── General tab ──
        let mut name_buf = [0u8; 64];
        let name_len = dlg.name_field.get_text(&mut name_buf);
        let name = bytes_to_string(&name_buf[..name_len as usize]);

        let slider_val = dlg.ram_slider.get_state();
        let ram_mb = 16 + (slider_val as u32) * 496 / 100;
        let ram_mb = ((ram_mb + 8) / 16) * 16;

        let ram_alloc = match dlg.ram_alloc_seg.get_state() {
            1 => RamAlloc::Preallocate,
            _ => RamAlloc::OnDemand,
        };

        let bios_type = match dlg.bios_seg.get_state() {
            1 => BiosType::SeaBios,
            _ => BiosType::CoreVm,
        };

        let cpu_cores = seg_state_to_cpu_cores(dlg.cores_seg.get_state());
        // ── Devices tab ──
        let gpu_type = match dlg.gpu_seg.get_state() {
            _ => GpuType::SvgaFb,
        };

        let net_enabled = dlg.net_toggle.get_state() != 0;

        let net_mode = match dlg.net_mode_seg.get_state() {
            1 => NetMode::Bridge,
            _ => NetMode::Nat,
        };

        let mut nic_buf = [0u8; 64];
        let nic_len = dlg.net_host_field.get_text(&mut nic_buf);
        let net_host_nic = bytes_to_string(&nic_buf[..nic_len as usize]);

        let mac_mode = match dlg.mac_mode_seg.get_state() {
            1 => MacMode::Static,
            _ => MacMode::Dynamic,
        };

        let mut mac_buf = [0u8; 32];
        let mac_len = dlg.mac_field.get_text(&mut mac_buf);
        let mac_address = bytes_to_string(&mac_buf[..mac_len as usize]);

        // ── Boot tab ──
        let boot_order = match dlg.boot_seg.get_state() {
            1 => BootOrder::CdFirst,
            2 => BootOrder::FloppyFirst,
            _ => BootOrder::DiskFirst,
        };

        let mut disk_buf = [0u8; 256];
        let disk_len = dlg.disk_field.get_text(&mut disk_buf);
        let disk_image = bytes_to_string(&disk_buf[..disk_len as usize]);

        let mut iso_buf = [0u8; 256];
        let iso_len = dlg.iso_field.get_text(&mut iso_buf);
        let iso_image = bytes_to_string(&iso_buf[..iso_len as usize]);

        // Apply to the selected VM config.
        let config = &mut a.vms[a.selected_vm].config;
        if !name.is_empty() {
            config.name = name;
        }
        config.ram_mb = ram_mb.max(16).min(512);
        config.cpu_cores = cpu_cores.clamp(1, 8);
        config.ram_alloc = ram_alloc;
        config.bios_type = bios_type;
        config.gpu_type = gpu_type;
        config.net_enabled = net_enabled;
        config.net_mode = net_mode;
        config.net_host_nic = net_host_nic;
        config.mac_mode = mac_mode;
        config.mac_address = mac_address;
        config.boot_order = boot_order;
        config.disk_image = disk_image;
        config.iso_image = iso_image;

        // Persist and refresh UI.
        save_vm_config(config);
        rebuild_sidebar();
        update_info_labels();
    }

    close_settings_dialog();
}

/// Close the settings dialog window.
fn close_settings_dialog() {
    let a = app();
    if let Some(dlg) = a.settings.take() {
        dlg.win.destroy();
    }
}

// ── New VM creation ────────────────────────────────────────────────────

/// Create a new VM with a default configuration and add it to the list.
///
/// Automatically opens the settings dialog for the new VM so the user
/// can configure it immediately.
fn create_new_vm() {
    let a = app();
    if a.vms.len() >= MAX_VMS {
        return;
    }

    // Generate a unique default name.
    let vm_num = a.vms.len() + 1;
    let mut name_buf = [0u8; 32];
    let mut p = 0;
    let prefix = b"New VM ";
    for &b in prefix {
        name_buf[p] = b;
        p += 1;
    }
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, vm_num as u32);
    for &b in s.as_bytes() {
        name_buf[p] = b;
        p += 1;
    }
    let name = unsafe { core::str::from_utf8_unchecked(&name_buf[..p]) };

    let config = VmConfig::new(name);
    save_vm_config(&config);
    a.layout.root_vms.push(config.uuid.clone());
    save_layout(&a.layout);

    a.vms.push(VmEntry {
        config,
        state: VmState::Stopped,
        vmd_tid: 0,
        cmd_pipe: 0,
        status_pipe: 0,
        shm_id: 0,
        shm_ptr: core::ptr::null(),
    });

    a.selected_vm = a.vms.len() - 1;

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();

    // Immediately open settings so the user can configure the new VM.
    open_settings_dialog();
}

// ── VM execution tick ──────────────────────────────────────────────────

/// Main timer callback: advance the selected running VM and update display.
///
/// Called periodically (~33ms = ~30 fps) by the anyui timer. For the
/// selected running VM:
/// 1. Advance the PIT and deliver timer interrupts via the PIC.
/// 2. Run the CPU for a batch of instructions.
/// 3. Read the SHM framebuffer and render to the canvas.
/// 4. Update the info labels with cached instruction count.
fn vm_tick() {
    let a = app();

    if a.selected_vm >= a.vms.len() {
        return;
    }

    let entry = &mut a.vms[a.selected_vm];
    if entry.state != VmState::Running {
        return;
    }

    // Poll status pipe for state changes, errors, serial output.
    if entry.status_pipe != 0 {
        let mut buf = [0u8; 512];
        loop {
            let n = ipc::pipe_read(entry.status_pipe, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            let msg = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            for line in msg.split('\n') {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line.starts_with("state 0 halted") {
                    entry.state = VmState::Stopped;
                    a.status_label.set_text("VM halted");
                    a.status_label.set_text_color(0xFF999999);
                } else if line.starts_with("state 0 stopped") {
                    entry.state = VmState::Stopped;
                } else if line.starts_with("state 0 error") || line.starts_with("error 0") {
                    entry.state = VmState::Stopped;
                    let detail = if line.len() > 8 { &line[8..] } else { "error" };
                    a.status_label.set_text(detail);
                    a.status_label.set_text_color(0xFFFF4040);
                    anyos_std::println!("vmmanager: {}", detail);
                } else if line.starts_with("serial 0 ") {
                    let text = &line[9..];
                    anyos_std::print!("{}", text);
                }
            }
        }
    }

    // Read SHM header for dirty flag and framebuffer.
    if !entry.shm_ptr.is_null() {
        unsafe {
            let hdr = entry.shm_ptr;

            // Check dirty flag.
            let dirty = (hdr.add(12) as *const u32).read_volatile();
            if dirty != 0 {
                // Clear dirty flag.
                (hdr as *mut u8).add(12).cast::<u32>().write_volatile(0);

                // Read display info.
                let width = (hdr.add(0) as *const u32).read_volatile();
                let height = (hdr.add(4) as *const u32).read_volatile();
                let bpp = (hdr.add(8) as *const u32).read_volatile();
                let payload = hdr.add(SHM_HEADER);

                if bpp == 0 && width == 80 && height == 25 {
                    // Text mode: payload is u16 cells.
                    a.canvas.clear(0xFF000000);
                    let text_ptr = payload as *const u16;
                    let text_len = (width * height) as usize;
                    let text_buf = core::slice::from_raw_parts(text_ptr, text_len);
                    render_text_mode(&a.canvas, text_buf);
                } else if bpp > 0 && width > 0 && height > 0 {
                    // Graphics mode: payload is raw pixel data.
                    let bpp_u8 = bpp as u8;
                    // Accept known pixel formats only to avoid invalid SHM payloads.
                    if bpp_u8 == 4 || bpp_u8 == 8 || bpp_u8 == 24 || bpp_u8 == 32 {
                        let bytes_per_pixel = ((bpp as usize) + 7) / 8;
                        let byte_len = (width as usize)
                            .checked_mul(height as usize)
                            .and_then(|px| px.checked_mul(bytes_per_pixel));
                        if let Some(byte_len) = byte_len {
                            if byte_len <= SHM_SIZE.saturating_sub(SHM_HEADER) {
                                let fb = core::slice::from_raw_parts(payload, byte_len);
                                render_graphics_mode(&a.canvas, fb, width, height, bpp_u8);
                            } else {
                                anyos_std::println!(
                                    "vmmanager: invalid SHM frame size w={} h={} bpp={} len={}",
                                    width,
                                    height,
                                    bpp,
                                    byte_len
                                );
                            }
                        } else {
                            anyos_std::println!(
                                "vmmanager: SHM frame size overflow w={} h={} bpp={}",
                                width,
                                height,
                                bpp
                            );
                        }
                    } else {
                        anyos_std::println!(
                            "vmmanager: unsupported SHM bpp={} (w={} h={})",
                            bpp,
                            width,
                            height
                        );
                    }
                }
            }

            // Also check VM state from SHM header.
            let shm_state = (hdr.add(16) as *const u32).read_volatile();
            if shm_state == 2 || shm_state == 3 {
                // Halted or error — VM stopped running in vmd.
                if entry.state == VmState::Running {
                    entry.state = VmState::Stopped;
                    if shm_state == 2 {
                        a.status_label.set_text("VM halted");
                        a.status_label.set_text_color(0xFF999999);
                    }
                }
            }
        }
    }

    // Update info labels.
    update_info_labels();

    // Refresh sidebar if VM state changed.
    if entry.state != VmState::Running {
        cleanup_vm_ipc(entry);
        rebuild_sidebar();
        update_status_bar();
    }
}

// ── Main entry point ───────────────────────────────────────────────────

fn main() {
    // Initialize the UI framework.
    if !anyui::init() {
        return;
    }

    // VM execution is now handled by the vmd daemon process.
    // No libcorevm initialization needed in vmmanager.

    // ── Main window ────────────────────────────────────────────────

    let win = anyui::Window::new("VM Manager", -1, -1, WIN_W, WIN_H);

    // ── Toolbar (DOCK_TOP) ─────────────────────────────────────────

    let tc = anyui::theme::colors();

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 42);
    toolbar.set_color(tc.sidebar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let title_lbl = toolbar.add_label("VM Manager");
    title_lbl.set_text_color(0xFF00C8FF);
    title_lbl.set_size(110, 34);
    title_lbl.set_font_size(14);
    title_lbl.set_font(1); // bold

    toolbar.add_separator();

    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_system_icon("circle-plus", anyui::IconType::Outline, tc.text, 24);
    btn_new.set_tooltip("New VM");

    let btn_start = toolbar.add_icon_button("");
    btn_start.set_size(34, 34);
    btn_start.set_system_icon("player-play", anyui::IconType::Outline, tc.check_mark, 24);
    btn_start.set_color(tc.success);
    btn_start.set_tooltip("Start VM");

    let btn_stop = toolbar.add_icon_button("");
    btn_stop.set_size(34, 34);
    btn_stop.set_system_icon("player-stop", anyui::IconType::Outline, tc.text, 24);
    btn_stop.set_tooltip("Stop VM");

    toolbar.add_separator();

    let btn_settings = toolbar.add_icon_button("");
    btn_settings.set_size(34, 34);
    btn_settings.set_system_icon("settings", anyui::IconType::Outline, tc.text, 24);
    btn_settings.set_tooltip("VM Settings");

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", anyui::IconType::Outline, tc.text, 24);
    btn_delete.set_tooltip("Delete VM");

    win.add(&toolbar);

    // ── Menu bar ───────────────────────────────────────────────────

    let mut mb = anyui::MenuBarBuilder::new()
        .menu("File")
        .item(1, "New VM", 0)
        .separator()
        .item(2, "Quit", 0)
        .end_menu()
        .menu("VM")
        .item(10, "Start", 0)
        .item(11, "Stop", 0)
        .separator()
        .item(12, "Settings", 0)
        .end_menu();
    let menu_data = mb.build();

    let menu_bar = anyui::MenuBar::set(win.id(), menu_data);
    menu_bar.on_item(|e| match e.item_id {
        1 => create_new_vm(),
        2 => anyui::quit(),
        10 => start_selected_vm(),
        11 => stop_selected_vm(),
        12 => open_settings_dialog(),
        _ => {}
    });

    // ── Status bar (DOCK_BOTTOM) ───────────────────────────────────

    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(0xFF1A1A2E);

    let status_label = anyui::Label::new("Ready | 0 VM running");
    status_label.set_position(10, 3);
    status_label.set_size(WIN_W - 20, 18);
    status_label.set_text_color(0xFF8888AA);
    status_label.set_font_size(11);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Sidebar (DOCK_LEFT) ────────────────────────────────────────

    let sidebar = anyui::View::new();
    sidebar.set_dock(anyui::DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, WIN_H);
    sidebar.set_color(0xFF1E1E2E);

    // TreeView for VM list with folder organization.
    let sidebar_tree = anyui::TreeView::new(SIDEBAR_W, WIN_H - 42);
    sidebar_tree.set_dock(anyui::DOCK_FILL);
    sidebar_tree.set_row_height(28);
    sidebar_tree.set_indent_width(16);
    sidebar.add(&sidebar_tree);

    let sidebar_ctx_menu = anyui::ContextMenu::new("New VM|-|Start|Stop|Settings|-|Delete");
    sidebar_tree.set_context_menu(&sidebar_ctx_menu);
    sidebar.add(&sidebar_ctx_menu);

    win.add(&sidebar);

    // ── Main content area (DOCK_FILL) ──────────────────────────────

    let content_view = anyui::View::new();
    content_view.set_dock(anyui::DOCK_FILL);
    content_view.set_color(0xFF16161E);

    // Compact info bar (DOCK_BOTTOM) — single line showing VM status.
    let info_bar = anyui::View::new();
    info_bar.set_dock(anyui::DOCK_BOTTOM);
    info_bar.set_size(700, INFO_BAR_H);
    info_bar.set_color(0xFF12121A);

    let info_label = anyui::Label::new(" No VM selected");
    info_label.set_dock(anyui::DOCK_FILL);
    info_label.set_text_color(0xFF666680);
    info_label.set_font_size(11);
    info_bar.add(&info_label);
    content_view.add(&info_bar);

    // Canvas for VGA display (fills remaining content area).
    let canvas = anyui::Canvas::new(640, 400);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.clear(0xFF000000);
    canvas.set_interactive(true);
    content_view.add(&canvas);

    win.add(&content_view);

    // ── Load saved VMs ─────────────────────────────────────────────

    let (vms, layout) = load_all_vms();

    // ── Initialize global state ────────────────────────────────────

    unsafe {
        APP = Some(AppState {
            win,
            sidebar,
            canvas,
            toolbar,
            status_label,
            info: VmInfoLabels { label: info_label },
            content_view,
            sidebar_tree,
            tree_root: 0,
            node_map: Vec::new(),
            vms,
            selected_vm: 0,
            layout,
            settings: None,
            create_disk_dlg: None,
            btn_start_id: btn_start.id(),
            btn_stop_id: btn_stop.id(),
            btn_settings_id: btn_settings.id(),
            btn_delete_id: btn_delete.id(),
        });
    }

    // Build the initial sidebar.
    rebuild_sidebar();
    update_info_labels();
    update_status_bar();

    // ── Event handlers ─────────────────────────────────────────────

    // TreeView selection: use node_map to resolve node → VM or folder.
    app().sidebar_tree.on_selection_changed(|ev| {
        let a = app();
        let sel = ev.index as usize;
        if ev.index == u32::MAX || sel >= a.node_map.len() {
            return;
        }
        match a.node_map[sel] {
            NodeKind::Vm(vm_idx) => {
                if vm_idx < a.vms.len() && a.selected_vm != vm_idx {
                    a.selected_vm = vm_idx;
                    update_info_labels();
                    update_toolbar_buttons();

                    if a.vms[vm_idx].state != VmState::Running {
                        a.canvas.clear(0xFF0A0A14);
                    }
                }
            }
            NodeKind::Root | NodeKind::Folder(_) => {
                // Selecting a folder or root — ignore (expand/collapse handled by TreeView).
            }
        }
    });

    sidebar_ctx_menu.on_item_click(|e| match e.index {
        0 => create_new_vm(),
        2 if sidebar_selection_is_vm() => start_selected_vm(),
        3 if sidebar_selection_is_vm() => stop_selected_vm(),
        4 if sidebar_selection_is_vm() => open_settings_dialog(),
        6 if sidebar_selection_is_vm() => delete_selected_vm(),
        _ => {}
    });

    // Toolbar button: New VM.
    btn_new.on_click(|_| {
        create_new_vm();
    });

    // Toolbar button: Start.
    btn_start.on_click(|_| {
        start_selected_vm();
    });

    // Toolbar button: Stop.
    btn_stop.on_click(|_| {
        stop_selected_vm();
    });

    // Toolbar button: Settings.
    btn_settings.on_click(|_| {
        open_settings_dialog();
    });

    // Toolbar button: Delete.
    btn_delete.on_click(|_| {
        delete_selected_vm();
    });

    // Window keyboard handlers: forward key events to the VM when running.
    app().win.on_key_down(|ke| {
        if send_key_to_vm(ke.keycode, true) {
            return;
        }
        // App-level keyboard shortcut: Escape quits.
        if ke.keycode == anyui::KEY_ESCAPE {
            anyui::quit();
        }
    });

    app().win.on_key_up(|ke| {
        send_key_to_vm(ke.keycode, false);
    });

    // Canvas mouse handlers: forward to VM when running.
    app().canvas.on_mouse_down(|_x, _y, button| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running && entry.cmd_pipe != 0 {
                let ps2_buttons = match button {
                    1 => 0x01u8,
                    2 => 0x04,
                    3 => 0x02,
                    _ => 0,
                };
                let cmd = format!("mouse 0 0 {}", ps2_buttons);
                ipc::pipe_write(entry.cmd_pipe, cmd.as_bytes());
            }
        }
    });

    app().canvas.on_mouse_up(|_x, _y, _button| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running && entry.cmd_pipe != 0 {
                ipc::pipe_write(entry.cmd_pipe, b"mouse 0 0 0");
            }
        }
    });

    app().canvas.on_mouse_move(|x, y| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running && entry.cmd_pipe != 0 {
                static mut LAST_X: i32 = 0;
                static mut LAST_Y: i32 = 0;
                let (dx, dy) = unsafe {
                    let dx = x - LAST_X;
                    let dy = y - LAST_Y;
                    LAST_X = x;
                    LAST_Y = y;
                    (dx, dy)
                };
                if dx != 0 || dy != 0 {
                    let cmd = format!("mouse {} {} 0", dx, dy);
                    ipc::pipe_write(entry.cmd_pipe, cmd.as_bytes());
                }
            }
        }
    });

    // Window close handler.
    app().win.on_close(|_| {
        // Stop all running VMs before exit.
        let a = app();
        for entry in a.vms.iter_mut() {
            if entry.state != VmState::Stopped {
                if entry.cmd_pipe != 0 {
                    ipc::pipe_write(entry.cmd_pipe, b"stop");
                    ipc::pipe_write(entry.cmd_pipe, b"quit");
                }
                cleanup_vm_ipc(entry);
                entry.state = VmState::Stopped;
            }
        }
        anyui::quit();
    });

    // ── Timer: VM execution loop (~33ms = ~30 fps) ─────────────────

    anyui::set_timer(33, || {
        vm_tick();
    });

    // ── Enter event loop ───────────────────────────────────────────

    anyui::run();
}
