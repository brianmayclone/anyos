# anyOS CoreVM (libcorevm) API Reference

**CoreVM** is an x86 virtual machine monitor built entirely in Rust (`#![no_std]`) and NASM assembly, running in anyOS userspace. VM creation now requires host hardware virtualization support and selects either **Intel VT-x** or **AMD-V** through a backend-neutral abstraction layer before the monitor starts.

CoreVM can boot real operating systems: load a BIOS ROM, attach a disk image, and watch the guest transition from 16-bit real mode through 32-bit protected mode to 64-bit long mode — exactly as a physical PC does.

**Format:** ELF64 shared object (.so), loaded via `dl_open("/Libraries/libcorevm.so")`
**Exports:** 59
**Client crate:** `libcorevm_client` (uses `dynlink::dl_open` / `dl_sym`)
**ISA:** Intel x86 / IA-32 / x86-64 (AMD64)
**BIOS:** Built-in CoreVM BIOS (64 KB, NASM) or SeaBIOS (256 KB)

---

## Table of Contents

- [Overview](#overview)
  - [Architecture](#architecture)
  - [Components](#components)
- [Getting Started](#getting-started)
  - [Dependencies](#dependencies)
  - [Minimal Example — Boot a BIOS ROM](#minimal-example--boot-a-bios-rom)
  - [Full Example — Boot from Disk](#full-example--boot-from-disk)
- [CPU Emulation](#cpu-emulation)
  - [Execution Modes](#execution-modes)
  - [Instruction Set Coverage](#instruction-set-coverage)
  - [Registers](#registers)
  - [Paging and Memory Management](#paging-and-memory-management)
  - [Interrupt and Exception Handling](#interrupt-and-exception-handling)
  - [x87 FPU and SSE](#x87-fpu-and-sse)
  - [System Instructions](#system-instructions)
- [JIT Compiler](#jit-compiler)
  - [Decode Cache (Phase 1)](#decode-cache-phase-1)
  - [Native Code Compilation (Phase 2)](#native-code-compilation-phase-2)
  - [JIT Statistics](#jit-statistics)
- [Emulated Devices](#emulated-devices)
  - [Standard Devices](#standard-devices)
  - [IDE/ATA Disk Controller](#ideata-disk-controller)
  - [E1000 Network Interface](#e1000-network-interface)
  - [VGA/SVGA Display](#vgasvga-display)
  - [Serial Port (COM1)](#serial-port-com1)
  - [PS/2 Keyboard and Mouse](#ps2-keyboard-and-mouse)
  - [PCI Bus](#pci-bus)
  - [fw_cfg Device](#fw_cfg-device)
  - [Debug Port](#debug-port)
- [BIOS Firmware](#bios-firmware)
  - [CoreVM BIOS](#corevm-bios)
  - [SeaBIOS Support](#seabios-support)
- [Client API (libcorevm_client)](#client-api-libcorevm_client)
  - [Initialization](#initialization)
  - [VmHandle — VM Lifecycle](#vmhandle--vm-lifecycle)
  - [CPU State Access](#cpu-state-access)
  - [Memory Access](#memory-access)
  - [Device Setup](#device-setup)
  - [Input Injection](#input-injection)
  - [Display Access](#display-access)
  - [Serial and Debug I/O](#serial-and-debug-io)
  - [Network I/O](#network-io)
  - [Timer and Interrupt Control](#timer-and-interrupt-control)
  - [Disk Management](#disk-management)
  - [JIT Control](#jit-control)
  - [Error Reporting](#error-reporting)
  - [GPR Index Constants](#gpr-index-constants)
- [C ABI Exports](#c-abi-exports)
- [VM Manager (vmmanager)](#vm-manager-vmmanager)
  - [User Interface](#user-interface)
  - [VM Configuration](#vm-configuration)
  - [Settings Dialog](#settings-dialog)
  - [Create Disk Image](#create-disk-image)
- [VM Daemon (vmd)](#vm-daemon-vmd)
  - [IPC Protocol](#ipc-protocol)
  - [Execution Loop](#execution-loop)
  - [Shared Memory Framebuffer](#shared-memory-framebuffer)
- [Disk Image Support](#disk-image-support)
- [Source Files](#source-files)

---

## Overview

### Architecture

CoreVM uses a **client-server architecture** split across three processes, connected by IPC pipes and shared memory for zero-copy display:

```
┌────────────────────┐          IPC pipes            ┌──────────────────────┐
│  vmmanager (GUI)   │───── vmd_cmd (commands) ─────>│    vmd (daemon)      │
│                    │<──── vmd_status (events) ──────│                      │
│  - Sidebar / Tree  │                                │  - dl_open libcorevm │
│  - Canvas display  │<──── SHM (4 MiB framebuffer) ──│  - fetch/decode/exec │
│  - Settings dialog │                                │  - device emulation  │
│  - Keyboard/mouse  │                                │  - timer/interrupt   │
└────────────────────┘                                └──────────────────────┘
                                                              │
                                                      ┌───────┴───────┐
                                                      │ libcorevm.so  │
                                                      │               │
                                                      │  VmInstance    │
                                                      │  ├── Cpu       │
                                                      │  ├── Memory    │
                                                      │  ├── MMU       │
                                                      │  ├── Devices   │
                                                      │  └── JIT       │
                                                      └───────────────┘
```

### Components

| Component | Path | Description |
|-----------|------|-------------|
| **libcorevm** | `libs/libcorevm/` | VM engine shared library — machine monitor, memory, MMU, devices, JIT, BIOS |
| **libcorevm_client** | `libs/libcorevm_client/` | Typed Rust client wrapper — resolves 59 C ABI exports via `dl_open`/`dl_sym` |
| **vmd** | `bin/vmd/` | VM daemon — owns the execution loop, bridges IPC to/from libcorevm |
| **vmmanager** | `apps/vmmanager/` | GUI application — VM list, live VGA display, settings, disk creation |

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libcorevm_client = { path = "../../libs/libcorevm_client" }
```

### Minimal Example — Boot a BIOS ROM

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcorevm_client::{self as vm, VmHandle, ExitReason};

fn main() {
    // Load the shared library
    vm::init();

    // Create a VM with 16 MiB guest RAM.
    // This now requires Intel VT-x or AMD-V support on the host.
    let vm = VmHandle::new(16).expect("Intel VT-x or AMD-V hardware virtualization is required");

    // Load a BIOS ROM at the reset vector address
    let bios_rom = anyos_std::fs::read("/Libraries/libcorevm/bios/bios.bin").unwrap();
    vm.load_binary(0xF_0000, &bios_rom);

    // Set the CPU to the standard x86 reset vector
    vm.set_rip(0xFFF0);

    // Register standard PC devices (PIC, PIT, PS/2, VGA, serial, CMOS)
    vm.setup_standard_devices();

    // Run until the guest halts
    loop {
        match vm.run(1_000_000) {
            ExitReason::Halted => break,
            ExitReason::InstructionLimit => {
                // Advance the PIT timer
                if vm.pit_tick() {
                    vm.pic_raise_irq(0);
                }
            }
            ExitReason::Exception => {
                if let Some(err) = vm.last_error() {
                    anyos_std::println!("Exception at RIP={:#x}: {}",
                        vm.last_error_rip(), err);
                }
                break;
            }
            _ => break,
        }
    }
}
```

### Full Example — Boot from Disk

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcorevm_client::{self as vm, VmHandle, ExitReason};

fn main() {
    vm::init();

    // Create a VM with 64 MiB RAM
    let vm = VmHandle::new(64).unwrap();

    // Load BIOS
    let bios = anyos_std::fs::read("/Libraries/libcorevm/bios/bios.bin").unwrap();
    vm.load_binary(0xF_0000, &bios);
    vm.set_rip(0xFFF0);

    // Set up all devices
    vm.setup_standard_devices();
    vm.setup_ide();

    // Attach a disk image
    let disk = anyos_std::fs::read("/Users/Shared/disks/freedos.img").unwrap();
    vm.ide_attach_disk(&disk);

    // Enable JIT for faster execution
    vm.jit_enable(true);

    // Execution loop
    loop {
        match vm.run(5_000_000) {
            ExitReason::Halted => break,
            ExitReason::InstructionLimit => {
                // Timer ticks
                for _ in 0..4 {
                    if vm.pit_tick() {
                        vm.pic_raise_irq(0);
                    }
                }
                // Check IDE IRQ
                if vm.ide_irq_raised() {
                    vm.pic_raise_irq(14);
                    vm.ide_clear_irq();
                }
                // Drain serial output
                let output = vm.serial_take_output_vec();
                if !output.is_empty() {
                    let s = core::str::from_utf8(&output).unwrap_or("?");
                    anyos_std::print!("{}", s);
                }
            }
            _ => break,
        }
    }
}
```

---

## CPU Emulation

CoreVM implements a complete x86 CPU interpreter that starts at the standard power-on reset vector (`CS=0xF000, IP=0xFFF0`) in 16-bit real mode and supports the guest transitioning through all three execution modes.

### Execution Modes

| Mode | Bits | Activation | Features |
|------|------|------------|----------|
| **Real Mode** | 16-bit | Power-on default | Segmented addressing, IVT, BIOS services |
| **Protected Mode** | 32-bit | Set CR0.PE=1 | Flat/segmented, GDT/IDT, privilege rings |
| **Long Mode** | 64-bit | Set CR4.PAE + EFER.LME + CR0.PG | 4-level paging, 64-bit registers, SYSCALL/SYSRET |

The CPU mode is determined from CR0/CR4/EFER state, exactly as on real hardware. The `CpuMode` enum (`RealMode`, `ProtectedMode`, `LongMode`) is available via `vm.mode()`.

### Instruction Set Coverage

CoreVM implements the complete x86 primary opcode map (all 256 entries) plus the 0F secondary opcode map, x87 FPU escape opcodes (D8-DF), and SSE instructions.

**Arithmetic:**
ADD, SUB, ADC, SBB, CMP, MUL, IMUL (1/2/3-operand), DIV, IDIV, INC, DEC, NEG

**Logic:**
AND, OR, XOR, NOT, TEST, BT, BTS, BTR, BTC, BSF, BSR

**Shift/Rotate:**
ROL, ROR, RCL, RCR, SHL/SAL, SHR, SAR (imm8 and CL variants)

**Data Movement:**
MOV (all forms including segment, memory-offset, immediate), LEA, MOVZX, MOVSX, MOVSXD, XCHG, CMPXCHG, XADD, BSWAP, CMOVcc, CBW/CWDE/CDQE, CWD/CDQ/CQO, SAHF, LAHF, XLAT

**Stack:**
PUSH/POP (reg/rm/imm/seg/flags), PUSHA/POPA, ENTER, LEAVE

**Control Flow:**
JMP (near/far/indirect), CALL (near/far/indirect), RET (near/far +/- imm), Jcc (8/32-bit), LOOP/LOOPE/LOOPNE, JCXZ/JECXZ/JRCXZ, INT 3, INT imm8, INTO, IRET/IRETD/IRETQ

**String:**
MOVS, CMPS, STOS, LODS, SCAS, INS, OUTS (all with REP/REPE/REPNE prefixes)

**Bit Manipulation:**
SETcc (0F 90-9F), SHLD, SHRD

**System:**
HLT, CLI, STI, CLD, STD, CLC, STC, CMC, NOP, FWAIT, CPUID, RDTSC, RDMSR, WRMSR, MOV CR, MOV DR, LGDT, SGDT, LIDT, SIDT, LLDT, SLDT, LTR, STR, LMSW, SMSW, INVLPG, SWAPGS, SYSCALL, SYSRET, WBINVD, CLTS

**x87 FPU:**
Full D8-DF escape opcode dispatch (FLD, FST, FSTP, FADD, FSUB, FMUL, FDIV, FCOM, FCOMP, etc.)

**SSE:**
0F 10-17, 28-2F, 50-7F, C2-C6, D0-FE (MOVAPS, MOVUPS, ADDPS, SUBPS, MULPS, DIVPS, CMPPS, ANDPS, ORPS, XORPS, etc.)

### Registers

**General-Purpose Registers (16):**

| Index | 64-bit | 32-bit | 16-bit | 8-bit |
|-------|--------|--------|--------|-------|
| 0 | RAX | EAX | AX | AL/AH |
| 1 | RCX | ECX | CX | CL/CH |
| 2 | RDX | EDX | DX | DL/DH |
| 3 | RBX | EBX | BX | BL/BH |
| 4 | RSP | ESP | SP | SPL |
| 5 | RBP | EBP | BP | BPL |
| 6 | RSI | ESI | SI | SIL |
| 7 | RDI | EDI | DI | DIL |
| 8-15 | R8-R15 | R8D-R15D | R8W-R15W | R8B-R15B |

**Segment Registers:** CS, DS, ES, FS, GS, SS — each with cached base, limit, and access rights from the GDT/LDT.

**Control Registers:**

| Register | Bits | Purpose |
|----------|------|---------|
| CR0 | PE, MP, EM, TS, ET, NE, WP, AM, NW, CD, PG | CPU mode, paging, FPU control |
| CR2 | — | Page fault linear address |
| CR3 | — | Page directory base register |
| CR4 | VME, PVI, TSD, DE, PSE, PAE, MCE, PGE, OSFXSR, OSXMMEXCPT, PCIDE | Feature enables |
| CR8 | — | Task priority register |

**Debug Registers:** DR0-DR3 (breakpoint addresses), DR6 (status), DR7 (control).

**Model-Specific Registers (MSRs):**
EFER (SCE, LME, LMA, NXE), STAR, LSTAR, CSTAR, SFMASK, FS.base, GS.base, KernelGSBase, TSC.

**RFLAGS:** CF, PF, AF, ZF, SF, OF, DF, IF, TF, AC, and all other standard flags.

### Paging and Memory Management

CoreVM supports all three x86 paging modes:

| Mode | Levels | Page Sizes | Activation |
|------|--------|------------|------------|
| **32-bit** | 2 (PD → PT) | 4 KB, 4 MB (PSE) | CR0.PG=1, CR4.PAE=0 |
| **PAE** | 3 (PDPT → PD → PT) | 4 KB, 2 MB | CR0.PG=1, CR4.PAE=1 |
| **4-Level (Long)** | 4 (PML4 → PDPT → PD → PT) | 4 KB, 2 MB, 1 GB | CR0.PG=1, CR4.PAE=1, EFER.LME=1 |

Page table walks enforce:
- NX (no-execute) bit checking via EFER.NXE
- Write-protect (CR0.WP) enforcement
- User/supervisor access control
- A20 gate masking

Segment translation is fully implemented for real mode and protected mode, including limit checking, type checking, and GDT/LDT descriptor caching.

### Interrupt and Exception Handling

| Mode | Vector Table | Format |
|------|-------------|--------|
| Real Mode | IVT at physical 0x0000 | 4-byte entries (segment:offset) |
| Protected Mode | IDT (via LIDT) | 8-byte gate descriptors (32-bit) |
| Long Mode | IDT (via LIDT) | 16-byte gate descriptors (64-bit, IST) |

Double-fault detection prevents infinite exception loops. Hardware exceptions are mapped to `VmError` enum values and result in `ExitReason::Exception` if unhandled.

### x87 FPU and SSE

**x87 FPU State:**
- 8 stack registers (ST0-ST7) as 64-bit `f64` values
- Control word (FCW), status word (FSW), tag word (FTW)
- Instruction pointer (FIP), data pointer (FDP)

**SSE State:**
- 16 registers (XMM0-XMM15) as 128-bit values
- MXCSR control/status register

### System Instructions

CoreVM implements the full complement of privileged system instructions needed to boot an operating system:

- **Descriptor tables:** LGDT, SGDT, LIDT, SIDT, LLDT, SLDT, LTR, STR
- **Control registers:** MOV CR0/CR2/CR3/CR4/CR8, LMSW, SMSW, CLTS
- **Paging:** INVLPG
- **Mode switching:** SYSCALL, SYSRET, SWAPGS
- **Identification:** CPUID, RDTSC, RDMSR, WRMSR
- **Cache:** WBINVD

---

## JIT Compiler

CoreVM includes a two-phase JIT (Just-In-Time) compilation engine that accelerates guest execution.

### Decode Cache (Phase 1)

The decode cache eliminates redundant instruction decoding for hot code paths:

- Basic blocks are identified by `(physical_address, cpu_mode, CS.base)` keys
- Pre-decoded instruction sequences are stored in an LRU/hash cache
- On cache hit, the interpreter skips the decode stage entirely
- Typically delivers a 2-3x speedup for tight loops

### Native Code Compilation (Phase 2)

Hot basic blocks are compiled to native x86-64 machine code:

- **Tier 1 instructions** (compiled natively): MOV, ADD, SUB, CMP, JMP, Jcc, TEST, INC, DEC, NOP
- All other instructions fall back to the interpreter via C ABI helper calls
- W^X memory management: code is emitted into writable pages, then made executable via `mprotect`
- JIT buffer is flushed automatically on CPU mode switches or CR3 changes

### JIT Statistics

The client API provides two statistics queries:

```rust
// Decode cache stats
let (cached_blocks, cache_hits, cache_misses) = vm.jit_cache_stats();

// Native compilation stats
let (blocks_compiled, native_insns, fallback_insns, code_buf_used) = vm.jit_stats();
```

---

## Emulated Devices

### Standard Devices

Calling `vm.setup_standard_devices()` registers the full complement of PC-compatible hardware:

| Device | I/O Ports | IRQ | Description |
|--------|-----------|-----|-------------|
| **8259A PIC** (dual) | 0x20-0x21, 0xA0-0xA1 | — | Programmable Interrupt Controller (master + slave, 16 IRQ lines) |
| **8254 PIT** | 0x40-0x43 | 0 | Programmable Interval Timer (3 channels) |
| **PS/2 Controller** | 0x60, 0x64 | 1 (kbd), 12 (mouse) | Keyboard scan codes + mouse packets |
| **CMOS RTC** | 0x70-0x71 | 8 | Real-time clock + 128 bytes NVRAM |
| **16550 UART** | 0x3F8-0x3FF | 4 | Serial port (COM1) with TX/RX ring buffers |
| **VGA/SVGA** | 0x3C0-0x3DA, MMIO 0xA0000 | — | Text mode (80x25), Mode 13h (320x200x256), linear framebuffer |
| **Debug Port** | 0x402 | — | QEMU-style debug console output |

### IDE/ATA Disk Controller

```rust
vm.setup_ide();                             // Register the controller
vm.ide_attach_disk(&disk_image_bytes);      // Attach raw disk image
```

| Feature | Details |
|---------|---------|
| **I/O Ports** | Primary: 0x1F0-0x1F7 (command), 0x3F6-0x3F7 (control) |
| **IRQ** | 14 |
| **Transfer Mode** | PIO (programmed I/O) |
| **LBA** | 28-bit (up to 128 GiB), 48-bit (up to 128 PiB) |
| **ATA Commands** | IDENTIFY DEVICE (0xEC), READ SECTORS (0x20), WRITE SECTORS (0x30), READ/WRITE SECTORS EXT (0x24/0x34), READ/WRITE MULTIPLE (0xC4/0xC5), SET MULTIPLE (0xC6), SET FEATURES (0xEF), FLUSH CACHE (0xE7), DEVICE RESET (0x08) |

### E1000 Network Interface

```rust
vm.setup_pci_bus();                          // PCI bus required first
vm.setup_e1000(0xD000_0000, &mac_address);   // MMIO base + 6-byte MAC
```

| Feature | Details |
|---------|---------|
| **Type** | Intel E1000 (MMIO-mapped) |
| **MMIO Region** | 128 KB at configurable base (default `0xD000_0000`) |
| **PCI** | Appears on the PCI configuration bus |
| **RX** | `vm.e1000_receive_packet(&ethernet_frame)` — inject a packet into the guest |
| **TX** | `vm.e1000_take_tx_packets(&mut buf)` — drain transmitted packets |

### VGA/SVGA Display

The VGA adapter supports multiple display modes:

| Mode | Resolution | Colors | Access |
|------|-----------|--------|--------|
| Text mode | 80x25 characters | 16 fg + 16 bg | `vm.vga_text_buffer()` → `&[u16]` (attribute:char cells) |
| Mode 13h | 320x200 | 256 (palette) | `vm.vga_framebuffer()` → 8bpp pixel data |
| 640x480 | 640x480 | 16 | `vm.vga_framebuffer()` |
| Linear FB | Arbitrary | 24/32bpp | `vm.vga_framebuffer()` → `(pixels, width, height, bpp)` |

### Serial Port (COM1)

```rust
// Send input to the guest
vm.serial_send_input(b"Hello\n");

// Read guest serial output
let output = vm.serial_take_output_vec();
```

The 16550 UART emulates TX/RX ring buffers at COM1 (0x3F8) with DLAB support.

### PS/2 Keyboard and Mouse

```rust
// Key press + release (scancode set 1)
vm.ps2_key_press(0x1C);   // Enter key down
vm.ps2_key_release(0x1C); // Enter key up

// Mouse movement (dx, dy, buttons)
vm.ps2_mouse_move(10, -5, 0x01); // move right+up, left button down
```

### PCI Bus

```rust
vm.setup_pci_bus();  // PCI configuration space at ports 0xCF8/0xCFC
```

The PCI bus provides standard configuration space access for attached PCI devices (currently the E1000 NIC).

### fw_cfg Device

The fw_cfg device implements the QEMU firmware configuration interface, allowing the host to inject named files that SeaBIOS can discover:

```rust
let vgabios = anyos_std::fs::read("/System/shared/corevm/bios/vgabios.bin").unwrap();
vm.fw_cfg_add_file("vgaroms/vgabios.bin", &vgabios);
```

### Debug Port

Port 0x402 captures debug output from the guest (used by SeaBIOS during POST):

```rust
let debug_output = vm.debug_take_output_vec();
```

---

## BIOS Firmware

### CoreVM BIOS

The built-in BIOS is a 64 KB NASM-assembled firmware at `libs/libcorevm/bios/`. It provides:

| Interrupt | Service | Description |
|-----------|---------|-------------|
| INT 10h | Video | Text mode output, cursor positioning, video mode switching |
| INT 13h | Disk | Read/write sectors via ATA PIO |
| INT 15h | Memory | E820 memory map, extended memory size |
| INT 16h | Keyboard | Key input, shift state |
| INT 19h | Boot | Boot sector loading, El Torito CD boot |
| INT 1Ah | Time | RTC read, tick count |

**BIOS source files:**

| File | Purpose |
|------|---------|
| `bios.asm` | Top-level include, reset vector at 0xFFF0, 64 KB padding |
| `post.asm` | Power-On Self-Test entry point |
| `ivt.asm` | Interrupt Vector Table setup |
| `int10h.asm` | Video services |
| `int13h.asm` | Disk services |
| `int15h.asm` | Memory services (E820) |
| `int16h.asm` | Keyboard services |
| `int19h.asm` | Boot (disk + El Torito CD) |
| `int1ah.asm` | RTC/time services |
| `bda.asm` | BIOS Data Area |
| `e820.asm` | Memory map tables |
| `ide.asm` | ATA PIO helper |
| `pic_pit.asm` | PIC/PIT initialization |
| `serial.asm` | Serial port init |
| `video_init.asm` | VGA mode initialization |
| `pci.asm` | PCI enumeration |
| `boot.asm` | Boot sector loading |

The BIOS is loaded at guest physical address `0xF0000` (64 KB below the 1 MB boundary), with the CPU starting execution at `0xFFF0` (the x86 reset vector).

### SeaBIOS Support

CoreVM can alternatively use SeaBIOS, an open-source BIOS implementation:

- SeaBIOS binary (256 KB) loaded at `0xC0000` via `load_rom()` for the high-address alias
- VGA BIOS injected via fw_cfg as `vgaroms/vgabios.bin`
- Debug output captured via port 0x402

---

## Client API (libcorevm_client)

### Initialization

```rust
use libcorevm_client::{self as vm, VmHandle, ExitReason, CpuMode};

// Load libcorevm.so — must be called once before any other function
let ok = vm::init();  // returns true on success
```

### VmHandle — VM Lifecycle

```rust
// Create a VM with N MiB of guest RAM (returns None on allocation failure)
let vm = VmHandle::new(ram_size_mb: u32) -> Option<VmHandle>;

// Reset to power-on state (preserves RAM and I/O handlers)
vm.reset();

// Execute up to max_instructions guest instructions (0 = unlimited)
let reason: ExitReason = vm.run(max_instructions: u64);

// Request stop at next instruction boundary (thread-safe)
vm.request_stop();

// VmHandle implements Drop — VM is destroyed automatically
```

**ExitReason enum:**

| Variant | Value | Meaning |
|---------|-------|---------|
| `Halted` | 0 | Guest executed HLT |
| `Exception` | 1 | Unrecoverable CPU exception |
| `InstructionLimit` | 2 | `max_instructions` reached |
| `Breakpoint` | 3 | INT 3 hit |
| `StopRequested` | 4 | External stop via `request_stop()` |

### CPU State Access

```rust
// Instruction pointer
let rip: u64 = vm.rip();
vm.set_rip(0x7C00);

// General-purpose registers (index 0=RAX .. 15=R15)
let rax: u64 = vm.gpr(0);
vm.set_gpr(0, 0xDEAD_BEEF);

// RFLAGS
let flags: u64 = vm.rflags();
vm.set_rflags(0x202); // IF=1

// Control registers (valid: 0, 2, 3, 4, 8)
let cr0: u64 = vm.cr(0);
vm.set_cr(0, cr0 | 1); // set PE bit

// CPU mode and privilege level
let mode: CpuMode = vm.mode();       // RealMode / ProtectedMode / LongMode
let ring: u8 = vm.cpl();             // 0 (kernel) .. 3 (user)

// Instruction count since last reset
let count: u64 = vm.instruction_count();
```

### Memory Access

```rust
// Load binary data at a guest physical address (bypasses MMU)
let ok: bool = vm.load_binary(addr: u64, data: &[u8]);

// Map read-only ROM (writes silently ignored)
let ok: bool = vm.load_rom(addr: u64, data: &[u8]);

// Direct physical memory read/write
let byte: u8 = vm.read_phys_u8(addr);
let word: u16 = vm.read_phys_u16(addr);
let dword: u32 = vm.read_phys_u32(addr);
vm.write_phys_u8(addr, val);
vm.write_phys_u16(addr, val);
vm.write_phys_u32(addr, val);
```

### Device Setup

```rust
// Register all standard PC devices (PIC, PIT, PS/2, CMOS, UART, VGA)
vm.setup_standard_devices();

// Register PCI configuration bus (required before setup_e1000)
vm.setup_pci_bus();

// Register Intel E1000 NIC
vm.setup_e1000(mmio_base: u64, mac: &[u8; 6]);

// Register IDE controller on primary channel
vm.setup_ide();
```

### Input Injection

```rust
// Keyboard (PS/2 scancodes)
vm.ps2_key_press(scancode: u8);
vm.ps2_key_release(scancode: u8);

// Mouse (relative movement + button state)
vm.ps2_mouse_move(dx: i16, dy: i16, buttons: u8);
```

### Display Access

```rust
// Graphics mode: returns (pixel_data, width, height, bpp) or None
let fb: Option<(&[u8], u32, u32, u8)> = vm.vga_framebuffer();

// Text mode: returns 80x25 u16 cells (attr<<8 | char) or None
let text: Option<&[u16]> = vm.vga_text_buffer();

// Debug counters
let (total_writes, text_writes): (u64, u64) = vm.vga_debug_counters();
```

### Serial and Debug I/O

```rust
// Serial port (COM1)
vm.serial_send_input(data: &[u8]);
let n: usize = vm.serial_take_output(buf: &mut [u8]);
let output: Vec<u8> = vm.serial_take_output_vec();

// Debug port (0x402)
let n: usize = vm.debug_take_output(buf: &mut [u8]);
let output: Vec<u8> = vm.debug_take_output_vec();
```

### Network I/O

```rust
// Inject a packet into the guest E1000 NIC
vm.e1000_receive_packet(data: &[u8]);

// Drain transmitted packets
let n: usize = vm.e1000_take_tx_packets(buf: &mut [u8]);
```

### Timer and Interrupt Control

```rust
// Advance PIT by one tick (returns true if channel 0 fired)
let fired: bool = vm.pit_tick();

// Raise an IRQ on the PIC (0-15)
vm.pic_raise_irq(irq: u8);

// Get next pending interrupt vector (None if all masked)
let vector: Option<u8> = vm.pic_get_interrupt();
```

### Disk Management

```rust
// Attach/detach raw disk image
vm.ide_attach_disk(data: &[u8]);
vm.ide_detach_disk();

// Check/clear IDE IRQ (IRQ 14)
let pending: bool = vm.ide_irq_raised();
vm.ide_clear_irq();
```

### JIT Control

```rust
// Enable/disable JIT compilation
vm.jit_enable(true);

// Flush all caches (after loading new guest code)
vm.jit_flush_cache();

// Decode cache statistics
let (cached_blocks, hits, misses): (u32, u64, u64) = vm.jit_cache_stats();

// JIT compilation statistics
let (compiled, native, fallback, buf_used): (u64, u64, u64, u32) = vm.jit_stats();
```

### Error Reporting

```rust
// Last error message (None if no error since reset)
let err: Option<String> = vm.last_error();

// RIP at time of last error (0 if no error)
let rip: u64 = vm.last_error_rip();

// MMIO diagnostic info
let (region_count, min_base, max_end, ram_at_b8000): (u32, u64, u64, u32) = vm.mmio_diag();

// fw_cfg file injection
let result: i32 = vm.fw_cfg_add_file(name: &str, data: &[u8]);
```

### GPR Index Constants

Convenience constants for register access:

```rust
use libcorevm_client::*;

let rax = vm.gpr(GPR_RAX);  // 0
let rcx = vm.gpr(GPR_RCX);  // 1
let rdx = vm.gpr(GPR_RDX);  // 2
let rbx = vm.gpr(GPR_RBX);  // 3
let rsp = vm.gpr(GPR_RSP);  // 4
let rbp = vm.gpr(GPR_RBP);  // 5
let rsi = vm.gpr(GPR_RSI);  // 6
let rdi = vm.gpr(GPR_RDI);  // 7
// GPR_R8 (8) through GPR_R15 (15)
```

---

## C ABI Exports

All 59 functions exported by `libcorevm.so` (listed in `libs/libcorevm/exports.def`):

### VM Lifecycle (6)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_create` | `(ram_mb: u32) -> u64` | Create VM, returns opaque handle (0 on failure) |
| `corevm_host_virtualization_backend` | `() -> u32` | Detect host backend (0 = unavailable, 1 = Intel VT-x, 2 = AMD-V) |
| `corevm_destroy` | `(h: u64)` | Destroy VM and free all resources |
| `corevm_reset` | `(h: u64)` | Reset CPU/MMU to power-on state |
| `corevm_run` | `(h: u64, max: u64) -> u32` | Execute instructions, returns ExitReason |
| `corevm_request_stop` | `(h: u64)` | Request stop at next instruction boundary |

### CPU State (11)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_get_rip` | `(h: u64) -> u64` | Read instruction pointer |
| `corevm_set_rip` | `(h: u64, v: u64)` | Write instruction pointer |
| `corevm_get_gpr` | `(h: u64, idx: u8) -> u64` | Read GPR by index (0-15) |
| `corevm_set_gpr` | `(h: u64, idx: u8, v: u64)` | Write GPR by index |
| `corevm_get_rflags` | `(h: u64) -> u64` | Read RFLAGS |
| `corevm_set_rflags` | `(h: u64, v: u64)` | Write RFLAGS |
| `corevm_get_cr` | `(h: u64, n: u8) -> u64` | Read control register (0,2,3,4,8) |
| `corevm_set_cr` | `(h: u64, n: u8, v: u64)` | Write control register |
| `corevm_get_segment_selector` | `(h: u64, seg: u8) -> u16` | Read segment selector |
| `corevm_get_segment_base` | `(h: u64, seg: u8) -> u64` | Read segment base address |
| `corevm_get_mode` | `(h: u64) -> u32` | Get CPU mode (0=real, 1=prot, 2=long) |

### CPU Info (2)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_get_cpl` | `(h: u64) -> u8` | Get current privilege level (0-3) |
| `corevm_get_instruction_count` | `(h: u64) -> u64` | Total instructions since reset |

### Memory (8)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_load_binary` | `(h: u64, addr: u64, ptr: *const u8, len: u32) -> u32` | Load data at physical address |
| `corevm_load_rom` | `(h: u64, addr: u64, ptr: *const u8, len: u32) -> i32` | Map read-only ROM |
| `corevm_read_phys_u8` | `(h: u64, addr: u64) -> u8` | Read byte from physical memory |
| `corevm_read_phys_u16` | `(h: u64, addr: u64) -> u16` | Read u16 from physical memory |
| `corevm_read_phys_u32` | `(h: u64, addr: u64) -> u32` | Read u32 from physical memory |
| `corevm_write_phys_u8` | `(h: u64, addr: u64, val: u8)` | Write byte to physical memory |
| `corevm_write_phys_u16` | `(h: u64, addr: u64, val: u16)` | Write u16 to physical memory |
| `corevm_write_phys_u32` | `(h: u64, addr: u64, val: u32)` | Write u32 to physical memory |

### Device Setup (4)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_setup_standard_devices` | `(h: u64)` | Register PIC, PIT, PS/2, CMOS, UART, VGA |
| `corevm_setup_pci_bus` | `(h: u64)` | Register PCI configuration bus |
| `corevm_setup_e1000` | `(h: u64, mmio: u64, mac: *const u8)` | Register E1000 NIC |
| `corevm_setup_ide` | `(h: u64)` | Register IDE controller |

### PS/2 Input (3)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_ps2_key_press` | `(h: u64, sc: u8)` | Inject key press |
| `corevm_ps2_key_release` | `(h: u64, sc: u8)` | Inject key release |
| `corevm_ps2_mouse_move` | `(h: u64, dx: i16, dy: i16, btn: u8)` | Inject mouse packet |

### VGA Display (3)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_vga_get_framebuffer` | `(h: u64, w: *mut u32, h: *mut u32, bpp: *mut u8) -> *const u8` | Get framebuffer pointer |
| `corevm_vga_get_text_buffer` | `(h: u64, count: *mut u32) -> *const u16` | Get text mode buffer |
| `corevm_vga_debug_counters` | `(h: u64, total: *mut u64, text: *mut u64)` | MMIO write counters |

### Serial Port (2)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_serial_send_input` | `(h: u64, ptr: *const u8, len: u32)` | Send data to guest COM1 |
| `corevm_serial_take_output` | `(h: u64, buf: *mut u8, len: u32) -> u32` | Read guest COM1 output |

### E1000 Network (2)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_e1000_receive_packet` | `(h: u64, ptr: *const u8, len: u32)` | Deliver packet to guest NIC |
| `corevm_e1000_take_tx_packets` | `(h: u64, buf: *mut u8, len: u32) -> u32` | Drain TX packets |

### Timer / Interrupt (3)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_pit_tick` | `(h: u64) -> u32` | Advance PIT, returns 1 if IRQ 0 fired |
| `corevm_pic_raise_irq` | `(h: u64, irq: u8)` | Assert IRQ line on PIC |
| `corevm_pic_get_interrupt` | `(h: u64) -> u32` | Get pending vector (0xFFFF = none) |

### IDE Disk (4)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_ide_attach_disk` | `(h: u64, ptr: *const u8, len: u32)` | Attach raw disk image |
| `corevm_ide_detach_disk` | `(h: u64)` | Detach disk image |
| `corevm_ide_irq_raised` | `(h: u64) -> u32` | Check pending IDE IRQ |
| `corevm_ide_clear_irq` | `(h: u64)` | Clear IDE IRQ |

### fw_cfg (1)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_fw_cfg_add_file` | `(h: u64, name: *const u8, data: *const u8, len: u32) -> i32` | Add named file |

### Debug (1)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_debug_take_output` | `(h: u64, buf: *mut u8, len: u32) -> u32` | Read debug port output |

### JIT (4)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_jit_enable` | `(h: u64, on: u32)` | Enable (1) or disable (0) JIT |
| `corevm_jit_flush_cache` | `(h: u64)` | Flush decode + JIT caches |
| `corevm_jit_cache_stats` | `(h: u64, blocks: *mut u32, hits: *mut u64, misses: *mut u64)` | Decode cache stats |
| `corevm_jit_stats` | `(h: u64, compiled: *mut u64, native: *mut u64, fallback: *mut u64, buf: *mut u32)` | JIT stats |

### Diagnostics (3)

| Export | Signature | Description |
|--------|-----------|-------------|
| `corevm_mmio_diag` | `(h: u64, count: *mut u32, lo: *mut u64, hi: *mut u64, ram: *mut u32)` | MMIO region info |
| `corevm_get_last_error` | `(h: u64, buf: *mut u8, len: u32) -> u32` | Last error message |
| `corevm_get_last_error_rip` | `(h: u64) -> u64` | RIP at last error |

---

## VM Manager (vmmanager)

The VM Manager is a GUI application (`apps/vmmanager/`) providing a graphical interface for creating, configuring, and running virtual machines.

### User Interface

- **Window:** 900x640 pixels with a 200-pixel sidebar
- **Sidebar:** TreeView listing VMs organized in named folders; status dots (green = running, grey = stopped)
- **Content Area:** Canvas displaying the live VGA framebuffer with real-time updates via SHM polling
- **Info Bar:** CPU mode, instruction count, RAM size
- **Toolbar:** New VM, Start, Stop, Settings, Create Disk Image

### VM Configuration

Each VM has a UUID (32 hex chars) and its configuration stored as a key-value text file:

**Path:** `/System/shared/vmmanager/vms/<uuid>.conf`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | (required) | Human-readable VM name |
| `ram` | integer (MB) | 64 | Guest RAM size (1-1024 MB) |
| `ram_alloc` | `prealloc` / `ondemand` | `ondemand` | RAM allocation strategy |
| `disk` | path | — | Path to raw disk image |
| `iso` | path | — | Path to ISO image (El Torito bootable) |
| `bios` | `corevm` / `seabios` | `corevm` | BIOS firmware selection |
| `jit` | `0` / `1` | `0` | Enable JIT acceleration |
| `net_enabled` | `0` / `1` | `0` | Enable E1000 NIC |
| `net_mode` | `nat` / `bridge` | `nat` | Network mode |
| `net_host_nic` | string | — | Host NIC name (bridged mode) |
| `mac_mode` | `dynamic` / `static` | `dynamic` | MAC address assignment |
| `mac_address` | `XX:XX:XX:XX:XX:XX` | auto-generated | Static MAC address |

**Layout file:** `/System/shared/vmmanager/layout.conf` stores sidebar folder structure and VM ordering.

### Settings Dialog

Three tabs of configuration:

- **General:** VM name, RAM (slider 1-1024 MB), RAM allocation mode, BIOS selection, JIT toggle
- **Devices:** GPU (SVGA FB), network enable/disable, network mode, host NIC, MAC mode, MAC address
- **Boot:** Boot order (Disk / CD / Floppy), disk image path, ISO image path

### Create Disk Image

A dialog for creating raw sparse disk images:
- Path selection (TextField)
- Size in MB (TextField)
- Creates a zero-filled raw disk image at the specified path

---

## VM Daemon (vmd)

The VM daemon (`bin/vmd/`) is a headless process that owns the VM execution loop and bridges IPC commands to the libcorevm engine.

### IPC Protocol

Communication happens over two named pipes:

- **`vmd_cmd`** (vmmanager → vmd): Text-based commands, one per line
- **`vmd_status`** (vmd → vmmanager): Status messages and serial output

**Commands:**

| Command | Description |
|---------|-------------|
| `create <uuid>` | Read VM config, create VM instance, allocate SHM; responds with `created 0 <shm_id>` |
| `start` | Load BIOS, attach disk/ISO, begin execution |
| `stop` | Stop VM execution |
| `destroy` | Tear down SHM, destroy VM instance |
| `key <scancode>` | Inject PS/2 key press + release |
| `mouse <dx> <dy> <buttons>` | Inject mouse movement |
| `quit` | Exit the vmd process |

**Status messages:**

| Message | Description |
|---------|-------------|
| `ready` | vmd is initialized and waiting for commands |
| `created 0 <shm_id>` | VM created, SHM ID for framebuffer |
| `started` | VM execution has begun |
| `stopped` | VM execution has stopped |
| `serial 0 <text>` | Serial output from the guest |

### Execution Loop

```
loop:
  1. Drain all pending IPC commands
  2. If VM is running:
     a. Advance PIT by 4 ticks, raise IRQ 0 if fired
     b. Execute 5,000,000 guest instructions
     c. Handle ExitReason (Halted/InstructionLimit/Exception/StopRequested)
     d. Drain serial output → forward to vmmanager
     e. Drain debug port output → local stdout
     f. Update SHM framebuffer from VGA state
  3. Sleep 1 ms (running) or 10 ms (idle)
```

### Shared Memory Framebuffer

A 4 MiB shared memory region provides zero-copy display access between vmd and vmmanager:

**Header (64 bytes):**

| Offset | Size | Type | Field |
|--------|------|------|-------|
| 0 | 4 | u32 | width (columns in text mode, pixels in graphics) |
| 4 | 4 | u32 | height (rows in text mode, pixels in graphics) |
| 8 | 4 | u8 | bpp (0 = text mode, 8/24/32 = graphics) |
| 12 | 4 | u32 | dirty flag (1 = framebuffer updated since last read) |
| 16 | 4 | u32 | vm_state (0=stopped, 1=running, 2=halted, 3=error) |
| 20 | 4 | u32 | instruction_count (low 32 bits) |
| 24 | 4 | u32 | instruction_count (high 32 bits) |
| 28 | 36 | — | reserved |

**Payload (offset 64):**
- Text mode: `80 * 25 * 2 = 4000` bytes (u16 cells: `attribute << 8 | character`)
- Graphics mode: `width * height * (bpp/8)` bytes (raw pixels)

---

## Disk Image Support

CoreVM supports **raw disk images** — flat byte arrays with no container format:

| Format | Extension | Description |
|--------|-----------|-------------|
| Raw disk image | `.img` | Flat binary, sector-addressable (512-byte sectors) |
| ISO image | `.iso` | Attached as second IDE device, El Torito CD boot via BIOS INT 19h |

The disk image is loaded entirely into memory inside the IDE controller. The "Create Disk Image" dialog in vmmanager creates zero-filled sparse files at a user-specified size.

**Addressing:**
- 28-bit LBA: up to 128 GiB per disk
- 48-bit LBA (EXT commands): up to 128 PiB per disk

---

## Source Files

### libcorevm (`libs/libcorevm/src/`)

| File | Description |
|------|-------------|
| `lib.rs` | Module root, `VmEngine` wrapper, `VmInstance` C ABI layer, all `extern "C"` exports |
| `cpu.rs` | `Cpu` struct: fetch-decode-execute loop, decode cache + JIT integration |
| `registers.rs` | `RegisterFile` (`#[repr(C)]`): GPRs, segments, CRs, DRs, MSRs |
| `instruction.rs` | `DecodedInst`: decoded instruction representation |
| `decoder.rs` | Variable-length x86 instruction decoder (16/32/64-bit) |
| `flags.rs` | RFLAGS bit constants, `OperandSize` enum |
| `error.rs` | `VmError` enum (x86 hardware exceptions) |
| `interrupts.rs` | `InterruptController`: IVT/IDT, interrupt delivery, PIC interface |
| `io.rs` | `IoDispatch` + `IoHandler` trait: port I/O routing |
| `fpu_state.rs` | x87 FPU state (ST0-ST7, FCW, FSW, FTW) |
| `sse_state.rs` | SSE state (XMM0-XMM15, MXCSR) |

### Executor (`libs/libcorevm/src/executor/`)

| File | Description |
|------|-------------|
| `mod.rs` | Dispatch tables, shared helpers (`read_operand`, `write_operand`, `compute_effective_address`) |
| `arith.rs` | ADD, SUB, ADC, SBB, CMP, MUL, IMUL, DIV, IDIV, INC, DEC, NEG |
| `logic.rs` | AND, OR, XOR, NOT, TEST, BT/BTS/BTR/BTC, BSF, BSR, shifts, rotates |
| `data.rs` | MOV, MOVZX, MOVSX, MOVSXD, XCHG, CMPXCHG, XADD, BSWAP, LEA, CMOVcc |
| `control.rs` | JMP, CALL, RET, Jcc, IRET, INT, LOOP, JCXZ |
| `stack.rs` | PUSH, POP, PUSHA, POPA, ENTER, LEAVE |
| `string.rs` | MOVS, CMPS, STOS, LODS, SCAS, INS, OUTS (with REP) |
| `system.rs` | HLT, CPUID, RDTSC, RDMSR, WRMSR, MOV CR/DR, LGDT, LIDT, SYSCALL, SYSRET |
| `fpu.rs` | x87 FPU dispatch (D8-DF escape opcodes) |
| `sse.rs` | SSE instruction dispatch |
| `setcc.rs` | SETcc instructions (0F 90-9F) |

### Memory (`libs/libcorevm/src/memory/`)

| File | Description |
|------|-------------|
| `mod.rs` | `GuestMemory`, `Mmu`, `MemoryBus` trait, `AccessType` |
| `flat.rs` | `FlatMemory`: `Vec<u8>` backing store |
| `mmio.rs` | `MmioDispatch` + `MmioHandler` trait |
| `paging.rs` | Page table walker (32-bit, PAE, 4-level long mode) |
| `segment.rs` | Segment translation and descriptor validation |

### JIT (`libs/libcorevm/src/jit/`)

| File | Description |
|------|-------------|
| `mod.rs` | `JitEngine`: orchestration |
| `block.rs` | `BasicBlock` + `BlockKey`: basic block detection |
| `cache.rs` | `DecodeCache`: LRU/hash lookup |
| `emitter.rs` | x86-64 machine code assembler |
| `executable_mem.rs` | `JitBuffer`: W^X memory management |
| `helpers.rs` | C ABI runtime helpers |
| `translator.rs` | Guest x86 → Host x86-64 translator |

### Devices (`libs/libcorevm/src/devices/`)

| File | Description |
|------|-------------|
| `pic.rs` | Intel 8259A dual PIC |
| `pit.rs` | Intel 8253/8254 PIT |
| `cmos.rs` | CMOS RTC + NVRAM |
| `ps2.rs` | PS/2 keyboard + mouse |
| `serial.rs` | 16550 UART (COM1) |
| `svga.rs` | VGA/SVGA framebuffer (text + graphics modes) |
| `e1000.rs` | Intel E1000 NIC (MMIO, RX/TX rings) |
| `bus.rs` | PCI configuration space |
| `ide.rs` | ATA/IDE controller (PIO, 28/48-bit LBA) |
| `fw_cfg.rs` | QEMU fw_cfg device |
| `debug_port.rs` | QEMU debug port (0x402) |

### BIOS (`libs/libcorevm/bios/`)

| File | Description |
|------|-------------|
| `bios.asm` | Top-level, reset vector, 64 KB padding |
| `post.asm` | Power-On Self-Test |
| `ivt.asm` | Interrupt Vector Table setup |
| `int10h.asm` | Video services |
| `int13h.asm` | Disk services |
| `int15h.asm` | Memory services (E820) |
| `int16h.asm` | Keyboard services |
| `int19h.asm` | Boot (disk + El Torito CD) |
| `int1ah.asm` | RTC/time services |
| `bda.asm` | BIOS Data Area |
| `e820.asm` | Memory map tables |
| `ide.asm` | ATA PIO helper |
| `pic_pit.asm` | PIC/PIT init |
| `serial.asm` | Serial port init |
| `video_init.asm` | VGA mode init |
| `pci.asm` | PCI enumeration |
| `boot.asm` | Boot sector loading |

### Client (`libs/libcorevm_client/src/`)

| File | Description |
|------|-------------|
| `lib.rs` | `CoreVmLib` (58 function pointers), `VmHandle` RAII wrapper, `ExitReason`, `CpuMode`, GPR constants |

### VM Daemon (`bin/vmd/src/`)

| File | Description |
|------|-------------|
| `main.rs` | IPC command loop, VM config parser, execution batch loop, SHM framebuffer updater |

### VM Manager (`apps/vmmanager/src/`)

| File | Description |
|------|-------------|
| `main.rs` | GUI application: sidebar, Canvas display, settings dialog, create-disk dialog, SHM polling |
