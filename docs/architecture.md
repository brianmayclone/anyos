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
- [Syscall Interface](#syscall-interface)
- [DLL System](#dll-system)

---

## Boot Process

anyOS uses a custom two-stage bootloader written in NASM assembly.

### Stage 1 (MBR)

- 512 bytes, loaded at `0x7C00` by BIOS
- Reads Stage 2 from disk sectors 1-3
- Jumps to Stage 2

### Stage 2

1. **A20 Line** -- Enables access to memory above 1 MB
2. **E820 Memory Map** -- Queries BIOS for available physical memory regions
3. **VESA VBE** -- Sets graphics mode to 1024x768x32bpp (or best available)
4. **Protected Mode** -- Sets up GDT, switches CPU to 32-bit protected mode
5. **Kernel Loading** -- Reads kernel flat binary from disk to physical address `0x100000`
6. **Paging** -- Enables 4 MB PSE pages: identity-maps first 8 MB, maps kernel to `0xC0000000` (higher-half), maps framebuffer
7. **Jump to Kernel** -- Transfers control to `0xC0100000` (kernel entry point)

### Boot Info

Stage 2 passes a `BootInfo` struct at a known address containing:
- Framebuffer address, width, height, pitch
- E820 memory map entries
- Disk geometry

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
0x00000000 - 0x01FFFFFF    Identity-mapped (first 32 MB, for DMA/legacy)
0xC0000000 - 0xC0FFFFFF    Kernel code + data (higher-half mapping)
0xC1000000+                 Kernel heap (grows via linked-list allocator)
0xD0000000 - 0xD001FFFF    E1000 MMIO (128 KiB)
0xD0020000 - 0xD002FFFF    VMware SVGA FIFO (256 KiB)
0xFD000000 - 0xFDFFFFFF    Framebuffer (identity-mapped via 4K pages)
0xFFC00000 - 0xFFFFFFFF    Recursive page directory self-mapping
```

### Virtual Memory (User Process)

```
0x04000000 - 0x07FFFFFF    DLL mappings (uisys.dll at 0x04000000)
0x08000000 - 0x080XXXXX    Program text + data + BSS
0x080XXXXX - 0x0BFEFFFF    Heap (grows via sbrk)
0x0BFF0000 - 0x0BFFFFFF    User stack (64 KiB, grows downward)
0xC0000000+                 Kernel space (not accessible from Ring 3)
```

### Paging

- **2-level paging**: Page Directory (1024 PDEs) + Page Tables (1024 PTEs each)
- **4 KiB pages** for fine-grained mapping
- **Recursive mapping**: PDE 1023 points to the page directory itself, enabling access to page tables at `0xFFC00000`
- Each process has its own page directory; kernel PDEs are cloned into every process

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
    |               |input| +---------+   |
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

The kernel initializes subsystems in 10 phases:

1. **Serial** -- Debug output via COM1
2. **Boot Info** -- Parse framebuffer, memory map from bootloader
3. **GDT + IDT** -- CPU descriptor tables, interrupt handlers
4. **Physical Memory** -- Frame allocator from E820 map
5. **Virtual Memory** -- Page tables, kernel heap
6. **PCI + HAL** -- Bus enumeration, driver binding (GPU, NIC, ATA)
7. **Interrupts** -- PIC/APIC setup, keyboard, mouse, timer (100 Hz PIT)
8. **Scheduler** -- Thread system, idle task
9. **Graphics** -- Compositor, desktop environment, GPU acceleration
10. **Userspace** -- Load `/system/init` as first Ring 3 process

---

## Process Model

### Threads & Scheduling

- Each "process" is one or more **kernel threads** sharing the same page directory
- **Round-robin** scheduler with 10ms time slices (PIT at 100 Hz)
- Thread states: `Ready`, `Running`, `Sleeping`, `Blocked`, `Dead`
- Context switch saves/restores: EAX-EDI, ESP, EBP, EIP, EFLAGS, CR3

### Ring 3 User Mode

- **GDT segments**: Kernel Code (0x08), Kernel Data (0x10), User Code (0x1B), User Data (0x23), TSS (0x28)
- **Syscalls**: `int 0x80` trap gate (DPL=3), args in EAX (number), EBX-EDI (params)
- **TSS**: ESP0 updated on every context switch for kernel stack
- **Per-process address spaces**: Each process gets a cloned page directory

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

### FAT16

- **64 MiB disk image** with FAT16 filesystem
- **8 sectors/cluster** (4 KiB clusters)
- **VFAT long filenames**: LFN entries with UTF-16 to ASCII conversion
- **Operations**: read, write, create, delete, mkdir, readdir, stat, seek
- **ATA PIO driver**: 28-bit LBA, sector read/write via I/O ports

### Virtual File System (VFS)

- **File descriptors**: Global FD table, per-process open files
- **Paths**: `/bin/`, `/system/`, `/include/`, `/lib/`
- **Device files**: `/dev/serial`, `/dev/null`, `/dev/random`
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

## Syscall Interface

Syscalls use `int 0x80` with the following register convention:

| Register | Purpose |
|----------|---------|
| EAX | Syscall number (in) / return value (out) |
| EBX | Argument 1 |
| ECX | Argument 2 |
| EDX | Argument 3 |
| ESI | Argument 4 |
| EDI | Argument 5 |

There are 100+ syscalls organized by category:

- **Process** (1-13, 27-29): exit, spawn, kill, sleep, sbrk, waitpid
- **Filesystem** (2-5, 23-25, 90-92, 105-108): read, write, open, close, readdir, stat, mkdir, unlink
- **Time/System** (30-33): time, uptime, sysinfo, dmesg
- **Networking** (40-44, 100-104): config, ping, dhcp, dns, arp, tcp_connect/send/recv/close
- **IPC** (45-49, 60-68): pipes, event bus, module channels
- **Window Manager** (50-59, 70-72): create, destroy, draw, events, screen info
- **DLL** (80): dll_load
- **Display** (110-112): set_resolution, list_resolutions, gpu_info

See [stdlib API](stdlib-api.md) for the complete reference.

---

## DLL System

### Design

- DLLs are **stateless shared code** at fixed virtual addresses (0x04000000+)
- Built as `bin` crates with custom linker scripts
- Binary format: `DLIB` magic header + `#[repr(C)]` export function pointer table
- Kernel loads DLL pages at boot, maps into every new process page directory
- Client programs read function pointers from the export table at the known base address

### uisys.dll

The main UI system DLL provides 73 exported functions implementing 30 UI components:
- Buttons, toggles, checkboxes, radio buttons, sliders
- Text fields, search fields, text areas
- Tables, sidebars, tab bars, navigation bars
- Cards, badges, tags, tooltips, progress bars
- And more...

See [uisys API](uisys-api.md) for the complete component reference.
