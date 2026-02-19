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

---

## Boot Process

anyOS supports three boot methods: BIOS (MBR + FAT16), UEFI (GPT + exFAT), and ISO (El Torito).

### BIOS Boot (MBR + FAT16)

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
0xFD000000 - 0xFDFFFFFF                      Framebuffer (16 MiB, mapped via 4K pages)
PML4[510] recursive self-mapping              Page table access
```

### Virtual Memory (User Process)

```
0x04000000 - 0x07FFFFFF    DLL mappings:
                             0x04000000 = uisys.dll
                             0x04100000 = libimage.dll
                             0x04200000 = libfont.dll
                             0x04300000 = librender.dll
                             0x04380000 = libcompositor.dll
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
    +--------+---------+---------+--------+
    |        |         |         |        |
+---+---+ +--+--+ +---+---+ +---+--+ +---+---+
|arch/  | |mem/ | |drivers/| |task/ | |syscall/|
|x86    | |     | |        | |      | |        |
+-------+ +-----+ +--------+ +------+ +--------+
    |        |         |         |        |
    |   +----+----+    |    +----+----+   |
    |   |physical | +--+--+ |scheduler|   |
    |   |virtual  | |GPU  | |loader   |   |
    |   |heap     | |E1000| |thread   |   |
    |   +---------+ |ATA  | |process  |   |
    |               |AHCI | +---------+   |
    |               |input|               |
    +--+  +--+      +-----+              |
    |GDT| |IDT|                           |
    |TSS| |PIC|   +-------+  +-----+     |
    |PIT| |APIC|  |  fs/  |  | net/|     |
    +---+ +----+  | FAT16 |  | TCP |     |
                  | VFS   |  | UDP |     |
                  +-------+  | DNS |     |
                             +-----+     |
              +----------+               |
              |graphics/ |  +-----+      |
              |compositor|  | ui/ |      |
              |surface   |  |desk |      |
              |font      |  |dock |      |
              +----------+  +-----+      |
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
8. **PCI + HAL** -- Bus enumeration, driver binding (GPU, NIC, ATA/AHCI, AC'97, USB)
9. **APIC** -- Local APIC + I/O APIC setup, LAPIC timer calibrated from TSC
10. **SMP** -- AP (Application Processor) startup via INIT-SIPI-SIPI sequence
11. **SYSCALL/SYSRET** -- MSR configuration (EFER.SCE, STAR, LSTAR, SFMASK)
12. **Scheduler** -- Thread system, idle task per CPU, round-robin with priorities
13. **Keyboard/Mouse** -- PS/2 driver with IntelliMouse scroll wheel
14. **DLL Loading** -- Map all DLLs into kernel PD (uisys, libimage, libfont, librender, libcompositor)
15. **Userspace** -- Load `/System/init` as first Ring 3 process, which starts the compositor

---

## Process Model

### Threads & Scheduling

- Each "process" is one or more **kernel threads** sharing the same page directory
- **SMP-aware round-robin** scheduler with 1ms time slices (LAPIC timer at 1000 Hz)
- Thread states: `Ready`, `Running`, `Sleeping`, `Blocked`, `Dead`
- Thread priorities (0=highest, 255=lowest) affect scheduling order
- Context switch saves/restores: RAX-RDI, R8-R15, RSP, RBP, RIP, RFLAGS, CR3, FPU state (FXSAVE/FXRSTOR)
- Eager FPU switching: save/restore 512-byte `FxState` on every context switch
- Per-CPU idle threads, one scheduler lock with `try_lock()` contention handling

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

anyOS supports two GPU backends via the `GpuDriver` trait:

| Driver | PCI ID | Features |
|--------|--------|----------|
| **Bochs VGA** | 1234:1111 | VESA VBE, DISPI registers, page flipping (double buffer) |
| **VMware SVGA II** | 15AD:0405 | FIFO command queue, 2D acceleration (rect fill/copy), hardware cursor |

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

### FAT16 (BIOS Boot)

- **256 MiB disk image** with FAT16 filesystem
- **8 sectors/cluster** (4 KiB clusters)
- **VFAT long filenames**: LFN entries with UTF-16 to ASCII conversion
- **Operations**: read, write, create, delete, mkdir, readdir, stat, seek, symlink, chmod, chown
- **Storage dispatch**: Routes I/O to the active backend (auto-detected at boot)
  - **ATA PIO**: 28-bit LBA, sector read/write via I/O ports (legacy IDE, default)
  - **AHCI DMA**: SATA DMA transfers via MMIO + bounce buffer (ICH9 AHCI, `--ahci` flag)

### exFAT (UEFI Boot)

- Used on the UEFI GPT disk image
- Same VFS interface as FAT16 -- user programs see no difference
- Supports large files and modern partition layouts

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

### AC'97 Driver

anyOS includes an Intel AC'97 audio codec driver for PCM playback.

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

### Named Pipes

Kernel-managed byte streams for inter-process communication.

- Named pipes identified by string names
- Create/open/read/write/close semantics
- Used by terminal for process output capture (`spawn_piped`)
- Ring buffer with 64 KiB default capacity

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
| RDI | Argument 1 |
| RSI | Argument 2 |
| RDX | Argument 3 |
| R10 | Argument 4 (not RCX -- SYSCALL clobbers it) |
| R8 | Argument 5 |
| R9 | Argument 6 |

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

There are 118 syscalls organized by category:

| Category | Syscall Numbers | Count | Examples |
|----------|----------------|-------|----------|
| Process Management | 1, 6-9, 12-13, 27-29 | 10 | exit, spawn, kill, sleep, sbrk, waitpid |
| Threading | 130-132 | 3 | thread_create, set_priority, set_critical |
| File I/O | 2-5, 23-25, 90-92, 105-108, 93 | 16 | read, write, open, close, readdir, stat, mkdir, symlink |
| Mount | 94-96 | 3 | mount, umount, list_mounts |
| Memory | 9, 133-134 | 3 | sbrk, mmap, munmap |
| Networking | 40-44, 100-104, 140-144 | 15 | ping, dhcp, dns, tcp_*, udp_* |
| Pipes/IPC | 45-49, 60-68 | 11 | pipe_create/read/write, evt_chan_*, evt_sys_* |
| Shared Memory | 75-78 | 4 | shm_create, shm_map, shm_unmap, shm_destroy |
| Window Manager | 50-59, 70-72 | 13 | win_create, draw_text, blit, present |
| Display/GPU | 110-117 | 8 | set_resolution, set_wallpaper, capture_screen |
| Compositor | 150-154 | 5 | map_framebuffer, gpu_command, input_poll |
| Audio | 120-121 | 2 | audio_write, audio_ctl |
| DLL | 80-81 | 2 | dll_load, set_dll_u32 |
| Device/System | 30-33, 160-165 | 10 | time, uptime, sysinfo, devlist, random |
| Environment | 170-172 | 3 | setenv, getenv, listenv |
| Keyboard | 180-182 | 3 | kbd_get_layout, kbd_set_layout, kbd_list_layouts |
| User/Identity | 190-201 | 14 | getuid, authenticate, adduser, chpasswd |
| App Permissions | 250-254 | 5 | perm_check, perm_store, perm_list, perm_delete, perm_pending_info |
| Filesystem ext | 97-99 | 3 | chmod, chown, chdir |

See [syscalls reference](syscalls.md) for the complete list with all arguments and return values.

---

## DLL System

### Design

- DLLs are **stateless shared code** at fixed virtual addresses (0x04000000+)
- Built as `bin` crates with custom linker scripts
- Binary format: `DLIB` magic header + `#[repr(C)]` export function pointer table
- Kernel loads DLL pages at boot, maps into every new process page directory
- Client programs read function pointers from the export table at the known base address

### DLL Overview

| DLL | Base Address | Exports | Description |
|-----|-------------|---------|-------------|
| **uisys.dll** | `0x04000000` | 84 | macOS-style UI components (31 component types) |
| **libimage.dll** | `0x04100000` | 7 | Image/video decoding (BMP, PNG, JPEG, GIF, ICO, MJV) + scaling |
| **libfont.dll** | `0x04200000` | 7 | TrueType font rendering (greyscale + LCD subpixel AA) |
| **librender.dll** | `0x04300000` | 18 | 2D rendering primitives (shapes, gradients, anti-aliasing) |
| **libcompositor.dll** | `0x04380000` | 16 | Window management IPC (SHM surfaces, event channels) |

### uisys.dll

The main UI system DLL provides 84 exported functions implementing 31 UI components:
- Inputs: Button, Toggle, Checkbox, Radio, Slider, Stepper, TextField, SearchField, TextArea
- Layout: Sidebar, NavigationBar, Toolbar, TabBar, SegmentedControl, SplitView, ScrollView
- Data: TableView, ContextMenu, Card, GroupBox, Badge, Tag, ProgressBar
- Display: Label, Tooltip, StatusIndicator, ColorWell, ImageView, IconButton, Divider, Alert
- v2 API: GPU acceleration, anti-aliased shapes, shadow/blur effects, font-aware text

See [uisys API](uisys-api.md) for the complete component reference.

### libimage.dll

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

### libfont.dll

TrueType font rendering with greyscale and LCD subpixel anti-aliasing. System fonts loaded from `/System/Fonts/`.

See [libfont API](libfont-api.md) for the complete reference.

### librender.dll

2D software rendering primitives: filled/outlined shapes (rect, rounded rect, circle, line), horizontal/vertical gradients, anti-aliased variants. Operates on caller-provided pixel buffers.

See [librender API](librender-api.md) for the complete reference.

### libcompositor.dll

IPC-based window management for GUI applications. Uses shared memory (SHM) pixel buffers and event channels to communicate with the compositor process. Provides window lifecycle, menu bars, status icons, and blur-behind effects.

See [libcompositor API](libcompositor-api.md) for the complete reference.
