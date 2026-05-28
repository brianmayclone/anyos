# anyOS Architecture Overview

This document describes the internal architecture of anyOS, from boot to desktop.

## Table of Contents

- [Boot Process](#boot-process)
- [Memory Layout](#memory-layout)
- [Kernel Architecture](#kernel-architecture)
- [ARM64 Port](#arm64-port)
- [Process Model](#process-model)
- [Security](#security)
- [Graphics & Compositor](#graphics--compositor)
- [Filesystem](#filesystem)
- [Networking](#networking)
- [Bluetooth Subsystem](#bluetooth-subsystem)
- [USB Subsystem](#usb-subsystem)
- [IPC Architecture](#ipc-architecture)
- [User Identity System](#user-identity-system)
- [Syscall Interface](#syscall-interface)
- [Audio](#audio)
- [DLL System](#dll-system)
- [Build System Tools](#build-system-tools)

---

## Boot Process

anyOS supports three boot methods: **BIOS** (two-stage MBR bootloader with graphical boot menu), **UEFI** (Rust EFI application), and **ISO** (El Torito CD-ROM).

The BIOS bootloader features:
- **Graphical splash screen** with boot logo and timeout countdown
- **Interactive boot menu** with keyboard navigation (Up/Down/Enter)
- **INI-style `boot.cfg`** supporting multiple boot entries, kernel parameters, and chainloading to other operating systems
- **Custom boot parameters** — `params=custom` prompts for interactive input at boot time

The bootloader passes a `BootInfo` struct at `0x9000` to the kernel containing framebuffer info, E820 memory map, disk geometry, boot mode, and boot parameters (64-byte string from `boot.cfg`).

See **[Bootloader Documentation](bootloader.md)** for the complete reference including boot.cfg format, memory layout, and source file inventory.

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
                             0x05000000 = libfont.so (~517 KiB, fonts via fontd SHM)
0x08000000 - 0x080XXXXX    Program text + data + BSS (ELF64/ELF32)
0x080XXXXX - 0x0BFEFFFF    Heap (grows via sbrk)
0x20000000+                mmap region (base randomized ±16 MiB by ASLR)
0x0BFF0000 - 0x0BFFFFFF    User stack (64 KiB, top randomized ±1 MiB by ASLR)
0xFFFFFFFF80000000+         Kernel space (not accessible from Ring 3)
```

> **ASLR**: Stack top and mmap base are randomized at each process launch using RDRAND hardware entropy (TSC-based xorshift64 fallback on CPUs without RDRAND). ET_EXEC program text remains at fixed ELF-header addresses; PIE binaries would be required for full load-address randomization.

### Paging

- **4-level paging**: PML4 → PDPT → PD → PT (x86_64 long mode)
- **4 KiB pages** for fine-grained mapping
- **Recursive mapping**: PML4[510] points to the PML4 itself, enabling access to all paging structures
- Kernel at PML4[511], PDPT[510] (higher-half `0xFFFFFFFF80000000`)
- Each process has its own PML4; kernel entries are cloned into every process

---

## Kernel Architecture

anyOS uses a **hybrid kernel** architecture. Filesystems, the TCP/IP network stack, and device drivers all run in kernel space for performance, while the compositor, GUI framework, and system services run as userspace processes communicating via IPC.

### Module Overview

```
                      +-----------+
                      |  boot/    |  Kernel entry, staged boot flow
                      +-----+-----+
                            |
    +--------+---------+----+----+--------+--------+--------+
    |        |         |         |        |        |        |
+---+---+ +--+--+ +---+---+ +---+--+ +---+---+ +--+--+ +--+--+
|arch/  | |mem/ | |drivers/| |task/ | |syscall/| |ipc/ | |net/ |
|x86_64 | |     | |        | |      | | (232)  | |     | |     |
|arm64  | +-----+ +--------+ +------+ +--------+ +-----+ +-----+
+-------+    |         |         |                  |        |
    |   +----+----+ +--+--------+-+            +----+----+ +-+------+
    |   |physical | |GPU (7)     | |scheduler| |pipes   | |TCP/IP |
    |   |virtual  | |Network (8) | |loader   | |signals | |UDP    |
    |   |heap     | |Storage (7) | |thread   | |shm     | |DHCP   |
    |               |USB (3+cls) | |KDRV     | |events  | |DNS    |
    |               |Audio (2)   | +---------+ +--------+ |ARP    |
    |               |Bluetooth   |                        |WiFi   |
    +--+  +--+      |Input       |                        +-------+
    |GDT| |IDT|     |VirtIO      |
    |TSS| |PIC|     |I2C/SMBus   |    +----------+
    |PIT| |APIC|    |Thermal     |    |   fs/    |
    +---+ +----+    |Watchdog    |    | exFAT    |
                    +------------+    | FAT32    |
                                      | NTFS(ro) |
                                      | ISO 9660 |
                                      | OverlayFS|
                                      | RamFS    |
                                      | SMBFS    |
                                      | DevFS    |
                                      | VFS      |
                                      +----------+
```

### Init Sequence (`boot/mod.rs`, `boot/x86.rs`)

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

The crate root `main.rs` now acts only as the ABI entry wrapper; the actual boot orchestration lives in `kernel/src/boot/` so platform bring-up, storage discovery, and userspace launch remain structurally separated.

The kernel exposes swap only as a mechanism (`swapon`/`swapoff` and backing
slot I/O). Boot-time swap policy lives in userspace: `/System/init` registers
the `kernel` system configuration block with `confd`, reads `kernel/swap/*`,
prepares the configured file, and enables it before the normal service wave
starts. Defaults are `kernel/swap/enabled=true`, `kernel/swap/path=/swap`, and
`kernel/swap/size_mb=256`. See [Kernel Configuration](kernel-config.md).

### nogui Boot Mode

When the boot parameter `params=nogui` is set, the kernel skips the compositor and desktop entirely and directly launches `/System/bin/textmode_console`. This provides a full-screen text console on the framebuffer with login, an interactive shell, ANSI color support, scrollback, and cursor blinking — no GUI required.

See **[nogui Mode Documentation](nogui.md)** for the complete reference.

### Userspace Init Sequence

After the kernel hands off to `/System/init`, the init program:

1. Recovers interrupted upgrades if needed.
2. Waits for `confd` readiness.
3. Registers early system schemas such as `profile/power` and `kernel`.
4. Applies early kernel-adjacent policy such as CPU power profile and swap.
5. Launches `/System/bin/svc start-all`.

The **service manager** (`svc start-all`) registers and reads service
configuration in `confd` under `system/services/`, then starts each service
(e.g., `logd`, `networkd`, `dnsd`) with dependency resolution.

See [services.md](services.md) for the full service system documentation.

---

## ARM64 Port

anyOS has an in-progress ARM64 (AArch64) port targeting QEMU `virt` (with planned Raspberry Pi 4/5 support). The ARM64 kernel shares all architecture-independent subsystems (VFS, TCP/IP stack, IPC, scheduler, syscall handlers) with x86-64 via a HAL abstraction layer.

### Architecture-Specific Modules (`kernel/src/arch/arm64/`)

| Module | Description |
|--------|-------------|
| `boot.rs` | Boot entry, DTB (Device Tree Blob) address from X0, FDT parser |
| `context.rs` | Thread CPU context (X0-X30, SP, PC, SPSR, PSTATE) for context switching |
| `cpu_features.rs` | AArch64 CPU feature detection |
| `exceptions.rs` | VBAR_EL1 exception vector table, IRQ/SVC/fault dispatch from EL1 |
| `gic.rs` | GICv3 interrupt controller (Distributor GICD + Redistributor GICR, system register access) |
| `generic_timer.rs` | ARM Generic Timer (CNTPCT_EL0 monotonic counter, CNTP_TVAL_EL0 periodic 1000 Hz tick) |
| `mmu.rs` | VMSAv8-A MMU configuration (TCR_EL1, MAIR_EL1, 4-level page tables, 48-bit VA, 4 KiB granule) |
| `serial.rs` | PL011 UART driver (MMIO at 0x0900_0000 on QEMU virt) |
| `smp.rs` | SMP bring-up via PSCI CPU_ON (HVC #0), up to 16 CPUs, per-CPU kernel stacks |
| `syscall.rs` | SVC #0 syscall entry (X8=number, X0-X5=args, X0=return), dispatches to shared syscall handlers |
| `power.rs` | PSCI-based shutdown and reboot |

### Memory Model

- **4-level paging** (VMSAv8-A): PGD -> PUD -> PMD -> PTE, 4 KiB pages, 48-bit virtual addresses
- **TTBR0_EL1**: User address space (lower half)
- **TTBR1_EL1**: Kernel address space (upper half, `0xFFFF_0000_0000_0000+`)
- **MAIR_EL1**: Three memory attributes -- Device-nGnRnE (MMIO), Normal Non-Cacheable, Normal Write-Back Cacheable
- **Device MMIO**: Mapped via TTBR1 PUD[3] at `0xFFFF_0000_C000_0000` (1 GiB device region)

### Interrupt Controller (GICv3)

- GICD (Distributor) at PA `0x0800_0000`: SPI routing, enable/disable, priority
- GICR (Redistributor) at PA `0x080A_0000`: Per-CPU PPI/SGI configuration (128 KiB stride per CPU)
- System register access (ICC_*) for interrupt acknowledge/EOI, priority mask
- IRQ routing: Timer (PPI 30) -> scheduler tick, UART (SPI 33) -> serial input

### Current Status

The ARM64 port boots to kernel_main with working MMU, GICv3 interrupts, Generic Timer, SMP, and syscall dispatch. Userspace loading, storage drivers, and framebuffer are still planned; current follow-up work is tracked in `todos/github-issue-import.md`.

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
3. `load_elf()` / `load_flat()` -- Map program segments with NX flags, zero BSS
4. Thread starts at entry point in Ring 3 via `iret` (ASLR-randomised stack)
5. `sys_exit(code)` -- Thread terminates, pages freed

### Process Detach

By default, when a parent process terminates, all its children are cascade-killed. The `sys_detach(child_tid)` syscall (314) allows a parent to release ownership of a child — the child's `parent_tid` is set to 0 and it becomes a root process that survives the parent's exit. Used by `svc start-all` to spawn long-lived daemons that outlive the `svc` tool.

### Stack Guard Pages

Each user process has an 8 MiB stack (2048 pages) with the bottom page left **unmapped** as a guard page. If user code overflows the stack and touches the guard page, the CPU triggers a page fault:

- **User-mode fault**: Kernel detects the unmapped access, prints `USER STACK OVERFLOW!`, and kills the thread with SIGSEGV (exit code 139).
- **Kernel-mode fault during syscall**: If the kernel accesses the user stack during a syscall (e.g. `read`/`write` on user buffers) and hits the guard page, the fault handler still identifies the thread as a user thread and kills it cleanly via `try_exit_current`.

```
Stack layout (8 MiB, grows downward):
  ┌─────────────────────┐  ← aslr_stack_top (randomised)
  │  Usable stack       │  2047 pages (8 MiB − 4 KiB)
  │  (grows downward)   │
  ├─────────────────────┤  ← stack_usable_bottom
  │  GUARD PAGE         │  1 page (4 KiB, UNMAPPED)
  │  (triggers #PF)     │
  └─────────────────────┘  ← stack_bottom
```

Kernel stacks have a similar mechanism with a dedicated guard page at the bottom of each 512 KiB kernel stack, plus a canary word above the guard for early detection.

**Source:** `kernel/src/task/loader.rs` (guard page allocation), `kernel/src/arch/x86/idt.rs` (fault handler)

---

## Security

### NX-Bit / Data Execution Prevention (DEP)

anyOS enforces hardware DEP via the x86-64 NX (No-Execute) bit in page table entries:

- **EFER.NXE** (bit 11 of `IA32_EFER` MSR) is set at boot on every CPU after verifying CPUID `NX=true`. Without this bit, bit 63 in a PTE is treated as reserved and raises `#GP`; DEP only becomes active once the MSR is set.
- **`PAGE_NX` flag** (`bit 63` of any PTE): set by the virtual memory layer on data pages, cleared on executable pages.
- **ELF segment flags**: the loader reads `p_flags` (`PF_X=1`, `PF_W=2`) for each `PT_LOAD` segment and derives the mapping flags per segment:
  - Read-only executable code → `PAGE_USER` (no NX, no write)
  - Writable data / BSS / heap → `PAGE_USER | PAGE_WRITABLE | PAGE_NX`
  - Stack → `PAGE_USER | PAGE_WRITABLE | PAGE_NX`
- **`clone_user_page_directory`** preserves bit 63 when copying PTEs (earlier `pte & 0xFFF` mask silently stripped the NX flag during fork/exec; fixed to `(pte & 0xFFF) | (pte & PAGE_NX)`).
- Flat binaries (no ELF `p_flags`) are mapped RWX for backward compatibility.

### Address Space Layout Randomization (ASLR)

Stack and mmap base addresses are randomized at process spawn using hardware entropy:

| Region | Randomization | Entropy |
|--------|--------------|---------|
| **User stack** | ±1 MiB (256 pages) below `USER_STACK_TOP` | RDRAND (TSC fallback if CPU lacks RDRAND) |
| **mmap base** | ±16 MiB (4096 pages) above `0x20000000` | Same |

Implementation notes:
- `random_page_offset(max_pages)` in `loader.rs` checks CPUID `RDRAND` before executing the instruction (executing RDRAND on a CPU without it raises `#UD`). Falls back to a TSC-based xorshift64 on CPUs that report `RDRAND=false`.
- ET_EXEC (non-PIE) binaries cannot have their text base randomized without recompilation as PIE; only stack and mmap are affected.
- The stack ASLR offset is returned via `LoadResult.stack_top` so all code paths (`exec`, `spawn`, flat binary) use the same randomised pointer.

### File Descriptor Namespace

- **MAX_FDS = 256**: each process may have up to 256 simultaneously open file descriptors (FDs 0–255). The per-process `FdTable` is a fixed-size array on the thread struct (3 KiB, no heap allocation).
- **SOCKET_FD_BASE = 256**: socket FDs in libc start at 256 to avoid colliding with file FDs 0–255. This separation prevents ambiguous `read()`/`write()` routing.
- **MAX_OPEN_FILES = 1024**: global VFS open-file table supports up to 1024 simultaneous open file slots across all processes.
- **PATH_MAX = 4096**: `read_user_str` reads up to 4096 bytes when copying path strings from user space.

---

## Graphics & Compositor

### GPU Drivers

anyOS supports seven GPU backends via the `GpuDriver` trait:

| Driver | PCI ID | Features |
|--------|--------|----------|
| **Bochs VGA** | 1234:1111 | VESA VBE, DISPI registers, page flipping (double buffer) |
| **VMware SVGA II** | 15AD:0405 | FIFO command queue, 2D acceleration (rect fill/copy), hardware cursor, **3D via SVGA3D** |
| **VirtualBox VGA** | 80EE:BEEF | VirtualBox guest display adapter |
| **VirtIO GPU** | 1AF4:1050 | VirtIO graphics device, **3D via virgl** (when `VIRTIO_GPU_F_VIRGL` negotiated) |
| **Intel HD/UHD/Iris** | 8086:* (class 03:00) | Firmware-inherited framebuffer (UEFI GOP / VBE), Gen 5+ (HD 3000 through Xe) |
| **AMD Radeon** | 1002:* (class 03:00) | Firmware-inherited framebuffer, Radeon HD 5000+ through RX 7000 (RDNA 3) |
| **NVIDIA GeForce** | 10DE:* (class 03:00/03:02) | Firmware-inherited framebuffer, GeForce 600+ (Kepler) through RTX 4000 (Ada) |

GPU auto-detection happens during PCI enumeration. The compositor uses whichever driver is available, falling back to software-only rendering if no known GPU is found. The Intel, AMD, and NVIDIA drivers inherit the display mode configured by UEFI GOP or VBIOS firmware, providing a working framebuffer at native resolution without full modesetting.

### GPU Driver HAL (Userspace 3D Drivers)

3D graphics acceleration uses loadable userspace `.drv` shared libraries, following the Windows ICD / Linux Mesa DRI model. This keeps complex shader compilation and state tracking in userspace — crashes don't take down the kernel.

```
App (glDrawArrays, glTexImage2D, ...)
    │
libgl.so  (GL API + Software Rasterizer + drv_loader)
    │── gl_init() → SYS_GPU_QUERY_TYPE → "svga3d" / "virgl" / "none"
    │── dl_open("/System/Drivers/gpu/{type}.drv")
    │
svga3d.drv  │  virgl.drv
    │── Translate drv_* calls → device-specific command buffers
    │── Call kernel 3D syscalls (SYS_GPU_3D_SUBMIT, etc.)
    │
Kernel: generic 3D syscalls → GpuDriver trait implementations
```

Each `.drv` exports 21 `extern "C"` functions (lifecycle, resources, shaders, render state, uniforms, drawing, sync). Drivers are located at `/System/Drivers/gpu/` and built from source under `drivers/gpu/`.

**Kernel 3D syscalls:**

| Syscall | Number | Purpose |
|---------|--------|---------|
| `SYS_GPU_QUERY_TYPE` | 517 | Returns GPU driver type ("svga3d", "virgl", "none") |
| `SYS_GPU_3D_SUBMIT` | 512 | Submit command buffer (driver-specific validation) |
| `SYS_GPU_3D_SYNC` | 514 | Wait for GPU completion |
| `SYS_GPU_3D_SURFACE_DMA` | 515 | Upload data to GPU surface |
| `SYS_GPU_3D_SURFACE_DMA_READ` | 516 | Download from GPU surface |

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

### VSync & Frame Pacing

anyOS implements event-driven frame pacing modeled after Windows DWM (`DwmFlush`) and macOS (`CVDisplayLink`). Instead of blind polling, the compositor signals apps when their content has been composited to the display.

**VSync Mechanism:**

VirtIO-GPU 2D has no hardware VSync interrupt. However, `RESOURCE_FLUSH` is a **synchronous** VirtIO command — when the kernel's `execute_sync()` returns, the frame is guaranteed to be on the virtual display. This return is the VSync moment.

```
App                     Compositor Mgmt Thread          Render Thread
 |                              |                            |
 |-- CMD_PRESENT -------------->|                            |
 |                              |-- signal_render() -------->|
 |                              |                            |-- compose()
 |                              |                            |   flush_gpu() ← VSync
 |<----- EVT_FRAME_ACK --------+--(direct from render thread)|   emit ACK
 |                              |                            |
 |-- next frame --------------->|                            |
```

**Key design decisions:**

1. **Direct ACK from render thread**: The render thread emits `EVT_FRAME_ACK` via `evt_chan_emit_to()` immediately after `compose()` + `flush_gpu()` complete. This bypasses the management thread entirely, eliminating a full 16ms polling cycle from the latency path.

2. **Event-driven render thread**: The render thread sleeps until the management thread signals `RENDER_NEEDED`. No blind 60fps loop — when idle, the render thread sleeps 2-16ms adaptively (2ms when recently active, ramping to 16ms after sustained idle).

3. **Adaptive management thread**: The management thread uses adaptive sleep — 2ms after processing events (fast IPC turnaround), ramping to 16ms during idle periods (saves CPU).

4. **Client-side back-pressure**: Apps track a `frame_presented` flag per window. After `present()`, the flag is set. On receiving `EVT_FRAME_ACK`, the flag is cleared. The UI framework skips rendering windows whose previous frame hasn't been composited yet, with a 64ms safety timeout (4 frames) to handle lost ACKs.

**Latency comparison:**

| Path | Before | After |
|------|--------|-------|
| CMD_PRESENT → compositor picks up | 0-16ms | 0-2ms (adaptive sleep) |
| Compose + GPU flush | ~2-5ms | ~2-5ms |
| ACK → app receives | 0-32ms (two polling cycles) | 0-2ms (direct + adaptive) |
| **Total round-trip** | **~18-53ms** | **~4-9ms** |

This is comparable to Windows DWM (~8-16ms per frame).

### Window Management

- **Window = Layer + Content Surface**: Each window has chrome (title bar, buttons) and a client area
- **Hit testing**: Title bar drag, traffic light buttons, resize edges/corners
- **Resize**: Outline shown during drag, actual resize on mouse-up
- **Maximize/Minimize**: State machine (Normal/Maximized/Minimized)

---

## Filesystem

### Supported Filesystems

| Filesystem | Mode | Description |
|------------|------|-------------|
| **exFAT** | Read/Write | Primary filesystem for disk images (4 KiB clusters, long filenames, contiguous allocation) |
| **FAT12/16/32** | Read/Write | FAT family with VFAT long filename (LFN) support, DOS datetime conversion |
| **NTFS** | Read-only | Minimal NTFS driver (MFT parsing, B+ tree index, runlist decoding, fixup arrays) |
| **ISO 9660** | Read-only | CD-ROM/DVD-ROM filesystem (2048-byte blocks, Primary Volume Descriptor at LBA 16) |
| **OverlayFS** | Read/Write | Union mount -- writable RamFS layer over read-only ISO 9660 (whiteout support for deletes) |
| **RamFS** | Read/Write | In-memory inode-based filesystem (upper layer for OverlayFS, volatile) |
| **SMBFS** | Read/Write | SMB2 network filesystem client (dialect 0x0202, TCP port 445, anonymous/guest session) |
| **DevFS** | Virtual | Device filesystem at `/dev` -- maps file ops to kernel device drivers (`/dev/null`, `/dev/zero`, `/dev/console`, etc.) |

### Storage Dispatch

I/O is routed to the active backend, auto-detected at boot:

| Backend | Description |
|---------|-------------|
| **ATA PIO** | 28-bit LBA, sector read/write via I/O ports (legacy IDE) |
| **AHCI DMA** | SATA DMA transfers via MMIO + bounce buffer (ICH9 AHCI) |
| **NVMe** | PCIe NVMe controller (submission/completion queue pairs) |
| **ATAPI** | CD-ROM / ISO 9660 access |
| **LSI SCSI** | LSI MegaRAID SCSI controller |
| **SDHCI** | SD Host Controller (PCI class 08:05, SD/SDHC/SDXC cards, PIO mode, Realtek/O2 Micro/Genesys Logic) |

### Virtual File System (VFS)

- **File descriptors**: Global FD table (1024 slots), per-process open files (256 max)
- **Mount points**: Runtime mount/unmount of additional filesystems
- **Paths**: `/bin/`, `/System/`, `/Libraries/`, `/include/`, `/lib/`, `/home/`
- **Device files**: `/dev/serial`, `/dev/null`, `/dev/random`
- **Symbolic links**: Symlink creation and resolution
- **Block cache**: Sector-level caching for disk I/O
- **Standard FDs**: 0=stdin, 1=stdout (serial), 2=stderr (serial)

---

## Networking

### Protocol Stack

```
+------------------+
|   Applications   |  wget, ftp, ping, dns, dhcp, ssh, httpd, curl
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
|  Ethernet / WiFi |  Data link layer
+------------------+
         |
+--------+---------+
|   NIC Drivers    |  See table below
+------------------+
```

### Network Drivers

anyOS includes 8 NIC drivers, covering Ethernet and WiFi:

| Driver | Type | Devices | Speed |
|--------|------|---------|-------|
| **Intel E1000** | Ethernet | 82540EM (8086:100E), 82545EM (8086:100F) | 1 GbE |
| **Intel IGC** | Ethernet | I225/I226-V/LM, I210, I211, I219 | 1/2.5 GbE |
| **Realtek RTL8168** | Ethernet | RTL8111B+, RTL8101/8102, RTL8169SC | 1 GbE |
| **Realtek RTL8125** | Ethernet | RTL8125B, RTL8125BG | 2.5 GbE |
| **VirtIO Net** | Ethernet | VirtIO modern/transitional (1AF4:1000/1041) | Virtual |
| **Intel iwlwifi** | WiFi | AX200/201/210/211, BE200, AC 9260/9560, 8265 | WiFi 6/6E/7 |
| **Qualcomm Atheros** | WiFi | QCA6174, QCA9377, QCA6390, WCN6855 | WiFi 5/6/6E |
| **Realtek RTL8188EU** | WiFi (USB) | RTL8188EUS, D-Link DWA-131, Edimax EW-7811Un | 802.11n |

### WiFi Stack (`kernel/src/net/wifi.rs`)

Hardware-independent 802.11 management layer:
- **State machine**: Disconnected -> Scanning -> Associating -> Authenticating -> Connected
- **WPA2 (CCMP/AES)**: Full EAPOL 4-way handshake (ANonce/SNonce, PTK derivation, GTK installation)
- **Data encryption**: AES-128-CCM with 48-bit packet number replay counter
- After association, the WiFi interface presents as a standard `NetworkDriver` for the TCP/IP stack

### Interface Configuration

- **Loopback** (`lo`, 127.0.0.1/255.0.0.0): Defined in `/System/etc/network/interfaces`, auto-injected if missing
- **Own-IP loopback**: Packets to the host's own IP are routed via loopback (no ARP)
- **Config file**: `/System/etc/network/interfaces` supports `static`, `dhcp`, and `loopback` methods

### QEMU Networking

- Guest IP: `10.0.2.15` (QEMU user-mode NAT)
- Gateway: `10.0.2.2`
- DNS: `10.0.2.3`
- DHCP auto-configuration at boot

---

## Bluetooth Subsystem

anyOS includes a Bluetooth stack (`kernel/src/drivers/bluetooth/`) with the following layers:

```
+-------------------+
| HID Profile       |  (Keyboard/Mouse over BT)
+--------+----------+
         |
+--------+----------+
|      L2CAP        |  Logical Link Control and Adaptation Protocol
+--------+----------+
         |
+--------+----------+
|     HCI Core      |  Host Controller Interface (commands, events, ACL data)
+--------+----------+
         |
+--------+----------+
| USB Transport     |  HCI over USB bulk/interrupt endpoints
+-------------------+
```

- **HCI**: Command/event processing, connection management, device discovery
- **L2CAP**: Channel multiplexing, signaling (connection request/response, config, disconnect)
- **USB Transport**: HCI packets over USB bulk (ACL data) and interrupt (events) endpoints

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

anyOS includes USB host controller drivers for all USB generations plus class drivers.

### Host Controllers

| Controller | Standard | PCI Class | Features |
|------------|----------|-----------|----------|
| **UHCI** | USB 1.1 | 0x0C03/0x00 | 1.5/12 Mbps, polled I/O |
| **EHCI** | USB 2.0 | 0x0C03/0x20 | 480 Mbps, async/periodic schedules |
| **xHCI** | USB 3.x | 0x0C03/0x30 | 5/10 Gbps, all speeds (SS/HS/FS/LS), slot/endpoint model, command/transfer rings |

### Class Drivers

| Driver | Description |
|--------|-------------|
| **HID** | USB keyboards and mice (interrupt transfers, polling thread) |
| **Mass Storage** | USB storage devices (Bulk-Only Transport, SCSI pass-through) |
| **Hub** | Hub detection, port enumeration, device attach/detach |
| **Audio** | USB audio class devices |
| **CDC-ACM** | USB serial ports (Abstract Control Model, e.g., Arduino/modems) |
| **CDC-ECM** | USB Ethernet adapters (Ethernet Control Model) |
| **Digitizer** | USB digitizer/touchscreen devices (absolute positioning) |

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
- Used by **fontd** for shared font data (all processes map the same font SHM)

### fontd — Font Server (SHM-based)

The **fontd** daemon (`system/daemons/fontd/`) loads TTF font files from `/System/fonts/` into SHM regions on demand. All processes that need font data request it from fontd and map the returned SHM — the font bytes exist exactly once in physical RAM.

**Startup:** Compositor spawns fontd before `libfont_client::init()`, subscribes to the `"fontd"` event channel before the spawn to avoid a race, then waits for `EVT_FONTD_READY` (0x6000).

**Protocol:** Event channel `"fontd"` with `CMD_LOAD_BY_NAME` (filename in SHM) → `EVT_FONT_READY` (data SHM ID + size). See `docs/libfont-api.md` for full protocol.

**Caching:** fontd maintains a 64-slot path→SHM cache. Once loaded, a font's SHM persists for the lifetime of fontd. Repeated requests return the cached SHM ID instantly.

**Lazy loading:** Only sfpro.ttf (5.9 MB) and andale-mono.ttf (108 KB) are loaded at boot. Bold, thin, italic, and emoji (total ~21 MB) are loaded on first use.

---

## User Identity System

anyOS supports multi-user identity with per-process UID/GID.

- **User accounts**: username, password (PBKDF2-HMAC-SHA256 hashed), full name, home directory
- **Groups**: name + GID
- **Authentication**: `sys_authenticate(user, pass)` verifies credentials
- **Identity switching**: `sys_set_identity(uid)` changes the process UID
- **File ownership**: `chmod` and `chown` syscalls for permission management
- User database stored in `/System/users/`

### App Permissions

anyOS enforces a runtime permission system similar to macOS/Android for `.app` bundles:

- **Capability bitmask**: 16 capability bits (bits 0-15) enforced at syscall dispatch
- **Sensitive capabilities** (require user consent): Filesystem, Network, Audio, Display, Device, Process, System, Compositor
- **Auto-granted capabilities** (infrastructure, no prompt): DLL, Thread, SHM, Event, Pipe
- **Restricted capabilities**: Manage_Perms (bit 13), Debug (bit 14), Hypervisor (bit 15)
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

**INT 0x80 (32-bit C/TCC programs, compatibility mode):**

| Register | Purpose |
|----------|---------|
| EAX | Syscall number (in) / return value (out) |
| EBX | Argument 1 |
| ECX | Argument 2 |
| EDX | Argument 3 |
| ESI | Argument 4 |
| EDI | Argument 5 |

**SVC #0 (ARM64 programs):**

| Register | Purpose |
|----------|---------|
| X8 | Syscall number |
| X0-X5 | Arguments |
| X0 | Return value |

### Syscall Categories

There are **232 syscalls** organized by category:

| Category | Count | Examples |
|----------|-------|----------|
| Process Management | 14 | exit, spawn, fork, exec, kill, sleep, sbrk, waitpid, getppid |
| Threading | 3 | thread_create, set_priority, set_critical |
| File I/O | 16 | read, write, open, close, readdir, stat, mkdir, symlink, rename |
| Mount | 3 | mount, umount, list_mounts |
| Memory | 3 | sbrk, mmap, munmap |
| Networking | 24 | ping, dhcp, dns, tcp_*, udp_*, net_poll, net_stats |
| Pipes/IPC | 11 | pipe_create/read/write, evt_chan_*, evt_sys_* |
| POSIX Pipes/FD | 5 | pipe2, dup, dup2, fcntl, pipe_bytes_available |
| Shared Memory | 4 | shm_create, shm_map, shm_unmap, shm_destroy |
| Signals | 2 | sigaction, sigprocmask |
| Window Manager | 13 | win_create, draw_text, blit, present |
| Display/GPU | 8 | set_resolution, set_wallpaper, capture_screen, gpu_vram_size |
| Compositor | 5 | map_framebuffer, gpu_command, input_poll |
| GPU 3D | 5 | gpu_query_type, gpu_3d_submit, gpu_3d_sync, gpu_3d_surface_dma, gpu_3d_surface_dma_read |
| Audio | 2 | audio_write, audio_ctl |
| DLL | 2 | dll_load, set_dll_u32 |
| Device/System | 10 | time, uptime, sysinfo, devlist, random |
| Environment | 3 | setenv, getenv, listenv |
| Keyboard | 3 | kbd_get_layout, kbd_set_layout, kbd_list_layouts |
| User/Identity | 16 | getuid, authenticate, adduser, chpasswd, getppid |
| App Permissions | 5 | perm_check, perm_store, perm_list, perm_delete, perm_pending_info |
| Filesystem ext | 3 | chmod, chown, chdir |
| Capabilities | 1 | get_capabilities |
| Debug | 4 | debug_read_mem, debug_get_mem_map, debug_wait_event, thread_info_ex |
| Hypervisor (VM) | 22 | vm_create, vcpu_create, vcpu_run, vm_set_memory, vm_map_mmio, vcpu_get/set_regs, etc. |

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
| **uisys** | DLIB | `0x04000000` | 80 | Legacy UI components (31 types, deprecated -- use libanyui) |
| **libimage** | DLIB | `0x04100000` | 10 | Image/video decoding (BMP, PNG, JPEG, GIF, ICO, MJV) + scaling + BMP encoding + iconpack rendering |
| **librender** | DLIB | `0x04300000` | 18 | 2D rendering primitives (shapes, gradients, anti-aliasing) |
| **libcompositor** | DLIB | `0x04380000` | 16 | Window management IPC (SHM surfaces, event channels) |
| **libanyui** | .so | `0x04400000` | 178 | anyui UI framework (44 controls, Windows Forms-style, clipboard, theming, tooltips, dialogs, icons) |
| **libfont** | .so | `0x05000000` | 9 | TrueType font rendering (gamma-corrected greyscale + LCD subpixel AA); font data served from fontd via SHM |
| **libini** | .so | -- | 13 | INI/conf file parser (sections, typed values, iteration) |
| **libgl** | .so | -- | -- | OpenGL ES 2.0 3D engine (software rasterizer + userspace GPU driver loading via .drv) |
| **libhttp** | .so | -- | -- | HTTP client/server library |
| **libm** | .so | -- | -- | Hardware-accelerated math (SSE2 + x87 FPU: sin, cos, sqrt, matrix ops) |
| **libjs** | .so | -- | -- | JavaScript engine (ES2023 support) |
| **libwebview** | .so | -- | -- | HTML/CSS/JS web rendering engine |
| **libsvg** | .so | -- | -- | SVG rasterizer |
| **libzip** | .so | -- | -- | ZIP/TAR/GZIP archive handling |
| **libdb** | .so | -- | -- | Key-value database |
| **libphysics** | .so | -- | -- | Physics engine |
| **libcorevm** | .so | -- | -- | CoreVM x86 virtual machine engine (KVM backend on Linux) |
| **libc64** | static | -- | -- | 64-bit C standard library |
| **libcxx** | static | -- | -- | C++20 standard library |
| **libcxxabi** | static | -- | -- | C++ ABI runtime (exception handling, RTTI) |

### uisys.dlib

The main UI system DLIB provides 80 exported functions implementing 31 UI components:
- Inputs: Button, Toggle, Checkbox, Radio, Slider, Stepper, TextField, SearchField, TextArea
- Layout: Sidebar, NavigationBar, Toolbar, TabBar, SegmentedControl, SplitView, ScrollView
- Data: TableView, ContextMenu, Card, GroupBox, Badge, Tag, ProgressBar
- Display: Label, Tooltip, StatusIndicator, ColorWell, ImageView, IconButton, Divider, Alert
- v2 API: GPU acceleration, anti-aliased shapes, shadow/blur effects, font-aware text

See [uisys API](uisys-api.md) for the complete component reference.

### libimage.dlib

Image and video decoding/encoding library. Uses caller-provided memory for most operations; the `iconpack_render_cached` function lazy-loads and caches ico.pak internally.

| Format | Features |
|--------|----------|
| **BMP** | 24-bit RGB, 32-bit ARGB (decode + encode) |
| **PNG** | 8-bit RGB/RGBA/grayscale, DEFLATE, all filter types |
| **JPEG** | Baseline DCT, 4:2:0/4:2:2/4:4:4, LLM fast integer IDCT |
| **GIF** | LZW, transparency, interlacing (first frame) |
| **ICO** | Multi-size selection, BMP-in-ICO (1/4/8/24/32bpp), PNG-in-ICO |
| **MJV** | Motion JPEG Video container (per-frame JPEG decode) |
| **IPAK** | Icon pack v2 (6000+ Tabler Icons, pre-rasterized alpha maps) |

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

---

## Applications & Programs

### GUI Applications (`apps/`) -- 33 apps

| App | Description |
|-----|-------------|
| **anycode** | Code editor (VSCode-like, reference anyui app) |
| **anybench** | Benchmarking tool |
| **anymail** | Email client (IMAP/SMTP, address book, autocomplete) |
| **anyzilla** | FTP client (FileZilla-like, dual-pane, PASV transfers) |
| **button_demo** | Button demo |
| **calc** | Calculator |
| **clipman** | Clipboard history manager |
| **clock** | Clock widget |
| **demo_anyui** | anyui widget demo/showcase |
| **diagnostics** | System diagnostics |
| **diff** | Diff/merge tool (Meld-like, syntax highlighting, themes) |
| **fontviewer** | Font browser |
| **forger** | Application builder |
| **ftp-settings** | FTP server settings |
| **gldemo** | OpenGL ES 2.0 demo |
| **iconview** | Icon viewer |
| **imgview** | Image viewer |
| **installer** | System installer |
| **keyboard** | On-screen keyboard |
| **mdview** | Markdown viewer |
| **minesweeper** | Minesweeper game |
| **notepad** | Simple text editor |
| **notifications** | Notification settings |
| **paint** | Paint application (Canvas-based) |
| **runner** | Application launcher |
| **screenshot** | Screenshot tool |
| **store** | App Store |
| **surf** | Web browser (HTML/CSS/JS via libwebview) |
| **updater** | System updater |
| **videoplayer** | Video player |
| **vmmanager** | VM Manager (create, configure, run VMs via CoreVM) |
| **vnc-settings** | VNC server settings |
| **webmanager** | Web management tool |

### System Services (`system/`) -- 24 daemons

| Service | Description |
|---------|-------------|
| **amid** | Anywhere Management Interface daemon (system info database) |
| **anybout** | About dialog (system version info) |
| **anytrace** | Interactive debugger, profiler, and process inspector |
| **audiomon** | Audio monitoring service |
| **compositor** | Display server / window manager |
| **crashdialog** | Crash report dialog (displays crash info to user) |
| **desktopd** | Desktop daemon (desktop icon management, wallpaper, session) |
| **diskutil** | Disk utility GUI |
| **eventviewer** | Event/log viewer GUI |
| **finder** | File manager/browser |
| **fontd** | Font server (loads TTF into SHM, shared across all processes) |
| **init** | Boot initialization |
| **inputmon** | Input device monitoring |
| **login** | Login manager |
| **netmon** | Network monitoring |
| **notifyd** | Notification daemon (iOS-style banners, top-right, always-on-top) |
| **permdialog** | Permission dialog daemon |
| **sessionhost** | Session host (user session lifecycle management) |
| **settings** | System settings GUI |
| **shell** | Command-line shell (job control, pipes, redirects, variables, here-docs) |
| **taskmanager** | Task manager / activity monitor |
| **terminal** | Terminal emulator (256-color, true-color, alternate screen, mouse tracking, hyperlinks, Unicode) |
| **textmode_console** | Full-screen text console (for `nogui` boot mode) |
| **wifimon** | WiFi tray icon (macOS-style WiFi menu in menu bar) |

### CLI Programs (`bin/`) -- 122 programs

- **Standard tools**: ls, cat, cp, mv, rm, mkdir, grep, find, head, tail, sort, uniq, wc, xargs, ln, seq, rev, strings, base64, hexdump, xxd, echo, yes, true, false, sleep, clear
- **Editors**: nano, vi, nvi, sed, awk
- **Archive**: tar, zip, unzip, gzip
- **Network**: ping, ssh, sshd, scp, wget, ftp, ftpd, dhcp, dns, ifconfig, arp, netstat, httpd, vncd, curl, ntp, ntpd, wifi
- **System**: ps, top, htop, mount, umount, swapon, swapoff, sysinfo, dmesg, neofetch, stat, df, free, fdisk, devlist, reboot, sync, mode, uptime
- **User management**: adduser, deluser, listuser, addgroup, delgroup, listgroups, passwd, su, sudo, whoami
- **Version control**: git
- **Package manager**: ami, apkg
- **Dev tools**: jscript (JavaScript REPL), make
- **VM**: vmd (CoreVM daemon), vmctl (CoreVM CLI controller)
- **Process management**: kill, killall, nice, crond, crontab
- **Service manager**: svc, logd
- **Misc**: cal, banner, jp2a, open, play, pipes, vdagent, crust, ccargo, git, echoserver

---

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
