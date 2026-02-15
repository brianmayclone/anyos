#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

set -e

echo "Setting up anyOS development toolchain..."
echo ""

# ── Detect OS ─────────────────────────────────────────────────────────────────

OS="$(uname -s)"
case "$OS" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    *)
        echo "Error: Unsupported OS '$OS'. anyOS builds on macOS and Linux."
        exit 1
        ;;
esac

echo "Detected platform: $PLATFORM"
echo ""

# ── Helper: install a package ────────────────────────────────────────────────

install_pkg() {
    local name="$1"         # human-readable name
    local brew_pkg="$2"     # Homebrew package name
    local apt_pkg="$3"      # apt package name

    echo "Installing $name..."
    if [ "$PLATFORM" = "macos" ]; then
        if command -v brew &> /dev/null; then
            brew install "$brew_pkg"
        else
            echo "Error: Homebrew not found. Install from https://brew.sh"
            exit 1
        fi
    else
        if command -v apt-get &> /dev/null; then
            sudo apt-get install -y "$apt_pkg"
        else
            echo "Error: apt-get not found. Please install $name manually."
            exit 1
        fi
    fi
}

# ── Rust nightly ──────────────────────────────────────────────────────────────

echo "--- Rust nightly ---"
if ! command -v rustup &> /dev/null; then
    echo "Installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
rustup install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
echo ""

# ── NASM ──────────────────────────────────────────────────────────────────────

echo "--- NASM ---"
if ! command -v nasm &> /dev/null; then
    install_pkg "NASM" "nasm" "nasm"
fi
echo ""

# ── CMake ─────────────────────────────────────────────────────────────────────

echo "--- CMake ---"
if ! command -v cmake &> /dev/null; then
    install_pkg "CMake" "cmake" "cmake"
fi
echo ""

# ── Ninja ─────────────────────────────────────────────────────────────────────

echo "--- Ninja ---"
if ! command -v ninja &> /dev/null; then
    install_pkg "Ninja" "ninja" "ninja-build"
fi
echo ""

# ── QEMU ──────────────────────────────────────────────────────────────────────

echo "--- QEMU ---"
if ! command -v qemu-system-x86_64 &> /dev/null; then
    install_pkg "QEMU" "qemu" "qemu-system-x86"
fi
echo ""

# ── Python 3 + pip packages ──────────────────────────────────────────────────

echo "--- Python 3 ---"
if ! command -v python3 &> /dev/null; then
    install_pkg "Python 3" "python3" "python3"
fi

# pip packages for build scripts (mkimage.py, font rendering)
if ! python3 -c "import PIL" &> /dev/null || ! python3 -c "import fontTools" &> /dev/null; then
    echo "Installing Python packages (Pillow, fonttools)..."
    if [ "$PLATFORM" = "linux" ]; then
        # Ensure pip is available on Ubuntu
        if ! command -v pip3 &> /dev/null; then
            sudo apt-get install -y python3-pip
        fi
    fi
    pip3 install --user Pillow fonttools 2>/dev/null || python3 -m pip install --user Pillow fonttools
fi
echo ""

# ── i686-elf cross-compiler (for libc + TCC) ─────────────────────────────────

echo "--- i686-elf-gcc cross-compiler ---"
if ! command -v i686-elf-gcc &> /dev/null; then
    echo "i686-elf-gcc not found."
    if [ "$PLATFORM" = "macos" ]; then
        echo "Installing via Homebrew tap..."
        brew tap nativeos/i386-elf-toolchain
        brew install i386-elf-binutils i386-elf-gcc
        # Create symlinks: i386-elf-* -> i686-elf-*
        BREW_PREFIX="$(brew --prefix)"
        for tool in gcc ar as ld objcopy objdump; do
            if [ -f "$BREW_PREFIX/bin/i386-elf-$tool" ] && [ ! -f "$BREW_PREFIX/bin/i686-elf-$tool" ]; then
                ln -sf "$BREW_PREFIX/bin/i386-elf-$tool" "$BREW_PREFIX/bin/i686-elf-$tool"
            fi
        done
    else
        echo ""
        echo "On Ubuntu/Debian, you can build from source or use a prebuilt toolchain:"
        echo ""
        echo "  Option 1 — Build from source (takes ~15 min):"
        echo "    sudo apt-get install -y build-essential bison flex libgmp-dev"
        echo "    libmpc-dev libmpfr-dev texinfo"
        echo "    ./scripts/build_cross_compiler.sh"
        echo ""
        echo "  Option 2 — Use a prebuilt toolchain:"
        echo "    Download from https://github.com/lordmilko/i686-elf-tools/releases"
        echo "    Extract and add to PATH"
        echo ""
        echo "  The cross-compiler is needed for the C library (libc) and TCC."
        echo "  If you only work on Rust programs, you can skip this for now."
        echo ""
    fi
else
    echo "i686-elf-gcc found: $(i686-elf-gcc --version | head -1)"
fi
echo ""

# ── OVMF firmware (UEFI boot, optional) ──────────────────────────────────────

echo "--- OVMF firmware (optional, for UEFI boot) ---"
if [ "$PLATFORM" = "macos" ]; then
    OVMF_PATH="/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
    if [ -f "$OVMF_PATH" ]; then
        echo "OVMF found at $OVMF_PATH"
    else
        echo "OVMF not found. Install QEMU via Homebrew to get OVMF firmware."
        echo "  (UEFI boot is optional — BIOS boot works without it)"
    fi
else
    OVMF_PATH="/usr/share/OVMF/OVMF_CODE.fd"
    if [ -f "$OVMF_PATH" ]; then
        echo "OVMF found at $OVMF_PATH"
    else
        echo "OVMF not found. Install with: sudo apt-get install ovmf"
        echo "  (UEFI boot is optional — BIOS boot works without it)"
    fi
fi
echo ""

# ── Summary ───────────────────────────────────────────────────────────────────

echo "========================================"
echo "Toolchain versions:"
echo "  rustc: $(rustc +nightly --version 2>/dev/null || echo 'not found')"
echo "  nasm:  $(nasm --version 2>/dev/null || echo 'not found')"
echo "  cmake: $(cmake --version 2>/dev/null | head -1 || echo 'not found')"
echo "  ninja: $(ninja --version 2>/dev/null || echo 'not found')"
echo "  qemu:  $(qemu-system-x86_64 --version 2>/dev/null | head -1 || echo 'not found')"
if command -v i686-elf-gcc &> /dev/null; then
    echo "  i686-elf-gcc: $(i686-elf-gcc --version 2>/dev/null | head -1)"
else
    echo "  i686-elf-gcc: not installed (needed for libc/TCC only)"
fi
echo "========================================"
echo ""
echo "Toolchain setup complete!"
echo ""
echo "Next steps:"
echo "  mkdir -p build && cd build"
echo "  cmake .. -G Ninja"
echo "  ninja          # Build everything"
echo "  ninja run      # Run in QEMU"
