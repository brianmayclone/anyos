# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU on Windows
# Usage: .\scripts\run.ps1 [-Vmware] [-Std] [-Virtio] [-Ide] [-Cdrom] [-Audio] [-Usb] [-Uefi]
#
#   -Vmware   VMware SVGA II (2D acceleration, HW cursor)
#   -Std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   -Virtio   VirtIO GPU (modern transport, ARGB cursor)
#   -Ide      Use legacy IDE (PIO) instead of AHCI (DMA) for disk I/O
#   -Cdrom    Boot from ISO image (CD-ROM) instead of hard drive
#   -Audio    Enable AC'97 audio device
#   -Usb      Enable USB controller with keyboard + mouse devices
#   -Uefi     Boot via UEFI (OVMF) instead of BIOS

param(
    [switch]$Vmware,
    [switch]$Std,
    [switch]$Virtio,
    [switch]$Ide,
    [switch]$Cdrom,
    [switch]$Audio,
    [switch]$Usb,
    [switch]$Uefi
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = Join-Path $ProjectDir "build"

# ── Find QEMU ────────────────────────────────────────────────────────────────

$qemu = Get-Command "qemu-system-x86_64" -ErrorAction SilentlyContinue
if (-not $qemu) {
    # Check default install location
    $qemuDefault = "C:\Program Files\qemu\qemu-system-x86_64.exe"
    if (Test-Path $qemuDefault) {
        $qemu = $qemuDefault
    } else {
        Write-Host "Error: qemu-system-x86_64 not found in PATH or $qemuDefault" -ForegroundColor Red
        Write-Host "Install with: winget install SoftwareFreedomConservancy.QEMU"
        exit 1
    }
} else {
    $qemu = $qemu.Source
}

# ── VGA selection ────────────────────────────────────────────────────────────

$vga = "std"
$vgaLabel = "Bochs VGA (standard)"

if ($Vmware) {
    $vga = "vmware"
    $vgaLabel = "VMware SVGA II (accelerated)"
} elseif ($Virtio) {
    $vga = "virtio"
    $vgaLabel = "Virtio GPU (paravirtualized)"
}

# ── Build QEMU arguments ────────────────────────────────────────────────────

$args = @()

# Disk / boot mode
if ($Cdrom) {
    $image = Join-Path $BuildDir "anyos.iso"
    $driveLabel = "CD-ROM (ISO 9660)"
    $args += "-cdrom", $image, "-boot", "d"
} elseif ($Uefi) {
    $image = Join-Path $BuildDir "anyos-uefi.img"
    $driveLabel = "UEFI (GPT)"

    # Find OVMF firmware
    $ovmfPaths = @(
        "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
        "C:\Program Files\qemu\share\OVMF_CODE.fd"
    )
    $ovmfFw = $null
    foreach ($p in $ovmfPaths) {
        if (Test-Path $p) { $ovmfFw = $p; break }
    }
    if (-not $ovmfFw) {
        Write-Host "Error: OVMF firmware not found." -ForegroundColor Red
        Write-Host "Searched:"
        foreach ($p in $ovmfPaths) { Write-Host "  $p" }
        exit 1
    }
    $args += "-drive", "if=pflash,format=raw,readonly=on,file=$ovmfFw"
    $args += "-drive", "format=raw,file=$image"
} else {
    $image = Join-Path $BuildDir "anyos.img"
    if ($Ide) {
        $driveLabel = "IDE (PIO)"
        $args += "-drive", "format=raw,file=$image"
    } else {
        $driveLabel = "AHCI (DMA)"
        $args += "-drive", "id=hd0,if=none,format=raw,file=$image"
        $args += "-device", "ich9-ahci,id=ahci"
        $args += "-device", "ide-hd,drive=hd0,bus=ahci.0"
    }
}

# Check image exists
if (-not (Test-Path $image)) {
    Write-Host "Error: Image not found at $image" -ForegroundColor Red
    if ($Cdrom) {
        Write-Host "Run: .\scripts\build.ps1 -Iso"
    } else {
        Write-Host "Run: .\scripts\build.ps1"
    }
    exit 1
}

# Core settings
$args += "-m", "1024M"
$args += "-smp", "cpus=4"
$args += "-serial", "stdio"
$args += "-vga", $vga
$args += "-netdev", "user,id=net0"
$args += "-device", "e1000,netdev=net0"
$args += "-no-reboot"
$args += "-no-shutdown"

# Audio (Windows uses wasapi backend)
$audioLabel = ""
if ($Audio) {
    $args += "-device", "AC97,audiodev=audio0"
    $args += "-audiodev", "wasapi,id=audio0"
    $audioLabel = ", audio: AC'97"
}

# USB
$usbLabel = ""
if ($Usb) {
    $args += "-usb"
    $args += "-device", "usb-kbd"
    $args += "-device", "usb-mouse"
    $usbLabel = ", USB: keyboard + mouse"
}

# ── Launch ───────────────────────────────────────────────────────────────────

Write-Host "Starting anyOS with $vgaLabel (-vga $vga), disk: $driveLabel$audioLabel$usbLabel" -ForegroundColor Cyan
& $qemu @args
