# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build anyOS on Windows
# Usage: .\scripts\build.ps1 [-Clean] [-Reset] [-Uefi] [-Iso] [-All] [-Debug] [-NoCross]

param(
    [switch]$Clean,
    [switch]$Reset,
    [switch]$Uefi,
    [switch]$Iso,
    [switch]$All,
    [switch]$Debug,
    [switch]$NoCross
)

$ErrorActionPreference = "Stop"

$BuildStart = Get-Date
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = Join-Path $ProjectDir "build"

# Ensure cargo is in PATH
$cargoDir = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoDir) {
    if ($env:Path -notlike "*$cargoDir*") {
        $env:Path = "$cargoDir;$env:Path"
    }
}

# CMake flags
$debugFlag   = if ($Debug)   { "ON" } else { "OFF" }
$noCrossFlag = if ($NoCross) { "ON" } else { "OFF" }
$resetFlag   = if ($Reset)   { "ON" } else { "OFF" }
$cmakeExtra  = "-DANYOS_DEBUG_VERBOSE=$debugFlag", "-DANYOS_NO_CROSS=$noCrossFlag", "-DANYOS_RESET=$resetFlag"

# Ensure build directory exists and is configured
if (-not (Test-Path (Join-Path $BuildDir "build.ninja"))) {
    Write-Host "Configuring build..."
    & cmake -B $BuildDir -G Ninja $cmakeExtra $ProjectDir
    if ($LASTEXITCODE -ne 0) {
        Write-Host "CMake configuration failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

# Force full rebuild if -Clean
if ($Clean) {
    Write-Host "Cleaning build..."
    & (Join-Path $ScriptDir "clean.ps1") -All
    # Re-configure CMake after clean (entire build dir was removed)
    Write-Host "Configuring build..."
    & cmake -B $BuildDir -G Ninja $cmakeExtra $ProjectDir
    if ($LASTEXITCODE -ne 0) {
        Write-Host "CMake configuration failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

# Always re-run cmake to pick up flag changes (fast if nothing changed)
& cmake -B $BuildDir -G Ninja $cmakeExtra $ProjectDir 2>$null | Out-Null

# Suppress Rust warnings — only show errors
if ($env:RUSTFLAGS) {
    $env:RUSTFLAGS = "$($env:RUSTFLAGS) -Awarnings"
} else {
    $env:RUSTFLAGS = "-Awarnings"
}

# Build BIOS image (default target)
Write-Host "Building anyOS (BIOS)..." -ForegroundColor Cyan
& ninja -C $BuildDir
if ($LASTEXITCODE -ne 0) {
    Write-Host "BIOS build failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}
Write-Host "BIOS build successful." -ForegroundColor Green

# Build UEFI image if requested
if ($Uefi -or $All) {
    Write-Host "Building anyOS (UEFI)..." -ForegroundColor Cyan
    & ninja -C $BuildDir uefi-image
    if ($LASTEXITCODE -ne 0) {
        Write-Host "UEFI build failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "UEFI build successful." -ForegroundColor Green
}

# Build ISO image if requested
if ($Iso -or $All) {
    Write-Host "Building anyOS (ISO 9660, El Torito)..." -ForegroundColor Cyan
    & ninja -C $BuildDir iso
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ISO build failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "ISO build successful: $BuildDir\anyos.iso" -ForegroundColor Green
}

$elapsed = (Get-Date) - $BuildStart
Write-Host ("Build complete in {0:mm\:ss}" -f $elapsed) -ForegroundColor Green
