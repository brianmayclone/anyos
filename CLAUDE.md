# anyOS Project Notes

## Build System

- **Build**: `cd build && cmake .. -G Ninja && ninja`
- **Rebuild after .asm changes**: Cargo may not detect .asm changes -- run `touch kernel/build.rs` then rebuild if needed
- **Cross-compiler**: `i686-elf-gcc` / `i686-elf-ar` must be in PATH (`$HOME/opt/cross/bin`)
- **Host tools** (anyld, mkimage, anyelf): Built with `cc -std=c99`; on Linux add `-D_POSIX_C_SOURCE=200809L` (defined via `POSIX_FLAG` in CMakeLists.txt -- must be set BEFORE tool definitions)

## Architecture

- **Kernel**: Rust (`#![no_std]`), x86-64 long mode, higher-half at 0xFFFFFFFF80000000
- **User programs**: Two modes:
  - **Rust programs** (`bin/`): `#![no_std]`, `#![no_main]`, use `anyos_std` crate, built with `add_rust_user_program()` in CMakeLists.txt, installed to `/System/bin/`
  - **C programs** (`bin/` with Makefile): Cross-compiled with `i686-elf-gcc`, run in 32-bit compat mode (CS=0x1B), use INT 0x80 syscalls
- **GUI apps** (`apps/`): `#![no_std]` Rust, use `libanyui_client` for GUI, built with `add_rust_app()` in CMakeLists.txt, installed to `/Applications/`
- **GDT layout**: Null(0x00), KernelCode64(0x08), KernelData(0x10), UserCode32(0x18), UserData(0x20), UserCode64(0x28), TSS(0x30+)
- **Segment selectors with RPL**: Kernel CS=0x08, Kernel DS=0x10, User CS32=0x1B, User DS=0x23, User CS64=0x2B

## Critical: DS/ES Restoration on Ring Transitions

When returning from kernel (CPL 0) to user mode (CPL 3) via IRETQ, DS/ES **must** be restored to 0x23 (user data segment) before IRETQ. The CPU nulls DS/ES when DPL < new CPL on privilege transitions. IRETQ does NOT restore DS/ES automatically.

Affected files:
- `kernel/asm/syscall_entry.asm` -- INT 0x80 handler
- `kernel/asm/interrupts.asm` -- ISR and IRQ stubs
- `kernel/src/task/loader.rs` -- jump_to_user_mode, trampolines, fork_return_to_user
- `kernel/asm/syscall_fast.asm` -- 64-bit SYSCALL (DS/ES not needed in long mode)

## Common Pitfalls

- **strdup() on Linux with -std=c99**: Needs `-D_POSIX_C_SOURCE=200809L` or strdup() is undeclared, causing pointer truncation (64-bit pointer returned as int -> segfault)
- **QEMU VM caching**: Rebuilding `anyos.img` on disk does NOT update a running QEMU VM -- must restart QEMU
- **PATH issues**: Ensure `$HOME/opt/cross/bin` is in PATH and no `.` entries that could pick up wrong tools
- **Toolbar sizing**: Toolbar widget defaults to size (0,0) and is invisible. Always set: `toolbar.set_size(800, 36)`, `toolbar.set_color(0xFF252526)`, `toolbar.set_padding(4, 4, 4, 4)`, and explicit `set_size()` on each button
- **IconButton text+icon overflow**: At 28px height, icon (16px centered) + text below overflows. Either use text-only or icon-only, not both, for small heights
- **DOCK_FILL must be added last**: Docking order matters. Add DOCK_TOP/BOTTOM/LEFT/RIGHT controls first, then DOCK_FILL last
- **Multiple DOCK_BOTTOM**: First added = bottommost. Second added = above it. Useful for stacking status bar + edit panels.

## Project Structure

### Kernel
- `kernel/` -- Rust x86-64 kernel with no_std

### Libraries
- `libs/stdlib/` -- anyos_std: Core no_std standard library (Vec, String, HashMap, fs, process, args)
- `libs/dynlink/` -- Dynamic library loading (dl_open, dl_sym)
- `libs/libanyui/` -- Server-side GUI framework (44 controls, 178 exports, runs in compositor process)
- `libs/libanyui_client/` -- Client-side GUI wrapper (user apps link against this)
- `libs/libcompositor/` / `libs/libcompositor_client/` -- Low-level compositor protocol
- `libs/libfont/` / `libs/libfont_client/` -- Font rendering
- `libs/libimage/` / `libs/libimage_client/` -- Image decoding (BMP, PNG, JPEG, GIF, ICO)
- `libs/librender/` / `libs/librender_client/` -- 2D rendering primitives
- `libs/libdb/` / `libs/libdb_client/` -- Key-value database
- `libs/libzip/` / `libs/libzip_client/` -- ZIP/TAR/GZIP archive handling
- `libs/libhttp/` / `libs/libhttp_client/` -- HTTP client/server
- `libs/libsvg/` / `libs/libsvg_client/` -- SVG rasterizer
- `libs/libjs/` -- JavaScript engine
- `libs/libwebview/` -- HTML/CSS/JS rendering engine
- `libs/libm/` / `libs/libm_client/` -- Hardware-accelerated math (SSE2 + x87 FPU)
- `libs/libgl/` / `libs/libgl_client/` -- OpenGL ES 2.0 3D engine
- `libs/libcorevm/` / `libs/libcorevm_client/` -- CoreVM x86 virtual machine engine
- `libs/libheap/` -- Heap allocator
- `libs/libsyscall/` -- Low-level syscall interface
- `libs/libunwind/` -- Stack unwinding support
- `libs/libc/` / `libs/libc64/` -- C standard library (32-bit and 64-bit)
- `libs/libcxx/` / `libs/libcxxabi/` -- C++20 standard library
- `libs/uisys/` / `libs/uisys_client/` -- Legacy UI system (deprecated, use libanyui_client)

### CLI Programs (`bin/`) -- 105 programs
- Standard Unix-like tools: ls, cat, cp, mv, rm, mkdir, grep, find, head, tail, sort, tar, gzip, zip, unzip, etc.
- Network tools: ping, ssh, sshd, scp, wget, ftp, ftpd, curl, dhcp, dns, ifconfig, arp, netstat, vncd, httpd, openvpn
- System: ps, top, htop, mount, umount, sysinfo, dmesg, neofetch, kill, killall
- Editors: nano, vi, nvi, sed, awk
- Package manager: ami, apkg
- Version control: git
- Dev tools: cc (TCC), nasm, make, jscript
- VM: vmd (CoreVM daemon), vmctl (AI-friendly CoreVM CLI controller)

### GUI Applications (`apps/`) -- 27 apps
- `anycode/` -- Code editor (VSCode-like, reference anyui app)
- `anymail/` -- Email client (IMAP/SMTP, address book, autocomplete)
- `anyzilla/` -- FTP client (FileZilla-like, dual-pane, PASV transfers)
- `diff/` -- Diff/merge tool (Meld-like, syntax highlighting, themes)
- `paint/` -- Paint application (Canvas-based)
- `notepad/` -- Simple text editor
- `calc/` -- Calculator
- `clock/` -- Clock widget
- `imgview/` -- Image viewer
- `iconview/` -- Icon viewer
- `fontviewer/` -- Font browser
- `minesweeper/` -- Minesweeper game
- `surf/` -- Web browser
- `videoplayer/` -- Video player
- `diagnostics/` -- System diagnostics
- `screenshot/` -- Screenshot tool
- `clipman/` -- Clipboard history manager (timer-polled, JSON-persistent per user)
- `mdview/` -- Markdown viewer
- `vmmanager/` -- VM Manager (create, configure, run VMs)
- `store/` -- App Store
- `anybench/` -- Benchmarking tool
- `gldemo/` -- OpenGL ES 2.0 demo
- `ftp-settings/` -- FTP server settings
- `vnc-settings/` -- VNC server settings
- `webmanager/` -- Web manager
- `demo_anyui/` -- anyui widget demo/showcase
- `button_demo/` -- Button demo

