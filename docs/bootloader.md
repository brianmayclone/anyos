# anyOS Bootloader

anyOS supports three boot methods: **BIOS** (two-stage MBR bootloader), **UEFI** (Rust EFI application), and **ISO** (El Torito CD-ROM). The BIOS bootloader features a graphical splash screen, interactive boot menu, and INI-style configuration via `boot.cfg`.

---

## Table of Contents

- [BIOS Boot (Stage 1 + Stage 2)](#bios-boot-stage-1--stage-2)
  - [Stage 1 (MBR)](#stage-1-mbr)
  - [Stage 2](#stage-2)
  - [Boot Flow](#boot-flow)
- [Boot Configuration (boot.cfg)](#boot-configuration-bootcfg)
  - [Global Options](#global-options)
  - [Boot Entries](#boot-entries)
  - [Kernel Parameters](#kernel-parameters)
  - [Chainloading](#chainloading)
  - [Example Configuration](#example-configuration)
- [Boot Menu](#boot-menu)
  - [Splash Screen](#splash-screen)
  - [Menu Navigation](#menu-navigation)
- [UEFI Boot](#uefi-boot)
- [ISO Boot (El Torito)](#iso-boot-el-torito)
- [BootInfo Structure](#bootinfo-structure)
- [Memory Layout](#memory-layout)
- [Source Files](#source-files)

---

## BIOS Boot (Stage 1 + Stage 2)

### Stage 1 (MBR)

The first-stage bootloader is a 512-byte MBR boot sector written in NASM assembly (`bootloader/stage1/boot.asm`).

1. Loaded by BIOS at `0x7C00`
2. Loads Stage 2 from disk sectors 1–63 to `0x8000` via INT 13h
3. Fallback for CD-ROM: copies from `0x7E00` (contiguous El Torito load image)
4. Jumps to Stage 2 with `DL` = boot drive number

### Stage 2

Stage 2 is a modular NASM bootloader (~32 KB) that handles hardware setup, configuration parsing, graphical UI, and kernel loading.

**Modules:**

| Module | Purpose |
|--------|---------|
| `stage2.asm` | Main boot flow coordinator and entry point |
| `a20.asm` | A20 line enable (3 methods: BIOS INT 15h, keyboard controller, port 0x92) |
| `memory_map.asm` | E820 BIOS memory map query |
| `disk.asm` | Sector loading, HDD/CD-ROM detection via INT 13h |
| `vesa.asm` | VBE graphics mode setup (1024x768 → 800x600 → 640x480, 32bpp) |
| `protected_mode.asm` | GDT, 4-level page tables (PML4), PAE, long mode entry |
| `config.asm` | `boot.cfg` INI parser |
| `splash.asm` | Boot logo display with 2x upscaling |
| `menu.asm` | Interactive boot menu UI with keyboard navigation |
| `font.asm` | 8x16 bitmap font renderer for VESA framebuffer |
| `chainload.asm` | Entry dispatch, chainload sequence, custom boot parameter input |
| `exfat.asm` | Minimal read-only exFAT driver (init, directory search, file read) |

### Boot Flow

```
Stage 1 (MBR, 0x7C00)
    │ Load sectors 1-63 to 0x8000
    v
Stage 2 (0x8000)
    ├── Enable A20 line
    ├── Query E820 memory map
    ├── Enter unreal mode (32-bit addressing in real mode)
    ├── Set VESA VBE graphics mode (32bpp)
    ├── Detect boot medium (CD-ROM vs HDD)
    │     ├── HDD path (exFAT):
    │     │     ├── exfat_init → mount exFAT filesystem at sector 128+
    │     │     └── Load /System/krnl64, /boot/boot.cfg, /boot/logo.bin,
    │     │           /boot/font.bin from exFAT
    │     └── CD-ROM path (raw sectors, legacy):
    │           └── Load files from patched sector LBAs (El Torito layout)
    ├── Parse boot.cfg
    ├── Display splash screen (timeout countdown)
    │     └── Press Escape → open boot menu
    ├── Show boot menu (if Escape pressed)
    │     └── Up/Down to select, Enter to boot
    └── Execute selected entry:
          ├── Kernel boot (type=0):
          │     ├── Load kernel to 0x100000
          │     ├── Fill BootInfo at 0x9000
          │     ├── Switch to protected mode → long mode
          │     └── Jump to 0xFFFFFFFF80100000 (kernel entry)
          └── Chainload (type=1):
                ├── Read MBR of target disk
                ├── Find partition entry
                ├── Load VBR to 0x7C00
                └── Jump to 0x7C00
```

---

## Boot Configuration (boot.cfg)

The bootloader reads an INI-style configuration file from `/boot/boot.cfg` on the boot disk (HDD exFAT path). For CD-ROM/ISO boot, the configuration is loaded from a patched raw sector LBA using the legacy sector-based loading path. The file is loaded into memory at `0x30000` (max 8 KB).

### Global Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `timeout` | Integer | 5 | Seconds before auto-booting the default entry. Minimum: 3 seconds (enforced). |
| `default` | Integer | 0 | 0-based index of the default boot entry. Clamped to entry count. |

### Boot Entries

Each entry is defined as an INI section with a name in square brackets:

```ini
[Entry Name]
kernel=0                # Boot type: kernel (0) or chainload (1)
params=verbose          # Kernel boot parameters (max 64 chars)
description=text        # Description (for documentation, not displayed)
```

**Limits:**
- Maximum 8 boot entries
- Entry name: max 31 characters
- Parameters: max 64 characters

**Entry fields:**

| Field | Type | Description |
|-------|------|-------------|
| `kernel=0` | Flag | Boot as anyOS kernel (type=0) |
| `chainload=1` | Flag | Chainload another OS from disk/partition (type=1) |
| `params` | String | Kernel parameters passed via BootInfo (max 64 chars) |
| `disk` | Integer | Target disk number for chainloading (0 = first HDD) |
| `partition` | Integer | MBR partition index for chainloading (0-based) |
| `description` | String | Human-readable description |

### Kernel Parameters

The `params` field is copied into the `BootInfo` structure and made available to the kernel at boot. The kernel can use these to enable verbose logging, safe mode, or other runtime options.

**Special value:** `params=custom` prompts the user for interactive input at boot time. A text input field appears below the menu where the user can type custom boot parameters (max 63 characters).

### Chainloading

To boot another operating system, use `chainload=1` with `disk` and `partition`:

```ini
[Windows]
chainload=1
disk=1
partition=1
```

The bootloader reads the MBR of the target disk, locates the specified partition entry, loads the Volume Boot Record (VBR) to `0x7C00`, resets video mode, and jumps to `0x7C00`.

### Example Configuration

```ini
# anyOS Boot Configuration
timeout=5
default=0

[anyOS]
kernel=0
description=anyOS with default settings

[anyOS (Verbose)]
kernel=0
params=verbose
description=anyOS with verbose kernel logging

[anyOS (Custom)]
kernel=0
params=custom
description=anyOS with custom boot parameters

[Windows]
chainload=1
disk=1
partition=1
```

---

## Boot Menu

### Splash Screen

On boot, the bootloader displays a graphical splash screen:

- **Background:** Black screen
- **Boot logo:** 52x52 RGB image, upscaled 2x to 104x104 pixels (nearest-neighbor), centered horizontally at ~15% from the top. Black pixels (0,0,0) are treated as transparent.
- **Duration:** Controlled by `timeout` setting (default 5 seconds)
- **Interaction:** Press **Escape** to skip the splash and open the boot menu immediately

If no VESA graphics mode is available, the bootloader skips the splash and menu entirely, auto-booting the default entry in text mode.

### Menu Navigation

When the boot menu is displayed:

| Key | Action |
|-----|--------|
| **Up Arrow** | Select previous entry |
| **Down Arrow** | Select next entry |
| **Enter** | Boot selected entry |

**Visual style:**
- **Selected entry:** Light blue text (0x00AACCFF) on dark highlight bar (0x00202830), 20px tall
- **Unselected entries:** Gray text (0x00888888) on black
- **Footer:** "Up/Down Select   Enter Boot" in dark gray (0x00666666)
- **Font:** 8x16 bitmap, ASCII 32–126

The boot logo is repositioned to ~20% from the top when the menu is displayed, with entries listed below it.

---

## UEFI Boot

The UEFI bootloader (`bootloader/uefi/`) is a Rust application compiled as a PE/COFF EFI binary (`bootx64.efi`).

1. UEFI firmware loads `\EFI\BOOT\bootx64.efi` from the EFI System Partition (FAT32)
2. Queries memory map via `GetMemoryMap()`
3. Sets graphics mode via GOP (Graphics Output Protocol)
4. Reads kernel from the exFAT data partition (`System/kernel.bin`, fallback `System/kernel.bak`)
5. Converts UEFI memory map to E820 format
6. Fills `BootInfo` at `0x9000`
7. Builds 4-level page tables (identity-map first 128 MiB + higher-half kernel mapping)
8. Calls `ExitBootServices()`
9. Loads CR3, jumps to kernel entry point

**Note:** The UEFI path does not currently support `boot.cfg` or the graphical boot menu. It always boots the default kernel.

---

## ISO Boot (El Torito)

ISO 9660 boot images use an El Torito no-emulation boot catalog. The Stage 1 MBR detects the CD-ROM boot and falls back to copying Stage 2 from the contiguous El Torito load image at `0x7E00`. From there, the normal Stage 2 flow applies.

QEMU flags: `ninja run-cdrom` or `ninja run-cdrom-std`

---

## BootInfo Structure

The bootloader passes a `BootInfo` structure at physical address `0x9000` to the kernel:

| Field | Offset | Size | Description |
|-------|--------|------|-------------|
| Framebuffer address | 0 | 8 | Physical address of VBE/GOP framebuffer |
| Framebuffer width | 8 | 4 | Pixels |
| Framebuffer height | 12 | 4 | Pixels |
| Framebuffer pitch | 16 | 4 | Bytes per scanline |
| Memory map pointer | 20 | 8 | Pointer to E820 entries |
| Memory map count | 28 | 4 | Number of E820 entries |
| Disk geometry | 32 | 8 | Boot disk info |
| Boot mode | 40 | 4 | 0=BIOS, 1=UEFI |
| Boot parameters | 44 | 64 | Null-terminated string from `params=` in boot.cfg |

---

## Memory Layout

```
0x0000-0x7BFF    Real-mode IVT, BIOS data area
0x7C00-0x7DFF    Stage 1 (MBR, 512 bytes)
0x7E00-0xFBFF    CD-ROM stage 2 contiguous load (El Torito fallback)
0x8000-0xFDFF    Stage 2 (up to 63 sectors)
0x1000           E820 memory map entries
0x2000           VESA VBE mode info block (256 bytes)
0x2200           VBE controller info (512 bytes)
0x9000           BootInfo structure
0x10000-0x1FFFF  Temporary sector buffer for disk I/O
0x20000-0x27FFF  Boot logo (52x52 RGB, max 8 KB)
0x28000-0x29FFF  Boot font (95 glyphs x 16 bytes)
0x30000-0x37FFF  boot.cfg config file (max 8 KB)
0x100000+        Kernel binary (loaded here for long mode jump)
```

### Disk Sector Layout

**HDD (exFAT):**
```
Sector 0:        MBR (Stage 1)
Sectors 1-63:    Stage 2 (~7.5 KB)
Sectors 64-127:  Reserved
Sector 128+:     exFAT filesystem (kernel, boot assets, configuration)
```

**ISO/CD-ROM (legacy raw-sector layout):**
The ISO boot path does not use exFAT. Boot assets (logo, font, config) are embedded at fixed patched sector LBAs within the El Torito load image, as in previous releases.

---

## Source Files

```
bootloader/
  stage1/
    boot.asm              MBR boot sector (512 bytes)
  stage2/
    stage2.asm            Main boot flow coordinator
    a20.asm               A20 line enable (3 fallback methods)
    memory_map.asm        E820 BIOS memory map query
    disk.asm              Sector loading, HDD/CD-ROM detection
    vesa.asm              VBE graphics mode setup
    protected_mode.asm    GDT, 4-level paging, long mode entry
    config.asm            boot.cfg INI parser
    splash.asm            Boot logo display (2x upscale, transparency)
    menu.asm              Interactive boot menu UI
    font.asm              8x16 bitmap font renderer
    chainload.asm         Entry dispatch, chainload, custom params input
    exfat.asm             Minimal read-only exFAT driver (HDD boot only)
  uefi/
    Cargo.toml            Rust UEFI bootloader crate
    src/main.rs           UEFI kernel loader
```

The boot configuration file is located at `sysroot/boot/boot.cfg` and is installed to `/boot/boot.cfg` on the HDD exFAT partition. For ISO/CD-ROM boot, the legacy raw-sector path is used instead.
