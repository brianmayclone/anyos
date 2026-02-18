# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Setup anyOS development toolchain on Windows
# Usage: powershell -ExecutionPolicy Bypass -File scripts\setup_toolchain.ps1

#Requires -Version 5.1
$ErrorActionPreference = "Stop"

Write-Host "Setting up anyOS development toolchain (Windows)..." -ForegroundColor Cyan
Write-Host ""

# ── Helper: check if a command exists ────────────────────────────────────────

function Test-CommandExists {
    param([string]$Command)
    $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

# ── Helper: install via winget ───────────────────────────────────────────────

function Install-ViaWinget {
    param(
        [string]$Name,
        [string]$WingetId,
        [string]$FallbackUrl
    )
    Write-Host "Installing $Name..."
    if (Test-CommandExists "winget") {
        winget install --accept-source-agreements --accept-package-agreements -e --id $WingetId
        if ($LASTEXITCODE -ne 0) {
            Write-Host "  winget install failed. Download manually from: $FallbackUrl" -ForegroundColor Yellow
            return $false
        }
        return $true
    } else {
        Write-Host "  winget not found. Download manually from: $FallbackUrl" -ForegroundColor Yellow
        return $false
    }
}

# ── Helper: refresh PATH within this session ─────────────────────────────────

function Update-SessionPath {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machinePath;$userPath"
}

# ── Rust nightly ─────────────────────────────────────────────────────────────

Write-Host "--- Rust nightly ---" -ForegroundColor Green
if (-not (Test-CommandExists "rustup")) {
    Write-Host "Installing rustup..."
    $rustupInit = Join-Path $env:TEMP "rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit
    & $rustupInit -y --default-toolchain nightly
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: rustup installation failed." -ForegroundColor Red
        exit 1
    }
    Remove-Item $rustupInit -ErrorAction SilentlyContinue
    # Add cargo to current session PATH
    $cargoPath = Join-Path $env:USERPROFILE ".cargo\bin"
    if ($env:Path -notlike "*$cargoPath*") {
        $env:Path = "$cargoPath;$env:Path"
    }
}
rustup install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
Write-Host ""

# ── NASM ─────────────────────────────────────────────────────────────────────

Write-Host "--- NASM ---" -ForegroundColor Green
if (-not (Test-CommandExists "nasm")) {
    Install-ViaWinget "NASM" "NASM.NASM" "https://www.nasm.us/pub/nasm/releasebuilds/"
    Update-SessionPath
    # NASM installs to C:\Program Files\NASM but may not add to PATH
    $nasmDir = "C:\Program Files\NASM"
    if ((Test-Path $nasmDir) -and ($env:Path -notlike "*$nasmDir*")) {
        $env:Path = "$nasmDir;$env:Path"
        # Persist to user PATH
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$nasmDir*") {
            [Environment]::SetEnvironmentVariable("Path", "$userPath;$nasmDir", "User")
            Write-Host "  Added $nasmDir to user PATH" -ForegroundColor DarkGray
        }
    }
}
Write-Host ""

# ── CMake ────────────────────────────────────────────────────────────────────

Write-Host "--- CMake ---" -ForegroundColor Green
if (-not (Test-CommandExists "cmake")) {
    Install-ViaWinget "CMake" "Kitware.CMake" "https://cmake.org/download/"
    Update-SessionPath
}
Write-Host ""

# ── Ninja ────────────────────────────────────────────────────────────────────

Write-Host "--- Ninja ---" -ForegroundColor Green
if (-not (Test-CommandExists "ninja")) {
    Install-ViaWinget "Ninja" "Ninja-build.Ninja" "https://github.com/ninja-build/ninja/releases"
    Update-SessionPath
}
Write-Host ""

# ── QEMU ─────────────────────────────────────────────────────────────────────

Write-Host "--- QEMU ---" -ForegroundColor Green
$qemuExe = Get-Command "qemu-system-x86_64" -ErrorAction SilentlyContinue
if (-not $qemuExe) {
    # Check default install location
    $qemuDefaultDir = "C:\Program Files\qemu"
    if (Test-Path (Join-Path $qemuDefaultDir "qemu-system-x86_64.exe")) {
        $env:Path = "$qemuDefaultDir;$env:Path"
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$qemuDefaultDir*") {
            [Environment]::SetEnvironmentVariable("Path", "$userPath;$qemuDefaultDir", "User")
            Write-Host "  Added $qemuDefaultDir to user PATH" -ForegroundColor DarkGray
        }
    } else {
        Install-ViaWinget "QEMU" "SoftwareFreedomConservancy.QEMU" "https://www.qemu.org/download/#windows"
        Update-SessionPath
    }
}
Write-Host ""

# ── Python 3 + pip packages ─────────────────────────────────────────────────

Write-Host "--- Python 3 ---" -ForegroundColor Green
if (-not (Test-CommandExists "python")) {
    Install-ViaWinget "Python 3" "Python.Python.3.12" "https://www.python.org/downloads/"
    Update-SessionPath
}

