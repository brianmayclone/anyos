# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Debug anyOS in QEMU on Windows (GDB server on :1234)
# Usage: .\scripts\debug.ps1 [-Vmware] [-Std]
#
#   -Vmware   VMware SVGA II (2D acceleration, HW cursor)
#   -Std      Bochs VGA / Standard VGA (double-buffering) [default]

param(
    [switch]$Vmware,
    [switch]$Std
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = Join-Path $ProjectDir "build"
$Image = Join-Path $BuildDir "anyos.img"

# ── Check image exists ────────────────────────────────────────────────────────

if (-not (Test-Path $Image)) {
    Write-Host "Error: Disk image not found at $Image" -ForegroundColor Red
    Write-Host "Run: .\scripts\build.ps1 first"
    exit 1
}

# ── Find QEMU ────────────────────────────────────────────────────────────────

$qemu = Get-Command "qemu-system-x86_64" -ErrorAction SilentlyContinue
if (-not $qemu) {
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

# ── VGA selection ─────────────────────────────────────────────────────────────

$vga = "std"
$vgaLabel = "Bochs VGA (standard)"

if ($Vmware) {
    $vga = "vmware"
    $vgaLabel = "VMware SVGA II (accelerated)"
}

# ── Launch ────────────────────────────────────────────────────────────────────

Write-Host "Starting anyOS in debug mode with $vgaLabel (-vga $vga)" -ForegroundColor Cyan
Write-Host "Connect GDB with:" -ForegroundColor Yellow
Write-Host "  gdb -ex 'target remote :1234' -ex 'symbol-file build/kernel/x86_64-anyos/debug/anyos_kernel'" -ForegroundColor Yellow
Write-Host ""

$args = @(
    "-drive", "format=raw,file=$Image",
    "-cpu", "qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt",
    "-m", "1024M",
    "-smp", "cpus=4",
    "-serial", "stdio",
    "-vga", $vga,
    "-netdev", "user,id=net0",
    "-device", "e1000,netdev=net0",
    "-s", "-S",
    "-no-reboot",
    "-no-shutdown"
)

& $qemu @args
