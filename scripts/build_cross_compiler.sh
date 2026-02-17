#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# SPDX-License-Identifier: MIT
#
# Build i686-elf cross-compiler (binutils + GCC) from source.
# Tested on Ubuntu 24.04 LTS.
#
# Installs to $HOME/opt/cross/ by default.
# After install, add to PATH:
#   export PATH="$HOME/opt/cross/bin:$PATH"

set -euo pipefail

TARGET="i686-elf"
PREFIX="${CROSS_PREFIX:-$HOME/opt/cross}"
BINUTILS_VERSION="2.44"
GCC_VERSION="14.2.0"
JOBS="$(nproc)"

SRC_DIR="$HOME/src/cross-compiler"
BUILD_DIR="$HOME/build/cross-compiler"

echo "========================================="
echo " anyOS cross-compiler setup"
echo "========================================="
echo "  Target:   $TARGET"
echo "  Prefix:   $PREFIX"
echo "  Binutils: $BINUTILS_VERSION"
echo "  GCC:      $GCC_VERSION"
echo "  Jobs:     $JOBS"
echo ""

# ── Prerequisites ────────────────────────────────────────────────────────────

echo "--- Installing build dependencies ---"
sudo apt-get update -qq
sudo apt-get install -y \
    build-essential \
    bison \
    flex \
    libgmp-dev \
    libmpc-dev \
    libmpfr-dev \
    texinfo \
    wget \
    xz-utils

# ── Directories ──────────────────────────────────────────────────────────────

mkdir -p "$PREFIX" "$SRC_DIR" "$BUILD_DIR"
export PATH="$PREFIX/bin:$PATH"

# ── Download ─────────────────────────────────────────────────────────────────

cd "$SRC_DIR"

if [ ! -f "binutils-${BINUTILS_VERSION}.tar.xz" ]; then
    echo "--- Downloading binutils-${BINUTILS_VERSION} ---"
    wget -q --show-progress "https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
fi

if [ ! -f "gcc-${GCC_VERSION}.tar.xz" ]; then
    echo "--- Downloading gcc-${GCC_VERSION} ---"
    wget -q --show-progress "https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
fi

# ── Extract ──────────────────────────────────────────────────────────────────

echo "--- Extracting sources ---"
[ ! -d "binutils-${BINUTILS_VERSION}" ] && tar xf "binutils-${BINUTILS_VERSION}.tar.xz"
[ ! -d "gcc-${GCC_VERSION}" ]           && tar xf "gcc-${GCC_VERSION}.tar.xz"

# ── Build binutils ───────────────────────────────────────────────────────────

echo ""
echo "--- Building binutils (${TARGET}) ---"
rm -rf "$BUILD_DIR/binutils"
mkdir -p "$BUILD_DIR/binutils" && cd "$BUILD_DIR/binutils"

"$SRC_DIR/binutils-${BINUTILS_VERSION}/configure" \
    --target="$TARGET" \
    --prefix="$PREFIX" \
    --with-sysroot \
    --disable-nls \
    --disable-werror

make -j"$JOBS"
make install
echo "--- binutils installed ---"

# ── Build GCC ────────────────────────────────────────────────────────────────

echo ""
echo "--- Building GCC (${TARGET}) ---"
rm -rf "$BUILD_DIR/gcc"
mkdir -p "$BUILD_DIR/gcc" && cd "$BUILD_DIR/gcc"

"$SRC_DIR/gcc-${GCC_VERSION}/configure" \
    --target="$TARGET" \
    --prefix="$PREFIX" \
    --disable-nls \
    --enable-languages=c \
    --without-headers

make -j"$JOBS" all-gcc all-target-libgcc
make install-gcc install-target-libgcc
echo "--- GCC installed ---"

# ── Verify ───────────────────────────────────────────────────────────────────

echo ""
echo "========================================="
echo " Installation complete!"
echo "========================================="
echo ""

"$PREFIX/bin/${TARGET}-gcc" --version | head -1
"$PREFIX/bin/${TARGET}-ld"  --version | head -1
"$PREFIX/bin/${TARGET}-ar"  --version | head -1

echo ""
echo "Add to your shell profile (~/.bashrc):"
echo ""
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo ""

if ! grep -q "$PREFIX/bin" ~/.bashrc 2>/dev/null; then
    read -p "Add to ~/.bashrc now? [y/N] " answer
    if [[ "$answer" =~ ^[Yy]$ ]]; then
        echo "export PATH=\"$PREFIX/bin:\$PATH\"" >> ~/.bashrc
        echo "Added. Run 'source ~/.bashrc' or open a new terminal."
    fi
fi
