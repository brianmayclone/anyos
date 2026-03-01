//! VM Manager — VMware Workstation-style virtual machine manager for anyOS.
//!
//! Provides a graphical interface for creating, configuring, and running x86
//! virtual machines powered by libcorevm. Features include:
//! - VM list sidebar with status indicators
//! - Live VGA framebuffer display on a Canvas control
//! - Real-time CPU/memory/instruction count monitoring
//! - Settings dialog for editing VM configurations
//! - Keyboard and mouse forwarding to the guest OS
//!
//! VM configurations are persisted in `/System/vmmanager/vms.conf`.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use libanyui_client as anyui;
use libanyui_client::Widget;
use libcorevm_client::{CpuMode, ExitReason, VmHandle};

anyos_std::entry!(main);

// ── Constants ──────────────────────────────────────────────────────────

/// Main window dimensions.
const WIN_W: u32 = 900;
const WIN_H: u32 = 640;

/// Sidebar width in pixels.
const SIDEBAR_W: u32 = 200;

/// VGA display canvas dimensions (standard VGA text mode pixel area).
const CANVAS_W: u32 = 640;
const CANVAS_H: u32 = 400;

/// Number of guest instructions to execute per timer tick (~30 fps).
const INSTRUCTIONS_PER_TICK: u64 = 200_000;

/// Number of PIT ticks to advance per timer callback. Approximates the
/// real PIT rate of ~1.193 MHz scaled to a 33 ms timer interval: roughly
/// 40 ticks per frame at 1193 ticks/ms * 33 ms / 1000.
const PIT_TICKS_PER_FRAME: u32 = 40;

/// Maximum number of VMs that can be configured.
const MAX_VMS: usize = 16;

/// Path to the VM configuration file.
const CONFIG_PATH: &str = "/System/vmmanager/vms.conf";

/// Path to the SeaBIOS ROM image (256 KiB, loaded at physical address 0xC0000).
const SEABIOS_PATH: &str = "/System/shared/corevm/bios/seabios.bin";

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

/// Persistent configuration for a single VM.
#[derive(Clone)]
struct VmConfig {
    /// Human-readable name displayed in the sidebar.
    name: String,
    /// Guest RAM size in megabytes.
    ram_mb: u32,
    /// Path to a raw disk image file on the host filesystem.
    disk_image: String,
    /// Path to an ISO image file for CD-ROM emulation.
    iso_image: String,
    /// Boot device ordering.
    boot_order: BootOrder,
}

impl VmConfig {
    /// Create a new VM configuration with defaults.
    fn new(name: &str) -> Self {
        VmConfig {
            name: String::from(name),
            ram_mb: 64,
            disk_image: String::new(),
            iso_image: String::new(),
            boot_order: BootOrder::DiskFirst,
        }
    }
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
    config: VmConfig,
    handle: Option<VmHandle>,
    state: VmState,
}

/// Labels displaying real-time VM information.
struct VmInfoLabels {
    state_label: anyui::Label,
    mode_label: anyui::Label,
    ram_label: anyui::Label,
    insn_label: anyui::Label,
}

/// Controls used in the settings dialog window.
struct SettingsDialog {
    win: anyui::Window,
    name_field: anyui::TextField,
    ram_slider: anyui::Slider,
    ram_value_label: anyui::Label,
    disk_field: anyui::TextField,
    iso_field: anyui::TextField,
    boot_seg: anyui::SegmentedControl,
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

    // Sidebar item labels (one per VM, up to MAX_VMS)
    sidebar_labels: Vec<anyui::Label>,

    // VM data
    vms: Vec<VmEntry>,
    selected_vm: usize,

    // Settings dialog (created on demand)
    settings: Option<SettingsDialog>,

    // Tracks whether libcorevm initialized successfully
    corevm_available: bool,
}

static mut APP: Option<AppState> = None;

/// Get a mutable reference to the global application state.
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── Number formatting (no_std) ─────────────────────────────────────────

/// Format a `u32` value into a decimal string within `buf`.
fn fmt_u32<'a>(buf: &'a mut [u8], val: u32) -> &'a str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = val;
    let mut tmp = [0u8; 12];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

/// Format a `u64` value into a decimal string within `buf`.
fn fmt_u64<'a>(buf: &'a mut [u8], val: u64) -> &'a str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = val;
    let mut tmp = [0u8; 20];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
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

