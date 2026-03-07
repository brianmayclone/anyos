# CoreVM VMManager Design

## Overview

Cross-platform VM management application for Linux and Windows, built with egui.
Visually inspired by VirtualBox/VMware. Uses libcorevm in-process for VM execution.

## Project Structure

```
corevm/
  vmmanager/           # Single Cargo project, cross-platform via #[cfg]
    Cargo.toml
    src/
      main.rs          # Entry point, eframe::run_native
      app.rs           # AppState, top-level UI routing
      vm.rs            # VM lifecycle (create/start/stop/pause), libcorevm thread
      config.rs        # VmConfig load/save (key=value .conf, anyOS-compatible)
      display.rs       # Framebuffer -> egui TextureHandle (all VGA modes)
      input.rs         # Keyboard/mouse -> PS/2 scancodes
      sidebar.rs       # VM list with folders, TreeView
      settings.rs      # Settings dialog (tabs: General, Devices, Boot)
      toolbar.rs       # Toolbar (Start/Stop/Pause/Settings/Snapshot)
      statusbar.rs     # Status bar (MIPS, IPC, CPU mode, JIT stats)
      dialogs.rs       # Create VM, Create Disk, Snapshots, Network config
      theme.rs         # Dark theme, custom styling
      platform.rs      # #[cfg]-based paths, BIOS search, OS-specific
    assets/icons/      # App icon, toolbar icons, VM status icons
    linux/             # .desktop file, packaging scripts
    win64/             # .rc/.ico, installer config
  tools/
    build_linux.sh     # Build + package for Linux
    build_win64.bat    # Build + package for Windows
```

## Architecture

- **GUI**: egui + eframe (GPU-accelerated, cross-platform)
- **VM Engine**: libcorevm + libcorevm_client as Rust path dependencies (feature `host_test`)
- **Threading**: VM runs in dedicated thread, framebuffer shared via `Arc<Mutex<FrameBuffer>>`
- **Config**: key=value `.conf` files (compatible with anyOS VMManager format)

## UI Layout

```
+-----------------------------------------------------+
| Menu Bar (File | VM | View | Help)                  |
+----------+------------------------------------------+
| Toolbar  |  > Start  || Pause  [] Stop  * Settings  |
+----------+------------------------------------------+
| Sidebar  |                                          |
|          |  VM Display (Framebuffer / Text Mode)     |
| [Dev]    |                                          |
|  +- VM1  |  Or when no VM running:                  |
|  +- VM2  |  Summary panel with VM details           |
| [Test]   |  (RAM, CPU, Disk, ISO, status)           |
|  +- VM3  |                                          |
+----------+------------------------------------------+
| Status: Running | 450 MIPS | IPC: 12.3 | Long Mode |
+-----------------------------------------------------+
```

## Features (v1)

1. VM Management: create, clone, delete, folder grouping
2. Configuration: RAM (slider), CPU cores, boot order, BIOS (CoreVM/SeaBIOS), JIT toggle
3. Storage: raw disk image creation, ISO attach
4. Display: live framebuffer (text 80x25, mode 13h, 640x480x16, VBE linear), scaled to window
5. Input: keyboard (keycodes -> PS/2 scancodes), mouse (relative + buttons)
6. Network UI: NAT/bridge config, MAC address
7. Snapshots UI: create/restore (UI placeholder, backend later)
8. Monitoring: MIPS, IPC, CPU mode, JIT stats in status bar
9. Theme: dark, modern design

## Framebuffer Integration

- VM thread polls `vga_framebuffer()` / `vga_text_buffer()` at ~60Hz
- Converts to RGBA32 in shared buffer
- Main thread creates `TextureHandle`, renders as `egui::Image`
- Dirty flag avoids unnecessary texture uploads
- Supports 4/8/16/24/32 bpp conversion

## VGA Mode Support (from libcorevm)

| Mode | Resolution | BPP | Notes |
|------|-----------|-----|-------|
| Text80x25 | 80x25 chars | n/a | Character + attribute cells |
| Graphics320x200x256 | 320x200 | 8 | VGA Mode 13h |
| Graphics640x480x16 | 640x480 | 4 | Standard VGA |
| LinearFramebuffer | arbitrary | 8/16/24/32 | Bochs VBE |

## Input Handling

- Keyboard: platform keycodes mapped to PS/2 scancode set 2
- Mouse: egui pointer events -> relative deltas -> `ps2_mouse_move(dx, dy, buttons)`
- Mouse capture mode: click to capture, Ctrl+Alt to release

## Config Format (anyOS-compatible)

```
name=My VM
ram=512
cpu_cores=4
disk=/path/to/disk.img
iso=/path/to/iso.iso
boot=disk
bios=corevm
jit=1
ram_alloc=ondemand
gpu=svga
net_enabled=1
net_mode=nat
net_host_nic=
mac_mode=dynamic
mac_address=
```

## Dependencies

```toml
[dependencies]
eframe = "0.31"
egui = "0.31"
egui_extras = "0.31"
libcorevm = { path = "../../libs/libcorevm", features = ["host_test"] }
libcorevm_client = { path = "../../libs/libcorevm_client" }
uuid = { version = "1", features = ["v4"] }
rfd = "0.15"
```

## Platform Differences (#[cfg])

| Concern | Linux | Windows |
|---------|-------|---------|
| Config dir | `~/.config/corevm/vms/` | `%APPDATA%\CoreVM\vms\` |
| BIOS search | `/usr/share/corevm/` | `.\bios\` |
| File dialogs | rfd (GTK backend) | rfd (Win32 backend) |
| Packaging | .deb/.AppImage | .msi/.exe installer |
