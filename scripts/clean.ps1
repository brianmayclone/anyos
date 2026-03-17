# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Clean anyOS build artifacts on Windows
# Usage: .\scripts\clean.ps1 [-All]
#
#   (no args)  Remove all build artifacts (kernel, stdlib, DLLs, shared libs,
#              user/system programs, libc, TCC, buildsystem tools, sysroot, image).
#              Preserves CMake cache — just run: ninja -C build
#   -All       Remove entire build directory (requires re-running CMake + ninja)

param(
    [switch]$All
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = Join-Path $ProjectDir "build"

if (-not (Test-Path $BuildDir)) {
    Write-Host "Nothing to clean (no build directory)"
    exit 0
}

function Remove-Quietly {
    param([string]$Path)
    if (Test-Path $Path) {
        Remove-Item -Recurse -Force $Path -ErrorAction SilentlyContinue
    }
}

if ($All) {
    Write-Host "Removing entire build directory..."
    Remove-Item -Recurse -Force $BuildDir -ErrorAction SilentlyContinue
    # Also clean libc build artifacts in source tree
    $libcDir = Join-Path $ProjectDir "libs\libc"
    if (Test-Path (Join-Path $libcDir "Makefile")) {
        $makeCmd = Get-Command "make" -ErrorAction SilentlyContinue
        if ($makeCmd) {
            & make -C $libcDir clean 2>$null
        }
    }
    Write-Host "Done. Run: cmake -B build -G Ninja; ninja -C build"
    exit 0
}

Write-Host "Cleaning build artifacts..."

# Kernel
Write-Host "  Kernel..."
Remove-Quietly (Join-Path $BuildDir "kernel")

# DLLs (uisys, libcompositor, libimage, librender)
Write-Host "  DLLs..."
Remove-Quietly (Join-Path $BuildDir "dll")

# Shared libraries (.so — libanyui, libfont, libdb)
Write-Host "  Shared libraries..."
Remove-Quietly (Join-Path $BuildDir "shlib")

# User and system programs
Write-Host "  Programs..."
Remove-Quietly (Join-Path $BuildDir "programs")

# Buildsystem tools (anyelf, anyld, mkimage, mkappbundle)
Write-Host "  Buildsystem tools..."
Remove-Quietly (Join-Path $BuildDir "buildsystem")

# Libc (built in source tree via Makefile)
Write-Host "  Libc..."
$libcDir = Join-Path $ProjectDir "libs\libc"
if (Test-Path (Join-Path $libcDir "Makefile")) {
    $makeCmd = Get-Command "make" -ErrorAction SilentlyContinue
    if ($makeCmd) {
        & make -C $libcDir clean 2>$null
    }
}

# TCC
Write-Host "  TCC..."
Remove-Item -Force (Join-Path $BuildDir "tcc.o") -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $BuildDir "tcc.elf") -ErrorAction SilentlyContinue

# Sysroot
Write-Host "  Sysroot..."
Remove-Quietly (Join-Path $BuildDir "sysroot")

# Disk images
Write-Host "  Disk images..."
Remove-Item -Force (Join-Path $BuildDir "anyos.img") -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $BuildDir "anyos-uefi.img") -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $BuildDir "anyos.iso") -ErrorAction SilentlyContinue

# Flat binaries
Write-Host "  Flat binaries..."
Remove-Item -Force (Join-Path $BuildDir "anyos_kernel.bin") -ErrorAction SilentlyContinue

Write-Host "Done. Run: ninja -C build"