## Documentation
- `docs/anyui-api.md` -- anyui widget library reference (44 controls, 178 exports, events, layout, dialogs, icons)
- `docs/syscalls.md` -- Kernel syscall reference (183 syscalls)
- `docs/stdlib-api.md` -- anyos_std library reference
- `docs/architecture.md` -- System architecture overview
- `docs/services.md` -- System services documentation
- `docs/ami.md` -- Package manager / system info daemon documentation
- `docs/libc-api.md` -- C library reference (32-bit)
- `docs/libcxx-api.md` -- 64-bit C (libc64) and C++20 (libcxx) standard library reference
- `docs/libcompositor-api.md` -- Compositor protocol reference
- `docs/libfont-api.md` -- Font rendering API
- `docs/libimage-api.md` -- Image decoding API
- `docs/librender-api.md` -- 2D rendering API
- `docs/libdb-api.md` -- Key-value database API
- `docs/libzip-api.md` -- ZIP/TAR/GZIP archive API
- `docs/libsvg-api.md` -- SVG rasterizer API
- `docs/libjs-api.md` -- JavaScript engine API
- `docs/libwebview-api.md` -- HTML/CSS/JS rendering engine API
- `docs/libgl-api.md` -- OpenGL ES 2.0 3D engine API
- `docs/libm-api.md` -- Hardware-accelerated math API
- `docs/corevm-api.md` -- CoreVM x86 virtual machine API
- `docs/vmctl.md` -- vmctl CLI tool reference (AI-friendly VM controller)
- `docs/uisys-api.md` -- Legacy UI system (deprecated)

## User Program Template (Rust CLI)

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");
    anyos_std::println!("Hello from anyOS!");
}
```

Cargo.toml:
```toml
[package]
name = "progname"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }

[profile.dev]
panic = "abort"
opt-level = 2

[profile.release]
panic = "abort"
```

Register in CMakeLists.txt:
```cmake
add_rust_user_program(progname ${CMAKE_SOURCE_DIR}/bin/progname)
```

## GUI App Template (anyui)

```rust
#![no_std]
#![no_main]

use anyos_std::{String, Vec};
use libanyui_client as anyui;

anyos_std::entry!(main);

struct AppState {
    // UI widget handles and app data
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

fn main() {
    if !anyui::init() { return; }

    let win = anyui::Window::new("My App", -1, -1, 800, 500);

    // Toolbar (DOCK_TOP, must set size explicitly)
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(800, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);
    let btn = toolbar.add_icon_button("Action");
    btn.set_size(80, 28);
    win.add(&toolbar);

    // Status bar (DOCK_BOTTOM, add before DOCK_FILL)
    let status = anyui::View::new();
    status.set_dock(anyui::DOCK_BOTTOM);
    status.set_size(800, 24);
    status.set_color(0xFF252525);
    win.add(&status);

    // Content area (DOCK_FILL, add last)
    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    win.add(&content);

    // Initialize global state
    unsafe { APP = Some(AppState { /* ... */ }); }

    // Register callbacks
    btn.on_click(|_| { /* handle click */ });
    win.on_close(|_| { anyui::quit(); });

    anyui::run();
}
```

Cargo.toml:
```toml
[package]
name = "myapp"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libanyui_client = { path = "../../libs/libanyui_client" }

[profile.dev]
panic = "abort"
opt-level = 2

[profile.release]
panic = "abort"
```

Register in CMakeLists.txt:
```cmake
add_rust_app(myapp ${CMAKE_SOURCE_DIR}/apps/myapp "My App" "1.0")
```

## Key Patterns

### Global State Singleton (used by all GUI apps)
```rust
static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }
```

### File I/O
```rust
// Read file
let fd = anyos_std::fs::open(path, 0);
let mut buf = [0u8; 512];
let n = anyos_std::fs::read(fd, &mut buf);
anyos_std::fs::close(fd);

// Write file
anyos_std::fs::write_bytes(path, data).is_ok();

// Directory listing
let mut buf = [0u8; 64 * 128];
let count = anyos_std::fs::readdir(path, &mut buf);
// Each entry: [type:u8, name_len:u8, flags:u8, pad:u8, size:u32, name:56bytes]
```

### FileDialog
```rust
if let Some(path) = anyui::FileDialog::open_file() {
    // User selected a file
}
if let Some(path) = anyui::FileDialog::save_file("default.txt") {
    // User chose save location
}
```

### JSON Persistence
```rust
use anyos_std::json::Value;

// Parse
let val = Value::parse(json_str).unwrap();
let name = val["key"].as_str().unwrap_or("");     // Index operator returns Null if missing
let num = val["count"].as_i64().unwrap_or(0);
if let Some(arr) = val["items"].as_array() { /* iterate */ }

// Build
let mut obj = Value::new_object();
obj.set("key", "value".into());                    // .set() not .insert()
obj.set("num", (42i64).into());                    // .into() for type inference
let mut arr = Value::new_array();
arr.push(item);                                     // .push() for arrays
obj.set("items", arr);
let json = obj.to_json_string_pretty();
```

### Clipboard API
```rust
// Read clipboard
let mut buf = [0u8; 4096];
let len = anyui::clipboard_get(&mut buf);           // Returns 0 if empty
let text = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");

// Write clipboard
anyui::clipboard_set("text to copy");
```

### DataGrid with Per-Cell Colors
```rust
let grid = anyui::DataGrid::new(600, 400);
grid.set_columns(&[
    anyui::ColumnDef::new("Name").width(200),
    anyui::ColumnDef::new("Value").width(100).align(anyui::ALIGN_RIGHT).numeric(),
]);
grid.set_row_count(n as u32);
grid.set_data_raw(&data_buf);            // 0x1E=row sep, 0x1F=col sep
grid.set_cell_colors(&text_colors);       // Flat array: row * cols + col
grid.set_cell_bg_colors(&bg_colors);      // Same indexing
```

## Network Architecture

- **Loopback interface** (`lo` 127.0.0.1/255.0.0.0) defined in `/System/etc/network/interfaces`
- **Own-IP loopback**: Packets to the host's own configured IP are routed via loopback (not ARP)
- **FTP server** (`ftpd`): Config at `/System/etc/ftpd/ftpd.conf` and `/System/etc/ftpd/shares.conf`
  - PASV mode default, auto-creates share directories on startup
  - Loopback-aware: PASV reply uses 127.0.0.1 when client connects via loopback
- **Interface config**: `/System/etc/network/interfaces` parsed by `kernel/src/net/interfaces.rs`
  - Supports `static`, `dhcp`, and `loopback` methods
  - Auto-injects `lo` if not present in config file

## App Capabilities

GUI apps in `apps/` have an `Info.conf` that lists required capabilities. Missing capabilities cause crashes.
Common capabilities: `filesystem`, `network`, `display`, `dll`, `event`, `thread`, `pipe`, `shm`, `process`

Example (`apps/anyzilla/Info.conf`):
```
capabilities=filesystem,network,display,dll,event,thread,pipe,shm,process
```

**Important**: Apps using `Thread::spawn_with_stack()` or `process::kill()` need the `process` capability.

---

## CoreVM Testing (Linux/KVM)

### Build vmctl (Host-Tool)

```bash
cd corevm/vmctl && cargo +stable build --release
```

Binary: `corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl`

**Wichtig**: Muss mit `cargo +stable` gebaut werden (nightly hat doppeltes alloc-Problem).
Das `linux`-Feature wird automatisch via Cargo.toml aktiviert.

### Voraussetzungen

- KVM aktiviert (`/dev/kvm` vorhanden)
- SeaBIOS: `/usr/share/seabios/bios.bin` + `vgabios.bin` (oder `/usr/share/qemu/`)
- Fuer Direct Kernel Boot: `/usr/share/qemu/linuxboot_dma.bin`

### Test-ISOs

- `corevm/test-isos/TinyCore-current.iso` — TinyCore Linux (~25MB, 32-bit, bootet komplett im RAM)
- `corevm/test-isos/ventoy-1.1.10-livecd.iso` — Ventoy Live-CD
- `corevm/test-isos/memtest86+.iso` — Memtest86+
- `corevm/test-isos/Win7Pro32bit.iso` — Windows 7 Pro 32-bit (braucht `--hpet`)

### Testmethodik: Ausgabe in Datei schreiben

**WICHTIG**: Testausgabe IMMER zuerst in eine Datei schreiben, danach mit grep/read analysieren. So kann man mehrfach in das Ergebnis schauen ohne den Test neu laufen zu lassen.

```bash
# 1. Test ausfuehren und GESAMTE Ausgabe (stdout+stderr) in Datei speichern:
corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl run \
  -r 512 -i corevm/test-isos/TinyCore-current.iso -b seabios -t 40 -s \
  --key 4000:enter > /tmp/corevm-test.log 2>&1