# Install pip packages for build scripts
$missingPkg = $false
try { & python -c "import PIL" 2>$null; if ($LASTEXITCODE -ne 0) { $missingPkg = $true } } catch { $missingPkg = $true }
try { & python -c "import fontTools" 2>$null; if ($LASTEXITCODE -ne 0) { $missingPkg = $true } } catch { $missingPkg = $true }
if ($missingPkg) {
    Write-Host "Installing Python packages (Pillow, fonttools)..."
    & python -m pip install --user Pillow fonttools 2>$null
}
Write-Host ""

# ── MSYS2 + i686-elf cross-compiler ─────────────────────────────────────────

Write-Host "--- MSYS2 + i686-elf-gcc cross-compiler ---" -ForegroundColor Green
$msys2Root = "C:\msys64"
$msys2Bash = Join-Path $msys2Root "usr\bin\bash.exe"

if (-not (Test-Path $msys2Bash)) {
    Write-Host "Installing MSYS2..."
    Install-ViaWinget "MSYS2" "MSYS2.MSYS2" "https://www.msys2.org/"
    Update-SessionPath
}

if (Test-Path $msys2Bash) {
    # Update MSYS2 package database and install cross-compiler
    Write-Host "Installing i686-elf cross-compiler via MSYS2 pacman..."
    & $msys2Bash --login -c "pacman -Syu --noconfirm" 2>$null
    & $msys2Bash --login -c "pacman -S --needed --noconfirm mingw-w64-x86_64-i686-elf-gcc mingw-w64-x86_64-i686-elf-binutils make" 2>$null

    # Add MSYS2 MinGW64 bin to PATH (where i686-elf-gcc lives)
    $mingw64Bin = Join-Path $msys2Root "mingw64\bin"
    if ((Test-Path $mingw64Bin) -and ($env:Path -notlike "*$mingw64Bin*")) {
        $env:Path = "$mingw64Bin;$env:Path"
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$mingw64Bin*") {
            [Environment]::SetEnvironmentVariable("Path", "$userPath;$mingw64Bin", "User")
            Write-Host "  Added $mingw64Bin to user PATH" -ForegroundColor DarkGray
        }
    }

    # Also add MSYS2 usr/bin for make
    $msys2Bin = Join-Path $msys2Root "usr\bin"
    if ((Test-Path $msys2Bin) -and ($env:Path -notlike "*$msys2Bin*")) {
        $env:Path = "$msys2Bin;$env:Path"
    }
} else {
    Write-Host "  MSYS2 not found at $msys2Root." -ForegroundColor Yellow
    Write-Host "  Install manually from https://www.msys2.org/ then re-run this script."
    Write-Host "  The cross-compiler is needed for libc/TCC. Rust-only builds work without it."
}
Write-Host ""

# ── OVMF firmware (UEFI boot, optional) ─────────────────────────────────────

Write-Host "--- OVMF firmware (optional, for UEFI boot) ---" -ForegroundColor Green
$ovmfPaths = @(
    "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
    "C:\Program Files\qemu\share\OVMF_CODE.fd",
    "C:\Program Files\qemu\share\edk2-x86_64-code.fd.bz2"
)
$ovmfFound = $false
foreach ($p in $ovmfPaths) {
    if (Test-Path $p) {
        Write-Host "  OVMF found at $p"
        $ovmfFound = $true
        break
    }
}
if (-not $ovmfFound) {
    Write-Host "  OVMF not found. It is usually bundled with QEMU."
    Write-Host "  (UEFI boot is optional - BIOS boot works without it)"
}
Write-Host ""

# ── Summary ──────────────────────────────────────────────────────────────────

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Toolchain versions:"

$tools = @(
    @{ Name = "rustc";          Cmd = { & rustc +nightly --version 2>$null } },
    @{ Name = "nasm";           Cmd = { & nasm -v 2>$null } },
    @{ Name = "cmake";          Cmd = { (& cmake --version 2>$null) | Select-Object -First 1 } },
    @{ Name = "ninja";          Cmd = { & ninja --version 2>$null } },
    @{ Name = "qemu";           Cmd = { (& qemu-system-x86_64 --version 2>$null) | Select-Object -First 1 } },
    @{ Name = "python";         Cmd = { & python --version 2>$null } },
    @{ Name = "i686-elf-gcc";   Cmd = { (& i686-elf-gcc --version 2>$null) | Select-Object -First 1 } }
)

foreach ($tool in $tools) {
    $ver = try { & $tool.Cmd } catch { $null }
    if ($ver) {
        Write-Host ("  {0}: {1}" -f $tool.Name, $ver)
    } else {
        Write-Host ("  {0}: not found" -f $tool.Name) -ForegroundColor Yellow
    }
}

Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Toolchain setup complete!" -ForegroundColor Green
Write-Host ""
Write-Host "Next steps:"
Write-Host "  .\scripts\build.ps1           # Build everything"
Write-Host "  .\scripts\run.ps1             # Run in QEMU"
Write-Host "  .\scripts\run.ps1 -Vmware     # Run with VMware SVGA"