/// Save all VM configurations to the config file.
///
/// Format: one VM per block, fields separated by newlines, blocks separated
/// by a blank line. Fields: `name=`, `ram=`, `disk=`, `iso=`, `boot=`.
fn save_config(vms: &[VmEntry]) {
    let mut data: Vec<u8> = Vec::with_capacity(1024);
    for entry in vms.iter() {
        let c = &entry.config;
        data.extend_from_slice(b"name=");
        data.extend_from_slice(c.name.as_bytes());
        data.push(b'\n');
        data.extend_from_slice(b"ram=");
        let mut buf = [0u8; 12];
        let s = fmt_u32(&mut buf, c.ram_mb);
        data.extend_from_slice(s.as_bytes());
        data.push(b'\n');
        data.extend_from_slice(b"disk=");
        data.extend_from_slice(c.disk_image.as_bytes());
        data.push(b'\n');
        data.extend_from_slice(b"iso=");
        data.extend_from_slice(c.iso_image.as_bytes());
        data.push(b'\n');
        data.extend_from_slice(b"boot=");
        let boot_str = match c.boot_order {
            BootOrder::DiskFirst => "disk",
            BootOrder::CdFirst => "cd",
            BootOrder::FloppyFirst => "floppy",
        };
        data.extend_from_slice(boot_str.as_bytes());
        data.push(b'\n');
        data.push(b'\n');
    }

    let fd = fs::open(CONFIG_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &data);
        fs::close(fd);
    }
}

