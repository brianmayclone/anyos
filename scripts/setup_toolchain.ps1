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

# ── Helper: check if python is real (not a Windows Store App Execution Alias) ─

function Test-PythonReal {
    try {
        $ver = & python --version 2>&1
        return ($LASTEXITCODE -eq 0 -and "$ver" -match "Python \d")
    } catch {
        return $false
    }
}

# ── Helper: install via winget ───────────────────────────────────────────────

function Install-ViaWinget {
    param(
        [string]$Name,
        [string]$WingetId,
        [string]$FallbackUrl
    )
    if (-not (Test-CommandExists "winget")) {
        Write-Host "  winget not found. Download manually from: $FallbackUrl" -ForegroundColor Yellow
        return $false
    }
    Write-Host "Installing $Name via winget..."
    winget install --accept-source-agreements --accept-package-agreements -e --id $WingetId
    $ec = $LASTEXITCODE
    # 0                = success
    # 0x8A15002B (-1978335189) = already installed (treat as success)
    if ($ec -eq 0 -or $ec -eq -1978335189) {
        return $true
    }
    Write-Host "  winget exited with code $ec. Download manually from: $FallbackUrl" -ForegroundColor Yellow
    return $false
}

# ── Helper: refresh PATH within this session ─────────────────────────────────

function Update-SessionPath {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machinePath;$userPath"
}

# ── Helper: add a dir to current + user PATH if not already present ───────────

function Add-ToPathIfNeeded {
    param([string]$Dir)
    if (-not (Test-Path $Dir)) { return }
    if ($env:Path -notlike "*$Dir*") {
        $env:Path = "$Dir;$env:Path"
    }
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$Dir*") {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$Dir", "User")
        Write-Host "  Added $Dir to user PATH" -ForegroundColor DarkGray
    }
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
# Fix PATH first in case NASM is installed but wasn't added to PATH
Add-ToPathIfNeeded "C:\Program Files\NASM"
if (-not (Test-CommandExists "nasm")) {
    Install-ViaWinget "NASM" "NASM.NASM" "https://www.nasm.us/pub/nasm/releasebuilds/"
    Update-SessionPath
    Add-ToPathIfNeeded "C:\Program Files\NASM"
}
Write-Host ""

# ── CMake ────────────────────────────────────────────────────────────────────

Write-Host "--- CMake ---" -ForegroundColor Green
Add-ToPathIfNeeded "C:\Program Files\CMake\bin"
if (-not (Test-CommandExists "cmake")) {
    Install-ViaWinget "CMake" "Kitware.CMake" "https://cmake.org/download/"
    Update-SessionPath
    Add-ToPathIfNeeded "C:\Program Files\CMake\bin"
}
Write-Host ""

# ── Ninja ────────────────────────────────────────────────────────────────────

Write-Host "--- Ninja ---" -ForegroundColor Green
# Ninja is typically added to PATH by winget; no fixed install dir to probe
if (-not (Test-CommandExists "ninja")) {
    Install-ViaWinget "Ninja" "Ninja-build.Ninja" "https://github.com/ninja-build/ninja/releases"
    Update-SessionPath
}
Write-Host ""

# ── QEMU ─────────────────────────────────────────────────────────────────────

Write-Host "--- QEMU ---" -ForegroundColor Green
Add-ToPathIfNeeded "C:\Program Files\qemu"
if (-not (Test-CommandExists "qemu-system-x86_64")) {
    Install-ViaWinget "QEMU" "SoftwareFreedomConservancy.QEMU" "https://www.qemu.org/download/#windows"
    Update-SessionPath
    Add-ToPathIfNeeded "C:\Program Files\qemu"
}
Write-Host ""

# ── Python 3 + pip packages ─────────────────────────────────────────────────

Write-Host "--- Python 3 ---" -ForegroundColor Green
if (-not (Test-PythonReal)) {
    Install-ViaWinget "Python 3" "Python.Python.3.12" "https://www.python.org/downloads/"
    Update-SessionPath
}

