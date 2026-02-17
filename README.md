![OS-Image](assets/images/logo_bw.png)


A 64-bit x86_64 operating system built from scratch in **Rust** and **NASM assembly**, featuring a macOS-inspired dark GUI with a window compositor, network stack, and on-disk C compiler.

This is a **learning project** created purely for fun and education. It demonstrates how operating systems work under the hood -- from bootloader to desktop environment -- all without relying on any existing OS or standard library.

**Contributions are welcome!** Whether you want to fix bugs, add features, improve documentation, or just explore the code -- feel free to get involved.

## Screenshots

<img src="assets/screenshots/shot1.png" alt="drawing" width="200"/>
<img src="assets/screenshots/shot2.png" alt="drawing" width="200"/>
<img src="assets/screenshots/shot3.png" alt="drawing" width="200"/>
<img src="assets/screenshots/shot4.png" alt="drawing" width="200"/>
<img src="assets/screenshots/shot5.png" alt="drawing" width="200"/>
<img src="assets/screenshots/shot7.png" alt="drawing" width="200"/>

The OS boots into a graphical desktop environment with:
- Window compositor with shadows, rounded corners, and transparency
- macOS-style menu bar and dock
- Resizable, draggable windows with traffic light buttons
- Hardware-accelerated cursor (VMware SVGA II) or software cursor (Bochs VGA)

## Features

### Kernel
- **64-bit x86_64** long mode with 4-level paging (4 KiB pages)
- **Preemptive multitasking** with round-robin scheduler
- **Per-process address spaces** (isolated page directories)
- **Ring 3 user mode** with syscall interface (`int 0x80`)
- **SMP support** (multi-core via APIC/IOAPIC)
- **Physical + virtual memory manager** with kernel heap allocator
- **FAT16 filesystem** with VFAT long filename support
- **Storage dispatch**: ATA PIO (legacy IDE) and **AHCI** (SATA DMA) backends
- **ELF loader** for user programs

### Graphics & UI
- **VESA VBE** framebuffer (1024x768x32, runtime resolution switching)
- **Double-buffered compositor** with damage-based partial updates
- **GPU drivers**: Bochs VGA (page flipping) and VMware SVGA II (2D acceleration, hardware cursor)
- **macOS-inspired dark theme** with rounded windows, shadows, and alpha blending
- **30 UI components** via the uisys shared library (buttons, text fields, sliders, tables, etc.)

### Networking
- **Intel E1000** NIC driver (MMIO, DMA)
- **Protocol stack**: Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS
- Userspace utilities: `ping`, `ifconfig`, `arp`, `dhcp`, `dns`, `wget`, `ftp`

### C Toolchain
- **TCC** (Tiny C Compiler) 0.9.27 running natively on the OS
- **Minimal POSIX libc** (stdio, stdlib, string, math, etc.)
- Write, compile, and run C programs directly on anyOS: `cc hello.c -o hello && hello`

### User Programs
20+ command-line and GUI applications including: `ls`, `cat`, `cp`, `mv`, `rm`, `mkdir`, `touch`, `ping`, `wget`, `ftp`, `dns`, `date`, `sleep`, `hostname`, `sysinfo`, `dmesg`, plus GUI apps: Terminal, Finder, Settings, Task Manager, Notepad

## Getting Started

### Prerequisites

- **macOS or Linux** host (macOS aarch64 or x86_64, Linux x86_64)
- **Rust nightly** toolchain (`rustup install nightly`)
- **NASM** assembler (`brew install nasm` or `apt install nasm`)
- **QEMU** (`brew install qemu` or `apt install qemu-system-x86`)
- **CMake** + **Ninja** (`brew install cmake ninja`)
- **i686-elf-gcc** cross-compiler (for libc): see `scripts/setup_toolchain.sh`
- **Python 3** with `Pillow` and `fonttools` (`pip install Pillow fonttools`)

### Quick Start