/// Load VM configurations from the config file.
///
/// Returns a list of `VmEntry` values with `state = Stopped` and no handle.
fn load_config() -> Vec<VmEntry> {
    let mut result = Vec::new();
    let fd = fs::open(CONFIG_PATH, 0);
    if fd == u32::MAX {
        return result;
    }

    let mut buf = [0u8; 4096];
    let n = fs::read(fd, &mut buf);
    fs::close(fd);
    if n == 0 || n == u32::MAX {
        return result;
    }

    let text = &buf[..n as usize];
    let mut current = VmConfig::new("");

    for line in ByteLines::new(text) {
        if line.is_empty() {
            // End of a VM block.
            if !current.name.is_empty() {
                result.push(VmEntry {
                    config: current.clone(),
                    handle: None,
                    state: VmState::Stopped,
                });
            }
            current = VmConfig::new("");
            continue;
        }

        if let Some(val) = strip_prefix(line, b"name=") {
            current.name = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"ram=") {
            current.ram_mb = parse_u32(val).unwrap_or(64);
        } else if let Some(val) = strip_prefix(line, b"disk=") {
            current.disk_image = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"iso=") {
            current.iso_image = bytes_to_string(val);
        } else if let Some(val) = strip_prefix(line, b"boot=") {
            current.boot_order = match val {
                b"cd" => BootOrder::CdFirst,
                b"floppy" => BootOrder::FloppyFirst,
                _ => BootOrder::DiskFirst,
            };
        }
    }
    // Handle last block if file doesn't end with a blank line.
    if !current.name.is_empty() {
        result.push(VmEntry {
            config: current,
            handle: None,
            state: VmState::Stopped,
        });
    }

    result
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

/// Rebuild the sidebar labels to reflect the current VM list.
///
/// Destroys existing labels and creates new ones for each VM entry,
/// showing a status indicator and the VM name. Also wires up click
/// handlers for VM selection.
fn rebuild_sidebar() {
    let a = app();

    // Remove old labels.
    for lbl in a.sidebar_labels.drain(..) {
        lbl.remove();
    }

    for (i, entry) in a.vms.iter().enumerate() {
        let lbl = anyui::Label::new("");
        // Offset below the "My VMs" title (24px height + 8px padding).
        lbl.set_position(8, (i as i32) * 32 + 32);
        lbl.set_size(SIDEBAR_W - 16, 24);
        lbl.set_font_size(13);

        // Build display text: status indicator + name.
        let dot = match entry.state {
            VmState::Running => ">> ",
            VmState::Paused => "|| ",
            VmState::Stopped => "   ",
        };
        let color = match entry.state {
            VmState::Running => 0xFF00FF80, // green
            VmState::Paused => 0xFFFFCC00,  // yellow
            VmState::Stopped => 0xFF999999, // gray
        };

        let mut text_buf = [0u8; 64];
        let mut p = 0;
        for &b in dot.as_bytes() {
            if p < 63 {
                text_buf[p] = b;
                p += 1;
            }
        }
        for &b in entry.config.name.as_bytes() {
            if p < 63 {
                text_buf[p] = b;
                p += 1;
            }
        }
        let text = unsafe { core::str::from_utf8_unchecked(&text_buf[..p]) };
        lbl.set_text(text);
        lbl.set_text_color(color);

        // Highlight selected item with a background color.
        if i == a.selected_vm {
            lbl.set_color(0xFF3A3A3C);
        } else {
            lbl.set_color(0x00000000); // transparent
        }

        a.sidebar.add(&lbl);
        a.sidebar_labels.push(lbl);
    }

    // Wire up click handlers on the newly created labels.
    setup_sidebar_click_handlers();
}

/// Wire up click handlers on sidebar labels to detect VM selection.
///
/// Each label's on_click closure selects the corresponding VM and
/// refreshes the UI.
fn setup_sidebar_click_handlers() {
    let a = app();
    for i in 0..a.sidebar_labels.len() {
        let label = a.sidebar_labels[i];
        label.on_click(move |_| {
            let a = app();
            if i < a.vms.len() && a.selected_vm != i {
                a.selected_vm = i;
                rebuild_sidebar();
                update_info_labels();

                // Clear canvas when switching to a non-running VM.
                if a.selected_vm < a.vms.len()
                    && a.vms[a.selected_vm].state != VmState::Running
                {
                    a.canvas.clear(0xFF1E1E1E);
                }
            }
        });
    }
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
fn update_info_labels() {
    let a = app();

    if a.selected_vm >= a.vms.len() {
        a.info.state_label.set_text("State: No VM selected");
        a.info.state_label.set_text_color(0xFF999999);
        a.info.mode_label.set_text("Mode: -");
        a.info.ram_label.set_text("RAM: -");
        a.info.insn_label.set_text("Instructions: -");
        return;
    }

    let entry = &a.vms[a.selected_vm];

    // State label.
    let (state_text, state_color) = match entry.state {
        VmState::Running => ("State: Running", 0xFF00FF80u32),
        VmState::Paused => ("State: Paused", 0xFFFFCC00u32),
        VmState::Stopped => ("State: Stopped", 0xFF999999u32),
    };
    a.info.state_label.set_text(state_text);
    a.info.state_label.set_text_color(state_color);

    // CPU mode, RAM, instruction count from the VM handle.
    if let Some(ref handle) = entry.handle {
        let mode_str = match handle.mode() {
            CpuMode::RealMode => "Mode: Real (16-bit)",
            CpuMode::ProtectedMode => "Mode: Protected (32-bit)",
            CpuMode::LongMode => "Mode: Long (64-bit)",
        };
        a.info.mode_label.set_text(mode_str);

        let mut buf = [0u8; 32];
        let s = fmt_label_val(&mut buf, "RAM: ", entry.config.ram_mb, " MB");
        a.info.ram_label.set_text(s);

        let mut ibuf = [0u8; 40];
        let s = fmt_label_u64(&mut ibuf, "Instructions: ", handle.instruction_count());
        a.info.insn_label.set_text(s);
    } else {
        a.info.mode_label.set_text("Mode: -");

        let mut buf = [0u8; 32];
        let s = fmt_label_val(&mut buf, "RAM: ", entry.config.ram_mb, " MB");
        a.info.ram_label.set_text(s);

        a.info.insn_label.set_text("Instructions: 0");
    }
}

// ── VM lifecycle operations ────────────────────────────────────────────

/// Start the selected VM.
///
/// Creates a `VmHandle`, registers standard devices, loads the disk/ISO
/// image, and begins execution on the next timer tick.
fn start_selected_vm() {
    let a = app();
    if a.selected_vm >= a.vms.len() {
        return;
    }
    if !a.corevm_available {
        a.status_label.set_text("Error: libcorevm not available");
        a.status_label.set_text_color(0xFFFF4040);
        return;
    }

    let entry = &mut a.vms[a.selected_vm];
    if entry.state != VmState::Stopped {
        return;
    }

    // Create the VM handle.
    let handle = match VmHandle::new(entry.config.ram_mb) {
        Some(h) => h,
        None => {
            a.status_label.set_text("Error: failed to create VM (out of memory?)");
            a.status_label.set_text_color(0xFFFF4040);
            return;
        }
    };

    // Register standard PC devices (PIC, PIT, PS/2, VGA, serial, CMOS).
    handle.setup_standard_devices();

    // Register IDE controller and attach disk image if configured.
    handle.setup_ide();
    if !entry.config.disk_image.is_empty() {
        let disk_data = read_file_to_vec(&entry.config.disk_image);
        if !disk_data.is_empty() {
            handle.ide_attach_disk(&disk_data);
        }
    }

    // Load ISO image into high memory if configured (above 1 MB).
    if !entry.config.iso_image.is_empty() {
        load_image_to_vm(&handle, &entry.config.iso_image, 0x10_0000);
    }

    // Try to load SeaBIOS. If present, load at 0xF0000 (64 KiB ROM) and
    // start at the x86 reset vector (CS:IP = F000:FFF0 = 0xFFFF0).
    // If BIOS is not available, fall back to direct boot from disk at 0x7C00.
    let bios_data = read_file_to_vec(SEABIOS_PATH);
    if !bios_data.is_empty() {
        // SeaBIOS ROM is loaded at the top of the first megabyte.
        // For a 64 KiB ROM: load at 0xF0000. For larger ROMs: load at
        // 0x100000 - rom_size so the reset vector at 0xFFFF0 is correct.
        let load_addr = if bios_data.len() <= 0x10000 {
            0xF0000u64
        } else {
            (0x10_0000u64).wrapping_sub(bios_data.len() as u64)
        };
        handle.load_binary(load_addr, &bios_data);
        // CPU starts at x86 reset vector.
        handle.set_rip(0xFFF0);
        // Set CS base to 0xF0000 for real-mode segment F000.
        // The BIOS expects CS:IP = F000:FFF0 = linear 0xFFFF0.
    } else if !entry.config.disk_image.is_empty() {
        // No BIOS available — direct boot from disk boot sector.
        load_image_to_vm(&handle, &entry.config.disk_image, 0x7C00);
        handle.set_rip(0x7C00);
    }

    entry.handle = Some(handle);
    entry.state = VmState::Running;

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();
}

/// Read an entire file into a `Vec<u8>`. Returns an empty Vec on failure.
fn read_file_to_vec(path: &str) -> Vec<u8> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Vec::new();
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    data
}

/// Load a disk/ISO image file into guest memory at the given address.
fn load_image_to_vm(handle: &VmHandle, path: &str, addr: u64) {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return;
    }

    let mut offset = addr;
    let mut buf = [0u8; 8192];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        handle.load_binary(offset, &buf[..n as usize]);
        offset += n as u64;
    }
    fs::close(fd);
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

    // Request stop and drop the handle.
    if let Some(ref handle) = entry.handle {
        handle.request_stop();
    }
    entry.handle = None;
    entry.state = VmState::Stopped;

    // Clear the canvas to show the VM is off.
    a.canvas.clear(0xFF1E1E1E);

    rebuild_sidebar();
    update_info_labels();
    update_status_bar();
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

    a.vms.remove(a.selected_vm);
    if a.selected_vm > 0 && a.selected_vm >= a.vms.len() {
        a.selected_vm = a.vms.len().saturating_sub(1);
    }

    save_config(&a.vms);
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
    let char_w: i32 = 8;
    let char_h: i32 = 16;

    for row in 0..rows {
        for col in 0..cols {
            let idx = row * cols + col;
            if idx >= text_buf.len() {
                break;
            }
            let entry = text_buf[idx];
            let ch = (entry & 0xFF) as u8;
            let attr = (entry >> 8) as u8;
            let fg_idx = (attr & 0x0F) as usize;
            let bg_idx = ((attr >> 4) & 0x07) as usize;
            let fg = VGA_COLORS[fg_idx];
            let bg = VGA_COLORS[bg_idx];

            let x = (col as i32) * char_w;
            let y = (row as i32) * char_h;

            // Draw background.
            canvas.fill_rect(x, y, char_w as u32, char_h as u32, bg);

            // Draw foreground character (skip non-printable for performance).
            if ch > 0x20 && ch < 0x7F {
                render_char_pixels(canvas, x, y, ch, fg);
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
    let canvas_h = canvas.get_height();

    let render_w = width.min(CANVAS_W);
    let render_h = height.min(canvas_h);

    match bpp {
        32 => {
            // BGRA format: read 4 bytes per pixel, convert to ARGB.
            for y in 0..render_h {
                for x in 0..render_w {
                    let src_off = ((y * width + x) * 4) as usize;
                    if src_off + 3 < fb.len() {
                        let b = fb[src_off] as u32;
                        let g = fb[src_off + 1] as u32;
                        let r = fb[src_off + 2] as u32;
                        let color = 0xFF000000 | (r << 16) | (g << 8) | b;
                        unsafe {
                            let dst = buf_ptr.add((y * stride + x) as usize);
                            *dst = color;
                        }
                    }
                }
            }
        }
        24 => {
            // BGR format: read 3 bytes per pixel.
            for y in 0..render_h {
                for x in 0..render_w {
                    let src_off = ((y * width + x) * 3) as usize;
                    if src_off + 2 < fb.len() {
                        let b = fb[src_off] as u32;
                        let g = fb[src_off + 1] as u32;
                        let r = fb[src_off + 2] as u32;
                        let color = 0xFF000000 | (r << 16) | (g << 8) | b;
                        unsafe {
                            let dst = buf_ptr.add((y * stride + x) as usize);
                            *dst = color;
                        }
                    }
                }
            }
        }
        8 => {
            // 256-color mode: first 16 use VGA palette, rest grayscale.
            for y in 0..render_h {
                for x in 0..render_w {
                    let src_off = (y * width + x) as usize;
                    if src_off < fb.len() {
                        let idx = fb[src_off] as usize;
                        let color = if idx < 16 {
                            VGA_COLORS[idx]
                        } else {
                            let gray = (idx as u32) & 0xFF;
                            0xFF000000 | (gray << 16) | (gray << 8) | gray
                        };
                        unsafe {
                            let dst = buf_ptr.add((y * stride + x) as usize);
                            *dst = color;
                        }
                    }
                }
            }
        }
        _ => {
            // Unsupported bpp: show placeholder.
            canvas.clear(0xFF2D2D2D);
        }
    }
}

// ── Keyboard scancode mapping ──────────────────────────────────────────

/// Map an anyui virtual keycode to a PS/2 scancode set 1 make code.
///
/// Returns 0 for unmapped keys.
fn keycode_to_scancode(keycode: u32) -> u8 {
    match keycode {
        0x1B => 0x01, // Escape
        0x31 => 0x02, // '1'
        0x32 => 0x03, // '2'
        0x33 => 0x04, // '3'
        0x34 => 0x05, // '4'
        0x35 => 0x06, // '5'
        0x36 => 0x07, // '6'
        0x37 => 0x08, // '7'
        0x38 => 0x09, // '8'
        0x39 => 0x0A, // '9'
        0x30 => 0x0B, // '0'
        0x2D => 0x0C, // '-'
        0x3D => 0x0D, // '='
        anyui::KEY_BACKSPACE => 0x0E,
        anyui::KEY_TAB => 0x0F,
        0x71 | 0x51 => 0x10, // q/Q
        0x77 | 0x57 => 0x11, // w/W
        0x65 | 0x45 => 0x12, // e/E
        0x72 | 0x52 => 0x13, // r/R
        0x74 | 0x54 => 0x14, // t/T
        0x79 | 0x59 => 0x15, // y/Y
        0x75 | 0x55 => 0x16, // u/U
        0x69 | 0x49 => 0x17, // i/I
        0x6F | 0x4F => 0x18, // o/O
        0x70 | 0x50 => 0x19, // p/P
        0x5B => 0x1A, // '['
        0x5D => 0x1B, // ']'
        anyui::KEY_ENTER => 0x1C,
        0x61 | 0x41 => 0x1E, // a/A
        0x73 | 0x53 => 0x1F, // s/S
        0x64 | 0x44 => 0x20, // d/D
        0x66 | 0x46 => 0x21, // f/F
        0x67 | 0x47 => 0x22, // g/G
        0x68 | 0x48 => 0x23, // h/H
        0x6A | 0x4A => 0x24, // j/J
        0x6B | 0x4B => 0x25, // k/K
        0x6C | 0x4C => 0x26, // l/L
        0x3B => 0x27, // ';'
        0x27 => 0x28, // '\''
        0x60 => 0x29, // '`'
        0x5C => 0x2B, // '\\'
        0x7A | 0x5A => 0x2C, // z/Z
        0x78 | 0x58 => 0x2D, // x/X
        0x63 | 0x43 => 0x2E, // c/C
        0x76 | 0x56 => 0x2F, // v/V
        0x62 | 0x42 => 0x30, // b/B
        0x6E | 0x4E => 0x31, // n/N
        0x6D | 0x4D => 0x32, // m/M
        0x2C => 0x33, // ','
        0x2E => 0x34, // '.'
        0x2F => 0x35, // '/'
        0x20 => 0x39, // ' '
        anyui::KEY_F1 => 0x3B,
        anyui::KEY_F2 => 0x3C,
        anyui::KEY_F3 => 0x3D,
        anyui::KEY_F4 => 0x3E,
        anyui::KEY_F5 => 0x3F,
        anyui::KEY_F6 => 0x40,
        anyui::KEY_F7 => 0x41,
        anyui::KEY_F8 => 0x42,
        anyui::KEY_F9 => 0x43,
        anyui::KEY_F10 => 0x44,
        anyui::KEY_F11 => 0x57,
        anyui::KEY_F12 => 0x58,
        anyui::KEY_UP => 0x48,
        anyui::KEY_DOWN => 0x50,
        anyui::KEY_LEFT => 0x4B,
        anyui::KEY_RIGHT => 0x4D,
        anyui::KEY_HOME => 0x47,
        anyui::KEY_END => 0x4F,
        anyui::KEY_PAGE_UP => 0x49,
        anyui::KEY_PAGE_DOWN => 0x51,
        anyui::KEY_DELETE => 0x53,
        anyui::KEY_ESCAPE => 0x01,
        _ => 0,
    }
}

// ── Settings dialog ────────────────────────────────────────────────────

/// Open the settings dialog for the currently selected VM.
///
/// Creates a new window with text fields, a slider, and a segmented
/// control for editing the VM configuration. Save/Cancel buttons commit
/// or discard changes.
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

    let win = anyui::Window::new("VM Settings", -1, -1, 420, 380);

    // Content area.
    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    content.set_color(0xFF1E1E1E);

    // VM Name.
    let name_lbl = anyui::Label::new("Name:");
    name_lbl.set_position(16, 16);
    name_lbl.set_size(80, 24);
    name_lbl.set_text_color(0xFFE6E6E6);
    content.add(&name_lbl);

    let name_field = anyui::TextField::new();
    name_field.set_position(100, 12);
    name_field.set_size(300, 28);
    name_field.set_text(&config.name);
    content.add(&name_field);

    // RAM slider.
    let ram_lbl = anyui::Label::new("RAM:");
    ram_lbl.set_position(16, 56);
    ram_lbl.set_size(80, 24);
    ram_lbl.set_text_color(0xFFE6E6E6);
    content.add(&ram_lbl);

    // Slider value 0-100 maps to 16-512 MB.
    let slider_val = ((config.ram_mb.saturating_sub(16)) * 100 / 496).min(100);
    let ram_slider = anyui::Slider::new(slider_val);
    ram_slider.set_position(100, 56);
    ram_slider.set_size(220, 24);
    content.add(&ram_slider);

    let mut rbuf = [0u8; 16];
    let ram_text = fmt_label_val(&mut rbuf, "", config.ram_mb, " MB");
    let ram_value_label = anyui::Label::new(ram_text);
    ram_value_label.set_position(330, 56);
    ram_value_label.set_size(70, 24);
    ram_value_label.set_text_color(0xFFE6E6E6);
    content.add(&ram_value_label);

    // Disk image path.
    let disk_lbl = anyui::Label::new("Disk:");
    disk_lbl.set_position(16, 100);
    disk_lbl.set_size(80, 24);
    disk_lbl.set_text_color(0xFFE6E6E6);
    content.add(&disk_lbl);

    let disk_field = anyui::TextField::new();
    disk_field.set_position(100, 96);
    disk_field.set_size(300, 28);
    disk_field.set_text(&config.disk_image);
    disk_field.set_placeholder("/path/to/disk.img");
    content.add(&disk_field);

    // ISO image path.
    let iso_lbl = anyui::Label::new("ISO:");
    iso_lbl.set_position(16, 144);
    iso_lbl.set_size(80, 24);
    iso_lbl.set_text_color(0xFFE6E6E6);
    content.add(&iso_lbl);

    let iso_field = anyui::TextField::new();
    iso_field.set_position(100, 140);
    iso_field.set_size(300, 28);
    iso_field.set_text(&config.iso_image);
    iso_field.set_placeholder("/path/to/image.iso");
    content.add(&iso_field);

    // Boot order.
    let boot_lbl = anyui::Label::new("Boot:");
    boot_lbl.set_position(16, 188);
    boot_lbl.set_size(80, 24);
    boot_lbl.set_text_color(0xFFE6E6E6);
    content.add(&boot_lbl);

    let boot_seg = anyui::SegmentedControl::new("Disk|CD|Floppy");
    boot_seg.set_position(100, 184);
    boot_seg.set_size(300, 28);
    let boot_idx = match config.boot_order {
        BootOrder::DiskFirst => 0u32,
        BootOrder::CdFirst => 1,
        BootOrder::FloppyFirst => 2,
    };
    boot_seg.set_state(boot_idx);
    content.add(&boot_seg);

    // Buttons.
    let save_btn = anyui::Button::new("Save");
    save_btn.set_position(220, 330);
    save_btn.set_size(80, 30);
    content.add(&save_btn);

    let cancel_btn = anyui::Button::new("Cancel");
    cancel_btn.set_position(310, 330);
    cancel_btn.set_size(80, 30);
    content.add(&cancel_btn);

    win.add(&content);

    // Wire up RAM slider value display.
    let ram_val_id = ram_value_label.id();
    ram_slider.on_value_changed(move |e| {
        let ram_mb = 16 + (e.value as u32) * 496 / 100;
        let ram_mb = ((ram_mb + 8) / 16) * 16;
        let mut buf = [0u8; 16];
        let s = fmt_label_val(&mut buf, "", ram_mb, " MB");
        anyui::Control::from_id(ram_val_id).set_text(s);
    });

    // Save button handler.
    save_btn.on_click(|_| {
        save_settings();
    });

    // Cancel button handler.
    cancel_btn.on_click(|_| {
        close_settings_dialog();
    });

    // Close window handler.
    win.on_close(|_| {
        close_settings_dialog();
    });

    a.settings = Some(SettingsDialog {
        win,
        name_field,
        ram_slider,
        ram_value_label,
        disk_field,
        iso_field,
        boot_seg,
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
        // Read name.
        let mut name_buf = [0u8; 64];
        let name_len = dlg.name_field.get_text(&mut name_buf);
        let name = bytes_to_string(&name_buf[..name_len as usize]);

        // Read RAM from slider position.
        let slider_val = dlg.ram_slider.get_state();
        let ram_mb = 16 + (slider_val as u32) * 496 / 100;
        let ram_mb = ((ram_mb + 8) / 16) * 16;

        // Read disk path.
        let mut disk_buf = [0u8; 256];
        let disk_len = dlg.disk_field.get_text(&mut disk_buf);
        let disk_image = bytes_to_string(&disk_buf[..disk_len as usize]);

        // Read ISO path.
        let mut iso_buf = [0u8; 256];
        let iso_len = dlg.iso_field.get_text(&mut iso_buf);
        let iso_image = bytes_to_string(&iso_buf[..iso_len as usize]);

        // Read boot order.
        let boot_order = match dlg.boot_seg.get_state() {
            1 => BootOrder::CdFirst,
            2 => BootOrder::FloppyFirst,
            _ => BootOrder::DiskFirst,
        };

        // Apply to the selected VM config.
        let config = &mut a.vms[a.selected_vm].config;
        if !name.is_empty() {
            config.name = name;
        }
        config.ram_mb = ram_mb.max(16).min(512);
        config.disk_image = disk_image;
        config.iso_image = iso_image;
        config.boot_order = boot_order;

        // Persist and refresh UI.
        save_config(&a.vms);
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
    a.vms.push(VmEntry {
        config,
        handle: None,
        state: VmState::Stopped,
    });

    a.selected_vm = a.vms.len() - 1;

    save_config(&a.vms);
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
/// 3. Update the canvas with the current VGA framebuffer content.
/// 4. Update the info labels with CPU state.
fn vm_tick() {
    let a = app();

    if a.selected_vm >= a.vms.len() {
        return;
    }

    let entry = &mut a.vms[a.selected_vm];
    if entry.state != VmState::Running {
        return;
    }

    if let Some(ref handle) = entry.handle {
        // Advance PIT and deliver timer interrupts.
        for _ in 0..PIT_TICKS_PER_FRAME {
            if handle.pit_tick() {
                handle.pic_raise_irq(0);
            }
        }

        // Run the VM for a batch of instructions.
        let exit = handle.run(INSTRUCTIONS_PER_TICK);

        match exit {
            ExitReason::Halted => {
                entry.state = VmState::Stopped;
                a.canvas.clear(0xFF1E1E1E);
                a.status_label.set_text("VM halted");
                a.status_label.set_text_color(0xFF999999);
            }
            ExitReason::Exception => {
                entry.state = VmState::Stopped;
                a.canvas.clear(0xFF1E1E1E);
                a.status_label.set_text("VM stopped: unrecoverable exception");
                a.status_label.set_text_color(0xFFFF4040);
            }
            ExitReason::InstructionLimit => {
                // Normal: ran out of instruction budget, continue next tick.
            }
            ExitReason::StopRequested => {
                entry.state = VmState::Stopped;
                a.canvas.clear(0xFF1E1E1E);
            }
            ExitReason::Breakpoint => {
                entry.state = VmState::Paused;
            }
        }

        // Update the VGA display if the VM is still running.
        if entry.state == VmState::Running {
            if let Some(text_buf) = handle.vga_text_buffer() {
                render_text_mode(&a.canvas, text_buf);
            } else if let Some((fb, w, h, bpp)) = handle.vga_framebuffer() {
                render_graphics_mode(&a.canvas, fb, w, h, bpp);
            }
        }

        // Update info labels.
        update_info_labels();

        // Refresh sidebar if VM state changed.
        if entry.state != VmState::Running {
            if entry.state == VmState::Stopped {
                entry.handle = None;
            }
            rebuild_sidebar();
            update_status_bar();
        }
    }
}

// ── Main entry point ───────────────────────────────────────────────────

fn main() {
    // Initialize the UI framework.
    if !anyui::init() {
        return;
    }

    // Initialize the VM library.
    let corevm_available = libcorevm_client::init();
    if !corevm_available {
        anyos_std::println!("vmmanager: libcorevm not available, VM execution disabled");
    }

    // ── Main window ────────────────────────────────────────────────

    let win = anyui::Window::new("VM Manager", -1, -1, WIN_W, WIN_H);

    // ── Toolbar (DOCK_TOP) ─────────────────────────────────────────

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 40);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(6, 6, 6, 6);

    let title_lbl = toolbar.add_label("VM Manager");
    title_lbl.set_text_color(0xFF00C8FF);
    title_lbl.set_size(100, 28);
    title_lbl.set_font_size(14);

    toolbar.add_separator();

    let btn_new = toolbar.add_button("New VM");
    btn_new.set_size(70, 28);

    let btn_start = toolbar.add_button("Start");
    btn_start.set_size(60, 28);

    let btn_stop = toolbar.add_button("Stop");
    btn_stop.set_size(60, 28);

    let btn_settings = toolbar.add_button("Settings");
    btn_settings.set_size(70, 28);

    let btn_delete = toolbar.add_button("Delete");
    btn_delete.set_size(60, 28);

    win.add(&toolbar);

    // ── Status bar (DOCK_BOTTOM) ───────────────────────────────────

    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 28);
    status_bar.set_color(0xFF252526);

    let status_label = anyui::Label::new("Ready | 0 VM running");
    status_label.set_position(8, 4);
    status_label.set_size(WIN_W - 16, 20);
    status_label.set_text_color(0xFF999999);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Sidebar (DOCK_LEFT) ────────────────────────────────────────

    let sidebar = anyui::View::new();
    sidebar.set_dock(anyui::DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, WIN_H);
    sidebar.set_color(0xFF2C2C2E);

    let sidebar_title = anyui::Label::new("My VMs");
    sidebar_title.set_position(8, 4);
    sidebar_title.set_size(SIDEBAR_W - 16, 24);
    sidebar_title.set_text_color(0xFF00C8FF);
    sidebar_title.set_font_size(13);
    sidebar.add(&sidebar_title);

    win.add(&sidebar);

    // ── Main content area (DOCK_FILL) ──────────────────────────────

    let content_view = anyui::View::new();
    content_view.set_dock(anyui::DOCK_FILL);
    content_view.set_color(0xFF1E1E1E);

    // Canvas for VGA display.
    let canvas = anyui::Canvas::new(CANVAS_W, CANVAS_H);
    canvas.set_position(8, 8);
    canvas.set_size(CANVAS_W, CANVAS_H);
    canvas.clear(0xFF1E1E1E);
    canvas.set_interactive(true);
    content_view.add(&canvas);

    // VM info panel (below the canvas).
    let info_y = CANVAS_H as i32 + 16;

    let state_label = anyui::Label::new("State: No VM selected");
    state_label.set_position(8, info_y);
    state_label.set_size(320, 20);
    state_label.set_text_color(0xFF999999);
    state_label.set_font_size(13);
    content_view.add(&state_label);

    let mode_label = anyui::Label::new("Mode: -");
    mode_label.set_position(8, info_y + 24);
    mode_label.set_size(320, 20);
    mode_label.set_text_color(0xFFE6E6E6);
    mode_label.set_font_size(13);
    content_view.add(&mode_label);

    let ram_label = anyui::Label::new("RAM: -");
    ram_label.set_position(340, info_y);
    ram_label.set_size(200, 20);
    ram_label.set_text_color(0xFFE6E6E6);
    ram_label.set_font_size(13);
    content_view.add(&ram_label);

    let insn_label = anyui::Label::new("Instructions: -");
    insn_label.set_position(340, info_y + 24);
    insn_label.set_size(300, 20);
    insn_label.set_text_color(0xFFE6E6E6);
    insn_label.set_font_size(13);
    content_view.add(&insn_label);

    win.add(&content_view);

    // ── Load saved VMs ─────────────────────────────────────────────

    let vms = load_config();

    // ── Initialize global state ────────────────────────────────────

    unsafe {
        APP = Some(AppState {
            win,
            sidebar,
            canvas,
            toolbar,
            status_label,
            info: VmInfoLabels {
                state_label,
                mode_label,
                ram_label,
                insn_label,
            },
            content_view,
            sidebar_labels: Vec::new(),
            vms,
            selected_vm: 0,
            settings: None,
            corevm_available,
        });
    }

    // Build the initial sidebar.
    rebuild_sidebar();
    update_info_labels();
    update_status_bar();

    // ── Event handlers ─────────────────────────────────────────────

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

    // Window keyboard handler: forward keys to the VM when running.
    app().win.on_key_down(|ke| {
        let a = app();

        // Forward to running VM.
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running {
                if let Some(ref handle) = entry.handle {
                    let scancode = keycode_to_scancode(ke.keycode);
                    if scancode != 0 {
                        handle.ps2_key_press(scancode);
                        handle.ps2_key_release(scancode);
                        return;
                    }
                }
            }
        }

        // App-level keyboard shortcut: Escape quits.
        if ke.keycode == anyui::KEY_ESCAPE {
            anyui::quit();
        }
    });

    // Canvas mouse handlers: forward to VM when running.
    app().canvas.on_mouse_down(|_x, _y, button| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running {
                if let Some(ref handle) = entry.handle {
                    let ps2_buttons = match button {
                        1 => 0x01, // left
                        2 => 0x04, // middle
                        3 => 0x02, // right
                        _ => 0,
                    };
                    handle.ps2_mouse_move(0, 0, ps2_buttons);
                }
            }
        }
    });

    app().canvas.on_mouse_up(|_x, _y, _button| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running {
                if let Some(ref handle) = entry.handle {
                    handle.ps2_mouse_move(0, 0, 0);
                }
            }
        }
    });

    app().canvas.on_mouse_move(|x, y| {
        let a = app();
        if a.selected_vm < a.vms.len() {
            let entry = &a.vms[a.selected_vm];
            if entry.state == VmState::Running {
                if let Some(ref handle) = entry.handle {
                    // Track relative mouse movement using statics.
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
                        handle.ps2_mouse_move(dx as i16, dy as i16, 0);
                    }
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
                if let Some(ref handle) = entry.handle {
                    handle.request_stop();
                }
                entry.handle = None;
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