# Install pip packages for build scripts (only if python is now available)
if (Test-PythonReal) {
    $missingPkg = $false
    try { & python -c "import PIL" 2>$null; if ($LASTEXITCODE -ne 0) { $missingPkg = $true } } catch { $missingPkg = $true }
    try { & python -c "import fontTools" 2>$null; if ($LASTEXITCODE -ne 0) { $missingPkg = $true } } catch { $missingPkg = $true }
    if ($missingPkg) {
        Write-Host "Installing Python packages (Pillow, fonttools)..."
        try {
            & python -m pip install --user Pillow fonttools 2>$null
        } catch {
            Write-Host "  pip install failed. Run manually: python -m pip install --user Pillow fonttools" -ForegroundColor Yellow
        }
    }
} else {
    Write-Host "  Python not found after install attempt. Install manually from https://www.python.org/downloads/" -ForegroundColor Yellow
    Write-Host "  Then run: python -m pip install --user Pillow fonttools" -ForegroundColor Yellow
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
    # MSYS2 has no pre-built i686-elf-gcc package.
    # The cross-compiler installs to ~/opt/cross/bin inside MSYS2,
    # which maps to %USERPROFILE%\opt\cross\bin on Windows.
    $crossBin = Join-Path $env:USERPROFILE "opt\cross\bin"
    Add-ToPathIfNeeded $crossBin
    # NOTE: do NOT add C:\msys64\usr\bin to the Windows PATH —
    # it contains a GNU coreutils link.exe that shadows MSVC link.exe
    # and breaks Rust/MSVC builds. make is found via CMake's find_program.

    $crossGcc = Join-Path $crossBin "i686-elf-gcc.exe"
    if (Test-Path $crossGcc) {
        Write-Host "  i686-elf-gcc already built at $crossBin" -ForegroundColor DarkGray
    } else {
        Write-Host "  i686-elf-gcc not found. Building from source via MSYS2..."
        Write-Host "  This downloads binutils + GCC and compiles them -- expect 20-30 min."

        # Update package database and install build deps (pacman replaces apt-get here)
        & $msys2Bash --login -c "pacman -Syu --noconfirm"
        & $msys2Bash --login -c "pacman -S --needed --noconfirm base-devel gcc wget gmp-devel mpc-devel mpfr-devel"

        # Inline build matching build_cross_compiler.sh but for MSYS2
        $buildCmd = @'
set -euo pipefail
TARGET="i686-elf"
PREFIX="$HOME/opt/cross"
BINUTILS_VERSION="2.44"
GCC_VERSION="14.2.0"
JOBS="$(nproc)"
SRC_DIR="$HOME/src/cross-compiler"
BUILD_DIR="$HOME/build/cross-compiler"
mkdir -p "$PREFIX" "$SRC_DIR" "$BUILD_DIR"
export PATH="$PREFIX/bin:$PATH"
cd "$SRC_DIR"
[ ! -f "binutils-${BINUTILS_VERSION}.tar.xz" ] && wget -q --show-progress "https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
[ ! -f "gcc-${GCC_VERSION}.tar.xz" ]           && wget -q --show-progress "https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
[ ! -d "binutils-${BINUTILS_VERSION}" ] && tar xf "binutils-${BINUTILS_VERSION}.tar.xz"
[ ! -d "gcc-${GCC_VERSION}" ]           && tar xf "gcc-${GCC_VERSION}.tar.xz"
rm -rf "$BUILD_DIR/binutils" && mkdir -p "$BUILD_DIR/binutils" && cd "$BUILD_DIR/binutils"
"$SRC_DIR/binutils-${BINUTILS_VERSION}/configure" --target="$TARGET" --prefix="$PREFIX" --with-sysroot --disable-nls --disable-werror
make -j"$JOBS" && make install
rm -rf "$BUILD_DIR/gcc" && mkdir -p "$BUILD_DIR/gcc" && cd "$BUILD_DIR/gcc"
"$SRC_DIR/gcc-${GCC_VERSION}/configure" --target="$TARGET" --prefix="$PREFIX" --disable-nls --enable-languages=c --without-headers
make -j"$JOBS" all-gcc all-target-libgcc && make install-gcc install-target-libgcc
echo "Done: $("$PREFIX/bin/${TARGET}-gcc" --version | head -1)"
'@
        & $msys2Bash --login -c $buildCmd
        if ($LASTEXITCODE -eq 0) {
            Add-ToPathIfNeeded $crossBin
            Write-Host "  i686-elf-gcc built successfully." -ForegroundColor Green
        } else {
            Write-Host "  Build failed. C libraries and games will be skipped by CMake." -ForegroundColor Yellow
            Write-Host "  To retry manually: run scripts\build_cross_compiler.sh inside MSYS2."
        }
    }
} else {
    Write-Host "  MSYS2 not found at $msys2Root." -ForegroundColor Yellow
    Write-Host "  Install manually from https://www.msys2.org/ then re-run this script."
    Write-Host "  Without it, C libraries/TCC/games are skipped. Rust kernel builds fine."
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