```bash
# 1. Clone the repository
git clone https://github.com/cmoeller/anyos.git
cd anyos

# 2. Set up the toolchain (installs cross-compiler)
./scripts/setup_toolchain.sh

# 3. Build everything
mkdir -p build && cd build
cmake .. -G Ninja
ninja

# 4. Run in QEMU
ninja run
```

### Build Targets

| Target | Description |
|--------|-------------|
| `ninja` | Build the complete OS (bootloader + kernel + programs + disk image) |
| `ninja run` | Build and launch in QEMU with Bochs VGA |
| `ninja run-vmware` | Build and launch with VMware SVGA II (hardware acceleration) |
| `ninja run-ahci` | Build and launch with AHCI (SATA DMA) disk I/O, Bochs VGA |
| `ninja run-ahci-vmware` | Build and launch with AHCI + VMware SVGA II |
| `ninja debug` | Launch with GDB server on localhost:1234 |

### QEMU Configuration

The default `run` target uses:
```
qemu-system-x86_64 -drive format=raw,file=anyos.img -m 128M -smp cpus=4 \
  -serial stdio -vga std -netdev user,id=net0 -device e1000,netdev=net0
```

Key flags:
- `-vga std` -- Bochs VGA (VESA + page flipping)
- `-vga vmware` -- VMware SVGA II (2D acceleration + hardware cursor)
- `-serial stdio` -- Kernel serial output to terminal
- `-m 128M` -- 128 MB RAM (minimum recommended)

For AHCI (SATA DMA) disk I/O instead of legacy ATA PIO:
```
-drive id=hd0,if=none,format=raw,file=anyos.img \
  -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
```

Or use `./scripts/run.sh --ahci [--vmware | --std]` for quick testing.

## Project Structure

```
anyos/
  bootloader/          Two-stage x86 bootloader (NASM)
    stage1/              MBR boot sector (512 bytes)
    stage2/              Protected mode setup, VESA VBE, kernel loading
  kernel/              Kernel source (Rust + ASM)
    asm/                 Context switch, interrupts, syscall entry
    src/
      arch/x86/          GDT, IDT, APIC, PIT, paging
      drivers/           PCI, GPU, keyboard, mouse, E1000, ATA, AHCI, serial
      fs/                FAT16, VFS, devfs
      graphics/          Compositor, surface, font rendering
      ipc/               Pipes, event bus, shared memory
      memory/            Physical allocator, virtual memory, heap
      net/               Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS
      sync/              Spinlock, mutex, semaphore
      syscall/           Syscall handlers (100+ syscalls)
      task/              Scheduler, context switch, ELF loader, DLL
      ui/                Desktop, window manager, dock, widgets
  stdlib/              User-space standard library (anyos_std)
  programs/            User applications (20+ Rust programs)
    libc/                Minimal POSIX C library
    dll/uisys/           UI system shared library (30 components)
    system/              System apps (terminal, finder, settings, ...)
  tools/               Build utilities (mkimage, elf2bin, font generators)
  scripts/             Build, run, debug shell scripts
  third_party/         TCC compiler source
  docs/                Documentation
```

## Documentation

- **[Architecture Overview](docs/architecture.md)** -- Boot process, memory layout, kernel subsystems
- **[Standard Library API](docs/stdlib-api.md)** -- `anyos_std` crate reference for user programs
- **[UI System API](docs/uisys-api.md)** -- `uisys` DLL component reference for GUI development

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

See [uisys API docs](docs/uisys-api.md) for all 30 UI components.

### C Programs

Write C programs and compile them directly on the OS:

```bash
# In the anyOS terminal:
cc hello.c -o hello
hello
```

The on-disk TCC compiler supports standard C with the bundled libc.

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

## License

This project is licensed under the MIT License -- see [LICENSE](LICENSE) for details.

## Contact

- **Christian Moeller**
- Email: c.moeller.ffo@gmail.com / brianmayclone@googlemail.com

---

*Built with curiosity and a lot of coffee. If you're learning OS development, I hope this codebase helps you on your journey!*
