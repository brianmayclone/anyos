# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build i686-elf cross-compiler (binutils + GCC) from source on Windows.
# Uses MSYS2/MinGW64 shell for the GNU build system (configure/make).
#
# Installs to $env:USERPROFILE\opt\cross by default.
# After install, add to PATH:
#   $env:Path = "$env:USERPROFILE\opt\cross\bin;$env:Path"
#
# Usage: .\scripts\build_cross_compiler.ps1

$ErrorActionPreference = "Stop"

$TARGET = "i686-elf"
$PREFIX = if ($env:CROSS_PREFIX) { $env:CROSS_PREFIX } else { Join-Path $env:USERPROFILE "opt\cross" }
$BINUTILS_VERSION = "2.44"
$GCC_VERSION = "14.2.0"
$JOBS = $env:NUMBER_OF_PROCESSORS

$SRC_DIR = Join-Path $env:USERPROFILE "src\cross-compiler"
$BUILD_DIR_CC = Join-Path $env:USERPROFILE "build\cross-compiler"

Write-Host "=========================================" -ForegroundColor Cyan
Write-Host " anyOS cross-compiler setup (Windows)"
Write-Host "========================================="
Write-Host "  Target:   $TARGET"
Write-Host "  Prefix:   $PREFIX"
Write-Host "  Binutils: $BINUTILS_VERSION"
Write-Host "  GCC:      $GCC_VERSION"
Write-Host "  Jobs:     $JOBS"
Write-Host ""

# ── Find MSYS2 ───────────────────────────────────────────────────────────────

$msys2Root = "C:\msys64"
$msys2Bash = Join-Path $msys2Root "usr\bin\bash.exe"

if (-not (Test-Path $msys2Bash)) {
    Write-Host "Error: MSYS2 not found at $msys2Root" -ForegroundColor Red
    Write-Host "Install with: winget install MSYS2.MSYS2"
    Write-Host "Or run: .\scripts\setup_toolchain.ps1"
    exit 1
}

# ── Install build dependencies via pacman ─────────────────────────────────────

Write-Host "--- Installing build dependencies via MSYS2 pacman ---"
& $msys2Bash --login -c "pacman -S --needed --noconfirm base-devel mingw-w64-x86_64-gcc bison flex gmp-devel mpc-devel mpfr-devel texinfo wget tar xz" 2>$null

# ── Build via MSYS2 shell (configure/make require a Unix-like environment) ────

# Convert Windows paths to MSYS2 paths
$msys2Prefix = ($PREFIX -replace '\\', '/') -replace '^([A-Za-z]):', '/$1'
$msys2SrcDir = ($SRC_DIR -replace '\\', '/') -replace '^([A-Za-z]):', '/$1'
$msys2BuildDir = ($BUILD_DIR_CC -replace '\\', '/') -replace '^([A-Za-z]):', '/$1'

# Create directories
New-Item -ItemType Directory -Force -Path $PREFIX | Out-Null
New-Item -ItemType Directory -Force -Path $SRC_DIR | Out-Null
New-Item -ItemType Directory -Force -Path $BUILD_DIR_CC | Out-Null

# Build script to run inside MSYS2
$buildScript = @"
set -euo pipefail

TARGET="$TARGET"
PREFIX="$msys2Prefix"
BINUTILS_VERSION="$BINUTILS_VERSION"
GCC_VERSION="$GCC_VERSION"
JOBS="$JOBS"
SRC_DIR="$msys2SrcDir"
BUILD_DIR="$msys2BuildDir"

export PATH="`$PREFIX/bin:`$PATH"

cd "`$SRC_DIR"

# Download
if [ ! -f "binutils-`${BINUTILS_VERSION}.tar.xz" ]; then
    echo "--- Downloading binutils-`${BINUTILS_VERSION} ---"
    wget -q --show-progress "https://ftp.gnu.org/gnu/binutils/binutils-`${BINUTILS_VERSION}.tar.xz"
fi

if [ ! -f "gcc-`${GCC_VERSION}.tar.xz" ]; then
    echo "--- Downloading gcc-`${GCC_VERSION} ---"
    wget -q --show-progress "https://ftp.gnu.org/gnu/gcc/gcc-`${GCC_VERSION}/gcc-`${GCC_VERSION}.tar.xz"
fi

# Extract
echo "--- Extracting sources ---"
[ ! -d "binutils-`${BINUTILS_VERSION}" ] && tar xf "binutils-`${BINUTILS_VERSION}.tar.xz"
[ ! -d "gcc-`${GCC_VERSION}" ]           && tar xf "gcc-`${GCC_VERSION}.tar.xz"

# Build binutils
echo ""
echo "--- Building binutils (`${TARGET}) ---"
rm -rf "`$BUILD_DIR/binutils"
mkdir -p "`$BUILD_DIR/binutils" && cd "`$BUILD_DIR/binutils"

"`$SRC_DIR/binutils-`${BINUTILS_VERSION}/configure" \
    --target="`$TARGET" \
    --prefix="`$PREFIX" \
    --with-sysroot \
    --disable-nls \
    --disable-werror

make -j"`$JOBS"
make install
echo "--- binutils installed ---"

# Build GCC
echo ""
echo "--- Building GCC (`${TARGET}) ---"
rm -rf "`$BUILD_DIR/gcc"
mkdir -p "`$BUILD_DIR/gcc" && cd "`$BUILD_DIR/gcc"

"`$SRC_DIR/gcc-`${GCC_VERSION}/configure" \
    --target="`$TARGET" \
    --prefix="`$PREFIX" \
    --disable-nls \
    --enable-languages=c \
    --without-headers

make -j"`$JOBS" all-gcc all-target-libgcc
make install-gcc install-target-libgcc
echo "--- GCC installed ---"

echo ""
echo "========================================="
echo " Installation complete!"
echo "========================================="
"`$PREFIX/bin/`${TARGET}-gcc" --version | head -1
"`$PREFIX/bin/`${TARGET}-ld"  --version | head -1
"@

& $msys2Bash --login -c $buildScript
if ($LASTEXITCODE -ne 0) {
    Write-Host "Cross-compiler build failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}

# ── Add to PATH ──────────────────────────────────────────────────────────────

$crossBin = Join-Path $PREFIX "bin"
Write-Host ""
Write-Host "Add to your user PATH:" -ForegroundColor Yellow
Write-Host "  `$env:Path = `"$crossBin;`$env:Path`""
Write-Host ""

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$crossBin*") {
    $answer = Read-Host "Add to user PATH now? [y/N]"
    if ($answer -match '^[Yy]$') {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$crossBin", "User")
        $env:Path = "$crossBin;$env:Path"
        Write-Host "Added. Open a new terminal or restart PowerShell." -ForegroundColor Green
    }
}
