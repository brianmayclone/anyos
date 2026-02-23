<div align="center">

<img src="assets/images/logo_bw.png" alt="anyOS" width="280">

<br><br>

**A 64-bit operating system built from scratch in Rust and Assembly**

macOS-inspired desktop with window compositor, network stack, USB support,<br>
audio playback, TrueType fonts, and an on-disk C compiler — all running bare-metal on x86_64.

<br>

![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white)
![NASM](https://img.shields.io/badge/NASM-Assembly-0066B8?style=flat-square)
![x86_64](https://img.shields.io/badge/Arch-x86__64-4B7BEC?style=flat-square)
![License: MIT](https://img.shields.io/badge/License-MIT-2ecc71?style=flat-square)
![Programs](https://img.shields.io/badge/Programs-125+-e67e22?style=flat-square)
![Syscalls](https://img.shields.io/badge/Syscalls-140-9b59b6?style=flat-square)

<br>

<img src="assets/screenshots/shot3.png" alt="anyOS Desktop — Terminal and Activity Monitor" width="760">

<sub>Terminal and Activity Monitor running side by side on the anyOS desktop</sub>

<br><br>

[Features](#features) · [Screenshots](#screenshots) · [Quick Start](#quick-start) · [Documentation](#documentation) · [Contributing](#contributing)

</div>

<br>

> **A learning project** created purely for fun and curiosity. It demonstrates how operating systems work under the hood — from bootloader to desktop environment — all without relying on any existing OS or standard library. **Contributions are welcome!**

---

## Screenshots

<div align="center">
<table>
<tr>
<td align="center" width="50%">
<img src="assets/screenshots/shot1.png" width="100%"><br>
<sub><b>Login Screen</b> — User authentication with wave wallpaper</sub>
</td>
<td align="center" width="50%">
<img src="assets/screenshots/shot2.png" width="100%"><br>
<sub><b>Desktop</b> — Menu bar, dock, and dynamic wallpaper</sub>
</td>
</tr>
<tr>
<td align="center">
<img src="assets/screenshots/shot4.png" width="100%"><br>
<sub><b>CPU Monitoring</b> — Real-time per-core graphs across 4 SMP cores</sub>
</td>
<td align="center">
<img src="assets/screenshots/shot5.png" width="100%"><br>
<sub><b>Finder</b> — File browser with sidebar and icon view</sub>
</td>
</tr>
<tr>
<td align="center">
<img src="assets/screenshots/shot6.png" width="100%"><br>
<sub><b>DOOM</b> — Running natively in a window</sub>
</td>
<td align="center">
<img src="assets/screenshots/shot7.png" width="100%"><br>
<sub><b>Quake & DOOM</b> — Classic games running side by side</sub>
</td>
</tr>
</table>
</div>

---

## Features

### Kernel

- **Mach-style microkernel architecture** with message-based IPC
- **64-bit x86_64** long mode with 4-level paging (4 KiB pages)
- **Preemptive multitasking** with Mach-style multi-level priority scheduler (128 levels, bitmap-indexed O(1) thread selection, per-CPU run queues)
- **SMP support** — multi-core (up to 16 CPUs) via LAPIC/IOAPIC with per-CPU idle threads and work stealing
- **Per-process address spaces** with isolated PML4 page directories
- **Ring 3 user mode** with dual syscall interface: `SYSCALL/SYSRET` (64-bit) and `INT 0x80` (32-bit compat)
- **140 system calls** across 22 categories (process, file I/O, networking, IPC, display, audio, USB, permissions, signals, ...)
- **Physical + virtual memory manager** with kernel heap allocator
- **exFAT filesystem** with long filename support, symlinks, mount points, chmod/chown
- **Storage drivers**: ATA PIO, **AHCI** (SATA DMA), **NVMe** (PCIe), ATAPI (CD-ROM), LSI SCSI
- **ELF loader** for user programs (ELF64 native + ELF32 compat)
- **Loadable kernel drivers** (KDRV format) with PCI device matching and hot-loading from `.ddv` bundles
- **FPU/SSE support** with lazy save/restore (CR0.TS flag) per context switch
- **TSC-calibrated timekeeping** via PIT channel 2 polled (no IRQ dependency)
- **POSIX compatibility**: `fork`, `exec`, `pipe`, `dup2`, `signals` (SIGUSR1, SIGCHLD, SIG_IGN)
- **User identity system** — UID/GID, user accounts, groups, authentication
- **Runtime app permissions** — per-user capability grants with consent dialog on first launch, reviewable in Settings

### Graphics & UI

- **VESA VBE** framebuffer (1024x768x32, runtime resolution switching)
- **Double-buffered compositor** with damage-based partial updates and blur effects
- **GPU drivers**: Bochs VGA (page flipping), VMware SVGA II (2D acceleration, hardware cursor), VirtualBox VGA, VirtIO GPU
- **macOS-inspired dark theme** with rounded windows, shadows, and alpha blending
- **42 UI controls** via the anyui framework + uisys shared library (buttons, text fields, code editor, tree view, data grid, toolbars, canvas, expander, flow/stack panels, etc.)
- **6 shared libraries** — uisys, libimage, librender, libcompositor (DLIB format) + libanyui, libfont (.so format with ELF dynamic linking)
- **TrueType font rendering** with gamma-corrected subpixel LCD anti-aliasing and size-adaptive smoothing (SF Pro family)

### Networking

- **Intel E1000** NIC driver (MMIO, DMA)
- **Protocol stack**: Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS
- **TLS support** via BearSSL
- Userspace utilities: `ping`, `ifconfig`, `arp`, `dhcp`, `dns`, `wget`, `ftp`, `curl`

### USB

- **UHCI** (USB 1.1) and **EHCI** (USB 2.0) host controller drivers
- HID keyboard and mouse support
- Mass storage (bulk-only transport)

### Audio

- **AC'97** codec driver and **Intel HDA** (High Definition Audio) driver
- WAV/PCM playback via `play` command

### Hypervisor Integration

- **VirtualBox**: VMMDev guest integration (absolute mouse, host events, capability negotiation)
- **VMware**: vmmouse absolute mouse input, SVGA II 2D acceleration
- **QEMU/KVM**: Bochs VGA, E1000, AC'97/HDA, AHCI, NVMe, VirtIO

### C Toolchain & Shell

- **TCC** (Tiny C Compiler) 0.9.27 running natively on the OS
- **NASM** 2.15+ assembler running natively
- **dash** (Debian Almquist Shell) — POSIX-compliant shell
- **Minimal POSIX libc** (35 headers, stdio, stdlib, string, math, socket, etc.)
- Write, compile, and run C programs directly on anyOS: `cc hello.c -o hello && hello`

### Build System Tools

Four native C99 tools replace all Python build scripts. They compile at the start of each build and are also available as source on the disk for self-hosting:

| Tool | Purpose | Key Features |
|------|---------|--------------|
| **anyelf** | ELF conversion | `bin` (flat binary), `dlib` (DLIB v3 shared library), `kdrv` (kernel driver) |
| **mkimage** | Disk image creation | BIOS (MBR + exFAT), UEFI (GPT + ESP + exFAT), ISO (El Torito + ISO 9660); **incremental updates** (use `--reset` for full rebuild) |
| **anyld** | ELF64 linker | Links `.o` + `.a` into shared objects (ET_DYN with .dynsym/.hash/.dynamic) |
| **mkappbundle** | App bundle creator | Validates Info.conf, capabilities, icon (ICO), executable; assembles `.app` directory structure |

All tools support `ONE_SOURCE` single-file compilation for TCC compatibility, enabling self-hosted builds directly on anyOS.

### Boot Methods

- **BIOS/MBR** — traditional PC boot (256 MiB disk, exFAT)
- **UEFI** — modern firmware boot (64 MiB GPT disk, exFAT, Rust UEFI bootloader)
- **ISO 9660** — CD-ROM/USB boot (El Torito, Rock Ridge extensions)

### User Programs

125+ command-line and GUI applications:

**GUI Applications (13):** anyOS Code (IDE), Calculator, Clock, Diagnostics, Font Viewer, Image Viewer, Minesweeper, Notepad, Paint, Screenshot, Surf (web browser prototype), Video Player, anyui Demo

**System Applications (15):** Init, Login, Compositor, Terminal, Finder, Settings, Activity Monitor, Permission Dialog, Shell (dash), Audio Monitor, Network Monitor, Input Monitor, Event Viewer, Disk Utility, amid (statistics daemon)

**Games (2):** DOOM (doomgeneric port), Quake (WinQuake software renderer port)

**CLI Utilities (95):**

| Category | Programs |
|----------|----------|
| File Management | `ls` `cat` `cp` `mv` `rm` `mkdir` `touch` `ln` `readlink` `find` `stat` `df` `mount` `umount` `fdisk` `zip` `unzip` |
| Text Processing | `echo` `grep` `sed` `awk` `wc` `head` `tail` `sort` `uniq` `rev` `strings` `base64` `xargs` |
| System Info | `sysinfo` `dmesg` `devlist` `ps` `top` `htop` `free` `uptime` `uname` `hostname` `whoami` `which` `date` `cal` |
| Networking | `ping` `dhcp` `dns` `ifconfig` `arp` `wget` `ftp` `curl` `netstat` `echoserver` |
| User Mgmt | `chmod` `chown` `su` `listuser` `listgroups` `adduser` `deluser` `addgroup` `delgroup` `passwd` |
| Shell & Process | `env` `set` `export` `pwd` `clear` `sleep` `seq` `yes` `true` `false` `nice` `kill` |
| Shell Builtins | `alias` `unalias` `eval` (via dash) |
| System Admin | `svc` `logd` `crond` `crontab` `ami` |
| Binary/Hex | `hexdump` `xxd` |
| Multimedia | `play` `pipes` |
| Dev Tools | `cc` (TCC) `nasm` `make` `git` `open` `vi` `nano` |

---

## Quick Start

```bash
# Clone the repository
git clone https://github.com/nicosommelier/anyos.git
cd anyos

# Set up the toolchain (installs cross-compiler)
./scripts/setup_toolchain.sh

# Build everything
mkdir -p build && cd build
cmake .. -G Ninja
ninja

# Run in QEMU
ninja run
```

### Prerequisites

<details>
<summary><b>macOS</b> (aarch64 or x86_64)</summary>

```bash
# Homebrew packages
brew install nasm qemu cmake ninja

# Rust nightly toolchain
rustup install nightly

# Cross-compiler for libc (run once)
./scripts/setup_toolchain.sh
```

</details>

<details>
<summary><b>Linux</b> (x86_64)</summary>

```bash
# Ubuntu/Debian
sudo apt install nasm qemu-system-x86 cmake ninja-build

# Rust nightly toolchain
rustup install nightly

# Cross-compiler for libc (run once)
./scripts/setup_toolchain.sh
```

</details>

<details>
<summary><b>Windows</b> (x86_64)</summary>

Requires MSYS2 or WSL for the cross-compiler. PowerShell build scripts are provided.

```powershell
# Install prerequisites via winget or manual download:
# - Rust nightly: https://rustup.rs
# - NASM: https://www.nasm.us
# - QEMU: https://www.qemu.org
# - CMake + Ninja: https://cmake.org
# - i686-elf-gcc: run scripts/build_cross_compiler.ps1

# Set up toolchain
.\scripts\setup_toolchain.ps1
```

</details>

### Build Targets

<details>
<summary><b>All build and run targets</b></summary>

#### Disk Images

| Target | Description |
|--------|-------------|
| `ninja` | Build the complete OS (bootloader + kernel + programs + BIOS disk image) |
| `ninja uefi-image` | Build UEFI GPT disk image (64 MiB, exFAT) |
| `ninja iso` | Build ISO 9660 CD-ROM image (El Torito bootable) |

#### BIOS Boot

| Target | Description |
|--------|-------------|
| `ninja run` | Launch with Bochs VGA (software rendering) |
| `ninja run-vmware` | Launch with VMware SVGA II (2D acceleration, hardware cursor, absolute mouse) |
| `ninja run-ahci` | Launch with AHCI (SATA DMA) + Bochs VGA |
| `ninja run-ahci-vmware` | Launch with AHCI + VMware SVGA II |
| `ninja run-audio` | Launch with HDA audio device |
| `ninja run-usb` | Launch with USB host controller + keyboard/mouse |
| `ninja run-usb-ehci` | Launch with EHCI USB 2.0 keyboard + mouse |
| `ninja debug` | Launch with GDB server on localhost:1234 |
| `ninja run-vmware-debug` | VMware SVGA + GDB server |

#### UEFI Boot

| Target | Description |
|--------|-------------|
| `ninja run-uefi` | OVMF UEFI firmware + VMware SVGA II |
| `ninja run-uefi-std` | OVMF UEFI firmware + Bochs VGA |

#### ISO Boot

| Target | Description |
|--------|-------------|
| `ninja run-cdrom` | Boot from ISO with VMware SVGA II |
| `ninja run-cdrom-std` | Boot from ISO with Bochs VGA |
| `ninja run-cdrom-with-disk` | Boot from ISO with HDD attached |

</details>

<details>
<summary><b>QEMU configuration</b></summary>

The default `run` target uses:
```
qemu-system-x86_64 -drive format=raw,file=anyos.img -m 1024M -smp cpus=4 \
  -serial stdio -vga std -netdev user,id=net0 -device e1000,netdev=net0 \
  -no-reboot -no-shutdown
```

Key flags:
- `-vga std` — Bochs VGA (VESA + page flipping)
- `-vga vmware` — VMware SVGA II (2D acceleration + hardware cursor)
- `-serial stdio` — Kernel serial output to terminal
- `-m 1024M` — 1 GiB RAM
- `-smp cpus=4` — 4 CPU cores

For AHCI (SATA DMA) disk I/O instead of legacy ATA PIO:
```
-drive id=hd0,if=none,format=raw,file=anyos.img \
  -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
```

Or use `./scripts/run.sh --ahci [--vmware | --std]` for quick testing.

</details>

---

## Project Structure

<details>
<summary><b>Full directory tree</b></summary>

```
anyos/
  bootloader/            Bootloader sources
    stage1/                MBR boot sector (512 bytes, NASM)
    stage2/                Protected mode setup, VESA VBE, kernel loading (NASM)
    uefi/                  UEFI bootloader (Rust, x86_64-unknown-uefi)
  kernel/                Kernel source (Rust + ASM)
    asm/                   Context switch, ISR/IRQ stubs, syscall entry, SMP trampoline
    src/
      arch/x86/            GDT, IDT, APIC, PIT, TSC, paging, CPUID
      drivers/             PCI, GPU (Bochs/VMware/VBox/VirtIO), keyboard, mouse, vmmouse,
                           E1000, ATA, AHCI, NVMe, ATAPI, LSI SCSI,
                           serial, AC'97, HDA audio, UHCI, EHCI, VMMDev, VirtIO
      fs/                  VFS, exFAT, FAT16, devfs
      graphics/            Framebuffer management
      ipc/                 Pipes, anonymous pipes, event bus, shared memory, message queues, signals
      memory/              Physical allocator, virtual memory, heap
      net/                 Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS
      sync/                Spinlock, mutex
      syscall/             140 syscall handlers
      task/                Mach-style scheduler, context switch, ELF loader, DLL loader, KDRV loader
      crypto/              MD5 hash
  libs/                  Libraries
    stdlib/                anyos_std — Rust standard library for user programs
    libc/                  POSIX C library (35 headers, i686-elf-gcc)
    uisys/                 uisys.dlib — UI component DLL (31 components, 80 exports)
    uisys_client/          Client stub crate for uisys
    libimage/              libimage.dlib — Image decoding DLL (PNG, BMP, JPEG, ICO, MJV)
    libimage_client/       Client stub crate for libimage
    libanyui/              libanyui.so — anyui UI framework (41 controls, 112 exports)
    libanyui_client/       Client crate for libanyui (dynlink-based)
    libfont/               libfont.so — TrueType font rendering (embedded system fonts in .rodata)
    libfont_client/        Client crate for libfont (dynlink-based)
    dynlink/               Minimal user-space dynamic linker (dl_open/dl_sym for .so files)
    librender/             librender.dlib — 2D graphics primitives DLL
    librender_client/      Client stub crate for librender
    libcompositor/         libcompositor.dlib — Compositor client API DLL
    libcompositor_client/  Client stub crate for libcompositor
  bin/                   CLI program sources (87 Rust programs)
  apps/                  GUI application sources (13 .app bundles)
  system/                System programs (15)
    init/                  Init system (PID 1)
    login/                 Login manager
    shell/                 POSIX shell (dash)
    audiomon/              Audio monitor daemon
    netmon/                Network monitor daemon
    inputmon/              Input event monitor
    compositor/            Window compositor + dock
    terminal/              Terminal emulator
    finder/                File browser
    settings/              System preferences
    permdialog/            Permission consent dialog
    taskmanager/           Activity Monitor
    eventviewer/           Event Viewer
    diskutil/              Disk Utility
    amid/                  Application statistics daemon
  third_party/           External dependencies
    tcc-0.9.27/            Tiny C Compiler
    nasm/                  NASM assembler
    dash-0.5.12/           POSIX shell (Debian Almquist Shell)
    doom/                  doomgeneric port
    quake/                 WinQuake port
    curl/                  curl HTTP client
    bearssl/               BearSSL TLS library
    libgit2/               Git library
    minigit/               Mini git CLI
  buildsystem/           Native C build tools (compiled at build start)
    anyelf/                ELF conversion tool (bin, dlib, kdrv modes)
    mkimage/               Disk image builder (BIOS/UEFI/ISO, exFAT/FAT16/GPT, incremental updates)
    anyld/                 ELF64 shared object linker (.so generation)
    mkappbundle/           App bundle creator (validates Info.conf, capabilities, icon, executable)
  tools/                 Legacy build utilities (Python, kept as reference)
    gen_font.py            Bitmap font generator
    encode_mjv.py          MJV video encoder
  scripts/               Build, run, debug scripts (.sh + .ps1)
  sysroot/               Disk filesystem template
  docs/                  API documentation
```

</details>

### Shared Library Architecture

anyOS uses two shared library formats with **dynamic kernel-managed addressing**. The kernel allocates virtual addresses at load time from a contiguous region (`0x04000000`–`0x07FFFFFF`), applies ELF relocations for position-independent `.so` files, and demand-pages `.data`/`.bss` sections per process:

- **DLIB v3**: Custom format with `DLIB` magic header + `#[repr(C)]` function pointer export table. Loaded by the kernel at boot into every process. `.rodata`/`.text` pages are shared read-only; `.data` pages are copied on demand per process.
- **.so (ELF64 ET_DYN)**: Standard ELF shared objects with `.dynsym`/`.hash` sections, linked by `anyld`. Base-0 `.so` files receive a dynamically allocated address and are relocated at load time (`R_X86_64_RELATIVE`). Loaded on demand via `SYS_DLL_LOAD`, symbols resolved via `dl_open`/`dl_sym`.

| Library | Format | Exports | Purpose |
|---------|--------|---------|---------|
| uisys | DLIB | 80 | UI controls (buttons, text fields, scroll views, context menus, toolbars, ...) |
| libimage | DLIB | 7 | Image decoding (PNG, BMP, JPEG, ICO) and scaling |
| librender | DLIB | 18 | 2D drawing primitives (lines, rects, circles, gradients) |
| libcompositor | DLIB | 16 | Window creation, event handling, IPC with compositor |
| libanyui | .so | 112 | anyui UI framework (41 controls, Windows Forms-style) |
| libfont | .so | 7 | TrueType font rendering with LCD subpixel AA (system fonts embedded in .rodata) |

DLIB programs link against lightweight client stub crates (e.g. `uisys_client`) that read the export table at the kernel-assigned base address. `.so` programs use `dynlink` crate (`dl_open`/`dl_sym`) for ELF symbol resolution.

---

## Documentation

- **[Architecture Overview](docs/architecture.md)** — Boot process, memory layout, scheduling, IPC, USB, user identity
- **[Syscall Reference](docs/syscalls.md)** — Complete reference for all 140 system calls
- **[Standard Library API](docs/stdlib-api.md)** — `anyos_std` crate reference for Rust user programs
- **[UI System API](docs/uisys-api.md)** — `uisys` DLL component reference (31 components, 80 exports)
- **[anyui Controls API](docs/anyui-api.md)** — anyui framework reference (41 controls, 112 exports)
- **[C Library API](docs/libc-api.md)** — POSIX libc reference (35 headers) for C programs
- **[libimage API](docs/libimage-api.md)** — Image decoding, scaling, ICO, and video (MJV)
- **[libfont API](docs/libfont-api.md)** — TrueType font rendering with subpixel LCD anti-aliasing
- **[librender API](docs/librender-api.md)** — 2D graphics primitives (fill, stroke, gradient, AA)
- **[libcompositor API](docs/libcompositor-api.md)** — Window management and compositor IPC

---

## Developing User Programs

### Rust Programs

User programs use the `anyos_std` crate and are structured as `#![no_std]` binaries:

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::println!("Hello from anyOS!");
}
```

Each program needs:
1. `Cargo.toml` depending on `anyos_std`
2. `build.rs` that sets the linker script (`-T stdlib/link.ld`)
3. Entry in root `Cargo.toml` exclude list
4. Entry in `CMakeLists.txt` via `add_rust_user_program()`

See [stdlib API docs](docs/stdlib-api.md) for the full syscall and library reference.

### GUI Programs

GUI programs use the `uisys_client` crate for macOS-style UI components:

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use uisys_client::*;

anyos_std::entry!(main);

fn main() {
    let win = ui::window::create("My App", 100, 100, 400, 300);

    let mut btn = button::UiButton::new(20, 20, 120, 32, types::ButtonStyle::Primary);
    let mut event = [0u32; 5];

    loop {
        if ui::window::get_event(win, &mut event) == 1 {
            let ev = types::UiEvent::from_raw(&event);
            if ev.event_type == 0 { break; } // window closed
            if btn.handle_event(&ev) {
                println!("Button clicked!");
            }
        }
        btn.render(win, "Click Me");
        ui::window::present(win);
        process::yield_cpu();
    }
}
```

See [uisys API docs](docs/uisys-api.md) for all 31 UI components.

### C Programs

Write C programs and compile them directly on the OS:

```bash
# In the anyOS terminal:
cc hello.c -o hello
hello
```

The on-disk TCC compiler supports standard C with the bundled libc. See [libc API docs](docs/libc-api.md) for the full header reference.

---

## On-Device Test Suite

anyOS ships with a built-in test suite at `/Library/system/tests/` that verifies core OS functionality **directly on the running system**. The tests are plain C programs compiled on-device using the bundled TCC compiler — no cross-compilation or external tooling needed.

```bash
# In the anyOS terminal:
cd /Library/system/tests
make            # compile all tests
./testsuite     # run the full suite
```

The test runner (`testsuite`) forks and executes each test as a child process, checks its exit code, and prints a summary:

```
=== Running test 1/5: fork_test ===
...
=== Results: 5/5 passed, 0 FAILED ===
```

### Test Coverage

| Test | What it verifies | Key syscalls |
|------|-----------------|--------------|
| **fork_test** | Process creation, parent-child relationships, exit code propagation | `fork` `waitpid` `getpid` `_exit` |
| **pipe_test** | Anonymous pipe IPC, data integrity across processes | `pipe` `fork` `read` `write` `close` |
| **dup_test** | File descriptor duplication, stdout redirection via `dup2` | `dup` `dup2` `pipe` `read` `write` |
| **pipe_chain** | Shell-style pipeline simulation (`echo` | `cat`), stdin redirection | `pipe` `fork` `dup2` `read` `write` |
| **signal_test** | Signal handlers (`SIGUSR1`, `SIGCHLD`), `SIG_IGN` | `signal` `kill` `fork` `waitpid` |

Each test is self-contained, exits with code 0 on success, and can also be run individually (e.g. `./fork_test`).

---

## Contributing

This is a community project and contributions are welcome! Here's how to get started:

1. **Fork** the repository
2. **Create a branch** for your feature or fix
3. **Build and test** with `ninja run`
4. **Submit a pull request** with a clear description

Areas where help is appreciated:
- Bug fixes and stability improvements
- New user programs and utilities
- UI component improvements
- Filesystem enhancements (FAT32, ext2)
- Network protocol improvements (full TCP, HTTPS)
- Documentation and tutorials
- Testing on different QEMU versions and configurations

### Code Style

- Rust: standard `rustfmt` formatting
- Assembly: NASM syntax with clear comments
- All source files include a copyright header (run `scripts/add_copyright.sh` to add)

---

## License

This project is licensed under the MIT License — see [LICENSE](LICENSE) for details.

## Contact

**Christian Moeller** — [c.moeller.ffo@gmail.com](mailto:c.moeller.ffo@gmail.com) · [brianmayclone@googlemail.com](mailto:brianmayclone@googlemail.com)

---

<div align="center">
<sub>Built with curiosity and a lot of coffee. If you're learning OS development, I hope this codebase helps you on your journey.</sub>
</div>
