# anyOS Architecture Overview

This document describes the internal architecture of anyOS, from boot to desktop.

## Table of Contents

- [Boot Process](#boot-process)
- [Memory Layout](#memory-layout)
- [Kernel Architecture](#kernel-architecture)
- [Process Model](#process-model)
- [Graphics & Compositor](#graphics--compositor)
- [Filesystem](#filesystem)
- [Networking](#networking)
- [USB Subsystem](#usb-subsystem)
- [IPC Architecture](#ipc-architecture)
- [User Identity System](#user-identity-system)
- [Syscall Interface](#syscall-interface)
- [Audio](#audio)
- [DLL System](#dll-system)
- [Build System Tools](#build-system-tools)

---

## Boot Process

anyOS supports three boot methods: BIOS (MBR + exFAT), UEFI (GPT + exFAT), and ISO (El Torito).

### BIOS Boot (MBR + exFAT)

Custom two-stage bootloader written in NASM assembly.

**Stage 1 (MBR):**
- 512 bytes, loaded at `0x7C00` by BIOS
- Reads Stage 2 from disk sectors 1-3
- Jumps to Stage 2

**Stage 2:**
1. **A20 Line** -- Enables access to memory above 1 MB
2. **E820 Memory Map** -- Queries BIOS for available physical memory regions
3. **VESA VBE** -- Sets graphics mode to 1024x768x32bpp (or best available)
4. **Protected Mode** -- Sets up GDT, switches CPU to 32-bit protected mode
5. **Kernel Loading** -- Reads kernel flat binary from disk to physical address `0x100000`
6. **Paging** -- Enables 4-level paging (PML4): identity-maps first 128 MiB, maps kernel to higher-half (`0xFFFFFFFF80000000`), maps framebuffer
7. **Long Mode** -- Switches to 64-bit mode with full PML4 paging
8. **Jump to Kernel** -- Transfers control to `0xFFFFFFFF80100000` (kernel entry point)

### UEFI Boot (GPT + exFAT)

The UEFI bootloader is a PE/COFF EFI application (`bootx64.efi`).

1. **UEFI firmware** loads `\EFI\BOOT\bootx64.efi` from the EFI System Partition (FAT32)
2. **EFI bootloader** uses UEFI protocols to:
   - Query memory map via `GetMemoryMap()`
   - Set graphics mode via GOP (Graphics Output Protocol)
   - Read kernel from disk via UEFI file I/O
   - Exit boot services
3. **Paging** -- Sets up PML4 with same layout as BIOS path
4. **Jump to Kernel** -- Same entry point as BIOS boot

### ISO Boot (El Torito)

ISO 9660 boot image with El Torito no-emulation boot catalog. Used for CD/DVD boot.

### Boot Info

The bootloader passes a `BootInfo` struct at a known address containing:
- Framebuffer address, width, height, pitch
- E820/UEFI memory map entries
- Disk geometry
- Boot mode indicator (BIOS/UEFI)

---

## Memory Layout

### Physical Memory

```
0x00000000 - 0x000FFFFF    Legacy area (BIOS, VGA, bootloader)
0x00100000 - 0x00239FFF    Kernel code + data + BSS
0x0023A000 - 0x003FFFFF    Kernel stack + early allocations
0x00400000+                 Free physical frames (managed by allocator)
0xFD000000 - 0xFDFFFFFF    VESA VBE framebuffer (MMIO, not RAM)
0xFEBC0000 - 0xFEBFFFFF    E1000 NIC MMIO BAR
```

### Virtual Memory (Kernel)

```
0x00000000_00000000 - 0x00000000_07FFFFFF    Identity-mapped (first 128 MiB, for DMA/legacy)
0xFFFFFFFF_80000000 - 0xFFFFFFFF_80FFFFFF    Kernel code + data (higher-half mapping)
0xFFFFFFFF_81000000+                         Kernel heap (grows via linked-list allocator)
0xFFFFFFFF_D0000000 - 0xFFFFFFFF_D001FFFF    E1000 MMIO (128 KiB)
0xFFFFFFFF_D0020000 - 0xFFFFFFFF_D005FFFF    VMware SVGA FIFO (256 KiB)
0xFFFFFFFF_D0060000 - 0xFFFFFFFF_D0067FFF    AHCI MMIO (32 KiB)
0xFFFFFFFF_D00A0000+                         KDRV MMIO (loadable kernel drivers)
0xFFFFFFFF_D0120000 - 0xFFFFFFFF_D012FFFF    VMMDev MMIO (VirtualBox guest integration)
0xFFFFFFFF_D0140000 - 0xFFFFFFFF_D0143FFF    NVMe MMIO (16 KiB)
0xFFFFFFFF_B0000000 - 0xFFFFFFFF_BFE00000    KDRV code/data (loadable kernel driver region)
0xFD000000 - 0xFDFFFFFF                      Framebuffer (16 MiB, mapped via 4K pages)
PML4[510] recursive self-mapping              Page table access
```

### Virtual Memory (User Process)

```
0x04000000 - 0x07FFFFFF    DLL/shared library mappings:
                             0x04000000 = uisys.dlib
                             0x04100000 = libimage.dlib
                             0x04300000 = librender.dlib
                             0x04380000 = libcompositor.dlib
                             0x04400000 = libanyui.so
                             0x05000000 = libfont.so (~17 MiB, embedded fonts)
0x08000000 - 0x080XXXXX    Program text + data + BSS
0x080XXXXX - 0x0BFEFFFF    Heap (grows via sbrk)
0x0BFF0000 - 0x0BFFFFFF    User stack (64 KiB, grows downward)
0xFFFFFFFF80000000+         Kernel space (not accessible from Ring 3)
```

### Paging

- **4-level paging**: PML4 → PDPT → PD → PT (x86_64 long mode)
- **4 KiB pages** for fine-grained mapping
- **Recursive mapping**: PML4[510] points to the PML4 itself, enabling access to all paging structures
- Kernel at PML4[511], PDPT[510] (higher-half `0xFFFFFFFF80000000`)
- Each process has its own PML4; kernel entries are cloned into every process

---

## Kernel Architecture

### Module Overview

```
                 +-----------+
                 |  main.rs  |  Kernel entry, init sequence
                 +-----+-----+
                       |
    +--------+---------+---------+--------+--------+
    |        |         |         |        |        |
+---+---+ +--+--+ +---+---+ +---+--+ +---+---+ +--+--+
|arch/  | |mem/ | |drivers/| |task/ | |syscall/| |ipc/ |
|x86    | |     | |        | |      | |        | |     |
+-------+ +-----+ +--------+ +------+ +--------+ +-----+
    |        |         |         |        |        |
    |   +----+----+    |    +----+----+   |   +----+----+
    |   |physical | +--+--+ |scheduler|   |   |msg queue|
    |   |virtual  | |GPU  | |loader   |   |   |pipes    |
    |   |heap     | |E1000| |thread   |   |   |signals  |
    |   +---------+ |ATA  | |KDRV     |   |   |shm      |
    |               |AHCI | +---------+   |   +---------+
    |               |NVMe |               |
    |               |USB  |               |
    +--+  +--+      |HDA  |              |
    |GDT| |IDT|     |input|              |
    |TSS| |PIC|     +-----+              |
    |PIT| |APIC|                          |
    +---+ +----+  +-------+  +-----+     |
                  |  fs/  |  | net/|     |
                  | exFAT |  | TCP |     |
                  | VFS   |  | UDP |     |
                  +-------+  | DNS |     |
                             +-----+     |
```

### Init Sequence (main.rs)

The kernel initializes subsystems in phases:

1. **Serial** -- Debug output via COM1
2. **Boot Info** -- Parse framebuffer, memory map from bootloader
3. **GDT + TSS + IDT** -- CPU descriptor tables, interrupt handlers, TSS for Ring 0 stack
4. **FPU/SSE** -- Enable SSE/SSE2 (CR0/CR4 flags), `fninit`, CPUID verification
5. **PIT + TSC Calibration** -- PIT channel 2 polled calibration (no IRQ dependency)
6. **Physical Memory** -- Frame allocator from E820/UEFI memory map
7. **Virtual Memory** -- Page tables, kernel heap (linked-list allocator)
8. **PCI + HAL** -- Bus enumeration, driver binding (GPU, NIC, ATA/AHCI/NVMe, HDA, USB, VMMDev)
9. **KDRV** -- Load kernel driver bundles (`.ddv`) from `/System/Drivers/`, match PCI devices
10. **APIC** -- Local APIC + I/O APIC setup, LAPIC timer calibrated from TSC
11. **SMP** -- AP (Application Processor) startup via INIT-SIPI-SIPI sequence (up to 16 CPUs)
12. **SYSCALL/SYSRET** -- MSR configuration (EFER.SCE, STAR, LSTAR, SFMASK)
13. **Scheduler** -- Mach-style multi-level priority queue (128 levels, per-CPU run queues, O(1) bitmap dispatch)
14. **Keyboard/Mouse** -- PS/2 driver with IntelliMouse scroll wheel; VMware vmmouse / VMMDev absolute mouse
15. **DLL Loading** -- Map boot-time DLIBs into kernel PD (uisys, libimage, librender, libcompositor); .so libraries (libanyui, libfont) loaded on demand via SYS_DLL_LOAD
16. **Userspace** -- Load `/System/init` as first Ring 3 process, which starts the compositor

---

## Process Model

### Threads & Scheduling

- Each "process" is one or more **kernel threads** sharing the same page directory
- **Mach-style multi-level priority scheduler** with 128 priority levels (0-127, higher = more important)
- **Bitmap-indexed O(1) dispatch**: 2x u64 bitmap for instant highest-priority thread selection
- **Per-CPU run queues** with FIFO ordering within each priority level and inter-CPU work stealing
- LAPIC timer at 1000 Hz (1 ms time slices) for preemption
- Thread states: `Ready`, `Running`, `Sleeping`, `Blocked`, `Dead`
- Context switch saves/restores: RAX-RDI, R8-R15, RSP, RBP, RIP, RFLAGS, CR3, FPU state
- Lazy FPU switching via CR0.TS flag (only saves/restores 512-byte `FxState` when needed)
- Per-CPU idle threads, one scheduler lock with `try_lock()` contention handling
- POSIX process model: `fork`, `exec`, `waitpid`, `pipe`, `dup2`, `signals`

### Ring 3 User Mode

- **GDT segments**: Kernel Code (0x08), Kernel Data (0x10), User Code (0x1B), User Data (0x23), TSS (0x28)
- **Dual syscall paths**:
  - **SYSCALL/SYSRET** (64-bit Rust programs): RAX=number, RDI/RSI/RDX/R10/R8/R9=args. 10x faster than INT 0x80.
  - **INT 0x80** (32-bit C/TCC programs): EAX=number, EBX-EDI=args. Compatibility mode.
- **TSS**: RSP0 updated on every context switch for kernel stack
- **Per-process address spaces**: Each process gets its own PML4 with kernel entries cloned

### Process Lifecycle

1. `sys_spawn(path, args)` -- Kernel reads ELF/flat binary from disk
2. `create_user_page_directory()` -- Clone kernel PDEs, allocate user pages
3. `load_elf()` / `load_flat()` -- Map program segments, zero BSS
4. Thread starts at entry point in Ring 3 via `iret`
5. `sys_exit(code)` -- Thread terminates, pages freed

---

## Graphics & Compositor

### GPU Drivers

anyOS supports four GPU backends via the `GpuDriver` trait:

| Driver | PCI ID | Features |
|--------|--------|----------|
| **Bochs VGA** | 1234:1111 | VESA VBE, DISPI registers, page flipping (double buffer) |
| **VMware SVGA II** | 15AD:0405 | FIFO command queue, 2D acceleration (rect fill/copy), hardware cursor |
| **VirtualBox VGA** | 80EE:BEEF | VirtualBox guest display adapter |
| **VirtIO GPU** | 1AF4:1050 | VirtIO graphics device |

GPU auto-detection happens during PCI enumeration. The compositor uses whichever driver is available, falling back to software-only rendering if no known GPU is found.

### Compositor

- **Double-buffered**: Renders to a back buffer (`Surface`), then flushes changed regions to the framebuffer
- **Damage-based**: Only recomposes regions that changed (dirty rectangles)
- **Z-ordered layers**: Each window is a layer; layers are ordered back-to-front
- **Alpha blending**: Windows with rounded corners use per-pixel alpha
- **Hardware acceleration** (VMware SVGA II):
  - `RECT_COPY` for window dragging (moves pixels on GPU)
  - `RECT_FILL` for background fills
  - `UPDATE` to notify GPU of changed regions
  - Hardware cursor (no software cursor drawing needed)

### Window Management

- **Window = Layer + Content Surface**: Each window has chrome (title bar, buttons) and a client area
- **Hit testing**: Title bar drag, traffic light buttons, resize edges/corners
- **Resize**: Outline shown during drag, actual resize on mouse-up
- **Maximize/Minimize**: State machine (Normal/Maximized/Minimized)

---

## Filesystem

### exFAT (Primary Filesystem)

- **256 MiB disk image** with exFAT filesystem (both BIOS and UEFI modes)
- **4 KiB clusters**, contiguous allocation preferred
- **Long filenames**: File + Stream + FileName entry sets
- **Operations**: read, write, create, delete, mkdir, readdir, stat, seek, symlink, chmod, chown, rename
- **Storage dispatch**: Routes I/O to the active backend (auto-detected at boot)
  - **ATA PIO**: 28-bit LBA, sector read/write via I/O ports (legacy IDE, default)
  - **AHCI DMA**: SATA DMA transfers via MMIO + bounce buffer (ICH9 AHCI)
  - **NVMe**: PCIe NVMe controller (submission/completion queue pairs)
  - **ATAPI**: CD-ROM / ISO 9660 access
  - **LSI SCSI**: LSI MegaRAID SCSI controller

### Virtual File System (VFS)

- **File descriptors**: Global FD table, per-process open files
- **Mount points**: Runtime mount/unmount of additional filesystems
- **Paths**: `/bin/`, `/System/`, `/Libraries/`, `/include/`, `/lib/`, `/home/`
- **Device files**: `/dev/serial`, `/dev/null`, `/dev/random`
- **Symbolic links**: Symlink creation and resolution
- **Standard FDs**: 0=stdin, 1=stdout (serial), 2=stderr (serial)

---

## Networking

### Protocol Stack

```
+------------------+
|   Applications   |  wget, ftp, ping, dns, dhcp
+--------+---------+
         |
+--------+---------+
|   TCP  |   UDP   |  Transport layer
+--------+---------+
         |
+--------+---------+
|      IPv4        |  Network layer (+ ICMP)
+------------------+
         |
+--------+---------+
|      ARP         |  Address resolution
+------------------+
         |
+--------+---------+
|    Ethernet      |  Data link layer
+------------------+
         |
+--------+---------+
|  E1000 Driver    |  Intel 82540EM (MMIO, DMA, IRQ)
+------------------+
```

### QEMU Networking

- Guest IP: `10.0.2.15` (QEMU user-mode NAT)
- Gateway: `10.0.2.2`
- DNS: `10.0.2.3`
- DHCP auto-configuration at boot

---

## Audio

### Audio Drivers

anyOS includes two audio codec drivers for PCM playback: **AC'97** (legacy) and **Intel HDA** (High Definition Audio).

### AC'97 Driver

| Property | Value |
|----------|-------|
| **PCI Device** | Intel 82801AA (8086:2415) |
| **PCI Class** | 0x04 (Multimedia), Subclass 0x01 (Audio) |
| **Register Access** | I/O ports (BAR0 = mixer, BAR1 = bus master) |
| **Sample Rate** | 48,000 Hz (AC'97 native) |
| **Format** | 16-bit signed little-endian stereo (4 bytes/frame) |
| **DMA** | 32-entry Buffer Descriptor List, 4 KiB per buffer |

**Key registers:**

- **NAMBAR** (BAR0): Native Audio Mixer -- volume, sample rate, codec control
- **NABMBAR** (BAR1): Native Audio Bus Master -- DMA control, buffer descriptors, status

**Playback flow:**

1. User program calls `audio_write()` syscall with PCM data
2. Kernel copies data into identity-mapped DMA buffers
3. Buffer Descriptor List (BDL) entry updated with address + sample count
4. Last Valid Index (LVI) advanced to tell hardware about new data
5. Hardware DMAs buffer data to DAC, generates audio output
6. IRQ fires on buffer completion, acknowledges status

**DMA memory layout:**

| Structure | Size | Location |
|-----------|------|----------|
| BDL (32 entries x 8 bytes) | 256 bytes | 1 physical frame |
| Audio buffers (32 x 4 KiB) | 128 KiB | 32 physical frames |

All DMA structures are in identity-mapped memory (physical < 128 MiB).

**QEMU:** `-device AC97,audiodev=audio0 -audiodev coreaudio,id=audio0` (macOS)

### WAV File Support

The standard library includes a WAV parser that handles format conversion:

- **Input:** PCM WAV files (RIFF/WAVE, format tag 1)
- **Supported:** 8-bit/16-bit, mono/stereo, any sample rate
- **Output:** Resampled to 48 kHz 16-bit stereo (nearest-neighbor)
- 8-bit unsigned samples converted to 16-bit signed
- Mono channels duplicated to stereo

---

## USB Subsystem

anyOS includes USB host controller drivers for device connectivity.

### Controllers

| Controller | Standard | PCI Class | Features |
|------------|----------|-----------|----------|
| **UHCI** | USB 1.1 | 0x0C03/0x00 | 12 Mbps, polled I/O, keyboard/mouse/storage |
| **EHCI** | USB 2.0 | 0x0C03/0x20 | 480 Mbps, async/periodic schedules |

### Device Support

- **HID**: USB keyboards and mice (via polling thread)
- **Mass Storage**: USB storage devices (Bulk-Only Transport)
- Hub detection and port enumeration

QEMU flags: `-device qemu-xhci` or `-device usb-ehci` with `-device usb-kbd`, `-device usb-mouse`, etc.

---

## IPC Architecture

### Pipes

Kernel-managed byte streams for inter-process communication.

- **Named pipes**: Identified by string names, create/open/read/write/close semantics, ring buffer with 64 KiB default capacity
- **Anonymous pipes**: POSIX-style `pipe()` syscall for parent-child IPC, used by shell for pipelines (`cmd1 | cmd2`)
- Used by terminal for process output capture (`spawn_piped`)

### Message Queues

Bounded message queues for structured IPC between processes.

- `Message` struct with sender PID, message type, and variable-length payload
- Create/send/receive/destroy semantics

### Signals

POSIX-style signal delivery for process notification.

- Signal handlers: `sigaction` for registering handlers, `kill` for sending signals
- Supported signals: SIGUSR1, SIGCHLD, SIG_IGN, SIG_DFL
- Used by test suite for verifying process lifecycle

### Event Bus

Two-tier event system for decoupled communication:

**System Event Bus:**
- Global broadcast channel for system-wide events (process lifecycle, hardware events)
- Subscribe with optional filter, poll for events

**Module Event Channels:**
- Named channels for scoped communication (e.g., compositor IPC)
- Supports targeted emit to a specific subscriber (`evt_chan_emit_to`)
- Events are 5 x u32 values: `[type, p1, p2, p3, p4]`

### Shared Memory (SHM)

Page-granular shared memory regions for zero-copy data transfer.

- `shm_create(size)` allocates physical pages
- `shm_map(id)` maps into the calling process's address space
- Multiple processes can map the same SHM region
- Used by libcompositor for window pixel buffers

---

## User Identity System

anyOS supports multi-user identity with per-process UID/GID.

- **User accounts**: username, password (MD5 hashed), full name, home directory
- **Groups**: name + GID
- **Authentication**: `sys_authenticate(user, pass)` verifies credentials
- **Identity switching**: `sys_set_identity(uid)` changes the process UID
- **File ownership**: `chmod` and `chown` syscalls for permission management
- User database stored in `/System/users/`

### App Permissions

anyOS enforces a runtime permission system similar to macOS/Android for `.app` bundles:

- **Capability bitmask**: 14 capability bits enforced at syscall dispatch
- **Sensitive capabilities** (require user consent): Filesystem, Network, Audio, Display, Device, Process, System, Compositor
- **Auto-granted capabilities** (infrastructure, no prompt): DLL, Thread, SHM, Event, Pipe
- **Permission storage**: `/System/users/perm/{uid}/{app_id}` files containing `granted=0x{hex}`

**First-launch flow:**

1. `SYS_SPAWN` detects `.app` bundle with sensitive capabilities and no stored permission → returns `PERM_NEEDED`
2. Stdlib `spawn()` launches `/System/permdialog` (a modal dialog with dimmed background)
3. User selects which permissions to grant via checkboxes → stored via `SYS_PERM_STORE`
4. Stdlib retries spawn — kernel intersects declared capabilities with user-granted capabilities

**Settings app** provides an "Apps" page where users can review and toggle per-app permissions or reset them entirely (triggers re-prompt on next launch).

---

## Syscall Interface

### Calling Conventions

anyOS supports two syscall paths:

**SYSCALL instruction (64-bit Rust programs):**

| Register | Purpose |
|----------|---------|
| RAX | Syscall number (in) / return value (out) |
| RBX | Argument 1 |
| R10 | Argument 2 (not RCX -- SYSCALL clobbers it) |
| RDX | Argument 3 |
| RSI | Argument 4 |
| RDI | Argument 5 |

**INT 0x80 (32-bit C/TCC programs, compatibility mode):

| Register | Purpose |
|----------|---------|
| EAX | Syscall number (in) / return value (out) |
| EBX | Argument 1 |
| ECX | Argument 2 |
| EDX | Argument 3 |
| ESI | Argument 4 |
| EDI | Argument 5 |

### Syscall Categories

There are 140 syscalls organized by category:

| Category | Count | Examples |
|----------|-------|----------|
| Process Management | 14 | exit, spawn, fork, exec, kill, sleep, sbrk, waitpid, getppid |
| Threading | 3 | thread_create, set_priority, set_critical |
| File I/O | 16 | read, write, open, close, readdir, stat, mkdir, symlink, rename |
| Mount | 3 | mount, umount, list_mounts |
| Memory | 3 | sbrk, mmap, munmap |
| Networking | 24 | ping, dhcp, dns, tcp_*, udp_*, net_poll |
| Pipes/IPC | 11 | pipe_create/read/write, evt_chan_*, evt_sys_* |
| POSIX Pipes/FD | 4 | pipe2, dup, dup2, fcntl |
| Shared Memory | 4 | shm_create, shm_map, shm_unmap, shm_destroy |
| Signals | 2 | sigaction, sigprocmask |
| Window Manager | 13 | win_create, draw_text, blit, present |
| Display/GPU | 8 | set_resolution, set_wallpaper, capture_screen, gpu_vram_size |
| Compositor | 5 | map_framebuffer, gpu_command, input_poll |
| Audio | 2 | audio_write, audio_ctl |
| DLL | 2 | dll_load, set_dll_u32 |
| Device/System | 10 | time, uptime, sysinfo, devlist, random |
| Environment | 3 | setenv, getenv, listenv |
| Keyboard | 3 | kbd_get_layout, kbd_set_layout, kbd_list_layouts |
| User/Identity | 16 | getuid, authenticate, adduser, chpasswd, getppid |
| App Permissions | 5 | perm_check, perm_store, perm_list, perm_delete, perm_pending_info |
| Filesystem ext | 3 | chmod, chown, chdir |
| Capabilities | 1 | get_capabilities |

See [syscalls reference](syscalls.md) for the complete list with all arguments and return values.

---

## DLL System

### Design

anyOS uses two shared library formats at fixed virtual addresses (0x04000000+):

- **DLIB (legacy)**: Built as `bin` crates with custom linker scripts. Binary format: `DLIB` magic header + `#[repr(C)]` export function pointer table. Kernel loads DLIB pages at boot, maps into every new process page directory. Client programs read function pointers from the export table at the known base address.
- **.so (modern)**: Built as `staticlib` crates, linked by `anyld` into ELF64 ET_DYN shared objects with `.dynsym`/`.dynstr`/`.hash` sections. Loaded on demand via `SYS_DLL_LOAD` (syscall 80). Client programs resolve symbols at runtime using `dl_open`/`dl_sym` (ELF hash lookup).

### Library Overview

| Library | Format | Base Address | Exports | Description |
|---------|--------|-------------|---------|-------------|
| **uisys** | DLIB | `0x04000000` | 81 | macOS-style UI components (31 component types) |
| **libimage** | DLIB | `0x04100000` | 7 | Image/video decoding (BMP, PNG, JPEG, GIF, ICO, MJV) + scaling |
| **librender** | DLIB | `0x04300000` | 18 | 2D rendering primitives (shapes, gradients, anti-aliasing) |
| **libcompositor** | DLIB | `0x04380000` | 16 | Window management IPC (SHM surfaces, event channels) |
| **libanyui** | .so | `0x04400000` | 111 | anyui UI framework (42 controls, Windows Forms-style) |
| **libfont** | .so | `0x05000000` | 7 | TrueType font rendering (gamma-corrected greyscale + LCD subpixel AA), system fonts embedded in .rodata |

### uisys.dlib

The main UI system DLIB provides 81 exported functions implementing 31 UI components:
- Inputs: Button, Toggle, Checkbox, Radio, Slider, Stepper, TextField, SearchField, TextArea
- Layout: Sidebar, NavigationBar, Toolbar, TabBar, SegmentedControl, SplitView, ScrollView
- Data: TableView, ContextMenu, Card, GroupBox, Badge, Tag, ProgressBar
- Display: Label, Tooltip, StatusIndicator, ColorWell, ImageView, IconButton, Divider, Alert
- v2 API: GPU acceleration, anti-aliased shapes, shadow/blur effects, font-aware text

See [uisys API](uisys-api.md) for the complete component reference.

### libimage.dlib

Image and video decoding library. Stateless, heap-free -- callers provide all memory.

| Format | Features |
|--------|----------|
| **BMP** | 24-bit RGB, 32-bit ARGB |
| **PNG** | 8-bit RGB/RGBA/grayscale, DEFLATE, all filter types |
| **JPEG** | Baseline DCT, 4:2:0/4:2:2/4:4:4, LLM fast integer IDCT |
| **GIF** | LZW, transparency, interlacing (first frame) |
| **ICO** | Multi-size selection, BMP-in-ICO (1/4/8/24/32bpp), PNG-in-ICO |
| **MJV** | Motion JPEG Video container (per-frame JPEG decode) |

Also provides bilinear image scaling (stretch/contain/cover modes).

See [libimage API](libimage-api.md) for the complete reference.

### libfont.so

TrueType font rendering with greyscale and LCD subpixel anti-aliasing. System fonts (SF Pro family + Andale Mono, ~17 MiB) are embedded in `.rodata` via `include_bytes!()`, so the font data is shared read-only across all processes — zero disk I/O at init, zero per-process memory duplication.

Loaded on demand via `SYS_DLL_LOAD` when first needed. Custom fonts can still be loaded from disk via `font_load()`.

See [libfont API](libfont-api.md) for the complete reference.

### librender.dlib

2D software rendering primitives: filled/outlined shapes (rect, rounded rect, circle, line), horizontal/vertical gradients, anti-aliased variants. Operates on caller-provided pixel buffers.

See [librender API](librender-api.md) for the complete reference.

### libcompositor.dlib

IPC-based window management for GUI applications. Uses shared memory (SHM) pixel buffers and event channels to communicate with the compositor process. Provides window lifecycle, menu bars, status icons, and blur-behind effects.

See [libcompositor API](libcompositor-api.md) for the complete reference.

---

## Build System Tools

anyOS uses four native C99 tools for the build pipeline. They are compiled at the start of each build (before any programs) and replace all Python build scripts. Each tool supports `ONE_SOURCE` single-file compilation for TCC, making them available for self-hosted builds directly on anyOS.

### anyelf — ELF Conversion Tool

Converts ELF binaries into the formats used by the kernel and loader.

| Mode | Input | Output | Description |
|------|-------|--------|-------------|
| `bin` | ELF64/ELF32 | flat binary | Loads PT_LOAD segments by vaddr, outputs contiguous bytes. Used for user programs. |
| `dlib` | ELF64 | DLIB v3 | anyOS shared library format: 4096-byte header + read-only pages + `.data` template. |
| `kdrv` | ELF64 | KDRV | Kernel driver format: 4096-byte header + code pages + data pages + exports offset. |

**Usage:** `anyelf <mode> <input.elf> <output> [options]`

**DLIB v3 format:**

```
Offset  Size  Content
0x000   4     Magic "DLIB"
0x004   4     Version (3)
0x008   8     RO size (bytes)
0x010   8     Data template size (bytes)
0x018   8     BSS size (bytes, zero-filled at load)
0x020   8     Entry point offset (into RO region)
0x1000  ...   RO pages (code + rodata)
...     ...   Data template pages (.data initial values)
```

### mkimage — Disk Image Builder

Creates bootable disk images from bootloader, kernel ELF, and sysroot directory tree. Supports **incremental updates** — by default, only modified files are rewritten to the existing image. Use `--reset` to force a full rebuild.

| Mode | Flag | Layout | Filesystem |
|------|------|--------|------------|
| BIOS | *(default)* | MBR + kernel sectors + filesystem partition | exFAT |
| UEFI | `--uefi` | GPT + EFI System Partition (FAT16) + data partition | exFAT |
| ISO | `--iso` | ISO 9660 + El Torito boot catalog | ISO 9660 |

**Usage:**
```
mkimage --stage1 s1.bin --stage2 s2.bin --kernel kernel.elf \
        --output disk.img --image-size 256 --sysroot sysroot/ --fs-start 8192
```

**BIOS image layout:**

```
Sector 0        MBR (stage1, 512 bytes)
Sectors 1-7     Stage 2 bootloader
Sector 8+       Kernel flat binary (converted from ELF by paddr)
Sector fs-start exFAT filesystem with sysroot contents
```

**exFAT features:**
- Boot sector + backup, allocation bitmap, upcase table
- 4 KiB clusters, contiguous allocation preferred
- VFAT-style long filenames (File + Stream + FileName entry sets)
- `ROOT_ONLY_DIRS` support (`/System/sbin/`, `/System/users/perm/`) for permission enforcement

### anyld — ELF64 Shared Object Linker

Links ELF64 relocatable objects (`.o`) and AR archives (`.a`) into a shared object (`ET_DYN`).

**Usage:** `anyld -o output.so input1.o input2.o libfoo.a`

**Features:**
- Reads ELF64 relocatable objects and GNU AR archives
- Merges `.text`, `.rodata`, `.data`, `.bss` sections with alignment
- Resolves symbols with standard precedence (strong > weak > undefined)
- Applies x86_64 relocations: `R_X86_64_64`, `R_X86_64_PC32`, `R_X86_64_32`, `R_X86_64_32S`, `R_X86_64_PLT32`
- Generates ELF64 ET_DYN output with `.dynsym`, `.dynstr`, `.hash`, `.dynamic` sections
- Global symbols exported in `.dynsym` for runtime linking

### mkappbundle — Application Bundle Creator

Validates and assembles `.app` bundle directories from metadata, executables, icons, and resources.

**Usage:** `mkappbundle -i Info.conf -e <binary> [-c Icon.ico] [-r resource]... -o Output.app`

**Features:**
- Validates `Info.conf` metadata (required keys: id, name, exec, version, category)
- Validates capability names and application categories
- Auto-converts ELF binaries to flat binary via `anyelf` (or `--keep-elf` to skip)
- Validates ICO icon format (Windows ICO header check)
- Recursive resource directory copying (max 64 resources)
- Cross-platform (Unix/Windows)

**Info.conf format:**
```ini
id=com.anyos.appname        # Reverse-DNS identifier (required)
name=App Name               # Display name (required)
exec=AppName                # Executable filename in bundle (required)
version=1.0                 # Version string (required)
category=Utilities          # Category (required)
capabilities=filesystem,dll # Comma-separated capability list
```

### Self-Hosting

Build tool sources are installed to `/Libraries/system/buildsystem/` on the disk image. On anyOS, they can be compiled with TCC:

```bash
cd /Libraries/system/buildsystem/anyelf
make CC=cc one    # builds anyelf with TCC in ONE_SOURCE mode

cd /Libraries/system/buildsystem/mkimage
make CC=cc one    # builds mkimage with TCC

cd /Libraries/system/buildsystem/anyld
make CC=cc one    # builds anyld with TCC

cd /Libraries/system/buildsystem/mkappbundle
make CC=cc one    # builds mkappbundle with TCC
```
