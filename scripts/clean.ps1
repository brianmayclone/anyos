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
#   (no args)  Remove Cargo/program build artifacts (forces full rebuild)
#   -All       Remove entire build directory (requires re-running CMake)

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
    $libcDir = Join-Path $ProjectDir "programs\libc"
    if (Test-Path (Join-Path $libcDir "Makefile")) {
        $makeCmd = Get-Command "make" -ErrorAction SilentlyContinue
        if ($makeCmd) {
            & make -C $libcDir clean 2>$null
        }
    }
    Write-Host "Done. Run .\scripts\build.ps1 to rebuild from scratch."
    exit 0
}

Write-Host "Cleaning build artifacts..."

# Kernel
Write-Host "  Kernel..."
Remove-Quietly (Join-Path $BuildDir "kernel\x86_64-anyos")

# DLLs
Write-Host "  DLLs..."
Remove-Quietly (Join-Path $BuildDir "dll")

# User and system programs
Write-Host "  Programs..."
$programsDir = Join-Path $BuildDir "programs"
if (Test-Path $programsDir) {
    Get-ChildItem -Path $programsDir -Directory | ForEach-Object {
        Remove-Quietly (Join-Path $_.FullName "x86_64-anyos-user")
        Remove-Quietly (Join-Path $_.FullName "debug")
        Remove-Quietly (Join-Path $_.FullName "release")
        # Nested dirs (e.g. programs\compositor\dock)
        Get-ChildItem -Path $_.FullName -Directory -ErrorAction SilentlyContinue | ForEach-Object {
            Remove-Quietly (Join-Path $_.FullName "x86_64-anyos-user")
            Remove-Quietly (Join-Path $_.FullName "debug")
            Remove-Quietly (Join-Path $_.FullName "release")
        }
    }
}

# Libc
Write-Host "  Libc..."
$libcDir = Join-Path $ProjectDir "programs\libc"
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

# Disk image
Write-Host "  Disk image..."
Remove-Item -Force (Join-Path $BuildDir "anyos.img") -ErrorAction SilentlyContinue

# Flat binaries
Write-Host "  Flat binaries..."
Remove-Item -Force (Join-Path $BuildDir "anyos_kernel.bin") -ErrorAction SilentlyContinue

Write-Host "Done. Run .\scripts\build.ps1 to rebuild."