# 2. Danach gezielt nach Ergebnissen suchen:
grep -E '\[acpi\]' /tmp/corevm-test.log          # ACPI-Tabellen pruefen
grep -E '\[lapic\]' /tmp/corevm-test.log          # LAPIC-Status
grep -E '\[ioapic\].*pin' /tmp/corevm-test.log    # IOAPIC konfigurierte Pins
grep -E '\[vga-text\]' /tmp/corevm-test.log       # VGA-Text (Boot-Meldungen)
grep -E '\[kernel-scan\]' /tmp/corevm-test.log    # Kernel-Messages im RAM
grep -E 'vm-state.*PG=1' /tmp/corevm-test.log     # Kernel im Paging-Modus?
grep -E 'exit_reason' /tmp/corevm-test.log         # Wie hat die VM beendet?
grep -E '\[fw_cfg\].*DMA' /tmp/corevm-test.log    # fw_cfg DMA-Transfers
```

### TinyCore Linux booten (Referenztest)

TinyCore (32-bit) ist der primaere Regressionstest. Muss IMMER funktionieren.

```bash
# TinyCore booten: 512MB RAM, 40s Timeout, Enter nach 4s fuer Boot-Prompt
corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl run \
  -r 512 -i corevm/test-isos/TinyCore-current.iso -b seabios -t 40 -s \
  --key 4000:enter > /tmp/corevm-tinycore.log 2>&1

# Erwartetes Ergebnis pruefen:
grep '\[lapic\] Timer:' /tmp/corevm-tinycore.log
# -> Timer: vector=0xec mode=one-shot masked=0   (Timer laeuft!)
grep '\[ioapic\] pin' /tmp/corevm-tinycore.log
# -> pin 0 (timer), pin 1 (kbd), pin 8 (RTC), pin 9 (ACPI), pin 11 (AHCI)
grep 'fb_nonzero=[1-9]' /tmp/corevm-tinycore.log
# -> fb_nonzero=307200  (Grafik sichtbar, 1024x768)
grep 'PG=1' /tmp/corevm-tinycore.log | tail -1
# -> CR0=0x80050033 IF=1 PE=1 PG=1  (Kernel laeuft mit Paging)
```

### Ventoy Live-CD booten (64-bit Test)

Ventoy (64-bit Linux 5.10.25) testet den 64-bit ACPI/APIC-Pfad.

```bash
# Ventoy booten: 2048MB RAM, 55s Timeout, Enter nach 8s fuer GRUB
corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl run \
  -r 2048 -i corevm/test-isos/ventoy-1.1.10-livecd.iso -b seabios -t 55 -s \
  --key 8000:enter > /tmp/corevm-ventoy.log 2>&1

# Erwartetes Ergebnis:
grep '\[lapic\] Timer:' /tmp/corevm-ventoy.log
# -> Timer: vector=0xec mode=one-shot masked=0
grep '\[ioapic\] pin' /tmp/corevm-ventoy.log
# -> 5 Pins konfiguriert (0, 1, 8, 9, 11)
grep 'CS=0x33' /tmp/corevm-ventoy.log | head -1
# -> CS=0x33 = Userspace laeuft (Ring 3, 64-bit)
```

### Windows 7 booten (HPET erforderlich)

Windows 7 (32-bit) braucht das `--hpet` Flag, da Windows HPET als Timer-Quelle benoetigt.

```bash
# Windows 7 booten: 2048MB RAM, 120s Timeout, --hpet fuer HPET-Timer
corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl run \
  -r 2048 -i corevm/test-isos/Win7Pro32bit.iso -b seabios -t 120 -s \
  --hpet --key 6000:enter > /tmp/corevm-win7.log 2>&1

# Erwartetes Ergebnis pruefen:
grep '\[ioapic\] pin 2:' /tmp/corevm-win7.log
# -> pin 2: vec=0xd1 ... mask=0   (HPET Timer auf IRQ 2)
grep 'fb_nz' /tmp/corevm-win7.log | tail -1
# -> fb_nz=316414/...  (Boot-Splash sichtbar)
grep 'HPET enabled' /tmp/corevm-win7.log
# -> [vmctl] HPET enabled (--hpet flag)
```

**WICHTIG**: `--hpet` darf NICHT fuer Linux-Gaeste verwendet werden!
Linux aktiviert HPET Legacy Replacement Mode, welcher den PIT deaktiviert.
Da unser HPET-Timer per Polling (10ms Intervall) arbeitet, kommt der Interrupt
zu spaet fuer Linux's fruehen Timer-Test → Kernel Panic:
"IO-APIC + timer doesn't work! Boot with apic=debug"

### Direct Kernel Boot (wie QEMU -kernel)

Linux-Kernel direkt booten ohne ISO, via fw_cfg + linuxboot_dma.bin:

```bash
corevm/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl run \
  -r 256 -b seabios -t 60 -s \
  -k /path/to/vmlinuz --initrd /path/to/initrd.gz \
  --append "console=ttyS0 quiet" > /tmp/corevm-direct.log 2>&1
```

### Keypress-Injection (Boot-Wartezeiten vermeiden)

Viele ISOs haben Boot-Prompts mit Countdown. `--key` sendet PS/2-Scancodes nach einer Verzoegerung:

```bash
--key 3000:enter              # Enter nach 3 Sekunden
--key 2000:esc --key 5000:enter   # Escape nach 2s, Enter nach 5s
--key 1000:f12                # F12 fuer Boot-Menue
--key 1000:3b                 # Raw Scancode hex (0x3B = F1)
```

Verfuegbare Key-Namen: `enter`, `esc`, `space`, `tab`, `up`, `down`, `left`, `right`, `f1`-`f10`, `f12`

**Wie es funktioniert**: Keys werden doppelt injiziert — als PS/2-Scancode (IRQ 1) UND direkt in den BIOS-Keyboard-Buffer (BDA 0x41E). Das stellt sicher, dass sowohl BIOS INT 16h als auch Guest-OS-Treiber die Eingabe erkennen.

### Diagnostik-Output

vmctl gibt automatisch alle 2 Sekunden einen State-Dump auf stderr aus:
- `[vm-state]` — RIP, CS, CR0, Protected Mode, Paging, VGA-Textzeile
- `[vm-exits]` — Exit-Typ-Verteilung (IO, MMIO, HLT, Cancel)
- `[vm-gfx]` — VGA-Modus, Framebuffer-Status
- `[vm-timer]` — BIOS Tick Counter
- `[fw_cfg]` — fw_cfg Selektor-Zugriffe und DMA-Transfers (Debug-Build)
- `[lapic]` / `[ioapic]` — Interrupt-Controller-Status (bei Exit)
- `[acpi]` — RSDP/RSDT/XSDT Tabellen-Scan (bei Exit)
- `[vga-text]` — Erste 10 Zeilen VGA-Text (bei Exit)
- `[kernel-scan]` — Kernel-Messages im RAM (APIC, panic, etc.)

Framebuffer wird bei Exit als `/tmp/corevm-framebuffer.raw` (1024x768 BGRA) gespeichert.

### HPET (High Precision Event Timer)

HPET-Emulation unter `libs/libcorevm/src/devices/hpet.rs`:
- MMIO-Device bei 0xFED0_0000 (1KB Region)
- ~14.318 MHz Counter (69.841.279 Femtosekunden pro Tick)
- 3 Timer (Timer 0-2), Timer 0 mit periodischem Modus
- Legacy Replacement Mode (Timer 0 ersetzt PIT auf IRQ 2)
- Wall-Clock-basierter Counter (std::time::Instant)

**ACPI-Tabelle**: Wird nur mit `--hpet` Flag generiert (`generate_acpi_tables_with_hpet()`).
Ohne `--hpet` wird `generate_acpi_tables()` ohne HPET-Tabelle verwendet.

**Timer-Delivery**: HPET-Interrupts werden per Polling in `corevm_poll_irqs()` geprueft.
Cancel-Timer: 10ms mit `--hpet` (fuer ~64 Hz HPET), 100ms ohne.
Windows programmiert Timer 0 auf ~300 Hz (Comparator 0xba6f).

**Bekannte Einschraenkung**: HPET Legacy Replacement Mode bricht Linux-Gaeste,
da Linux den PIT deaktiviert und HPET-Interrupts per Polling zu spaet kommen.
Deshalb ist HPET nur per `--hpet` Flag aktivierbar (fuer Windows-Gaeste).

### Bekannte Einschraenkungen

- Kein USB-Passthrough
- VGA nur Textmodus-Dump (kein grafischer Framebuffer-Viewer)
- HPET nur fuer Windows-Gaeste (`--hpet`), bricht Linux (siehe oben)
- Cancel-Timer: 100ms (Standard) bzw. 10ms (mit `--hpet`)

---

## Raspberry Pi 4/5 Port — Implementierungsplan

Ziel: Bootfaehiges SD-Karten-Image das anyOS auf Raspberry Pi 4 (BCM2711) und Pi 5 (BCM2712) startet.

### Ausgangslage

Der ARM64-Port ist bereits weit fortgeschritten (Branch `feat/arm64`):
- **Bereits fertig**: Boot (EL2→EL1), MMU-Setup, GICv3, Generic Timer, SMP (PSCI), Syscall-Dispatch (SVC #0), Kontextwechsel, VMSAv8-A Paging (4-Level), Physical Memory Manager, HAL-Abstraktion, Exception Handling
- **Build-System**: Komplett dual-arch (CMake + Cargo), `aarch64-anyos.json` Target, `link_arm64.ld` Linker-Script
- **Portabel (kein Aenderungsbedarf)**: VFS, TCP/IP-Stack, IPC, Sync-Primitives, Syscall-Handler, Crypto, Graphics

### Delta: QEMU virt → Raspberry Pi 4/5

| Komponente | QEMU virt (aktuell) | RPi 4 (BCM2711) | RPi 5 (BCM2712) |
|---|---|---|---|
| RAM-Basis | `0x40000000` | `0x00000000` | `0x00000000` |
| UART (PL011) | `0x09000000` | `0xFE201000` | `0x107D001000` |
| GIC | GICv3 @ `0x08000000` | GICv2 @ `0xFF841000` | GICv3 |
| SD/eMMC | virtio-blk | EMMC2 @ `0xFE340000` | EMMC2 @ `0x1000FFF000` |
| GPU/FB | virtio-gpu | VideoCore VI (Mailbox) | VideoCore VII (Mailbox) |
| USB | virtio | xHCI (VL805 PCIe) | xHCI (RP1) |
| Ethernet | virtio-net | GENET @ `0xFD580000` | RP1 PCIe NIC |
| Boot | `-kernel` Flag | config.txt + kernel8.img | config.txt + kernel_2712.img |
| SMP | PSCI `hvc #0` | PSCI `smc #0` (EL3 Firmware) | PSCI `smc #0` |
| PCIe | Keins | BCM2711 PCIe root @ `0xFD500000` | RP1 PCIe |

