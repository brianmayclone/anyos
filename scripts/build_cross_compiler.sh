#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT
#
# Build i686-elf cross-compiler (binutils + GCC) from source.
# Installs to $HOME/opt/cross/ by default.
#
# Prerequisites (Ubuntu/Debian):
#   sudo apt-get install -y build-essential bison flex libgmp-dev \
#       libmpc-dev libmpfr-dev texinfo

set -e

TARGET="i686-elf"
PREFIX="${CROSS_PREFIX:-$HOME/opt/cross}"
BINUTILS_VERSION="2.42"
GCC_VERSION="13.2.0"
JOBS="$(nproc 2>/dev/null || echo 4)"

BINUTILS_URL="https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
GCC_URL="https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"

BUILD_DIR="/tmp/cross-build-$$"

echo "Building $TARGET cross-compiler"
echo "  Prefix:   $PREFIX"
echo "  Binutils: $BINUTILS_VERSION"
echo "  GCC:      $GCC_VERSION"
echo "  Jobs:     $JOBS"
echo ""

# Check prerequisites
for cmd in make gcc g++ bison flex makeinfo; do
    if ! command -v "$cmd" &> /dev/null; then
        echo "Error: '$cmd' not found. Install build prerequisites:"
        echo "  sudo apt-get install -y build-essential bison flex libgmp-dev libmpc-dev libmpfr-dev texinfo"
        exit 1
    fi
done

mkdir -p "$PREFIX"
mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

export PATH="$PREFIX/bin:$PATH"

# ── Binutils ──────────────────────────────────────────────────────────────────

echo "--- Downloading binutils-${BINUTILS_VERSION} ---"
if [ ! -f "binutils-${BINUTILS_VERSION}.tar.xz" ]; then
    curl -LO "$BINUTILS_URL"
fi
echo "--- Extracting ---"
tar xf "binutils-${BINUTILS_VERSION}.tar.xz"

echo "--- Building binutils ---"
mkdir -p build-binutils && cd build-binutils
"../binutils-${BINUTILS_VERSION}/configure" \
    --target="$TARGET" \
    --prefix="$PREFIX" \
    --with-sysroot \
    --disable-nls \
    --disable-werror
make -j"$JOBS"
make install
cd ..

echo "--- binutils installed ---"
echo ""

# ── GCC ───────────────────────────────────────────────────────────────────────

echo "--- Downloading gcc-${GCC_VERSION} ---"
if [ ! -f "gcc-${GCC_VERSION}.tar.xz" ]; then
    curl -LO "$GCC_URL"
fi
echo "--- Extracting ---"
tar xf "gcc-${GCC_VERSION}.tar.xz"

echo "--- Building GCC ---"
mkdir -p build-gcc && cd build-gcc
"../gcc-${GCC_VERSION}/configure" \
    --target="$TARGET" \
    --prefix="$PREFIX" \
    --disable-nls \
    --enable-languages=c \
    --without-headers
make -j"$JOBS" all-gcc all-target-libgcc
make install-gcc install-target-libgcc
cd ..

echo "--- GCC installed ---"
echo ""

# ── Cleanup ───────────────────────────────────────────────────────────────────

rm -rf "$BUILD_DIR"

echo "========================================"
echo "Cross-compiler installed to: $PREFIX"
echo ""
echo "Add to your PATH:"
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo ""
echo "Add this line to ~/.bashrc or ~/.profile to make it permanent."
echo ""
echo "Verify:"
echo "  $TARGET-gcc --version"
echo "========================================"