### Phase 1: DTB-Parser + Hardware-Abstraktion (Grundlage)

**Warum zuerst**: Ohne DTB-Parser muss jede MMIO-Adresse hardcoded werden. Ein DTB-Parser macht den Kernel auf RPi4, RPi5 UND QEMU virt gleichzeitig lauffaehig.

- [ ] **DTB/FDT-Parser implementieren** (`kernel/src/drivers/dtb.rs`)
  - X0 enthaelt DTB-Physadresse beim Boot (bereits in `boot.S` gesichert)
  - FDT-Header parsen (magic `0xD00DFEED`, struct/strings/mem-rsvmap Offsets)
  - Node-/Property-Iterator (kein Alloc noetig, arbeitet direkt auf dem DTB-Blob)
  - Properties lesen: `reg`, `compatible`, `#address-cells`, `#size-cells`, `interrupts`
  - Convenience: `find_node("/soc/serial@...")`, `get_property_u32()`, `get_property_str()`
  - **Memory-Detection aus DTB** statt hardcoded 512 MiB — `/memory@0` Node lesen

- [ ] **Board-Detection via DTB** (`kernel/src/arch/arm64/board.rs`)
  - `compatible` Property des Root-Nodes lesen
  - Enum: `Board::QemuVirt`, `Board::RaspberryPi4`, `Board::RaspberryPi5`
  - MMIO-Basisadressen pro Board (UART, GIC, EMMC, Mailbox, GPIO)
  - Globale `BOARD_INFO` Struktur, einmalig beim Boot initialisiert

- [ ] **UART dynamisch konfigurieren**
  - `serial.rs` PL011-Basisadresse aus DTB oder Board-Info statt hardcoded `0x09000000`
  - Fruehe serielle Ausgabe (vor DTB): Kernel-Kommandozeile oder Board-spezifischer Fallback

### Phase 2: GICv2-Treiber (RPi 4)

RPi 4 hat GICv2 (nicht GICv3 wie QEMU virt). RPi 5 hat GICv3, also reicht der bestehende Treiber dort.

- [ ] **GICv2-Treiber** (`kernel/src/arch/arm64/gicv2.rs`)
  - Distributor (GICD): `GICD_CTLR`, `GICD_ISENABLER`, `GICD_ITARGETSR`, `GICD_IPRIORITYR`, `GICD_ICPENDR`
  - CPU Interface (GICC): `GICC_CTLR`, `GICC_PMR`, `GICC_IAR`, `GICC_EOIR`
  - MMIO-Register statt System-Register (anders als GICv3)
  - Gleiche API wie `gic.rs`: `init()`, `enable_irq()`, `ack()`, `eoi()`

- [ ] **GIC-Abstraktion** — `exceptions.rs` waehlt GICv2/v3 je nach Board
  - HAL `irq_eoi()` Stub ausfuellen (derzeit no-op)
  - IPI-Senden via GIC (derzeit Stub in `hal.rs`)

### Phase 3: SD-Karten-Treiber (EMMC2)

Ohne Storage kein Dateisystem, ohne Dateisystem kein Userspace.

- [ ] **EMMC2/SDHCI-Treiber** (`kernel/src/drivers/storage/emmc.rs`)
  - SDHCI-kompatibles Interface (SD Host Controller Spec 3.0)
  - Register: `CMDTM`, `DATA`, `STATUS`, `CONTROL0/1`, `IRPT_MASK`, `IRPT_EN`
  - Initialisierung: Reset, Clock-Setup (400kHz init → 25/50MHz), Voltage, Bus-Width
  - CMD0 (GO_IDLE), CMD8 (SEND_IF_COND), ACMD41 (SD_SEND_OP_COND), CMD2/CMD3 (Identify)
  - CMD17 (READ_SINGLE_BLOCK), CMD18 (READ_MULTIPLE_BLOCK)
  - CMD24 (WRITE_BLOCK), CMD25 (WRITE_MULTIPLE_BLOCK)
  - DMA oder PIO-Modus (PIO zuerst fuer Einfachheit)
  - 512-Byte Sektoren, Block-Device-Interface fuer VFS (`read_sector`, `write_sector`)
  - RPi4 EMMC2: `0xFE340000`, RPi5: `0x1000FFF000` (aus DTB)

- [ ] **Block-Device in VFS einhaengen**
  - `drivers/hal.rs` Storage-Treiber registrieren
  - `fs/mod.rs` Root-Filesystem von SD-Karte mounten (exFAT oder FAT32)

### Phase 4: Userspace auf ARM64

Der Syscall-Pfad ist fertig (SVC #0 → dispatch_inner), aber der Loader startet noch keinen Userspace.

- [ ] **ARM64 User-Mode-Entry** (`kernel/src/task/loader.rs`)
  - `jump_to_user_mode_arm64()`: `ELR_EL1` = Entry-Point, `SPSR_EL1` = EL0 + IRQ-enabled, SP_EL0 = User-Stack, `ERET`
  - ELF-Loader: `is_compat32` ignorieren auf ARM64, nur AArch64-ELFs
  - `fork_return_to_user_arm64()`: Kontext aus `CpuContext` wiederherstellen, `ERET`

- [ ] **FPU/NEON Save/Restore** (`kernel/src/arch/arm64/fpu.rs`)
  - `fpu_save()`: Q0-Q31 (32×128-bit), FPCR, FPSR speichern
  - `fpu_restore()`: Q0-Q31, FPCR, FPSR wiederherstellen
  - HAL-Stubs in `hal.rs` ausfuellen (derzeit no-op)
  - Lazy FPU-Switching: CPACR_EL1.FPEN trap bei erstem FP-Zugriff

- [ ] **Init-Prozess starten**
  - ARM64-Pfad in `kernel_main()` — nach Storage-Init: `/System/bin/init` laden
  - Compositor, Login, Shell-Kette wie auf x86

### Phase 5: Framebuffer / GPU (Grafische Ausgabe)

- [ ] **VideoCore Mailbox Interface** (`kernel/src/drivers/gpu/vc_mailbox.rs`)
  - Mailbox-Register: RPi4 `0xFE00B880`, RPi5 aus DTB
  - Property-Tags Channel (Channel 8): Request/Response Protocol
  - Tag `0x00040001` (Allocate Buffer), `0x00048003` (Set Physical Size), `0x00048004` (Set Virtual Size), `0x00048005` (Set Depth), `0x00040008` (Get Pitch)
  - Framebuffer-Adresse aus Mailbox-Response → MMIO-Mapping

- [ ] **Framebuffer-Treiber** (`kernel/src/drivers/gpu/rpi_fb.rs`)
  - 1920×1080 (oder HDMI EDID), 32bpp BGRA
  - Linear Framebuffer schreiben (kompatibel mit bestehendem `graphics/` Code)
  - Compositor-Integration: Framebuffer-Adresse an `graphics/framebuffer.rs` uebergeben
  - HDMI-Aufloesung aus config.txt oder EDID

### Phase 6: USB + Eingabegeraete

- [ ] **xHCI USB-Treiber** (bereits teilweise vorhanden fuer x86 PCI)
  - RPi4: VL805 xHCI ueber PCIe (braucht minimalen PCIe-Root-Complex-Treiber)
  - RPi5: RP1-integrierter xHCI
  - USB-HID: Tastatur + Maus fuer Compositor

- [ ] **GPIO / UART-Konsole als Fallback-Input**
  - Serielle Konsole ueber PL011 (bereits implementiert)
  - Nutzbar fuer headless Betrieb oder Debug

### Phase 7: Netzwerk

- [ ] **GENET Ethernet-Treiber** (RPi 4) (`kernel/src/drivers/net/genet.rs`)
  - BCM54213PE PHY, MMIO @ `0xFD580000`
  - DMA-Deskriptor-Ringe (TX/RX)
  - MDIO fuer PHY-Konfiguration (Auto-Negotiation, Link-Status)
  - MAC-Adresse aus OTP oder DTB

- [ ] **RPi 5 Netzwerk** — RP1-integriert, PCIe-Anbindung
  - Aehnlich wie GENET aber andere Register-Map

- [ ] **WiFi (optional)** — RPi hat Broadcom BCM43xx WiFi
  - Braucht Firmware-Blob + WPA Supplicant — deutlich aufwaendiger
  - Erstmal nur Ethernet, WiFi spaeter

### Phase 8: SD-Karten-Image erstellen

- [ ] **Boot-Partition (FAT32)** — RPi Firmware erwartet:
  - `bootcode.bin` (nur RPi4, RPi5 hat SPI-Flash)
  - `start4.elf` / `start4cd.elf` (VideoCore Firmware)
  - `fixup4.dat` / `fixup4cd.dat`
  - `bcm2711-rpi-4-b.dtb` / `bcm2712-rpi-5-b.dtb` (Standard-DTBs)
  - `config.txt`: `arm_64bit=1`, `kernel=kernel8.img`, `enable_uart=1`, `disable_overscan=1`
  - `kernel8.img` — anyOS Kernel (flat binary, kein ELF)
  - `cmdline.txt` (optional)

- [ ] **System-Partition (exFAT)** — anyOS Root-Filesystem:
  - `/System/bin/` — CLI-Programme (ARM64)
  - `/Applications/` — GUI-Apps (ARM64)
  - `/System/lib/` — Shared Libraries (ARM64)
  - `/System/fonts/` — Schriftarten
  - `/System/icons/` — Icon-Pakete (ico.pak)

- [ ] **mkimage erweitern** (`tools/mkimage/`)
  - `--rpi4` / `--rpi5` Flag: GPT mit 2 Partitionen (FAT32 Boot + exFAT Root)
  - RPi-Firmware-Dateien in Boot-Partition kopieren
  - Kernel als flat binary (objcopy -O binary) statt ELF
  - `config.txt` generieren

- [ ] **Build-Target** (`cmake/Targets.cmake`)
  - `rpi4-image` / `rpi5-image` Target
  - `dd if=anyos-rpi4.img of=/dev/sdX bs=4M` zum Schreiben auf SD-Karte
  - Alternativ: komprimiertes Image (`.img.gz`) zum Download

### Phase 9: SMP auf RPi

- [ ] **PSCI via SMC** statt HVC
  - RPi4 EL3 Firmware (ARM Trusted Firmware) erwartet `smc #0` statt `hvc #0`
  - `smp.rs` + `ap_startup.S`: Kondition auf Board-Typ
  - `CPU_ON` function ID: `0xC4000003` (gleich, nur SMC statt HVC)

- [ ] **Per-CPU Kernel-Stacks**
  - HAL-Stubs `set_kernel_stack_for_cpu` / `get_kernel_stack_for_cpu` ausfuellen
  - `TPIDR_EL1` fuer per-CPU-Daten (bereits im Kontextwechsel gesichert)

- [ ] **cpu_count aus DTB**
  - `/cpus/cpu@N` Nodes zaehlen statt hardcoded `1`

### Priorisierung / Reihenfolge

```
Phase 1 (DTB + Board-Detection)     → Grundlage fuer alles
  ↓
Phase 2 (GICv2)                      → Interrupts auf RPi4
  ↓
Phase 3 (EMMC2 SD-Treiber)          → Storage / Dateisystem
  ↓
Phase 4 (Userspace ARM64)           → Programme ausfuehren
  ↓
Phase 8 (SD-Image)                   → Erstes bootbares Image (Text-only via UART)
  ↓
Phase 5 (Framebuffer)               → Grafische Ausgabe
  ↓
Phase 6 (USB HID)                    → Tastatur/Maus
  ↓
Phase 7 (Netzwerk)                   → Ethernet
  ↓
Phase 9 (SMP)                        → Alle 4 Kerne nutzen
```

**Meilenstein 1** (Phase 1-4 + 8): Kernel bootet auf RPi4, Shell ueber UART, Programme von SD-Karte
**Meilenstein 2** (+ Phase 5-6): GUI-Desktop auf HDMI mit USB-Tastatur/Maus
**Meilenstein 3** (+ Phase 7, 9): Voller Desktop mit Netzwerk und 4 Kernen

### QEMU raspi4b Emulation

QEMU 9.0+ unterstuetzt `-M raspi4b` (Cortex-A72 x4, BCM2711). Emuliert: UART, GICv2, Mailbox, SDHCI, GPIO, Timer, Framebuffer. NICHT emuliert: PCIe, GENET Ethernet, xHCI USB, WiFi.

```bash
qemu-system-aarch64 -M raspi4b -m 2G -kernel kernel8.img \
  -dtb bcm2711-rpi-4-b.dtb -sd anyos-rpi4.img -serial stdio -display none
```

Kernel muss flat binary sein: `objcopy -O binary kernel.elf kernel8.img` (Ladeadresse `0x80000`).

Phase 1-5 in QEMU entwickelbar, Phase 6+ (USB/Ethernet) braucht echte Hardware.

---

## Terminal — Implementierte Features

### ANSI Escape Sequences ✅
- [x] **256-Farben + True-Color** — `\x1B[38;5;Nm`, `\x1B[48;5;Nm`, `\x1B[38;2;R;G;Bm`, `\x1B[48;2;R;G;Bm`
- [x] **Hintergrundfarben (BG)** — `\x1B[40-47m`, `\x1B[100-107m` + Extended
- [x] **Underline** — SGR 4 / 24
- [x] **Italic** — SGR 3 / 23 (attr tracked, Rendering font-abhängig)
- [x] **Strikethrough** — SGR 9 / 29
- [x] **Blink** — SGR 5 / 25 (attr tracked, kein visuelles Blinken)
- [x] **Reverse Video** — SGR 7 / 27
- [x] **Bold/Bright** — SGR 1 / 22 (schaltet auf Bright-Farbpalette)
- [x] **Alternate Screen Buffer** — `\x1B[?1049h/l` (für vi, nano, htop)
- [x] **Scroll-Regionen** — `\x1B[r` (DECSTBM)
- [x] **Tab Stops** — 8-Spalten-Tabs (kein HTS/TBC)
- [x] **Insert/Delete Line/Char** — `\x1B[L`, `\x1B[M`, `\x1B[@`, `\x1B[P`, `\x1B[X`
- [x] **Cursor Show/Hide** — `\x1B[?25h/l`
- [x] **Window Title (OSC)** — `\x1B]0;title\x07`, `\x1B]2;title\x07`

### Terminal-Emulation ✅
- [x] **Mouse Tracking** — Modi 1000, 1002, 1003, 1006 (SGR) + Forwarding an Kindprozesse
- [x] **Bracketed Paste Mode** — `\x1B[?2004h/l` + Forwarding an Kindprozesse
- [x] **Bell** — `\x07` (BEL, erkannt, still ignoriert)
- [x] **DEC Private Modes** — ?25, ?47, ?1047, ?1049, ?1000, ?1002, ?1003, ?1006, ?2004
- [x] **Hyperlinks** — OSC 8 (Cell.link_id, Hyperlink-Tabelle, Ctrl+Click zum Öffnen)

### Text & Rendering ✅
- [x] **Unicode Wide Characters (CJK)** — `char_width()`, 2-Spalten-Rendering, Continuation-Cells
- [x] **Combining Characters** — diakritische Zeichen via `Cell.combining`, `is_combining()`
- [x] **Regex-Suche im Scrollback** — `simple_regex_match()` Engine, Ctrl+R Toggle

### Shell-Features ✅
- [x] **Job Control** — `bg`, `fg`, `jobs`, Ctrl+Z (SIGTSTP/SIGCONT, vollständiges Stop/Resume)
- [x] **Logical Operators** — `&&`, `||`, `;` via `split_logical_operators()`
- [x] **Input-Redirection** — `< file` via `parse_input_redirect()`
- [x] **Output-Redirection** — `>`, `>>`, `2>`, `2>>` via `parse_redirects()`
- [x] **Variablen-Expansion** — `$VAR`, `${VAR}` via `expand_vars()`
- [x] **Command Substitution** — `$(cmd)` und Backticks via `capture_command_output()`
- [x] **Here-Documents** — `<< EOF` (interaktive Eingabe mit `> ` Prompt)
- [x] **Pipes** — `cmd1 | cmd2 | cmd3` via `execute_pipeline()`

### Terminal TODO — Noch nicht implementiert
- [ ] **Sixel Graphics** — Inline-Grafiken im Terminal
- [ ] **Drag & Drop** — Dateien ins Terminal ziehen
- [ ] **Fallback-Fonts** — fehlende Glyphen aus alternativen Fonts laden
- [ ] **Ligature-Support** — z.B. `->`, `=>`, `!=` als Ligaturen
- [ ] **Notifications** bei Befehlsende (Desktop-Notification)
- [x] **Ctrl+Z Suspend** — Kernel SIGTSTP/SIGCONT + Terminal Ctrl+Z Handler
- [ ] **Tab Stop Customization** — HTS (`\x1BH`), TBC (`\x1B[3g`)

---

## CoreVM Manager — TODO (Code Review)

Priorisierte Verbesserungen fuer bessere Debuggbarkeit und Wartbarkeit. Details siehe `memory/corevm-code-review.md`.

### Prio 1: Strukturiertes Logging
- [ ] Log-Levels einfuehren (ERROR, WARN, INFO, DEBUG, TRACE) — aktuell nur print oder nichts
- [ ] Device-spezifische Log-Tags: `[corevm:ahci]`, `[corevm:ps2]`, `[corevm:pic]` statt nur `[corevm]`
- [ ] Optionales I/O-Tracing (Port-I/O und MMIO bei Bedarf loggen)
- [ ] Exit-Reason-Logging in vmd `run_vm_step()` mit Kontext (Port, Adresse, Daten)

### Prio 2: Konsistentes Error Handling in FFI
- [ ] `set_last_error()` bei JEDEM `-1` Return in `libs/libcorevm/src/ffi.rs`
- [ ] Betroffene Funktionen: `corevm_reset`, `corevm_destroy_vcpu`, `corevm_get_vcpu_regs`, `corevm_set_vcpu_regs`, `corevm_inject_interrupt`, `corevm_inject_exception`

### Prio 3: CPU-Register-Dump bei VM-Crash
- [ ] Bei Shutdown/Error in vmd: RIP, RSP, CS, CR0, CR3, RFLAGS ausgeben
- [ ] Fehler-Kontext in Status-Messages (z.B. `error 0 triple fault at RIP=0x7C00`)

### Prio 4: Gemeinsame Konstanten
- [ ] `SHM_HEADER`, `SHM_SIZE`, State-Konstanten in gemeinsame Datei auslagern (aktuell dupliziert in vmmanager + vmd)

### Prio 5: vmmanager modularisieren
- [ ] `apps/vmmanager/src/main.rs` (~2800 Zeilen) aufteilen: `config.rs`, `ipc.rs`, `ui/sidebar.rs`, `ui/settings.rs`, `ui/canvas.rs`

### Prio 6: Config-Parsing deduplizieren
- [ ] VM-Config wird unabhaengig in vmmanager (`load_vm_config`) und vmd (`read_vm_config`) geparst — gemeinsame Library oder synchronisierte Parser

### Prio 7: IPC-Protokoll verbessern
- [ ] Command-Ack (vmd bestaetigt Empfang)
- [ ] Sequenznummern fuer Commands/Responses
- [ ] CMD_BUF erweitern (aktuell nur 512 Bytes, 1 Command gleichzeitig)

### Prio 8: Device-Pointer-Pattern
- [ ] 10 raw `*mut` Pointer in `Vm` struct (`libs/libcorevm/src/vm.rs`) refactorn — mindestens in separates Struct kapseln

### Prio 9: Pause/Resume
- [ ] `VmState::Paused` existiert als Enum aber kein `cmd_pause()`/`cmd_resume()` in vmd

### Prio 10: AHCI Memory Leak
- [ ] `core::mem::forget(ahci)` in `ffi.rs corevm_setup_ahci()` — kein Cleanup bei `corevm_destroy()`. AHCI als `Option<Box<Ahci>>` in Vm speichern.

---

---

## AnyStream — Window Streaming Tool (geplant, noch nicht implementiert)

### Ziel

Ein Tool das auf ein laufendes anyOS zugreift, eine Anwendung startet, und das Fenster dieser Anwendung an den Host streamt — inkl. Maus/Tastatur-Forwarding. Anwendungsfaelle:
- **Tests**: Automatisierte UI-Tests die eine App starten und den Fensterinhalt pruefen
- **Remote-App**: Eine anyOS-Anwendung von einem anderen Rechner aus bedienen

### Ausgangslage (was bereits existiert)

| Vorhanden | Details |
|-----------|---------|
| `vncd` | Vollstaendiger VNC-Server, Full-Screen, Dirty-Tile + Motion-Detection, Zlib |
| `SYS_CAPTURE_SCREEN` | Kernel-Syscall fuer Fullscreen-Capture (ARGB8888) |
| `CMD_INJECT_KEY/POINTER` | Compositor IPC fuer Input-Injection |
| Window-SHM-System | Jede App schreibt in eigenen SHM-Pixel-Buffer |

### Kernidee: Offscreen-Framebuffer im Compositor

Stream-Fenster bekommen einen eigenen Offscreen-RAM-Buffer im Compositor und sind auf dem Desktop **nicht sichtbar**. Der Compositor unterscheidet `WindowType::Normal` und `WindowType::Stream`. Der anystream-Daemon mappt den Offscreen-Buffer direkt via SHM — kein `SYS_CAPTURE_SCREEN`, kein Cropping, kein Overlap-Problem.

```
Normal-Fenster:                      Stream-Fenster:

App → SHM-Surface → CMD_PRESENT      App → SHM-Surface → CMD_PRESENT
           ↓                                     ↓
  Compositor → GPU-Framebuffer        Compositor → Offscreen-RAM-Buffer
  (sichtbar am Desktop)               (UNSICHTBAR am Desktop)
                                                 ↓
                                      anystream-Daemon mappt SHM
                                      → liest Pixel direkt (zero-copy)
                                      → zlib-komprimieren → TCP streamen
```

**Vorteile gegenueber Full-Screen-Capture:**
- Kein `SYS_CAPTURE_SCREEN` noetig — Compositor hat die Pixel schon im App-SHM
- Kein Cropping, kein Focus-Stealing, kein Overlap-Problem
- Mehrere Stream-Fenster gleichzeitig moeglich
- Effizienter: Compositor kopiert direkt in Offscreen-Buffer bei `CMD_PRESENT`

### Vollstaendiger Flow

```
1. Daemon sendet CMD_REGISTER_STREAM_PID [pid=0, notify_pipe_fd]
   (pid=0 = "naechste App die ich starte")

2. Daemon: process::spawn("/Applications/paint.app") → PID

3. App: CMD_CREATE_WINDOW
   → Compositor erkennt PID als Stream-registriert
   → Allokiert Offscreen-SHM (w*h*4 Bytes, normales RAM)
   → Fenster NICHT in Desktop-Render-Liste aufnehmen (unsichtbar)
   → Antwortet mit CMD_STREAM_WINDOW_READY [win_id, shm_id, w, h]

4. Daemon mappt shm_id → hat direkten Zeiger auf Pixel-Buffer

5. App zeichnet normal, ruft CMD_PRESENT auf
   → Compositor compositet in Offscreen-SHM
   → Schreibt DirtyNotification { win_id, x, y, w, h, seq } in notify_pipe

6. Daemon liest DirtyNotification aus pipe (wakeup ohne polling)
   → Liest dirty rect aus SHM (zero-copy)
   → zlib komprimieren → FRAME-Packet senden

7. Host-Client empfaengt → anzeigen
```

### Phase 1 — Compositor-Erweiterung: Stream-Window-Support

Aenderungen in `libs/libcompositor/src/` und `libs/libcompositor_client/src/lib.rs`:

```rust
// Neue Window-Typen
enum WindowType { Normal, Stream { offscreen_shm: u32, notify_pipe: Fd } }

// Neue Commands
CMD_REGISTER_STREAM_PID  = 0x1030  // [u32 pid, u32 notify_pipe_fd]
                                    // pid=0 = naechste gespawnte App
CMD_STREAM_WINDOW_READY  = 0x1031  // Response: [u32 win_id, u32 shm_id, u32 w, u32 h]
CMD_FOCUS_WINDOW         = 0x1032  // [u32 win_id] (fuer normale Fenster, falls benoetigt)
CMD_GET_WINDOW_LIST      = 0x1033  // → [u32 count, [id, pid, x, y, w, h, title]...]
                                    // (nur Normal-Fenster, Stream-Fenster nicht aufgelistet)
```

Dirty-Notification-Struct (wird in notify_pipe geschrieben):
```rust
struct DirtyNotification {
    win_id: u32,
    x: u32, y: u32,    // geaenderter Bereich
    w: u32, h: u32,
    seq: u32,          // Sequenznummer fuer Reihenfolge-Pruefung
}  // 24 Bytes, atomar in pipe schreibbar
```

**Aufwand**: ~250 Zeilen in `libs/libcompositor/`, ~50 Zeilen Client-API

### Phase 2 — anystream-Daemon auf anyOS

Neues Programm `bin/anystream/` — TCP-Server (Port 7722).

#### Protokoll (binaer, 1 Byte Type + Payload)

**Client → Server**:
```
0x01  LAUNCH       [u16 path_len][path][u16 args_len][args]
0x02  STOP         [u32 session_id]
0x03  INPUT_KEY    [u32 scancode][u32 char_val][u8 is_down][u8 modifiers]
0x04  INPUT_PTR    [u32 x][u32 y][u8 buttons]   (x/y relativ zum Stream-Fenster)
0x05  PING
0x06  SCREENSHOT                                 (einmaliger Frame, kein Stream)
0x07  LIST_WINDOWS                               (nur Normal-Fenster am Desktop)
0x08  RESIZE       [u32 session_id][u32 w][u32 h]
```

**Server → Client**:
```
0x80  SESSION_START   [u32 session_id][u32 win_id][u32 w][u32 h][u16 title_len][title]
0x81  WINDOW_RESIZED  [u32 session_id][u32 w][u32 h]
0x82  FRAME           [u32 session_id][u32 x][u32 y][u32 w][u32 h]
                      [u32 data_len][zlib ARGB8888]
0x83  WINDOW_CLOSED   [u32 session_id]
0x84  PONG
0x85  WINDOW_LIST     [u32 count][[u32 id,pid,x,y,w,h, u16 title_len, title],...]
0x86  SCREENSHOT_DATA [u32 w][u32 h][u32 data_len][zlib ARGB8888]
0xF0  ERROR           [u16 msg_len][message]
```

#### Daemon-Implementierung

```
- TCP accept-Loop, pro Verbindung ein Session-State
- CMD_REGISTER_STREAM_PID an Compositor senden (notify_pipe anlegen)
- App spawnen, auf CMD_STREAM_WINDOW_READY warten
- SESSION_START an Client senden
- Haupt-Loop: pipe-fd via poll() auf DirtyNotification warten
  → Dirty-Rect aus SHM lesen
  → Dirty-Tile-Vergleich (32x32 Tiles) gegen prev-Buffer
  → Geaenderte Region zlib-komprimieren → FRAME senden
  → prev-Buffer aktualisieren
- Input-Packets vom Client → CMD_INJECT_KEY/POINTER an Compositor
```

**Dateien**: `bin/anystream/src/main.rs`, `protocol.rs`, `stream.rs`, `input.rs`
**Dependencies**: `anyos_std`, `libcompositor_client`
**Aufwand**: ~500 Zeilen

### Phase 3a — Host-Client: anystream CLI (Linux/macOS/Windows)

Rust-Projekt `tools/anystream-client/` (laeuft nativ auf Linux/macOS/Windows):

```bash
anystream --host 192.168.1.100:7722 launch /Applications/paint.app
anystream --host ... screenshot /Applications/calc.app --output out.png
anystream --host ... list
```

Fensteranzeige mit `winit` + `softbuffer` (pure Rust, keine C-Deps):

```toml
[dependencies]
winit = "0.30"
softbuffer = "0.4"
flate2 = "1.0"   # zlib decode
clap = "4"
```

**Aufwand**: ~400 Zeilen

### Phase 3b — anyOS-native Streaming-App: anystream GUI

GUI-App `apps/anystream/` fuer anyOS selbst — damit kann man von einem anyOS-Rechner
eine Anwendung auf einem anderen anyOS starten und in einem lokalen Fenster bedienen.

```
┌─────────────────────────────────────────────────────┐
│  AnyStream                                    _ □ ✕ │
├──────────┬──────────────────────────────────────────┤
│ Verbinden│  Host: [192.168.1.100      ] Port: [7722]│
│          │  [Verbinden]                             │
│ Sessions │──────────────────────────────────────────│
│ ▶ calc   │                                          │
│   paint  │   [Gestreamtes Fenster wird hier         │
│          │    in einem Canvas gerendert]            │
│ [Starten]│                                          │
│          │   App starten: [________________] [▶]    │
└──────────┴──────────────────────────────────────────┘
```

**Layout:**
- Sidebar (DOCK_LEFT, 200px): Verbindungs-Eingabe, Session-Liste, App-Launcher
- Canvas (DOCK_FILL): Empfangene Frames werden via `canvas.set_pixels()` gezeichnet
- Statusbar (DOCK_BOTTOM): Verbindungsstatus, FPS, Latenz

**Implementierungsdetails:**
- Netzwerk-Thread: TCP-Verbindung, Frame-Empfang, zlib-Dekomprimierung → Frame-Queue
- UI-Thread: Canvas per Timer-Callback (~30fps) aus Frame-Queue befuellen
- Input-Forwarding: `canvas.on_mouse_move/click()` + `win.on_key()` → INPUT_PTR/INPUT_KEY an Daemon
- Koordinaten-Mapping: Canvas-Pixel → Fenster-Koordinaten (bei Skalierung)

**Cargo.toml**:
```toml
[dependencies]
anyos_std        = { path = "../../libs/stdlib" }
dynlink          = { path = "../../libs/dynlink" }
libanyui_client  = { path = "../../libs/libanyui_client" }
```
(zlib-Dekomprimierung ueber anyos_std oder eigene miniz-Implementierung)

**Dateien**: `apps/anystream/src/main.rs`, `net.rs`, `ui.rs`
**CMakeLists.txt**: `add_rust_app(anystream ${CMAKE_SOURCE_DIR}/apps/anystream "AnyStream" "1.0")`
**Info.conf**: `capabilities=filesystem,network,display,dll,event,thread,pipe`
**Aufwand**: ~600 Zeilen

### Phase 4 — Test-Bibliothek: anyos_testkit

Rust-Crate `tools/anyos_testkit/` fuer automatisierte UI-Tests:

```rust
use anyos_testkit::AnyStream;

#[test]
fn test_calculator() {
    let stream = AnyStream::connect("localhost:7722").unwrap();
    let session = stream.launch("/Applications/calc.app").unwrap();
    session.wait_ready(Duration::from_secs(5)).unwrap();
    session.click(150, 200).unwrap();
    session.type_text("3+4=").unwrap();
    session.wait_for_pixel(300, 50, |px| px.r > 200, Duration::from_secs(2)).unwrap();
    session.screenshot().unwrap().save("result.png").unwrap();
}
```

API: `connect()`, `launch()`, `screenshot()`, `click(x,y)`, `right_click(x,y)`, `type_text()`, `press_key()`, `wait_ready()`, `wait_for_pixel()`, `window_size()`, `close()`

**Aufwand**: ~300 Zeilen

### Umsetzungsreihenfolge

```
Phase 1: Compositor Stream-Window-Support                           (~2 Tage)
         (WindowType::Stream, Offscreen-SHM, DirtyNotification-Pipe,
          CMD_REGISTER_STREAM_PID, CMD_STREAM_WINDOW_READY)
    ↓
Phase 2a: anystream-Daemon Grundgeruest (TCP, LAUNCH, SCREENSHOT)  (~2 Tage)
    ↓
Phase 2b: Frame-Streaming (Dirty-Detection, zlib, notify_pipe)      (~1 Tag)
    ↓
Phase 3a: Host-Client CLI für Linux/macOS/Windows                   (~2 Tage)
Phase 3b: anyOS-native GUI-App apps/anystream/                      (~2 Tage)
    (3a und 3b koennen parallel entwickelt werden, teilen dasselbe Protokoll)
    ↓
Phase 4: anyos_testkit Crate                                        (~1 Tag)
```

### Hinweise zur Implementierung

- **Offscreen-SHM-Groesse**: w*h*4 Bytes (ARGB8888). Bei Resize: neuen SHM allokieren, SESSION_START erneut senden.
- **Mehrere Stream-Sessions**: Jede Session bekommt eigene notify_pipe + eigenen Offscreen-SHM. Compositor unterstuetzt N gleichzeitige Stream-Fenster.
- **QEMU Port-Forwarding**: `-netdev user,hostfwd=tcp::7722-:7722` in `scripts/run.sh` eintragen.
- **VNC-Koexistenz**: anystream laeuft auf Port 7722, vncd auf 5900 — keine Konflikte.
- **Authentifizierung**: Erstimplementierung ohne Auth. Spaeter Token-basiert (wie vncd-Passwort).
- **App-Sicht**: Die gestreamte App bemerkt nicht, dass sie im Stream-Modus laeuft — sie nutzt die normale Compositor-API unveraendert.
- **Localhost-Betrieb**: anystream funktioniert auch mit `127.0.0.1:7722` auf demselben anyOS-Rechner. anyOS hat ein Loopback-Interface (`lo`, 127.0.0.1/255.0.0.0). Anwendungsfaelle:
  - **Tests direkt auf anyOS**: `anystream-client --host 127.0.0.1:7722 launch /Applications/calc.app` ohne externen Host
  - **Fenster-im-Fenster**: `apps/anystream` verbindet sich per Localhost und zeigt eine gestreamte App im Canvas — auf demselben Rechner
  - **Sandboxed-Vorschau** (nach Offscreen-Framebuffer-Implementierung): App laeuft unsichtbar im Hintergrund, nur im Stream-Canvas sichtbar — kein Fenster am Desktop
  - **Automatisierte UI-Tests** mit `anyos-testkit` koennen direkt auf anyOS laufen ohne Netzwerk zu einem zweiten Rechner

---

## surf-host — Standalone Linux Rendering (tools/surf-host/)

Standalone Linux-Build der Surf Rendering-Engine. Nutzt die gleiche HTML/CSS/JS-Pipeline wie anyOS Surf, kompiliert fuer den Host via `host` Feature-Flags.

### Build & Aufruf

```bash
cd tools/surf-host
./build.sh                                         # Nur bauen
./build.sh run https://www.wikipedia.de             # Fenster oeffnen
./build.sh run https://www.wikipedia.de 1280x960    # Mit Viewport-Groesse
./build.sh screenshot https://example.com           # Headless Screenshot
./build.sh screenshot https://example.com out.png full 1280x960 3000
./build.sh screenshot https://example.com 400-900 crop.png
```

Muss mit `cargo +stable` gebaut werden (nicht nightly). Die `.cargo/config.toml` im surf-host Verzeichnis ueberschreibt das anyOS-Target.

### Host Feature-Flags

Betroffene Crates: `anyos_std`, `libheap`, `dynlink`, `libfont`, `libfont_client`, `libanyui_client`, `libjs`, `libwebview`. Jedes hat ein `host` Feature das `#![no_std]` → `std` umschaltet und OS-spezifische Teile durch Linux-Aequivalente ersetzt.

**Wichtig bei Aenderungen an diesen Crates**: Sicherstellen dass sowohl `ninja` (anyOS-Build) als auch `cargo +stable build --release` in `tools/surf-host/` funktionieren.

### Architektur-Entscheidungen

- `libanyui_client/src/lib.rs` nutzt `include!("anyos_rest.rs")` um den anyOS-Code einzubinden, da Rust kein Block-Level `#[cfg]` fuer 1200 Zeilen erlaubt
- `libfont_client` im Host-Modus nutzt `extern "C"` Deklarationen (nicht `dep:libfont`), weil libfont ein eigenes `[workspace]` hat und als Cargo-Dependency den anyOS-Workspace-Resolver bricht
- `libfont` hat `crate-type = ["staticlib", "lib"]` — beide noetig (staticlib fuer anyOS-Linking, lib fuer surf-host)
- `tools/surf-host` ist in `Cargo.toml` (Root) unter `exclude` eingetragen um Workspace-Konflikte zu vermeiden

### TODO — Noch nicht implementiert

#### Prio 1: Rendering-Paritaet mit anyOS Surf
- [ ] **Formular-Controls rendern** — TextField, Checkbox, RadioButton, TextArea sind aktuell Stubs (unsichtbar). Im Host-Modus eigene Render-Funktion in Canvas implementieren (Rahmen, Platzhalter-Text, Checkbox-Quadrat)
- [ ] **CSS background-color auf Body/HTML** — Hintergrundfarbe des Dokuments wird nicht auf den gesamten Viewport angewendet. Viewport-Hintergrund muss aus der Cascade kommen statt hardcoded weiss
- [ ] **Subpixel Font-Rendering** — Aktuell nur Greyscale-AA. Optional LCD-Subpixel-Rendering aktivieren (libfont unterstuetzt es, nur `read_font_smoothing()` gibt im Host-Modus immer 1 zurueck)
- [ ] **CSS background-image: url(...)** — Background-Images werden nicht geladen. DOM-Walk muesste auch computed styles nach `background-image` URLs scannen

#### Prio 2: Ressourcen-Loading
- [ ] **CSS @import Kaskade** — Aktuell nur eine Ebene tief. Verschachtelte @imports (import in import) werden nicht verfolgt
- [ ] **data: URIs fuer Bilder** — `<img src="data:image/png;base64,...">` werden uebersprungen. Base64-Dekodierung + Bild-Dekodierung einbauen
- [ ] **`<picture>` / `srcset`** — Responsive Bilder werden ignoriert. Erstes `<source>` oder `srcset` Bild laden
- [ ] **CSS-Bilder via `<style>` Blocks** — Inline-CSS `background-image` URLs werden nicht gefetcht

#### Prio 3: Interaktiver Modus
- [ ] **Maus-Klick auf Links** — Klick-Events an libwebview weiterleiten, neue URL laden
- [ ] **URL-Eingabe** — Tastatureingabe fuer URL-Aenderung (aktuell nur Kommandozeile)
- [ ] **Formular-Eingabe** — TextField/TextArea Eingabe im Fenster-Modus
- [ ] **Fenster-Resize** — Viewport-Groesse anpassen wenn das Fenster vergroessert wird, relayout triggern
- [ ] **Zurueck/Vorwaerts** — Navigation-History

#### Prio 4: Erweiterte Features
- [ ] **HTTP/2** — ureq unterstuetzt nur HTTP/1.1. Fuer moderne Seiten (multiplexing, server push) waere reqwest oder hyper noetig
- [ ] **Cookie-Persistenz** — Cookies zwischen Seitenladungen speichern
- [ ] **JavaScript-Netzwerk-APIs** — fetch(), XMLHttpRequest Responses an libwebview zurueckliefern (aktuell werden JS-HTTP-Requests ignoriert)
- [ ] **WebSocket-Support** — JS WebSocket-Verbindungen ueber Host-Netzwerk
- [ ] **Automatisierte Tests** — CI-Integration: URL laden, Screenshot vergleichen mit Referenz-Bild (Pixel-Diff)

---

## User communicates in German
